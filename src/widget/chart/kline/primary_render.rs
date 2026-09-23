use super::composition::{BarMode, HistogramMode, PanelPresentation};
use super::footprint::{
    FootprintColumnLayout, FootprintImbalance, FootprintTextMode, draw_footprint_candle,
    draw_footprint_cluster_row, footprint_text_content_lengths, resolve_footprint_text_mode,
};
use super::helpers::{PixelGeometry, PixelSpan};
use super::scene::IndicatorPanelScene;
use super::scene_builder::{
    IndicatorLinePointStatus, IndicatorLineTraversal, bar_index_range_for_units,
    shifted_output_unit,
};
use super::*;
use data::chart::kline::series::KlineSeries;
use data::chart::kline::{
    IndicatorFillGeometry, IndicatorFillSpec, IndicatorInputSource, IndicatorOutputRef,
    LineStrokeStyle, OutputValue, ProfileDistribution, StudyDataSource, TableColumnColor,
    TableColumnFormat, TableColumnValue, TpoColorConfig, TpoProfile, TpoProfileStatus,
};
use data::chart::kline_data::ClusterKind;
use data::config::theme::ACCENT_COUNT;
use data::util::abbr_large_numbers;
use exchange::UnixMs;
use exchange::unit::{Price, Qty};
use rustc_hash::FxHashSet;
use std::collections::BTreeMap;
use std::ops::Range;

const FOOTPRINT_MIN_COLUMN_WIDTH_PX: f32 = 24.0;
const FOOTPRINT_CELL_INSET_PX: f32 = 2.0;
const TABLE_TEXT_MIN_COLUMN_WIDTH_PX: f32 = 48.0;
const TABLE_TEXT_MIN_ROW_HEIGHT_PX: f32 = 9.0;
const TABLE_TEXT_MIN_SIZE_PX: f32 = 8.0;
const TABLE_CELL_INSET_PX: f32 = 1.0;
const INDICATOR_MARKER_TEXT_SIZE_PX: f32 = 8.0;
const DASHED_LINE_SEGMENTS: [f32; 2] = [8.0, 5.0];
const DOTTED_LINE_SEGMENTS: [f32; 2] = [1.0, 4.0];
const MAX_BIG_TRADE_BUBBLES: usize = 2_048;
const BIG_TRADE_MIN_ROW_PX: f32 = 16.0;
const BIG_TRADE_MIN_COL_PX: f32 = 16.0;
const BIG_TRADE_MAX_RADIUS_PX: f32 = 24.0;
const BIG_TRADE_MIN_RADIUS_PX: f32 = 3.0;

#[derive(Default)]
struct BigTradeCell {
    notional: f64,
    count: u32,
    unit_sum: f64,
}

struct PixelRows {
    top: f32,
    height: f32,
    bucket_height: f32,
    quantities: Vec<(Qty, Qty)>,
    touched: Vec<usize>,
}

impl PixelRows {
    fn new(plot: Rectangle, pixels: PixelGeometry) -> Self {
        let pixel_ratio = pixels.ratio();
        let origin_y = pixels.origin().y;
        let top_physical = ((plot.y + origin_y) * pixel_ratio).floor() as i32;
        let bottom_physical = ((plot.y + plot.height + origin_y) * pixel_ratio).ceil() as i32;
        let bucket_count = bottom_physical.saturating_sub(top_physical) as usize;
        let bucket_height = 1.0 / pixel_ratio;
        Self {
            top: top_physical as f32 / pixel_ratio - origin_y,
            height: bucket_count as f32 / pixel_ratio,
            bucket_height,
            quantities: vec![(Qty::ZERO, Qty::ZERO); bucket_count],
            touched: Vec::new(),
        }
    }

    fn add(&mut self, y: f32, buy: Qty, sell: Qty) {
        let offset = y - self.top;
        let half_bucket = self.bucket_height * 0.5;
        if !offset.is_finite()
            || offset < -half_bucket
            || offset > self.height + half_bucket
            || self.quantities.is_empty()
        {
            return;
        }
        let index = (offset / self.bucket_height)
            .floor()
            .clamp(0.0, self.quantities.len().saturating_sub(1) as f32)
            as usize;
        if let Some(row) = self.quantities.get_mut(index) {
            let was_empty = row.0.is_zero() && row.1.is_zero();
            row.0 += buy;
            row.1 += sell;
            if was_empty && (!row.0.is_zero() || !row.1.is_zero()) {
                self.touched.push(index);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.touched.iter().all(|&index| {
            let (buy, sell) = self.quantities[index];
            buy.is_zero() && sell.is_zero()
        })
    }

    fn max_qty(&self, kind: ClusterKind) -> Qty {
        self.touched
            .iter()
            .map(|&index| {
                let (buy, sell) = self.quantities[index];
                match kind {
                    ClusterKind::BidAsk | ClusterKind::Table => buy.max(sell),
                    ClusterKind::VolumeProfile => buy + sell,
                    ClusterKind::DeltaProfile => buy.abs_diff(sell),
                }
            })
            .max()
            .unwrap_or_default()
    }

    fn rows(&self) -> impl Iterator<Item = (f32, Qty, Qty)> + '_ {
        let mut touched = self.touched.clone();
        touched.sort_unstable();
        touched.into_iter().filter_map(|index| {
            let (buy, sell) = self.quantities[index];
            (!buy.is_zero() || !sell.is_zero()).then_some((
                self.top + (index as f32 + 0.5) * self.bucket_height,
                buy,
                sell,
            ))
        })
    }
}

fn indicator_line_stroke(
    width: f32,
    style: LineStrokeStyle,
    color: iced::Color,
) -> canvas::Stroke<'static> {
    let mut stroke = canvas::Stroke::default()
        .with_width(width)
        .with_color(color);
    stroke.line_dash = match style {
        LineStrokeStyle::Solid => canvas::LineDash::default(),
        LineStrokeStyle::Dashed => canvas::LineDash {
            segments: &DASHED_LINE_SEGMENTS,
            offset: 0,
        },
        LineStrokeStyle::Dotted => canvas::LineDash {
            segments: &DOTTED_LINE_SEGMENTS,
            offset: 0,
        },
    };
    if matches!(style, LineStrokeStyle::Dotted) {
        stroke.line_cap = canvas::LineCap::Round;
    }
    stroke
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrimaryPlotDetail {
    CandleOnly,
    Footprint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrimaryPlotLayer {
    Background,
    Tpo,
    Foreground,
}

fn footprint_detail_enabled(trade_data_enabled: bool, logical_bar_spacing: f32) -> bool {
    trade_data_enabled && logical_bar_spacing >= FOOTPRINT_MIN_COLUMN_WIDTH_PX
}

#[derive(Debug, Clone, Copy)]
struct BandPoint {
    upper: Point,
    lower: Point,
}

#[derive(Debug)]
struct BandSegment {
    points: Vec<BandPoint>,
    style_by: Option<bool>,
}

#[derive(Debug)]
struct IndicatorLineRun {
    points: Vec<Point>,
    color: iced::Color,
}

fn append_indicator_line_point(
    runs: &mut Vec<IndicatorLineRun>,
    previous: Option<Point>,
    point: Point,
    color: iced::Color,
    step_line: bool,
) {
    let Some(previous) = previous else {
        runs.push(IndicatorLineRun {
            points: vec![point],
            color,
        });
        return;
    };

    let same_color = runs.last().is_some_and(|run| run.color == color);
    if !same_color {
        runs.push(IndicatorLineRun {
            points: vec![previous, point],
            color,
        });
        return;
    }

    let Some(run) = runs.last_mut() else {
        return;
    };
    if step_line {
        run.points.push(Point::new(point.x, previous.y));
    }
    run.points.push(point);
}

fn trim_indicator_line_runs(runs: Vec<IndicatorLineRun>, plot: Rectangle) -> Vec<IndicatorLineRun> {
    let left = plot.x;
    let right = plot.x + plot.width;

    runs.into_iter()
        .filter_map(|run| {
            if run.points.len() == 1 {
                return (run.points[0].x >= left && run.points[0].x <= right).then_some(run);
            }
            let mut points = Vec::new();
            for pair in run.points.windows(2) {
                let segment_left = pair[0].x.min(pair[1].x);
                let segment_right = pair[0].x.max(pair[1].x);
                if segment_right < left || segment_left > right {
                    continue;
                }
                if points.last().copied() != Some(pair[0]) {
                    points.push(pair[0]);
                }
                if points.last().copied() != Some(pair[1]) {
                    points.push(pair[1]);
                }
            }
            (!points.is_empty()).then_some(IndicatorLineRun {
                points,
                color: run.color,
            })
        })
        .collect()
}

fn indicator_line_path(points: &[Point]) -> canvas::Path {
    canvas::Path::new(|builder| {
        builder.move_to(points[0]);
        for point in points.iter().skip(1) {
            builder.line_to(*point);
        }
    })
}

fn snap_indicator_line_points(
    points: &[Point],
    pixels: PixelGeometry,
    stroke_width_phys: i32,
) -> Vec<Point> {
    points
        .iter()
        .map(|point| {
            Point::new(
                pixels.snap_x_for_stroke(point.x, stroke_width_phys),
                pixels.snap_y_for_stroke(point.y, stroke_width_phys),
            )
        })
        .collect()
}

fn fill_minimum_mark(
    frame: &mut canvas::Frame,
    point: Point,
    color: iced::Color,
    pixels: PixelGeometry,
) {
    if !point.x.is_finite() || !point.y.is_finite() {
        return;
    }
    let size = pixels.logical_width_from_physical(1);
    let x = pixels.snapped_x_span(point.x - size * 0.5, point.x + size * 0.5);
    let y = pixels.snapped_y_span(point.y - size * 0.5, point.y + size * 0.5);
    frame.fill_rectangle(
        Point::new(x.start, y.start),
        Size::new(x.size, y.size),
        color,
    );
}

fn fill_indicator_area(
    frame: &mut canvas::Frame,
    raw_points: &[Point],
    snapped_points: &[Point],
    baseline: f32,
    color: iced::Color,
    pixels: PixelGeometry,
) {
    if raw_points.len() < 2
        || raw_points.len() != snapped_points.len()
        || !baseline.is_finite()
        || raw_points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        || snapped_points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return;
    }

    let (min_x, max_x, min_y, max_y) = raw_points.iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY, baseline, baseline),
        |(min_x, max_x, min_y, max_y), point| {
            (
                min_x.min(point.x),
                max_x.max(point.x),
                min_y.min(point.y),
                max_y.max(point.y),
            )
        },
    );
    let min_size = pixels.logical_width_from_physical(1);
    if max_x - min_x < min_size || max_y - min_y < min_size {
        let Some(x) = pixels.positive_x_span(min_x, max_x) else {
            return;
        };
        let Some(y) = pixels.positive_y_span(min_y, max_y) else {
            return;
        };
        frame.fill_rectangle(
            Point::new(x.start, y.start),
            Size::new(x.size, y.size),
            color.scale_alpha(0.2),
        );
        return;
    }

    let path = canvas::Path::new(|builder| {
        builder.move_to(snapped_points[0]);
        for point in snapped_points.iter().skip(1) {
            builder.line_to(*point);
        }
        builder.line_to(Point::new(
            snapped_points[snapped_points.len() - 1].x,
            baseline,
        ));
        builder.line_to(Point::new(snapped_points[0].x, baseline));
        builder.close();
    });
    frame.fill(&path, color.scale_alpha(0.2));
}

fn draw_indicator_bar_outline(
    frame: &mut canvas::Frame,
    origin: Point,
    size: Size,
    color: iced::Color,
    line_width: f32,
    stroke_style: LineStrokeStyle,
) {
    frame.stroke(
        &canvas::Path::rectangle(origin, size),
        indicator_line_stroke(line_width, stroke_style, color),
    );
}

fn indicator_region_rect(
    plot: Rectangle,
    left_x: f32,
    right_x: f32,
    upper_y: f32,
    lower_y: f32,
    extend_right: bool,
    pixels: PixelGeometry,
) -> Option<Rectangle> {
    if [left_x, right_x, upper_y, lower_y]
        .into_iter()
        .any(|value| !value.is_finite())
    {
        return None;
    }
    let right_x = if extend_right {
        plot.x + plot.width
    } else {
        right_x
    };
    if plot.width <= 0.0 || plot.height <= 0.0 {
        return None;
    }
    let plot_right = plot.x + plot.width;
    let plot_bottom = plot.y + plot.height;
    let left_edge = left_x.min(right_x);
    let right_edge = left_x.max(right_x);
    let top_edge = upper_y.min(lower_y);
    let bottom_edge = upper_y.max(lower_y);
    if right_edge < plot.x
        || left_edge > plot_right
        || bottom_edge < plot.y
        || top_edge > plot_bottom
    {
        return None;
    }
    let x = pixels.positive_x_span_in_bounds(left_edge, right_edge, plot.x, plot_right)?;
    let y = pixels.positive_y_span_in_bounds(top_edge, bottom_edge, plot.y, plot_bottom)?;
    Some(Rectangle {
        x: x.start,
        y: y.start,
        width: x.size,
        height: y.size,
    })
}

fn indicator_region_source_range(
    point_count: usize,
    visible_start: usize,
    visible_end: usize,
) -> Range<usize> {
    if visible_start < visible_end {
        visible_start..visible_end
    } else {
        0..point_count
    }
}

fn fill_static_band(
    frame: &mut canvas::Frame,
    plot: Rectangle,
    upper_y: f32,
    lower_y: f32,
    color: iced::Color,
    pixels: PixelGeometry,
) {
    let top_edge = upper_y.min(lower_y);
    let bottom_edge = upper_y.max(lower_y);
    if !top_edge.is_finite()
        || !bottom_edge.is_finite()
        || plot.width <= 0.0
        || plot.height <= 0.0
        || bottom_edge < plot.y
        || top_edge > plot.y + plot.height
    {
        return;
    }
    let Some(span) =
        pixels.positive_y_span_in_bounds(top_edge, bottom_edge, plot.y, plot.y + plot.height)
    else {
        return;
    };

    frame.fill_rectangle(
        Point::new(plot.x, span.start),
        Size::new(plot.width, span.size),
        color,
    );
}

