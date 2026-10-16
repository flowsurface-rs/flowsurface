use super::footprint::{
    FootprintColumnLayout, FootprintImbalance, draw_footprint_candle, draw_footprint_cluster_row,
    footprint_text_column_is_enabled,
};
use super::scene::IndicatorPanelScene;
use super::*;
use data::chart::kline::{
    IndicatorColorSlot, IndicatorFillGeometry, IndicatorFillSpec, LineStrokeStyle,
};
use data::util::abbr_large_numbers;

const FOOTPRINT_MIN_COLUMN_WIDTH_PX: f32 = 24.0;
const FOOTPRINT_CELL_INSET_PX: f32 = 2.0;
const TABLE_TEXT_MIN_COLUMN_WIDTH_PX: f32 = 48.0;
const TABLE_TEXT_MIN_ROW_HEIGHT_PX: f32 = 9.0;
const TABLE_CELL_INSET_PX: f32 = 1.0;
const DASHED_LINE_SEGMENTS: [f32; 2] = [8.0, 5.0];
const DOTTED_LINE_SEGMENTS: [f32; 2] = [1.0, 4.0];

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

fn footprint_detail_enabled(footprints_enabled: bool, logical_bar_spacing: f32) -> bool {
    footprints_enabled && logical_bar_spacing >= FOOTPRINT_MIN_COLUMN_WIDTH_PX
}

#[derive(Debug, Clone, Copy)]
struct BandPoint {
    upper: Point,
    lower: Point,
}

fn fill_static_band(
    frame: &mut canvas::Frame,
    plot: Rectangle,
    upper_y: f32,
    lower_y: f32,
    color: iced::Color,
) {
    let top = upper_y.min(lower_y);
    let height = (upper_y - lower_y).abs();
    if !top.is_finite() || !height.is_finite() || height <= 0.0 {
        return;
    }

    frame.fill_rectangle(
        Point::new(plot.x, top),
        Size::new(plot.width, height),
        color,
    );
}

