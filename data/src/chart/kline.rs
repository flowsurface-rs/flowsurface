pub mod bar;
pub mod chart_data;
pub mod composition;
pub mod drawing;
pub mod fetch_plan;
pub mod footprint;
pub mod indicators;
pub mod poc;
pub mod series;
pub mod studies;
pub mod viewport;

use self::footprint::{ClusterKind, ClusterScaling};
use crate::chart::Basis;
pub use crate::config::theme::ThemeAccent;
use crate::config::theme::ThemeColor;
use crate::config::theme::ThemeColorRole;
use crate::config::theme::ThemeColorShade;

pub use chart_data::{HistoricalInputUpdate, KlineData, KlineDataChange, LiveIndicatorUpdate};
use composition::{
    LayerSource as DomainLayerSource, LayerSpec, PanelSpec,
    PanelValuePrecision as DomainPanelValuePrecision,
};
pub use fetch_plan::{FetchBatch, FetchPlanInput, FetchRange, FetchStage, visible_window};
pub use indicators::{
    BaseSeriesSource, CatalogInstallError, IndicatorAuthor, IndicatorCatalog, IndicatorColorSpec,
    IndicatorConfigError, IndicatorDefinition, IndicatorDefinitionError, IndicatorFillGeometry,
    IndicatorFillSpec, IndicatorHistoryRequirement, IndicatorHost, IndicatorInputAvailability,
    IndicatorInputCoverage, IndicatorInputCoverageState, IndicatorInputRequirement,
    IndicatorInputSource, IndicatorMarkSpec, IndicatorOutputLayerSpec, IndicatorOutputRef,
    IndicatorOverlayChannelSpec, IndicatorOverlaySource, IndicatorPanelRecipe,
    IndicatorReadoutScope, IndicatorReadoutSpec, IndicatorSegmentSpec, IndicatorSetting,
    IndicatorSettingChoice, IndicatorSettingControl, IndicatorSettingValue,
    IndicatorSettingsContext, IndicatorSourceInput, IndicatorUnsupportedReason, OpenInterestSeries,
    OutputChannelSpec, OutputValue, OutputValueType, TableColumnColor, TableColumnFormat,
    TableColumnSpec, TableColumnValue, availability, declared_input_requirements,
    indicator_definition, table_columns, unsupported_reason,
};
pub use indicators::{IndicatorConfig, IndicatorId};
pub use series::{KlineIndexChange, KlineSeriesPoint, KlineSeriesPoints, KlineUpdate, SeriesColor};
pub use studies::tpo::{
    TPO_TRADE_COVERAGE_TIMEFRAME, TpoAnchor, TpoBoundaryEdits, TpoCalculationError, TpoProfile,
    TpoProfileSettings, TpoProfileStatus, TpoRow, TradeTpoCache,
    build_tpo_profile_with_source_timeframe, build_tpo_profiles_with_source_timeframe,
};
pub use studies::volume_profile::{
    AnchoredProfileDistribution, ProfileDistribution, ProfileLevel, RecurringKlineProfileIndex,
    kline_volume_profile, recurring_kline_volume_profiles,
};
pub use studies::{
    BigTradesConfig, DEFAULT_BIG_TRADES_THRESHOLD, DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL,
    DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT, DEFAULT_TPO_LETTER_PERIOD,
    DEFAULT_TPO_VALUE_AREA_PERCENT, DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT,
    DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT, KlineStudy, KlineStudyId, MAX_BIG_TRADES_THRESHOLD,
    MAX_TPO_VALUE_AREA_PERCENT, MAX_VOLUME_PROFILE_OPACITY_PERCENT,
    MAX_VOLUME_PROFILE_WIDTH_PERCENT, MIN_BIG_TRADES_THRESHOLD, MIN_TPO_VALUE_AREA_PERCENT,
    MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT, SourceConstraint,
    StudyDataSource, TpoColorConfig, VolumeProfileColorConfig, VolumeProfileColorSlot,
    VolumeProfileColorSpec, VolumeProfileSchedule, big_trades_step_count,
    big_trades_step_for_threshold, big_trades_threshold_at_step, big_trades_threshold_label, tpo,
    volume_profile,
};
pub use viewport::{
    ChartCoord, ChartStepMs, DEFAULT_HORIZONTAL_SPACING_PX, HorizontalWorld,
    MAX_HORIZONTAL_SPACING_PX, MIN_HORIZONTAL_SPACING_PX, PanelYValueRange, WorldAxis, YUnit,
    YUnitRange, ZOOM_SENSITIVITY,
};

use rustc_hash::FxHashSet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_CANDLESTICK_OPACITY_PERCENT: u8 = 5;
pub const MAX_CANDLESTICK_OPACITY_PERCENT: u8 = 100;
pub const MIN_LINE_WIDTH: u8 = 1;
pub const MAX_LINE_WIDTH: u8 = 4;

const DEFAULT_FOOTPRINT_IMBALANCE_THRESHOLD_PERCENT: usize = 200;
const DEFAULT_FOOTPRINT_IMBALANCE_IGNORE_ZEROS: bool = true;
const DEFAULT_FOOTPRINT_IMBALANCE_COLOR: ThemeColor =
    ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong);
const DEFAULT_CANDLESTICK_OPACITY_PERCENT: u8 = 100;

