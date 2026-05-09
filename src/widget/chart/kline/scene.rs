use super::chrome::{
    CornerControlHit, PanelControlHit, TickerLegendHit, TickerLegendIconKind, TickerLegendLayout,
};
use super::coord::{ChartCoord, ChartStepMs, RoundedOffsetUnits};
use super::layout::{LayoutHitZone, PanelLayoutTree};
use super::{
    BarSpacingPx, HorizontalScale, KlinePanelKind, KlineSeriesLike, KlineWidget, PanelYViewport,
    YUnit,
};
use crate::widget::chart::kline::composition::{
    BarMode, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode, PanelValueId,
    PanelValuePrecision,
};

use exchange::{Kline, Timeframe, UnixMs};

use iced::advanced::Layout;
use iced::{Rectangle, mouse};

const Y_UNIT_STEP_FALLBACK: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TickIndex(u64);

impl TickIndex {
    const ZERO: Self = Self(0);

    #[inline]
    fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum XAxis {
    Time {
        timeframe: Timeframe,
        anchor: UnixMs,
    },
    Tick {
        anchor: TickIndex,
    },
}

impl XAxis {
    #[inline]
    fn unit_from_time(self, value: UnixMs) -> i64 {
        match self {
            Self::Time { timeframe, anchor } => {
                let aligned = ChartCoord::from_unix_ms(value.floor_to(timeframe)).get();
                let anchor = ChartCoord::from_unix_ms(anchor).get();
                let step = ChartStepMs::from_u64(timeframe.to_milliseconds().max(1)).get();

                aligned.saturating_sub(anchor) / step
            }
            Self::Tick { .. } => 0,
        }
    }

    #[inline]
    fn unit_from_tick(self, value: TickIndex) -> i64 {
        match self {
            Self::Tick { anchor } => {
                let anchor = ChartCoord::from_u64_clamped(anchor.as_u64()).get();
                let index = ChartCoord::from_u64_clamped(value.as_u64()).get();

                anchor.saturating_sub(index)
            }
            Self::Time { .. } => 0,
        }
    }