fn fill_band_segments(
    frame: &mut canvas::Frame,
    segments: Vec<BandSegment>,
    pixels: PixelGeometry,
    mut color_for: impl FnMut(Option<bool>) -> iced::Color,
) {
    let min_size = pixels.logical_width_from_physical(1);
    for segment in segments
        .into_iter()
        .filter(|segment| !segment.points.is_empty())
    {
        let color = color_for(segment.style_by);
        let points = segment.points;
        let (min_x, max_x, min_y, max_y) = points.iter().fold(
            (
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), point| {
                (
                    min_x.min(point.upper.x).min(point.lower.x),
                    max_x.max(point.upper.x).max(point.lower.x),
                    min_y.min(point.upper.y).min(point.lower.y),
                    max_y.max(point.upper.y).max(point.lower.y),
                )
            },
        );
        if !min_x.is_finite() || !max_x.is_finite() || !min_y.is_finite() || !max_y.is_finite() {
            continue;
        }
        if max_x - min_x < min_size || max_y - min_y < min_size {
            let Some(x) = pixels.positive_x_span(min_x, max_x) else {
                continue;
            };
            let Some(y) = pixels.positive_y_span(min_y, max_y) else {
                continue;
            };
            frame.fill_rectangle(
                Point::new(x.start, y.start),
                Size::new(x.size, y.size),
                color,
            );
            continue;
        }
        if points.len() < 2 {
            continue;
        }
        let path = canvas::Path::new(|builder| {
            builder.move_to(points[0].upper);
            for point in points.iter().skip(1) {
                builder.line_to(point.upper);
            }
            for point in points.iter().rev() {
                builder.line_to(point.lower);
            }
            builder.close();
        });
        frame.fill(&path, color);
    }
}