const DEFAULT_INDICATOR_INSTANCE_ID_BASE: u64 = 1 << 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootprintPocColorSlot {
    Poc,
    Npoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FootprintPocColorSpec {
    pub label: &'static str,
    pub slot: FootprintPocColorSlot,
}

const FOOTPRINT_POC_COLOR_SPECS: [FootprintPocColorSpec; 2] = [
    FootprintPocColorSpec {
        label: "POC",
        slot: FootprintPocColorSlot::Poc,
    },
    FootprintPocColorSpec {
        label: "NPOC",
        slot: FootprintPocColorSlot::Npoc,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintPocColorConfig {
    pub poc: ThemeColor,
    pub npoc: ThemeColor,
}

impl Default for FootprintPocColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl FootprintPocColorConfig {
    pub const fn defaults() -> Self {
        Self {
            poc: ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Strong),
            npoc: ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Strong),
        }
    }

    pub const fn color_specs() -> &'static [FootprintPocColorSpec] {
        &FOOTPRINT_POC_COLOR_SPECS
    }

    pub const fn color_for(self, slot: FootprintPocColorSlot) -> ThemeColor {
        match slot {
            FootprintPocColorSlot::Poc => self.poc,
            FootprintPocColorSlot::Npoc => self.npoc,
        }
    }

    pub const fn with_color(self, slot: FootprintPocColorSlot, color: ThemeColor) -> Self {
        match slot {
            FootprintPocColorSlot::Poc => Self { poc: color, ..self },
            FootprintPocColorSlot::Npoc => Self {
                npoc: color,
                ..self
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintPocConfig {
    pub show_poc: bool,
    pub show_npoc: bool,
    #[serde(default)]
    pub colors: FootprintPocColorConfig,
}

impl FootprintPocConfig {
    pub const fn with_show_poc(self, show_poc: bool) -> Self {
        Self { show_poc, ..self }
    }

    pub const fn with_show_npoc(self, show_npoc: bool) -> Self {
        Self { show_npoc, ..self }
    }

    pub const fn with_colors(self, colors: FootprintPocColorConfig) -> Self {
        Self { colors, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FootprintImbalanceConfig {
    pub show: bool,
    pub threshold_percent: usize,
    pub ignore_zeros: bool,
    pub color: ThemeColor,
}

impl Default for FootprintImbalanceConfig {
    fn default() -> Self {
        Self {
            show: false,
            threshold_percent: DEFAULT_FOOTPRINT_IMBALANCE_THRESHOLD_PERCENT,
            ignore_zeros: DEFAULT_FOOTPRINT_IMBALANCE_IGNORE_ZEROS,
            color: DEFAULT_FOOTPRINT_IMBALANCE_COLOR,
        }
    }
}

impl FootprintImbalanceConfig {
    pub const fn with_show(self, show: bool) -> Self {
        Self { show, ..self }
    }

    pub const fn with_threshold_percent(self, threshold_percent: usize) -> Self {
        Self {
            threshold_percent,
            ..self
        }
    }

    pub const fn with_ignore_zeros(self, ignore_zeros: bool) -> Self {
        Self {
            ignore_zeros,
            ..self
        }
    }

    pub const fn with_color(self, color: ThemeColor) -> Self {
        Self { color, ..self }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FootprintConfig {
    pub visible: bool,
    pub cluster_kind: ClusterKind,
    pub cluster_scaling: ClusterScaling,
    pub imbalance: FootprintImbalanceConfig,
    pub poc: FootprintPocConfig,
}

impl FootprintConfig {
    pub const fn with_visible(self, visible: bool) -> Self {
        Self { visible, ..self }
    }

    pub const fn with_cluster_kind(self, cluster_kind: ClusterKind) -> Self {
        Self {
            cluster_kind,
            ..self
        }
    }

    pub const fn with_cluster_scaling(self, cluster_scaling: ClusterScaling) -> Self {
        Self {
            cluster_scaling,
            ..self
        }
    }

    pub const fn with_imbalance(self, imbalance: FootprintImbalanceConfig) -> Self {
        Self { imbalance, ..self }
    }

    pub const fn with_poc(self, poc: FootprintPocConfig) -> Self {
        Self { poc, ..self }
    }

    pub fn with_default_options(self) -> Self {
        Self {
            visible: self.visible,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndicatorInstanceId(u64);

impl IndicatorInstanceId {
    fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn default_for(kind: &IndicatorId) -> Self {
        Self(DEFAULT_INDICATOR_INSTANCE_ID_BASE + kind.default_instance_slot())
    }
}

impl Serialize for IndicatorInstanceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for IndicatorInstanceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        u64::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorUpdateScope {
    Full,
    From(usize),
}

impl IndicatorUpdateScope {
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::From(first), Self::From(second)) => Self::From(first.min(second)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandlestickColorSlot {
    Bullish,
    Bearish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandlestickColorConfig {
    pub bullish: ThemeColor,
    pub bearish: ThemeColor,
}

impl Default for CandlestickColorConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl CandlestickColorConfig {
    pub const fn defaults() -> Self {
        Self {
            bullish: ThemeColor::new(ThemeColorRole::Success, ThemeColorShade::Base),
            bearish: ThemeColor::new(ThemeColorRole::Danger, ThemeColorShade::Base),
        }
    }

    pub const fn color_for(self, slot: CandlestickColorSlot) -> ThemeColor {
        match slot {
            CandlestickColorSlot::Bullish => self.bullish,
            CandlestickColorSlot::Bearish => self.bearish,
        }
    }

    pub const fn with_color(self, slot: CandlestickColorSlot, color: ThemeColor) -> Self {
        match slot {
            CandlestickColorSlot::Bullish => Self {
                bullish: color,
                ..self
            },
            CandlestickColorSlot::Bearish => Self {
                bearish: color,
                ..self
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CandlestickConfig {
    pub visible: bool,
    pub colors: CandlestickColorConfig,
    pub opacity_percent: u8,
}

impl Default for CandlestickConfig {
    fn default() -> Self {
        Self {
            visible: true,
            colors: CandlestickColorConfig::defaults(),
            opacity_percent: DEFAULT_CANDLESTICK_OPACITY_PERCENT,
        }
    }
}

impl CandlestickConfig {
    pub const fn with_visible(self, visible: bool) -> Self {
        Self { visible, ..self }
    }

    pub const fn with_color(self, slot: CandlestickColorSlot, color: ThemeColor) -> Self {
        Self {
            colors: self.colors.with_color(slot, color),
            ..self
        }
    }

    pub const fn with_opacity_percent(self, opacity_percent: u8) -> Self {
        Self {
            opacity_percent,
            ..self
        }
    }

    pub fn with_default_style(self) -> Self {
        Self {
            visible: self.visible,
            ..Self::default()
        }
    }

    pub fn opacity_ratio(self) -> f32 {
        f32::from(self.opacity_percent.clamp(
            MIN_CANDLESTICK_OPACITY_PERCENT,
            MAX_CANDLESTICK_OPACITY_PERCENT,
        )) / f32::from(MAX_CANDLESTICK_OPACITY_PERCENT)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineStrokeStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl LineStrokeStyle {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Dashed, Self::Dotted];
}

impl std::fmt::Display for LineStrokeStyle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Solid => "Solid",
            Self::Dashed => "Dashed",
            Self::Dotted => "Dotted",
        })
    }
}

/// Definition-local style overrides keyed by stable declaration IDs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndicatorStyleOverrides {
    pub fill_visible: bool,
    #[serde(default)]
    colors: BTreeMap<String, ThemeColor>,
    #[serde(default)]
    line_widths: BTreeMap<String, u8>,
    #[serde(default)]
    stroke_styles: BTreeMap<String, LineStrokeStyle>,
}

impl Default for IndicatorStyleOverrides {
    fn default() -> Self {
        Self {
            fill_visible: true,
            colors: BTreeMap::new(),
            line_widths: BTreeMap::new(),
            stroke_styles: BTreeMap::new(),
        }
    }
}

impl IndicatorStyleOverrides {
    pub fn color(&self, id: &str) -> Option<ThemeColor> {
        self.colors.get(id).copied()
    }

    pub fn set_color(&mut self, id: impl Into<String>, color: ThemeColor) -> bool {
        let id = id.into();
        if id.trim().is_empty() {
            return false;
        }
        if self.color(&id) == Some(color) {
            return false;
        }
        self.colors.insert(id, color);
        true
    }

    pub fn line_width(&self, id: &str) -> Option<u8> {
        self.line_widths.get(id).copied()
    }

    pub fn stroke_style(&self, id: &str) -> Option<LineStrokeStyle> {
        self.stroke_styles.get(id).copied()
    }

    pub fn set_line_width(&mut self, id: impl Into<String>, width: u8) -> bool {
        let id = id.into();
        if id.trim().is_empty() {
            return false;
        }
        let width = width.clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH);
        if self.line_width(&id) == Some(width) {
            return false;
        }
        self.line_widths.insert(id, width);
        true
    }

    pub fn set_stroke_style(&mut self, id: impl Into<String>, style: LineStrokeStyle) -> bool {
        let id = id.into();
        if id.trim().is_empty() {
            return false;
        }
        if self.stroke_style(&id) == Some(style) {
            return false;
        }
        self.stroke_styles.insert(id, style);
        true
    }

    pub fn normalized(mut self) -> Self {
        for width in self.line_widths.values_mut() {
            *width = (*width).clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH);
        }
        self.line_widths.retain(|id, _| !id.trim().is_empty());
        self.stroke_styles.retain(|id, _| !id.trim().is_empty());
        self.colors.retain(|id, _| !id.trim().is_empty());
        self
    }

    fn normalized_for_slots(self, slots: &[indicators::IndicatorColorSpec]) -> Self {
        let mut normalized = self.normalized();
        normalized
            .colors
            .retain(|id, _| slots.iter().any(|slot| slot.id == *id));
        normalized
            .line_widths
            .retain(|id, _| slots.iter().any(|slot| slot.id == *id));
        normalized
            .stroke_styles
            .retain(|id, _| slots.iter().any(|slot| slot.id == *id));
        normalized
    }

    pub fn clear(&mut self) -> bool {
        let changed = !self.fill_visible
            || !self.colors.is_empty()
            || !self.line_widths.is_empty()
            || !self.stroke_styles.is_empty();
        self.colors.clear();
        self.line_widths.clear();
        self.stroke_styles.clear();
        self.fill_visible = true;
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndicatorInstance {
    pub id: IndicatorInstanceId,
    #[serde(rename = "indicator")]
    pub kind: IndicatorId,
    pub config: IndicatorConfig,
    pub style_overrides: IndicatorStyleOverrides,
    #[serde(default)]
    pub source_bindings: BTreeMap<String, IndicatorOutputRef>,
    #[serde(default)]
    pub base_source_selections: BTreeMap<String, BaseSeriesSource>,
}

impl IndicatorInstance {
    pub fn update_source_binding(
        &mut self,
        input_id: impl Into<String>,
        output: Option<IndicatorOutputRef>,
    ) -> bool {
        let input_id = input_id.into();
        if !self
            .kind
            .definition()
            .source_inputs
            .iter()
            .any(|input| input.id == input_id)
        {
            return false;
        }
        match output {
            Some(output) => {
                let binding_changed = self.source_bindings.get(&input_id) != Some(&output);
                let base_selection_removed = self.base_source_selections.remove(&input_id);
                self.source_bindings.insert(input_id, output);
                binding_changed || base_selection_removed.is_some()
            }
            None => self.source_bindings.remove(&input_id).is_some(),
        }
    }

    pub fn update_base_source(
        &mut self,
        input_id: impl Into<String>,
        source: Option<BaseSeriesSource>,
    ) -> bool {
        let input_id = input_id.into();
        if !self
            .kind
            .definition()
            .source_inputs
            .iter()
            .any(|input| input.id == input_id)
        {
            return false;
        }
        if self.source_bindings.contains_key(&input_id) {
            return false;
        }
        match source {
            Some(source) => {
                if self.base_source_selections.get(&input_id) == Some(&source) {
                    return false;
                }
                self.base_source_selections.insert(input_id, source);
                true
            }
            None => self.base_source_selections.remove(&input_id).is_some(),
        }
    }

    pub fn configured_color(&self, slot: &str) -> Option<ThemeColor> {
        self.style_overrides.color(slot).or_else(|| {
            self.kind
                .color_specs()
                .iter()
                .find(|spec| spec.id == slot)
                .map(|spec| spec.default_color())
        })
    }

    pub fn new(id: IndicatorInstanceId, kind: IndicatorId) -> Self {
        let config = IndicatorConfig::default_for(&kind);
        Self {
            id,
            kind,
            config,
            style_overrides: IndicatorStyleOverrides::default(),
            source_bindings: BTreeMap::new(),
            base_source_selections: BTreeMap::new(),
        }
    }

    pub fn normalized(mut self) -> Self {
        self.config = self.config.normalized_for(&self.kind);
        self.style_overrides = if self.kind.is_registered() {
            self.style_overrides
                .normalized_for_slots(&self.kind.definition().style_slots)
        } else {
            self.style_overrides.normalized()
        };
        for input_id in self.source_bindings.keys().cloned().collect::<Vec<_>>() {
            self.base_source_selections.remove(&input_id);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndicatorInstanceStore {
    instances: Vec<IndicatorInstance>,
    next_id: u64,
    warmup_bars_by_id: BTreeMap<IndicatorInstanceId, Option<u64>>,
}

impl PartialEq for IndicatorInstanceStore {
    fn eq(&self, other: &Self) -> bool {
        self.instances == other.instances && self.next_id == other.next_id
    }
}

impl IndicatorInstanceStore {
    pub fn validate_source_binding(
        &self,
        active: &[IndicatorInstanceId],
        id: IndicatorInstanceId,
        input_id: &str,
    ) -> Result<(), String> {
        let active = active
            .iter()
            .copied()
            .collect::<BTreeSet<IndicatorInstanceId>>();
        let Some(instance) = self.get(id) else {
            return Err(format!(
                "source binding references missing indicator instance {id:?}"
            ));
        };
        if !active.contains(&id) {
            return Err("source binding target is not active".to_owned());
        }
        if !instance.kind.definition().has_numeric_plot_layers() {
            return Err("attached indicator has no numeric plot layer".to_owned());
        }
        let Some(target) = self
            .dependency_target(instance, input_id, &active)
            .map_err(|problem| problem.to_string())?
        else {
            return Ok(());
        };

        let edges = self.dependency_edges(&active, Some((id, input_id)));
        let mut pending = vec![target.id];
        let mut visited = FxHashSet::default();
        while let Some(current) = pending.pop() {
            if current == id {
                return Err("indicator dependency cycle".to_owned());
            }
            if visited.insert(current) {
                pending.extend(edges.get(&current).into_iter().flatten().copied());
            }
        }
        Ok(())
    }

    pub fn source_offsets(&self) -> Vec<i64> {
        let mut offsets = vec![0];
        for instance in self.instances() {
            let kind = &instance.kind;
            offsets.extend(
                kind.output_channels()
                    .iter()
                    .map(|output| i64::from(output.offset_bars)),
            );

            for fill in kind.fill_specs() {
                let channel_offset = |source: &IndicatorOverlaySource| {
                    let IndicatorOverlaySource::Channel(channel) = source;
                    kind.output_channels()
                        .iter()
                        .find(|output| output.id == *channel)
                        .map(|output| i64::from(output.offset_bars))
                        .unwrap_or_default()
                };
                offsets.push(if fill.upper_offset_bars == 0 {
                    channel_offset(&fill.upper)
                } else {
                    i64::from(fill.upper_offset_bars)
                });
                offsets.push(if fill.lower_offset_bars == 0 {
                    channel_offset(&fill.lower)
                } else {
                    i64::from(fill.lower_offset_bars)
                });
            }
        }
        offsets
    }

    pub fn max_positive_output_offset(&self) -> i64 {
        self.instances()
            .iter()
            .filter_map(|instance| {
                instance
                    .kind
                    .output_channels()
                    .iter()
                    .map(|output| output.offset_bars)
                    .filter(|offset| *offset > 0)
                    .max()
            })
            .map(i64::from)
            .max()
            .unwrap_or_default()
    }

    pub fn from_instances(instances: Vec<IndicatorInstance>) -> Self {
        let mut store = Self::default();
        for instance in instances {
            if !instance.kind.is_registered() {
                continue;
            }
            let instance = instance.normalized();
            if !instance.config.is_valid_for(&instance.kind) {
                continue;
            }
            if store.get(instance.id).is_none() {
                store.next_id = store.next_id.max(
                    instance
                        .id
                        .0
                        .checked_add(1)
                        .filter(|next| *next < DEFAULT_INDICATOR_INSTANCE_ID_BASE)
                        .unwrap_or(1),
                );
                store.instances.push(instance);
            }
        }
        store.rebuild_warmup_cache();
        store.next_id = store.next_id.max(1);
        store
    }

    pub fn instances(&self) -> &[IndicatorInstance] {
        &self.instances
    }

    pub fn get(&self, id: IndicatorInstanceId) -> Option<&IndicatorInstance> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    pub fn warmup_bars(&self, id: IndicatorInstanceId) -> Option<u64> {
        self.warmup_bars_by_id.get(&id).copied().unwrap_or_else(|| {
            self.get(id)
                .and_then(|instance| instance.kind.warmup_bars(&instance.config))
        })
    }

    pub fn create(&mut self, kind: IndicatorId) -> IndicatorInstanceId {
        while self
            .instances
            .iter()
            .any(|instance| instance.id == IndicatorInstanceId::new(self.next_id))
        {
            self.next_id += 1;
        }
        let id = IndicatorInstanceId::new(self.next_id);
        self.next_id += 1;
        let instance = IndicatorInstance::new(id, kind);
        self.warmup_bars_by_id
            .insert(id, instance.kind.warmup_bars(&instance.config));
        self.instances.push(instance);
        id
    }

    pub fn update_config(&mut self, id: IndicatorInstanceId, config: IndicatorConfig) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.id == id) else {
            return false;
        };
        let config = config.normalized_for(&instance.kind);
        if !config.is_valid_for(&instance.kind) {
            return false;
        }
        if instance.config == config {
            return false;
        }
        instance.config = config;
        let warmup_bars = instance.kind.warmup_bars(&instance.config);
        self.warmup_bars_by_id.insert(id, warmup_bars);
        true
    }

    pub fn reset_config(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(kind) = self.get(id).map(|instance| instance.kind.clone()) else {
            return false;
        };
        self.update_config(id, IndicatorConfig::default_for(&kind))
    }

    pub fn update_source_binding(
        &mut self,
        id: IndicatorInstanceId,
        input_id: impl Into<String>,
        output: Option<IndicatorOutputRef>,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.update_source_binding(input_id, output))
    }

    pub fn update_base_source(
        &mut self,
        id: IndicatorInstanceId,
        input_id: impl Into<String>,
        source: Option<BaseSeriesSource>,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.update_base_source(input_id, source))
    }

    pub fn update_color(
        &mut self,
        id: IndicatorInstanceId,
        slot: impl Into<String>,
        color: ThemeColor,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.style_overrides.set_color(slot, color))
    }

    pub fn update_fill_visibility(&mut self, id: IndicatorInstanceId, visible: bool) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| {
                if instance.style_overrides.fill_visible == visible {
                    return false;
                }
                instance.style_overrides.fill_visible = visible;
                true
            })
    }

    pub fn update_line_width(
        &mut self,
        id: IndicatorInstanceId,
        slot: impl Into<String>,
        width: u8,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.style_overrides.set_line_width(slot, width))
    }

    pub fn update_stroke_style(
        &mut self,
        id: IndicatorInstanceId,
        slot: impl Into<String>,
        stroke_style: LineStrokeStyle,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| {
                instance
                    .style_overrides
                    .set_stroke_style(slot, stroke_style)
            })
    }

    pub fn reset_style(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.id == id) else {
            return false;
        };
        instance.style_overrides.clear()
    }

    pub fn remove(&mut self, id: IndicatorInstanceId) -> Option<IndicatorInstance> {
        let index = self
            .instances
            .iter()
            .position(|instance| instance.id == id)?;
        self.warmup_bars_by_id.remove(&id);
        Some(self.instances.remove(index))
    }

    pub fn history_requirement(
        &self,
        active: &[IndicatorInstanceId],
        basis: Basis,
        reference_time: exchange::UnixMs,
    ) -> IndicatorHistoryRequirement {
        let active_set = active
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some())
            .collect::<BTreeSet<_>>();
        let mut memo = BTreeMap::new();
        active
            .iter()
            .filter_map(|id| {
                history_requirement_for_instance(
                    *id,
                    self,
                    &active_set,
                    basis,
                    reference_time,
                    &mut memo,
                    &mut BTreeSet::new(),
                )
            })
            .fold(Default::default(), IndicatorHistoryRequirement::merge)
    }

    pub fn dependency_warmup_bars(&self, active: &[IndicatorInstanceId]) -> u64 {
        let active_set = active
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some())
            .collect::<BTreeSet<_>>();
        let mut memo = BTreeMap::new();
        active
            .iter()
            .filter_map(|id| {
                dependency_warmup_for_instance(
                    *id,
                    self,
                    &active_set,
                    &mut memo,
                    &mut BTreeSet::new(),
                )
            })
            .max()
            .unwrap_or(0)
    }

    fn rebuild_warmup_cache(&mut self) {
        self.warmup_bars_by_id = self
            .instances
            .iter()
            .map(|instance| (instance.id, instance.kind.warmup_bars(&instance.config)))
            .collect();
    }
}

const MAX_INDICATOR_HISTORY_BARS: u64 = 50_000;

/// Why a declared source binding cannot act as an indicator dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceBindingProblem {
    UnknownInput(String),
    InputDisallowsIndicator(String),
    SelfReference,
    MissingInstance(IndicatorInstanceId),
    MissingOutput(String),
    NotSourceableOutput(String),
}

impl std::fmt::Display for SourceBindingProblem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownInput(input) => {
                write!(
                    formatter,
                    "source binding references unknown input '{input}'"
                )
            }
            Self::InputDisallowsIndicator(input) => write!(
                formatter,
                "source input '{input}' does not allow indicator bindings"
            ),
            Self::SelfReference => formatter.write_str("indicator cannot depend on itself"),
            Self::MissingInstance(id) => write!(
                formatter,
                "source binding references missing indicator instance {id:?}"
            ),
            Self::MissingOutput(channel) => write!(
                formatter,
                "source binding references missing output '{channel}'"
            ),
            Self::NotSourceableOutput(channel) => write!(
                formatter,
                "source binding output '{channel}' is not a sourceable numeric plot output"
            ),
        }
    }
}

impl IndicatorInstanceStore {
    /// Resolves `source_id` on `instance` to the instance it draws from.
    ///
    /// `Ok(None)` means the binding is absent, so the input contributes nothing.
    /// `Err` means the binding is present but cannot act as a dependency.
    pub fn dependency_target<'a>(
        &'a self,
        instance: &IndicatorInstance,
        source_id: &str,
        active: &BTreeSet<IndicatorInstanceId>,
    ) -> Result<Option<&'a IndicatorInstance>, SourceBindingProblem> {
        let definition = instance.kind.definition();
        let Some(input) = definition
            .source_inputs
            .iter()
            .find(|input| input.id == source_id)
        else {
            return Err(SourceBindingProblem::UnknownInput(source_id.to_owned()));
        };
        if !input.allow_indicator {
            return Err(SourceBindingProblem::InputDisallowsIndicator(
                source_id.to_owned(),
            ));
        }
        let Some(reference) = instance.source_bindings.get(source_id) else {
            return Ok(None);
        };
        if reference.instance_id == instance.id {
            return Err(SourceBindingProblem::SelfReference);
        }
        if !active.contains(&reference.instance_id) {
            return Err(SourceBindingProblem::MissingInstance(reference.instance_id));
        }
        let Some(target) = self.get(reference.instance_id) else {
            return Err(SourceBindingProblem::MissingInstance(reference.instance_id));
        };
        let target_definition = target.kind.definition();
        if target_definition.output(&reference.channel_id).is_none() {
            return Err(SourceBindingProblem::MissingOutput(
                reference.channel_id.clone(),
            ));
        }
        if target_definition
            .sourceable_numeric_output(&reference.channel_id)
            .is_none()
        {
            return Err(SourceBindingProblem::NotSourceableOutput(
                reference.channel_id.clone(),
            ));
        }
        Ok(Some(target))
    }

    /// Dependency edges among `active` instances, optionally ignoring one binding.
    pub fn dependency_edges(
        &self,
        active: &BTreeSet<IndicatorInstanceId>,
        excluded: Option<(IndicatorInstanceId, &str)>,
    ) -> BTreeMap<IndicatorInstanceId, Vec<IndicatorInstanceId>> {
        let mut edges = BTreeMap::<IndicatorInstanceId, Vec<IndicatorInstanceId>>::new();
        for id in active {
            let Some(instance) = self.get(*id) else {
                continue;
            };
            let mut targets = Vec::new();
            for source_id in instance.source_bindings.keys() {
                if excluded == Some((*id, source_id.as_str())) {
                    continue;
                }
                if let Ok(Some(target)) = self.dependency_target(instance, source_id, active)
                    && !targets.contains(&target.id)
                {
                    targets.push(target.id);
                }
            }
            if !targets.is_empty() {
                edges.insert(*id, targets);
            }
        }
        edges
    }
}

