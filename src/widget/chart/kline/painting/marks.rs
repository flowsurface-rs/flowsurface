use crate::widget::chart::kline::TEXT_SIZE;
use crate::widget::chart::kline::composition::{BarMode, HistogramMode};
use crate::widget::chart::kline::composition::{MarkKind, PanelValueId};
use crate::widget::chart::kline::painting::{PaintInputs, PixelGeometry};
use crate::widget::chart::kline::presentation::IndicatorView;
use crate::widget::chart::kline::presentation::{IndicatorBarValues, ResolvedIndicatorLayer};
use crate::widget::chart::kline::presentation::{
    IndicatorLinePointStatus, IndicatorLineTraversal, OverlayChannelSpec,
};
use crate::widget::chart::kline::scene::{IndicatorPanelScene, Scene};
use crate::{style, widget::chart::kline::painting};
use data::chart::kline::series::KlineSeries;
use data::chart::kline::{
    IndicatorFillGeometry, IndicatorFillSpec, IndicatorInputSource, IndicatorOutputRef,
    LineStrokeStyle, OutputValue, TableColumnColor, TableColumnFormat,
};
use data::config::theme::ACCENT_COUNT;
use data::config::theme::ThemeColorRole;
use data::util::abbr_large_numbers;
use iced::theme::palette::Extended;
use iced::widget::canvas;
use iced::{Point, Rectangle, Size};
use rustc_hash::FxHashSet;
use std::collections::BTreeMap;

const TABLE_TEXT_MIN_COLUMN_WIDTH_PX: f32 = 48.0;
const TABLE_TEXT_MIN_ROW_HEIGHT_PX: f32 = 9.0;
const TABLE_TEXT_MIN_SIZE_PX: f32 = 8.0;
const TABLE_CELL_INSET_PX: f32 = 1.0;

const INDICATOR_MARKER_TEXT_SIZE_PX: f32 = 8.0;

const DASHED_LINE_SEGMENTS: [f32; 2] = [8.0, 5.0];
const DOTTED_LINE_SEGMENTS: [f32; 2] = [1.0, 4.0];

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

pub(crate) struct MarkRenderer<'a> {
    pub(crate) indicators: IndicatorView<'a>,
    pub(crate) scene: &'a Scene,
    pub(crate) palette: &'a Extended,
    pub(crate) generated_accents: &'a [iced::Color; ACCENT_COUNT],
    pub(crate) pixels: PixelGeometry,
}