impl<'a> KlineWidget<'a> {
    #[allow(clippy::too_many_arguments)]
    fn draw_primary_indicator_points(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        value_id: PanelValueId,
        output: &IndicatorOutputRef,
        channel: OverlayChannelSpec<'_>,
        mark: MarkKind,
        required_input: Option<IndicatorInputSource>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        anchor: Option<f32>,
        pixels: PixelGeometry,
    ) {
        let source_channel = channel.source;
        let has_dynamic_style = channel.style_by.is_some()
            || self
                .indicator_output_spec_for_channel(value_id, source_channel)
                .is_some_and(|spec| spec.style_by.is_some());
        let fallback_color = self.overlay_channel_color_for_output(
            Some(value_id),
            Some(output),
            &channel,
            palette,
            generated_accents,
        );
        let stroke_width =
            self.indicator_line_width_for_output(Some(value_id), Some(output), &channel);
        self.for_each_scene_indicator_output_index_for_channel(
            scene,
            value_id,
            source_channel,
            |series, source_index, x_unit| {
                if !self.indicator_input_available_at(series, source_index, required_input) {
                    return;
                }
                let Some(value) = self
                    .indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        source_channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_number)
                else {
                    return;
                };
                let Some(y) = scene.map_primary_plot_with_anchor(value, anchor) else {
                    return;
                };
                let color = if has_dynamic_style {
                    self.overlay_channel_color_for_channel_at(
                        series,
                        source_index,
                        value_id,
                        value_id,
                        source_channel,
                        &channel,
                        palette,
                        generated_accents,
                    )
                } else {
                    fallback_color
                };
                let stroke_width_phys = pixels.quantized_stroke_width(stroke_width).1;
                let point = Point::new(
                    pixels.snap_x_for_stroke(scene.map_x_plot(x_unit), stroke_width_phys),
                    pixels.snap_y_for_stroke(y, stroke_width_phys),
                );
                let radius = (stroke_width * 1.6).max(2.0);
                if mark == MarkKind::Point {
                    frame.fill(&canvas::Path::circle(point, radius), color);
                } else {
                    let path = canvas::Path::new(|builder| {
                        builder.move_to(Point::new(point.x - radius, point.y));
                        builder.line_to(Point::new(point.x + radius, point.y));
                        builder.move_to(Point::new(point.x, point.y - radius));
                        builder.line_to(Point::new(point.x, point.y + radius));
                    });
                    frame.stroke(
                        &path,
                        indicator_line_stroke(
                            stroke_width,
                            self.indicator_stroke_style_for_output(
                                Some(value_id),
                                Some(output),
                                &channel,
                            ),
                            color,
                        ),
                    );
                }
            },
        );
    }

    pub(super) fn primary_plot_detail(&self, scene: &Scene) -> PrimaryPlotDetail {
        if footprint_detail_enabled(self.trade_data_enabled, scene.logical_bar_spacing()) {
            PrimaryPlotDetail::Footprint
        } else {
            PrimaryPlotDetail::CandleOnly
        }
    }

    pub(super) fn footprint_row_height(
        &self,
        scene: &Scene,
        price: Price,
        tick_size: PriceStep,
        anchor: Option<f32>,
    ) -> f32 {
        let current_y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
        let Some(next_price) = price.checked_add_steps(1, tick_size) else {
            return 0.0;
        };
        let next_y = scene.map_primary_plot_with_anchor_f32(next_price.to_f32_lossy(), anchor);
        (current_y - next_y).abs()
    }

    fn coalesce_price_rows(
        &self,
        scene: &Scene,
        tick_size: PriceStep,
        anchor: Option<f32>,
        pixels: PixelGeometry,
    ) -> bool {
        let pixel_ratio = pixels.ratio();
        let (low, high) = scene.primary_domain_value_range();
        [low, high].into_iter().all(|price| {
            let row_height = self.footprint_row_height(scene, price, tick_size, anchor);
            row_height.is_finite() && row_height * pixel_ratio < 1.0
        })
    }

    pub(super) fn candlestick_color(
        &self,
        bullish: bool,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let slot = if bullish {
            CandlestickColorSlot::Bullish
        } else {
            CandlestickColorSlot::Bearish
        };
        Self::resolve_theme_color(
            self.candlestick.colors.color_for(slot),
            palette,
            generated_accents,
        )
        .scale_alpha(self.candlestick.opacity_ratio())
    }

    pub(super) fn fill_footprint_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        draw_clusters: bool,
        pixels: PixelGeometry,
    ) {
        let Some(base_series) = self.series.first() else {
            return;
        };
        let tick_size = base_series.footprint_tick_size();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let min_row_height = pixels.logical_width_from_physical(1);
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.x.min_unit && x_unit <= scene.x.max_unit;
        let coalesce = draw_clusters && self.coalesce_price_rows(scene, tick_size, anchor, pixels);

        let mut pixel_rows = BTreeMap::new();
        if coalesce {
            self.for_each_scene_bar_unit_index(scene, |series_index, index, _, _, point| {
                if series_index != 0 || point.footprint.trades.is_empty() {
                    return;
                }
                let mut rows = PixelRows::new(*scene.primary_plot(), pixels);
                for (price, group) in &point.footprint.trades {
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    rows.add(y, group.buy_qty, group.sell_qty);
                }
                if !rows.is_empty() {
                    pixel_rows.insert(index, rows);
                }
            });
        }
        let max_cluster_qty = if coalesce {
            pixel_rows
                .values()
                .map(|rows| rows.max_qty(self.footprint.cluster_kind))
                .max()
                .unwrap_or_default()
                .to_f64()
        } else if draw_clusters {
            let visible_point_range = scene.bar_units.first().map(|units| {
                let (start, end) =
                    bar_index_range_for_units(&units.values, scene.x.min_unit, scene.x.max_unit);
                let end = end
                    .min(units.values.len())
                    .min(base_series.point_count().saturating_sub(units.start_index));
                units.global_range(start..end)
            });
            let (lowest, highest) = scene.primary_domain_value_range();
            visible_point_range
                .map(|point_range| {
                    base_series
                        .max_cluster_qty_in_point_range(
                            self.footprint.cluster_kind,
                            point_range,
                            lowest,
                            highest,
                        )
                        .to_f64()
                })
                .unwrap_or_default()
        } else {
            0.0
        };

        let primary_plot = *scene.primary_plot();
        let footprint_width =
            (scene.logical_bar_spacing() - FOOTPRINT_CELL_INSET_PX * 2.0).max(1.0);
        let imbalance = (self.footprint.imbalance.show && !coalesce).then(|| {
            (
                self.footprint.imbalance.threshold_percent,
                self.footprint.imbalance.ignore_zeros,
                Self::resolve_theme_color(
                    self.footprint.imbalance.color,
                    palette,
                    generated_accents,
                ),
            )
        });
        let primary_plot_bottom = primary_plot.y + primary_plot.height;
        let mut minimum_row_height: Option<f32> = None;
        let mut maximum_content_chars = 0;
        if draw_clusters && !coalesce {
            self.for_each_scene_bar_unit_index(scene, |series_index, _, _series, x_unit, point| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let footprint = point.footprint;
                for (price, group) in &footprint.trades {
                    let row_height = self
                        .footprint_row_height(scene, *price, tick_size, anchor)
                        .max(min_row_height);
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    let cell_height = (row_height - min_row_height).max(min_row_height);
                    let row_top = y - cell_height * 0.5;
                    let row_bottom = row_top + cell_height;
                    if row_bottom < primary_plot.y || row_top > primary_plot_bottom {
                        continue;
                    }

                    minimum_row_height = Some(
                        minimum_row_height.map_or(row_height, |minimum| minimum.min(row_height)),
                    );

                    let [sell_chars, buy_chars] = footprint_text_content_lengths(
                        self.footprint.cluster_kind,
                        group.sell_qty.to_f64(),
                        group.buy_qty.to_f64(),
                    );
                    let row_max_chars = sell_chars.max(buy_chars);
                    if row_max_chars == 0 {
                        continue;
                    }

                    maximum_content_chars = maximum_content_chars.max(row_max_chars);
                }
            });
        }
        let text_mode = resolve_footprint_text_mode(
            footprint_width,
            imbalance.is_some(),
            minimum_row_height,
            maximum_content_chars,
        );
        let draw_candlesticks = self.candlestick.visible
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_));

        frame.with_clip(primary_plot, |frame| {
            self.for_each_scene_bar_unit_index(
                scene,
                |series_index, index, _series, x_unit, point| {
                    if series_index != 0 || !in_visible_range(x_unit) {
                        return;
                    }

                    let series_anchor = scene
                        .primary
                        .series_percent_anchors
                        .first()
                        .copied()
                        .flatten();
                    let y_open = scene.map_primary_plot_with_anchor_f32(
                        point.kline.open.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_high = scene.map_primary_plot_with_anchor_f32(
                        point.kline.high.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_low = scene.map_primary_plot_with_anchor_f32(
                        point.kline.low.to_f32_lossy(),
                        series_anchor,
                    );
                    let y_close = scene.map_primary_plot_with_anchor_f32(
                        point.kline.close.to_f32_lossy(),
                        series_anchor,
                    );
                    let footprint = point.footprint;
                    let x_center = scene.map_x_plot(x_unit);
                    let layout = FootprintColumnLayout::new(
                        x_center,
                        footprint_width,
                        imbalance.is_some() && !footprint.trades.is_empty(),
                    );

                    if let Some(rows) = pixel_rows.get(&index) {
                        let scale = self.footprint.cluster_scaling.effective_qty(
                            max_cluster_qty,
                            rows.max_qty(self.footprint.cluster_kind),
                        );
                        for (y, buy, sell) in rows.rows() {
                            draw_footprint_cluster_row(
                                frame,
                                self.footprint.cluster_kind,
                                layout,
                                y,
                                rows.bucket_height,
                                min_row_height,
                                sell.to_f64(),
                                buy.to_f64(),
                                scale,
                                FootprintTextMode::Hidden,
                                palette,
                                None,
                            );
                        }
                    } else if draw_clusters && !coalesce {
                        let cluster_scale = self.footprint.cluster_scaling.effective_qty(
                            max_cluster_qty,
                            footprint.max_cluster_qty_all(self.footprint.cluster_kind),
                        );
                        for (price, group) in &footprint.trades {
                            let row_height = self
                                .footprint_row_height(scene, *price, tick_size, anchor)
                                .max(min_row_height);
                            let y = scene
                                .map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                            if !text_mode.row_intersects_plot(
                                y,
                                row_height,
                                min_row_height,
                                primary_plot,
                            ) {
                                continue;
                            }
                            draw_footprint_cluster_row(
                                frame,
                                self.footprint.cluster_kind,
                                layout,
                                y,
                                row_height,
                                min_row_height,
                                group.sell_qty.to_f64(),
                                group.buy_qty.to_f64(),
                                cluster_scale,
                                text_mode,
                                palette,
                                imbalance.map(|(threshold_percent, ignore_zeros, color)| {
                                    FootprintImbalance::new(
                                        footprint,
                                        *price,
                                        tick_size,
                                        threshold_percent,
                                        ignore_zeros,
                                        color,
                                    )
                                }),
                            );
                        }
                    }

                    if draw_candlesticks {
                        draw_footprint_candle(
                            frame,
                            layout,
                            y_open,
                            y_high,
                            y_low,
                            y_close,
                            pixels,
                            self.candlestick_color(
                                point.kline.close >= point.kline.open,
                                palette,
                                generated_accents,
                            ),
                        );
                    }
                },
            );
        });
    }

    pub(super) fn draw_profile_distribution(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        distribution: &ProfileDistribution,
        colors: VolumeProfileColorConfig,
        profile_edge: f32,
        profile_width: f32,
        bar_alpha: f32,
        opacity_scale: f32,
        line_start: f32,
        naked_line_end: f32,
        grow_right: bool,
        pixels: PixelGeometry,
    ) {
        let tick_size = distribution.price_step;
        let max_qty = distribution.max_qty.to_f64();
        if max_qty <= 0.0 {
            return;
        }

        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let min_row_height = pixels.logical_width_from_physical(1);
        let sell_color = Self::resolve_theme_color(
            colors.color_for(VolumeProfileColorSlot::Sell),
            palette,
            generated_accents,
        )
        .scale_alpha(bar_alpha);
        let buy_color = Self::resolve_theme_color(
            colors.color_for(VolumeProfileColorSlot::Buy),
            palette,
            generated_accents,
        )
        .scale_alpha(bar_alpha);

        let plot = *scene.primary_plot();
        let body_left = if grow_right {
            profile_edge
        } else {
            profile_edge - profile_width
        };
        if profile_width > 0.0
            && body_left < plot.x + plot.width
            && body_left + profile_width > plot.x
        {
            let profile_width = profile_width.max(min_row_height);
            let rows = self
                .coalesce_price_rows(scene, tick_size, anchor, pixels)
                .then(|| {
                    let mut rows = PixelRows::new(*scene.primary_plot(), pixels);
                    for level in &distribution.levels {
                        rows.add(
                            scene.map_primary_plot_with_anchor_f32(
                                level.price.to_f32_lossy(),
                                anchor,
                            ),
                            level.positive_qty,
                            level.negative_qty,
                        );
                    }
                    rows
                });
            let max_qty = rows.as_ref().map_or(max_qty, |rows| {
                let coalesced_max = rows.max_qty(ClusterKind::VolumeProfile).to_f64();
                if coalesced_max > 0.0 {
                    coalesced_max
                } else {
                    max_qty
                }
            });
            let mut draw_row = |y: f32, row_height: f32, positive: Qty, negative: Qty| {
                let row_height = row_height.max(min_row_height);
                let row_top = y - row_height * 0.5;
                let row_bottom = row_top + row_height;
                if row_bottom < scene.primary_plot().y
                    || row_top > scene.primary_plot().y + scene.primary_plot().height
                {
                    return;
                }
                let total_qty = (positive + negative).to_f64();
                if total_qty <= 0.0 {
                    return;
                }

                let total_width = profile_width * (total_qty / max_qty).clamp(0.0, 1.0) as f32;
                if total_width <= 0.0 {
                    return;
                }

                let row_span = pixels.snapped_y_span(
                    row_top,
                    row_top + (row_height - min_row_height).max(min_row_height),
                );

                let left = if grow_right {
                    profile_edge
                } else {
                    profile_edge - total_width
                };
                let left_span = pixels.snapped_x_span(left, left + total_width);
                let negative_width =
                    left_span.size * (negative.to_f64() / total_qty).clamp(0.0, 1.0) as f32;
                let positive_width = left_span.size - negative_width;

                if negative_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left_span.start, row_span.start),
                        Size::new(negative_width, row_span.size),
                        sell_color,
                    );
                }
                if positive_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left_span.start + negative_width, row_span.start),
                        Size::new(positive_width, row_span.size),
                        buy_color,
                    );
                }
            };
            if let Some(rows) = rows {
                for (y, positive, negative) in rows.rows() {
                    draw_row(y, rows.bucket_height, positive, negative);
                }
            } else {
                for level in &distribution.levels {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(level.price.to_f32_lossy(), anchor);
                    draw_row(
                        y,
                        self.footprint_row_height(scene, level.price, tick_size, anchor),
                        level.positive_qty,
                        level.negative_qty,
                    );
                }
            }
        }

        let Some(poc) = distribution.poc else {
            return;
        };
        let (line_end, line_color) = match poc.status {
            NPoc::Naked if colors.show_npoc => (
                naked_line_end,
                Self::resolve_theme_color(
                    colors.color_for(VolumeProfileColorSlot::NakedPoc),
                    palette,
                    generated_accents,
                )
                .scale_alpha((0.7 * opacity_scale).min(1.0)),
            ),
            NPoc::Filled { at } if colors.show_poc => (
                scene
                    .x
                    .axis
                    .unit_from_time_f32(UnixMs::new(at))
                    .map(|unit| scene.map_x_plot_f32(unit))
                    .unwrap_or(naked_line_end),
                Self::resolve_theme_color(
                    colors.color_for(VolumeProfileColorSlot::FilledPoc),
                    palette,
                    generated_accents,
                )
                .scale_alpha((0.85 * opacity_scale).min(1.0)),
            ),
            _ => return,
        };
        if !line_start.is_finite() || !line_end.is_finite() {
            return;
        }

        let poc_y = scene.map_primary_plot_with_anchor_f32(poc.price.to_f32_lossy(), anchor);
        if !poc_y.is_finite() {
            return;
        }
        let Some(line) = pixels.positive_x_span(line_start, line_end) else {
            return;
        };
        let raw_line_height = self
            .footprint_row_height(scene, poc.price, tick_size, anchor)
            .abs()
            .max(min_row_height)
            .min(1.0_f32.max(min_row_height));
        let line_span =
            pixels.snapped_y_span(poc_y - raw_line_height * 0.5, poc_y + raw_line_height * 0.5);

        frame.fill_rectangle(
            Point::new(line.start, line_span.start),
            Size::new(line.size, line_span.size),
            line_color,
        );
    }

    pub(super) fn fill_footprint_poc_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if !self.footprint.visible {
            return;
        }

        let config = self.footprint.poc;
        if !config.show_poc && !config.show_npoc {
            return;
        }
        let Some(base_series) = self.series.first() else {
            return;
        };
        let point_count = base_series.point_count();
        let (earliest, latest) = match scene.x.axis {
            XAxis::Time { .. } => {
                let (Some(earliest), Some(latest)) = (
                    scene.x.axis.time_from_unit(scene.x.min_unit),
                    scene.x.axis.time_from_unit(scene.x.max_unit),
                ) else {
                    return;
                };
                (earliest, latest)
            }
            XAxis::Tick { .. } => {
                let to_forward_index = |unit: i64| {
                    scene
                        .x
                        .axis
                        .tick_from_unit(unit)
                        .map(|from_latest| {
                            point_count
                                .saturating_sub(1)
                                .saturating_sub(from_latest.as_u64() as usize)
                        })
                        .unwrap_or_default()
                };
                (
                    UnixMs::new(to_forward_index(scene.x.min_unit) as u64),
                    UnixMs::new(to_forward_index(scene.x.max_unit) as u64),
                )
            }
        };
        let lines = base_series.footprint_poc_lines(earliest, latest);
        if lines.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let tick_size = base_series.footprint_tick_size();
        let min_row_height = pixels.logical_width_from_physical(1);
        let naked_line_end = primary_plot.x + primary_plot.width;
        let npoc_color = Self::resolve_theme_color(
            config.colors.color_for(FootprintPocColorSlot::Npoc),
            palette,
            generated_accents,
        )
        .scale_alpha(0.7);
        let poc_color = Self::resolve_theme_color(
            config.colors.color_for(FootprintPocColorSlot::Poc),
            palette,
            generated_accents,
        )
        .scale_alpha(0.85);
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let x_for_time = |value: UnixMs| -> Option<f32> {
            scene
                .x
                .axis
                .unit_from_time_f32(value)
                .map(|unit| scene.map_x_plot_f32(unit))
        };
        let x_for_tick_from_latest = |value: UnixMs| -> Option<f32> {
            Some(
                scene
                    .x
                    .axis
                    .unit_from_tick(TickIndex::from_u64(value.as_u64())) as f32,
            )
            .map(|unit| scene.map_x_plot_f32(unit))
        };
        let x_for_tick_start = |value: UnixMs| -> Option<f32> {
            let index = usize::try_from(value.as_u64()).ok()?;
            let from_latest = point_count.checked_sub(index.checked_add(1)?)?;
            x_for_tick_from_latest(UnixMs::new(from_latest as u64))
        };
        frame.with_clip(primary_plot, |frame| {
            for line in lines {
                let start_x = match scene.x.axis {
                    XAxis::Time { .. } => x_for_time(line.start),
                    XAxis::Tick { .. } => x_for_tick_start(line.start),
                };
                let Some(start_x) = start_x else {
                    continue;
                };
                let (end_x, color) = match line.poc.status {
                    NPoc::Naked if config.show_npoc => (naked_line_end, npoc_color),
                    NPoc::Filled { at } if config.show_poc => {
                        let x = match scene.x.axis {
                            XAxis::Time { .. } => x_for_time(UnixMs::new(at)),
                            XAxis::Tick { .. } => x_for_tick_from_latest(UnixMs::new(at)),
                        };
                        (x.unwrap_or(naked_line_end), poc_color)
                    }
                    NPoc::Filled { .. } => continue,
                    NPoc::None => continue,
                    _ => continue,
                };
                if !start_x.is_finite() || !end_x.is_finite() {
                    continue;
                }

                let poc_y =
                    scene.map_primary_plot_with_anchor_f32(line.poc.price.to_f32_lossy(), anchor);
                if !poc_y.is_finite() {
                    continue;
                }
                let poc_price = line.poc.price;
                let Some(line) = pixels.positive_x_span(start_x, end_x) else {
                    continue;
                };
                let raw_line_height = self
                    .footprint_row_height(scene, poc_price, tick_size, anchor)
                    .abs()
                    .max(min_row_height)
                    .min(1.0_f32.max(min_row_height));
                let line_span = pixels
                    .snapped_y_span(poc_y - raw_line_height * 0.5, poc_y + raw_line_height * 0.5);
                frame.fill_rectangle(
                    Point::new(line.start, line_span.start),
                    Size::new(line.size, line_span.size),
                    color,
                );
            }
        });
    }

    fn fill_big_trades_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if !self.trade_data_enabled {
            return;
        }
        let Some(config) = self
            .studies
            .iter()
            .filter(|study| study.visible())
            .find_map(|study| study.big_trades())
        else {
            return;
        };
        let Some(series) = self.series.first() else {
            return;
        };
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let plot = *scene.primary_plot();

        let bar_spacing = scene.logical_bar_spacing();
        let col_merge = (BIG_TRADE_MIN_COL_PX / bar_spacing).ceil().max(1.0) as i64;
        let tick_size = series.footprint_tick_size();
        let (low, high) = scene.primary_domain_value_range();
        let target_rows = (plot.height / BIG_TRADE_MIN_ROW_PX).floor().max(1.0) as usize;
        let row_step = tick_size
            .coarsened_to_fit(low, high, target_rows)
            .unwrap_or(tick_size);
        let cell_width_px = bar_spacing * col_merge as f32;
        let cell_height_px = self.footprint_row_height(scene, low, row_step, anchor);
        let max_radius = (0.25 * cell_width_px)
            .min(0.5 * cell_height_px)
            .clamp(BIG_TRADE_MIN_RADIUS_PX, BIG_TRADE_MAX_RADIUS_PX);

        let mut groups = rustc_hash::FxHashMap::<(i64, i64, bool), BigTradeCell>::default();

        match (self.basis, scene.x.axis) {
            (data::chart::Basis::Time(timeframe), XAxis::Time { .. }) => {
                let Some(window_start) = scene.x.axis.time_from_unit(scene.x.min_unit) else {
                    return;
                };
                let Some(window_end) = scene
                    .x
                    .axis
                    .time_from_unit(scene.x.max_unit.saturating_add(1))
                else {
                    return;
                };
                for entry in series.big_trades_in_time_range(window_start, window_end) {
                    let x_unit = scene.x.axis.unit_from_time(entry.time.floor_to(timeframe));
                    if x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                        continue;
                    }
                    let y =
                        scene.map_primary_plot_with_anchor_f32(entry.price.to_f32_lossy(), anchor);
                    if !y.is_finite() || y < plot.y || y > plot.y + plot.height {
                        continue;
                    }
                    let row = entry.price.round_to_step(row_step);
                    let cell = groups
                        .entry((x_unit.div_euclid(col_merge), row.units, entry.is_sell))
                        .or_default();
                    cell.notional += entry.notional;
                    cell.count = cell.count.saturating_add(1);
                    cell.unit_sum += x_unit as f64;
                }
            }
            (data::chart::Basis::Tick(tick_count), XAxis::Tick { .. }) => {
                let point_count = series.point_count();
                let tick_count = usize::from(tick_count.0).max(1);
                let start_bar = self
                    .bar_index_at_or_before_unit(scene, 0, scene.x.min_unit)
                    .unwrap_or_default();
                let end_bar = self
                    .bar_index_at_or_before_unit(scene, 0, scene.x.max_unit)
                    .unwrap_or_else(|| point_count.saturating_sub(1))
                    .min(point_count.saturating_sub(1));
                let lo = start_bar.saturating_mul(tick_count);
                let hi = end_bar.saturating_add(1).saturating_mul(tick_count);
                for entry in series.big_trades_in_index_range(lo, hi) {
                    let bar_index = entry.index / tick_count;
                    let from_latest =
                        point_count.saturating_sub(1).saturating_sub(bar_index) as u64;
                    let x_unit = scene
                        .x
                        .axis
                        .unit_from_tick(TickIndex::from_u64(from_latest));
                    if x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                        continue;
                    }
                    let y =
                        scene.map_primary_plot_with_anchor_f32(entry.price.to_f32_lossy(), anchor);
                    if !y.is_finite() || y < plot.y || y > plot.y + plot.height {
                        continue;
                    }
                    let row = entry.price.round_to_step(row_step);
                    let cell_x = (bar_index as i64).div_euclid(col_merge);
                    let cell = groups
                        .entry((cell_x, row.units, entry.is_sell))
                        .or_default();
                    cell.notional += entry.notional;
                    cell.count = cell.count.saturating_add(1);
                    cell.unit_sum += x_unit as f64;
                }
            }
            _ => return,
        }

        if groups.is_empty() {
            return;
        }
        let max_notional = groups
            .values()
            .map(|cell| cell.notional)
            .fold(0.0_f64, f64::max);
        if max_notional <= 0.0 {
            return;
        }
        let buy_color = Self::resolve_theme_color(config.buy_color, palette, generated_accents);
        let sell_color = Self::resolve_theme_color(config.sell_color, palette, generated_accents);
        let stroke_width = pixels.quantized_stroke_width(1.0).0;

        let mut groups = groups.into_iter().collect::<Vec<_>>();
        if groups.len() > MAX_BIG_TRADE_BUBBLES {
            groups.sort_unstable_by(|left, right| right.1.notional.total_cmp(&left.1.notional));
            groups.truncate(MAX_BIG_TRADE_BUBBLES);
        }

        frame.with_clip(plot, |frame| {
            for ((_, row_units, is_sell), cell) in groups {
                let count = cell.count;
                let x_unit = (cell.unit_sum / f64::from(count)) as f32;
                let x = scene.map_x_plot_f32(x_unit);
                let y = scene.map_primary_plot_with_anchor_f32(
                    Price { units: row_units }.to_f32_lossy(),
                    anchor,
                );
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                let normalized = (cell.notional / max_notional).sqrt().clamp(0.0, 1.0) as f32;
                let radius = max_radius * (0.4 + 0.6 * normalized);
                let side_offset = if is_sell { -radius } else { radius };
                let center = Point::new(x + side_offset, y);
                let color = if is_sell { sell_color } else { buy_color };
                frame.fill(
                    &canvas::Path::circle(center, radius),
                    color.scale_alpha(0.8),
                );
                for ring in 1..count.min(3) {
                    frame.stroke(
                        &canvas::Path::circle(center, radius + 2.0 * ring as f32),
                        canvas::Stroke::default()
                            .with_color(color)
                            .with_width(stroke_width),
                    );
                }
            }
        });
    }

    fn effective_study_tick_size(
        &self,
        study: KlineStudy,
        footprint_tick_size: PriceStep,
    ) -> PriceStep {
        let Some(base_series) = self.series.first() else {
            return footprint_tick_size;
        };
        study.effective_profile_tick_size(base_series.ticker_info, footprint_tick_size)
    }

    pub(super) fn fill_volume_profile_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let Some(base_series) = self.series.first() else {
            return;
        };

        let Some(earliest) = scene.x.axis.time_from_unit(scene.x.min_unit) else {
            return;
        };
        let Some(latest) = scene.x.axis.time_from_unit(scene.x.max_unit) else {
            return;
        };

        let footprint_tick_size = base_series.footprint_tick_size();
        let profile_bars = self.profile_bars.collect(self.series);
        let visible_profile = self.studies.iter().find_map(|study| {
            if !study.visible() {
                return None;
            }

            if study.volume_profile_schedule()? != VolumeProfileSchedule::VisibleRange {
                return None;
            }

            let colors = study.volume_profile_colors()?;
            let width_ratio = study.volume_profile_width_ratio()?;
            let opacity_ratio = study.volume_profile_opacity_ratio()?;
            let opacity_scale = study.volume_profile_opacity_scale()?;
            let tick_size = self.effective_study_tick_size(*study, footprint_tick_size);
            let profile = match study.source() {
                StudyDataSource::Kline => data::chart::kline::kline_volume_profile(
                    &profile_bars,
                    earliest,
                    latest,
                    tick_size,
                ),
                StudyDataSource::Trades => {
                    base_series.trade_price_distribution(earliest, latest, tick_size)
                }
            };
            profile.map(|profile| (profile, colors, width_ratio, opacity_ratio, opacity_scale))
        });
        let recurring_intervals = self
            .studies
            .iter()
            .filter(|study| study.visible())
            .filter_map(|study| {
                let VolumeProfileSchedule::Recurring { interval } =
                    study.volume_profile_schedule()?
                else {
                    return None;
                };
                if !study.is_interval_available(self.basis) {
                    return None;
                }
                Some((
                    study.source(),
                    interval,
                    study.volume_profile_colors()?,
                    study.volume_profile_width_ratio()?,
                    study.volume_profile_opacity_ratio()?,
                    study.volume_profile_opacity_scale()?,
                    self.effective_study_tick_size(*study, footprint_tick_size),
                ))
            });
        let profiles = recurring_intervals
            .flat_map(
                |(
                    source,
                    interval,
                    colors,
                    width_ratio,
                    opacity_ratio,
                    opacity_scale,
                    tick_size,
                )| {
                    let profiles = match source {
                        StudyDataSource::Kline => {
                            data::chart::kline::recurring_kline_volume_profiles(
                                &profile_bars,
                                interval,
                                earliest,
                                latest,
                                tick_size,
                            )
                        }
                        StudyDataSource::Trades => base_series.recurring_trade_price_distributions(
                            interval, tick_size, earliest, latest,
                        ),
                    };
                    profiles.into_iter().map(move |profile| {
                        (profile, colors, width_ratio, opacity_ratio, opacity_scale)
                    })
                },
            )
            .collect::<Vec<_>>();
        if visible_profile.is_none() && profiles.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let min_profile_width = pixels.logical_width_from_physical(1);
        frame.with_clip(primary_plot, |frame| {
            if let Some((profile, colors, width_ratio, opacity_ratio, opacity_scale)) =
                visible_profile.as_ref()
            {
                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    profile,
                    *colors,
                    primary_plot.x + primary_plot.width,
                    primary_plot.width * width_ratio,
                    *opacity_ratio,
                    *opacity_scale,
                    primary_plot.x,
                    primary_plot.x + primary_plot.width,
                    false,
                    pixels,
                );
            }

            for (anchored_profile, colors, width_ratio, opacity_ratio, opacity_scale) in profiles {
                let Some(start_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.start)
                else {
                    continue;
                };
                let Some(end_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.end) else {
                    continue;
                };

                let segment_left = scene.map_x_plot_f32(start_unit);
                let segment_right = scene.map_x_plot_f32(end_unit);
                if !width_ratio.is_finite() || width_ratio <= 0.0 {
                    continue;
                }
                let segment_width = (segment_right - segment_left).abs();
                if !segment_width.is_finite() || segment_width <= 0.0 {
                    continue;
                }
                let profile_width = (segment_width * width_ratio).max(min_profile_width);

                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    &anchored_profile.distribution,
                    colors,
                    segment_left,
                    profile_width,
                    opacity_ratio,
                    opacity_scale,
                    segment_left,
                    primary_plot.x + primary_plot.width,
                    true,
                    pixels,
                );
            }
        });
    }

    fn tpo_row_height(scene: &Scene, profile: &TpoProfile, anchor: Option<f32>) -> f32 {
        let row_height = if profile.rows.len() < 2 {
            12.0
        } else {
            profile
                .rows
                .windows(2)
                .filter_map(|rows| {
                    let y1 = scene
                        .map_primary_plot_with_anchor_f32(rows[0].price.to_f32_lossy(), anchor);
                    let y2 = scene
                        .map_primary_plot_with_anchor_f32(rows[1].price.to_f32_lossy(), anchor);
                    (y1.is_finite() && y2.is_finite()).then_some((y1 - y2).abs())
                })
                .fold(f32::INFINITY, f32::min)
                .min(24.0)
        };

        if row_height.is_finite() {
            row_height
        } else {
            0.0
        }
    }

    pub(super) fn tpo_profile_geometries(
        &self,
        scene: &Scene,
        selected_profile: Option<UnixMs>,
    ) -> Vec<TpoProfileGeometry> {
        if self.tpo_profiles.is_empty() || !self.basis.is_time() {
            return Vec::new();
        }
        if !self
            .studies
            .iter()
            .any(|study| study.visible() && study.is_tpo_profile())
        {
            return Vec::new();
        }

        let plot = *scene.primary_plot();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let plot_right = plot.x + plot.width;
        let plot_bottom = plot.y + plot.height;

        self.tpo_profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                let start_unit = scene.x.axis.unit_from_time_f32(profile.start)?;
                let end_unit = scene.x.axis.unit_from_time_f32(profile.end)?;
                let left = scene.map_x_plot_f32(start_unit);
                let right = scene.map_x_plot_f32(end_unit);
                let (line_start, line_end) = (left.min(right), left.max(right));
                if line_end < plot.x || line_start > plot_right {
                    return None;
                }

                let row_height = Self::tpo_row_height(scene, profile, anchor);
                if row_height <= 0.0 || profile.rows.is_empty() {
                    return None;
                }

                let mut top = f32::INFINITY;
                let mut bottom = f32::NEG_INFINITY;
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if y.is_finite() {
                        top = top.min(y - row_height * 0.5);
                        bottom = bottom.max(y + row_height * 0.5);
                    }
                }
                if !top.is_finite() || !bottom.is_finite() {
                    return None;
                }

                let visible_left = line_start.max(plot.x);
                let visible_right = line_end.min(plot_right);
                let visible_top = top.max(plot.y);
                let visible_bottom = bottom.min(plot_bottom);
                if visible_right <= visible_left || visible_bottom <= visible_top {
                    return None;
                }

                let overlay_bounds = Rectangle {
                    x: scene.layout.regions.plot.x + visible_left,
                    y: scene.layout.regions.plot.y + visible_top,
                    width: visible_right - visible_left,
                    height: visible_bottom - visible_top,
                };
                let button_y = (overlay_bounds.y + TPO_ACTION_BUTTON_GAP_PX)
                    .min(scene.layout.regions.plot.y + plot_bottom - TPO_ACTION_BUTTON_HEIGHT_PX);
                let selected = selected_profile == Some(profile.start);
                let reset_visible = selected
                    && self
                        .tpo_edits
                        .has_boundary_in_range(profile.start, profile.end);
                let merge_with_reset_width = TPO_RESET_BUTTON_WIDTH_PX
                    + TPO_ACTION_BUTTON_WIDTH_PX
                    + TPO_ACTION_BUTTON_GAP_PX * 3.0;
                let button_bounds = (selected
                    && index > 0
                    && overlay_bounds.width
                        >= if reset_visible {
                            merge_with_reset_width
                        } else {
                            TPO_ACTION_BUTTON_WIDTH_PX + TPO_ACTION_BUTTON_GAP_PX * 2.0
                        })
                .then_some(Rectangle {
                    x: overlay_bounds.x + overlay_bounds.width
                        - TPO_ACTION_BUTTON_WIDTH_PX
                        - TPO_ACTION_BUTTON_GAP_PX,
                    y: button_y,
                    width: TPO_ACTION_BUTTON_WIDTH_PX,
                    height: TPO_ACTION_BUTTON_HEIGHT_PX,
                });
                let reset_button_bounds = (selected
                    && reset_visible
                    && overlay_bounds.width
                        >= TPO_RESET_BUTTON_WIDTH_PX + TPO_ACTION_BUTTON_GAP_PX * 2.0)
                    .then(|| {
                        let x = button_bounds.map_or(
                            overlay_bounds.x + overlay_bounds.width
                                - TPO_RESET_BUTTON_WIDTH_PX
                                - TPO_ACTION_BUTTON_GAP_PX,
                            |button| {
                                button.x - TPO_ACTION_BUTTON_GAP_PX - TPO_RESET_BUTTON_WIDTH_PX
                            },
                        );
                        Rectangle {
                            x,
                            y: button_y,
                            width: TPO_RESET_BUTTON_WIDTH_PX,
                            height: TPO_ACTION_BUTTON_HEIGHT_PX,
                        }
                    });

                Some(TpoProfileGeometry {
                    index,
                    start: profile.start,
                    end: profile.end,
                    overlay_bounds,
                    button_bounds,
                    reset_button_bounds,
                })
            })
            .collect()
    }

    pub(super) fn fill_tpo_profile_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if self.tpo_profiles.is_empty() || !self.basis.is_time() {
            return;
        }
        let Some(tpo_study) = self
            .studies
            .iter()
            .find(|study| study.visible() && study.is_tpo_profile())
        else {
            return;
        };
        let plot = *scene.primary_plot();
        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let line_width = pixels.quantized_stroke_width(1.0).0;
        let colors = tpo_study
            .tpo_colors()
            .unwrap_or_else(TpoColorConfig::defaults);

        frame.with_clip(plot, |frame| {
            for profile in self.tpo_profiles {
                let Some(start_unit) = scene.x.axis.unit_from_time_f32(profile.start) else {
                    continue;
                };
                let Some(end_unit) = scene.x.axis.unit_from_time_f32(profile.end) else {
                    continue;
                };
                let left = scene.map_x_plot_f32(start_unit);
                let right = scene.map_x_plot_f32(end_unit);
                let (line_start, line_end) = (left.min(right), left.max(right));
                if line_end < plot.x || line_start > plot.x + plot.width {
                    continue;
                }
                let profile_width = (line_end - line_start).max(0.0);
                let row_height = Self::tpo_row_height(scene, profile, anchor);
                let column_width = profile.period_count.max(1) as f32;
                let column_width = profile_width / column_width;
                let show_letters = column_width >= 8.0 && row_height >= 8.0;
                let status_alpha = match profile.status {
                    TpoProfileStatus::Complete => 0.95,
                    TpoProfileStatus::Developing => 0.8,
                    TpoProfileStatus::Incomplete => 0.55,
                };
                let color = |configured| {
                    Self::resolve_theme_color(configured, palette, generated_accents)
                        .scale_alpha(status_alpha)
                };
                let is_single_print = |price: Price| profile.single_prints.contains(&price);
                let poor_color = |price: Price| {
                    if colors.show_poor_high && profile.poor_highs.contains(&price) {
                        Some(colors.poor_high)
                    } else if colors.show_poor_low && profile.poor_lows.contains(&price) {
                        Some(colors.poor_low)
                    } else {
                        None
                    }
                };
                let row_color = |price: Price| {
                    if colors.show_poc && profile.poc == Some(price) {
                        colors.poc
                    } else if profile.val.is_some_and(|val| price >= val)
                        && profile.vah.is_some_and(|vah| price <= vah)
                    {
                        colors.value_area
                    } else {
                        colors.letters
                    }
                };
                let cell_color = |price: Price| {
                    if !show_letters {
                        if let Some(configured) = poor_color(price) {
                            configured
                        } else if colors.show_single_print && is_single_print(price) {
                            colors.single_print
                        } else {
                            row_color(price)
                        }
                    } else {
                        row_color(price)
                    }
                };
                let text_color = |price: Price| {
                    poor_color(price).unwrap_or_else(|| {
                        if colors.show_single_print && is_single_print(price) {
                            colors.single_print
                        } else {
                            row_color(price)
                        }
                    })
                };

                let cell_gap = pixels.logical_width_from_physical(1);
                let plot_right = plot.x + plot.width;
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        continue;
                    }
                    let y_span = pixels.snapped_y_span(y - row_height * 0.5, y + row_height * 0.5);
                    let cell_y = PixelSpan {
                        start: y_span.start + cell_gap * 0.5,
                        size: (y_span.size - cell_gap).max(cell_gap),
                    };
                    let configured = cell_color(row.price);
                    let cell_alpha = if !show_letters
                        && (poor_color(row.price).is_some()
                            || (colors.show_single_print && is_single_print(row.price)))
                        || (colors.show_poc && profile.poc == Some(row.price))
                    {
                        0.38
                    } else if profile.val.is_some_and(|val| row.price >= val)
                        && profile.vah.is_some_and(|vah| row.price <= vah)
                    {
                        0.26
                    } else {
                        0.18
                    };
                    for (column_index, period_index) in row.periods.iter().enumerate() {
                        if usize::from(*period_index) >= profile.period_count {
                            continue;
                        }
                        let x_start = line_start + column_width * column_index as f32;
                        let x_end = x_start + column_width;
                        let Some(x_span) =
                            pixels.positive_x_span_in_bounds(x_start, x_end, plot.x, plot_right)
                        else {
                            continue;
                        };
                        let cell_x = PixelSpan {
                            start: x_span.start + cell_gap * 0.5,
                            size: (x_span.size - cell_gap).max(cell_gap),
                        };
                        frame.fill_rectangle(
                            Point::new(cell_x.start, cell_y.start),
                            Size::new(cell_x.size, cell_y.size),
                            color(configured).scale_alpha(cell_alpha),
                        );
                    }
                }
                let line = |frame: &mut canvas::Frame,
                            price: Option<Price>,
                            configured: data::config::theme::ThemeColor,
                            width: f32| {
                    let Some(price) = price else { return };
                    let y = scene.map_primary_plot_with_anchor_f32(price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        return;
                    }
                    frame.stroke(
                        &canvas::Path::line(Point::new(line_start, y), Point::new(line_end, y)),
                        canvas::Stroke::default()
                            .with_width(width)
                            .with_color(color(configured)),
                    );
                };

                line(frame, profile.vah, colors.value_area, line_width);
                line(frame, profile.val, colors.value_area, line_width);
                if colors.show_poc {
                    line(frame, profile.poc, colors.poc, line_width.max(1.5));
                }
                line(frame, profile.session_high, colors.letters, line_width);
                line(frame, profile.session_low, colors.letters, line_width);
                if colors.show_initial_balance {
                    line(
                        frame,
                        profile.initial_balance_high,
                        colors.initial_balance,
                        line_width,
                    );
                    line(
                        frame,
                        profile.initial_balance_low,
                        colors.initial_balance,
                        line_width,
                    );
                }

                if !show_letters {
                    continue;
                }
                for row in &profile.rows {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(row.price.to_f32_lossy(), anchor);
                    if !y.is_finite() {
                        continue;
                    }
                    for (column_index, period_index) in row.periods.iter().enumerate() {
                        if usize::from(*period_index) >= profile.period_count {
                            continue;
                        }
                        let x = line_start + column_width * (column_index as f32 + 0.5);
                        frame.fill_text(canvas::Text {
                            content: TpoProfile::letter_label(usize::from(*period_index)),
                            position: Point::new(x, y),
                            color: color(text_color(row.price)),
                            size: row_height.min(column_width * 0.8).max(8.0).into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                    }
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_indicator_ohlc_output(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        value_id: PanelValueId,
        output: &IndicatorOutputRef,
        required_input: Option<IndicatorInputSource>,
        map_y: impl Fn(f64) -> Option<f32>,
        body_width_phys: i32,
        wick_width_phys: i32,
        pixels: PixelGeometry,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let body_width = pixels.logical_width_from_physical(body_width_phys);
        let wick_width = pixels.logical_width_from_physical(wick_width_phys);
        let line_width = pixels.quantized_stroke_width(1.0);
        self.for_each_scene_indicator_output_index(
            scene,
            output,
            |series, source_index, x_unit| {
                if !self.indicator_input_available_at(series, source_index, required_input) {
                    return;
                }
                let Some(ohlc) =
                    self.indicator_ohlc_output_for_series_at(series, output, source_index)
                else {
                    return;
                };
                let (Some(y_open), Some(y_high), Some(y_low), Some(y_close)) = (
                    ohlc.open.and_then(&map_y),
                    ohlc.high.and_then(&map_y),
                    ohlc.low.and_then(&map_y),
                    ohlc.close.and_then(&map_y),
                ) else {
                    return;
                };
                if [y_open, y_high, y_low, y_close]
                    .into_iter()
                    .any(|value| !value.is_finite())
                {
                    return;
                }
                let x = scene.map_x_plot(x_unit);
                if !x.is_finite() {
                    return;
                }
                let body_left = pixels.centered_left(x, body_width_phys);
                let wick_left = pixels.centered_left(x, wick_width_phys);
                let body = pixels.snapped_y_span(y_open, y_close);
                let wick = pixels.snapped_y_span(y_high, y_low);
                let wick_x = pixels.snap_x_for_stroke(x, line_width.1);
                let color = ohlc
                    .color
                    .map(Self::resolve_indicator_color)
                    .unwrap_or_else(|| {
                        self.indicator_color(
                            Some(value_id),
                            self.indicator_style_slot_for_output(output),
                            palette,
                            generated_accents,
                        )
                    });
                frame.stroke(
                    &canvas::Path::line(
                        Point::new(wick_x, wick.start),
                        Point::new(wick_x, wick.start + wick.size),
                    ),
                    indicator_line_stroke(line_width.0, LineStrokeStyle::Solid, color),
                );
                frame.fill_rectangle(
                    Point::new(body_left, body.start),
                    Size::new(body_width, body.size),
                    color,
                );
                if body_width <= wick_width {
                    frame.fill_rectangle(
                        Point::new(wick_left, body.start),
                        Size::new(wick_width, body.size),
                        color,
                    );
                }
            },
        );
    }

    fn draw_indicator_range_output(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        output: &IndicatorOutputRef,
        required_input: Option<IndicatorInputSource>,
        map_y: impl Fn(f64) -> Option<f32>,
        pixels: PixelGeometry,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let mut segments: Vec<(Vec<Point>, Vec<Point>, iced::Color)> = Vec::new();
        let mut upper = Vec::new();
        let mut lower = Vec::new();
        let mut color = None;
        let flush = |segments: &mut Vec<(Vec<Point>, Vec<Point>, iced::Color)>,
                     upper: &mut Vec<Point>,
                     lower: &mut Vec<Point>,
                     color: &mut Option<iced::Color>| {
            if !upper.is_empty() && !lower.is_empty() {
                segments.push((
                    std::mem::take(upper),
                    std::mem::take(lower),
                    color.unwrap_or(iced::Color::TRANSPARENT),
                ));
            } else {
                upper.clear();
                lower.clear();
            }
            *color = None;
        };

        self.for_each_scene_indicator_output_index(scene, output, |_, source_index, x_unit| {
            if !self.indicator_input_available_at(series, source_index, required_input) {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            }
            let Some(range) =
                self.indicator_range_output_for_series_at(series, output, source_index)
            else {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            };
            let (Some(upper_value), Some(lower_value)) = (range.upper, range.lower) else {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            };
            let (Some(upper_y), Some(lower_y)) = (map_y(upper_value), map_y(lower_value)) else {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            };
            if !upper_y.is_finite() || !lower_y.is_finite() {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            }
            let point_color = range
                .color
                .map(Self::resolve_indicator_color)
                .unwrap_or_else(|| {
                    self.indicator_color_for_output_at(
                        series,
                        source_index,
                        output,
                        None,
                        self.indicator_style_slot_for_output(output),
                        ThemeColorRole::Neutral,
                        palette,
                        generated_accents,
                    )
                });
            if color.is_some_and(|current| current != point_color) {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
            }
            color = Some(point_color);
            let x = scene.map_x_plot(x_unit);
            if !x.is_finite() {
                flush(&mut segments, &mut upper, &mut lower, &mut color);
                return;
            }
            let point = Point::new(x, upper_y);
            upper.push(point);
            lower.push(Point::new(point.x, lower_y));
        });
        flush(&mut segments, &mut upper, &mut lower, &mut color);

        for (upper, lower, color) in segments {
            let (min_x, max_x, min_y, max_y) = upper.iter().chain(&lower).fold(
                (
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                ),
                |(min_x, max_x, min_y, max_y), point| {
                    (
                        min_x.min(point.x),
                        max_x.max(point.x),
                        min_y.min(point.y),
                        max_y.max(point.y),
                    )
                },
            );
            if !min_x.is_finite() || !max_x.is_finite() || !min_y.is_finite() || !max_y.is_finite()
            {
                continue;
            }
            let min_size = pixels.logical_width_from_physical(1);
            if max_x - min_x < min_size || max_y - min_y < min_size {
                let Some(x) = pixels.positive_x_span(min_x, max_x) else {
                    continue;
                };
                let Some(y) = pixels.positive_y_span(min_y, max_y) else {
                    continue;
                };
                frame.fill_rectangle(
                    Point::new(x.start, y.start),
                    Size::new(x.size, y.size),
                    color.scale_alpha(0.25),
                );
                continue;
            }
            let path = canvas::Path::new(|builder| {
                builder.move_to(upper[0]);
                for point in upper.iter().skip(1) {
                    builder.line_to(*point);
                }
                for point in lower.iter().rev() {
                    builder.line_to(*point);
                }
                builder.close();
            });
            frame.fill(&path, color.scale_alpha(0.25));
        }
    }

    fn draw_indicator_regions(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        value_id: PanelValueId,
        map_y: impl Fn(f64) -> Option<f32>,
        pixels: PixelGeometry,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let Some(units) = scene.bar_units.first() else {
            return;
        };
        let point_count = series.point_count();
        let (visible_start, visible_end) =
            bar_index_range_for_units(&units.values, scene.x.min_unit, scene.x.max_unit);
        let visible_start = units
            .global_index(visible_start)
            .unwrap_or(point_count)
            .min(point_count);
        let visible_end = units
            .global_index(visible_end)
            .unwrap_or(point_count)
            .min(point_count);
        let visible_range = indicator_region_source_range(point_count, visible_start, visible_end);
        let plot = *scene.primary_plot();
        for region in self.indicator_regions(value_id, visible_range) {
            let Some(left_unit) = self.bar_unit_at_series_index(scene, 0, region.left_index) else {
                continue;
            };
            let right_x = if region.extend_right {
                plot.x
            } else {
                let Some(right_unit) = self.bar_unit_at_series_index(scene, 0, region.right_index)
                else {
                    continue;
                };
                scene.map_x_plot(right_unit)
            };
            let Some(rect) = indicator_region_rect(
                plot,
                scene.map_x_plot(left_unit),
                right_x,
                map_y(region.upper).unwrap_or(f32::NAN),
                map_y(region.lower).unwrap_or(f32::NAN),
                region.extend_right,
                pixels,
            ) else {
                continue;
            };
            let fill_color = region
                .fill_color
                .map(Self::resolve_indicator_color)
                .or_else(|| {
                    region.fill_color_slot.as_deref().map(|slot| {
                        self.indicator_color(Some(value_id), slot, palette, generated_accents)
                            .scale_alpha(0.2)
                    })
                });
            if let Some(fill_color) = fill_color {
                frame.fill_rectangle(
                    Point::new(rect.x, rect.y),
                    Size::new(rect.width, rect.height),
                    fill_color,
                );
            }
            let border_color = region
                .border_color
                .map(Self::resolve_indicator_color)
                .or_else(|| {
                    region.border_color_slot.as_deref().map(|slot| {
                        self.indicator_color(Some(value_id), slot, palette, generated_accents)
                    })
                });
            if let Some(border_color) = border_color {
                let line_width = pixels
                    .quantized_stroke_width(region.width.max(pixels.logical_width_from_physical(1)))
                    .0;
                frame.stroke(
                    &canvas::Path::rectangle(
                        Point::new(rect.x, rect.y),
                        Size::new(rect.width, rect.height),
                    ),
                    indicator_line_stroke(line_width, LineStrokeStyle::Solid, border_color),
                );
            }
        }
    }

    fn draw_indicator_markers_and_levels(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        value_id: PanelValueId,
        map_y: impl Fn(f64) -> Option<f32>,
        pixels: PixelGeometry,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        self.draw_indicator_segments(
            frame,
            scene,
            value_id,
            &map_y,
            pixels,
            palette,
            generated_accents,
        );
        self.for_each_scene_bar_unit_index(
            scene,
            |series_index, bar_index, series, x_unit, _point| {
                if series_index != 0 {
                    return;
                }
                for marker in self.indicator_markers_for_series_at(series, value_id, bar_index) {
                    let Some(value) = marker.value.filter(|value| value.is_finite()) else {
                        continue;
                    };
                    let Some(y) = map_y(value) else {
                        continue;
                    };
                    let color = marker
                        .color
                        .map(Self::resolve_indicator_color)
                        .unwrap_or_else(|| {
                            self.indicator_color(
                                Some(value_id),
                                self.indicator_default_style_slot(value_id),
                                palette,
                                generated_accents,
                            )
                        });
                    frame.fill(
                        &canvas::Path::circle(Point::new(scene.map_x_plot(x_unit), y), 3.0),
                        color,
                    );
                    if let Some(text) = marker.text {
                        frame.fill_text(canvas::Text {
                            content: text,
                            position: Point::new(scene.map_x_plot(x_unit), y - 8.0),
                            color,
                            size: INDICATOR_MARKER_TEXT_SIZE_PX.into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                    }
                }
            },
        );
        let line_width = pixels.quantized_stroke_width(1.0);
        let plot = scene.plot_rect();
        for level in self.indicator_levels(value_id) {
            let Some(value) = level.value.filter(|value| value.is_finite()) else {
                continue;
            };
            let Some(y) = map_y(value) else {
                continue;
            };
            let color = level
                .color
                .map(Self::resolve_indicator_color)
                .unwrap_or_else(|| {
                    self.indicator_color(
                        Some(value_id),
                        self.indicator_default_style_slot(value_id),
                        palette,
                        generated_accents,
                    )
                });
            let path =
                canvas::Path::line(Point::new(plot.x, y), Point::new(plot.x + plot.width, y));
            frame.stroke(
                &path,
                indicator_line_stroke(line_width.0, LineStrokeStyle::Dashed, color),
            );
        }
    }

    fn draw_indicator_segments(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        value_id: PanelValueId,
        map_y: &impl Fn(f64) -> Option<f32>,
        pixels: PixelGeometry,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let Some(series) = self.series.first() else {
            return;
        };
        let start = self
            .bar_index_at_or_before_unit(scene, 0, scene.x.min_unit)
            .unwrap_or(0);
        let end = self
            .bar_index_at_or_before_unit(scene, 0, scene.x.max_unit)
            .map_or(0, |index| index.saturating_add(1))
            .saturating_add(1)
            .min(series.point_count());
        let source_index_to_unit =
            |source_index: usize| self.bar_unit_at_series_index(scene, 0, source_index);
        for segment in self.indicator_segments(value_id, start..end, scene) {
            let Some(start_value) = segment.start_value.filter(|value| value.is_finite()) else {
                continue;
            };
            let Some(end_value) = segment.end_value.filter(|value| value.is_finite()) else {
                continue;
            };
            let Some(start_unit) = source_index_to_unit(segment.start_index) else {
                continue;
            };
            let Some(end_unit) = source_index_to_unit(segment.end_index) else {
                continue;
            };
            let Some(start_y) = map_y(start_value) else {
                continue;
            };
            let Some(end_y) = map_y(end_value) else {
                continue;
            };
            let segment_spec = self.indicator_segment_spec(value_id, &segment.id);
            let (line_width, stroke_style, fallback_color) = if let Some(spec) = segment_spec {
                (
                    pixels
                        .quantized_stroke_width(self.indicator_line_width_for_style_slot(
                            value_id,
                            &spec.color_slot,
                            spec.line_width,
                        ))
                        .0,
                    self.indicator_stroke_style_for_style_slot(value_id, &spec.color_slot),
                    self.indicator_color(
                        Some(value_id),
                        &spec.color_slot,
                        palette,
                        generated_accents,
                    ),
                )
            } else {
                (
                    pixels.quantized_stroke_width(1.4).0,
                    LineStrokeStyle::Solid,
                    self.indicator_color(
                        Some(value_id),
                        self.indicator_default_style_slot(value_id),
                        palette,
                        generated_accents,
                    ),
                )
            };
            let color = segment
                .color
                .map(Self::resolve_indicator_color)
                .unwrap_or(fallback_color);
            let start = Point::new(scene.map_x_plot(start_unit), start_y);
            let end = Point::new(scene.map_x_plot(end_unit), end_y);
            frame.stroke(
                &canvas::Path::line(start, end),
                indicator_line_stroke(line_width, stroke_style, color),
            );
            frame.fill(&canvas::Path::circle(start, 3.0), color);
            frame.fill(&canvas::Path::circle(end, 3.0), color);
            if let Some(text) = segment.text {
                let text_offset = if text.contains("Bull") { -8.0 } else { 8.0 };
                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(end.x, end.y + text_offset),
                    color,
                    size: INDICATOR_MARKER_TEXT_SIZE_PX.into(),
                    align_x: iced::Alignment::Center.into(),
                    align_y: iced::Alignment::Center.into(),
                    font: style::AZERET_MONO,
                    ..Default::default()
                });
            }
        }
    }

    fn band_segments(
        &self,
        scene: &Scene,
        value_id: PanelValueId,
        fill: &IndicatorFillSpec,
        mut map_y: impl FnMut(f64) -> Option<f32>,
    ) -> Vec<BandSegment> {
        let mut segments = Vec::new();
        let mut segment = Vec::new();
        let mut segment_style = None;

        let Some(series) = self.series.first() else {
            return segments;
        };
        let Some(units) = scene.bar_units.first() else {
            return segments;
        };
        let data::chart::kline::IndicatorOverlaySource::Channel(upper_channel) = &fill.upper;
        let data::chart::kline::IndicatorOverlaySource::Channel(lower_channel) = &fill.lower;
        let effective_offset = |channel_id: &str, explicit: i32| {
            if explicit != 0 {
                i64::from(explicit)
            } else {
                self.indicator_output_spec_for_channel(value_id, channel_id)
                    .map(|spec| i64::from(spec.offset_bars))
                    .unwrap_or_default()
            }
        };
        let upper_offset = effective_offset(upper_channel, fill.upper_offset_bars);
        let lower_offset = effective_offset(lower_channel, fill.lower_offset_bars);
        let mut points = BTreeMap::<i64, (Option<f64>, Option<f64>, Option<bool>)>::new();
        let style_channel = fill.style_by.as_deref();
        let end = series
            .point_count()
            .saturating_sub(units.start_index)
            .min(units.values.len());
        for (source_index, &unit) in units.values.iter().enumerate().take(end) {
            let Some(source_index) = units.global_index(source_index) else {
                continue;
            };
            let upper_unit = shifted_output_unit(scene.x.axis, unit, upper_offset);
            if upper_unit >= scene.x.min_unit
                && upper_unit <= scene.x.max_unit
                && let Some(value) = self
                    .indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        upper_channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_number)
            {
                let style = style_channel.and_then(|channel| {
                    self.indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_boolean)
                });
                let point = points.entry(upper_unit).or_default();
                point.0 = Some(value);
                point.2 = style;
            }
            let lower_unit = shifted_output_unit(scene.x.axis, unit, lower_offset);
            if lower_unit >= scene.x.min_unit
                && lower_unit <= scene.x.max_unit
                && let Some(value) = self
                    .indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        lower_channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_number)
            {
                let style = style_channel.and_then(|channel| {
                    self.indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_boolean)
                });
                let point = points.entry(lower_unit).or_default();
                point.1 = Some(value);
                if point.2.is_none() {
                    point.2 = style;
                }
            }
        }

        let flush = |segments: &mut Vec<BandSegment>, segment: &mut Vec<BandPoint>, style| {
            if !segment.is_empty() {
                segments.push(BandSegment {
                    points: std::mem::take(segment),
                    style_by: style,
                });
            }
        };
        for (x_unit, (upper_value, lower_value, style)) in points {
            let (Some(upper_value), Some(lower_value)) = (upper_value, lower_value) else {
                flush(&mut segments, &mut segment, segment_style);
                segment_style = None;
                continue;
            };
            let (Some(upper_y), Some(lower_y)) = (map_y(upper_value), map_y(lower_value)) else {
                flush(&mut segments, &mut segment, segment_style);
                segment_style = None;
                continue;
            };
            if !upper_y.is_finite() || !lower_y.is_finite() {
                flush(&mut segments, &mut segment, segment_style);
                segment_style = None;
                continue;
            }
            if !segment.is_empty() && segment_style != style {
                flush(&mut segments, &mut segment, segment_style);
            }
            segment_style = style;
            segment.push(BandPoint {
                upper: Point::new(scene.map_x_plot(x_unit), upper_y),
                lower: Point::new(scene.map_x_plot(x_unit), lower_y),
            });
        }
        flush(&mut segments, &mut segment, segment_style);
        segments
    }

    fn indicator_fill_color(
        &self,
        value_id: PanelValueId,
        fill: &IndicatorFillSpec,
        style_by: Option<bool>,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> iced::Color {
        let data::chart::kline::IndicatorOverlaySource::Channel(upper_channel) = &fill.upper;
        let style_slot = style_by.map_or_else(
            || fill.color_slot.as_str(),
            |state| match (
                fill.true_color_slot.as_deref(),
                fill.false_color_slot.as_deref(),
            ) {
                (Some(true_slot), Some(false_slot)) => {
                    if state {
                        true_slot
                    } else {
                        false_slot
                    }
                }
                _ => self.style_slot_for_output(
                    value_id,
                    upper_channel,
                    Some(state),
                    &fill.color_slot,
                ),
            },
        );
        self.indicator_color(Some(value_id), style_slot, palette, generated_accents)
            .scale_alpha(fill.opacity.clamp(0.0, 1.0))
    }

    fn fill_indicator_panel_background(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        indicator_panel: &IndicatorPanelScene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if self
            .panel_indicator_value_ids(indicator_panel.panel_index)
            .iter()
            .all(|value_id| self.indicator_is_unavailable(Some(*value_id)))
        {
            return;
        }
        let Some(panel_plot) = scene
            .layout
            .panel(indicator_panel.panel_index)
            .map(|panel| panel.plot)
        else {
            return;
        };
        for value_id in self.panel_indicator_value_ids(indicator_panel.panel_index) {
            if self.indicator_is_unavailable(Some(value_id))
                || !self.indicator_fill_visible(value_id)
            {
                continue;
            }
            for fill in self.fill_specs_for_panel_value(Some(value_id)).iter() {
                let color =
                    self.indicator_fill_color(value_id, fill, None, palette, generated_accents);
                frame.with_clip(panel_plot, |frame| match fill.geometry {
                    IndicatorFillGeometry::PanelBand => {
                        let Some(upper_value) = self
                            .indicator_source_value_for_latest_bar_with_offset(
                                value_id,
                                match &fill.upper {
                                    data::chart::kline::IndicatorOverlaySource::Channel(
                                        channel,
                                    ) => channel,
                                },
                                fill.upper_offset_bars,
                            )
                        else {
                            return;
                        };
                        let Some(lower_value) = self
                            .indicator_source_value_for_latest_bar_with_offset(
                                value_id,
                                match &fill.lower {
                                    data::chart::kline::IndicatorOverlaySource::Channel(
                                        channel,
                                    ) => channel,
                                },
                                fill.lower_offset_bars,
                            )
                        else {
                            return;
                        };
                        let Some(upper_y) =
                            scene.map_indicator_plot(indicator_panel.panel_index, upper_value)
                        else {
                            return;
                        };
                        let Some(lower_y) =
                            scene.map_indicator_plot(indicator_panel.panel_index, lower_value)
                        else {
                            return;
                        };
                        fill_static_band(frame, panel_plot, upper_y, lower_y, color, pixels);
                    }
                    IndicatorFillGeometry::SeriesBand => {
                        let segments = self.band_segments(scene, value_id, fill, |value| {
                            scene.map_indicator_plot(indicator_panel.panel_index, value)
                        });
                        fill_band_segments(frame, segments, pixels, |style| {
                            self.indicator_fill_color(
                                value_id,
                                fill,
                                style,
                                palette,
                                generated_accents,
                            )
                        });
                    }
                });
            }
        }
    }

    fn fill_primary_overlay_backgrounds(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return;
        };
        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return;
        };
        let primary_plot = *scene.primary_plot();
        let base_series_anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        frame.with_clip(primary_plot, |frame| {
            let mut filled_indicators = FxHashSet::default();
            for layer in &primary_panel.layers {
                if !layer.visible {
                    continue;
                }
                let Some(value_id) = layer
                    .source
                    .indicator_output()
                    .map(|output| output.instance_id)
                else {
                    continue;
                };
                if self.indicator_is_unavailable(Some(value_id))
                    || !self.indicator_fill_visible(value_id)
                    || !filled_indicators.insert(value_id)
                {
                    continue;
                }
                for fill in self.fill_specs_for_panel_value(Some(value_id)).iter() {
                    let color =
                        self.indicator_fill_color(value_id, fill, None, palette, generated_accents);
                    match fill.geometry {
                        IndicatorFillGeometry::PanelBand => {
                            let Some(upper_value) = self
                                .indicator_source_value_for_latest_bar_with_offset(
                                    value_id,
                                    match &fill.upper {
                                        data::chart::kline::IndicatorOverlaySource::Channel(
                                            channel,
                                        ) => channel,
                                    },
                                    fill.upper_offset_bars,
                                )
                            else {
                                continue;
                            };
                            let Some(lower_value) = self
                                .indicator_source_value_for_latest_bar_with_offset(
                                    value_id,
                                    match &fill.lower {
                                        data::chart::kline::IndicatorOverlaySource::Channel(
                                            channel,
                                        ) => channel,
                                    },
                                    fill.lower_offset_bars,
                                )
                            else {
                                continue;
                            };
                            let Some(upper_y) =
                                scene.map_primary_plot_with_anchor(upper_value, base_series_anchor)
                            else {
                                continue;
                            };
                            let Some(lower_y) =
                                scene.map_primary_plot_with_anchor(lower_value, base_series_anchor)
                            else {
                                continue;
                            };
                            fill_static_band(frame, primary_plot, upper_y, lower_y, color, pixels);
                        }
                        IndicatorFillGeometry::SeriesBand => {
                            let segments = self.band_segments(scene, value_id, fill, |value| {
                                scene.map_primary_plot_with_anchor(value, base_series_anchor)
                            });
                            fill_band_segments(frame, segments, pixels, |style| {
                                self.indicator_fill_color(
                                    value_id,
                                    fill,
                                    style,
                                    palette,
                                    generated_accents,
                                )
                            });
                        }
                    }
                }
            }
        });
    }

    pub(super) fn table_column_value(
        &self,
        series: &KlineSeries,
        value_id: PanelValueId,
        bar_index: usize,
        column: &TableColumnValue,
    ) -> Option<OutputValue> {
        match column {
            TableColumnValue::Channel(channel) => self
                .indicator_output_value_for_series_at_channel(series, value_id, channel, bar_index),
        }
    }

    pub(super) fn format_table_value(value: OutputValue, format: TableColumnFormat) -> String {
        match (value, format) {
            (OutputValue::Number(value), TableColumnFormat::Quantity) if value.is_finite() => {
                abbr_large_numbers(value)
            }
            (OutputValue::Number(value), TableColumnFormat::Integer) if value.is_finite() => {
                format!("{value:.0}")
            }
            (OutputValue::Number(value), TableColumnFormat::Percent) if value.is_finite() => {
                format!("{value:+.1}%")
            }
            (OutputValue::Boolean(true), TableColumnFormat::Boolean) => "✓".to_owned(),
            (OutputValue::Boolean(false), TableColumnFormat::Boolean) => String::new(),
            _ => String::new(),
        }
    }

    pub(super) fn table_value_color(
        &self,
        color: &TableColumnColor,
        value: OutputValue,
        value_id: PanelValueId,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) -> Option<iced::Color> {
        let color_slot = table_color_slot(color, value)?;
        Some(self.indicator_color(Some(value_id), color_slot, palette, generated_accents))
    }

    pub(super) fn table_value_alpha(value: f64, max_abs_value: f64) -> f32 {
        let relative = if max_abs_value > f64::EPSILON {
            (value.abs() / max_abs_value).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (0.1 + relative * 0.7) as f32
    }

    pub(super) fn fill_table_panel(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        indicator_panel: &IndicatorPanelScene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
    ) {
        let Some(value_id) = indicator_panel.value_id else {
            return;
        };
        let Some(indicator) = self.indicator_kind(value_id) else {
            return;
        };
        let columns = data::chart::kline::table_columns(indicator);
        if columns.is_empty() {
            return;
        }
        let Some(panel_plot) = scene
            .layout
            .panel(indicator_panel.panel_index)
            .map(|panel| panel.plot)
        else {
            return;
        };
        let row_height = (panel_plot.height / columns.len() as f32).max(1.0);
        let column_width = scene.logical_bar_spacing();
        let show_text = column_width >= TABLE_TEXT_MIN_COLUMN_WIDTH_PX
            && row_height >= TABLE_TEXT_MIN_ROW_HEIGHT_PX;
        let text_size = row_height
            .min(column_width / 8.0)
            .clamp(TABLE_TEXT_MIN_SIZE_PX, TEXT_SIZE);
        let cell_width = (column_width - TABLE_CELL_INSET_PX * 2.0).max(1.0);
        let separator_color = palette.background.strong.color.scale_alpha(0.35);
        frame.with_clip(panel_plot, |frame| {
            for (row, column) in columns.iter().enumerate() {
                let row_top = panel_plot.y + row as f32 * row_height;
                let row_center = row_top + row_height * 0.5;
                frame.stroke(
                    &canvas::Path::line(
                        Point::new(panel_plot.x, row_top),
                        Point::new(panel_plot.x + panel_plot.width, row_top),
                    ),
                    canvas::Stroke::default()
                        .with_width(1.0)
                        .with_color(separator_color),
                );
                let mut values = Vec::new();
                self.for_each_scene_bar_unit_index(
                    scene,
                    |series_index, bar_index, series, x_unit, _bar| {
                        if series_index != 0
                            || x_unit < scene.x.min_unit
                            || x_unit > scene.x.max_unit
                        {
                            return;
                        }
                        let Some(value) =
                            self.table_column_value(series, value_id, bar_index, &column.value)
                        else {
                            return;
                        };
                        let x = scene.map_x_plot(x_unit);
                        if x >= panel_plot.x && x <= panel_plot.x + panel_plot.width {
                            values.push((x, value));
                        }
                    },
                );
                if show_text {
                    for (x, value) in values {
                        let content = Self::format_table_value(value, column.format);
                        if content.is_empty() {
                            continue;
                        }
                        let Some(color) = self.table_value_color(
                            &column.color,
                            value,
                            value_id,
                            palette,
                            generated_accents,
                        ) else {
                            continue;
                        };
                        frame.fill_text(canvas::Text {
                            content,
                            position: Point::new(x, row_center),
                            color,
                            size: text_size.into(),
                            align_x: iced::Alignment::Center.into(),
                            align_y: iced::Alignment::Center.into(),
                            font: style::AZERET_MONO,
                            ..Default::default()
                        });
                    }
                } else {
                    let max_abs_value = values
                        .iter()
                        .filter_map(|(_, value)| match value {
                            OutputValue::Number(value) if value.is_finite() => Some(value.abs()),
                            OutputValue::Boolean(_) => None,
                            OutputValue::Number(_) => None,
                        })
                        .fold(0.0, f64::max);
                    for (x, value) in values {
                        let (alpha, color_value) = match value {
                            OutputValue::Number(value) if value.is_finite() => (
                                Self::table_value_alpha(value, max_abs_value),
                                OutputValue::Number(value),
                            ),
                            OutputValue::Number(_) => continue,
                            OutputValue::Boolean(true) => (0.85, OutputValue::Boolean(true)),
                            OutputValue::Boolean(false) => continue,
                        };
                        let Some(color) = self.table_value_color(
                            &column.color,
                            color_value,
                            value_id,
                            palette,
                            generated_accents,
                        ) else {
                            continue;
                        };
                        frame.fill_rectangle(
                            Point::new(x - cell_width * 0.5, row_top + TABLE_CELL_INSET_PX),
                            Size::new(
                                cell_width,
                                (row_height - TABLE_CELL_INSET_PX * 2.0).max(1.0),
                            ),
                            color.scale_alpha(alpha),
                        );
                    }
                }
            }
        });
    }

    pub(super) fn fill_main_geometry(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
        layer: PrimaryPlotLayer,
    ) {
        let primary_plot_detail = self.primary_plot_detail(scene);

        let (candle_body_width_phys, wick_width_phys) =
            Self::candlestick_widths(scene.logical_bar_spacing(), pixels);

        let indicator_width_phys = candle_body_width_phys;
        let candle_body_width = pixels.logical_width_from_physical(candle_body_width_phys);
        let wick_width = pixels.logical_width_from_physical(wick_width_phys);
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.x.min_unit && x_unit <= scene.x.max_unit;

        if layer == PrimaryPlotLayer::Background {
            for indicator_panel in &scene.indicator_panels {
                if matches!(indicator_panel.presentation, PanelPresentation::Table) {
                    self.fill_table_panel(
                        frame,
                        scene,
                        indicator_panel,
                        palette,
                        generated_accents,
                    );
                } else {
                    self.fill_indicator_panel_background(
                        frame,
                        scene,
                        indicator_panel,
                        palette,
                        generated_accents,
                        pixels,
                    );
                }
            }
            self.fill_primary_overlay_backgrounds(frame, scene, palette, generated_accents, pixels);

            for indicator_panel in &scene.indicator_panels {
                if self
                    .panel_indicator_value_ids(indicator_panel.panel_index)
                    .iter()
                    .all(|value_id| self.indicator_is_unavailable(Some(*value_id)))
                {
                    continue;
                }

                let Some(panel_plot) = scene
                    .layout
                    .panel(indicator_panel.panel_index)
                    .map(|panel| panel.plot)
                else {
                    continue;
                };

                let structural_layers = self
                    .panel_output_layers(indicator_panel.panel_index)
                    .into_iter()
                    .filter(|layer| layer.visible && layer.purpose.is_plot())
                    .collect::<Vec<_>>();
                frame.with_clip(panel_plot, |frame| {
                    for layer in &structural_layers {
                        let output = &layer.presentation_output;
                        let Some(spec) = self.indicator_output_spec(output) else {
                            continue;
                        };
                        match spec.value_type {
                            data::chart::kline::OutputValueType::Ohlc => {
                                self.draw_indicator_ohlc_output(
                                    frame,
                                    scene,
                                    output.instance_id,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                                    },
                                    indicator_width_phys,
                                    wick_width_phys,
                                    pixels,
                                    palette,
                                    generated_accents,
                                );
                            }
                            data::chart::kline::OutputValueType::Range => {
                                self.draw_indicator_range_output(
                                    frame,
                                    scene,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                                    },
                                    pixels,
                                    palette,
                                    generated_accents,
                                );
                            }
                            data::chart::kline::OutputValueType::Number
                            | data::chart::kline::OutputValueType::Boolean => {}
                        }
                    }
                    for value_id in self.panel_indicator_value_ids(indicator_panel.panel_index) {
                        self.draw_indicator_markers_and_levels(
                            frame,
                            scene,
                            value_id,
                            |value| scene.map_indicator_plot(indicator_panel.panel_index, value),
                            pixels,
                            palette,
                            generated_accents,
                        );
                    }
                });

                let output_layers = self
                    .panel_output_layers(indicator_panel.panel_index)
                    .into_iter()
                    .filter(|layer| {
                        layer.visible
                            && layer.purpose.is_plot()
                            && matches!(layer.mark, MarkKind::Candle | MarkKind::Bar(_))
                            && layer.data_kind != LayerDataKind::Ohlc
                            && self
                                .indicator_output_spec(&layer.presentation_output)
                                .is_some_and(|spec| {
                                    spec.value_type == data::chart::kline::OutputValueType::Number
                                })
                    })
                    .collect::<Vec<_>>();
                if output_layers.is_empty() {
                    continue;
                }

                frame.with_clip(panel_plot, |frame| {
                    self.for_each_scene_bar_unit_index(
                        scene,
                        |series_index, bar_index, series, x_unit, _bar| {
                            if series_index != 0 || !in_visible_range(x_unit) {
                                return;
                            }

                            let x_plot = scene.map_x_plot(x_unit);
                            let layer_count = output_layers.len();
                            let layer_width_phys =
                                (indicator_width_phys / layer_count as i32).max(1);
                            let layer_width = pixels.logical_width_from_physical(layer_width_phys);
                            for (layer_index, layer) in output_layers.iter().enumerate() {
                                let Some(values) =
                                    self.indicator_layer_values_at(series, layer, bar_index)
                                else {
                                    continue;
                                };
                                let slot_offset =
                                    (layer_index as f32 + 0.5) - (layer_count as f32 * 0.5);
                                let layer_center = x_plot + slot_offset * layer_width;
                                let layer_left =
                                    pixels.centered_left(layer_center, layer_width_phys);
                                let bar_output = &layer.presentation_output;
                                let Some(y_indicator_baseline) = self
                                    .map_indicator_plot_for_output(
                                        scene,
                                        indicator_panel.panel_index,
                                        &layer.base_output,
                                        0.0,
                                    )
                                    .or_else(|| {
                                        scene.indicator_panel_bottom(indicator_panel.panel_index)
                                    })
                                else {
                                    continue;
                                };

                                match values {
                                    super::IndicatorBarValues::SignedOverlay {
                                        background_value,
                                        signed_value,
                                    } => {
                                        let Some(y_base) = self.map_indicator_plot_for_output(
                                            scene,
                                            indicator_panel.panel_index,
                                            &layer.base_output,
                                            background_value,
                                        ) else {
                                            continue;
                                        };
                                        let base =
                                            pixels.snapped_y_span(y_base, y_indicator_baseline);
                                        let base_top = base.start;
                                        let base_height = base.size;
                                        let base_size = Size::new(layer_width, base_height);
                                        let base_origin = Point::new(layer_left, base_top);
                                        let overlay_output = bar_output;
                                        let color_slot = self.style_slot_for_output(
                                            layer.base_output.instance_id,
                                            overlay_output.channel_id.as_str(),
                                            Some(signed_value >= 0.0),
                                            self.indicator_style_slot_for_output(overlay_output),
                                        );
                                        let color = self.indicator_color(
                                            Some(layer.base_output.instance_id),
                                            color_slot,
                                            palette,
                                            generated_accents,
                                        );
                                        frame.fill_rectangle(
                                            base_origin,
                                            base_size,
                                            color.scale_alpha(0.3),
                                        );

                                        if signed_value.abs() > 0.0
                                            && let Some(y_overlay) = self
                                                .map_indicator_plot_for_output(
                                                    scene,
                                                    indicator_panel.panel_index,
                                                    overlay_output,
                                                    signed_value.abs(),
                                                )
                                        {
                                            let overlay = pixels
                                                .snapped_y_span(y_overlay, y_indicator_baseline);
                                            frame.fill_rectangle(
                                                Point::new(layer_left, overlay.start),
                                                Size::new(layer_width, overlay.size),
                                                color,
                                            );
                                        }
                                        if let Some((outline_color, outline_width, outline_style)) =
                                            self.indicator_highlight_style_for_mark_at(
                                                series,
                                                bar_index,
                                                overlay_output,
                                                layer.mark,
                                                palette,
                                                generated_accents,
                                            )
                                        {
                                            draw_indicator_bar_outline(
                                                frame,
                                                base_origin,
                                                base_size,
                                                outline_color,
                                                outline_width,
                                                outline_style,
                                            );
                                        }
                                    }
                                    super::IndicatorBarValues::Plain { value } => {
                                        let Some(y_base) = self.map_indicator_plot_for_output(
                                            scene,
                                            indicator_panel.panel_index,
                                            &layer.base_output,
                                            value,
                                        ) else {
                                            continue;
                                        };
                                        let base =
                                            pixels.snapped_y_span(y_base, y_indicator_baseline);
                                        let base_top = base.start;
                                        let base_height = base.size;
                                        let base_size = Size::new(layer_width, base_height);
                                        let base_origin = Point::new(layer_left, base_top);
                                        let color = self.indicator_color_for_output_at(
                                            series,
                                            bar_index,
                                            &layer.base_output,
                                            Some(MarkKind::Bar(BarMode::Histogram(
                                                HistogramMode::Plain,
                                            ))),
                                            "bars",
                                            ThemeColorRole::Neutral,
                                            palette,
                                            generated_accents,
                                        );
                                        frame.fill_rectangle(
                                            base_origin,
                                            base_size,
                                            color.scale_alpha(0.5),
                                        );
                                        if let Some((outline_color, outline_width, outline_style)) =
                                            self.indicator_highlight_style_for_mark_at(
                                                series,
                                                bar_index,
                                                bar_output,
                                                layer.mark,
                                                palette,
                                                generated_accents,
                                            )
                                        {
                                            draw_indicator_bar_outline(
                                                frame,
                                                base_origin,
                                                base_size,
                                                outline_color,
                                                outline_width,
                                                outline_style,
                                            );
                                        }
                                    }
                                }
                            }
                        },
                    );
                });
            }

            if let Some(primary_panel_id) = self.composition.primary_panel_id()
                && let Some(primary_panel) = self.composition.panel(primary_panel_id)
            {
                let base_series_anchor = scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten();
                let primary_plot = *scene.primary_plot();

                let region_ids = self
                    .primary_overlay_entries()
                    .into_iter()
                    .filter_map(|(value_id, visible)| visible.then_some(value_id))
                    .collect::<Vec<_>>();
                frame.with_clip(primary_plot, |frame| {
                    for value_id in region_ids {
                        self.draw_indicator_regions(
                            frame,
                            scene,
                            value_id,
                            |value| scene.map_primary_plot_with_anchor(value, base_series_anchor),
                            pixels,
                            palette,
                            generated_accents,
                        );
                    }
                });

                self.fill_footprint_poc_overlay(frame, scene, palette, generated_accents, pixels);
                self.fill_volume_profile_overlay(frame, scene, palette, generated_accents, pixels);

                let structural_layers = primary_panel
                    .layers
                    .iter()
                    .filter_map(|layer| {
                        let output = layer.source.indicator_output()?;
                        let resolved =
                            self.resolved_indicator_layer_for_composition_layer(layer)?;
                        (layer.visible
                            && resolved.purpose.is_plot()
                            && !self.indicator_is_unavailable(Some(output.instance_id)))
                        .then_some(resolved)
                    })
                    .collect::<Vec<_>>();
                frame.with_clip(primary_plot, |frame| {
                    let mut annotation_ids = FxHashSet::default();
                    for layer in &structural_layers {
                        let output = &layer.presentation_output;
                        let Some(spec) = self.indicator_output_spec(output) else {
                            continue;
                        };
                        match spec.value_type {
                            data::chart::kline::OutputValueType::Ohlc => {
                                self.draw_indicator_ohlc_output(
                                    frame,
                                    scene,
                                    output.instance_id,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene
                                            .map_primary_plot_with_anchor(value, base_series_anchor)
                                    },
                                    indicator_width_phys,
                                    wick_width_phys,
                                    pixels,
                                    palette,
                                    generated_accents,
                                );
                            }
                            data::chart::kline::OutputValueType::Range => {
                                self.draw_indicator_range_output(
                                    frame,
                                    scene,
                                    output,
                                    layer.required_input,
                                    |value| {
                                        scene
                                            .map_primary_plot_with_anchor(value, base_series_anchor)
                                    },
                                    pixels,
                                    palette,
                                    generated_accents,
                                );
                            }
                            data::chart::kline::OutputValueType::Number
                            | data::chart::kline::OutputValueType::Boolean => {}
                        }
                        if annotation_ids.insert(output.instance_id) {
                            self.draw_indicator_markers_and_levels(
                                frame,
                                scene,
                                output.instance_id,
                                |value| {
                                    scene.map_primary_plot_with_anchor(value, base_series_anchor)
                                },
                                pixels,
                                palette,
                                generated_accents,
                            );
                        }
                    }
                });

                frame.with_clip(primary_plot, |frame| {
                    for layer in &primary_panel.layers {
                        if !layer.visible {
                            continue;
                        }

                        let Some(resolved_layer) =
                            self.resolved_indicator_layer_for_composition_layer(layer)
                        else {
                            continue;
                        };
                        if !resolved_layer.purpose.is_plot() {
                            continue;
                        }
                        let value_id = resolved_layer.base_output.instance_id;
                        if self.indicator_is_unavailable(Some(value_id)) {
                            continue;
                        }

                        let output = &resolved_layer.presentation_output;
                        if self.indicator_output_spec(output).is_some_and(|spec| {
                            spec.value_type != data::chart::kline::OutputValueType::Number
                        }) {
                            // Boolean channels are event/table values. They do
                            // not have a numeric y-coordinate until a compatible
                            // mark is introduced.
                            continue;
                        }
                        let mark = resolved_layer.mark;
                        for channel in self.overlay_channels_for_layer(Some(output)).into_iter() {
                            let (channel_line_width, channel_line_width_phys) = pixels
                                .quantized_stroke_width(self.indicator_line_width_for_output(
                                    Some(value_id),
                                    Some(output),
                                    &channel,
                                ));
                            let bridge_sparse_values = mark == MarkKind::Segment;
                            if matches!(mark, MarkKind::Point | MarkKind::Cross) {
                                self.draw_primary_indicator_points(
                                    frame,
                                    scene,
                                    value_id,
                                    output,
                                    channel,
                                    mark,
                                    resolved_layer.required_input,
                                    palette,
                                    generated_accents,
                                    base_series_anchor,
                                    pixels,
                                );
                                continue;
                            }
                            let mut line_runs = Vec::<IndicatorLineRun>::new();
                            let mut previous_point: Option<Point> = None;
                            let mut previous_value: Option<f64> = None;
                            let source_channel = channel.source;
                            let has_dynamic_style = channel.style_by.is_some()
                                || self
                                    .indicator_output_spec_for_channel(value_id, source_channel)
                                    .is_some_and(|spec| spec.style_by.is_some());
                            let static_color = (!has_dynamic_style).then(|| {
                                self.overlay_channel_color_for_output(
                                    Some(value_id),
                                    Some(output),
                                    &channel,
                                    palette,
                                    generated_accents,
                                )
                            });
                            let traversal = if bridge_sparse_values {
                                IndicatorLineTraversal::SegmentWithNeighbors
                            } else {
                                IndicatorLineTraversal::VisibleWithNeighbors
                            };
                            self.for_each_scene_indicator_line_output_index_for_channel(
                                scene,
                                value_id,
                                source_channel,
                                traversal,
                                |series, source_index, x_unit| {
                                    if !self.indicator_input_available_at(
                                        series,
                                        source_index,
                                        resolved_layer.required_input,
                                    ) {
                                        previous_point = None;
                                        previous_value = None;
                                        return IndicatorLinePointStatus::Break;
                                    }
                                    let Some(value) = self
                                        .indicator_raw_output_value_for_series_at_channel(
                                            series,
                                            value_id,
                                            source_channel,
                                            source_index,
                                        )
                                        .and_then(OutputValue::as_number)
                                    else {
                                        if !bridge_sparse_values {
                                            previous_point = None;
                                            previous_value = None;
                                        }
                                        return if bridge_sparse_values {
                                            IndicatorLinePointStatus::Skipped
                                        } else {
                                            IndicatorLinePointStatus::Break
                                        };
                                    };
                                    if !value.is_finite() {
                                        if !bridge_sparse_values {
                                            previous_point = None;
                                            previous_value = None;
                                        }
                                        return if bridge_sparse_values {
                                            IndicatorLinePointStatus::Skipped
                                        } else {
                                            IndicatorLinePointStatus::Break
                                        };
                                    }
                                    let Some(y) = scene
                                        .map_primary_plot_with_anchor(value, base_series_anchor)
                                    else {
                                        if !bridge_sparse_values {
                                            previous_point = None;
                                            previous_value = None;
                                        }
                                        return if bridge_sparse_values {
                                            IndicatorLinePointStatus::Skipped
                                        } else {
                                            IndicatorLinePointStatus::Break
                                        };
                                    };
                                    let point = Point::new(scene.map_x_plot(x_unit), y);

                                    let color = if has_dynamic_style {
                                        self.overlay_channel_color_for_channel_at(
                                            series,
                                            source_index,
                                            value_id,
                                            value_id,
                                            source_channel,
                                            &channel,
                                            palette,
                                            generated_accents,
                                        )
                                    } else {
                                        static_color.expect("static overlay color is precomputed")
                                    };
                                    let horizontal_level_break = mark == MarkKind::HorizontalLevel
                                        && previous_point.is_some()
                                        && (previous_value != Some(value)
                                            || line_runs
                                                .last()
                                                .is_some_and(|run| run.color != color));
                                    if horizontal_level_break {
                                        if let Some(previous) = previous_point
                                            && let Some(run) = line_runs.last_mut()
                                        {
                                            run.points.push(Point::new(point.x, previous.y));
                                        }
                                        previous_point = None;
                                    }
                                    append_indicator_line_point(
                                        &mut line_runs,
                                        previous_point,
                                        point,
                                        color,
                                        mark == MarkKind::StepLine,
                                    );
                                    previous_point = Some(point);
                                    previous_value = Some(value);
                                    IndicatorLinePointStatus::Rendered
                                },
                            );

                            if mark == MarkKind::HorizontalLevel
                                && let Some(run) = line_runs.last_mut()
                                && let Some(last) = run.points.last().copied()
                            {
                                let end = Point::new(scene.map_x_plot(scene.x.max_unit), last.y);
                                if end.x > last.x {
                                    run.points.push(end);
                                }
                            }

                            let stroke_style = self.indicator_stroke_style_for_output(
                                Some(value_id),
                                Some(output),
                                &channel,
                            );
                            let line_runs = trim_indicator_line_runs(line_runs, primary_plot);
                            if mark == MarkKind::Area {
                                let baseline =
                                    scene.map_primary_plot_with_anchor_f32(0.0, base_series_anchor);
                                for run in &line_runs {
                                    let snapped_points = snap_indicator_line_points(
                                        &run.points,
                                        pixels,
                                        channel_line_width_phys,
                                    );
                                    fill_indicator_area(
                                        frame,
                                        &run.points,
                                        &snapped_points,
                                        baseline,
                                        run.color,
                                        pixels,
                                    );
                                }
                            }
                            for run in line_runs {
                                let snapped_points = snap_indicator_line_points(
                                    &run.points,
                                    pixels,
                                    channel_line_width_phys,
                                );
                                if snapped_points.len() >= 2
                                    && snapped_points.windows(2).any(|pair| pair[0] != pair[1])
                                {
                                    let path = indicator_line_path(&snapped_points);
                                    frame.stroke(
                                        &path,
                                        indicator_line_stroke(
                                            channel_line_width,
                                            stroke_style,
                                            run.color,
                                        ),
                                    );
                                } else if let Some(point) = snapped_points.first().copied() {
                                    fill_minimum_mark(frame, point, run.color, pixels);
                                }
                            }
                        }
                    }
                });
            }
        }

        if layer == PrimaryPlotLayer::Background {
            return;
        }
        if layer == PrimaryPlotLayer::Tpo {
            self.fill_tpo_profile_overlay(frame, scene, palette, generated_accents, pixels);
            return;
        }

        if self.candlestick.visible
            && matches!(primary_plot_detail, PrimaryPlotDetail::CandleOnly)
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_))
        {
            let primary_plot = *scene.primary_plot();
            frame.with_clip(primary_plot, |frame| {
                self.for_each_scene_bar_unit_index(
                    scene,
                    |series_index, _, _series, x_unit, point| {
                        if series_index != 0 || !in_visible_range(x_unit) {
                            return;
                        }

                        let series_anchor = scene
                            .primary
                            .series_percent_anchors
                            .first()
                            .copied()
                            .flatten();
                        let y_open = scene.map_primary_plot_with_anchor_f32(
                            point.kline.open.to_f32_lossy(),
                            series_anchor,
                        );
                        let y_high = scene.map_primary_plot_with_anchor_f32(
                            point.kline.high.to_f32_lossy(),
                            series_anchor,
                        );
                        let y_low = scene.map_primary_plot_with_anchor_f32(
                            point.kline.low.to_f32_lossy(),
                            series_anchor,
                        );
                        let y_close = scene.map_primary_plot_with_anchor_f32(
                            point.kline.close.to_f32_lossy(),
                            series_anchor,
                        );

                        let color = self.candlestick_color(
                            point.kline.close >= point.kline.open,
                            palette,
                            generated_accents,
                        );

                        let body = pixels.snapped_y_span(y_open, y_close);
                        let wick = pixels.snapped_y_span(y_high, y_low);

                        let x_plot = scene.map_x_plot(x_unit);
                        let candle_left = pixels.centered_left(x_plot, candle_body_width_phys);
                        let wick_left = pixels.centered_left(x_plot, wick_width_phys);

                        frame.fill_rectangle(
                            Point::new(candle_left, body.start),
                            Size::new(candle_body_width, body.size),
                            color,
                        );
                        frame.fill_rectangle(
                            Point::new(wick_left, wick.start),
                            Size::new(wick_width, wick.size),
                            color.scale_alpha(0.85),
                        );
                    },
                );
            });
        }

        if matches!(primary_plot_detail, PrimaryPlotDetail::Footprint) {
            self.fill_footprint_overlay(
                frame,
                scene,
                palette,
                generated_accents,
                self.footprint.visible,
                pixels,
            );
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            for series_index in 0..self.series.len() {
                let is_base_series = series_index == 0;
                let (line_width, line_width_phys) = pixels
                    .quantized_stroke_width(f32::from(self.series[series_index].line_width()));
                let mut point_count = 0usize;
                let mut first_point = None;
                let mut has_distinct_point = false;

                let path = canvas::Path::new(|builder| {
                    self.for_each_scene_bar_unit_index(
                        scene,
                        |iter_series_index, _, _series, x_unit, point| {
                            if iter_series_index != series_index || !in_visible_range(x_unit) {
                                return;
                            }

                            let is_base_series = iter_series_index == 0;
                            if is_base_series && !scene.primary.mark.is_line_like() {
                                return;
                            }

                            let series_anchor = scene
                                .primary
                                .series_percent_anchors
                                .get(iter_series_index)
                                .copied()
                                .flatten();
                            let point = Point::new(
                                pixels.snap_x_for_stroke(scene.map_x_plot(x_unit), line_width_phys),
                                pixels.snap_y_for_stroke(
                                    scene.map_primary_plot_with_anchor_f32(
                                        point.kline.close.to_f32_lossy(),
                                        series_anchor,
                                    ),
                                    line_width_phys,
                                ),
                            );

                            if let Some(first_point) = first_point {
                                has_distinct_point |= first_point != point;
                            } else {
                                first_point = Some(point);
                            }
                            if point_count == 0 {
                                builder.move_to(point);
                            } else {
                                builder.line_to(point);
                            }
                            point_count += 1;
                        },
                    );
                });

                let line_color = self
                    .series_line_color(&self.series[series_index], palette, generated_accents)
                    .scale_alpha(if is_base_series { 0.85 } else { 0.96 });
                if point_count < 2 {
                    continue;
                }
                if !has_distinct_point {
                    if let Some(point) = first_point {
                        fill_minimum_mark(frame, point, line_color, pixels);
                    }
                    continue;
                }

                frame.stroke(
                    &path,
                    indicator_line_stroke(
                        line_width,
                        self.series[series_index].stroke_style(),
                        line_color,
                    ),
                );
            }
        });

        for indicator_panel in &scene.indicator_panels {
            if self
                .panel_indicator_value_ids(indicator_panel.panel_index)
                .iter()
                .all(|value_id| self.indicator_is_unavailable(Some(*value_id)))
            {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            let output_layers = self
                .panel_output_layers(indicator_panel.panel_index)
                .into_iter()
                .filter(|layer| {
                    layer.visible
                        && layer.purpose.is_plot()
                        && layer.mark.is_series_line_like()
                        && self
                            .indicator_output_spec(&layer.presentation_output)
                            .is_some_and(|spec| {
                                spec.value_type == data::chart::kline::OutputValueType::Number
                            })
                })
                .collect::<Vec<_>>();
            if output_layers.is_empty() {
                continue;
            }

            frame.with_clip(panel_plot, |frame| {
                for layer in &output_layers {
                    let output = &layer.presentation_output;
                    let Some(channel) = self.overlay_channels_for_layer(Some(output)) else {
                        continue;
                    };
                    let (channel_line_width, channel_line_width_phys) = pixels
                        .quantized_stroke_width(self.indicator_line_width_for_output(
                            Some(output.instance_id),
                            Some(output),
                            &channel,
                        ));
                    let mark = layer.mark;
                    let bridge_sparse_values = mark == MarkKind::Segment;
                    let mut line_runs = Vec::<IndicatorLineRun>::new();
                    let mut previous_point: Option<Point> = None;
                    let mut previous_value: Option<f64> = None;

                    let traversal = if bridge_sparse_values {
                        IndicatorLineTraversal::SegmentWithNeighbors
                    } else {
                        IndicatorLineTraversal::VisibleWithNeighbors
                    };
                    self.for_each_scene_indicator_line_output_index(
                        scene,
                        output,
                        traversal,
                        |series, source_index, x_unit| {
                            if !self.indicator_input_available_at(
                                series,
                                source_index,
                                layer.required_input,
                            ) {
                                previous_point = None;
                                previous_value = None;
                                return IndicatorLinePointStatus::Break;
                            }
                            let Some(channel_value) = self
                                .indicator_raw_numeric_output_value_for_series_at(
                                    series,
                                    output,
                                    source_index,
                                )
                            else {
                                if !bridge_sparse_values {
                                    previous_point = None;
                                    previous_value = None;
                                }
                                return if bridge_sparse_values {
                                    IndicatorLinePointStatus::Skipped
                                } else {
                                    IndicatorLinePointStatus::Break
                                };
                            };
                            let Some(y_channel_value) = scene
                                .map_indicator_plot(indicator_panel.panel_index, channel_value)
                            else {
                                if !bridge_sparse_values {
                                    previous_point = None;
                                    previous_value = None;
                                }
                                return if bridge_sparse_values {
                                    IndicatorLinePointStatus::Skipped
                                } else {
                                    IndicatorLinePointStatus::Break
                                };
                            };

                            let point = Point::new(scene.map_x_plot(x_unit), y_channel_value);
                            let color = self.overlay_channel_color_for_output_at(
                                series,
                                source_index,
                                Some(output.instance_id),
                                Some(output),
                                &channel,
                                palette,
                                generated_accents,
                            );
                            let horizontal_level_break = mark == MarkKind::HorizontalLevel
                                && previous_point.is_some()
                                && (previous_value != Some(channel_value)
                                    || line_runs.last().is_some_and(|run| run.color != color));
                            if horizontal_level_break {
                                if let Some(previous) = previous_point
                                    && let Some(run) = line_runs.last_mut()
                                {
                                    run.points.push(Point::new(point.x, previous.y));
                                }
                                previous_point = None;
                            }
                            append_indicator_line_point(
                                &mut line_runs,
                                previous_point,
                                point,
                                color,
                                mark == MarkKind::StepLine,
                            );
                            previous_point = Some(point);
                            previous_value = Some(channel_value);
                            IndicatorLinePointStatus::Rendered
                        },
                    );

                    if mark == MarkKind::HorizontalLevel
                        && let Some(run) = line_runs.last_mut()
                        && let Some(last) = run.points.last().copied()
                    {
                        let end = Point::new(scene.map_x_plot(scene.x.max_unit), last.y);
                        if end.x > last.x {
                            run.points.push(end);
                        }
                    }

                    let stroke_style = self.indicator_stroke_style_for_output(
                        Some(output.instance_id),
                        Some(output),
                        &channel,
                    );
                    let line_runs = trim_indicator_line_runs(line_runs, panel_plot);
                    if mark == MarkKind::Area {
                        let baseline = scene
                            .map_indicator_plot(indicator_panel.panel_index, 0.0)
                            .unwrap_or(panel_plot.y + panel_plot.height);
                        for run in &line_runs {
                            let snapped_points = snap_indicator_line_points(
                                &run.points,
                                pixels,
                                channel_line_width_phys,
                            );
                            fill_indicator_area(
                                frame,
                                &run.points,
                                &snapped_points,
                                baseline,
                                run.color,
                                pixels,
                            );
                        }
                    }
                    for run in line_runs {
                        let snapped_points = snap_indicator_line_points(
                            &run.points,
                            pixels,
                            channel_line_width_phys,
                        );
                        if snapped_points.len() >= 2
                            && snapped_points.windows(2).any(|pair| pair[0] != pair[1])
                        {
                            let path = indicator_line_path(&snapped_points);
                            frame.stroke(
                                &path,
                                indicator_line_stroke(channel_line_width, stroke_style, run.color),
                            );
                        } else if let Some(point) = snapped_points.first().copied() {
                            fill_minimum_mark(frame, point, run.color, pixels);
                        }
                    }
                }
            });
        }

        self.fill_big_trades_overlay(frame, scene, palette, generated_accents, pixels);
        self.fill_panel_titles(frame, scene, palette);
        self.fill_last_price_line(frame, scene, palette);
    }
}

fn table_color_slot(color: &TableColumnColor, value: OutputValue) -> Option<&str> {
    match color {
        TableColumnColor::Static { slot } => match value {
            OutputValue::Number(value) if value.is_finite() => Some(slot.as_str()),
            OutputValue::Boolean(true) => Some(slot.as_str()),
            OutputValue::Number(_) | OutputValue::Boolean(false) => None,
        },
        TableColumnColor::Signed {
            positive_slot,
            negative_slot,
        } => match value {
            OutputValue::Number(value) if value.is_finite() => Some(if value < 0.0 {
                negative_slot.as_str()
            } else {
                positive_slot.as_str()
            }),
            OutputValue::Number(_) | OutputValue::Boolean(_) => None,
        },
    }
}
