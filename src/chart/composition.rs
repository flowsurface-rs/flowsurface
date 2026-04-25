#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::BTreeSet;

pub const DEFAULT_MIN_PANEL_RATIO: f32 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PanelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRole {
    Primary,
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Candle,
    Bar,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerDataKind {
    Ohlc,
    Scalar,
    Histogram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisBinding {
    Primary,
    Secondary,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DataSourceId {
    Primary,
    Symbol(&'static str),
    Synthetic(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelScaleMode {
    Absolute,
    FitVisible,
    FitVisibleIncludeZero,
    Logarithmic,
    PercentFromBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    BuiltInIndicator {
        name: &'static str,
        source: DataSourceId,
    },
    DslOutput {
        script_id: &'static str,
        output: &'static str,
        source: Option<DataSourceId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDataHint {
    ValueLike,
    HistogramLike,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct LayerSpec {
    pub id: LayerId,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
    pub mark: MarkKind,
    pub axis: AxisBinding,
    pub visible: bool,
    pub style: LayerStyle,
}

impl LayerSpec {
    pub fn is_histogram_like(&self) -> bool {
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
            Self::BuiltInIndicator { source, .. } => Some(*source),
            Self::DslOutput { source, .. } => *source,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PanelSpec {
    pub id: PanelId,
    pub role: PanelRole,
    pub title: String,
    pub base_layer: Option<LayerId>,
    pub preferred_scale: PanelScaleMode,
    pub comparison_policy: PanelComparisonPolicy,
    pub layers: Vec<LayerSpec>,
}

impl PanelSpec {
    pub fn data_hint(&self) -> PanelDataHint {
        if self.layers.iter().any(LayerSpec::is_histogram_like) {
            PanelDataHint::HistogramLike
        } else {
            PanelDataHint::ValueLike
        }
    }

    pub fn source_count(&self) -> usize {
        self.layers
            .iter()
            .filter_map(LayerSpec::source_id)
            .collect::<BTreeSet<DataSourceId>>()
            .len()
    }

    pub fn uses_multi_source(&self) -> bool {
        self.source_count() > 1
    }

    pub fn effective_scale_mode(&self) -> PanelScaleMode {
        if self.comparison_policy.force_percent_scale_on_multi_source && self.uses_multi_source() {
            PanelScaleMode::PercentFromBase
        } else {
            self.preferred_scale
        }
    }

    pub fn set_base_layer(&mut self, layer_id: LayerId) -> bool {
        if self.layers.iter().any(|layer| layer.id == layer_id) {
            self.base_layer = Some(layer_id);
            self.enforce_comparison_mark_policy();
            true
        } else {
            false
        }
    }

    pub fn set_layer_mark(&mut self, layer_id: LayerId, mark: MarkKind) -> bool {
        let is_multi_source = self.uses_multi_source();
        let force_line = self.comparison_policy.force_line_for_non_base_sources
            && is_multi_source
            && self.base_layer != Some(layer_id);

        let Some(layer) = self.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        layer.mark = if force_line { MarkKind::Line } else { mark };
        true
    }

    pub fn enforce_comparison_mark_policy(&mut self) {
        if !(self.comparison_policy.force_line_for_non_base_sources && self.uses_multi_source()) {
            return;
        }

        for layer in &mut self.layers {
            if Some(layer.id) != self.base_layer {
                layer.mark = MarkKind::Line;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChartComposition {
    pub panels: Vec<PanelSpec>,
    /// Normalized boundaries in ascending order, one fewer than panel count.
    pub splits: Vec<f32>,
    next_panel_id: u32,
    next_layer_id: u32,
}

impl ChartComposition {
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
            title: "Value".to_string(),
            base_layer: Some(candle_layer.id),
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers: vec![candle_layer],
        });

        composition.ensure_split_count();
        composition.splits = composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        composition
    }

    pub fn prototype_kline_comparison() -> Self {
        let mut composition = Self::prototype_kline();

        if let Some(primary_panel) = composition.primary_panel_id() {
            let _ = composition.add_comparison_source_to_panel(
                primary_panel,
                DataSourceId::Symbol("CMP-01"),
                "Compare #1",
            );
        }

        composition
    }

    pub fn prototype_kline_log_scale() -> Self {
        let mut composition = Self::prototype_kline();

        if let Some(primary_panel) = composition.primary_panel_id() {
            let _ =
                composition.set_panel_preferred_scale(primary_panel, PanelScaleMode::Logarithmic);
        }

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

    pub fn panel_data_hints(&self) -> Vec<PanelDataHint> {
        self.panels.iter().map(PanelSpec::data_hint).collect()
    }

    pub fn panel(&self, panel_id: PanelId) -> Option<&PanelSpec> {
        self.panels.iter().find(|panel| panel.id == panel_id)
    }

    pub fn panel_mut(&mut self, panel_id: PanelId) -> Option<&mut PanelSpec> {
        self.panels.iter_mut().find(|panel| panel.id == panel_id)
    }

    pub fn panel_effective_scale_mode(&self, panel_id: PanelId) -> Option<PanelScaleMode> {
        self.panel(panel_id).map(PanelSpec::effective_scale_mode)
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

    pub fn add_aux_panel(&mut self, title: impl Into<String>, layers: Vec<LayerSpec>) -> PanelId {
        let panel_id = self.new_panel_id();
        let base_layer = layers.first().map(|layer| layer.id);
        self.panels.push(PanelSpec {
            id: panel_id,
            role: PanelRole::Auxiliary,
            title: title.into(),
            base_layer,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers,
        });

        self.ensure_split_count();
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        panel_id
    }

    pub fn add_layer_to_panel(&mut self, panel_id: PanelId, layer: LayerSpec) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let layer_id = layer.id;
        panel.layers.push(layer);
        if panel.base_layer.is_none() {
            panel.base_layer = Some(layer_id);
        }
        panel.enforce_comparison_mark_policy();

        true
    }

    pub fn add_comparison_source_to_panel(
        &mut self,
        panel_id: PanelId,
        source: DataSourceId,
        name: impl Into<String>,
    ) -> Option<LayerId> {
        let data_hint = self.panel(panel_id)?.data_hint();

        let (data_kind, axis) = match data_hint {
            PanelDataHint::ValueLike => (LayerDataKind::Scalar, AxisBinding::Primary),
            PanelDataHint::HistogramLike => (LayerDataKind::Histogram, AxisBinding::Secondary),
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

    pub fn set_panel_base_layer(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.set_base_layer(layer_id)
    }

    pub fn set_panel_preferred_scale(&mut self, panel_id: PanelId, scale: PanelScaleMode) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.preferred_scale = scale;
        true
    }

    pub fn set_panel_comparison_policy(
        &mut self,
        panel_id: PanelId,
        comparison_policy: PanelComparisonPolicy,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.comparison_policy = comparison_policy;
        panel.enforce_comparison_mark_policy();
        true
    }

    pub fn set_panel_layer_mark(
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

    pub fn remove_layer_from_panel(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let Some(index) = panel.layers.iter().position(|layer| layer.id == layer_id) else {
            return false;
        };

        panel.layers.remove(index);

        if panel.base_layer == Some(layer_id) {
            panel.base_layer = panel.layers.first().map(|layer| layer.id);
        }

        panel.enforce_comparison_mark_policy();
        true
    }

    pub fn resolved_panel_marks(&self, panel_id: PanelId) -> Option<Vec<(LayerId, MarkKind)>> {
        let panel = self.panel(panel_id)?;
        let is_multi_source = panel.uses_multi_source();

        Some(
            panel
                .layers
                .iter()
                .map(|layer| {
                    let mark = if panel.comparison_policy.force_line_for_non_base_sources
                        && is_multi_source
                        && panel.base_layer != Some(layer.id)
                    {
                        MarkKind::Line
                    } else {
                        layer.mark
                    };

                    (layer.id, mark)
                })
                .collect(),
        )
    }

    pub fn remove_panel(&mut self, panel_id: PanelId) -> bool {
        let Some(index) = self.panels.iter().position(|panel| panel.id == panel_id) else {
            return false;
        };

        if matches!(self.panels[index].role, PanelRole::Primary) {
            return false;
        }

        self.panels.remove(index);
        self.ensure_split_count();
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    pub fn move_panel(&mut self, from_index: usize, to_index: usize) -> bool {
        let len = self.panels.len();
        if from_index >= len || to_index >= len || from_index == to_index {
            return false;
        }

        let panel = self.panels.remove(from_index);
        self.panels.insert(to_index, panel);
        self.ensure_split_count();
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    pub fn new_layer(
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
            mark,
            axis,
            visible: true,
            style: LayerStyle::default(),
        }
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

pub trait LayerRenderer<Frame> {
    fn supports(&self, mark: MarkKind, data_kind: LayerDataKind) -> bool;
    fn draw_layer(&self, frame: &mut Frame, layer: &LayerSpec);
}

pub trait IndicatorKernel<Input, Output> {
    fn name(&self) -> &'static str;
    fn evaluate(&self, input: Input) -> Output;
}
