pub(super) mod domain;
pub(super) mod viewport;
use super::KlineWidgetEvent;
use super::chrome::{ChromeClick, StudiesLegend};
use super::chrome::{
    CornerControlHit, CornerControlKind, FootprintLegendHit, FootprintLegendIconKind,
    FootprintLegendLayout, IndicatorLegendHit, IndicatorLegendLayout, LegendRowAction,
    PanelControlHit, PanelControlKind, StudiesLegendIconKind, StudyLegendHit, StudyLegendLayout,
    TickerLegendHit, TickerLegendIconKind, TickerLegendLayout,
};
use super::composition::{
    ChartComposition, MarkKind, PanelId, PanelPresentation, PanelScaleMode, PanelValueId,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use super::drawing::DrawingHit;
use super::layout::KlinePanelKind;
use super::layout::PanelLayoutTree;
use super::studies::LayerHit;
use super::{KlineSeries, YUnit};
use data::chart::kline::viewport::YUnitRange;
use data::{UserTimezone, chart::Basis};

use data::chart::kline::WorldAxis;
use data::chart::ticks::{Y_LABEL_DENSITY, y::YAxisScale, y_labels_that_fit};
use exchange::{TickerInfo, Timeframe, UnixMs, unit::Price};

use iced::Rectangle;
use std::sync::Arc;

pub(super) use data::chart::kline::viewport::PanelValueScale;

pub(super) use data::chart::kline::viewport::{BarUnitWindow, TickIndex, XAxis};

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
pub(super) struct PanelDomainState {
    active: YUnitRange,
    autoscale: YUnitRange,
}

impl PanelDomainState {
    pub(super) fn new(autoscale: YUnitRange) -> Self {
        Self {
            active: autoscale,
            autoscale,
        }
    }

    pub(super) fn active(self) -> YUnitRange {
        self.active
    }

    fn autoscale(self) -> YUnitRange {
        self.autoscale
    }

    fn set_active(&mut self, range: YUnitRange) {
        self.active = range;
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct XViewport {
    pub(super) axis: XAxis,
    pub(super) world: WorldAxis,
    pub(super) fractional_offset: f32,
    pub(super) min_unit: i64,
    pub(super) max_unit: i64,
}

#[derive(Debug, Clone)]
pub(super) struct PrimaryScale {
    pub(super) panel_index: usize,
    pub(super) domain: PanelDomainState,
    pub(super) display_domain_override: Option<(f32, f32)>,
    pub(super) value_scale: PanelValueScale,
    pub(super) mark: MarkKind,
    pub(super) mode: PanelScaleMode,
    pub(super) inverted: bool,
    pub(super) anchor: Option<f32>,
    pub(super) value_step: Option<f32>,
    pub(super) value_decimals: Option<usize>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IndicatorPanelScene {
    pub(super) panel_index: usize,
    pub(super) value_id: Option<PanelValueId>,
    pub(super) value_scale: PanelValueScale,
    pub(super) domain: PanelDomainState,
    pub(super) independent_domain: Option<YUnitRange>,
    pub(super) presentation: PanelPresentation,
}

impl IndicatorPanelScene {
    pub(super) fn min_value(self) -> f64 {
        self.value_scale.unit_to_value(self.domain.active().min())
    }

    pub(super) fn max_value(self) -> f64 {
        self.value_scale.unit_to_value(self.domain.active().max())
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LegendTarget {
    Ticker {
        row: usize,
        icon: Option<TickerLegendIconKind>,
    },
    Footprint {
        icon: Option<FootprintLegendIconKind>,
    },
    StudyCategory {
        icon: Option<StudiesLegendIconKind>,
    },
    Study {
        row: usize,
        icon: Option<LegendRowAction>,
    },
    Indicator {
        row: usize,
        icon: Option<LegendRowAction>,
    },
}

impl From<TickerLegendHit> for LegendTarget {
    fn from(hit: TickerLegendHit) -> Self {
        match hit {
            TickerLegendHit::Row(row) => Self::Ticker { row, icon: None },
            TickerLegendHit::Icon(row, icon) => Self::Ticker {
                row,
                icon: Some(icon),
            },
        }
    }
}

impl From<FootprintLegendHit> for LegendTarget {
    fn from(hit: FootprintLegendHit) -> Self {
        match hit {
            FootprintLegendHit::Row => Self::Footprint { icon: None },
            FootprintLegendHit::Icon(icon) => Self::Footprint { icon: Some(icon) },
        }
    }
}

impl From<StudyLegendHit> for LegendTarget {
    fn from(hit: StudyLegendHit) -> Self {
        match hit {
            StudyLegendHit::CategoryRow => Self::StudyCategory { icon: None },
            StudyLegendHit::CategoryIcon(icon) => Self::StudyCategory { icon: Some(icon) },
            StudyLegendHit::StudyRow(row) => Self::Study { row, icon: None },
            StudyLegendHit::StudyIcon(row, icon) => Self::Study {
                row,
                icon: Some(icon),
            },
        }
    }
}

impl From<IndicatorLegendHit> for LegendTarget {
    fn from(hit: IndicatorLegendHit) -> Self {
        match hit {
            IndicatorLegendHit::Row(row) => Self::Indicator { row, icon: None },
            IndicatorLegendHit::Icon(row, icon) => Self::Indicator {
                row,
                icon: Some(icon),
            },
        }
    }
}

impl LegendTarget {
    pub(super) fn has_icon(self) -> bool {
        matches!(
            self,
            Self::Ticker { icon: Some(_), .. }
                | Self::Footprint { icon: Some(_), .. }
                | Self::StudyCategory { icon: Some(_), .. }
                | Self::Study { icon: Some(_), .. }
                | Self::Indicator { icon: Some(_), .. }
        )
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorTarget {
    ActiveDrag,
    ActivePan,
    #[default]
    Outside,
    Plot(usize),
    Splitter(usize),
    YAxis(usize),
    PanelXAxis(usize),
    BottomXAxis,
    PanelControl {
        panel_index: usize,
        kind: PanelControlKind,
    },
    CornerControl(CornerControlKind),
    Legend(LegendTarget),
    Drawing(DrawingHit),
    StudyObject(LayerHit),
}

impl CursorTarget {
    fn readouts_visible(self) -> bool {
        match self {
            Self::StudyObject(hit) => hit.readouts_visible,
            _ => matches!(self, Self::Plot(_) | Self::Drawing(_) | Self::ActivePan),
        }
    }

    pub(super) fn ticker_row_is_hovered(self, index: usize) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::Ticker { row, .. }) if row == index
        )
    }

    pub(super) fn ticker_icon_is_hovered(self, index: usize, kind: TickerLegendIconKind) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::Ticker {
                row,
                icon: Some(icon),
            }) if row == index && icon == kind
        )
    }

    pub(super) fn footprint_legend_is_hovered(self) -> bool {
        matches!(self, Self::Legend(LegendTarget::Footprint { .. }))
    }

    pub(super) fn footprint_icon_is_hovered(self, kind: FootprintLegendIconKind) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::Footprint { icon: Some(icon) }) if icon == kind
        )
    }

    pub(super) fn study_category_is_hovered(self) -> bool {
        matches!(self, Self::Legend(LegendTarget::StudyCategory { .. }))
    }

    pub(super) fn studies_icon_is_hovered(self, kind: StudiesLegendIconKind) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::StudyCategory { icon: Some(icon) }) if icon == kind
        )
    }

    pub(super) fn study_row_is_hovered(self, index: usize) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::Study { row, .. }) if row == index
        )
    }

    pub(super) fn indicator_row_is_hovered(self, index: usize) -> bool {
        matches!(
            self,
            Self::Legend(LegendTarget::Indicator { row, .. }) if row == index
        )
    }

    pub(super) fn indicator_row(self) -> Option<usize> {
        match self {
            Self::Legend(LegendTarget::Indicator { row, .. }) => Some(row),
            _ => None,
        }
    }

    pub(super) fn panel_control_is_hovered(
        self,
        panel_index: usize,
        kind: PanelControlKind,
    ) -> bool {
        matches!(
            self,
            Self::PanelControl {
                panel_index: hovered_panel,
                kind: hovered_kind,
            } if hovered_panel == panel_index && hovered_kind == kind
        )
    }

    pub(super) fn corner_control_is_hovered(self, kind: CornerControlKind) -> bool {
        matches!(self, Self::CornerControl(hovered_kind) if hovered_kind == kind)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SceneInteraction {
    pub(super) target: CursorTarget,
    pub(super) controls_visible_for_panel: Option<usize>,
    pub(super) cursor: Option<CursorInfo>,
}

impl SceneInteraction {
    pub(super) fn readouts_visible(&self) -> bool {
        self.target.readouts_visible()
    }
}

#[derive(Debug)]
pub(super) struct SceneBase {
    pub(super) layout: PanelLayoutTree,
    pub(super) x: XViewport,
    pub(super) bar_units: Arc<[BarUnitWindow]>,
    pub(super) primary: PrimaryScale,
    pub(super) indicator_panels: Vec<IndicatorPanelScene>,
    pub(super) panel_controls: Vec<PanelControlHit>,
    pub(super) corner_controls: Vec<CornerControlHit>,
}

#[derive(Debug, Clone)]
pub(super) struct Scene {
    pub(super) layout: PanelLayoutTree,
    pub(super) x: XViewport,
    pub(super) bar_units: Arc<[BarUnitWindow]>,
    pub(super) primary: PrimaryScale,
    pub(super) indicator_panels: Vec<IndicatorPanelScene>,
    pub(super) chrome: ChromeScene,
    pub(super) interaction: SceneInteraction,
}

impl Scene {
    pub(super) fn from_base(base: &SceneBase) -> Self {
        Self {
            layout: base.layout.clone(),
            x: base.x,
            bar_units: Arc::clone(&base.bar_units),
            primary: base.primary.clone(),
            indicator_panels: base.indicator_panels.clone(),
            chrome: ChromeScene {
                panel_controls: base.panel_controls.clone(),
                corner_controls: base.corner_controls.clone(),
                ..ChromeScene::default()
            },
            interaction: SceneInteraction::default(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct SceneCache {
    entry: Option<SceneCacheEntry>,
}

#[derive(Debug)]
pub(super) struct SceneCacheEntry {
    version: u64,
    pub(super) base: SceneBase,
    pub(super) y_axis_labels: super::axes::AxisInputs,
}

impl SceneCacheEntry {
    fn matches(&self, version: u64, layout: &PanelLayoutTree) -> bool {
        self.version == version && self.base.layout == *layout
    }
}

impl SceneCache {
    pub(super) fn get(&self, version: u64) -> Option<&SceneCacheEntry> {
        let entry = self.entry.as_ref()?;
        (entry.version == version).then_some(entry)
    }

    pub(super) fn get_or_build(
        &mut self,
        version: u64,
        layout: PanelLayoutTree,
        build: impl FnOnce(PanelLayoutTree) -> Option<(SceneBase, super::axes::AxisInputs)>,
    ) -> Option<&SceneCacheEntry> {
        let cache_hit = self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.matches(version, &layout));

        if !cache_hit {
            self.entry = build(layout).map(|(base, y_axis_labels)| SceneCacheEntry {
                version,
                base,
                y_axis_labels,
            });
        }

        self.entry.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(super) struct PrimaryPercentScaleState {
    pub(super) primary_scale_anchor: Option<f32>,
    pub(super) series_percent_anchors: Vec<Option<f32>>,
    pub(super) primary_domain_display_override: Option<(f32, f32)>,
}

impl Scene {
    fn primary_value_to_unit(&self, value: f32) -> YUnit {
        self.primary.value_scale.value_to_unit_f32(value)
    }

    fn primary_unit_to_value(&self, y_unit: YUnit) -> f32 {
        self.primary.value_scale.unit_to_value_f32(y_unit)
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

    pub(super) fn logical_bar_spacing(&self) -> f32 {
        self.x.world.spacing()
    }

    pub(super) fn x_axis_plot_width(&self) -> f32 {
        (self.layout.regions.x_axis.width - self.layout.regions.y_axis.width)
            .max(1.0)
            .min(self.primary_plot().width.max(1.0))
    }

    pub(super) fn effective_right_x_unit(&self) -> f32 {
        self.x.max_unit as f32 + self.x.fractional_offset
    }

    pub(super) fn map_x_plot(&self, x_unit: i64) -> f32 {
        self.map_x_plot_f32(x_unit as f32)
    }

    pub(super) fn map_x_plot_f32(&self, x_unit: f32) -> f32 {
        self.x
            .world
            .world_to_screen(x_unit, self.x_axis_plot_width().max(1.0))
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

    pub(super) fn time_from_plot_x_f32(&self, x_plot: f32) -> Option<UnixMs> {
        let x_plot = x_plot.clamp(0.0, self.x_axis_plot_width().max(1.0));
        let unit = self
            .x
            .world
            .screen_to_logical_index(x_plot, self.x_axis_plot_width().max(1.0));
        self.x.axis.time_from_unit_f32(unit)
    }

    pub(super) fn unit_from_plot_x(&self, x_plot: f32) -> i64 {
        let right_edge_px = self.x_axis_plot_width().max(1.0);
        let unit = self
            .x
            .world
            .screen_to_logical_index(x_plot.clamp(0.0, right_edge_px), right_edge_px)
            .ceil() as i64;

        unit.clamp(self.x.min_unit, self.x.max_unit)
    }

    pub(super) fn primary_domain_display_values(&self) -> (f32, f32) {
        if let Some((min_display, max_display)) = self.primary.display_domain_override {
            return (min_display, max_display);
        }

        let (min_primary_value, max_primary_value) = self
            .primary
            .value_scale
            .values_f32(self.primary.domain.active());
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

    fn primary_autoscale_display_values(&self) -> (f32, f32) {
        if let Some((min_display, max_display)) = self.primary.display_domain_override {
            return (min_display, max_display);
        }

        let (min_value, max_value) = self
            .primary
            .value_scale
            .values_f32(self.primary.domain.autoscale());
        let log_scale_enabled = self.can_use_log_primary_scale();
        (
            self.primary
                .mode
                .value_to_display(min_value, self.primary.anchor, log_scale_enabled),
            self.primary
                .mode
                .value_to_display(max_value, self.primary.anchor, log_scale_enabled),
        )
    }

    pub(super) fn primary_display_to_value(&self, display_value: f32) -> f32 {
        self.primary.mode.display_to_value(
            display_value,
            self.primary.anchor,
            self.can_use_log_primary_scale(),
        )
    }

    pub(super) fn primary_domain_value_range(&self) -> (Price, Price) {
        let (min_display, max_display) = self.primary_domain_display_values();
        let min_value = self.primary_display_to_value(min_display);
        let max_value = self.primary_display_to_value(max_display);
        (
            Price::from_f32(min_value.min(max_value)),
            Price::from_f32(min_value.max(max_value)),
        )
    }

    pub(super) fn can_use_log_primary_scale(&self) -> bool {
        let (min, max) = self
            .primary
            .value_scale
            .values_f32(self.primary.domain.autoscale());
        min > f32::EPSILON && max > f32::EPSILON
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
            return format_value(value, self.primary_format_step(value_step));
        }

        let value = self.quantized_primary_value(display_value);
        if let Some(decimals) = self.primary.value_decimals {
            format!("{value:.decimals$}")
        } else if let Some(step) = self.primary.value_step {
            let fallback_decimals = Self::decimals_for_step(step);
            format!("{value:.fallback_decimals$}")
        } else {
            format_value(value, self.primary_format_step(display_step))
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
            format_value(quantized, self.primary_format_step(0.01))
        }
    }

    fn primary_percent_display_step(&self) -> Option<f32> {
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
        let (min, max) = self
            .primary
            .value_scale
            .values_f32(self.primary.domain.active());
        let ratio = (value - min) / (max - min).abs().max(1e-8);
        self.map_primary_plot_ratio(ratio)
    }

    pub(super) fn map_primary_plot_ratio(&self, ratio: f32) -> f32 {
        map_ratio_to_plot_y(*self.primary_plot(), ratio, self.primary.inverted)
    }

    pub(super) fn primary_value_ratio_at_plot_y(&self, y_in_panel: f32) -> f32 {
        let screen_ratio = (y_in_panel / self.primary_plot().height.max(1.0)).clamp(0.0, 1.0);
        if self.primary.inverted {
            screen_ratio
        } else {
            1.0 - screen_ratio
        }
    }

    pub(super) fn map_indicator_plot_unit(
        &self,
        panel_index: usize,
        indicator_unit: YUnit,
    ) -> Option<f32> {
        let panel = self.indicator_plot(panel_index)?;
        let indicator = self.indicator_panel_config(panel_index)?;
        let value = indicator.value_scale.unit_to_value(indicator_unit);
        let ratio = (value - indicator.min_value())
            / (indicator.max_value() - indicator.min_value())
                .abs()
                .max(1e-8);
        if !ratio.is_finite() {
            return None;
        }
        let y = f64::from(panel.y) + (1.0 - ratio) * f64::from(panel.height);
        y.is_finite().then_some(y as f32).filter(|y| y.is_finite())
    }

    pub(super) fn map_primary_plot_with_anchor_f32(&self, value: f32, anchor: Option<f32>) -> f32 {
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
        self.map_primary_plot_ratio(ratio)
    }

    pub(super) fn map_primary_plot_with_anchor(
        &self,
        value: f64,
        anchor: Option<f32>,
    ) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }

        let log_scale_enabled = self.can_use_log_primary_scale();
        let uses_display_transform = self.primary.mode.uses_display_transform(log_scale_enabled);
        let (min, max) = self
            .primary
            .value_scale
            .values_f32(self.primary.domain.active());

        if !uses_display_transform {
            let ratio =
                (value - f64::from(min)) / (f64::from(max) - f64::from(min)).abs().max(1e-8);
            return finite_plot_y(*self.primary_plot(), ratio, self.primary.inverted);
        }

        let (min_display, max_display) = self.primary_domain_display_values();
        let range = (f64::from(max_display) - f64::from(min_display))
            .abs()
            .max(1e-6);
        let display_anchor = if self.primary.mode.uses_percent_base() {
            anchor.map(f64::from)
        } else {
            self.primary.anchor.map(f64::from)
        };
        let display_value =
            if log_scale_enabled && matches!(self.primary.mode, PanelScaleMode::Logarithmic) {
                value.max(f64::MIN_POSITIVE).log10()
            } else if let Some(base) = display_anchor.filter(|base| base.abs() > f64::EPSILON) {
                ((value / base) - 1.0) * 100.0
            } else {
                value
            };
        let ratio = (display_value - f64::from(min_display)) / range;
        finite_plot_y(*self.primary_plot(), ratio, self.primary.inverted)
    }

    pub(super) fn map_indicator_plot(
        &self,
        panel_index: usize,
        indicator_value: f64,
    ) -> Option<f32> {
        self.map_indicator_plot_scaled(panel_index, indicator_value, false)
    }

    pub(super) fn map_indicator_plot_scaled(
        &self,
        panel_index: usize,
        indicator_value: f64,
        independent_scale: bool,
    ) -> Option<f32> {
        let panel = self.indicator_plot(panel_index)?;
        let indicator = self.indicator_panel_config(panel_index)?;
        let domain = independent_scale
            .then_some(indicator.independent_domain)
            .flatten()
            .unwrap_or(indicator.domain.active());
        let (min_value, max_value) = indicator.value_scale.values(domain);
        if !indicator_value.is_finite() || !min_value.is_finite() || !max_value.is_finite() {
            return None;
        }
        let ratio = (indicator_value - min_value) / (max_value - min_value).abs().max(1e-8);
        if !ratio.is_finite() {
            return None;
        }
        let y = f64::from(panel.y) + (1.0 - ratio) * f64::from(panel.height);
        y.is_finite().then_some(y as f32).filter(|y| y.is_finite())
    }
}

fn map_ratio_to_plot_y(plot: Rectangle, ratio: f32, inverted: bool) -> f32 {
    let screen_ratio = if inverted { ratio } else { 1.0 - ratio };
    plot.y + screen_ratio * plot.height
}

fn finite_plot_y(plot: Rectangle, ratio: f64, inverted: bool) -> Option<f32> {
    if !ratio.is_finite() {
        return None;
    }
    let screen_ratio = if inverted { ratio } else { 1.0 - ratio };
    let y = f64::from(plot.y) + screen_ratio * f64::from(plot.height);
    y.is_finite().then_some(y as f32).filter(|y| y.is_finite())
}

fn format_value(value: f32, step: f32) -> String {
    if step >= 10.0 {
        format!("{value:.0}")
    } else if step >= 1.0 {
        format!("{value:.2}")
    } else if step >= 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.4}")
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ChartCoordinates<'a> {
    pub(super) series: &'a [KlineSeries],
    pub(super) composition: &'a ChartComposition,
    pub(super) basis: Basis,
    pub(super) timezone: UserTimezone,
}

impl<'a> ChartCoordinates<'a> {
    pub(super) fn bar_index_at_or_before_unit(
        &self,
        scene: &Scene,
        series_index: usize,
        target_unit: i64,
    ) -> Option<usize> {
        let series = self.series.get(series_index)?;
        series.bar_index_at_or_before_unit(scene.x.axis, target_unit)
    }

    pub(super) fn panel_index_for_id(&self, panel_id: PanelId) -> Option<usize> {
        self.composition
            .panels
            .iter()
            .position(|panel| panel.id == panel_id)
    }

    pub(super) fn panel_id(&self, panel_index: usize) -> Option<PanelId> {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.id)
    }

    pub(super) fn panel_value_precision(&self, panel_index: usize) -> Option<PanelValuePrecision> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_precision)
    }

    fn panel_value_label_policy(&self, panel_index: usize) -> PanelValueLabelPolicy {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.value_label_policy)
            .unwrap_or_default()
    }

    pub(super) fn base_ticker_info(&self) -> Option<exchange::TickerInfo> {
        self.series.first().map(|series| series.ticker_info)
    }

    pub(super) fn panel_value_scale(&self, panel_index: usize) -> PanelValueScale {
        PanelValueScale::new(
            self.panel_value_precision(panel_index),
            self.base_ticker_info(),
        )
    }

    fn format_panel_value_compact(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f64,
        fallback_step: f64,
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

        let scale = self.panel_value_scale(panel_index);
        let quantized = scale.unit_to_value(scale.value_to_unit(value));
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
        value: f64,
        fallback_step: f64,
    ) -> String {
        let scale = self.panel_value_scale(panel_index);
        self.panel_value_label_policy(panel_index)
            .format_axis_value(
                || scale.unit_to_value(scale.value_to_unit(value)),
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

    pub(super) fn tick_time_for_unit(&self, x_axis: XAxis, unit: i64) -> Option<UnixMs> {
        let from_latest = usize::try_from(x_axis.tick_from_unit(unit)?.as_u64()).ok()?;
        let series = self.series.first()?;
        let index = series
            .point_count()
            .checked_sub(from_latest.checked_add(1)?)?;
        series.time_at_index(index)
    }

    pub(super) fn format_x_label(&self, x_axis: XAxis, unit: i64, step_units: i64) -> String {
        match x_axis {
            XAxis::Time { .. } => x_axis.time_from_unit(unit).map_or_else(
                || unit.to_string(),
                |ts| {
                    let ts_ms = ts.as_u64();
                    let step_ms = x_axis.step_ms(step_units);

                    self.timezone
                        .format_with_kind(
                            ts_ms as i64,
                            data::config::timezone::TimeLabelKind::AxisStepMs { step_ms },
                        )
                        .unwrap_or_else(|| ts_ms.to_string())
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
}

impl ChromeScene {
    pub(super) fn resolve_click(
        &self,
        target: Option<CursorTarget>,
        studies: Option<&StudiesLegend>,
    ) -> ChromeClick {
        let event = match target {
            Some(CursorTarget::Legend(target)) => {
                let event = match target {
                    LegendTarget::Ticker {
                        row,
                        icon: Some(icon),
                    } => self
                        .ticker_legend
                        .as_ref()
                        .and_then(|legend| legend.rows.get(row))
                        .map(|row| icon.into_event(row.ticker)),
                    LegendTarget::Footprint { icon: Some(icon) } => Some(icon.into_event()),
                    LegendTarget::StudyCategory { icon: Some(icon) } => {
                        studies.map(|_| match icon {
                            StudiesLegendIconKind::ExpandCollapse => {
                                KlineWidgetEvent::StudiesExpansionToggled
                            }
                            StudiesLegendIconKind::Visibility => {
                                KlineWidgetEvent::StudiesVisibilityToggled
                            }
                            StudiesLegendIconKind::Close => KlineWidgetEvent::StudiesRemove,
                        })
                    }
                    LegendTarget::Study {
                        row,
                        icon: Some(icon),
                    } => studies
                        .and_then(|legend| legend.rows.get(row))
                        .map(|row| match icon {
                            LegendRowAction::Visibility => {
                                KlineWidgetEvent::StudyVisibilityToggled(row.id)
                            }
                            LegendRowAction::Settings => KlineWidgetEvent::StudySettings(row.id),
                            LegendRowAction::Close => KlineWidgetEvent::StudyRemove(row.id),
                        }),
                    LegendTarget::Indicator {
                        row,
                        icon: Some(icon),
                    } => self
                        .indicator_legend
                        .as_ref()
                        .and_then(|legend| legend.rows.get(row))
                        .map(|row| match icon {
                            LegendRowAction::Visibility => {
                                KlineWidgetEvent::IndicatorVisibilityToggled(row.value_id)
                            }
                            LegendRowAction::Settings => {
                                KlineWidgetEvent::IndicatorSettings(row.value_id)
                            }
                            LegendRowAction::Close => {
                                KlineWidgetEvent::IndicatorRemove(row.value_id)
                            }
                        }),
                    _ => None,
                };
                return event.map_or(ChromeClick::Blocked, ChromeClick::Event);
            }
            Some(CursorTarget::CornerControl(kind)) => kind.into_event(),
            Some(CursorTarget::PanelControl { panel_index, kind }) => {
                KlineWidgetEvent::PanelInteraction(kind.into_interaction_kind(panel_index))
            }
            _ => return ChromeClick::Outside,
        };
        ChromeClick::Event(event)
    }
}

impl Scene {
    pub(super) fn for_each_bar(
        &self,
        series: &[KlineSeries],
        f: impl FnMut(usize, usize, &KlineSeries, i64, data::chart::kline::series::KlineSeriesPoint<'_>),
    ) {
        data::chart::kline::series::KlineWindows {
            series,
            windows: &self.bar_units,
            axis: self.x.axis,
        }
        .for_each_point(self.x.min_unit, self.x.max_unit, f);
    }
}

impl<'a> ChartCoordinates<'a> {
    pub(super) fn format_panel_header_value(
        &self,
        panel_index: usize,
        value_precision: Option<PanelValuePrecision>,
        value: f64,
    ) -> String {
        let scale = self.panel_value_scale(panel_index);
        self.panel_value_label_policy(panel_index)
            .format_header_value(
                || scale.unit_to_value(scale.value_to_unit(value)),
                || self.format_panel_value_compact(panel_index, value_precision, value, 0.01),
            )
    }

    pub(super) fn resolved_panel_title(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> Option<&'a str> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| Self::default_title_for_panel(panel_kind))
    }

    fn default_title_for_panel(kind: KlinePanelKind) -> Option<&'static str> {
        match kind {
            KlinePanelKind::PrimaryChart => None,
            KlinePanelKind::Indicator => Some("Indicator"),
        }
    }

    pub(super) fn series_participates_in_primary_plot(&self, ticker: TickerInfo) -> bool {
        self.series
            .first()
            .is_some_and(|series| series.ticker_info == ticker)
            || self
                .composition
                .comparison_layer(ticker)
                .is_some_and(|layer| layer.visible)
    }
}
