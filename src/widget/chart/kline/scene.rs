use super::chrome::{
    CornerControlHit, FootprintLegendIconKind, FootprintLegendLayout, IndicatorLegendLayout,
    PanelControlHit, StudiesLegendIconKind, StudyLegendIconKind, StudyLegendLayout,
    TickerLegendIconKind, TickerLegendLayout,
};
use super::coord::{ChartCoord, ChartStepMs};
use super::layout::PanelLayoutTree;
use super::{BarSpacingPx, YUnit};
use crate::widget::chart::kline::composition::{
    LayerDataKind, MarkKind, PanelPresentation, PanelScaleMode, PanelValueId,
};

use data::chart::kline_v2::WorldAxis;
use data::chart::ticks::{Y_LABEL_DENSITY, y::YAxisScale, y_labels_that_fit};
use exchange::{Timeframe, UnixMs};

use iced::Rectangle;

pub(super) const Y_UNIT_STEP_FALLBACK: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct TickIndex(u64);

impl TickIndex {
    pub(super) const ZERO: Self = Self(0);

    #[inline]
    pub(super) fn as_u64(self) -> u64 {
        self.0
    }

    pub(super) fn from_u64(value: u64) -> Self {
        Self(value)
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
    pub(super) fn unit_from_time(self, value: UnixMs) -> i64 {
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

    pub(super) fn unit_from_time_f32(self, value: UnixMs) -> Option<f32> {
        match self {
            Self::Time { timeframe, anchor } => {
                let delta_ms = value.as_u64() as f64 - anchor.as_u64() as f64;
                Some((delta_ms / timeframe.to_milliseconds().max(1) as f64) as f32)
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    pub(super) fn unit_from_tick(self, value: TickIndex) -> i64 {
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
    pub(super) fn time_from_unit(self, unit: i64) -> Option<UnixMs> {
        match self {
            Self::Time { timeframe, anchor } => {
                let step = i64::try_from(timeframe.to_milliseconds()).unwrap_or(i64::MAX);
                Some(anchor.saturating_add_signed(unit.saturating_mul(step)))
            }
            Self::Tick { .. } => None,
        }
    }

    #[inline]
    pub(super) fn tick_from_unit(self, unit: i64) -> Option<TickIndex> {
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
    pub(super) fn step_ms(self, step_units: i64) -> u64 {
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
pub(super) struct ValueDomain {
    pub(super) units: (YUnit, YUnit),
    pub(super) values: (f32, f32),
}

impl ValueDomain {
    pub(super) fn new(units: (YUnit, YUnit), values: (f32, f32)) -> Self {
        Self { units, values }
    }

    pub(super) fn min_unit(self) -> YUnit {
        self.units.0
    }

    pub(super) fn max_unit(self) -> YUnit {
        self.units.1
    }

    pub(super) fn min_value(self) -> f32 {
        self.values.0
    }

    pub(super) fn max_value(self) -> f32 {
        self.values.1
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct XViewport {
    pub(super) axis: XAxis,
    pub(super) world: WorldAxis,
    pub(super) bar_spacing_px: BarSpacingPx,
    pub(super) logical_bar_spacing: f32,
    pub(super) fractional_offset: f32,
    pub(super) min_unit: i64,
    pub(super) max_unit: i64,
}

#[derive(Debug, Clone)]
pub(super) struct PrimaryScale {
    pub(super) panel_index: usize,
    pub(super) domain: ValueDomain,
    pub(super) display_domain_override: Option<(f32, f32)>,
    pub(super) unit_step: f32,
    pub(super) mark: MarkKind,
    pub(super) mode: PanelScaleMode,
    pub(super) anchor: Option<f32>,
    pub(super) value_step: Option<f32>,
    pub(super) value_decimals: Option<usize>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorPanelScene {
    pub(super) panel_index: usize,
    pub(super) value_id: Option<PanelValueId>,
    pub(super) unit_step: f32,
    pub(super) domain: ValueDomain,
    pub(super) mark: MarkKind,
    pub(super) data_kind: LayerDataKind,
    pub(super) presentation: PanelPresentation,
}

impl IndicatorPanelScene {
    pub(super) fn min_value(self) -> f32 {
        self.domain.min_value()
    }

    pub(super) fn max_value(self) -> f32 {
        self.domain.max_value()
    }

    pub(super) fn min_unit(self) -> YUnit {
        self.domain.min_unit()
    }

    pub(super) fn max_unit(self) -> YUnit {
        self.domain.max_unit()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ChromeScene {
    pub(super) panel_controls: Vec<PanelControlHit>,
    pub(super) corner_controls: Vec<CornerControlHit>,
    pub(super) ticker_legend: Option<TickerLegendLayout>,
    pub(super) footprint_legend: Option<FootprintLegendLayout>,
    pub(super) study_legend: Option<StudyLegendLayout>,
    pub(super) indicator_legend: Option<IndicatorLegendLayout>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SceneInteraction {
    pub(super) controls_visible_for_panel: Option<usize>,
    pub(super) hovered_control: Option<PanelControlHit>,
    pub(super) hovered_corner_control: Option<CornerControlHit>,
    pub(super) hovering_ticker_legend: bool,
    pub(super) hovered_ticker_row: Option<usize>,
    pub(super) hovered_ticker_icon: Option<(usize, TickerLegendIconKind)>,
    pub(super) hovering_footprint_legend: bool,
    pub(super) hovered_footprint_icon: Option<FootprintLegendIconKind>,
    pub(super) hovering_study_legend: bool,
    pub(super) hovered_study_row: Option<usize>,
    pub(super) hovered_study_icon: Option<(usize, StudyLegendIconKind)>,
    pub(super) hovered_studies_icon: Option<StudiesLegendIconKind>,
    pub(super) hovering_indicator_legend: bool,
    pub(super) hovered_indicator_row: Option<usize>,
    pub(super) hovered_indicator_icon: Option<(usize, super::chrome::IndicatorLegendIconKind)>,
    pub(super) cursor: Option<CursorInfo>,
}

#[derive(Debug, Clone)]
pub(super) struct Scene {
    pub(super) layout: PanelLayoutTree,
    pub(super) x: XViewport,
    pub(super) primary: PrimaryScale,
    pub(super) indicator_panels: Vec<IndicatorPanelScene>,
    pub(super) chrome: ChromeScene,
    pub(super) interaction: SceneInteraction,
}

#[derive(Debug, Clone)]
pub(super) struct PrimaryPercentScaleState {
    pub(super) primary_scale_anchor: Option<f32>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
    pub(super) primary_domain_display_override: Option<(f32, f32)>,
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

    pub(super) fn unit_step_or_default(step: Option<f32>) -> f32 {
        step.unwrap_or(Y_UNIT_STEP_FALLBACK).abs().max(1e-8)
    }

    pub(super) fn value_to_unit_with_step(value: f32, unit_step: f32) -> YUnit {
        if !value.is_finite() || !unit_step.is_finite() || unit_step <= 0.0 {
            return YUnit(0);
        }

        YUnit(Self::round_to_i64_saturating(value / unit_step))
    }

    pub(super) fn unit_to_value_with_step(y_unit: YUnit, unit_step: f32) -> f32 {
        (y_unit.0 as f32) * unit_step
    }

    fn primary_value_to_unit(&self, value: f32) -> YUnit {
        Self::value_to_unit_with_step(value, self.primary.unit_step)
    }

    pub(super) fn primary_unit_to_value(&self, y_unit: YUnit) -> f32 {
        Self::unit_to_value_with_step(y_unit, self.primary.unit_step)
    }

    pub(super) fn lerp_unit(min_unit: YUnit, max_unit: YUnit, ratio: f32) -> YUnit {
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
            .panel(self.primary.panel_index)
            .map(|panel| &panel.plot)
            .or_else(|| self.layout.panels.first().map(|panel| &panel.plot))
            .unwrap_or(&self.layout.regions.plot)
    }

    pub(super) fn indicator_panel_config(
        &self,
        panel_index: usize,
    ) -> Option<&IndicatorPanelScene> {
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
        self.x.bar_spacing_px
    }

    pub(super) fn logical_bar_spacing(&self) -> f32 {
        self.x.logical_bar_spacing
    }

    pub(super) fn x_axis_plot_width(&self) -> f32 {
        (self.layout.regions.x_axis.width - self.layout.regions.y_axis.width)
            .max(1.0)
            .min(self.primary_plot().width.max(1.0))
    }

    pub(super) fn effective_right_x_unit(&self) -> f32 {
        self.x.max_unit as f32 + self.x.fractional_offset
    }

    pub(super) fn plot_right_edge_px(&self) -> f32 {
        self.x_axis_plot_width().floor().max(1.0)
    }

    pub(super) fn map_x_plot(&self, x_unit: i64) -> f32 {
        self.map_x_plot_f32(x_unit as f32)
    }

    pub(super) fn map_x_plot_f32(&self, x_unit: f32) -> f32 {
        self.x
            .world
            .world_to_screen(x_unit, self.plot_right_edge_px())
    }

    pub(super) fn x_unit_for_time(&self, time: UnixMs) -> Option<i64> {
        match self.x.axis {
            XAxis::Time { .. } => Some(self.x.axis.unit_from_time(time)),
            XAxis::Tick { .. } => None,
        }
    }

    pub(super) fn time_for_x_unit(&self, x_unit: i64) -> Option<UnixMs> {
        self.x.axis.time_from_unit(x_unit)
    }

    pub(super) fn unit_from_plot_x(&self, x_plot: f32) -> i64 {
        let right_edge_px = self.plot_right_edge_px();
        let unit = self
            .x
            .world
            .screen_to_world(x_plot.clamp(0.0, right_edge_px), right_edge_px)
            .round() as i64;

        unit.clamp(self.x.min_unit, self.x.max_unit)
    }

    pub(super) fn primary_domain_display_values(&self) -> (f32, f32) {
        if let Some((min_display, max_display)) = self.primary.display_domain_override {
            return (min_display, max_display);
        }

        let (min_primary_value, max_primary_value) = self.primary.domain.values;
        let log_scale_enabled = self.can_use_log_primary_scale();

        (
            self.primary.mode.value_to_display(
                min_primary_value,
                self.primary.anchor,
                log_scale_enabled,
            ),
            self.primary.mode.value_to_display(
                max_primary_value,
                self.primary.anchor,
                log_scale_enabled,
            ),
        )
    }

    pub(super) fn primary_display_to_value(&self, display_value: f32) -> f32 {
        self.primary.mode.display_to_value(
            display_value,
            self.primary.anchor,
            self.can_use_log_primary_scale(),
        )
    }

    pub(super) fn primary_domain_value_range(&self) -> (f32, f32) {
        let (min_display, max_display) = self.primary_domain_display_values();
        (
            self.primary_display_to_value(min_display),
            self.primary_display_to_value(max_display),
        )
    }

    pub(super) fn can_use_log_primary_scale(&self) -> bool {
        self.primary.domain.values.0 > f32::EPSILON && self.primary.domain.values.1 > f32::EPSILON
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
        self.primary
            .value_step
            .map(|step| step.max(fallback))
            .unwrap_or(fallback)
    }

    pub(super) fn format_primary_axis_label(
        &self,
        display_value: f32,
        display_step: f32,
    ) -> String {
        if let Some(percent_label) =
            self.primary
                .mode
                .format_percent_label(display_value, display_step, false)
        {
            return percent_label;
        }

        if let Some((log_value, value_step)) = self.primary.mode.log_axis_value_and_step(
            display_value,
            display_step,
            self.primary.anchor,
            self.can_use_log_primary_scale(),
        ) {
            let value = self.quantized_primary_value(log_value);
            return super::super::format_value(value, self.primary_format_step(value_step));
        }

        let value = self.quantized_primary_value(display_value);
        if let Some(decimals) = self.primary.value_decimals {
            format!("{value:.decimals$}")
        } else if let Some(step) = self.primary.value_step {
            let fallback_decimals = Self::decimals_for_step(step);
            format!("{value:.fallback_decimals$}")
        } else {
            super::super::format_value(value, self.primary_format_step(display_step))
        }
    }

    pub(super) fn format_primary_cursor_label(&self, raw_value: f32) -> String {
        let quantized = self.quantized_primary_value(raw_value);

        if let Some(display_step) = self.primary_percent_display_step() {
            let display_value = self.primary.mode.value_to_display(
                quantized,
                self.primary.anchor,
                self.can_use_log_primary_scale(),
            );

            if let Some(percent_label) =
                self.primary
                    .mode
                    .format_percent_label(display_value, display_step, true)
            {
                return percent_label;
            }
        }

        if let Some(decimals) = self.primary.value_decimals {
            format!("{quantized:.decimals$}")
        } else if let Some(step) = self.primary.value_step {
            let fallback_decimals = Self::decimals_for_step(step);
            format!("{quantized:.fallback_decimals$}")
        } else {
            super::super::format_value(quantized, self.primary_format_step(0.01))
        }
    }

    pub(super) fn primary_percent_display_step(&self) -> Option<f32> {
        if !self.primary.mode.uses_percent_base() {
            return None;
        }

        let (min_display, max_display) = self.primary_domain_display_values();
        let labels_can_fit = y_labels_that_fit(
            self.primary_plot().height,
            super::TEXT_SIZE,
            Y_LABEL_DENSITY,
        )
        .max(2) as i32;

        YAxisScale::Percent
            .ticks(
                f64::from(min_display),
                f64::from(max_display),
                labels_can_fit,
            )
            .step
            .map(|step| (step as f32).abs().max(1e-6))
    }

    pub(super) fn map_primary_plot_unit(&self, y_unit: YUnit) -> f32 {
        self.map_primary_plot_value(self.primary_unit_to_value(y_unit))
    }

    pub(super) fn map_primary_plot_value(&self, value: f32) -> f32 {
        let (min, max) = self.primary.domain.values;
        let ratio = (value - min) / (max - min).abs().max(1e-8);
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
        let value = Self::unit_to_value_with_step(indicator_unit, indicator.unit_step);
        let ratio = (value - indicator.min_value())
            / (indicator.max_value() - indicator.min_value())
                .abs()
                .max(1e-8);
        Some(panel.y + (1.0 - ratio) * panel.height)
    }

    pub(super) fn map_primary_plot_with_anchor(&self, value: f32, anchor: Option<f32>) -> f32 {
        let log_scale_enabled = self.can_use_log_primary_scale();
        let uses_display_transform = self.primary.mode.uses_display_transform(log_scale_enabled);

        if !uses_display_transform {
            return self.map_primary_plot_value(value);
        }

        let (min_display, max_display) = self.primary_domain_display_values();
        let range = (max_display - min_display).abs().max(1e-6);
        let display_anchor = self
            .primary
            .mode
            .display_anchor(anchor, self.primary.anchor);
        let display_value =
            self.primary
                .mode
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
        let panel = self.indicator_plot(panel_index)?;
        let indicator = self.indicator_panel_config(panel_index)?;
        let ratio = (indicator_value - indicator.min_value())
            / (indicator.max_value() - indicator.min_value())
                .abs()
                .max(1e-8);
        Some(panel.y + (1.0 - ratio) * panel.height)
    }
}