fn dependency_warmup_for_instance(
    id: IndicatorInstanceId,
    store: &IndicatorInstanceStore,
    active: &BTreeSet<IndicatorInstanceId>,
    memo: &mut BTreeMap<IndicatorInstanceId, u64>,
    visiting: &mut BTreeSet<IndicatorInstanceId>,
) -> Option<u64> {
    if let Some(warmup) = memo.get(&id) {
        return Some(*warmup);
    }
    if !visiting.insert(id) {
        return Some(0);
    }
    let instance = store.get(id)?;
    let own = store.warmup_bars(id).unwrap_or_default();
    let dependency = instance
        .source_bindings
        .keys()
        .filter_map(|source_id| {
            store
                .dependency_target(instance, source_id, active)
                .ok()
                .flatten()
                .and_then(|target| {
                    dependency_warmup_for_instance(target.id, store, active, memo, visiting)
                })
        })
        .max()
        .unwrap_or(0);
    visiting.remove(&id);
    let warmup = own
        .saturating_add(dependency)
        .min(MAX_INDICATOR_HISTORY_BARS);
    memo.insert(id, warmup);
    Some(warmup)
}

fn history_requirement_for_instance(
    id: IndicatorInstanceId,
    store: &IndicatorInstanceStore,
    active: &BTreeSet<IndicatorInstanceId>,
    basis: Basis,
    reference_time: exchange::UnixMs,
    memo: &mut BTreeMap<IndicatorInstanceId, IndicatorHistoryRequirement>,
    visiting: &mut BTreeSet<IndicatorInstanceId>,
) -> Option<IndicatorHistoryRequirement> {
    if let Some(requirement) = memo.get(&id) {
        return Some(*requirement);
    }
    if !visiting.insert(id) {
        return Some(IndicatorHistoryRequirement::default());
    }
    let instance = store.get(id)?;
    let own = instance.kind.history_requirement_with_warmup(
        instance.config.clone(),
        basis,
        reference_time,
        store.warmup_bars(id),
    );
    let dependency = instance
        .source_bindings
        .keys()
        .filter_map(|source_id| {
            store
                .dependency_target(instance, source_id, active)
                .ok()
                .flatten()
                .and_then(|target| {
                    history_requirement_for_instance(
                        target.id,
                        store,
                        active,
                        basis,
                        reference_time,
                        memo,
                        visiting,
                    )
                })
        })
        .fold(Default::default(), IndicatorHistoryRequirement::merge);
    visiting.remove(&id);
    let history_start = match (own.history_start(), dependency.history_start()) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(time), None) | (None, Some(time)) => Some(time),
        (None, None) => None,
    };
    let requirement = IndicatorHistoryRequirement::new(
        history_start,
        own.history_bars()
            .saturating_add(dependency.history_bars())
            .min(MAX_INDICATOR_HISTORY_BARS),
    );
    memo.insert(id, requirement);
    Some(requirement)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FloatingToolbarPosition {
    pub x_ratio: f32,
    pub y_ratio: f32,
}