    #[inline]
    fn time_from_unit(self, unit: i64) -> Option<UnixMs> {
        match self {
            Self::Time { timeframe, anchor } => {
                let step = i64::try_from(timeframe.to_milliseconds()).unwrap_or(i64::MAX);
                Some(anchor.saturating_add_signed(unit.saturating_mul(step)))
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    fn tick_from_unit(self, unit: i64) -> Option<TickIndex> {
        match self {
            Self::Tick { anchor } => {
                let anchor = anchor.as_u64();
                if unit >= 0 {
                    anchor.checked_sub(unit as u64).map(TickIndex)
                } else {
                    anchor.checked_add(unit.unsigned_abs()).map(TickIndex)
                }
            }
            Self::Time { .. } => None,
        }
    }

    #[inline]
    fn step_ms(self, step_units: i64) -> u64 {
        match self {
            Self::Time { timeframe, .. } => {
                let step = step_units.max(1) as u64;
                timeframe.to_milliseconds().max(1).saturating_mul(step)
            }
            Self::Tick { .. } => 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CursorInfo {
    pub(super) x_unit: i64,
    pub(super) panel_index: usize,
    pub(super) y_primary_unit: Option<YUnit>,
    pub(super) y_indicator_unit: Option<YUnit>,
    pub(super) x_plot: f32,
    pub(super) y_plot: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorPanelScene {
    pub(super) panel_index: usize,
    pub(super) value_id: Option<PanelValueId>,
    pub(super) unit_step: f32,
    pub(super) mark: MarkKind,
    pub(super) data_kind: LayerDataKind,
    pub(super) min_unit: YUnit,
    pub(super) max_unit: YUnit,
}

#[derive(Debug, Clone)]
pub(super) struct Scene {
    pub(super) layout: PanelLayoutTree,
    pub(super) x_axis: XAxis,
    bar_spacing_px: BarSpacingPx,
    pub(super) min_x_unit: i64,
    pub(super) max_x_unit: i64,
    pub(super) min_primary_unit: YUnit,
    pub(super) max_primary_unit: YUnit,
    pub(super) primary_domain_display_override: Option<(f32, f32)>,
    pub(super) primary_unit_step: f32,
    pub(super) primary_panel: usize,
    pub(super) primary_mark: MarkKind,
    pub(super) primary_scale_mode: PanelScaleMode,
    pub(super) primary_scale_anchor: Option<f32>,
    pub(super) primary_value_step: Option<f32>,
    pub(super) primary_value_decimals: Option<usize>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
    pub(super) indicator_panels: Vec<IndicatorPanelScene>,
    pub(super) panel_controls: Vec<PanelControlHit>,
    pub(super) corner_controls: Vec<CornerControlHit>,
    pub(super) ticker_legend: Option<TickerLegendLayout>,
    pub(super) controls_visible_for_panel: Option<usize>,
    pub(super) hovered_control: Option<PanelControlHit>,
    pub(super) hovered_corner_control: Option<CornerControlHit>,
    pub(super) hovering_ticker_legend: bool,
    pub(super) hovered_ticker_row: Option<usize>,
    pub(super) hovered_ticker_icon: Option<(usize, TickerLegendIconKind)>,
    pub(super) cursor: Option<CursorInfo>,
}

#[derive(Debug, Clone)]
struct PrimaryPercentScaleState {
    primary_scale_anchor: Option<f32>,
    series_percent_anchors: Vec<Option<f32>>,
    primary_domain_display_override: Option<(f32, f32)>,
}

impl Scene {
    fn round_to_i64_saturating(value: f32) -> i64 {
        if !value.is_finite() {
            return 0;
        }

        let rounded = value.round();
        if rounded > i64::MAX as f32 {
            i64::MAX
        } else if rounded < i64::MIN as f32 {
            i64::MIN
        } else {
            rounded as i64
        }
    }

    fn unit_step_or_default(step: Option<f32>) -> f32 {
        step.unwrap_or(Y_UNIT_STEP_FALLBACK).abs().max(1e-8)
    }

    fn value_to_unit_with_step(value: f32, unit_step: f32) -> YUnit {
        if !value.is_finite() || !unit_step.is_finite() || unit_step <= 0.0 {
            return YUnit(0);
        }

        YUnit(Self::round_to_i64_saturating(value / unit_step))
    }

    fn unit_to_value_with_step(y_unit: YUnit, unit_step: f32) -> f32 {
        (y_unit.0 as f32) * unit_step
    }

    fn primary_value_to_unit(&self, value: f32) -> YUnit {
        Self::value_to_unit_with_step(value, self.primary_unit_step)
    }

    fn primary_unit_to_value(&self, y_unit: YUnit) -> f32 {
        Self::unit_to_value_with_step(y_unit, self.primary_unit_step)
    }

    fn indicator_value_to_unit(&self, panel_index: usize, value: f32) -> Option<YUnit> {
        self.indicator_panel_config(panel_index)
            .map(|panel| Self::value_to_unit_with_step(value, panel.unit_step))
    }

    fn lerp_unit(min_unit: YUnit, max_unit: YUnit, ratio: f32) -> YUnit {
        let ratio = ratio.clamp(0.0, 1.0);
        let min = min_unit.0;
        let max = max_unit.0;
        let span = max.saturating_sub(min);
        if span == 0 {
            return min_unit;
        }

        let offset = ((span as f32) * ratio).round() as i64;
        let value = min.saturating_add(offset);
        YUnit(value)
    }

    pub(super) fn plot_rect(&self) -> Rectangle {
        self.layout.regions.plot
    }

    pub(super) fn primary_plot(&self) -> &Rectangle {
        self.layout
            .panel(self.primary_panel)
            .map(|panel| &panel.plot)
            .or_else(|| self.layout.panels.first().map(|panel| &panel.plot))
            .unwrap_or(&self.layout.regions.plot)
    }

    fn indicator_panel_config(&self, panel_index: usize) -> Option<&IndicatorPanelScene> {
        self.indicator_panels
            .iter()
            .find(|indicator| indicator.panel_index == panel_index)
    }

    fn indicator_plot(&self, panel_index: usize) -> Option<&Rectangle> {
        self.layout.panel(panel_index).map(|panel| &panel.plot)
    }

    pub(super) fn indicator_panel_bottom(&self, panel_index: usize) -> Option<f32> {
        self.indicator_plot(panel_index)
            .map(|rect| rect.y + rect.height)
    }

    pub(super) fn bar_spacing_px(&self) -> BarSpacingPx {
        self.bar_spacing_px
    }

    pub(super) fn x_axis_plot_width(&self) -> f32 {
        (self.layout.regions.x_axis.width - self.layout.regions.y_axis.width)
            .max(1.0)
            .min(self.primary_plot().width.max(1.0))
    }

    fn plot_right_edge_px(&self) -> f32 {
        self.x_axis_plot_width().floor().max(1.0)
    }

    pub(super) fn map_x_plot(&self, x_unit: i64) -> f32 {
        let steps_from_right = self.max_x_unit.saturating_sub(x_unit);
        let right_edge_px = self.plot_right_edge_px();
        let spacing_px = self.bar_spacing_px.as_f32().max(1.0);

        right_edge_px - (steps_from_right as f32 * spacing_px)
    }

    pub(super) fn x_unit_for_time(&self, time: UnixMs) -> Option<i64> {
        match self.x_axis {
            XAxis::Time { .. } => Some(self.x_axis.unit_from_time(time)),
            XAxis::Tick { .. } => None,
        }
    }

    pub(super) fn time_for_x_unit(&self, x_unit: i64) -> Option<UnixMs> {
        self.x_axis.time_from_unit(x_unit)
    }

    fn unit_from_plot_x(&self, x_plot: f32) -> i64 {
        let right_edge_px = self.plot_right_edge_px();
        let clamped_x = x_plot.clamp(0.0, right_edge_px);
        let spacing = self.bar_spacing_px.as_f32().max(1.0);
        let steps_from_right = ((right_edge_px - clamped_x) / spacing).round() as i64;

        self.max_x_unit
            .saturating_sub(steps_from_right)
            .clamp(self.min_x_unit, self.max_x_unit)
    }

    pub(super) fn primary_domain_display_values(&self) -> (f32, f32) {
        if let Some((min_display, max_display)) = self.primary_domain_display_override {
            return (min_display, max_display);
        }

        let min_primary_value = self.primary_unit_to_value(self.min_primary_unit);
        let max_primary_value = self.primary_unit_to_value(self.max_primary_unit);
        let log_scale_enabled = self.can_use_log_primary_scale();

        (
            self.primary_scale_mode.value_to_display(
                min_primary_value,
                self.primary_scale_anchor,
                log_scale_enabled,
            ),
            self.primary_scale_mode.value_to_display(
                max_primary_value,
                self.primary_scale_anchor,
                log_scale_enabled,
            ),
        )
    }

    pub(super) fn primary_display_to_value(&self, display_value: f32) -> f32 {
        self.primary_scale_mode.display_to_value(
            display_value,
            self.primary_scale_anchor,
            self.can_use_log_primary_scale(),
        )
    }

    fn can_use_log_primary_scale(&self) -> bool {
        self.primary_unit_to_value(self.min_primary_unit) > f32::EPSILON
            && self.primary_unit_to_value(self.max_primary_unit) > f32::EPSILON
    }

    fn quantized_primary_value(&self, value: f32) -> f32 {
        self.primary_unit_to_value(self.primary_value_to_unit(value))
    }

    fn decimals_for_step(step: f32) -> usize {
        let step = step.abs();
        if !step.is_finite() || step <= 0.0 {
            return 4;
        }

        for decimals in 0..=8 {
            let scaled = (step as f64) * 10_f64.powi(decimals as i32);
            let nearest = scaled.round();
            let tolerance = (scaled.abs() * 1e-9).max(1e-12);

            if (scaled - nearest).abs() <= tolerance {
                return decimals;
            }
        }

        8
    }

    fn primary_format_step(&self, fallback: f32) -> f32 {
        let fallback = fallback.abs().max(1e-6);
        self.primary_value_step
            .map(|step| step.max(fallback))
            .unwrap_or(fallback)
    }

    pub(super) fn format_primary_axis_label(
        &self,
        display_value: f32,
        display_step: f32,
    ) -> String {
        if let Some(percent_label) = self
            .primary_scale_mode
            .format_percent_label(display_value, display_step)
        {
            return percent_label;
        }

        if let Some((log_value, value_step)) = self.primary_scale_mode.log_axis_value_and_step(
            display_value,
            display_step,
            self.primary_scale_anchor,
            self.can_use_log_primary_scale(),
        ) {
            let value = self.quantized_primary_value(log_value);
            return super::super::format_value(value, self.primary_format_step(value_step));
        }

        let value = self.quantized_primary_value(display_value);
        if let Some(decimals) = self.primary_value_decimals {
            format!("{value:.decimals$}")
        } else if let Some(step) = self.primary_value_step {
            let fallback_decimals = Self::decimals_for_step(step);
            format!("{value:.fallback_decimals$}")
        } else {
            super::super::format_value(value, self.primary_format_step(display_step))
        }
    }

    pub(super) fn format_primary_cursor_label(&self, raw_value: f32) -> String {
        let quantized = self.quantized_primary_value(raw_value);

        if let Some(display_step) = self.primary_scale_mode.percent_display_step(
            self.primary_value_step,
            self.primary_scale_anchor,
            0.01,
        ) {
            let display_value = self.primary_scale_mode.value_to_display(
                quantized,
                self.primary_scale_anchor,
                self.can_use_log_primary_scale(),
            );

            if let Some(percent_label) = self
                .primary_scale_mode
                .format_percent_label(display_value, display_step)
            {
                return percent_label;
            }
        }

        if let Some(decimals) = self.primary_value_decimals {
            format!("{quantized:.decimals$}")
        } else if let Some(step) = self.primary_value_step {
            let fallback_decimals = Self::decimals_for_step(step);
            format!("{quantized:.fallback_decimals$}")
        } else {
            super::super::format_value(quantized, self.primary_format_step(0.01))
        }
    }

    pub(super) fn map_primary_plot_unit(&self, y_unit: YUnit) -> f32 {
        let min = self.min_primary_unit.0;
        let max = self.max_primary_unit.0;
        let range = max.saturating_sub(min).unsigned_abs().max(1);
        let delta = y_unit.0.saturating_sub(min);
        let ratio = delta as f32 / range as f32;
        let panel = self.primary_plot();
        panel.y + (1.0 - ratio) * panel.height
    }

    pub(super) fn map_indicator_plot_unit(
        &self,
        panel_index: usize,
        indicator_unit: YUnit,
    ) -> Option<f32> {
        let panel = self.indicator_plot(panel_index)?;
        let indicator = self.indicator_panel_config(panel_index)?;
        let min = indicator.min_unit.0;
        let max = indicator.max_unit.0;
        let range = max.saturating_sub(min).unsigned_abs().max(1);
        let delta = indicator_unit.0.saturating_sub(min);
        let ratio = delta as f32 / range as f32;
        Some(panel.y + (1.0 - ratio) * panel.height)
    }

    pub(super) fn map_primary_plot_with_anchor(&self, value: f32, anchor: Option<f32>) -> f32 {
        let log_scale_enabled = self.can_use_log_primary_scale();
        let uses_display_transform = self
            .primary_scale_mode
            .uses_display_transform(log_scale_enabled);

        if !uses_display_transform {
            return self.map_primary_plot_unit(self.primary_value_to_unit(value));
        }

        let (min_display, max_display) = self.primary_domain_display_values();
        let range = (max_display - min_display).abs().max(1e-6);
        let display_anchor = self
            .primary_scale_mode
            .display_anchor(anchor, self.primary_scale_anchor);
        let display_value =
            self.primary_scale_mode
                .value_to_display(value, display_anchor, log_scale_enabled);
        let ratio = (display_value - min_display) / range;
        let panel = self.primary_plot();
        panel.y + (1.0 - ratio) * panel.height
    }

    pub(super) fn map_indicator_plot(
        &self,
        panel_index: usize,
        indicator_value: f32,
    ) -> Option<f32> {
        let y_unit = self.indicator_value_to_unit(panel_index, indicator_value)?;
        self.map_indicator_plot_unit(panel_index, y_unit)
    }
}

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    fn base_ticker_info(&self) -> Option<exchange::TickerInfo> {
        self.series.first().map(|series| *series.ticker_info())
    }

    fn normalized_domain_from_values(min_value: f32, max_value: f32) -> Option<(f32, f32)> {
        PanelYViewport::normalized(min_value, max_value)
            .map(|viewport| (viewport.min_value, viewport.max_value))
    }

    fn panel_domain_from_units(
        &self,
        panel_precision: Option<PanelValuePrecision>,
        min_unit: YUnit,
        max_unit: YUnit,
    ) -> Option<(f32, f32)> {
        Self::normalized_domain_from_values(
            self.panel_unit_to_value(panel_precision, min_unit),
            self.panel_unit_to_value(panel_precision, max_unit),
        )
    }

    fn panel_value_domain_from_scene(
        &self,
        scene: &Scene,
        panel_index: usize,
    ) -> Option<(f32, f32)> {
        let panel = scene.layout.panel(panel_index)?;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                if panel_index == scene.primary_panel
                    && scene.primary_scale_mode.uses_percent_base()
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
                let precision = self.panel_value_precision(panel_index);

                self.panel_domain_from_units(precision, indicator.min_unit, indicator.max_unit)
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

        if panel_index == scene.primary_panel
            && !scene.primary_scale_mode.allows_primary_manual_zoom()
        {
            return None;
        }

        let panel = scene.layout.panel(panel_index)?;
        let panel_h = panel.plot.height.max(1.0);
        let y_in_panel =
            (cursor_y - scene.layout.regions.plot.y - panel.plot.y).clamp(0.0, panel_h);
        let anchor_ratio = (1.0 - (y_in_panel / panel_h)).clamp(0.0, 1.0);

        let zoom_amount = scroll_delta.abs().clamp(0.05, 6.0);
        let zoom_scale = (1.0 + zoom_amount * 0.14).clamp(1.02, 3.0);
        let zoom_in = scroll_delta > 0.0;

        match panel.kind {
            KlinePanelKind::PrimaryChart => {
                let (min_display, max_display) = scene.primary_domain_display_values();
                let display_viewport = PanelYViewport::normalized(min_display, max_display)?;
                let min_display_span =
                    if matches!(scene.primary_scale_mode, PanelScaleMode::Logarithmic) {
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

        if panel_index != scene.primary_panel
            || !scene
                .primary_scale_mode
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
                    let unit = x_axis.unit_from_tick(TickIndex(from_latest));

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
                base_close = Some(bar.close.to_f32());
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
                anchors[series_index] = Some(bar.close.to_f32());
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
        let primary_overlay_value_ids = self.primary_overlay_value_ids();
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
                emit_pct(bar.low.to_f32());
                emit_pct(bar.high.to_f32());
            } else {
                emit_pct(bar.close.to_f32());
            }

            if series_index == 0 {
                for (overlay_idx, value_id) in primary_overlay_value_ids.iter().enumerate() {
                    let Some(data) =
                        series.indicator_data_for_panel_value_opt(Some(*value_id), bar)
                    else {
                        continue;
                    };

                    let Some(channels) = primary_overlay_channels.get(overlay_idx) else {
                        continue;
                    };

                    for channel in channels.iter().copied() {
                        if let Some(channel_value) = Self::overlay_channel_value(data, channel) {
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
            ((max_pct - min_pct).abs() * 0.05).max(0.25)
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

    pub(super) fn normalize_horizontal_scale(&self, scale: HorizontalScale) -> HorizontalScale {
        HorizontalScale::pixels_per_bar(
            BarSpacingPx::from_logical(scale.as_pixels_per_bar()).as_f32(),
        )
    }

    fn render_bar_spacing_px(&self) -> BarSpacingPx {
        BarSpacingPx::from_logical(
            self.normalize_horizontal_scale(self.horizontal_scale)
                .as_pixels_per_bar(),
        )
    }

    fn max_points_available(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.bars().len())
            .max()
            .unwrap_or_default()
    }

    pub(super) fn step_horizontal_scale_percent(
        &self,
        current: HorizontalScale,
        zoom_scale: f32,
    ) -> HorizontalScale {
        let current_spacing = self.normalize_horizontal_scale(current).as_pixels_per_bar();
        let zoom_scale = zoom_scale.clamp(-1.0, 1.0);
        let next_spacing = current_spacing + zoom_scale * (current_spacing / 10.0);

        self.normalize_horizontal_scale(HorizontalScale::pixels_per_bar(next_spacing))
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
                            x_axis.unit_from_tick(TickIndex(from_latest)),
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

    fn visible_x_span_units_for_width(&self, plot_width: f32) -> i64 {
        let spacing = self.render_bar_spacing_px().as_f32().max(1.0);
        ((plot_width.floor().max(1.0) / spacing).floor() as i64).max(1)
    }

    fn compute_x_window(&self, plot_width: f32) -> Option<(XAxis, i64, i64)> {
        let x_axis = self.resolve_x_axis()?;
        let (data_min_x, mut data_max_x) = self.data_x_bounds(x_axis)?;

        if data_max_x == data_min_x {
            data_max_x = data_max_x.saturating_add(1);
        }

        let span = self.visible_x_span_units_for_width(plot_width);

        let pan_units = RoundedOffsetUnits::from_f32(self.horizontal_offset)
            .map(RoundedOffsetUnits::get)
            .unwrap_or(0);
        let mut right = data_max_x.saturating_add(pan_units);
        let right_cap = data_max_x.saturating_add(span);
        if right > right_cap {
            right = right_cap;
        }

        // Keep at least one bar in view when panning far left, but allow the
        // left bound to extend past loaded history so empty pre-history space is visible.
        let right_floor = data_min_x.saturating_add(1);
        if right < right_floor {
            right = right_floor;
        }

        let left = right.saturating_sub(span);

        if right <= left {
            right = left.saturating_add(1);
        }

        Some((x_axis, left, right))
    }

    fn compute_primary_domain(
        &self,
        x_axis: XAxis,
        min_x_unit: i64,
        max_x_unit: i64,
        primary_value_precision: Option<PanelValuePrecision>,
    ) -> Option<(YUnit, YUnit)> {
        let mut min_primary_unit: Option<YUnit> = None;
        let mut max_primary_unit: Option<YUnit> = None;
        let primary_overlay_value_ids = self.primary_overlay_value_ids();
        let primary_overlay_channels: Vec<_> = primary_overlay_value_ids
            .iter()
            .map(|value_id| self.overlay_channels_for_panel_value(Some(*value_id)))
            .collect();

        self.for_each_bar_unit_index(x_axis, |series_index, series, unit, bar| {
            if unit < min_x_unit || unit > max_x_unit {
                return;
            }

            let low = self.panel_value_to_unit(primary_value_precision, bar.low.to_f32());
            let high = self.panel_value_to_unit(primary_value_precision, bar.high.to_f32());

            min_primary_unit =
                Some(min_primary_unit.map_or(low, |value| YUnit(value.0.min(low.0))));
            max_primary_unit =
                Some(max_primary_unit.map_or(high, |value| YUnit(value.0.max(high.0))));

            if series_index == 0 {
                for (overlay_idx, value_id) in primary_overlay_value_ids.iter().enumerate() {
                    let Some(data) =
                        series.indicator_data_for_panel_value_opt(Some(*value_id), bar)
                    else {
                        continue;
                    };

                    let Some(channels) = primary_overlay_channels.get(overlay_idx) else {
                        continue;
                    };

                    for channel in channels.iter().copied() {
                        if let Some(channel_value) = Self::overlay_channel_value(data, channel) {
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

            let Some(indicator_data) =
                series.indicator_data_for_panel_value_opt(panel_value_id, bar)
            else {
                return;
            };
            let value_unit =
                self.panel_value_to_unit(panel_value_precision, indicator_data.value());

            any = true;
            min_unit = min_unit.min(value_unit.0);
            max_unit = max_unit.max(value_unit.0);

            for &channel in overlay_channels {
                if let Some(channel_value) = Self::overlay_channel_value(indicator_data, channel) {
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
                    visit_bar(x_axis.unit_from_tick(TickIndex(from_latest)), bar);
                }
            }
        }

        if !any {
            return None;
        }

        Some(scale_mode.indicator_domain_units(min_unit, max_unit))
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
        let bar_spacing_px = self.render_bar_spacing_px();

        let x_axis_plot_width = (panel_layout.regions.x_axis.width
            - panel_layout.regions.y_axis.width)
            .max(1.0)
            .min(primary_plot.width.max(1.0));

        let (x_axis, min_x_unit, max_x_unit) = self.compute_x_window(x_axis_plot_width)?;
        let primary_value_precision = self.panel_value_precision(primary_panel);
        let (mut min_primary_unit, mut max_primary_unit) =
            self.compute_primary_domain(x_axis, min_x_unit, max_x_unit, primary_value_precision)?;

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
                let scale_mode = self.resolved_panel_scale_mode(panel_index);
                let value_id = self.panel_value_id(panel_index);
                let value_precision = self.panel_value_precision(panel_index);
                let unit_step =
                    Scene::unit_step_or_default(self.panel_quantization_step(value_precision));
                let (mut min_unit, mut max_unit) = self
                    .compute_indicator_domain(
                        x_axis,
                        min_x_unit,
                        max_x_unit,
                        value_id,
                        value_precision,
                        data_kind,
                        mark,
                        scale_mode,
                    )
                    .unwrap_or((YUnit(0), YUnit(1)));

                if let Some(viewport) = self.panel_y_viewport_for_index(panel_index) {
                    let view_min_unit =
                        self.panel_value_to_unit(value_precision, viewport.min_value);
                    let view_max_unit =
                        self.panel_value_to_unit(value_precision, viewport.max_value);

                    if view_min_unit.0 != view_max_unit.0 {
                        min_unit = YUnit(view_min_unit.0.min(view_max_unit.0));
                        max_unit = YUnit(view_min_unit.0.max(view_max_unit.0));
                    }
                }

                Some(IndicatorPanelScene {
                    panel_index,
                    value_id,
                    unit_step,
                    mark,
                    data_kind,
                    min_unit,
                    max_unit,
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
            x_axis,
            bar_spacing_px,
            min_x_unit,
            max_x_unit,
            min_primary_unit,
            max_primary_unit,
            primary_domain_display_override,
            primary_unit_step,
            primary_panel,
            primary_mark,
            primary_scale_mode,
            primary_scale_anchor,
            primary_value_step,
            primary_value_decimals,
            series_percent_anchors,
            indicator_panels,
            panel_controls,
            corner_controls,
            ticker_legend,
            controls_visible_for_panel: None,
            hovered_control: None,
            hovered_corner_control: None,
            hovering_ticker_legend: false,
            hovered_ticker_row: None,
            hovered_ticker_icon: None,
            cursor: None,
        };

        if let Some(local) = cursor_root_local {
            scene.hovered_control =
                Self::hit_panel_control(&scene.layout, &scene.panel_controls, local);
            scene.hovered_corner_control = Self::hit_corner_control(&scene.corner_controls, local);

            let mut legend_hit = scene
                .ticker_legend
                .as_ref()
                .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));

            if legend_hit.is_some() {
                scene.ticker_legend =
                    self.build_ticker_legend_layout(&scene.layout, primary_panel, false, true);
                legend_hit = scene
                    .ticker_legend
                    .as_ref()
                    .and_then(|legend| Self::hit_ticker_legend(&scene.layout, legend, local));
            }

            if let Some(hit) = legend_hit {
                scene.hovering_ticker_legend = true;
                match hit {
                    TickerLegendHit::Background => {}
                    TickerLegendHit::Row(index) => {
                        scene.hovered_ticker_row = Some(index);
                    }
                    TickerLegendHit::Icon(index, kind) => {
                        scene.hovered_ticker_row = Some(index);
                        scene.hovered_ticker_icon = Some((index, kind));
                    }
                }
            }

            scene.controls_visible_for_panel = scene
                .hovered_control
                .map(|hit| hit.panel_index)
                .or_else(|| self.control_visibility_panel(&scene.layout, local));
        }

        let mut cursor_info = None;

        if let Some(local) = cursor_root_local {
            let zone = scene.layout.hit_test(local);

            if matches!(zone, LayoutHitZone::PanelPlot(_))
                && let Some(plot_local) = scene.layout.plot_local_point(local)
            {
                let x_plot = plot_local.x.clamp(0.0, primary_plot.width.max(1.0));
                let snapped_x_unit = scene.unit_from_plot_x(x_plot);
                let snapped_x_plot = scene
                    .map_x_plot(snapped_x_unit)
                    .clamp(0.0, scene.plot_right_edge_px());

                if let LayoutHitZone::PanelPlot(panel_index) = zone
                    && let Some(panel) = scene.layout.panel(panel_index)
                {
                    let panel_value_precision = self.panel_value_precision(panel_index);
                    let y_in_panel =
                        (plot_local.y - panel.plot.y).clamp(0.0, panel.plot.height.max(1.0));

                    let (y_primary_unit, y_indicator_unit, snapped_y_plot) = match panel.kind {
                        KlinePanelKind::PrimaryChart => {
                            let value_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let uses_display_transform = scene
                                .primary_scale_mode
                                .uses_display_transform(scene.can_use_log_primary_scale());

                            let (y_primary_unit, snapped_y_plot) = if uses_display_transform {
                                let (min_display, max_display) =
                                    scene.primary_domain_display_values();
                                let y_display_value =
                                    min_display + ((max_display - min_display) * value_ratio);
                                let y_primary_unit = self.panel_value_to_unit(
                                    panel_value_precision,
                                    scene.primary_display_to_value(y_display_value),
                                );
                                let y_primary_value =
                                    self.panel_unit_to_value(panel_value_precision, y_primary_unit);
                                let y_plot = scene
                                    .map_primary_plot_with_anchor(
                                        y_primary_value,
                                        scene.primary_scale_anchor,
                                    )
                                    .clamp(panel.plot.y, panel.plot.y + panel.plot.height);

                                (y_primary_unit, y_plot)
                            } else {
                                let y_primary_unit = Scene::lerp_unit(
                                    scene.min_primary_unit,
                                    scene.max_primary_unit,
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
                            let indicator_ratio = 1.0 - (y_in_panel / panel.plot.height.max(1.0));
                            let y_indicator_unit = scene
                                .indicator_panel_config(panel_index)
                                .map(|indicator| {
                                    Scene::lerp_unit(
                                        indicator.min_unit,
                                        indicator.max_unit,
                                        indicator_ratio,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    Scene::lerp_unit(YUnit(0), YUnit(1), indicator_ratio)
                                });
                            let snapped_y_plot = scene
                                .map_indicator_plot_unit(panel_index, y_indicator_unit)
                                .map(|y| y.clamp(panel.plot.y, panel.plot.y + panel.plot.height))
                                .unwrap_or(panel.plot.y + y_in_panel);

                            (None, Some(y_indicator_unit), snapped_y_plot)
                        }
                    };

                    cursor_info = Some(CursorInfo {
                        x_unit: snapped_x_unit,
                        panel_index,
                        y_primary_unit,
                        y_indicator_unit,
                        x_plot: snapped_x_plot,
                        y_plot: snapped_y_plot,
                    });
                }
            }
        }

        scene.cursor = cursor_info;
        Some(scene)
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
            XAxis::Tick { .. } => x_axis
                .tick_from_unit(unit)
                .map_or_else(|| unit.to_string(), |index| index.as_u64().to_string()),
        }
    }
}
