use super::indicators::{IndicatorAttachmentMode, IndicatorPanelRecipe};
use super::{IndicatorId, IndicatorInstanceId, IndicatorInstanceStore, IndicatorOutputRef};
use super::{YUnit, YUnitRange};
pub use crate::chart::value::{PanelValueLabelMode, PanelValueLabelPolicy, PanelValuePrecision};
use exchange::{Ticker, TickerInfo};
use indicator_engine::PaneTarget;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

pub const DEFAULT_MIN_PANEL_RATIO: f32 = 0.08;
const DEFAULT_AUXILIARY_PANEL_RATIO: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PanelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelRole {
    Primary,
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PanelPresentation {
    #[default]
    Plot,
    Table,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerPurpose {
    #[default]
    Plot,
    /// Internal output binding used to identify and populate a table panel.
    TableBinding,
}

impl LayerPurpose {
    pub const fn is_plot(self) -> bool {
        matches!(self, Self::Plot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkKind {
    Candle,
    Bar(BarMode),
    Line,
    StepLine,
    Area,
    Point,
    Cross,
    Segment,
    HorizontalLevel,
}

impl MarkKind {
    pub const fn supports_line_styling(self) -> bool {
        self.is_line_like()
    }

    pub const fn is_line_like(self) -> bool {
        matches!(
            self,
            Self::Line | Self::StepLine | Self::Area | Self::Segment | Self::HorizontalLevel
        )
    }

    /// Marks that consume a bar-anchored numeric series. Sparse outputs are
    /// joined only across consecutive available samples by the renderer.
    pub const fn is_series_line_like(self) -> bool {
        self.is_line_like()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerDataKind {
    Ohlc,
    Scalar,
    Histogram,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistogramMode {
    Plain,
    SignedOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarMode {
    Regular,
    Histogram(HistogramMode),
}

const fn default_bar_mode_for_data_kind(data_kind: LayerDataKind) -> BarMode {
    match data_kind {
        LayerDataKind::Histogram => BarMode::Histogram(HistogramMode::Plain),
        LayerDataKind::Ohlc | LayerDataKind::Scalar | LayerDataKind::Range => BarMode::Regular,
    }
}

pub const fn default_mark_for_data_kind(data_kind: LayerDataKind) -> MarkKind {
    match data_kind {
        LayerDataKind::Ohlc => MarkKind::Candle,
        LayerDataKind::Scalar | LayerDataKind::Range => MarkKind::Line,
        LayerDataKind::Histogram => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
    }
}

const fn supports_mark(data_kind: LayerDataKind, mark: MarkKind) -> bool {
    match data_kind {
        LayerDataKind::Ohlc => matches!(mark, MarkKind::Line | MarkKind::Candle),
        LayerDataKind::Scalar => matches!(
            mark,
            MarkKind::Line
                | MarkKind::StepLine
                | MarkKind::Area
                | MarkKind::Point
                | MarkKind::Cross
                | MarkKind::Segment
                | MarkKind::HorizontalLevel
                | MarkKind::Bar(_)
        ),
        LayerDataKind::Range => matches!(mark, MarkKind::Line),
        LayerDataKind::Histogram => matches!(
            mark,
            MarkKind::Line
                | MarkKind::StepLine
                | MarkKind::Area
                | MarkKind::Point
                | MarkKind::Cross
                | MarkKind::Segment
                | MarkKind::HorizontalLevel
                | MarkKind::Bar(_)
        ),
    }
}

const fn supports_bar_mode(data_kind: LayerDataKind, mode: BarMode) -> bool {
    match mode {
        BarMode::Regular => true,
        BarMode::Histogram(HistogramMode::Plain) => matches!(data_kind, LayerDataKind::Histogram),
        BarMode::Histogram(HistogramMode::SignedOverlay) => {
            matches!(data_kind, LayerDataKind::Histogram)
        }
    }
}

pub fn resolve_mark_for_data_kind(mark: MarkKind, data_kind: LayerDataKind) -> MarkKind {
    let mut resolved = if supports_mark(data_kind, mark) {
        mark
    } else {
        default_mark_for_data_kind(data_kind)
    };

    if let MarkKind::Bar(mode) = resolved
        && !supports_bar_mode(data_kind, mode)
    {
        resolved = MarkKind::Bar(default_bar_mode_for_data_kind(data_kind));
    }

    resolved
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisBinding {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataSourceId {
    Primary,
    Comparison(Ticker),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PanelScaleMode {
    Absolute,
    FitVisible,
    FitVisibleIncludeZero,
    Logarithmic,
    PercentFromBase,
}

impl PanelScaleMode {
    pub fn uses_percent_base(self) -> bool {
        matches!(self, Self::PercentFromBase)
    }

    pub fn uses_series_percent_anchors(self, series_count: usize) -> bool {
        self.uses_percent_base() && series_count > 1
    }

    pub fn primary_autoscale_enabled(self, user_autoscale_enabled: bool) -> bool {
        user_autoscale_enabled || matches!(self, Self::PercentFromBase)
    }

    pub fn allows_primary_manual_pan(self, user_autoscale_enabled: bool) -> bool {
        !self.primary_autoscale_enabled(user_autoscale_enabled)
    }

    pub fn allows_primary_manual_zoom(self) -> bool {
        !matches!(self, Self::PercentFromBase)
    }

    pub fn display_anchor(
        self,
        explicit_anchor: Option<f32>,
        default_anchor: Option<f32>,
    ) -> Option<f32> {
        if self.uses_percent_base() {
            explicit_anchor
        } else {
            default_anchor
        }
    }

    pub fn format_percent_label(
        self,
        display_value: f32,
        display_step: f32,
        show_decimals: bool,
    ) -> Option<String> {
        if !self.uses_percent_base() {
            return None;
        }

        let format_pct = |value: f32, step: f32, show_decimals: bool| {
            let precision = if show_decimals || step.fract() != 0.0 {
                if step >= 1.0 {
                    1
                } else if step >= 0.1 {
                    2
                } else {
                    3
                }
            } else if step >= 1.0 {
                0
            } else if step >= 0.1 {
                1
            } else {
                2
            };

            format!("{value:+.*}%", precision)
        };

        Some(format_pct(display_value, display_step, show_decimals))
    }

    pub fn log_axis_value_and_step(
        self,
        display_value: f32,
        display_step: f32,
        anchor: Option<f32>,
        log_scale_enabled: bool,
    ) -> Option<(f32, f32)> {
        if !matches!(self, Self::Logarithmic) || !log_scale_enabled {
            return None;
        }

        let value = self.display_to_value(display_value, anchor, true);
        let next_value =
            self.display_to_value(display_value + display_step.abs().max(1e-3), anchor, true);
        let value_step = (next_value - value).abs().max(1e-6);

        Some((value, value_step))
    }

    pub fn uses_display_transform(self, log_scale_enabled: bool) -> bool {
        matches!(self, Self::PercentFromBase)
            || (log_scale_enabled && matches!(self, Self::Logarithmic))
    }

    pub fn value_to_display(self, value: f32, anchor: Option<f32>, log_scale_enabled: bool) -> f32 {
        if log_scale_enabled && matches!(self, Self::Logarithmic) {
            value.max(f32::MIN_POSITIVE).log10()
        } else {
            match (self, anchor) {
                (Self::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    ((value / base) - 1.0) * 100.0
                }
                _ => value,
            }
        }
    }

    pub fn display_to_value(
        self,
        display_value: f32,
        anchor: Option<f32>,
        log_scale_enabled: bool,
    ) -> f32 {
        if log_scale_enabled && matches!(self, Self::Logarithmic) {
            10_f32.powf(display_value).max(f32::MIN_POSITIVE)
        } else {
            match (self, anchor) {
                (Self::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    base * (1.0 + display_value / 100.0)
                }
                _ => display_value,
            }
        }
    }

    pub fn indicator_domain_units(self, min_unit: i64, max_unit: i64) -> (YUnit, YUnit) {
        let range = match self {
            Self::FitVisible => YUnitRange::new(YUnit(min_unit), YUnit(max_unit)).padded(),
            Self::FitVisibleIncludeZero => {
                YUnitRange::new(YUnit(min_unit.min(0)), YUnit(max_unit.max(0))).padded()
            }
            _ => YUnitRange::new(YUnit(0), YUnit(max_unit.max(1))),
        };
        (range.min(), range.max())
    }
}

pub type PanelValueId = super::IndicatorInstanceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelComparisonPolicy {
    pub force_percent_scale_on_multi_source: bool,
    pub force_line_for_non_base_sources: bool,
}

impl Default for PanelComparisonPolicy {
    fn default() -> Self {
        Self {
            force_percent_scale_on_multi_source: true,
            force_line_for_non_base_sources: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerSource {
    RawKline {
        source: DataSourceId,
    },
    IndicatorOutput {
        source: DataSourceId,
        output: IndicatorOutputRef,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDataHint {
    ValueLike,
    HistogramLike,
    TableLike,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerStyle {
    pub line_width: f32,
    pub opacity: f32,
}

impl Default for LayerStyle {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerSpec {
    pub id: LayerId,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
    pub purpose: LayerPurpose,
    pub mark: MarkKind,
    pub mark_override: Option<MarkKind>,
    pub axis: AxisBinding,
    pub visible: bool,
    pub style: LayerStyle,
}

impl LayerSpec {
    pub fn effective_mark(&self) -> MarkKind {
        self.mark_override.unwrap_or(self.mark)
    }

    fn is_histogram_like(&self) -> bool {
        matches!(self.axis, AxisBinding::Secondary)
            || matches!(self.data_kind, LayerDataKind::Histogram)
    }

    pub fn source_id(&self) -> Option<DataSourceId> {
        self.source.source_id()
    }
}

impl LayerSource {
    pub fn source_id(&self) -> Option<DataSourceId> {
        match self {
            Self::RawKline { source } => Some(*source),
            Self::IndicatorOutput { source, .. } => Some(*source),
        }
    }

    pub fn indicator_output(&self) -> Option<&IndicatorOutputRef> {
        match self {
            Self::IndicatorOutput { output, .. } => Some(output),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelSpec {
    pub id: PanelId,
    pub role: PanelRole,
    pub presentation: PanelPresentation,
    pub named_pane: Option<u32>,
    pub title: Option<String>,
    pub value_precision: Option<PanelValuePrecision>,
    pub value_label_policy: PanelValueLabelPolicy,
    pub base_layer: Option<LayerId>,
    pub preferred_scale: PanelScaleMode,
    pub comparison_policy: PanelComparisonPolicy,
    pub layers: Vec<LayerSpec>,
}

impl PanelSpec {
    pub fn effective_data_kind(&self) -> LayerDataKind {
        let fallback = match self.role {
            PanelRole::Primary => LayerDataKind::Ohlc,
            PanelRole::Auxiliary => LayerDataKind::Scalar,
        };

        self.base_layer
            .and_then(|base| self.layers.iter().find(|layer| layer.id == base))
            .or_else(|| self.layers.first())
            .map(|layer| layer.data_kind)
            .unwrap_or(fallback)
    }

    pub fn effective_mark(&self) -> MarkKind {
        let fallback = match self.role {
            PanelRole::Primary => default_mark_for_data_kind(LayerDataKind::Ohlc),
            PanelRole::Auxiliary => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        };

        let base_layer_id = self
            .base_layer
            .or_else(|| self.layers.first().map(|layer| layer.id));

        let resolved_marks = self.resolved_marks();

        base_layer_id
            .and_then(|base| {
                resolved_marks
                    .iter()
                    .find(|(layer_id, _)| *layer_id == base)
                    .map(|(_, mark)| *mark)
            })
            .or_else(|| resolved_marks.first().map(|(_, mark)| *mark))
            .unwrap_or(fallback)
    }

    pub fn resolved_marks(&self) -> Vec<(LayerId, MarkKind)> {
        let is_multi_source = self.uses_multi_source();

        self.layers
            .iter()
            .map(|layer| {
                let mut mark = layer.effective_mark();

                if self.comparison_policy.force_line_for_non_base_sources
                    && is_multi_source
                    && self.base_layer != Some(layer.id)
                {
                    mark = MarkKind::Line;
                }

                (layer.id, resolve_mark_for_data_kind(mark, layer.data_kind))
            })
            .collect()
    }

    fn add_layer(&mut self, layer: LayerSpec) -> bool {
        let layer_id = layer.id;
        self.layers.push(layer);
        if self.base_layer.is_none() {
            self.base_layer = Some(layer_id);
        }
        self.enforce_comparison_mark_policy();

        true
    }

    fn remove_layer(&mut self, layer_id: LayerId) -> bool {
        let Some(index) = self.layers.iter().position(|layer| layer.id == layer_id) else {
            return false;
        };

        self.layers.remove(index);

        if self.base_layer == Some(layer_id) {
            self.base_layer = self.layers.first().map(|layer| layer.id);
        }

        self.enforce_comparison_mark_policy();
        true
    }

    pub fn indicator_visible(&self, id: PanelValueId) -> bool {
        self.layers.iter().any(|layer| {
            layer.visible
                && (!matches!(self.presentation, PanelPresentation::Table)
                    || layer.purpose == LayerPurpose::TableBinding)
                && layer
                    .source
                    .indicator_output()
                    .is_some_and(|output| output.instance_id == id)
        })
    }

    fn data_hint(&self) -> PanelDataHint {
        if matches!(self.presentation, PanelPresentation::Table) {
            return PanelDataHint::TableLike;
        }

        if self.layers.iter().any(LayerSpec::is_histogram_like) {
            PanelDataHint::HistogramLike
        } else {
            PanelDataHint::ValueLike
        }
    }

    fn source_count(&self) -> usize {
        self.layers
            .iter()
            .filter_map(LayerSpec::source_id)
            .collect::<FxHashSet<DataSourceId>>()
            .len()
    }

    pub fn uses_multi_source(&self) -> bool {
        self.source_count() > 1
    }

    fn effective_scale_mode(&self) -> PanelScaleMode {
        if self.comparison_policy.force_percent_scale_on_multi_source && self.uses_multi_source() {
            PanelScaleMode::PercentFromBase
        } else {
            self.preferred_scale
        }
    }

    fn set_layer_mark(&mut self, layer_id: LayerId, mark: MarkKind) -> bool {
        let is_multi_source = self.uses_multi_source();
        let force_line = self.comparison_policy.force_line_for_non_base_sources
            && is_multi_source
            && self.base_layer != Some(layer_id);

        let Some(layer) = self.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        layer.mark_override = Some(if force_line { MarkKind::Line } else { mark });
        true
    }

    fn enforce_comparison_mark_policy(&mut self) {
        if !(self.comparison_policy.force_line_for_non_base_sources && self.uses_multi_source()) {
            return;
        }

        for layer in &mut self.layers {
            if Some(layer.id) != self.base_layer {
                layer.mark_override = Some(MarkKind::Line);
            }
        }
    }
}

/// Chart panels and their cumulative height ratios.
///
/// Adding an auxiliary panel takes height from the primary; removing one
/// returns its height to the primary. Surviving auxiliary panels retain
/// their ratios, subject to minimum-height normalization. Reordering
/// carries each panel's ratio with it. Heights are proportional, not pixels.
///
/// New auxiliary panels receive up to 15%, reduced to an 8% minimum when
/// space is limited. The primary must retain at least 8%.
#[derive(Debug, Clone)]
pub struct ChartComposition {
    pub panels: Vec<PanelSpec>,
    /// Normalized boundaries in ascending order, one fewer than panel count.
    pub splits: Vec<f32>,
    next_panel_id: u32,
    next_layer_id: u32,
}

impl ChartComposition {
    pub fn comparison_layer(&self, ticker: TickerInfo) -> Option<&LayerSpec> {
        let panel = self.panel(self.primary_panel_id()?)?;
        panel.layers.iter().find(|layer| {
            matches!(
                layer.source,
                LayerSource::RawKline {
                    source: DataSourceId::Comparison(source)
                } if source == ticker.ticker
            )
        })
    }

    pub fn toggle_comparison_visibility(&mut self, ticker: TickerInfo) -> bool {
        let Some(panel_id) = self.primary_panel_id() else {
            return false;
        };
        let Some(layer) = self.comparison_layer(ticker) else {
            return false;
        };
        let (layer_id, visible) = (layer.id, !layer.visible);
        self.set_panel_layer_visibility(panel_id, layer_id, visible)
    }

    pub fn prototype_kline() -> Self {
        let mut composition = Self {
            panels: Vec::new(),
            splits: Vec::new(),
            next_panel_id: 1,
            next_layer_id: 1,
        };

        let candle_layer = composition.new_layer(
            "Candles",
            LayerSource::RawKline {
                source: DataSourceId::Primary,
            },
            LayerDataKind::Ohlc,
            MarkKind::Candle,
            AxisBinding::Primary,
        );

        let main_panel_id = composition.new_panel_id();
        composition.panels.push(PanelSpec {
            id: main_panel_id,
            role: PanelRole::Primary,
            presentation: PanelPresentation::Plot,
            named_pane: None,
            title: None,
            value_precision: Some(PanelValuePrecision::BaseTickerMinTick),
            value_label_policy: PanelValueLabelPolicy::default(),
            base_layer: Some(candle_layer.id),
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers: vec![candle_layer],
        });

        composition.ensure_split_count();
        composition.splits = composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        composition
    }

    pub fn from_parts(panels: Vec<PanelSpec>, splits: Vec<f32>) -> Self {
        let next_panel_id = panels
            .iter()
            .map(|panel| panel.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let next_layer_id = panels
            .iter()
            .flat_map(|panel| panel.layers.iter())
            .map(|layer| layer.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        let mut composition = Self {
            panels,
            splits,
            next_panel_id,
            next_layer_id,
        };
        composition.ensure_split_count();
        composition.splits = composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        composition
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn primary_panel_id(&self) -> Option<PanelId> {
        self.panels
            .iter()
            .find(|panel| matches!(panel.role, PanelRole::Primary))
            .map(|panel| panel.id)
    }

    pub fn split_count(&self) -> usize {
        self.panels.len().saturating_sub(1)
    }

    pub fn panel(&self, panel_id: PanelId) -> Option<&PanelSpec> {
        self.panels.iter().find(|panel| panel.id == panel_id)
    }

    fn panel_mut(&mut self, panel_id: PanelId) -> Option<&mut PanelSpec> {
        self.panels.iter_mut().find(|panel| panel.id == panel_id)
    }

    pub fn panel_effective_scale_mode(&self, panel_id: PanelId) -> Option<PanelScaleMode> {
        self.panel(panel_id).map(PanelSpec::effective_scale_mode)
    }

    pub fn panel_effective_mark(&self, panel_id: PanelId) -> Option<MarkKind> {
        self.panel(panel_id).map(PanelSpec::effective_mark)
    }

    pub fn resolved_panel_marks(&self, panel_id: PanelId) -> Option<Vec<(LayerId, MarkKind)>> {
        self.panel(panel_id).map(PanelSpec::resolved_marks)
    }

    pub fn panel_value_precision(&self, panel_id: PanelId) -> Option<PanelValuePrecision> {
        self.panel(panel_id).and_then(|panel| panel.value_precision)
    }

    pub fn panel_value_label_policy(&self, panel_id: PanelId) -> Option<PanelValueLabelPolicy> {
        self.panel(panel_id).map(|panel| panel.value_label_policy)
    }

    pub fn normalized_splits(&self, min_panel_ratio: f32) -> Vec<f32> {
        let panel_count = self.panel_count();
        let split_count = panel_count.saturating_sub(1);

        if split_count == 0 {
            return Vec::new();
        }

        let min_ratio = if panel_count == 0 {
            0.0
        } else {
            min_panel_ratio.clamp(0.0, 1.0 / panel_count as f32)
        };

        let mut splits = Vec::with_capacity(split_count);
        for index in 0..split_count {
            let fallback = (index + 1) as f32 / panel_count as f32;
            splits.push(self.splits.get(index).copied().unwrap_or(fallback));
        }

        for index in 0..split_count {
            let remaining_panels_after = panel_count.saturating_sub(index + 1);
            let lower = if index > 0 {
                splits[index - 1] + min_ratio
            } else {
                min_ratio
            };
            let upper = 1.0 - (remaining_panels_after as f32 * min_ratio);

            let (min_bound, max_bound) = if lower <= upper {
                (lower, upper)
            } else {
                (upper, lower)
            };

            splits[index] = splits[index].clamp(min_bound, max_bound);
        }

        splits
    }

    pub fn set_split(&mut self, split_index: usize, split: f32, min_panel_ratio: f32) -> bool {
        if split_index >= self.split_count() {
            return false;
        }

        self.ensure_split_count();
        let mut splits = self.normalized_splits(min_panel_ratio);

        let panel_count = self.panel_count();
        let min_ratio = if panel_count == 0 {
            0.0
        } else {
            min_panel_ratio.clamp(0.0, 1.0 / panel_count as f32)
        };

        let lower = if split_index > 0 {
            splits[split_index - 1] + min_ratio
        } else {
            min_ratio
        };

        let upper = if split_index + 1 < splits.len() {
            splits[split_index + 1] - min_ratio
        } else {
            1.0 - min_ratio
        };

        let (min_bound, max_bound) = if lower <= upper {
            (lower, upper)
        } else {
            (upper, lower)
        };

        let new_value = split.clamp(min_bound, max_bound);
        if let Some(target) = splits.get_mut(split_index) {
            *target = new_value;
        }

        self.splits = splits;
        true
    }

    /// Returns `None` if no primary panel exists or it cannot supply the
    /// minimum auxiliary height while retaining its own minimum height.
    fn add_aux_panel(
        &mut self,
        title: impl Into<String>,
        layers: Vec<LayerSpec>,
        value_precision: Option<PanelValuePrecision>,
        value_label_policy: PanelValueLabelPolicy,
        presentation: PanelPresentation,
    ) -> Option<PanelId> {
        let mut heights = self.normalized_panel_heights(DEFAULT_MIN_PANEL_RATIO);
        let primary_index = self
            .panels
            .iter()
            .position(|panel| panel.role == PanelRole::Primary)?;
        let available_height = heights[primary_index] - DEFAULT_MIN_PANEL_RATIO;
        if available_height + f32::EPSILON < DEFAULT_MIN_PANEL_RATIO {
            return None;
        }
        let preferred_height = self
            .panels
            .last()
            .filter(|panel| panel.role == PanelRole::Auxiliary)
            .and_then(|_| heights.last().copied())
            .unwrap_or(DEFAULT_AUXILIARY_PANEL_RATIO);
        let auxiliary_height = preferred_height.min(available_height.max(DEFAULT_MIN_PANEL_RATIO));
        let panel_id = self.new_panel_id();
        let base_layer = layers.first().map(|layer| layer.id);
        self.panels.push(PanelSpec {
            id: panel_id,
            role: PanelRole::Auxiliary,
            presentation,
            named_pane: None,
            title: Some(title.into()),
            value_precision,
            value_label_policy,
            base_layer,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers,
        });

        heights[primary_index] -= auxiliary_height;
        heights.push(auxiliary_height);
        self.splits = Self::splits_from_heights(&heights);
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        Some(panel_id)
    }

    fn add_table_panel(
        &mut self,
        title: impl Into<String>,
        source: DataSourceId,
    ) -> Option<PanelId> {
        let title = title.into();
        let panel_id = self.add_aux_panel(
            title.clone(),
            Vec::new(),
            None,
            PanelValueLabelPolicy::default(),
            PanelPresentation::Table,
        )?;
        let mut layer = self.new_layer(
            title,
            LayerSource::RawKline { source },
            LayerDataKind::Scalar,
            MarkKind::Line,
            AxisBinding::Secondary,
        );
        layer.purpose = LayerPurpose::TableBinding;

        self.add_layer_to_panel(panel_id, layer);
        Some(panel_id)
    }

    fn add_layer_to_panel(&mut self, panel_id: PanelId, layer: LayerSpec) -> bool {
        self.panel_mut(panel_id)
            .is_some_and(|panel| panel.add_layer(layer))
    }

    fn remove_layer_from_panel(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        self.panel_mut(panel_id)
            .is_some_and(|panel| panel.remove_layer(layer_id))
    }

    fn add_comparison_source_to_panel(
        &mut self,
        panel_id: PanelId,
        source: DataSourceId,
        name: impl Into<String>,
    ) -> Option<LayerId> {
        let data_hint = self.panel(panel_id)?.data_hint();

        let (data_kind, axis) = match data_hint {
            PanelDataHint::ValueLike => (LayerDataKind::Scalar, AxisBinding::Primary),
            PanelDataHint::HistogramLike => (LayerDataKind::Histogram, AxisBinding::Secondary),
            PanelDataHint::TableLike => return None,
        };

        let layer = self.new_layer(
            name,
            LayerSource::RawKline { source },
            data_kind,
            MarkKind::Line,
            axis,
        );

        let layer_id = layer.id;
        self.add_layer_to_panel(panel_id, layer).then_some(layer_id)
    }

    pub fn set_panel_preferred_scale(&mut self, panel_id: PanelId, scale: PanelScaleMode) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.preferred_scale = scale;
        true
    }

    fn set_panel_layer_mark(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        mark: MarkKind,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.set_layer_mark(layer_id, mark)
    }

    fn set_panel_layer_visibility(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        visible: bool,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };
        let Some(layer) = panel.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        if layer.visible == visible {
            return false;
        }

        layer.visible = visible;
        true
    }

    fn clear_panel_layer_mark(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };
        let Some(layer) = panel.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };
        layer.mark_override.take().is_some()
    }

    pub fn remove_panel(&mut self, panel_id: PanelId) -> bool {
        let Some(index) = self.panels.iter().position(|panel| panel.id == panel_id) else {
            return false;
        };

        if matches!(self.panels[index].role, PanelRole::Primary) {
            return false;
        }

        let mut heights = self.normalized_panel_heights(DEFAULT_MIN_PANEL_RATIO);
        let removed_height = heights.remove(index);
        self.panels.remove(index);
        if let Some(primary_index) = self
            .panels
            .iter()
            .position(|panel| panel.role == PanelRole::Primary)
        {
            heights[primary_index] += removed_height;
            self.splits = Self::splits_from_heights(&heights);
        } else {
            self.ensure_split_count();
        }
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    pub fn move_panel(&mut self, from_index: usize, to_index: usize) -> bool {
        let len = self.panels.len();
        if from_index >= len || to_index >= len || from_index == to_index {
            return false;
        }

        self.ensure_split_count();
        let mut heights = self.normalized_panel_heights(DEFAULT_MIN_PANEL_RATIO);

        let panel = self.panels.remove(from_index);
        let panel_height = heights.remove(from_index);

        self.panels.insert(to_index, panel);
        heights.insert(to_index, panel_height);

        self.splits = Self::splits_from_heights(&heights);
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    fn new_layer(
        &mut self,
        name: impl Into<String>,
        source: LayerSource,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
    ) -> LayerSpec {
        LayerSpec {
            id: self.new_layer_id(),
            name: name.into(),
            source,
            data_kind,
            purpose: LayerPurpose::Plot,
            mark,
            mark_override: None,
            axis,
            visible: true,
            style: LayerStyle::default(),
        }
    }

    pub fn sync_comparison_sources(
        &mut self,
        base_ticker: TickerInfo,
        selected_tickers: &[TickerInfo],
    ) {
        self.rebuild_comparison_bindings(selected_tickers, Some(base_ticker));
    }

    pub fn primary_uses_multi_source(&self) -> bool {
        self.primary_panel_id()
            .and_then(|panel_id| self.panel(panel_id))
            .is_some_and(PanelSpec::uses_multi_source)
    }

    pub fn restore_primary_scale_after_comparisons(&mut self) -> bool {
        let Some(panel_id) = self.primary_panel_id() else {
            return false;
        };
        let Some(panel) = self.panel(panel_id) else {
            return false;
        };
        if panel.uses_multi_source() || panel.preferred_scale != PanelScaleMode::PercentFromBase {
            return false;
        }

        self.set_panel_preferred_scale(panel_id, PanelScaleMode::Absolute)
    }

    pub fn selected_indicators(&self) -> Vec<IndicatorInstanceId> {
        let mut indicators = Vec::new();
        for panel in &self.panels {
            for layer in &panel.layers {
                if let Some(value_id) = layer
                    .source
                    .indicator_output()
                    .map(|output| output.instance_id)
                    && !indicators.contains(&value_id)
                {
                    indicators.push(value_id);
                }
            }
        }
        indicators
    }

    pub fn add_indicator_on_source(
        &mut self,
        id: IndicatorInstanceId,
        indicator: IndicatorId,
        source: &crate::chart::kline::IndicatorOutputRef,
    ) -> bool {
        if self.selected_indicators().contains(&id)
            || !matches!(indicator.default_view(), IndicatorPanelRecipe::Plot { .. })
            || !indicator.definition().has_numeric_plot_layers()
        {
            return false;
        }

        let Some(panel_id) = self.indicator_output_panel_id(source) else {
            return false;
        };
        if self
            .panel(panel_id)
            .is_none_or(|panel| panel.presentation != PanelPresentation::Plot)
        {
            return false;
        }

        let IndicatorPanelRecipe::Plot { layers, .. } = indicator.default_view().clone() else {
            return false;
        };
        match indicator.definition().attachment_mode {
            IndicatorAttachmentMode::Overlay => {
                self.add_indicator_layers_to_panel(id, layers, panel_id)
            }
            IndicatorAttachmentMode::Panel => self.add_indicator(id, indicator),
        }
    }

    fn set_panel_mark(&mut self, panel_index: usize, mark: MarkKind) -> bool {
        let Some((panel_id, layer_id)) = self.panel_base_layer_ids(panel_index) else {
            return false;
        };
        self.set_panel_layer_mark(panel_id, layer_id, mark)
    }

    pub fn primary_mark(&self) -> Option<MarkKind> {
        let panel_id = self.primary_panel_id()?;
        self.panel_effective_mark(panel_id)
    }

    pub fn set_primary_mark(&mut self, mark: MarkKind) -> bool {
        let Some(primary_panel_index) = self
            .panels
            .iter()
            .position(|panel| matches!(panel.role, PanelRole::Primary))
        else {
            return false;
        };

        self.set_panel_mark(primary_panel_index, mark)
    }

    pub fn reset_primary_mark(&mut self) -> bool {
        let Some(panel_id) = self.primary_panel_id() else {
            return false;
        };
        let Some(layer_id) = self.panel(panel_id).and_then(|panel| {
            panel
                .base_layer
                .or_else(|| panel.layers.first().map(|layer| layer.id))
        }) else {
            return false;
        };

        self.clear_panel_layer_mark(panel_id, layer_id)
    }

    fn indicator_layer_binding(&self, id: IndicatorInstanceId) -> Option<(PanelId, LayerId)> {
        for panel in &self.panels {
            if let Some(layer) = panel.layers.iter().find(|layer| {
                layer
                    .source
                    .indicator_output()
                    .is_some_and(|output| output.instance_id == id)
            }) {
                return Some((panel.id, layer.id));
            }
        }
        None
    }

    fn indicator_mark_layer_ids(
        &self,
        id: IndicatorInstanceId,
        kind: Option<&IndicatorId>,
        mark: Option<MarkKind>,
    ) -> Option<(PanelId, Vec<LayerId>)> {
        let (panel_id, _) = self.indicator_layer_binding(id)?;
        let panel = self.panel(panel_id)?;
        let output_layers = panel
            .layers
            .iter()
            .filter(|layer| {
                layer
                    .source
                    .indicator_output()
                    .is_some_and(|output| output.instance_id == id)
            })
            .collect::<Vec<_>>();
        if output_layers.is_empty() {
            return None;
        }

        if kind.is_none() {
            return Some((
                panel_id,
                output_layers.iter().map(|layer| layer.id).collect(),
            ));
        }

        let source_channel = mark
            .and_then(|mark| kind.and_then(|kind| kind.mark_source_channel(mark)))
            .or_else(|| {
                kind.and_then(|kind| {
                    output_layers.iter().find_map(|layer| {
                        let channel = layer.source.indicator_output()?.channel_id.as_str();
                        kind.mark_specs()
                            .iter()
                            .any(|spec| spec.source_channel == channel)
                            .then_some(channel)
                    })
                })
            });
        let matching_layers = source_channel.and_then(|source_channel| {
            let matching = output_layers
                .iter()
                .filter(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|output| output.channel_id == source_channel)
                })
                .map(|layer| layer.id)
                .collect::<Vec<_>>();
            (!matching.is_empty()).then_some(matching)
        });
        let layer_ids = matching_layers.unwrap_or_else(|| {
            if output_layers.len() == 1 {
                output_layers.iter().map(|layer| layer.id).collect()
            } else {
                output_layers
                    .first()
                    .map(|layer| vec![layer.id])
                    .unwrap_or_default()
            }
        });
        Some((panel_id, layer_ids))
    }

    pub fn indicator_mark(
        &self,
        id: IndicatorInstanceId,
        kind: Option<&IndicatorId>,
    ) -> Option<MarkKind> {
        let (panel_id, layer_ids) = self.indicator_mark_layer_ids(id, kind, None)?;

        self.resolved_panel_marks(panel_id)?
            .into_iter()
            .find(|(candidate_layer_id, _)| layer_ids.contains(candidate_layer_id))
            .map(|(_, mark)| mark)
    }

    pub fn indicator_mark_target_layer_ids(
        &self,
        id: IndicatorInstanceId,
        kind: &IndicatorId,
        mark: MarkKind,
    ) -> Option<(PanelId, Vec<LayerId>)> {
        self.indicator_mark_layer_ids(id, Some(kind), Some(mark))
    }

    pub fn set_indicator_mark(
        &mut self,
        id: IndicatorInstanceId,
        mark: MarkKind,
        kind: Option<&IndicatorId>,
    ) -> bool {
        let Some((panel_id, layer_ids)) = self.indicator_mark_layer_ids(id, kind, Some(mark))
        else {
            return false;
        };

        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };
        let mut changed = false;
        for layer in &mut panel.layers {
            if layer_ids.contains(&layer.id) && layer.mark_override != Some(mark) {
                layer.mark_override = Some(mark);
                changed = true;
            }
        }
        changed
    }

    pub fn reset_indicator_mark(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, layer_id)) = self.indicator_layer_binding(id) else {
            return false;
        };

        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };
        let has_output_layers = panel.layers.iter().any(|layer| {
            layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
        });
        if !has_output_layers {
            return panel
                .layers
                .iter_mut()
                .find(|layer| layer.id == layer_id)
                .map(|layer| layer.mark_override.take().is_some())
                .unwrap_or(false);
        }

        let mut changed = false;
        for layer in &mut panel.layers {
            if layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
            {
                changed |= layer.mark_override.take().is_some();
            }
        }
        changed
    }

    pub fn toggle_indicator_visibility(&mut self, id: IndicatorInstanceId) -> bool {
        let Some((panel_id, _)) = self.indicator_layer_binding(id) else {
            return false;
        };
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };
        let visible = !panel.indicator_visible(id);
        let mut changed = false;
        for layer in &mut panel.layers {
            if layer
                .source
                .indicator_output()
                .is_some_and(|output| output.instance_id == id)
                && layer.visible != visible
            {
                layer.visible = visible;
                changed = true;
            }
        }
        changed
    }

    pub fn add_indicator(&mut self, id: IndicatorInstanceId, indicator: IndicatorId) -> bool {
        if self.selected_indicators().contains(&id) {
            return false;
        }

        let definition = indicator.definition();
        match definition.panel_recipe.clone() {
            IndicatorPanelRecipe::Plot {
                panel_title,
                value_precision,
                value_label_policy,
                preferred_scale,
                layers: declared_layers,
                ..
            } => {
                if declared_layers.is_empty() {
                    return false;
                }
                let pane = declared_layers
                    .first()
                    .map(|layer| layer.pane)
                    .unwrap_or(PaneTarget::Indicator);
                match pane {
                    PaneTarget::Main => {
                        let Some(panel_id) = self.primary_panel_id() else {
                            return false;
                        };
                        if !self.add_indicator_layers_to_panel(id, declared_layers, panel_id) {
                            return false;
                        }
                    }
                    PaneTarget::Indicator => {
                        let layers = self.new_indicator_layers(id, declared_layers);
                        let Some(panel_id) = self.add_aux_panel(
                            panel_title,
                            layers,
                            Some(value_precision),
                            value_label_policy,
                            PanelPresentation::Plot,
                        ) else {
                            return false;
                        };
                        let _ = self.set_panel_preferred_scale(panel_id, preferred_scale);
                    }
                    PaneTarget::Named(named_pane) => {
                        let panel_id = self
                            .panels
                            .iter()
                            .find(|panel| {
                                panel.role == PanelRole::Auxiliary
                                    && panel.presentation == PanelPresentation::Plot
                                    && panel.named_pane == Some(named_pane)
                            })
                            .map(|panel| panel.id);
                        let panel_id = panel_id.or_else(|| {
                            let panel_id = self.add_aux_panel(
                                panel_title.clone(),
                                Vec::new(),
                                Some(value_precision),
                                value_label_policy,
                                PanelPresentation::Plot,
                            )?;
                            if let Some(panel) = self.panel_mut(panel_id) {
                                panel.named_pane = Some(named_pane);
                                panel.preferred_scale = preferred_scale;
                            }
                            Some(panel_id)
                        });
                        let Some(panel_id) = panel_id else {
                            return false;
                        };
                        if !self.add_indicator_layers_to_panel(id, declared_layers, panel_id) {
                            return false;
                        }
                    }
                }
            }
            IndicatorPanelRecipe::Table {
                panel_title,
                columns,
            } => {
                let Some(channel_id) = columns.first().map(|column| column.value.clone()) else {
                    return false;
                };
                let Some(panel_id) = self.add_table_panel(panel_title, DataSourceId::Primary)
                else {
                    return false;
                };
                if let Some(panel) = self.panel_mut(panel_id)
                    && let Some(layer) = panel.layers.first_mut()
                {
                    layer.source = LayerSource::IndicatorOutput {
                        source: DataSourceId::Primary,
                        output: crate::chart::kline::IndicatorOutputRef::new(id, channel_id),
                    };
                }
            }
            IndicatorPanelRecipe::PrimaryOverlay {
                layers: declared_layers,
                ..
            } => {
                let Some(primary_panel_id) = self.primary_panel_id() else {
                    return false;
                };
                if declared_layers.is_empty() {
                    return false;
                }
                for spec in declared_layers.iter().cloned() {
                    let layer = self.new_layer(
                        spec.layer_name,
                        LayerSource::IndicatorOutput {
                            source: DataSourceId::Primary,
                            output: crate::chart::kline::IndicatorOutputRef::new(
                                id,
                                spec.channel_id,
                            ),
                        },
                        spec.data_kind,
                        spec.mark,
                        spec.axis,
                    );
                    if !self.add_layer_to_panel(primary_panel_id, layer) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn new_indicator_layers(
        &mut self,
        id: IndicatorInstanceId,
        declared_layers: Vec<crate::chart::kline::IndicatorOutputLayerSpec>,
    ) -> Vec<crate::chart::kline::composition::LayerSpec> {
        declared_layers
            .into_iter()
            .map(|spec| {
                self.new_layer(
                    spec.layer_name,
                    LayerSource::IndicatorOutput {
                        source: DataSourceId::Primary,
                        output: crate::chart::kline::IndicatorOutputRef::new(id, spec.channel_id),
                    },
                    spec.data_kind,
                    spec.mark,
                    spec.axis,
                )
            })
            .collect()
    }

    fn add_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        declared_layers: Vec<crate::chart::kline::IndicatorOutputLayerSpec>,
        panel_id: PanelId,
    ) -> bool {
        self.new_indicator_layers(id, declared_layers)
            .into_iter()
            .all(|layer| self.add_layer_to_panel(panel_id, layer))
    }

    pub fn indicator_panel_id(&self, id: IndicatorInstanceId) -> Option<PanelId> {
        self.panels.iter().find_map(|panel| {
            panel
                .layers
                .iter()
                .any(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|output| output.instance_id == id)
                })
                .then_some(panel.id)
        })
    }

    pub fn indicator_output_panel_id(&self, output: &IndicatorOutputRef) -> Option<PanelId> {
        self.panels.iter().find_map(|panel| {
            panel
                .layers
                .iter()
                .any(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .is_some_and(|candidate| candidate == output)
                })
                .then_some(panel.id)
        })
    }

    pub fn reconcile_attached_indicator_placements(
        &mut self,
        instances: &IndicatorInstanceStore,
    ) -> bool {
        let attached_ids = instances
            .instances()
            .iter()
            .filter(|instance| !instance.source_bindings.is_empty())
            .map(|instance| instance.id)
            .collect::<Vec<_>>();
        let mut changed = false;

        for id in attached_ids {
            let Some(instance) = instances.get(id) else {
                continue;
            };
            let overlay = matches!(
                instance.kind.definition().attachment_mode,
                IndicatorAttachmentMode::Overlay
            );
            let mut source_panel = None;
            let mut valid = instance.kind.definition().has_numeric_plot_layers()
                && matches!(
                    instance.kind.default_view(),
                    IndicatorPanelRecipe::Plot { .. }
                );
            for (input_id, source) in &instance.source_bindings {
                if !instance
                    .kind
                    .definition()
                    .source_inputs
                    .iter()
                    .any(|input| input.id == *input_id && input.allow_indicator)
                    || source.instance_id == id
                    || instances
                        .get(source.instance_id)
                        .is_none_or(|source_instance| {
                            source_instance
                                .kind
                                .definition()
                                .sourceable_numeric_output(&source.channel_id)
                                .is_none()
                        })
                {
                    valid = false;
                    break;
                }

                let Some(panel_id) = self.indicator_output_panel_id(source) else {
                    valid = false;
                    break;
                };
                if self
                    .panel(panel_id)
                    .is_none_or(|panel| panel.presentation != PanelPresentation::Plot)
                    || (overlay && source_panel.is_some_and(|target| target != panel_id))
                {
                    valid = false;
                    break;
                }
                source_panel = Some(panel_id);
            }

            if !valid {
                continue;
            }
            let Some(target_panel) = (if overlay {
                source_panel
            } else {
                self.standalone_indicator_panel(id, &instance.kind, instances)
            }) else {
                continue;
            };
            changed |= self.ensure_indicator_layers_to_panel(id, &instance.kind, target_panel);
        }

        changed
    }

    fn standalone_indicator_panel(
        &mut self,
        id: IndicatorInstanceId,
        indicator: &IndicatorId,
        instances: &IndicatorInstanceStore,
    ) -> Option<PanelId> {
        let IndicatorPanelRecipe::Plot {
            panel_title,
            value_precision,
            value_label_policy,
            preferred_scale,
            layers,
            ..
        } = indicator.default_view().clone()
        else {
            return None;
        };
        let pane = layers.first()?.pane;

        match pane {
            PaneTarget::Main => self.primary_panel_id(),
            PaneTarget::Named(named_pane) => self
                .panels
                .iter()
                .find(|panel| {
                    panel.role == PanelRole::Auxiliary
                        && panel.presentation == PanelPresentation::Plot
                        && panel.named_pane == Some(named_pane)
                })
                .map(|panel| panel.id)
                .or_else(|| {
                    let panel_id = self.add_aux_panel(
                        panel_title,
                        Vec::new(),
                        Some(value_precision),
                        value_label_policy,
                        PanelPresentation::Plot,
                    )?;
                    if let Some(panel) = self.panel_mut(panel_id) {
                        panel.named_pane = Some(named_pane);
                        panel.preferred_scale = preferred_scale;
                    }
                    Some(panel_id)
                }),
            PaneTarget::Indicator => {
                let source_instances = instances
                    .get(id)
                    .into_iter()
                    .flat_map(|instance| instance.source_bindings.values())
                    .map(|source| source.instance_id)
                    .collect::<FxHashSet<_>>();
                self.panels
                    .iter()
                    .find(|panel| {
                        panel.role == PanelRole::Auxiliary
                            && panel.presentation == PanelPresentation::Plot
                            && panel.layers.iter().any(|layer| {
                                layer
                                    .source
                                    .indicator_output()
                                    .is_some_and(|output| output.instance_id == id)
                            })
                            && panel
                                .layers
                                .iter()
                                .filter_map(|layer| layer.source.indicator_output())
                                .all(|output| {
                                    output.instance_id == id
                                        || !source_instances.contains(&output.instance_id)
                                })
                    })
                    .map(|panel| panel.id)
                    .or_else(|| {
                        let panel_id = self.add_aux_panel(
                            panel_title,
                            Vec::new(),
                            Some(value_precision),
                            value_label_policy,
                            PanelPresentation::Plot,
                        )?;
                        let _ = self.set_panel_preferred_scale(panel_id, preferred_scale);
                        Some(panel_id)
                    })
            }
        }
    }

    fn ensure_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        indicator: &IndicatorId,
        target_panel: PanelId,
    ) -> bool {
        let IndicatorPanelRecipe::Plot { layers, .. } = indicator.default_view().clone() else {
            return false;
        };
        let mut existing_channels = self
            .panels
            .iter()
            .flat_map(|panel| panel.layers.iter())
            .filter_map(|layer| layer.source.indicator_output())
            .filter(|output| output.instance_id == id)
            .map(|output| output.channel_id.clone())
            .collect::<FxHashSet<_>>();
        let mut changed = false;

        for spec in layers {
            if !existing_channels.insert(spec.channel_id.clone()) {
                continue;
            }
            let layer = self.new_layer(
                spec.layer_name,
                LayerSource::IndicatorOutput {
                    source: DataSourceId::Primary,
                    output: crate::chart::kline::IndicatorOutputRef::new(id, spec.channel_id),
                },
                spec.data_kind,
                spec.mark,
                spec.axis,
            );
            changed |= self.add_layer_to_panel(target_panel, layer);
        }

        changed | self.move_indicator_layers_to_panel(id, target_panel)
    }

    fn move_indicator_layers_to_panel(
        &mut self,
        id: IndicatorInstanceId,
        target_panel: PanelId,
    ) -> bool {
        let locations = self
            .panels
            .iter()
            .flat_map(|panel| {
                panel.layers.iter().filter_map(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .filter(|output| output.instance_id == id)
                        .map(|_| (panel.id, layer.id))
                })
            })
            .collect::<Vec<_>>();
        if locations.is_empty()
            || locations
                .iter()
                .all(|(panel_id, _)| *panel_id == target_panel)
        {
            return false;
        }

        let layers = locations
            .iter()
            .filter_map(|(panel_id, layer_id)| {
                self.panel(*panel_id)
                    .and_then(|panel| panel.layers.iter().find(|layer| layer.id == *layer_id))
                    .cloned()
            })
            .collect::<Vec<_>>();
        for (panel_id, layer_id) in &locations {
            let _ = self.remove_layer_from_panel(*panel_id, *layer_id);
        }
        for layer in layers {
            let _ = self.add_layer_to_panel(target_panel, layer);
        }

        let touched_panels = locations
            .iter()
            .map(|(panel_id, _)| *panel_id)
            .collect::<FxHashSet<_>>();
        for panel_id in touched_panels {
            if self
                .panel(panel_id)
                .is_some_and(|panel| panel.role != PanelRole::Primary && panel.layers.is_empty())
            {
                let _ = self.remove_panel(panel_id);
            }
        }
        true
    }

    pub fn remove_indicator(&mut self, id: IndicatorInstanceId) -> bool {
        let layer_ids = self
            .panels
            .iter()
            .flat_map(|panel| {
                panel.layers.iter().filter_map(|layer| {
                    layer
                        .source
                        .indicator_output()
                        .filter(|output| output.instance_id == id)
                        .map(|_| (panel.id, layer.id))
                })
            })
            .collect::<Vec<_>>();
        if layer_ids.is_empty() {
            return false;
        }
        let touched_panels = layer_ids
            .iter()
            .map(|(panel_id, _)| *panel_id)
            .collect::<FxHashSet<_>>();
        let mut removed = false;
        for (panel_id, layer_id) in layer_ids {
            removed |= self.remove_layer_from_panel(panel_id, layer_id);
        }
        let empty_panels = self
            .panels
            .iter()
            .filter(|panel| {
                touched_panels.contains(&panel.id)
                    && panel.role != PanelRole::Primary
                    && panel.layers.is_empty()
            })
            .map(|panel| panel.id)
            .collect::<Vec<_>>();
        for panel_id in empty_panels {
            removed |= self.remove_panel(panel_id);
        }
        removed
    }

    fn rebuild_comparison_bindings(
        &mut self,
        selected_tickers: &[TickerInfo],
        base_ticker: Option<TickerInfo>,
    ) {
        let Some(primary_panel_id) = self.primary_panel_id() else {
            return;
        };
        let Some(primary_panel) = self.panel(primary_panel_id) else {
            return;
        };

        let stale_layer_ids: Vec<LayerId> = primary_panel
            .layers
            .iter()
            .filter_map(|layer| {
                let LayerSource::RawKline {
                    source: DataSourceId::Comparison(source),
                } = layer.source
                else {
                    return None;
                };
                let is_selected_comparison = base_ticker.is_some_and(|base| {
                    selected_tickers
                        .iter()
                        .any(|ticker| *ticker != base && ticker.ticker == source)
                });
                (!is_selected_comparison).then_some(layer.id)
            })
            .collect();

        for layer_id in stale_layer_ids {
            let _ = self.remove_layer_from_panel(primary_panel_id, layer_id);
        }

        let Some(base_ticker) = base_ticker else {
            return;
        };

        for &ticker in selected_tickers
            .iter()
            .filter(|ticker| **ticker != base_ticker)
        {
            if self.comparison_layer(ticker).is_some() {
                continue;
            }
            let name = ticker.ticker.symbol_and_exchange_string();
            let _ = self.add_comparison_source_to_panel(
                primary_panel_id,
                DataSourceId::Comparison(ticker.ticker),
                name,
            );
        }
    }

    fn panel_base_layer_ids(&self, panel_index: usize) -> Option<(PanelId, LayerId)> {
        let panel = self.panels.get(panel_index)?;
        let layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id))?;
        Some((panel.id, layer_id))
    }

    fn ensure_split_count(&mut self) {
        let target = self.split_count();
        match self.splits.len().cmp(&target) {
            Ordering::Equal => {}
            Ordering::Greater => self.splits.truncate(target),
            Ordering::Less => {
                for index in self.splits.len()..target {
                    let fallback = (index + 1) as f32 / (target + 1) as f32;
                    self.splits.push(fallback);
                }
            }
        }
    }

    fn normalized_panel_heights(&self, min_panel_ratio: f32) -> Vec<f32> {
        let panel_count = self.panel_count();
        if panel_count == 0 {
            return Vec::new();
        }

        let mut heights = Vec::with_capacity(panel_count);
        let mut previous = 0.0;

        for split in self.normalized_splits(min_panel_ratio) {
            let clamped = split.clamp(previous, 1.0);
            heights.push((clamped - previous).max(0.0));
            previous = clamped;
        }

        heights.push((1.0 - previous).max(0.0));

        let total: f32 = heights.iter().sum();
        if total > f32::EPSILON {
            for height in &mut heights {
                *height /= total;
            }
        }

        heights
    }

    fn splits_from_heights(heights: &[f32]) -> Vec<f32> {
        if heights.len() <= 1 {
            return Vec::new();
        }

        let total: f32 = heights.iter().copied().sum();
        if total <= f32::EPSILON {
            let count = heights.len();
            return (1..count)
                .map(|index| index as f32 / count as f32)
                .collect();
        }

        let mut splits = Vec::with_capacity(heights.len().saturating_sub(1));
        let mut acc = 0.0;
        for height in heights.iter().take(heights.len().saturating_sub(1)) {
            acc += *height / total;
            splits.push(acc.clamp(0.0, 1.0));
        }

        splits
    }

    fn new_panel_id(&mut self) -> PanelId {
        let id = PanelId(self.next_panel_id);
        self.next_panel_id = self.next_panel_id.wrapping_add(1);
        id
    }

    fn new_layer_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id = self.next_layer_id.wrapping_add(1);
        id
    }
}