impl MarkRenderer<'_> {
    pub(crate) fn draw_indicator_bars(
        &self,
        frame: &mut PaintInputs,
        panel_index: usize,
        output_layers: &[ResolvedIndicatorLayer],
        indicator_width_phys: i32,
    ) {
        let scene = self.scene;
        let pixels = self.pixels;
        let palette = self.palette;
        let generated_accents = self.generated_accents;
        let in_visible_range = |unit| unit >= scene.x.min_unit && unit <= scene.x.max_unit;
        scene.for_each_bar(
            self.indicators.series,
            |series_index, bar_index, series, x_unit, _bar| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let x_plot = scene.map_x_plot(x_unit);
                let layer_count = output_layers.len();
                let layer_width_phys = (indicator_width_phys / layer_count as i32).max(1);
                let layer_width = pixels.logical_width_from_physical(layer_width_phys);
                for (layer_index, layer) in output_layers.iter().enumerate() {
                    let Some(values) = self
                        .indicators
                        .indicator_layer_values_at(series, layer, bar_index)
                    else {
                        continue;
                    };
                    let slot_offset = (layer_index as f32 + 0.5) - (layer_count as f32 * 0.5);
                    let layer_center = x_plot + slot_offset * layer_width;
                    let layer_left = pixels.centered_left(layer_center, layer_width_phys);
                    let bar_output = &layer.presentation_output;
                    let Some(y_indicator_baseline) = self
                        .map_indicator_plot_for_output(panel_index, &layer.base_output, 0.0)
                        .or_else(|| scene.indicator_panel_bottom(panel_index))
                    else {
                        continue;
                    };

                    match values {
                        IndicatorBarValues::SignedOverlay {
                            background_value,
                            signed_value,
                        } => {
                            let Some(y_base) = self.map_indicator_plot_for_output(
                                panel_index,
                                &layer.base_output,
                                background_value,
                            ) else {
                                continue;
                            };
                            let base = pixels.snapped_y_span(y_base, y_indicator_baseline);
                            let base_top = base.start;
                            let base_height = base.size;
                            let base_size = Size::new(layer_width, base_height);
                            let base_origin = Point::new(layer_left, base_top);
                            let overlay_output = bar_output;
                            let color_slot = self.indicators.style_slot_for_output(
                                layer.base_output.instance_id,
                                overlay_output.channel_id.as_str(),
                                Some(signed_value >= 0.0),
                                self.indicators
                                    .indicator_style_slot_for_output(overlay_output),
                            );
                            let color = self.indicators.indicator_color(
                                Some(layer.base_output.instance_id),
                                color_slot,
                                palette,
                                generated_accents,
                            );
                            frame.fill_rectangle(base_origin, base_size, color.scale_alpha(0.3));

                            if signed_value.abs() > 0.0
                                && let Some(y_overlay) = self.map_indicator_plot_for_output(
                                    panel_index,
                                    overlay_output,
                                    signed_value.abs(),
                                )
                            {
                                let overlay =
                                    pixels.snapped_y_span(y_overlay, y_indicator_baseline);
                                frame.fill_rectangle(
                                    Point::new(layer_left, overlay.start),
                                    Size::new(layer_width, overlay.size),
                                    color,
                                );
                            }
                            if let Some((outline_color, outline_width, outline_style)) =
                                self.indicators.indicator_highlight_style_for_mark_at(
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
                        IndicatorBarValues::Plain { value } => {
                            let Some(y_base) = self.map_indicator_plot_for_output(
                                panel_index,
                                &layer.base_output,
                                value,
                            ) else {
                                continue;
                            };
                            let base = pixels.snapped_y_span(y_base, y_indicator_baseline);
                            let base_top = base.start;
                            let base_height = base.size;
                            let base_size = Size::new(layer_width, base_height);
                            let base_origin = Point::new(layer_left, base_top);
                            let color = self.indicators.indicator_color_for_output_at(
                                series,
                                bar_index,
                                &layer.base_output,
                                Some(MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain))),
                                "bars",
                                ThemeColorRole::Neutral,
                                palette,
                                generated_accents,
                            );
                            frame.fill_rectangle(base_origin, base_size, color.scale_alpha(0.5));
                            if let Some((outline_color, outline_width, outline_style)) =
                                self.indicators.indicator_highlight_style_for_mark_at(
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
    }

    pub(crate) fn draw_candles(
        &self,
        frame: &mut PaintInputs,
        map_y: impl Fn(f32) -> f32,
        color_for: impl Fn(bool) -> iced::Color,
    ) {
        let scene = self.scene;
        let pixels = self.pixels;
        let (candle_body_width_phys, wick_width_phys) =
            candlestick_widths(scene.logical_bar_spacing(), pixels);
        let candle_body_width = pixels.logical_width_from_physical(candle_body_width_phys);
        let wick_width = pixels.logical_width_from_physical(wick_width_phys);
        let in_visible_range = |unit| unit >= scene.x.min_unit && unit <= scene.x.max_unit;
        scene.for_each_bar(
            self.indicators.series,
            |series_index, _, _series, x_unit, point| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let y_open = map_y(point.kline.open.to_f32_lossy());
                let y_high = map_y(point.kline.high.to_f32_lossy());
                let y_low = map_y(point.kline.low.to_f32_lossy());
                let y_close = map_y(point.kline.close.to_f32_lossy());
                let color = color_for(point.kline.close >= point.kline.open);

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
    }

    pub(crate) fn draw_series_line(&self, frame: &mut PaintInputs, series_index: usize) {
        let scene = self.scene;
        let pixels = self.pixels;
        let palette = self.palette;
        let generated_accents = self.generated_accents;
        let in_visible_range = |unit| unit >= scene.x.min_unit && unit <= scene.x.max_unit;
        let is_base_series = series_index == 0;
        let (line_width, line_width_phys) = pixels
            .quantized_stroke_width(f32::from(self.indicators.series[series_index].line_width()));
        let mut point_count = 0usize;
        let mut first_point = None;
        let mut has_distinct_point = false;

        let path = canvas::Path::new(|builder| {
            scene.for_each_bar(
                self.indicators.series,
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

        let line_color = painting::series_line_color(
            &self.indicators.series[series_index],
            palette,
            generated_accents,
        )
        .scale_alpha(if is_base_series { 0.85 } else { 0.96 });
        if point_count < 2 {
            return;
        }
        if !has_distinct_point {
            if let Some(point) = first_point {
                fill_minimum_mark(frame, point, line_color, pixels);
            }
            return;
        }

        frame.stroke(
            &path,
            indicator_line_stroke(
                line_width,
                self.indicators.series[series_index].stroke_style(),
                line_color,
            ),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_line(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        output: &IndicatorOutputRef,
        channel: &OverlayChannelSpec<'_>,
        mark: MarkKind,
        required_input: Option<IndicatorInputSource>,
        plot: Rectangle,
        map_y: impl Fn(f64) -> Option<f32>,
        baseline: f32,
        color_at: impl Fn(&KlineSeries, usize) -> iced::Color,
    ) {
        let pixels = self.pixels;
        let scene = self.scene;
        let (channel_line_width, channel_line_width_phys) = pixels.quantized_stroke_width(
            self.indicators
                .indicator_line_width_for_output(Some(value_id), Some(output), channel),
        );
        let bridge_sparse_values = mark == MarkKind::Segment;
        let source_channel = channel.source;
        let mut line_runs = Vec::<IndicatorLineRun>::new();
        let mut previous_point: Option<Point> = None;
        let mut previous_value: Option<f64> = None;
        let traversal = if bridge_sparse_values {
            IndicatorLineTraversal::SegmentWithNeighbors
        } else {
            IndicatorLineTraversal::VisibleWithNeighbors
        };
        self.indicators
            .for_each_scene_indicator_line_output_index_for_channel(
                scene,
                value_id,
                source_channel,
                traversal,
                |series, source_index, x_unit| {
                    if !self.indicators.indicator_input_available_at(
                        series,
                        source_index,
                        required_input,
                    ) {
                        previous_point = None;
                        previous_value = None;
                        return IndicatorLinePointStatus::Break;
                    }
                    let Some(value) = self
                        .indicators
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
                    let Some(y) = map_y(value) else {
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

                    let color = color_at(series, source_index);
                    let horizontal_level_break = mark == MarkKind::HorizontalLevel
                        && previous_point.is_some()
                        && (previous_value != Some(value)
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

        let stroke_style = self.indicators.indicator_stroke_style_for_output(
            Some(value_id),
            Some(output),
            channel,
        );
        let line_runs = trim_indicator_line_runs(line_runs, plot);
        if mark == MarkKind::Area {
            for run in &line_runs {
                let snapped_points =
                    snap_indicator_line_points(&run.points, pixels, channel_line_width_phys);
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
            let snapped_points =
                snap_indicator_line_points(&run.points, pixels, channel_line_width_phys);
            if snapped_points.len() >= 2 && snapped_points.windows(2).any(|pair| pair[0] != pair[1])
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

    fn bar_unit_at_series_index(&self, series_index: usize, point_index: usize) -> Option<i64> {
        let scene = self.scene;
        data::chart::kline::series::KlineWindows {
            series: self.indicators.series,
            windows: &scene.bar_units,
            axis: scene.x.axis,
        }
        .unit_at_index(series_index, point_index)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_points(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        output: &IndicatorOutputRef,
        channel: OverlayChannelSpec<'_>,
        mark: MarkKind,
        required_input: Option<IndicatorInputSource>,
        map_y: impl Fn(f64) -> Option<f32>,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        let source_channel = channel.source;
        let has_dynamic_style = channel.style_by.is_some()
            || self
                .indicators
                .indicator_output_spec_for_channel(value_id, source_channel)
                .is_some_and(|spec| spec.style_by.is_some());
        let fallback_color = self.indicators.overlay_channel_color_for_output(
            Some(value_id),
            Some(output),
            &channel,
            palette,
            generated_accents,
        );
        let stroke_width =
            self.indicators
                .indicator_line_width_for_output(Some(value_id), Some(output), &channel);
        self.indicators
            .for_each_scene_indicator_output_index_for_channel(
                scene,
                value_id,
                source_channel,
                |series, source_index, x_unit| {
                    if !self.indicators.indicator_input_available_at(
                        series,
                        source_index,
                        required_input,
                    ) {
                        return;
                    }
                    let Some(value) = self
                        .indicators
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
                    let Some(y) = map_y(value) else {
                        return;
                    };
                    let color = if has_dynamic_style {
                        self.indicators.overlay_channel_color_for_channel_at(
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
                                self.indicators.indicator_stroke_style_for_output(
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

    pub(crate) fn draw_indicator_structural_output(
        &self,
        frame: &mut PaintInputs,
        layer: &ResolvedIndicatorLayer,
        value_type: data::chart::kline::OutputValueType,
        map_y: impl Fn(f64) -> Option<f32>,
    ) {
        let output = &layer.presentation_output;
        match value_type {
            data::chart::kline::OutputValueType::Ohlc => {
                let (body_width, wick_width) =
                    candlestick_widths(self.scene.logical_bar_spacing(), self.pixels);
                self.draw_indicator_ohlc_output(
                    frame,
                    output.instance_id,
                    output,
                    layer.required_input,
                    map_y,
                    body_width,
                    wick_width,
                );
            }
            data::chart::kline::OutputValueType::Range => {
                self.draw_indicator_range_output(frame, output, layer.required_input, map_y);
            }
            data::chart::kline::OutputValueType::Number
            | data::chart::kline::OutputValueType::Boolean => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_indicator_ohlc_output(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        output: &IndicatorOutputRef,
        required_input: Option<IndicatorInputSource>,
        map_y: impl Fn(f64) -> Option<f32>,
        body_width_phys: i32,
        wick_width_phys: i32,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        let body_width = pixels.logical_width_from_physical(body_width_phys);
        let wick_width = pixels.logical_width_from_physical(wick_width_phys);
        let line_width = pixels.quantized_stroke_width(1.0);
        self.indicators.for_each_scene_indicator_output_index(
            scene,
            output,
            |series, source_index, x_unit| {
                if !self.indicators.indicator_input_available_at(
                    series,
                    source_index,
                    required_input,
                ) {
                    return;
                }
                let Some(ohlc) = self.indicators.indicator_ohlc_output_for_series_at(
                    series,
                    output,
                    source_index,
                ) else {
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
                    .map(painting::resolve_indicator_color)
                    .unwrap_or_else(|| {
                        self.indicators.indicator_color(
                            Some(value_id),
                            self.indicators.indicator_style_slot_for_output(output),
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
        frame: &mut PaintInputs,
        output: &IndicatorOutputRef,
        required_input: Option<IndicatorInputSource>,
        map_y: impl Fn(f64) -> Option<f32>,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        let Some(series) = self.indicators.series.first() else {
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

        self.indicators.for_each_scene_indicator_output_index(
            scene,
            output,
            |_, source_index, x_unit| {
                if !self.indicators.indicator_input_available_at(
                    series,
                    source_index,
                    required_input,
                ) {
                    flush(&mut segments, &mut upper, &mut lower, &mut color);
                    return;
                }
                let Some(range) = self.indicators.indicator_range_output_for_series_at(
                    series,
                    output,
                    source_index,
                ) else {
                    flush(&mut segments, &mut upper, &mut lower, &mut color);
                    return;
                };
                let (Some(upper_value), Some(lower_value)) = (range.upper, range.lower) else {
                    flush(&mut segments, &mut upper, &mut lower, &mut color);
                    return;
                };
                let (Some(upper_y), Some(lower_y)) = (map_y(upper_value), map_y(lower_value))
                else {
                    flush(&mut segments, &mut upper, &mut lower, &mut color);
                    return;
                };
                if !upper_y.is_finite() || !lower_y.is_finite() {
                    flush(&mut segments, &mut upper, &mut lower, &mut color);
                    return;
                }
                let point_color = range
                    .color
                    .map(painting::resolve_indicator_color)
                    .unwrap_or_else(|| {
                        self.indicators.indicator_color_for_output_at(
                            series,
                            source_index,
                            output,
                            None,
                            self.indicators.indicator_style_slot_for_output(output),
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
            },
        );
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

    pub(crate) fn draw_indicator_regions(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        map_y: impl Fn(f64) -> Option<f32>,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        let Some(series) = self.indicators.series.first() else {
            return;
        };
        let Some(units) = scene.bar_units.first() else {
            return;
        };

        let point_count = series.point_count();
        let visible_range = units.index_range(scene.x.min_unit, scene.x.max_unit);
        let (visible_start, visible_end) = (visible_range.start, visible_range.end);
        let visible_start = units
            .global_index(visible_start)
            .unwrap_or(point_count)
            .min(point_count);
        let visible_end = units
            .global_index(visible_end)
            .unwrap_or(point_count)
            .min(point_count);

        let visible_range = {
            if visible_start < visible_end {
                visible_start..visible_end
            } else {
                0..point_count
            }
        };
        let plot = *scene.primary_plot();

        for region in self.indicators.indicator_regions(value_id, visible_range) {
            let Some(left_unit) = self.bar_unit_at_series_index(0, region.left_index) else {
                continue;
            };
            let right_x = if region.extend_right {
                plot.x
            } else {
                let Some(right_unit) = self.bar_unit_at_series_index(0, region.right_index) else {
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
                .map(painting::resolve_indicator_color)
                .or_else(|| {
                    region.fill_color_slot.as_deref().map(|slot| {
                        self.indicators
                            .indicator_color(Some(value_id), slot, palette, generated_accents)
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
                .map(painting::resolve_indicator_color)
                .or_else(|| {
                    region.border_color_slot.as_deref().map(|slot| {
                        self.indicators.indicator_color(
                            Some(value_id),
                            slot,
                            palette,
                            generated_accents,
                        )
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

    pub(crate) fn draw_indicator_markers_and_levels(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        map_y: impl Fn(f64) -> Option<f32>,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        self.draw_indicator_segments(frame, value_id, &map_y);
        scene.for_each_bar(
            self.indicators.series,
            |series_index, bar_index, series, x_unit, _point| {
                if series_index != 0 {
                    return;
                }
                for marker in self
                    .indicators
                    .indicator_markers_for_series_at(series, value_id, bar_index)
                {
                    let Some(value) = marker.value.filter(|value| value.is_finite()) else {
                        continue;
                    };
                    let Some(y) = map_y(value) else {
                        continue;
                    };
                    let color = marker
                        .color
                        .map(painting::resolve_indicator_color)
                        .unwrap_or_else(|| {
                            self.indicators.indicator_color(
                                Some(value_id),
                                self.indicators.indicator_default_style_slot(value_id),
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
        for level in self.indicators.indicator_levels(value_id) {
            let Some(value) = level.value.filter(|value| value.is_finite()) else {
                continue;
            };
            let Some(y) = map_y(value) else {
                continue;
            };
            let color = level
                .color
                .map(painting::resolve_indicator_color)
                .unwrap_or_else(|| {
                    self.indicators.indicator_color(
                        Some(value_id),
                        self.indicators.indicator_default_style_slot(value_id),
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
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        map_y: &impl Fn(f64) -> Option<f32>,
    ) {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let pixels = self.pixels;
        let scene = self.scene;
        let Some(series) = self.indicators.series.first() else {
            return;
        };
        let start = series
            .bar_index_at_or_before_unit(scene.x.axis, scene.x.min_unit)
            .unwrap_or(0);
        let end = series
            .bar_index_at_or_before_unit(scene.x.axis, scene.x.max_unit)
            .map_or(0, |index| index.saturating_add(1))
            .saturating_add(1)
            .min(series.point_count());
        let source_index_to_unit =
            |source_index: usize| self.bar_unit_at_series_index(0, source_index);
        for segment in self
            .indicators
            .indicator_segments(value_id, start..end, scene)
        {
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
            let segment_spec = self
                .indicators
                .indicator_segment_spec(value_id, &segment.id);
            let (line_width, stroke_style, fallback_color) = if let Some(spec) = segment_spec {
                (
                    pixels
                        .quantized_stroke_width(
                            self.indicators.indicator_line_width_for_style_slot(
                                value_id,
                                &spec.color_slot,
                                spec.line_width,
                            ),
                        )
                        .0,
                    self.indicators
                        .indicator_stroke_style_for_style_slot(value_id, &spec.color_slot),
                    self.indicators.indicator_color(
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
                    self.indicators.indicator_color(
                        Some(value_id),
                        self.indicators.indicator_default_style_slot(value_id),
                        palette,
                        generated_accents,
                    ),
                )
            };
            let color = segment
                .color
                .map(painting::resolve_indicator_color)
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
                frame.fill_text(canvas::Text {
                    content: text,
                    position: Point::new(end.x, end.y - 8.0),
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
        value_id: PanelValueId,
        fill: &IndicatorFillSpec,
        mut map_y: impl FnMut(f64) -> Option<f32>,
    ) -> Vec<BandSegment> {
        let scene = self.scene;
        let mut segments = Vec::new();
        let mut segment = Vec::new();
        let mut segment_style = None;

        let Some(series) = self.indicators.series.first() else {
            return segments;
        };
        let Some(units) = scene.bar_units.first() else {
            return segments;
        };
        let upper_channel = &fill.upper;
        let lower_channel = &fill.lower;
        let effective_offset = |channel_id: &str, explicit: i32| {
            if explicit != 0 {
                i64::from(explicit)
            } else {
                self.indicators
                    .indicator_output_spec_for_channel(value_id, channel_id)
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
            .saturating_sub(units.start_index())
            .min(units.values().len());
        for (source_index, &unit) in units.values().iter().enumerate().take(end) {
            let Some(source_index) = units.global_index(source_index) else {
                continue;
            };
            let upper_unit = scene.x.axis.output_unit(unit, upper_offset);
            if upper_unit >= scene.x.min_unit
                && upper_unit <= scene.x.max_unit
                && let Some(value) = self
                    .indicators
                    .indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        upper_channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_number)
            {
                let style = style_channel.and_then(|channel| {
                    self.indicators
                        .indicator_raw_output_value_for_series_at_channel(
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
            let lower_unit = scene.x.axis.output_unit(unit, lower_offset);
            if lower_unit >= scene.x.min_unit
                && lower_unit <= scene.x.max_unit
                && let Some(value) = self
                    .indicators
                    .indicator_raw_output_value_for_series_at_channel(
                        series,
                        value_id,
                        lower_channel,
                        source_index,
                    )
                    .and_then(OutputValue::as_number)
            {
                let style = style_channel.and_then(|channel| {
                    self.indicators
                        .indicator_raw_output_value_for_series_at_channel(
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
    ) -> iced::Color {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let upper_channel = &fill.upper;
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
                _ => self.indicators.style_slot_for_output(
                    value_id,
                    upper_channel,
                    Some(state),
                    &fill.color_slot,
                ),
            },
        );
        self.indicators
            .indicator_color(Some(value_id), style_slot, palette, generated_accents)
            .scale_alpha(fill.opacity.clamp(0.0, 1.0))
    }

    pub(crate) fn fill_indicator_panel_background(
        &self,
        frame: &mut PaintInputs,
        indicator_panel: &IndicatorPanelScene,
    ) {
        let scene = self.scene;
        if self
            .indicators
            .panel_indicator_value_ids(indicator_panel.panel_index)
            .iter()
            .all(|value_id| self.indicators.indicator_is_unavailable(Some(*value_id)))
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
        for value_id in self
            .indicators
            .panel_indicator_value_ids(indicator_panel.panel_index)
        {
            if !self
                .indicators
                .panel_indicator_visible(indicator_panel.panel_index, value_id)
                || self.indicators.indicator_is_unavailable(Some(value_id))
                || !self.indicators.indicator_fill_visible(value_id)
            {
                continue;
            }
            for fill in self
                .indicators
                .fill_specs_for_panel_value(Some(value_id))
                .iter()
            {
                frame.with_clip(panel_plot, |frame| {
                    self.fill_indicator_band(frame, value_id, fill, panel_plot, |value| {
                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                    });
                });
            }
        }
    }

    pub(crate) fn fill_primary_overlay_backgrounds(&self, frame: &mut PaintInputs) {
        let scene = self.scene;
        let Some(primary_panel_id) = self.indicators.composition.primary_panel_id() else {
            return;
        };
        let Some(primary_panel) = self.indicators.composition.panel(primary_panel_id) else {
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
                if self.indicators.indicator_is_unavailable(Some(value_id))
                    || !self.indicators.indicator_fill_visible(value_id)
                    || !filled_indicators.insert(value_id)
                {
                    continue;
                }
                for fill in self
                    .indicators
                    .fill_specs_for_panel_value(Some(value_id))
                    .iter()
                {
                    self.fill_indicator_band(frame, value_id, fill, primary_plot, |value| {
                        scene.map_primary_plot_with_anchor(value, base_series_anchor)
                    });
                }
            }
        });
    }

    fn fill_indicator_band(
        &self,
        frame: &mut PaintInputs,
        value_id: PanelValueId,
        fill: &IndicatorFillSpec,
        plot: Rectangle,
        mut map_y: impl FnMut(f64) -> Option<f32>,
    ) {
        match fill.geometry {
            IndicatorFillGeometry::PanelBand => {
                let Some(upper_y) = self
                    .indicators
                    .indicator_source_value_for_latest_bar_with_offset(
                        value_id,
                        &fill.upper,
                        fill.upper_offset_bars,
                    )
                    .and_then(&mut map_y)
                else {
                    return;
                };
                let Some(lower_y) = self
                    .indicators
                    .indicator_source_value_for_latest_bar_with_offset(
                        value_id,
                        &fill.lower,
                        fill.lower_offset_bars,
                    )
                    .and_then(&mut map_y)
                else {
                    return;
                };
                let color = self.indicator_fill_color(value_id, fill, None);
                fill_static_band(frame, plot, upper_y, lower_y, color, self.pixels);
            }
            IndicatorFillGeometry::SeriesBand => {
                let segments = self.band_segments(value_id, fill, map_y);
                fill_band_segments(frame, segments, self.pixels, |style| {
                    self.indicator_fill_color(value_id, fill, style)
                });
            }
        }
    }

    fn format_table_value(value: OutputValue, format: TableColumnFormat) -> String {
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

    fn table_value_color(
        &self,
        color: &TableColumnColor,
        value: OutputValue,
        value_id: PanelValueId,
    ) -> Option<iced::Color> {
        let generated_accents = self.generated_accents;
        let palette = self.palette;
        let color_slot = table_color_slot(color, value)?;
        Some(self.indicators.indicator_color(
            Some(value_id),
            color_slot,
            palette,
            generated_accents,
        ))
    }

    fn table_value_alpha(value: f64, max_abs_value: f64) -> f32 {
        let relative = if max_abs_value > f64::EPSILON {
            (value.abs() / max_abs_value).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (0.1 + relative * 0.7) as f32
    }

    pub(crate) fn fill_table_panel(
        &self,
        frame: &mut PaintInputs,
        indicator_panel: &IndicatorPanelScene,
    ) {
        let palette = self.palette;
        let scene = self.scene;
        let Some(value_id) = indicator_panel.value_id else {
            return;
        };
        if !self
            .indicators
            .panel_indicator_visible(indicator_panel.panel_index, value_id)
        {
            return;
        }
        let Some(indicator) = self.indicators.indicator_kind(value_id) else {
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
                scene.for_each_bar(
                    self.indicators.series,
                    |series_index, bar_index, series, x_unit, _bar| {
                        if series_index != 0
                            || x_unit < scene.x.min_unit
                            || x_unit > scene.x.max_unit
                        {
                            return;
                        }
                        let Some(value) = self
                            .indicators
                            .indicator_output_value_for_series_at_channel(
                                series,
                                value_id,
                                &column.value,
                                bar_index,
                            )
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
                        let Some(color) = self.table_value_color(&column.color, value, value_id)
                        else {
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
                        let Some(color) =
                            self.table_value_color(&column.color, color_value, value_id)
                        else {
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

    fn map_indicator_plot_for_output(
        &self,
        panel_index: usize,
        output: &IndicatorOutputRef,
        value: f64,
    ) -> Option<f32> {
        let scene = self.scene;
        let independent_scale = self
            .indicators
            .indicator_output_spec(output)
            .is_some_and(|spec| spec.independent_scale);
        scene.map_indicator_plot_scaled(panel_index, value, independent_scale)
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
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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

fn fill_static_band(
    frame: &mut PaintInputs,
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
    frame: &mut PaintInputs,
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

pub(crate) fn candlestick_widths(bar_spacing: f32, pixels: PixelGeometry) -> (i32, i32) {
    let pixel_ratio = pixels.ratio();
    let from = 2.5_f32;
    let to = 4.0_f32;
    let coeff_special = 3.0_f32;

    let mut body_width = if bar_spacing >= from && bar_spacing <= to {
        (coeff_special * pixel_ratio).floor() as i32
    } else {
        let reducing_coeff = 0.2_f32;
        let coeff = 1.0
            - (reducing_coeff * (bar_spacing.max(to) - to).atan()) / (std::f32::consts::PI * 0.5);
        let res = (bar_spacing * coeff * pixel_ratio).floor() as i32;
        let scaled_bar_spacing = (bar_spacing * pixel_ratio).floor() as i32;
        res.min(scaled_bar_spacing).max(pixel_ratio.floor() as i32)
    };

    let max_width = (bar_spacing * pixel_ratio).floor() as i32;
    body_width = body_width.clamp(1, max_width.max(1));
    if body_width >= 2 {
        let wick_width = pixel_ratio.floor() as i32;
        if (wick_width & 1) != (body_width & 1) {
            body_width -= 1;
        }
    }
    let wick_width = (pixel_ratio.floor() as i32).max(1).min(body_width);

    (body_width.max(1), wick_width)
}