fn fill_band_segments(
    frame: &mut canvas::Frame,
    segments: Vec<Vec<BandPoint>>,
    color: iced::Color,
) {
    for segment in segments.into_iter().filter(|segment| segment.len() >= 2) {
        let path = canvas::Path::new(|builder| {
            builder.move_to(segment[0].upper);
            for point in segment.iter().skip(1) {
                builder.line_to(point.upper);
            }
            for point in segment.iter().rev() {
                builder.line_to(point.lower);
            }
            builder.close();
        });

        frame.fill(&path, color);
    }
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn primary_plot_detail(&self, scene: &Scene) -> PrimaryPlotDetail {
        if footprint_detail_enabled(self.footprints_enabled, scene.logical_bar_spacing()) {
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
        let current_y = scene.map_primary_plot_with_anchor(price.to_f32_lossy(), anchor);
        let next_y = scene
            .map_primary_plot_with_anchor(price.add_steps(1, tick_size).to_f32_lossy(), anchor);
        (current_y - next_y).abs()
    }

    fn band_segments(
        &self,
        scene: &Scene,
        value_id: PanelValueId,
        fill: IndicatorFillSpec,
        mut map_y: impl FnMut(f32) -> Option<f32>,
    ) -> Vec<Vec<BandPoint>> {
        let mut segments = Vec::new();
        let mut segment = Vec::new();

        self.for_each_scene_bar_unit_index(
            scene,
            |series_index, bar_index, series, x_unit, _bar| {
                if series_index != 0 || x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                    return;
                }

                let Some(data) =
                    self.indicator_data_for_series_at(series, Some(value_id), bar_index)
                else {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                };

                let Some(upper_value) =
                    self.indicator_source_value(value_id, Some(data), fill.upper)
                else {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                };
                let Some(lower_value) =
                    self.indicator_source_value(value_id, Some(data), fill.lower)
                else {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                };

                let Some(upper_y) = map_y(upper_value) else {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                };
                let Some(lower_y) = map_y(lower_value) else {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                };

                if !upper_y.is_finite() || !lower_y.is_finite() {
                    if !segment.is_empty() {
                        segments.push(std::mem::take(&mut segment));
                    }
                    return;
                }

                segment.push(BandPoint {
                    upper: Point::new(scene.map_x_plot(x_unit), upper_y),
                    lower: Point::new(scene.map_x_plot(x_unit), lower_y),
                });
            },
        );

        if !segment.is_empty() {
            segments.push(segment);
        }

        segments
    }

    fn fill_indicator_panel_background(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        indicator_panel: &IndicatorPanelScene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) {
        if self.indicator_is_unavailable(indicator_panel.value_id)
            || matches!(indicator_panel.presentation, PanelPresentation::Table)
        {
            return;
        }

        let Some(value_id) = indicator_panel.value_id else {
            return;
        };
        if !self.indicator_fill_visible(value_id) {
            return;
        }
        let Some(panel_plot) = scene
            .layout
            .panel(indicator_panel.panel_index)
            .map(|panel| panel.plot)
        else {
            return;
        };

        for fill in self
            .fill_specs_for_panel_value(Some(value_id))
            .iter()
            .copied()
        {
            let color = self
                .indicator_color(Some(value_id), fill.color_slot, palette, generated_accents)
                .scale_alpha(fill.opacity.clamp(0.0, 1.0));

            frame.with_clip(panel_plot, |frame| match fill.geometry {
                IndicatorFillGeometry::PanelBand => {
                    let Some(upper_value) = self.indicator_source_value(value_id, None, fill.upper)
                    else {
                        return;
                    };
                    let Some(lower_value) = self.indicator_source_value(value_id, None, fill.lower)
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

                    fill_static_band(frame, panel_plot, upper_y, lower_y, color);
                }
                IndicatorFillGeometry::SeriesBand => {
                    let segments = self.band_segments(scene, value_id, fill, |value| {
                        scene.map_indicator_plot(indicator_panel.panel_index, value)
                    });
                    fill_band_segments(frame, segments, color);
                }
            });
        }
    }

    fn fill_primary_overlay_backgrounds(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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
            for layer in &primary_panel.layers {
                if !layer.visible {
                    continue;
                }

                let Some(value_id) = layer.source.indicator_value_id() else {
                    continue;
                };
                if self.indicator_is_unavailable(Some(value_id)) {
                    continue;
                }
                if !self.indicator_fill_visible(value_id) {
                    continue;
                }

                for fill in self
                    .fill_specs_for_panel_value(Some(value_id))
                    .iter()
                    .copied()
                {
                    let color = self
                        .indicator_color(
                            Some(value_id),
                            fill.color_slot,
                            palette,
                            generated_accents,
                        )
                        .scale_alpha(fill.opacity.clamp(0.0, 1.0));

                    match fill.geometry {
                        IndicatorFillGeometry::PanelBand => {
                            let Some(upper_value) =
                                self.indicator_source_value(value_id, None, fill.upper)
                            else {
                                continue;
                            };
                            let Some(lower_value) =
                                self.indicator_source_value(value_id, None, fill.lower)
                            else {
                                continue;
                            };
                            let upper_y =
                                scene.map_primary_plot_with_anchor(upper_value, base_series_anchor);
                            let lower_y =
                                scene.map_primary_plot_with_anchor(lower_value, base_series_anchor);
                            fill_static_band(frame, primary_plot, upper_y, lower_y, color);
                        }
                        IndicatorFillGeometry::SeriesBand => {
                            let segments = self.band_segments(scene, value_id, fill, |value| {
                                Some(scene.map_primary_plot_with_anchor(value, base_series_anchor))
                            });
                            fill_band_segments(frame, segments, color);
                        }
                    }
                }
            }
        });
    }

    pub(super) fn candlestick_color(
        &self,
        bullish: bool,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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
        generated_accents: &[iced::Color; 6],
        draw_clusters: bool,
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
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.x.min_unit && x_unit <= scene.x.max_unit;
        let mut max_cluster_qty = 0.0f64;

        if draw_clusters {
            self.for_each_scene_bar_unit_index(scene, |series_index, _, series, x_unit, bar| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let Some(footprint) = series.footprint_for_bar(bar) else {
                    return;
                };

                for group in footprint.trades.values() {
                    max_cluster_qty = max_cluster_qty
                        .max(group.max_cluster_qty(self.footprint.cluster_kind).to_f64());
                }
            });
        }

        let primary_plot = *scene.primary_plot();
        let footprint_width =
            (scene.logical_bar_spacing() - FOOTPRINT_CELL_INSET_PX * 2.0).max(1.0);
        let imbalance = self.footprint.imbalance.show.then(|| {
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
        let show_text = footprint_text_column_is_enabled(footprint_width, imbalance.is_some());
        let draw_candlesticks = self.candlestick.visible
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_));

        frame.with_clip(primary_plot, |frame| {
            self.for_each_scene_bar_unit_index(scene, |series_index, _, series, x_unit, bar| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let series_anchor = scene
                    .primary
                    .series_percent_anchors
                    .first()
                    .copied()
                    .flatten();
                let y_open =
                    scene.map_primary_plot_with_anchor(bar.open.to_f32_lossy(), series_anchor);
                let y_high =
                    scene.map_primary_plot_with_anchor(bar.high.to_f32_lossy(), series_anchor);
                let y_low =
                    scene.map_primary_plot_with_anchor(bar.low.to_f32_lossy(), series_anchor);
                let y_close =
                    scene.map_primary_plot_with_anchor(bar.close.to_f32_lossy(), series_anchor);
                let Some(footprint) = series.footprint_for_bar(bar) else {
                    if draw_candlesticks {
                        let layout = FootprintColumnLayout::new(
                            scene.map_x_plot(x_unit),
                            footprint_width,
                            false,
                        );
                        draw_footprint_candle(
                            frame,
                            layout,
                            y_open,
                            y_high,
                            y_low,
                            y_close,
                            self.candlestick_color(
                                bar.close >= bar.open,
                                palette,
                                generated_accents,
                            ),
                        );
                    }
                    return;
                };
                let x_center = scene.map_x_plot(x_unit);
                let layout =
                    FootprintColumnLayout::new(x_center, footprint_width, imbalance.is_some());
                let cluster_scale = self.footprint.cluster_scaling.effective_qty(
                    max_cluster_qty,
                    footprint.max_cluster_qty_all(self.footprint.cluster_kind),
                );

                if draw_clusters && cluster_scale > f64::EPSILON {
                    for (price, group) in &footprint.trades {
                        let row_height = self
                            .footprint_row_height(scene, *price, tick_size, anchor)
                            .max(1.0);
                        let y = scene.map_primary_plot_with_anchor(price.to_f32_lossy(), anchor);
                        draw_footprint_cluster_row(
                            frame,
                            self.footprint.cluster_kind,
                            layout,
                            y,
                            row_height,
                            group.sell_qty.to_f64(),
                            group.buy_qty.to_f64(),
                            cluster_scale,
                            show_text,
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
                        self.candlestick_color(bar.close >= bar.open, palette, generated_accents),
                    );
                }
            });
        });
    }

    pub(super) fn draw_profile_distribution(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
        distribution: &PriceLevelTradeDistribution,
        colors: VolumeProfileColorConfig,
        tick_size: PriceStep,
        profile_edge: f32,
        profile_width: f32,
        bar_alpha: f32,
        opacity_scale: f32,
        line_start: f32,
        naked_line_end: f32,
        grow_right: bool,
    ) {
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

        if profile_width > 0.0 {
            for level in &distribution.levels {
                let total_qty = level.total_qty().to_f64();
                if total_qty <= 0.0 {
                    continue;
                }

                let total_width = profile_width * (total_qty / max_qty).clamp(0.0, 1.0) as f32;
                if total_width <= 0.0 {
                    continue;
                }

                let y = scene.map_primary_plot_with_anchor(level.price.to_f32_lossy(), anchor);
                let next_y = scene.map_primary_plot_with_anchor(
                    level.price.add_steps(1, tick_size).to_f32_lossy(),
                    anchor,
                );
                let row_height = (next_y - y).abs().max(1.0);
                let row_top = y - row_height * 0.5;
                let row_height = (row_height - 1.0).max(1.0);

                let sell_width =
                    total_width * (level.sell_qty.to_f64() / total_qty).clamp(0.0, 1.0) as f32;
                let buy_width = total_width - sell_width;
                let left = if grow_right {
                    profile_edge
                } else {
                    profile_edge - total_width
                };

                if sell_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left, row_top),
                        Size::new(sell_width, row_height),
                        sell_color,
                    );
                }
                if buy_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left + sell_width, row_top),
                        Size::new(buy_width, row_height),
                        buy_color,
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
        let line_left = line_start.min(line_end);
        let line_width = (line_end - line_start).abs();
        if line_width <= 1.0 {
            return;
        }

        let poc_y = scene.map_primary_plot_with_anchor(poc.price.to_f32_lossy(), anchor);
        let poc_next_y = scene
            .map_primary_plot_with_anchor(poc.price.add_steps(1, tick_size).to_f32_lossy(), anchor);
        let line_height = (poc_next_y - poc_y).abs().clamp(0.5, 1.0);

        frame.fill_rectangle(
            Point::new(line_left, poc_y - line_height * 0.5),
            Size::new(line_width, line_height),
            line_color,
        );
    }

    pub(super) fn fill_footprint_poc_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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
            XAxis::Tick { .. } => (
                UnixMs::new(
                    base_series
                        .footprint_tick_index_offset()
                        .saturating_add(scene.x.min_unit.max(0) as u64),
                ),
                UnixMs::new(
                    base_series
                        .footprint_tick_index_offset()
                        .saturating_add(scene.x.max_unit.max(0) as u64),
                ),
            ),
        };
        let lines = base_series.footprint_poc_lines(earliest, latest);
        if lines.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let tick_size = base_series.footprint_tick_size();
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
        let x_for = |value: UnixMs| -> Option<f32> {
            match scene.x.axis {
                XAxis::Time { .. } => scene.x.axis.unit_from_time_f32(value),
                XAxis::Tick { .. } => Some(
                    scene.x.axis.unit_from_tick(TickIndex::from_u64(
                        value
                            .as_u64()
                            .checked_sub(base_series.footprint_tick_index_offset())?,
                    )) as f32,
                ),
            }
            .map(|unit| scene.map_x_plot_f32(unit))
        };
        frame.with_clip(primary_plot, |frame| {
            for line in lines {
                let Some(start_x) = x_for(line.start) else {
                    continue;
                };
                let (end_x, color) = match line.poc.status {
                    NPoc::Naked if config.show_npoc => (naked_line_end, npoc_color),
                    NPoc::Filled { at } if config.show_poc => {
                        (x_for(UnixMs::new(at)).unwrap_or(naked_line_end), poc_color)
                    }
                    NPoc::Filled { .. } => continue,
                    NPoc::None => continue,
                    _ => continue,
                };
                let line_width = (end_x - start_x).abs();
                if line_width <= 1.0 {
                    continue;
                }

                let poc_y =
                    scene.map_primary_plot_with_anchor(line.poc.price.to_f32_lossy(), anchor);
                let next_y = scene.map_primary_plot_with_anchor(
                    line.poc.price.add_steps(1, tick_size).to_f32_lossy(),
                    anchor,
                );
                let line_height = (next_y - poc_y).abs().clamp(0.5, 1.0);
                frame.fill_rectangle(
                    Point::new(start_x.min(end_x), poc_y - line_height * 0.5),
                    Size::new(line_width, line_height),
                    color,
                );
            }
        });
    }

    fn effective_study_tick_size(
        &self,
        study: KlineStudy,
        footprint_tick_size: PriceStep,
    ) -> PriceStep {
        let Some(multiplier) = study.volume_profile_tick_multiplier() else {
            return footprint_tick_size;
        };
        let Some(base_series) = self.series.first() else {
            return footprint_tick_size;
        };
        let step = multiplier.multiply_with_min_tick_step(*base_series.ticker_info());
        if step.units < footprint_tick_size.units {
            footprint_tick_size
        } else {
            step
        }
    }

    pub(super) fn fill_volume_profile_overlay(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
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

        let (lowest_value, highest_value) = scene.primary_domain_value_range();
        let (lowest, highest) = (
            Price::from_f32(lowest_value),
            Price::from_f32(highest_value),
        );
        let footprint_tick_size = base_series.footprint_tick_size();
        let visible_profile = self.studies.iter().find_map(|study| {
            if !study.visible() || !study.is_volume_profile() {
                return None;
            }

            if !matches!(
                study.volume_profile_schedule(),
                VolumeProfileSchedule::VisibleRange
            ) {
                return None;
            }

            let tick_size = self.effective_study_tick_size(*study, footprint_tick_size);
            base_series
                .trade_price_distribution(earliest, latest, lowest, highest, tick_size)
                .map(|profile| {
                    (
                        profile,
                        study.volume_profile_colors(),
                        study.volume_profile_width_ratio(),
                        study.volume_profile_opacity_ratio(),
                        study.volume_profile_opacity_scale(),
                        tick_size,
                    )
                })
        });
        let recurring_intervals = self
            .studies
            .iter()
            .filter(|study| study.visible() && study.is_volume_profile())
            .filter_map(|study| match study.volume_profile_schedule() {
                VolumeProfileSchedule::VisibleRange => None,
                VolumeProfileSchedule::Recurring { interval }
                    if study.is_interval_available(self.basis) =>
                {
                    Some((
                        interval,
                        study.volume_profile_colors(),
                        study.volume_profile_width_ratio(),
                        study.volume_profile_opacity_ratio(),
                        study.volume_profile_opacity_scale(),
                        self.effective_study_tick_size(*study, footprint_tick_size),
                    ))
                }
                VolumeProfileSchedule::Recurring { .. } => None,
            });
        let profiles = recurring_intervals
            .flat_map(
                |(interval, colors, width_ratio, opacity_ratio, opacity_scale, tick_size)| {
                    base_series
                        .recurring_trade_price_distributions(interval, tick_size, earliest, latest)
                        .into_iter()
                        .map(move |profile| {
                            (
                                profile,
                                colors,
                                width_ratio,
                                opacity_ratio,
                                opacity_scale,
                                tick_size,
                            )
                        })
                },
            )
            .collect::<Vec<_>>();
        if visible_profile.is_none() && profiles.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            if let Some((profile, colors, width_ratio, opacity_ratio, opacity_scale, tick_size)) =
                visible_profile.as_ref()
            {
                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    profile,
                    *colors,
                    *tick_size,
                    primary_plot.x + primary_plot.width,
                    primary_plot.width * width_ratio,
                    *opacity_ratio,
                    *opacity_scale,
                    primary_plot.x,
                    primary_plot.x + primary_plot.width,
                    false,
                );
            }

            for (anchored_profile, colors, width_ratio, opacity_ratio, opacity_scale, tick_size) in
                profiles
            {
                let Some(start_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.start)
                else {
                    continue;
                };
                let Some(end_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.end) else {
                    continue;
                };

                let segment_left = scene.map_x_plot_f32(start_unit);
                let segment_right = scene.map_x_plot_f32(end_unit);
                let segment_width = (segment_right - segment_left).abs();
                let profile_width = segment_width * width_ratio;
                if profile_width <= 0.0 {
                    continue;
                }

                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    &anchored_profile.distribution,
                    colors,
                    tick_size,
                    segment_left,
                    profile_width,
                    opacity_ratio,
                    opacity_scale,
                    segment_left,
                    primary_plot.x + primary_plot.width,
                    true,
                );
            }
        });
    }

    pub(super) fn table_column_value(
        data: IndicatorData,
        column: TableColumnValue,
    ) -> Option<IndicatorCellValue> {
        match column {
            TableColumnValue::Primary => Some(data.primary_value()),
            TableColumnValue::Field(key) => data.field_value(key),
        }
    }

    pub(super) fn format_table_value(
        value: IndicatorCellValue,
        format: TableColumnFormat,
    ) -> String {
        match (value, format) {
            (IndicatorCellValue::Number(value), TableColumnFormat::Quantity)
                if value.is_finite() =>
            {
                abbr_large_numbers(value as f64)
            }
            (IndicatorCellValue::Number(value), TableColumnFormat::Integer)
                if value.is_finite() =>
            {
                format!("{value:.0}")
            }
            (IndicatorCellValue::Number(value), TableColumnFormat::Percent)
                if value.is_finite() =>
            {
                format!("{value:+.1}%")
            }
            (IndicatorCellValue::Boolean(true), TableColumnFormat::Boolean) => "✓".to_string(),
            (IndicatorCellValue::Boolean(false), TableColumnFormat::Boolean) => String::new(),
            _ => String::new(),
        }
    }

    pub(super) fn table_value_color(
        &self,
        value: IndicatorCellValue,
        color: TableColumnColor,
        value_id: PanelValueId,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) -> iced::Color {
        let slot = match color {
            TableColumnColor::Neutral | TableColumnColor::Primary => IndicatorColorSlot::Main,
            TableColumnColor::Positive => IndicatorColorSlot::Positive,
            TableColumnColor::Negative => IndicatorColorSlot::Negative,
            TableColumnColor::Signed if matches!(value, IndicatorCellValue::Number(value) if value >= 0.0) => {
                IndicatorColorSlot::Positive
            }
            TableColumnColor::Signed => IndicatorColorSlot::Negative,
        };
        self.indicator_color(Some(value_id), slot, palette, generated_accents)
    }

    pub(super) fn table_value_alpha(value: f32, max_abs_value: f32) -> f32 {
        let relative = if max_abs_value > f32::EPSILON {
            (value.abs() / max_abs_value).clamp(0.0, 1.0)
        } else {
            0.0
        };
        0.1 + relative * 0.7
    }

    pub(super) fn fill_table_panel(
        &self,
        frame: &mut canvas::Frame,
        scene: &Scene,
        indicator_panel: &scene::IndicatorPanelScene,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) {
        let Some(value_id) = indicator_panel.value_id else {
            return;
        };
        let Some(indicator) = self.indicator_kind(value_id) else {
            return;
        };
        let columns = table_columns(indicator);
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
            .clamp(FOOTPRINT_TEXT_MIN_SIZE_PX, TEXT_SIZE);
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
                        let Some(data) =
                            self.indicator_data_for_series_at(series, Some(value_id), bar_index)
                        else {
                            return;
                        };
                        let Some(value) = Self::table_column_value(data, column.value) else {
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
                        frame.fill_text(canvas::Text {
                            content,
                            position: Point::new(x, row_center),
                            color: self.table_value_color(
                                value,
                                column.color,
                                value_id,
                                palette,
                                generated_accents,
                            ),
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
                            IndicatorCellValue::Number(value) => Some(value.abs()),
                            IndicatorCellValue::Boolean(_) => None,
                        })
                        .fold(0.0, f32::max);

                    for (x, value) in values {
                        let (color, alpha) = match value {
                            IndicatorCellValue::Number(value) => (
                                self.table_value_color(
                                    IndicatorCellValue::Number(value),
                                    column.color,
                                    value_id,
                                    palette,
                                    generated_accents,
                                ),
                                Self::table_value_alpha(value, max_abs_value),
                            ),
                            IndicatorCellValue::Boolean(true) => (
                                self.table_value_color(
                                    value,
                                    column.color,
                                    value_id,
                                    palette,
                                    generated_accents,
                                ),
                                0.85,
                            ),
                            IndicatorCellValue::Boolean(false) => continue,
                        };
                        let color = color.scale_alpha(alpha);
                        frame.fill_rectangle(
                            Point::new(x - cell_width * 0.5, row_top + TABLE_CELL_INSET_PX),
                            Size::new(
                                cell_width,
                                (row_height - TABLE_CELL_INSET_PX * 2.0).max(1.0),
                            ),
                            color,
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
        generated_accents: &[iced::Color; 6],
        plot_origin_in_window: Point,
    ) {
        let pixel_ratio = self.pixel_ratio_value();
        let plot_origin_x_global = plot_origin_in_window.x;
        let plot_origin_y_global = plot_origin_in_window.y;
        let primary_plot_detail = self.primary_plot_detail(scene);

        let spacing = scene.bar_spacing_px();
        let px_per_unit = spacing.as_f32();
        let max_width_phys = ((px_per_unit * pixel_ratio).floor() as i32).max(1);

        let mut candle_body_width_phys =
            Self::candlestick_width(px_per_unit, pixel_ratio).clamp(1, max_width_phys);
        let wick_width_phys = (pixel_ratio.floor() as i32)
            .max(1)
            .min(candle_body_width_phys);

        if candle_body_width_phys >= 2 && ((wick_width_phys ^ candle_body_width_phys) & 1) != 0 {
            candle_body_width_phys -= 1;
        }

        let indicator_width_phys = candle_body_width_phys;
        let indicator_width = Self::logical_width_from_physical(indicator_width_phys, pixel_ratio);
        let candle_body_width =
            Self::logical_width_from_physical(candle_body_width_phys, pixel_ratio);
        let wick_width = Self::logical_width_from_physical(wick_width_phys, pixel_ratio);
        let in_visible_range =
            |x_unit: i64| x_unit >= scene.x.min_unit && x_unit <= scene.x.max_unit;

        for indicator_panel in &scene.indicator_panels {
            self.fill_indicator_panel_background(
                frame,
                scene,
                indicator_panel,
                palette,
                generated_accents,
            );
        }
        self.fill_primary_overlay_backgrounds(frame, scene, palette, generated_accents);

        for indicator_panel in &scene.indicator_panels {
            if self.indicator_is_unavailable(indicator_panel.value_id) {
                continue;
            }

            if matches!(indicator_panel.presentation, PanelPresentation::Table) {
                self.fill_table_panel(frame, scene, indicator_panel, palette, generated_accents);
                continue;
            }

            if !matches!(indicator_panel.mark, MarkKind::Candle | MarkKind::Bar(_)) {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            let Some(y_indicator_baseline) = scene
                .map_indicator_plot(indicator_panel.panel_index, 0.0)
                .or_else(|| scene.indicator_panel_bottom(indicator_panel.panel_index))
            else {
                continue;
            };

            frame.with_clip(panel_plot, |frame| {
                self.for_each_scene_bar_unit_index(
                    scene,
                    |series_index, bar_index, series, x_unit, _bar| {
                        if series_index != 0 || !in_visible_range(x_unit) {
                            return;
                        }

                        let Some(indicator_data) = self.indicator_data_for_series_at(
                            series,
                            indicator_panel.value_id,
                            bar_index,
                        ) else {
                            return;
                        };

                        let indicator_value = indicator_data.value();
                        let Some(y_indicator_value) =
                            scene.map_indicator_plot(indicator_panel.panel_index, indicator_value)
                        else {
                            return;
                        };

                        let x_plot = scene.map_x_plot(x_unit);
                        let indicator_left = Self::centered_left_for_width_with_origin(
                            x_plot,
                            indicator_width_phys,
                            pixel_ratio,
                            plot_origin_x_global,
                        );
                        let indicator_top = y_indicator_value.min(y_indicator_baseline);
                        let indicator_height =
                            (y_indicator_baseline - y_indicator_value).abs().max(1.0);

                        if matches!(indicator_panel.data_kind, LayerDataKind::Histogram)
                            && matches!(
                                indicator_panel.mark,
                                MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
                            )
                        {
                            if let Some(overlay) = indicator_data.signed_overlay() {
                                let base_color = self.indicator_color(
                                    indicator_panel.value_id,
                                    if overlay >= 0.0 {
                                        IndicatorColorSlot::Positive
                                    } else {
                                        IndicatorColorSlot::Negative
                                    },
                                    palette,
                                    generated_accents,
                                );

                                frame.fill_rectangle(
                                    Point::new(indicator_left, indicator_top),
                                    Size::new(indicator_width, indicator_height),
                                    base_color.scale_alpha(0.3),
                                );

                                let overlay_abs = overlay.abs();
                                if overlay_abs > 0.0
                                    && let Some(y_overlay) = scene.map_indicator_plot(
                                        indicator_panel.panel_index,
                                        overlay_abs,
                                    )
                                {
                                    frame.fill_rectangle(
                                        Point::new(
                                            indicator_left,
                                            y_overlay.min(y_indicator_baseline),
                                        ),
                                        Size::new(
                                            indicator_width,
                                            (y_indicator_baseline - y_overlay).abs().max(1.0),
                                        ),
                                        base_color,
                                    );
                                }
                            } else {
                                frame.fill_rectangle(
                                    Point::new(indicator_left, indicator_top),
                                    Size::new(indicator_width, indicator_height),
                                    self.indicator_color(
                                        indicator_panel.value_id,
                                        IndicatorColorSlot::Bars,
                                        palette,
                                        generated_accents,
                                    ),
                                );
                            }
                        } else {
                            frame.fill_rectangle(
                                Point::new(indicator_left, indicator_top),
                                Size::new(indicator_width, indicator_height),
                                self.indicator_color(
                                    indicator_panel.value_id,
                                    IndicatorColorSlot::Bars,
                                    palette,
                                    generated_accents,
                                )
                                .scale_alpha(0.5),
                            );
                        }
                    },
                );
            });
        }

        if let Some(primary_panel_id) = self.composition.primary_panel_id()
            && let Some(primary_panel) = self.composition.panel(primary_panel_id)
        {
            self.fill_footprint_poc_overlay(frame, scene, palette, generated_accents);
            self.fill_volume_profile_overlay(frame, scene, palette, generated_accents);

            let base_series_anchor = scene
                .primary
                .series_percent_anchors
                .first()
                .copied()
                .flatten();
            let primary_plot = *scene.primary_plot();

            frame.with_clip(primary_plot, |frame| {
                for layer in &primary_panel.layers {
                    if !layer.visible {
                        continue;
                    }

                    let Some(value_id) = layer.source.indicator_value_id() else {
                        continue;
                    };
                    if self.indicator_is_unavailable(Some(value_id)) {
                        continue;
                    }

                    for channel in self
                        .overlay_channels_for_panel_value(Some(value_id))
                        .iter()
                        .copied()
                    {
                        let (channel_line_width, channel_line_width_phys) =
                            Self::quantized_stroke_width(
                                self.indicator_line_width(Some(value_id), channel),
                                pixel_ratio,
                            );
                        let mut point_count = 0usize;
                        let path = canvas::Path::new(|builder| {
                            self.for_each_scene_bar_unit_index(
                                scene,
                                |series_index, bar_index, series, x_unit, _bar| {
                                    if series_index != 0 || !in_visible_range(x_unit) {
                                        return;
                                    }

                                    let Some(data) = self.indicator_data_for_series_at(
                                        series,
                                        Some(value_id),
                                        bar_index,
                                    ) else {
                                        return;
                                    };

                                    let Some(value) =
                                        self.overlay_channel_value(value_id, data, channel)
                                    else {
                                        return;
                                    };

                                    let point = Point::new(
                                        Self::snap_plot_x_for_stroke_with_origin(
                                            scene.map_x_plot(x_unit),
                                            pixel_ratio,
                                            plot_origin_x_global,
                                            channel_line_width_phys,
                                        ),
                                        Self::snap_stroke_center_with_origin(
                                            scene.map_primary_plot_with_anchor(
                                                value,
                                                base_series_anchor,
                                            ),
                                            pixel_ratio,
                                            plot_origin_y_global,
                                            channel_line_width_phys,
                                        ),
                                    );

                                    if point_count == 0 {
                                        builder.move_to(point);
                                    } else {
                                        builder.line_to(point);
                                    }
                                    point_count += 1;
                                },
                            );
                        });

                        if point_count < 2 {
                            continue;
                        }

                        frame.stroke(
                            &path,
                            indicator_line_stroke(
                                channel_line_width,
                                self.indicator_stroke_style(Some(value_id), channel),
                                self.overlay_channel_color(
                                    Some(value_id),
                                    channel,
                                    palette,
                                    generated_accents,
                                ),
                            ),
                        );
                    }
                }
            });
        }

        if self.candlestick.visible
            && matches!(primary_plot_detail, PrimaryPlotDetail::CandleOnly)
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_))
        {
            let primary_plot = *scene.primary_plot();
            frame.with_clip(primary_plot, |frame| {
                self.for_each_scene_bar_unit_index(
                    scene,
                    |series_index, _, _series, x_unit, bar| {
                        if series_index != 0 || !in_visible_range(x_unit) {
                            return;
                        }

                        let series_anchor = scene
                            .primary
                            .series_percent_anchors
                            .first()
                            .copied()
                            .flatten();
                        let y_open = scene
                            .map_primary_plot_with_anchor(bar.open.to_f32_lossy(), series_anchor);
                        let y_high = scene
                            .map_primary_plot_with_anchor(bar.high.to_f32_lossy(), series_anchor);
                        let y_low = scene
                            .map_primary_plot_with_anchor(bar.low.to_f32_lossy(), series_anchor);
                        let y_close = scene
                            .map_primary_plot_with_anchor(bar.close.to_f32_lossy(), series_anchor);

                        let color = self.candlestick_color(
                            bar.close >= bar.open,
                            palette,
                            generated_accents,
                        );

                        let (body_top, body_h) = Self::snapped_span_with_origin(
                            y_open,
                            y_close,
                            pixel_ratio,
                            plot_origin_y_global,
                        );
                        let (wick_top, wick_h) = Self::snapped_span_with_origin(
                            y_high,
                            y_low,
                            pixel_ratio,
                            plot_origin_y_global,
                        );

                        let x_plot = scene.map_x_plot(x_unit);
                        let candle_left = Self::centered_left_for_width_with_origin(
                            x_plot,
                            candle_body_width_phys,
                            pixel_ratio,
                            plot_origin_x_global,
                        );
                        let wick_left = Self::centered_left_for_width_with_origin(
                            x_plot,
                            wick_width_phys,
                            pixel_ratio,
                            plot_origin_x_global,
                        );

                        frame.fill_rectangle(
                            Point::new(candle_left, body_top),
                            Size::new(candle_body_width, body_h),
                            color,
                        );
                        frame.fill_rectangle(
                            Point::new(wick_left, wick_top),
                            Size::new(wick_width, wick_h),
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
            );
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            for series_index in 0..self.series.len() {
                let is_base_series = series_index == 0;
                let (line_width, line_width_phys) = Self::quantized_stroke_width(
                    f32::from(self.series[series_index].series_line_width()),
                    pixel_ratio,
                );
                let mut point_count = 0usize;

                let path = canvas::Path::new(|builder| {
                    self.for_each_scene_bar_unit_index(
                        scene,
                        |iter_series_index, _, _series, x_unit, bar| {
                            if iter_series_index != series_index || !in_visible_range(x_unit) {
                                return;
                            }

                            let is_base_series = iter_series_index == 0;
                            if is_base_series && !matches!(scene.primary.mark, MarkKind::Line) {
                                return;
                            }

                            let series_anchor = scene
                                .primary
                                .series_percent_anchors
                                .get(iter_series_index)
                                .copied()
                                .flatten();
                            let point = Point::new(
                                Self::snap_plot_x_for_stroke_with_origin(
                                    scene.map_x_plot(x_unit),
                                    pixel_ratio,
                                    plot_origin_x_global,
                                    line_width_phys,
                                ),
                                Self::snap_stroke_center_with_origin(
                                    scene.map_primary_plot_with_anchor(
                                        bar.close.to_f32_lossy(),
                                        series_anchor,
                                    ),
                                    pixel_ratio,
                                    plot_origin_y_global,
                                    line_width_phys,
                                ),
                            );

                            if point_count == 0 {
                                builder.move_to(point);
                            } else {
                                builder.line_to(point);
                            }
                            point_count += 1;
                        },
                    );
                });

                if point_count < 2 {
                    continue;
                }

                let line_color = self
                    .series_line_color(&self.series[series_index], palette, generated_accents)
                    .scale_alpha(if is_base_series { 0.85 } else { 0.96 });

                frame.stroke(
                    &path,
                    indicator_line_stroke(
                        line_width,
                        self.series[series_index].series_stroke_style(),
                        line_color,
                    ),
                );
            }
        });

        for indicator_panel in &scene.indicator_panels {
            if self.indicator_is_unavailable(indicator_panel.value_id) {
                continue;
            }

            if matches!(indicator_panel.presentation, PanelPresentation::Table) {
                continue;
            }

            if !matches!(indicator_panel.mark, MarkKind::Line) {
                continue;
            }

            let Some(panel_plot) = scene
                .layout
                .panel(indicator_panel.panel_index)
                .map(|panel| panel.plot)
            else {
                continue;
            };

            frame.with_clip(panel_plot, |frame| {
                let line_channels = self.line_channels_for_panel_value(indicator_panel.value_id);
                for channel in line_channels.iter().copied() {
                    let (channel_line_width, channel_line_width_phys) =
                        Self::quantized_stroke_width(
                            self.indicator_line_width(indicator_panel.value_id, channel),
                            pixel_ratio,
                        );
                    let mut point_count = 0usize;

                    let path = canvas::Path::new(|builder| {
                        self.for_each_scene_bar_unit_index(
                            scene,
                            |series_index, bar_index, series, x_unit, _bar| {
                                if series_index != 0 || !in_visible_range(x_unit) {
                                    return;
                                }

                                let Some(indicator_data) = self.indicator_data_for_series_at(
                                    series,
                                    indicator_panel.value_id,
                                    bar_index,
                                ) else {
                                    point_count = 0;
                                    return;
                                };

                                let Some(value_id) = indicator_panel.value_id else {
                                    return;
                                };
                                let Some(channel_value) =
                                    self.overlay_channel_value(value_id, indicator_data, channel)
                                else {
                                    return;
                                };

                                let Some(y_channel_value) = scene
                                    .map_indicator_plot(indicator_panel.panel_index, channel_value)
                                else {
                                    return;
                                };

                                let point = Point::new(
                                    Self::snap_plot_x_for_stroke_with_origin(
                                        scene.map_x_plot(x_unit),
                                        pixel_ratio,
                                        plot_origin_x_global,
                                        channel_line_width_phys,
                                    ),
                                    Self::snap_stroke_center_with_origin(
                                        y_channel_value,
                                        pixel_ratio,
                                        plot_origin_y_global,
                                        channel_line_width_phys,
                                    ),
                                );
                                if point_count == 0 {
                                    builder.move_to(point);
                                } else {
                                    builder.line_to(point);
                                }
                                point_count += 1;
                            },
                        );
                    });

                    if point_count < 2 {
                        continue;
                    }

                    frame.stroke(
                        &path,
                        indicator_line_stroke(
                            channel_line_width,
                            self.indicator_stroke_style(indicator_panel.value_id, channel),
                            self.overlay_channel_color(
                                indicator_panel.value_id,
                                channel,
                                palette,
                                generated_accents,
                            ),
                        ),
                    );
                }
            });
        }

        self.fill_panel_titles(frame, scene, palette);
        self.fill_last_price_line(frame, scene, palette);
    }
}
