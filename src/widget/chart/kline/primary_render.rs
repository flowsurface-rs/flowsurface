use super::footprint::{draw_footprint_candle, draw_footprint_cluster_row};
use super::*;
use data::chart::kline_v2::IndicatorColorSlot;
use data::util::abbr_large_numbers;

const FOOTPRINT_MIN_COLUMN_WIDTH_PX: f32 = 24.0;
const FOOTPRINT_MIN_ROW_HEIGHT_PX: f32 = 2.0;
const FOOTPRINT_CELL_INSET_PX: f32 = 2.0;
// Reserve a complete label on both sides before drawing text. Eight characters
// covers the formatter's rounded `1000.00m` boundary without measuring values.
const FOOTPRINT_TEXT_MIN_COLUMN_WIDTH_PX: f32 = 76.0;
const TABLE_TEXT_MIN_COLUMN_WIDTH_PX: f32 = 48.0;
const TABLE_TEXT_MIN_ROW_HEIGHT_PX: f32 = 9.0;
const TABLE_CELL_INSET_PX: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrimaryPlotDetail {
    CandleOnly,
    Footprint,
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn primary_plot_detail(&self, scene: &Scene) -> PrimaryPlotDetail {
        if !self.footprints_enabled {
            return PrimaryPlotDetail::CandleOnly;
        }
        if !matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_)) {
            return PrimaryPlotDetail::CandleOnly;
        }

        let Some(base_series) = self.series.first() else {
            return PrimaryPlotDetail::CandleOnly;
        };
        let Some(last_bar) = base_series.bars().last() else {
            return PrimaryPlotDetail::CandleOnly;
        };

        let tick_size = base_series.footprint_tick_size();
        let row_height = self.footprint_row_height(
            scene,
            last_bar.close,
            tick_size,
            scene
                .primary
                .series_percent_anchors
                .first()
                .copied()
                .flatten(),
        );

        if scene.logical_bar_spacing() >= FOOTPRINT_MIN_COLUMN_WIDTH_PX
            && row_height >= FOOTPRINT_MIN_ROW_HEIGHT_PX
        {
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
            self.candlestick_config.colors.color_for(slot),
            palette,
            generated_accents,
        )
        .scale_alpha(self.candlestick_config.opacity_ratio())
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
            self.for_each_bar_unit_index(scene.x.axis, |series_index, series, x_unit, bar| {
                if series_index != 0 || !in_visible_range(x_unit) {
                    return;
                }

                let Some(footprint) = series.footprint_for_bar(bar) else {
                    return;
                };

                for group in footprint.trades.values() {
                    max_cluster_qty = max_cluster_qty
                        .max(group.max_cluster_qty(self.footprint_cluster_kind).to_f64());
                }
            });
        }

        let primary_plot = *scene.primary_plot();
        let footprint_width =
            (scene.logical_bar_spacing() - FOOTPRINT_CELL_INSET_PX * 2.0).max(1.0);
        let show_text = scene.logical_bar_spacing() >= FOOTPRINT_TEXT_MIN_COLUMN_WIDTH_PX;

        frame.with_clip(primary_plot, |frame| {
            self.for_each_bar_unit_index(scene.x.axis, |series_index, series, x_unit, bar| {
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
                    if self.candlesticks_visible {
                        draw_footprint_candle(
                            frame,
                            scene.map_x_plot(x_unit),
                            footprint_width,
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

                if draw_clusters && max_cluster_qty > f64::EPSILON {
                    for (price, group) in &footprint.trades {
                        let row_height = self
                            .footprint_row_height(scene, *price, tick_size, anchor)
                            .max(1.0);
                        let y = scene.map_primary_plot_with_anchor(price.to_f32_lossy(), anchor);
                        draw_footprint_cluster_row(
                            frame,
                            self.footprint_cluster_kind,
                            x_center,
                            y,
                            row_height,
                            footprint_width,
                            group.sell_qty.to_f64(),
                            group.buy_qty.to_f64(),
                            max_cluster_qty,
                            show_text,
                            palette,
                        );
                    }
                }

                if self.candlesticks_visible {
                    draw_footprint_candle(
                        frame,
                        x_center,
                        footprint_width,
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
        if !self.footprints_visible {
            return;
        }

        let Some(config) = self
            .studies
            .iter()
            .find_map(|study| study.footprint_poc_config())
        else {
            return;
        };
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
        let visible_profile = self.studies.iter().find_map(|study| {
            if !study.is_volume_profile() {
                return None;
            }

            if !matches!(
                study.volume_profile_schedule(),
                VolumeProfileSchedule::VisibleRange
            ) {
                return None;
            }

            base_series
                .trade_price_distribution(earliest, latest, lowest, highest)
                .map(|profile| {
                    (
                        profile,
                        study.volume_profile_colors(),
                        study.volume_profile_width_ratio(),
                        study.volume_profile_opacity_ratio(),
                        study.volume_profile_opacity_scale(),
                    )
                })
        });
        let recurring_intervals = self
            .studies
            .iter()
            .filter(|study| study.is_volume_profile())
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
                    ))
                }
                VolumeProfileSchedule::Recurring { .. } => None,
            });
        let profiles = recurring_intervals
            .flat_map(
                |(interval, colors, width_ratio, opacity_ratio, opacity_scale)| {
                    base_series
                        .recurring_trade_price_distributions(interval, earliest, latest)
                        .into_iter()
                        .map(move |profile| {
                            (profile, colors, width_ratio, opacity_ratio, opacity_scale)
                        })
                },
            )
            .collect::<Vec<_>>();
        if visible_profile.is_none() && profiles.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let tick_size = base_series.footprint_tick_size();
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
                    tick_size,
                    primary_plot.x + primary_plot.width,
                    primary_plot.width * width_ratio,
                    *opacity_ratio,
                    *opacity_scale,
                    primary_plot.x,
                    primary_plot.x + primary_plot.width,
                    false,
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
                self.for_each_bar_unit_index(scene.x.axis, |series_index, series, x_unit, bar| {
                    if series_index != 0 || x_unit < scene.x.min_unit || x_unit > scene.x.max_unit {
                        return;
                    }
                    let Some(data) = self.indicator_data_for_series(series, Some(value_id), bar)
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
                });

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
        let pixel_ratio = self.resolved_pixel_ratio();
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
                self.for_each_bar_unit_index(scene.x.axis, |series_index, series, x_unit, bar| {
                    if series_index != 0 || !in_visible_range(x_unit) {
                        return;
                    }

                    let Some(indicator_data) =
                        self.indicator_data_for_series(series, indicator_panel.value_id, bar)
                    else {
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
                                && let Some(y_overlay) = scene
                                    .map_indicator_plot(indicator_panel.panel_index, overlay_abs)
                            {
                                frame.fill_rectangle(
                                    Point::new(indicator_left, y_overlay.min(y_indicator_baseline)),
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
                });
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

                    for channel in self
                        .overlay_channels_for_panel_value(Some(value_id))
                        .iter()
                        .copied()
                    {
                        let (channel_line_width, channel_line_width_phys) =
                            Self::quantized_stroke_width(channel.line_width, pixel_ratio);
                        let mut point_count = 0usize;
                        let path = canvas::Path::new(|builder| {
                            self.for_each_bar_unit_index(
                                scene.x.axis,
                                |series_index, series, x_unit, bar| {
                                    if series_index != 0 || !in_visible_range(x_unit) {
                                        return;
                                    }

                                    let Some(data) =
                                        self.indicator_data_for_series(series, Some(value_id), bar)
                                    else {
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
                            canvas::Stroke::default()
                                .with_width(channel_line_width)
                                .with_color(self.overlay_channel_color(
                                    Some(value_id),
                                    channel,
                                    palette,
                                    generated_accents,
                                )),
                        );
                    }
                }
            });
        }

        if self.candlesticks_visible
            && matches!(primary_plot_detail, PrimaryPlotDetail::CandleOnly)
            && matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_))
        {
            let primary_plot = *scene.primary_plot();
            frame.with_clip(primary_plot, |frame| {
                self.for_each_bar_unit_index(scene.x.axis, |series_index, _series, x_unit, bar| {
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

                    let color =
                        self.candlestick_color(bar.close >= bar.open, palette, generated_accents);

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
                });
            });
        }

        if matches!(primary_plot_detail, PrimaryPlotDetail::Footprint) {
            self.fill_footprint_overlay(
                frame,
                scene,
                palette,
                generated_accents,
                self.footprints_visible,
            );
        }

        let primary_plot = *scene.primary_plot();
        frame.with_clip(primary_plot, |frame| {
            for series_index in 0..self.series.len() {
                let is_base_series = series_index == 0;
                let requested_line_width = if is_base_series { 1.5 } else { 1.3 };
                let (line_width, line_width_phys) =
                    Self::quantized_stroke_width(requested_line_width, pixel_ratio);
                let mut point_count = 0usize;

                let path = canvas::Path::new(|builder| {
                    self.for_each_bar_unit_index(
                        scene.x.axis,
                        |iter_series_index, _series, x_unit, bar| {
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
                    .series_line_color(&self.series[series_index], palette)
                    .scale_alpha(if is_base_series { 0.85 } else { 0.96 });

                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(line_width)
                        .with_color(line_color),
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
                for channel in self
                    .overlay_channels_for_panel_value(indicator_panel.value_id)
                    .iter()
                    .copied()
                {
                    let (channel_line_width, channel_line_width_phys) =
                        Self::quantized_stroke_width(channel.line_width, pixel_ratio);
                    let mut point_count = 0usize;

                    let path = canvas::Path::new(|builder| {
                        self.for_each_bar_unit_index(
                            scene.x.axis,
                            |series_index, series, x_unit, bar| {
                                if series_index != 0 || !in_visible_range(x_unit) {
                                    return;
                                }

                                let Some(indicator_data) = self.indicator_data_for_series(
                                    series,
                                    indicator_panel.value_id,
                                    bar,
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
                        canvas::Stroke::default()
                            .with_width(channel_line_width)
                            .with_color(self.overlay_channel_color(
                                indicator_panel.value_id,
                                channel,
                                palette,
                                generated_accents,
                            )),
                    );
                }
            });
        }

        self.fill_panel_titles(frame, scene, palette);
        self.fill_last_price_line(frame, scene, palette);
    }
}