impl Default for FloatingToolbarPosition {
    fn default() -> Self {
        Self {
            x_ratio: 0.0,
            y_ratio: 0.0,
        }
    }
}

impl FloatingToolbarPosition {
    pub fn clamped(self) -> Self {
        let clamp_ratio = |ratio: f32| {
            if ratio.is_finite() {
                ratio.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };

        Self {
            x_ratio: clamp_ratio(self.x_ratio),
            y_ratio: clamp_ratio(self.y_ratio),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegendVisibility {
    ShowOnHover,
    #[default]
    KeepLegendVisible,
    KeepLegendAndValuesVisible,
}

impl LegendVisibility {
    pub const ALL: [Self; 3] = [
        Self::ShowOnHover,
        Self::KeepLegendVisible,
        Self::KeepLegendAndValuesVisible,
    ];

    pub const fn legend_always_visible(self) -> bool {
        !matches!(self, Self::ShowOnHover)
    }

    pub const fn legend_values_always_visible(self) -> bool {
        matches!(self, Self::KeepLegendAndValuesVisible)
    }
}

impl std::fmt::Display for LegendVisibility {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ShowOnHover => "Show on hover",
            Self::KeepLegendVisible => "Keep legend visible",
            Self::KeepLegendAndValuesVisible => "Keep legend and values visible",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub primary_scale_inverted: bool,
    pub trade_data_enabled: bool,
    #[serde(default)]
    pub legend_visibility: LegendVisibility,
    pub candlestick: CandlestickConfig,
    pub footprint: FootprintConfig,
    pub panels: Vec<PanelConfig>,
    pub splits: Vec<f32>,
    pub drawings: Vec<DrawingConfig>,
    pub active_drawing_tool: DrawingTool,
    pub floating_toolbar_position: FloatingToolbarPosition,
    pub drawing_sidebar_visible: bool,
    pub studies: Vec<KlineStudy>,
    pub study_configs: Vec<KlineStudy>,
    #[serde(default)]
    pub tpo_profile_edits: TpoBoundaryEdits,
    pub indicator_instances: Vec<IndicatorInstance>,
    pub series: Vec<SeriesConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            primary_scale_inverted: false,
            trade_data_enabled: false,
            legend_visibility: LegendVisibility::default(),
            candlestick: CandlestickConfig::default(),
            footprint: FootprintConfig::default(),
            panels: Vec::new(),
            splits: Vec::new(),
            drawings: Vec::new(),
            active_drawing_tool: DrawingTool::Cursor,
            floating_toolbar_position: FloatingToolbarPosition::default(),
            drawing_sidebar_visible: true,
            studies: Vec::new(),
            study_configs: Vec::new(),
            tpo_profile_edits: TpoBoundaryEdits::default(),
            indicator_instances: Vec::new(),
            series: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsSnapshot {
    pub primary_scale_inverted: bool,
    pub trade_data_enabled: bool,
    #[serde(default)]
    pub legend_visibility: LegendVisibility,
    pub candlestick: CandlestickConfig,
    pub footprint: FootprintConfig,
    pub panels: Vec<PanelConfig>,
    pub splits: Vec<f32>,
    pub studies: Vec<KlineStudy>,
    pub study_configs: Vec<KlineStudy>,
    #[serde(default)]
    pub tpo_profile_edits: TpoBoundaryEdits,
    pub indicator_instances: Vec<IndicatorInstance>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ChartSettings {
    pub drawing: drawing::DrawingSettings,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Config::default().settings_snapshot()
    }
}

impl Config {
    pub fn settings_snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot {
            primary_scale_inverted: self.primary_scale_inverted,
            trade_data_enabled: self.trade_data_enabled,
            legend_visibility: self.legend_visibility,
            candlestick: self.candlestick,
            footprint: self.footprint,
            panels: self.panels.clone(),
            splits: self.splits.clone(),
            studies: self.studies.clone(),
            study_configs: self.study_configs.clone(),
            tpo_profile_edits: self.tpo_profile_edits.clone(),
            indicator_instances: self.indicator_instances.clone(),
        }
    }

    pub fn indicator_instance_store(&self) -> IndicatorInstanceStore {
        indicator_instance_store(&self.indicator_instances)
    }
}

impl SettingsSnapshot {
    pub fn indicator_instance_store(&self) -> IndicatorInstanceStore {
        indicator_instance_store(&self.indicator_instances)
    }
}

fn indicator_instance_store(instances: &[IndicatorInstance]) -> IndicatorInstanceStore {
    if !instances.is_empty() {
        return IndicatorInstanceStore::from_instances(instances.to_vec());
    }

    let kind = IndicatorId::parse("core.volume").expect("static ID is valid");
    IndicatorInstanceStore::from_instances(vec![IndicatorInstance::new(
        IndicatorInstanceId::default_for(&kind),
        kind,
    )])
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub id: u32,
    pub role: PanelRole,
    pub presentation: composition::PanelPresentation,
    #[serde(default)]
    pub named_pane: Option<u32>,
    pub title: Option<String>,
    pub value_precision: Option<ValuePrecision>,
    pub value_label_policy: ValueLabelPolicy,
    pub base_layer: Option<u32>,
    pub preferred_scale: PanelScaleMode,
    pub comparison_policy: ComparisonPolicy,
    pub layers: Vec<LayerConfig>,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            id: 0,
            role: PanelRole::Auxiliary,
            presentation: composition::PanelPresentation::Plot,
            named_pane: None,
            title: None,
            value_precision: None,
            value_label_policy: ValueLabelPolicy::default(),
            base_layer: None,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: ComparisonPolicy::default(),
            layers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub id: u32,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
    #[serde(default)]
    pub purpose: LayerPurpose,
    pub mark: Option<MarkKind>,
    pub mark_override: Option<MarkKind>,
    pub axis: AxisBinding,
    pub visible: bool,
    pub style: LayerStyle,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            source: LayerSource::Primary,
            data_kind: LayerDataKind::Scalar,
            purpose: LayerPurpose::Plot,
            mark: None,
            mark_override: None,
            axis: AxisBinding::Primary,
            visible: true,
            style: LayerStyle::default(),
        }
    }
}

pub use composition::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, LayerPurpose, LayerStyle, MarkKind,
    PanelPresentation, PanelRole, PanelScaleMode, PanelValueId, PanelValueLabelMode,
    PanelValueLabelPolicy,
};
pub type ComparisonPolicy = composition::PanelComparisonPolicy;
pub type ValueLabelPolicy = composition::PanelValueLabelPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerSource {
    #[default]
    Primary,
    Comparison,
    IndicatorOutput(indicators::IndicatorOutputRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(i8),
    FixedStep(f64),
}

pub use drawing::{
    DrawingAnchor, DrawingColor, DrawingEntity, DrawingObject, DrawingSettings, DrawingStyle,
    DrawingStyleDefaults, DrawingTool,
};
pub type DrawingConfig = DrawingEntity;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesConfig {
    pub ticker: exchange::Ticker,
    #[serde(default)]
    pub color: Option<iced_core::Color>,
    #[serde(default)]
    pub accent: Option<ThemeAccent>,
    #[serde(default)]
    pub shade: ThemeColorShade,
    #[serde(default = "default_series_line_width")]
    pub line_width: u8,
    #[serde(default)]
    pub stroke_style: LineStrokeStyle,
}

fn default_series_line_width() -> u8 {
    MIN_LINE_WIDTH
}

pub fn panel_config_from_spec(panel: &PanelSpec) -> PanelConfig {
    PanelConfig {
        id: panel.id.0,
        role: panel.role,
        presentation: panel.presentation,
        named_pane: panel.named_pane,
        title: panel.title.clone(),
        value_precision: panel.value_precision.map(value_precision_from_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(|layer_id| layer_id.0),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel.layers.iter().map(layer_config_from_spec).collect(),
    }
}

fn layer_config_from_spec(layer: &LayerSpec) -> LayerConfig {
    LayerConfig {
        id: layer.id.0,
        name: layer.name.clone(),
        source: layer_source_from_domain(&layer.source),
        data_kind: layer.data_kind,
        purpose: layer.purpose,
        mark: Some(layer.mark),
        mark_override: layer.mark_override,
        axis: layer.axis,
        visible: layer.visible,
        style: layer.style.clone(),
    }
}

pub fn panel_spec_from_config(panel: &PanelConfig) -> PanelSpec {
    PanelSpec {
        id: composition::PanelId(panel.id),
        role: panel.role,
        presentation: panel.presentation,
        named_pane: panel.named_pane,
        title: panel.title.clone(),
        value_precision: panel.value_precision.map(value_precision_into_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(composition::LayerId),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel.layers.iter().map(layer_spec_from_config).collect(),
    }
}

fn layer_spec_from_config(layer: &LayerConfig) -> LayerSpec {
    let source = layer_source_into_domain(layer.source.clone());
    LayerSpec {
        id: composition::LayerId(layer.id),
        name: layer.name.clone(),
        source: source.clone(),
        data_kind: layer.data_kind,
        purpose: layer.purpose,
        mark: layer
            .mark
            .unwrap_or_else(|| composition::default_mark_for_data_kind(layer.data_kind)),
        mark_override: layer.mark_override,
        axis: layer.axis,
        visible: layer.visible,
        style: layer.style.clone(),
    }
}

fn layer_source_from_domain(source: &DomainLayerSource) -> LayerSource {
    match source {
        DomainLayerSource::RawKline {
            source: composition::DataSourceId::Synthetic("Comparison"),
        } => LayerSource::Comparison,
        DomainLayerSource::RawKline { .. } => LayerSource::Primary,
        DomainLayerSource::IndicatorOutput { output, .. } => {
            LayerSource::IndicatorOutput(output.clone())
        }
    }
}

fn layer_source_into_domain(source: LayerSource) -> DomainLayerSource {
    match source {
        LayerSource::Primary => DomainLayerSource::RawKline {
            source: composition::DataSourceId::Primary,
        },
        LayerSource::Comparison => DomainLayerSource::RawKline {
            source: composition::DataSourceId::Synthetic("Comparison"),
        },
        LayerSource::IndicatorOutput(output) => DomainLayerSource::IndicatorOutput {
            source: composition::DataSourceId::Primary,
            output,
        },
    }
}

fn value_precision_from_domain(precision: DomainPanelValuePrecision) -> ValuePrecision {
    match precision {
        DomainPanelValuePrecision::BaseTickerMinTick => ValuePrecision::BaseTickerMinTick,
        DomainPanelValuePrecision::BaseTickerMinQty => ValuePrecision::BaseTickerMinQty,
        DomainPanelValuePrecision::FixedPower10(step) => ValuePrecision::FixedPower10(step.power),
        DomainPanelValuePrecision::FixedStep(step) => ValuePrecision::FixedStep(step),
    }
}

fn value_precision_into_domain(precision: ValuePrecision) -> DomainPanelValuePrecision {
    match precision {
        ValuePrecision::BaseTickerMinTick => DomainPanelValuePrecision::BaseTickerMinTick,
        ValuePrecision::BaseTickerMinQty => DomainPanelValuePrecision::BaseTickerMinQty,
        ValuePrecision::FixedPower10(power) => {
            DomainPanelValuePrecision::FixedPower10(exchange::unit::Power10::new(power))
        }
        ValuePrecision::FixedStep(step) => DomainPanelValuePrecision::FixedStep(step),
    }
}
