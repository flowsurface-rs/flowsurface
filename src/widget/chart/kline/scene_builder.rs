use super::chrome::{FootprintLegendHit, IndicatorLegendHit, TickerLegendHit};
use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::scene::{
    ChromeScene, CursorInfo, IndicatorPanelScene, PrimaryPercentScaleState, PrimaryScale, Scene,
    SceneInteraction, TickIndex, ValueDomain, XAxis, XViewport, Y_UNIT_STEP_FALLBACK,
};
use super::{BarSpacingPx, KlinePanelKind, KlineSeriesLike, KlineWidget, PanelYViewport, YUnit};
use crate::widget::chart::kline::composition::{
    BarMode, HistogramMode, LayerDataKind, MarkKind, PanelPresentation, PanelScaleMode,
    PanelValueId, PanelValuePrecision,
};

use data::chart::kline_v2::ZOOM_SENSITIVITY;
use exchange::{Kline, Timeframe, UnixMs};

use iced::advanced::Layout;
use iced::{Point, Rectangle, mouse};
impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn base_ticker_info(&self) -> Option<exchange::TickerInfo> {
        self.series.first().map(|series| *series.ticker_info())
    }

    fn normalized_domain_from_values(min_value: f32, max_value: f32) -> Option<(f32, f32)> {
        PanelYViewport::normalized(min_value, max_value)
            .map(|viewport| (viewport.min_value, viewport.max_value))
    }

    fn panel_value_domain_from_scene(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<(f32, f32)> {
        let panel = scene.layout.panel(panel_index)?;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                if panel_index == scene.primary.panel_index
                    && scene.primary.mode.uses_percent_base()
                {
                    return None;
                }

                let (min_display, max_display) = scene.primary_domain_display_values();
                Self::normalized_domain_from_values(
                    scene.primary_display_to_value(min_display),
                    scene.primary_display_to_value(max_display),
                )
            }
            KlinePanelKind::Indicator => {
                let indicator = scene
                    .indicator_panels
                    .iter()
                    .find(|indicator| indicator.panel_index == panel_index)?;
                Self::normalized_domain_from_values(indicator.min_value(), indicator.max_value())
            }
        }
    }

    pub(super) fn panel_viewport_after_y_zoom(
        &self,
        scene: &Scene,
        panel_index: usize,
        cursor_y: f32,
        scroll_delta: f32,
    ) -> Option<PanelYViewport> {
        if scroll_delta.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index == scene.primary.panel_index
            && !scene.primary.mode.allows_primary_manual_zoom()
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel =
            (cursor_y - scene.layout.regions.plot.y - panel.plot.y).clamp(0.0, panel_h);
        let anchor_ratio = (1.0 - (y_in_panel / panel_h)).clamp(0.0, 1.0);

        let zoom_amount = scroll_delta.abs().clamp(0.0, 6.0);
        let zoom_scale = (zoom_amount / ZOOM_SENSITIVITY).exp();
        let zoom_in = scroll_delta > 0.0;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let (min_display, max_display) = scene.primary_domain_display_values();
                let display_viewport = PanelYViewport::normalized(min_display, max_display)?;
                let min_display_span = if matches!(scene.primary.mode, PanelScaleMode::Logarithmic)
                {
                    1e-3
                } else {
                    let step = self
                        .panel_quantization_step(self.panel_value_precision(panel_index))
                        .unwrap_or(1e-4)
                        .abs()
                        .max(1e-8);
                    step * 8.0
                };
                let new_display_viewport = display_viewport.zoom_around_ratio(
                    anchor_ratio,
                    zoom_scale,
                    zoom_in,
                    min_display_span,
                )?;

                PanelYViewport::normalized(
                    scene.primary_display_to_value(new_display_viewport.min_value),
                    scene.primary_display_to_value(new_display_viewport.max_value),
                )
            }
            KlinePanelKind::Indicator => {
                let (min_value, max_value) =
                    self.panel_value_domain_from_scene(scene, panel_index)?;
                let viewport = PanelYViewport::normalized(min_value, max_value)?;
                let step = self
                    .panel_quantization_step(self.panel_value_precision(panel_index))
                    .unwrap_or(1e-4)
                    .abs()
                    .max(1e-8);
                let min_span = step * 8.0;

                viewport.zoom_around_ratio(anchor_ratio, zoom_scale, zoom_in, min_span)
            }
        }
    }

    pub(super) fn panel_viewport_after_primary_y_pan(
        &self,
        scene: &Scene,
        panel_index: usize,
        dy_px: f32,
    ) -> Option<PanelYViewport> {
        if dy_px.abs() <= f32::EPSILON {
            return None;
        }

        if panel_index != scene.primary.panel_index
            || !scene
                .primary
                .mode
                .allows_primary_manual_pan(self.primary_autoscale)
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let delta_ratio = dy_px / panel_h;

        let (min_display, max_display) = scene.primary_domain_display_values();
        let display_viewport = PanelYViewport::normalized(min_display, max_display)?;
        let shifted_display_viewport = display_viewport.pan_by_ratio(delta_ratio)?;

        PanelYViewport::normalized(
            scene.primary_display_to_value(shifted_display_viewport.min_value),
            scene.primary_display_to_value(shifted_display_viewport.max_value),
        )
    }

    pub(super) fn panel_quantization_step(
        &self,
        value_precision: Option<PanelValuePrecision>,
    ) -> Option<f32> {
        let base_ticker = self.base_ticker_info();
        value_precision.and_then(|precision| precision.quantization_step(base_ticker))
    }

    pub(super) fn panel_value_decimals(
        &self,
        value_precision: Option<PanelValuePrecision>,
    ) -> Option<usize> {
        let base_ticker = self.base_ticker_info();
        value_precision.and_then(|precision| precision.decimals(base_ticker))
    }

    pub(super) fn panel_value_to_unit(
        &self,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> YUnit {
        let base_ticker = self.base_ticker_info();

        match value_precision {
            Some(precision) => precision.value_to_unit(value, base_ticker, Y_UNIT_STEP_FALLBACK),
            None => Scene::value_to_unit_with_step(value, Y_UNIT_STEP_FALLBACK),
        }
    }

    pub(super) fn panel_unit_to_value(
        &self,
        value_precision: Option<PanelValuePrecision>,
        y_unit: YUnit,
    ) -> f32 {
        let base_ticker = self.base_ticker_info();

        match value_precision {
            Some(precision) => precision.unit_to_value(y_unit, base_ticker, Y_UNIT_STEP_FALLBACK),
            None => Scene::unit_to_value_with_step(y_unit, Y_UNIT_STEP_FALLBACK),
        }
    }

    fn quantize_panel_value(
        &self,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> f32 {
        let y_unit = self.panel_value_to_unit(value_precision, value);
        self.panel_unit_to_value(value_precision, y_unit)
    }

    fn format_panel_value_compact(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
        fallback_step: f32,
    ) -> String {
        let max_decimals = self.panel_value_label_policy(panel_index).max_decimals;

        if let Some(precision) = value_precision {
            return precision.format_compact(
                value,
                fallback_step,
                max_decimals,
                self.base_ticker_info(),
            );
        }

        let quantized = self.quantize_panel_value(value_precision, value);
        let fallback = fallback_step.abs().max(1e-6);
        let decimals = PanelValuePrecision::decimals_for_step(fallback);
        let decimals = max_decimals
            .map(|max| decimals.min(max as usize))
            .unwrap_or(decimals);
        format!("{quantized:.decimals$}")
    }

    pub(super) fn format_panel_axis_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
        fallback_step: f32,
    ) -> String {
        self.panel_value_label_policy(panel_index)
            .format_axis_value(
                || self.quantize_panel_value(value_precision, value),
                || {
                    self.format_panel_value_compact(
                        panel_index,
                        value_precision,
                        value,
                        fallback_step,
                    )
                },
            )
    }

    pub(super) fn format_panel_header_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f32,
    ) -> String {
        self.panel_value_label_policy(panel_index)
            .format_header_value(
                || self.quantize_panel_value(value_precision, value),
                || self.format_panel_value_compact(panel_index, value_precision, value, 0.01),
            )
    }

    pub(super) fn bar_at_or_before_unit<'b>(
        &self,
        series: &'b S,
        x_axis: XAxis,
        target_unit: i64,
    ) -> Option<&'b Kline> {
        let mut best: Option<(i64, &'b Kline)> = None;

        match x_axis {
            XAxis::Time { .. } => {
                for bar in series.bars() {
                    let unit = x_axis.unit_from_time(bar.time);
                    if unit == target_unit {
                        return Some(bar);
                    }

                    if unit <= target_unit
                        && best.map(|(best_unit, _)| unit > best_unit).unwrap_or(true)
                    {
                        best = Some((unit, bar));
                    }
                }
            }
            XAxis::Tick { .. } => {
                let len = series.bars().len();
                for (index, bar) in series.bars().iter().enumerate() {
                    let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                    let unit = x_axis.unit_from_tick(TickIndex::from_u64(from_latest));

                    if unit == target_unit {
                        return Some(bar);
                    }

                    if unit <= target_unit
                        && best.map(|(best_unit, _)| unit > best_unit).unwrap_or(true)
                    {
                        best = Some((unit, bar));
                    }
                }
            }
        }

        best.map(|(_, bar)| bar)
    }

    fn compute_primary_scale_anchor(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Option<f32> {
        let mut first_unit = i64::MAX;
        let mut base_close = None;

        self.for_each_bar_unit_index(x_axis, |series_index, _, unit, bar| {
            if series_index != 0 {
                return;
            }

            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            if unit < first_unit {
                first_unit = unit;
                base_close = Some(bar.close.to_f32_lossy());
            }
        });

        base_close
    }

    fn compute_series_percent_anchors(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
    ) -> Vec<Option<f32>> {
        let mut anchors = vec![None; self.series.len()];
        let mut first_units = vec![i64::MAX; self.series.len()];

        self.for_each_bar_unit_index(x_axis, |series_index, _, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            if unit < first_units[series_index] {
                first_units[series_index] = unit;
                anchors[series_index] = Some(bar.close.to_f32_lossy());
            }
        });

        anchors
    }

    fn compute_primary_percent_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_mark: MarkKind,
        anchors: &[Option<f32>],
    ) -> Option<(f32, f32)> {
        let mut min_pct: Option<f32> = None;
        let mut max_pct: Option<f32> = None;
        let primary_overlay_value_ids = self.visible_primary_overlay_value_ids();
        let primary_overlay_channels: Vec<_> = primary_overlay_value_ids
            .iter()
            .map(|value_id| self.overlay_channels_for_panel_value(Some(*value_id)))
            .collect();

        self.for_each_bar_unit_index(x_axis, |series_index, series, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let Some(anchor) = anchors
                .get(series_index)
                .copied()
                .flatten()
                .filter(|anchor| anchor.abs() > f32::EPSILON)
            else {
                return;
            };

            let mut emit_pct = |value: f32| {
                let pct = ((value / anchor) - 1.0) * 100.0;
                min_pct = Some(min_pct.map_or(pct, |current| current.min(pct)));
                max_pct = Some(max_pct.map_or(pct, |current| current.max(pct)));
            };

            if series_index == 0 && !matches!(primary_mark, MarkKind::Line) {
                emit_pct(bar.low.to_f32_lossy());
                emit_pct(bar.high.to_f32_lossy());
            } else {
                emit_pct(bar.close.to_f32_lossy());
            }

            if series_index == 0 {
                for (overlay_idx, value_id) in primary_overlay_value_ids.iter().enumerate() {
                    let Some(data) = self.indicator_data_for_series(series, Some(*value_id), bar)
                    else {
                        continue;
                    };

                    let Some(channels) = primary_overlay_channels.get(overlay_idx) else {
                        continue;
                    };

                    for channel in channels.iter().copied() {
                        if let Some(channel_value) = self.overlay_channel_value(data, channel) {
                            emit_pct(channel_value);
                        }
                    }
                }
            }
        });

        let min_pct = min_pct?;
        let max_pct = max_pct?;

        let pad = if (max_pct - min_pct).abs() <= f32::EPSILON {
            max_pct.abs().max(1.0)
        } else {
            (max_pct - min_pct).abs() * 0.05
        };

        Some((min_pct - pad, max_pct + pad))
    }

    fn compute_primary_percent_scale_state(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_mark: MarkKind,
        primary_scale_mode: PanelScaleMode,
    ) -> Option<PrimaryPercentScaleState> {
        if !primary_scale_mode.uses_percent_base() {
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: None,
                series_percent_anchors: vec![None; self.series.len()],
                primary_domain_display_override: None,
            });
        }

        if primary_scale_mode.uses_series_percent_anchors(self.series.len()) {
            let anchors = self.compute_series_percent_anchors(x_axis, min_x_unit, max_x_unit);
            let domain_override = self.compute_primary_percent_domain(
                x_axis,
                min_x_unit,
                max_x_unit,
                primary_mark,
                &anchors,
            )?;

            let base_anchor = anchors.first().copied().flatten();
            return Some(PrimaryPercentScaleState {
                primary_scale_anchor: base_anchor,
                series_percent_anchors: anchors,
                primary_domain_display_override: Some(domain_override),
            });
        }

        let base_anchor = self.compute_primary_scale_anchor(x_axis, min_x_unit, max_x_unit);
        Some(PrimaryPercentScaleState {
            primary_scale_anchor: base_anchor,
            series_percent_anchors: vec![base_anchor; self.series.len()],
            primary_domain_display_override: None,
        })
    }

    fn resolve_x_axis(&self) -> Option<XAxis> {
        match self.basis {
            data::chart::Basis::Time(timeframe) => {
                let anchor = self
                    .series
                    .iter()
                    .flat_map(|s| s.bars().iter())
                    .map(|bar| bar.time.floor_to(timeframe))
                    .max()?;

                Some(XAxis::Time { timeframe, anchor })
            }
            data::chart::Basis::Tick(_) => {
                if self.max_points_available() == 0 {
                    None
                } else {
                    Some(XAxis::Tick {
                        anchor: TickIndex::ZERO,
                    })
                }
            }
        }
    }

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.bars().len())
            .max()
            .unwrap_or_default()
    }

    pub(super) fn for_each_bar_unit_index(
        &self,
        x_axis: XAxis,
        mut f: impl FnMut(usize, &S, i64, &Kline),
    ) {
        for (series_index, series) in self.series.iter().enumerate() {
            match x_axis {
                XAxis::Time { .. } => {
                    for bar in series.bars() {
                        f(series_index, series, x_axis.unit_from_time(bar.time), bar);
                    }
                }
                XAxis::Tick { .. } => {
                    let len = series.bars().len();
                    for (index, bar) in series.bars().iter().enumerate() {
                        let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                        f(
                            series_index,
                            series,
                            x_axis.unit_from_tick(TickIndex::from_u64(from_latest)),
                            bar,
                        );
                    }
                }
            }
        }
    }

    fn for_each_bar_unit(&self, x_axis: XAxis, mut f: impl FnMut(&S, i64, &Kline)) {
        self.for_each_bar_unit_index(x_axis, |_, series, unit, bar| f(series, unit, bar));
    }

    fn data_x_bounds(&self, x_axis: XAxis) -> Option<(i64, i64)> {
        let mut any = false;
        let mut min_unit = i64::MAX;
        let mut max_unit = i64::MIN;

        self.for_each_bar_unit(x_axis, |_, unit, _| {
            any = true;
            min_unit = min_unit.min(unit);
            max_unit = max_unit.max(unit);
        });

        any.then_some((min_unit, max_unit))
    }

    fn effective_bar_spacing(&self) -> f32 {
        self.horizontal_world.effective_spacing()
    }

    fn compute_x_window(&self, plot_width: f32) -> Option<(XAxis, i64, i64, f32)> {
        let x_axis = self.resolve_x_axis()?;
        let (data_min_x, mut data_max_x) = self.data_x_bounds(x_axis)?;

        if data_max_x == data_min_x {
            data_max_x = data_max_x.saturating_add(1);
        }

        let span = self.effective_bar_spacing().max(1.0);
        let visible_units = (plot_width.max(1.0) / span).max(1.0);
        let right_cap = data_max_x as f32 + visible_units;
        let right_floor = data_min_x as f32 + 1.0;
        let right =
            (data_max_x as f32 + self.horizontal_world.offset()).clamp(right_floor, right_cap);
        let left = right - visible_units;
        let left_unit = left.floor() as i64;
        let right_unit = right.ceil() as i64;

        if right_unit <= left_unit {
            return Some((x_axis, left_unit, left_unit.saturating_add(1), right));
        }

        Some((x_axis, left_unit, right_unit, right))
    }

    fn compute_primary_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_value_precision: Option<PanelValuePrecision>,
        primary_mark: MarkKind,
    ) -> Option<(YUnit, YUnit)> {
        let mut min_primary_unit: Option<YUnit> = None;
        let mut max_primary_unit: Option<YUnit> = None;
        let primary_overlay_value_ids = self.visible_primary_overlay_value_ids();
        let primary_overlay_channels: Vec<_> = primary_overlay_value_ids
            .iter()
            .map(|value_id| self.overlay_channels_for_panel_value(Some(*value_id)))
            .collect();

        self.for_each_bar_unit_index(x_axis, |series_index, series, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let (low, high) = if matches!(primary_mark, MarkKind::Line) {
                let close =
                    self.panel_value_to_unit(primary_value_precision, bar.close.to_f32_lossy());
                (close, close)
            } else {
                (
                    self.panel_value_to_unit(primary_value_precision, bar.low.to_f32_lossy()),
                    self.panel_value_to_unit(primary_value_precision, bar.high.to_f32_lossy()),
                )
            };

            min_primary_unit =
                Some(min_primary_unit.map_or(low, |value| YUnit(value.0.min(low.0))));
            max_primary_unit =
                Some(max_primary_unit.map_or(high, |value| YUnit(value.0.max(high.0))));

            if series_index == 0 {
                for (overlay_idx, value_id) in primary_overlay_value_ids.iter().enumerate() {
                    let Some(data) = self.indicator_data_for_series(series, Some(*value_id), bar)
                    else {
                        continue;
                    };

                    let Some(channels) = primary_overlay_channels.get(overlay_idx) else {
                        continue;
                    };

                    for channel in channels.iter().copied() {
                        if let Some(channel_value) = self.overlay_channel_value(data, channel) {
                            let channel_unit =
                                self.panel_value_to_unit(primary_value_precision, channel_value);
                            min_primary_unit =
                                Some(min_primary_unit.map_or(channel_unit, |value| {
                                    YUnit(value.0.min(channel_unit.0))
                                }));
                            max_primary_unit =
                                Some(max_primary_unit.map_or(channel_unit, |value| {
                                    YUnit(value.0.max(channel_unit.0))
                                }));
                        }
                    }
                }
            }
        });

        let min_primary_unit = min_primary_unit?;
        let max_primary_unit = max_primary_unit?;

        let min_i = min_primary_unit.0;
        let max_i = max_primary_unit.0;
        let span = max_i.abs_diff(min_i);
        let pad = if span == 0 {
            max_i.unsigned_abs().max(1).saturating_add(199) / 200
        } else {
            span.saturating_mul(5).saturating_add(99) / 100
        }
        .max(1);
        let pad = i64::try_from(pad).unwrap_or(i64::MAX);

        let padded_min = min_i.saturating_sub(pad);
        let padded_max = max_i.saturating_add(pad);

        Some((YUnit(padded_min), YUnit(padded_max)))
    }

    fn compute_indicator_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        panel_value_id: Option<PanelValueId>,
        panel_value_precision: Option<PanelValuePrecision>,
        data_kind: LayerDataKind,
        mark: MarkKind,
        scale_mode: PanelScaleMode,
    ) -> Option<(YUnit, YUnit)> {
        let series = self.series.first()?;
        let overlay_channels = panel_value_id
            .map(|value_id| self.overlay_channels_for_panel_value(Some(value_id)))
            .unwrap_or(&[]);
        let uses_signed_overlay_histogram = matches!(data_kind, LayerDataKind::Histogram)
            && matches!(
                mark,
                MarkKind::Bar(BarMode::Histogram(HistogramMode::SignedOverlay))
            );

        let mut any = false;
        let mut min_unit = i64::MAX;
        let mut max_unit = i64::MIN;

        let mut visit_bar = |unit: i64, bar: &Kline| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let Some(indicator_data) = self.indicator_data_for_series(series, panel_value_id, bar)
            else {
                return;
            };
            let value_unit =
                self.panel_value_to_unit(panel_value_precision, indicator_data.value());

            any = true;
            min_unit = min_unit.min(value_unit.0);
            max_unit = max_unit.max(value_unit.0);

            for &channel in overlay_channels {
                if let Some(channel_value) = self.overlay_channel_value(indicator_data, channel) {
                    let channel_unit =
                        self.panel_value_to_unit(panel_value_precision, channel_value);
                    min_unit = min_unit.min(channel_unit.0);
                    max_unit = max_unit.max(channel_unit.0);
                }
            }

            if uses_signed_overlay_histogram && let Some(overlay) = indicator_data.signed_overlay()
            {
                let overlay_abs = overlay.abs();
                let overlay_unit = self.panel_value_to_unit(panel_value_precision, overlay_abs);
                min_unit = min_unit.min(overlay_unit.0);
                max_unit = max_unit.max(overlay_unit.0);
            }
        };

        match x_axis {
            XAxis::Time { .. } => {
                for bar in series.bars() {
                    visit_bar(x_axis.unit_from_time(bar.time), bar);
                }
            }
            XAxis::Tick { .. } => {
                let len = series.bars().len();
                for (index, bar) in series.bars().iter().enumerate() {
                    let from_latest = len.saturating_sub(1).saturating_sub(index) as u64;
                    visit_bar(x_axis.unit_from_tick(TickIndex::from_u64(from_latest)), bar);
                }
            }
        }

        if !any {
            return None;
        }

        Some(scale_mode.indicator_domain_units(min_unit, max_unit))
    }

    fn apply_cursor_interaction(
        &self,
        scene: &mut Scene,
        cursor_root_local: Option<Point>,
        primary_panel: usize,
        primary_plot: Rectangle,
        show_legend_values: bool,
    ) {
        let Some(local) = cursor_root_local else {
            return;
        };

        scene.interaction.hovered_control =
            Self::hit_panel_control(&scene.layout, &scene.chrome.panel_controls, local);
        scene.interaction.hovered_corner_control =
            Self::hit_corner_control(&scene.chrome.corner_controls, local);

        let mut ticker_hit = scene
            .chrome
            .ticker_legend
            .as_ref()
            .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));

        if ticker_hit.is_some() {
            scene.chrome.ticker_legend =
                self.build_ticker_legend_layout(&scene.layout, primary_panel, false, true);
            ticker_hit = scene
                .chrome
                .ticker_legend
                .as_ref()
                .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));
        }

        if let Some(hit) = ticker_hit {
            scene.interaction.hovering_ticker_legend = true;
            match hit {
                TickerLegendHit::Background => {}
                TickerLegendHit::Row(index) => {
                    scene.interaction.hovered_ticker_row = Some(index);
                }
                TickerLegendHit::Icon(index, kind) => {
                    scene.interaction.hovered_ticker_row = Some(index);
                    scene.interaction.hovered_ticker_icon = Some((index, kind));
                }
            }
        }

        if let Some(hit) = scene
            .chrome
            .footprint_legend
            .as_ref()
            .and_then(|legend| Self::hit_footprint_legend(&scene.layout, legend, local))
        {
            scene.interaction.hovering_footprint_legend = true;
            if let FootprintLegendHit::Icon(kind) = hit {
                scene.interaction.hovered_footprint_icon = Some(kind);
            }
        }

        let mut indicator_hit = scene
            .chrome
            .indicator_legend
            .as_ref()
            .and_then(|legend| Self::hit_indicator_legend(&scene.layout, legend, local));

        if indicator_hit.is_some() {
            scene.chrome.indicator_legend = self.build_indicator_legend_layout(
                &scene.layout,
                primary_panel,
                self.tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    scene.chrome.footprint_legend.as_ref(),
                    show_legend_values,
                ),
            );
            indicator_hit = scene
                .chrome
                .indicator_legend
                .as_ref()
                .and_then(|legend| Self::hit_indicator_legend(&scene.layout, legend, local));
        }

        if let Some(hit) = indicator_hit {
            scene.interaction.hovering_indicator_legend = true;
            match hit {
                IndicatorLegendHit::Row(index) => {
                    scene.interaction.hovered_indicator_row = Some(index);
                }
                IndicatorLegendHit::Icon(index, kind) => {
                    scene.interaction.hovered_indicator_row = Some(index);
                    scene.interaction.hovered_indicator_icon = Some((index, kind));
                }
            }
        }

        scene.interaction.controls_visible_for_panel = scene
            .interaction
            .hovered_control
            .map(|hit| hit.panel_index)
            .or_else(|| self.control_visibility_panel(&scene.layout, local));

        let zone = scene.layout.hit_test(local);
        if !matches!(zone, LayoutHitZone::PanelPlot(_)) {
            return;
        }

        let Some(plot_local) = scene.layout.plot_local_point(local) else {
            return;
        };

        let x_plot = plot_local.x.clamp(0.0, primary_plot.width.max(1.0));
        let snapped_x_unit = scene.unit_from_plot_x(x_plot);
        let snapped_x_plot = scene
            .map_x_plot(snapped_x_unit)
            .clamp(0.0, scene.plot_right_edge_px());

        let LayoutHitZone::PanelPlot(panel_index) = zone else {
            return;
        };
        let Some(panel) = scene.layout.panel(panel_index) else {
            return;
        };

        let panel_value_precision = self.panel_value_precision(panel_index);
        let y_in_panel = (plot_local.y - panel.plot.y).clamp(0.0, panel.plot.height.max(1.0));

        let (y_primary_unit, y_indicator_unit, snapped_y_plot) = match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let value_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                let uses_display_transform = scene
                    .primary
                    .mode
                    .uses_display_transform(scene.can_use_log_primary_scale());

                let (y_primary_unit, snapped_y_plot) = if uses_display_transform {
                    let (min_display, max_display) = scene.primary_domain_display_values();
                    let y_display_value = min_display + ((max_display - min_display) * value_ratio);
                    let y_primary_unit = self.panel_value_to_unit(
                        panel_value_precision,
                        scene.primary_display_to_value(y_display_value),
                    );
                    let y_primary_value =
                        self.panel_unit_to_value(panel_value_precision, y_primary_unit);
                    let y_plot = scene
                        .map_primary_plot_with_anchor(y_primary_value, scene.primary.anchor)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                } else {
                    let y_primary_unit = Scene::lerp_unit(
                        scene.primary.domain.min_unit(),
                        scene.primary.domain.max_unit(),
                        value_ratio,
                    );
                    let y_plot = scene
                        .map_primary_plot_unit(y_primary_unit)
                        .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                    (y_primary_unit, y_plot)
                };

                (Some(y_primary_unit), None, snapped_y_plot)
            }
            KlinePanelKind::Indicator => {
                if self.indicator_is_unavailable(self.panel_value_id(panel_index)) {
                    (None, None, panel.plot.y + y_in_panel)
                } else {
                    let indicator_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                    let y_indicator_unit = scene
                        .indicator_panel_config(panel_index)
                        .map(|indicator| {
                            Scene::lerp_unit(
                                indicator.min_unit(),
                                indicator.max_unit(),
                                indicator_ratio,
                            )
                        })
                        .unwrap_or_else(|| Scene::lerp_unit(YUnit(0), YUnit(1), indicator_ratio));
                    let snapped_y_plot = scene
                        .map_indicator_plot_unit(panel_index, y_indicator_unit)
                        .map(|y| y.clamp(panel.plot.y, panel.plot.y + panel.plot.height))
                        .unwrap_or(panel.plot.y + y_in_panel);

                    (None, Some(y_indicator_unit), snapped_y_plot)
                }
            }
        };

        scene.interaction.cursor = Some(CursorInfo {
            x_unit: snapped_x_unit,
            panel_index,
            y_primary_unit,
            y_indicator_unit,
            x_plot: snapped_x_plot,
            y_plot: snapped_y_plot,
        });
    }

    pub(super) fn compute_scene(&self, layout: Layout<'_>, cursor: mouse::Cursor) -> Option<Scene> {
        let panel_layout = PanelLayoutTree::from_layout(layout, &self.composition.panels)?;
        let primary_panel = panel_layout
            .panels
            .iter()
            .position(|panel| panel.kind == KlinePanelKind::PrimaryChart)?;
        let primary_mark = self.resolved_panel_mark(primary_panel, KlinePanelKind::PrimaryChart);
        let primary_scale_mode = self.resolved_panel_scale_mode(primary_panel);
        let primary_plot = panel_layout.panel(primary_panel)?.plot;
        let horizontal_world = self.horizontal_world;
        let logical_bar_spacing = horizontal_world.effective_spacing();
        let bar_spacing_px = BarSpacingPx::from_logical(logical_bar_spacing);

        let x_axis_plot_width = panel_layout.primary_plot_width()?;

        let (x_axis, min_x_unit, max_x_unit, right_x_unit) =
            self.compute_x_window(x_axis_plot_width)?;
        let x_world = horizontal_world.axis_at_right_unit(right_x_unit);
        let fractional_x_offset = right_x_unit - max_x_unit as f32;
        let primary_value_precision = self.panel_value_precision(primary_panel);
        let (mut min_primary_unit, mut max_primary_unit) = self.compute_primary_domain(
            x_axis,
            min_x_unit,
            max_x_unit,
            primary_value_precision,
            primary_mark,
        )?;
        let mut primary_domain_values = (
            self.panel_unit_to_value(primary_value_precision, min_primary_unit),
            self.panel_unit_to_value(primary_value_precision, max_primary_unit),
        );

        if let Some(viewport) = self.panel_y_viewport_for_index(primary_panel)
            && !primary_scale_mode.primary_autoscale_enabled(self.primary_autoscale)
        {
            let view_min_unit =
                self.panel_value_to_unit(primary_value_precision, viewport.min_value);
            let view_max_unit =
                self.panel_value_to_unit(primary_value_precision, viewport.max_value);

            if view_min_unit.0 != view_max_unit.0 {
                min_primary_unit = YUnit(view_min_unit.0.min(view_max_unit.0));
                max_primary_unit = YUnit(view_min_unit.0.max(view_max_unit.0));
                primary_domain_values = (viewport.min_value, viewport.max_value);
            }
        }

        let indicator_panels: Vec<IndicatorPanelScene> = panel_layout
            .panels
            .iter()
            .enumerate()
            .filter_map(|(panel_index, panel)| {
                if panel.kind != KlinePanelKind::Indicator {
                    return None;
                }

                let mark = self.resolved_panel_mark(panel_index, KlinePanelKind::Indicator);
                let data_kind =
                    self.resolved_panel_data_kind(panel_index, KlinePanelKind::Indicator);
                let presentation = self.composition.panels[panel_index].presentation;
                let scale_mode = self.resolved_panel_scale_mode(panel_index);
                let value_id = self.panel_value_id(panel_index);
                let value_precision = self.panel_value_precision(panel_index);
                let unit_step =
                    Scene::unit_step_or_default(self.panel_quantization_step(value_precision));
                let (mut min_unit, mut max_unit) =
                    if matches!(presentation, PanelPresentation::Table) {
                        (YUnit(0), YUnit(1))
                    } else {
                        self.compute_indicator_domain(
                            x_axis,
                            min_x_unit,
                            max_x_unit,
                            value_id,
                            value_precision,
                            data_kind,
                            mark,
                            scale_mode,
                        )
                        .unwrap_or((YUnit(0), YUnit(1)))
                    };
                let mut domain_values = (
                    self.panel_unit_to_value(value_precision, min_unit),
                    self.panel_unit_to_value(value_precision, max_unit),
                );

                if let Some(viewport) = self.panel_y_viewport_for_index(panel_index) {
                    let view_min_unit =
                        self.panel_value_to_unit(value_precision, viewport.min_value);
                    let view_max_unit =
                        self.panel_value_to_unit(value_precision, viewport.max_value);

                    if view_min_unit.0 != view_max_unit.0 {
                        min_unit = YUnit(view_min_unit.0.min(view_max_unit.0));
                        max_unit = YUnit(view_min_unit.0.max(view_max_unit.0));
                        domain_values = (viewport.min_value, viewport.max_value);
                    }
                }

                Some(IndicatorPanelScene {
                    panel_index,
                    value_id,
                    unit_step,
                    domain: ValueDomain::new((min_unit, max_unit), domain_values),
                    mark,
                    data_kind,
                    presentation,
                })
            })
            .collect();

        let PrimaryPercentScaleState {
            primary_scale_anchor,
            series_percent_anchors,
            primary_domain_display_override,
        } = self.compute_primary_percent_scale_state(
            x_axis,
            min_x_unit,
            max_x_unit,
            primary_mark,
            primary_scale_mode,
        )?;

        let cursor_root_local = cursor.position_in(layout.bounds());
        let show_legend_values = cursor_root_local
            .map(|local| matches!(panel_layout.hit_test(local), LayoutHitZone::PanelPlot(_)))
            .unwrap_or(false);

        let panel_controls = self.build_panel_control_hits(&panel_layout, primary_panel);
        let corner_controls = self.build_corner_control_hits(&panel_layout, primary_scale_mode);
        let ticker_legend = self.build_ticker_legend_layout(
            &panel_layout,
            primary_panel,
            show_legend_values,
            false,
        );
        let primary_value_step = self.panel_quantization_step(primary_value_precision);
        let primary_value_decimals = self.panel_value_decimals(primary_value_precision);
        let primary_unit_step = Scene::unit_step_or_default(primary_value_step);

        let mut scene = Scene {
            layout: panel_layout,
            x: XViewport {
                axis: x_axis,
                world: x_world,
                bar_spacing_px,
                logical_bar_spacing,
                fractional_offset: fractional_x_offset,
                min_unit: min_x_unit,
                max_unit: max_x_unit,
            },
            primary: PrimaryScale {
                panel_index: primary_panel,
                domain: ValueDomain::new(
                    (min_primary_unit, max_primary_unit),
                    primary_domain_values,
                ),
                display_domain_override: primary_domain_display_override,
                unit_step: primary_unit_step,
                mark: primary_mark,
                mode: primary_scale_mode,
                anchor: primary_scale_anchor,
                value_step: primary_value_step,
                value_decimals: primary_value_decimals,
                series_percent_anchors,
            },
            indicator_panels,
            chrome: ChromeScene {
                panel_controls,
                corner_controls,
                ticker_legend,
                footprint_legend: None,
                indicator_legend: None,
            },
            interaction: SceneInteraction::default(),
        };

        if self.footprints_enabled {
            scene.chrome.footprint_legend = self.build_footprint_legend_layout(
                &scene.layout,
                primary_panel,
                self.tooltip_rows_before_indicators(
                    scene.chrome.ticker_legend.as_ref(),
                    None,
                    show_legend_values,
                ),
                self.footprint_cluster_kind,
                matches!(scene.primary.mark, MarkKind::Candle | MarkKind::Bar(_)),
            );
        }
        scene.chrome.indicator_legend = self.build_indicator_legend_layout(
            &scene.layout,
            primary_panel,
            self.tooltip_rows_before_indicators(
                scene.chrome.ticker_legend.as_ref(),
                scene.chrome.footprint_legend.as_ref(),
                show_legend_values,
            ),
        );

        self.apply_cursor_interaction(
            &mut scene,
            cursor_root_local,
            primary_panel,
            primary_plot,
            show_legend_values,
        );
        Some(scene)
    }

    pub(super) fn tick_unit_for_index(&self, x_axis: XAxis, index: usize) -> Option<i64> {
        let series = self.series.first()?;
        if index >= series.bars().len() {
            return None;
        }

        let from_latest = series.bars().len().saturating_sub(1).saturating_sub(index) as u64;
        Some(x_axis.unit_from_tick(TickIndex::from_u64(from_latest)))
    }

    pub(super) fn tick_time_for_unit(&self, x_axis: XAxis, unit: i64) -> Option<UnixMs> {
        let from_latest = usize::try_from(x_axis.tick_from_unit(unit)?.as_u64()).ok()?;
        let series = self.series.first()?;
        let index = series
            .bars()
            .len()
            .checked_sub(from_latest.checked_add(1)?)?;
        series.time_at_index(index)
    }

    pub(super) fn format_x_label(&self, x_axis: XAxis, unit: i64, step_units: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    super::super::format_time_label(
                        ts.as_u64(),
                        x_axis.step_ms(step_units),
                        self.timezone,
                    )
                },
            ),
            XAxis::Tick { .. } => self.tick_time_for_unit(x_axis, unit).map_or_else(
                || unit.to_string(),
                |time| {
                    self.timezone
                        .format_with_kind(
                            time.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Axis {
                                timeframe: Timeframe::MS100,
                            },
                        )
                        .unwrap_or_else(|| time.as_u64().to_string())
                },
            ),
        }
    }

    pub(super) fn format_x_crosshair_label(&self, x_axis: XAxis, unit: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    self.timezone
                        .format_with_kind(
                            ts.as_u64() as i64,
                            data::config::timezone::TimeLabelKind::Crosshair { show_millis: false },
                        )
                        .unwrap_or_else(|| ts.as_u64().to_string())
                },
            ),
            XAxis::Tick { .. } => self
                .tick_time_for_unit(x_axis, unit)
                .and_then(|time| {
                    self.timezone.format_with_kind(
                        time.as_u64() as i64,
                        data::config::timezone::TimeLabelKind::Crosshair { show_millis: true },
                    )
                })
                .unwrap_or_else(|| self.format_x_label(x_axis, unit, 1)),
        }
    }
}
