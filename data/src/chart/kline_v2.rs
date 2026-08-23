use enum_map::EnumMap;
use exchange::Ticker;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

use crate::chart::Basis;
use crate::chart::kline::ClusterKind;
use crate::config::theme::ThemeColor;
use crate::config::theme::ThemeColorRole;
use crate::config::theme::ThemeColorShade;

use composition::{
    LayerSource as DomainLayerSource, LayerSpec, PanelSpec,
    PanelValuePrecision as DomainPanelValuePrecision,
};

pub use crate::config::theme::ThemeAccent;

pub mod composition;
pub mod drawing;
pub mod fetch_plan;
pub mod indicators;
pub mod series;
pub mod studies;
pub mod values;
pub mod viewport;

pub use series::SeriesColor;

pub use fetch_plan::{
    FetchBatch, FetchPlanInput, FetchRange, FetchStage, desired_fetch_batches,
    desired_trade_fetch_range, visible_window,
};
pub use indicators::AtrConfig;
pub use indicators::BollingerBandsCalculationConfig;
pub use indicators::KlineIndicator;
pub use indicators::MovingAverageMethod;
pub use indicators::MovingAveragesCalculationConfig;
pub use indicators::StochRsiConfig;
pub use indicators::{AnchoredVwapConfig, VwapAnchor};
pub use indicators::{
    IndicatorCellValue, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfigValue,
    IndicatorFillGeometry, IndicatorFillSpec, IndicatorHistoryRequirement, IndicatorInput,
    IndicatorInputCoverage, IndicatorInputSource, IndicatorMarkSpec, IndicatorOverlayChannelSpec,
    IndicatorOverlayColorRole, IndicatorOverlaySource, IndicatorRuntime, IndicatorRuntimeSet,
    IndicatorRuntimeSnapshot, IndicatorRuntimeState, IndicatorSetting, IndicatorSettingChoice,
    IndicatorSettingControl, IndicatorSettingValue, IndicatorSettingsContext, KlinePriceSource,
};
pub use indicators::{IndicatorConfig, IndicatorConfigStore};
pub use indicators::{RsiConfig, RsiSmoothing};
pub use studies::{
    DEFAULT_RECURRING_VOLUME_PROFILE_INTERVAL, DEFAULT_RECURRING_VOLUME_PROFILE_OPACITY_PERCENT,
    DEFAULT_VISIBLE_VOLUME_PROFILE_OPACITY_PERCENT, DEFAULT_VOLUME_PROFILE_WIDTH_PERCENT,
    KlineStudy, KlineStudyId, MAX_VOLUME_PROFILE_OPACITY_PERCENT, MAX_VOLUME_PROFILE_WIDTH_PERCENT,
    MIN_VOLUME_PROFILE_OPACITY_PERCENT, MIN_VOLUME_PROFILE_WIDTH_PERCENT, VolumeProfileColorConfig,
    VolumeProfileColorSlot, VolumeProfileColorSpec, VolumeProfileSchedule,
};
pub use viewport::{
    ChartCoord, ChartStepMs, DEFAULT_HORIZONTAL_SPACING_PX, HorizontalWorld,
    MAX_HORIZONTAL_SPACING_PX, MIN_HORIZONTAL_SPACING_PX, PanelYViewport, WorldAxis, YUnit,
    ZOOM_SENSITIVITY,
};

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

const fn default_show_footprint_poc() -> bool {
    false
}

const fn default_show_footprint_npoc() -> bool {
    true
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintPocConfig {
    #[serde(default = "default_show_footprint_poc")]
    pub show_poc: bool,
    #[serde(default = "default_show_footprint_npoc")]
    pub show_npoc: bool,
    #[serde(default)]
    pub colors: FootprintPocColorConfig,
}

impl Default for FootprintPocConfig {
    fn default() -> Self {
        Self {
            show_poc: false,
            show_npoc: true,
            colors: FootprintPocColorConfig::defaults(),
        }
    }
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

pub const DEFAULT_CANDLESTICK_OPACITY_PERCENT: u8 = 100;
pub const MIN_CANDLESTICK_OPACITY_PERCENT: u8 = 5;
pub const MAX_CANDLESTICK_OPACITY_PERCENT: u8 = 100;
pub const MIN_LINE_WIDTH: u8 = 1;
pub const MAX_LINE_WIDTH: u8 = 4;

const LEGACY_INDICATOR_INSTANCE_ID_BASE: u64 = 1 << 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IndicatorInstanceId(u64);

impl IndicatorInstanceId {
    fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn legacy_for(kind: KlineIndicator) -> Self {
        let index = indicators::all_indicators()
            .iter()
            .position(|candidate| *candidate == kind)
            .expect("catalog indicator must have an index");
        Self(LEGACY_INDICATOR_INSTANCE_ID_BASE + index as u64)
    }

    pub fn legacy_kind(self) -> Option<KlineIndicator> {
        let index = self.0.checked_sub(LEGACY_INDICATOR_INSTANCE_ID_BASE)? as usize;
        indicators::all_indicators().get(index).copied()
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
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum PersistedId {
            Instance(u64),
            Legacy(KlineIndicator),
        }

        Ok(match PersistedId::deserialize(deserializer)? {
            PersistedId::Instance(value) => Self::new(value),
            PersistedId::Legacy(kind) => Self::legacy_for(kind),
        })
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

    pub fn first_index(self) -> usize {
        match self {
            Self::Full => 0,
            Self::From(index) => index,
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
pub struct CandlestickConfig {
    #[serde(default)]
    pub colors: CandlestickColorConfig,
    #[serde(default = "default_candlestick_opacity_percent")]
    pub opacity_percent: u8,
}

fn default_candlestick_opacity_percent() -> u8 {
    DEFAULT_CANDLESTICK_OPACITY_PERCENT
}

fn default_footprints_visible() -> bool {
    false
}

fn default_trade_data_enabled() -> bool {
    false
}

fn default_candlesticks_visible() -> bool {
    true
}

impl Default for CandlestickConfig {
    fn default() -> Self {
        Self {
            colors: CandlestickColorConfig::defaults(),
            opacity_percent: DEFAULT_CANDLESTICK_OPACITY_PERCENT,
        }
    }
}

impl CandlestickConfig {
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

    pub fn opacity_ratio(self) -> f32 {
        f32::from(self.opacity_percent.clamp(
            MIN_CANDLESTICK_OPACITY_PERCENT,
            MAX_CANDLESTICK_OPACITY_PERCENT,
        )) / f32::from(MAX_CANDLESTICK_OPACITY_PERCENT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorColorConfig {
    colors: EnumMap<IndicatorColorSlot, ThemeColor>,
}

impl Default for IndicatorColorConfig {
    fn default() -> Self {
        let mut colors = EnumMap::default();
        for slot in IndicatorColorSlot::ALL {
            colors[slot] = ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Base);
        }
        colors[IndicatorColorSlot::Bars] =
            ThemeColor::new(ThemeColorRole::Secondary, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Positive] =
            ThemeColor::new(ThemeColorRole::Success, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Negative] =
            ThemeColor::new(ThemeColorRole::Danger, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Signal] =
            ThemeColor::new(ThemeColorRole::Warning, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Upper] =
            ThemeColor::new(ThemeColorRole::Success, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Lower] =
            ThemeColor::new(ThemeColorRole::Danger, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Medium] =
            ThemeColor::new(ThemeColorRole::Success, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Slow] =
            ThemeColor::new(ThemeColorRole::Danger, ThemeColorShade::Base);
        colors[IndicatorColorSlot::Fill] =
            ThemeColor::new(ThemeColorRole::Primary, ThemeColorShade::Faint);
        Self { colors }
    }
}

impl IndicatorColorConfig {
    pub fn color_for(self, slot: IndicatorColorSlot) -> ThemeColor {
        self.colors[slot]
    }

    pub fn set_color(&mut self, slot: IndicatorColorSlot, color: ThemeColor) -> bool {
        if self.colors[slot] == color {
            return false;
        }
        self.colors[slot] = color;
        true
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

fn default_indicator_line_widths() -> EnumMap<IndicatorColorSlot, u8> {
    let mut widths = EnumMap::default();
    for slot in IndicatorColorSlot::ALL {
        widths[slot] = MIN_LINE_WIDTH;
    }
    widths
}

fn default_indicator_stroke_styles() -> EnumMap<IndicatorColorSlot, LineStrokeStyle> {
    let mut styles = EnumMap::default();
    for slot in IndicatorColorSlot::ALL {
        styles[slot] = LineStrokeStyle::Solid;
    }
    styles
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndicatorStyleConfig {
    pub fill_visible: bool,
    #[serde(default = "default_indicator_line_widths")]
    line_widths: EnumMap<IndicatorColorSlot, u8>,
    #[serde(default = "default_indicator_stroke_styles")]
    stroke_styles: EnumMap<IndicatorColorSlot, LineStrokeStyle>,
}

impl Default for IndicatorStyleConfig {
    fn default() -> Self {
        Self {
            fill_visible: true,
            line_widths: default_indicator_line_widths(),
            stroke_styles: default_indicator_stroke_styles(),
        }
    }
}

impl IndicatorStyleConfig {
    pub fn normalized(mut self) -> Self {
        for slot in IndicatorColorSlot::ALL {
            self.line_widths[slot] = self.line_widths[slot].clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH);
        }
        self
    }

    pub fn line_width_for(self, slot: IndicatorColorSlot) -> u8 {
        self.line_widths[slot].clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH)
    }

    pub fn stroke_style_for(self, slot: IndicatorColorSlot) -> LineStrokeStyle {
        self.stroke_styles[slot]
    }

    pub fn set_line_width(&mut self, slot: IndicatorColorSlot, width: u8) -> bool {
        let width = width.clamp(MIN_LINE_WIDTH, MAX_LINE_WIDTH);
        if self.line_width_for(slot) == width {
            return false;
        }
        self.line_widths[slot] = width;
        true
    }

    pub fn set_stroke_style(
        &mut self,
        slot: IndicatorColorSlot,
        stroke_style: LineStrokeStyle,
    ) -> bool {
        if self.stroke_style_for(slot) == stroke_style {
            return false;
        }
        self.stroke_styles[slot] = stroke_style;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorInstance {
    pub id: IndicatorInstanceId,
    pub kind: KlineIndicator,
    pub config: IndicatorConfig,
    #[serde(default)]
    pub colors: IndicatorColorConfig,
    #[serde(default)]
    pub style: IndicatorStyleConfig,
}

impl IndicatorInstance {
    pub fn new(id: IndicatorInstanceId, kind: KlineIndicator) -> Self {
        Self {
            id,
            kind,
            config: IndicatorConfig::default_for(kind),
            colors: IndicatorColorConfig::default(),
            style: IndicatorStyleConfig::default(),
        }
    }

    pub fn normalized_for_basis(mut self, basis: Basis) -> Self {
        if self.config.indicator() != self.kind {
            self.config = IndicatorConfig::default_for(self.kind);
        }
        self.config = self.config.normalized_for_basis(basis);
        self.style = self.style.normalized();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndicatorInstanceStore {
    instances: Vec<IndicatorInstance>,
    next_id: u64,
}

impl IndicatorInstanceStore {
    pub fn from_instances(instances: Vec<IndicatorInstance>, basis: Basis) -> Self {
        let mut store = Self::default();
        for instance in instances {
            let instance = instance.normalized_for_basis(basis);
            if store.get(instance.id).is_none() {
                store.next_id = store.next_id.max(
                    instance
                        .id
                        .0
                        .checked_add(1)
                        .filter(|next| *next < LEGACY_INDICATOR_INSTANCE_ID_BASE)
                        .unwrap_or(1),
                );
                store.instances.push(instance);
            }
        }
        store.next_id = store.next_id.max(1);
        store
    }

    pub fn instances(&self) -> &[IndicatorInstance] {
        &self.instances
    }

    pub fn get(&self, id: IndicatorInstanceId) -> Option<&IndicatorInstance> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    pub fn create(&mut self, kind: KlineIndicator) -> IndicatorInstanceId {
        while self
            .instances
            .iter()
            .any(|instance| instance.id == IndicatorInstanceId::new(self.next_id))
        {
            self.next_id += 1;
        }
        let id = IndicatorInstanceId::new(self.next_id);
        self.next_id += 1;
        self.instances.push(IndicatorInstance::new(id, kind));
        id
    }

    pub fn update_config(&mut self, id: IndicatorInstanceId, config: IndicatorConfig) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.id == id) else {
            return false;
        };
        if config.indicator() != instance.kind {
            return false;
        }
        let config = config.normalized();
        if instance.config == config {
            return false;
        }
        instance.config = config;
        true
    }

    pub fn reset_config(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(kind) = self.get(id).map(|instance| instance.kind) else {
            return false;
        };
        self.update_config(id, IndicatorConfig::default_for(kind))
    }

    pub fn update_color(
        &mut self,
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        color: ThemeColor,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.colors.set_color(slot, color))
    }

    pub fn reset_colors(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.id == id) else {
            return false;
        };
        let colors = IndicatorColorConfig::default();
        if instance.colors == colors {
            return false;
        }
        instance.colors = colors;
        true
    }

    pub fn update_fill_visibility(&mut self, id: IndicatorInstanceId, visible: bool) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| {
                if instance.style.fill_visible == visible {
                    return false;
                }
                instance.style.fill_visible = visible;
                true
            })
    }

    pub fn update_line_width(
        &mut self,
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        width: u8,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.style.set_line_width(slot, width))
    }

    pub fn update_stroke_style(
        &mut self,
        id: IndicatorInstanceId,
        slot: IndicatorColorSlot,
        stroke_style: LineStrokeStyle,
    ) -> bool {
        self.instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .is_some_and(|instance| instance.style.set_stroke_style(slot, stroke_style))
    }

    pub fn reset_style(&mut self, id: IndicatorInstanceId) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.id == id) else {
            return false;
        };
        let style = IndicatorStyleConfig::default();
        if instance.style == style {
            return false;
        }
        instance.style = style;
        true
    }

    pub fn remove(&mut self, id: IndicatorInstanceId) -> Option<IndicatorInstance> {
        let index = self
            .instances
            .iter()
            .position(|instance| instance.id == id)?;
        Some(self.instances.remove(index))
    }

    pub fn history_requirement(
        &self,
        active: &[IndicatorInstanceId],
        basis: Basis,
        reference_time: exchange::UnixMs,
    ) -> IndicatorHistoryRequirement {
        active.iter().fold(Default::default(), |requirement, id| {
            self.get(*id).map_or(requirement, |instance| {
                requirement.merge(instance.config.history_requirement(basis, reference_time))
            })
        })
    }

    pub fn normalized_for_basis(&self, basis: Basis) -> Self {
        Self::from_instances(self.instances.clone(), basis)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorColorConfigStore {
    configs: EnumMap<KlineIndicator, IndicatorColorConfig>,
}

impl Default for IndicatorColorConfigStore {
    fn default() -> Self {
        let mut configs = EnumMap::default();
        for indicator in indicators::all_indicators() {
            configs[*indicator] = IndicatorColorConfig::default();
        }
        Self { configs }
    }
}

impl IndicatorColorConfigStore {
    pub fn config_for(&self, indicator: KlineIndicator) -> IndicatorColorConfig {
        self.configs[indicator]
    }

    pub fn update(
        &mut self,
        indicator: KlineIndicator,
        slot: IndicatorColorSlot,
        color: ThemeColor,
    ) -> bool {
        self.configs[indicator].set_color(slot, color)
    }

    pub fn reset(&mut self, indicator: KlineIndicator) -> bool {
        let default = IndicatorColorConfig::default();
        if self.configs[indicator] == default {
            return false;
        }
        self.configs[indicator] = default;
        true
    }
}

impl KlineIndicator {
    pub fn for_market(market: exchange::adapter::MarketKind) -> &'static [Self] {
        indicators::indicators_for_market(market)
    }

    pub fn requires_trade_data(self) -> bool {
        indicators::requires_trade_data(self)
    }

    pub fn input_source_category(self) -> &'static str {
        indicators::input_source_category(self)
    }

    pub fn color_specs(self) -> &'static [IndicatorColorSpec] {
        indicators::color_specs(self)
    }

    pub fn mark_specs(self) -> &'static [IndicatorMarkSpec] {
        indicators::mark_specs(self)
    }

    pub fn overlay_channels(self) -> &'static [IndicatorOverlayChannelSpec] {
        indicators::overlay_channels(self)
    }

    pub fn line_channels(self) -> Vec<IndicatorOverlayChannelSpec> {
        let channels = self.overlay_channels();
        if channels
            .iter()
            .any(|channel| matches!(channel.source, IndicatorOverlaySource::Primary))
        {
            return channels.to_vec();
        }

        let mut line_channels = Vec::with_capacity(channels.len() + 1);
        line_channels.push(IndicatorOverlayChannelSpec {
            label: "Line",
            source: IndicatorOverlaySource::Primary,
            line_width: 1.0,
            color_role: IndicatorOverlayColorRole::Neutral,
            color_slot: IndicatorColorSlot::Main,
        });
        line_channels.extend_from_slice(channels);
        line_channels
    }

    pub fn fill_specs(self) -> &'static [IndicatorFillSpec] {
        indicators::fill_specs(self)
    }

    /// Returns the color slots relevant to the selected display mark, or
    /// `None` when mark selection does not constrain the available colors
    /// (indicators without display marks, or marks that use every color).
    pub fn active_color_slots(
        self,
        mark: Option<composition::MarkKind>,
    ) -> Option<&'static [IndicatorColorSlot]> {
        let mark_specs = self.mark_specs();
        if mark_specs.is_empty() {
            return None;
        }
        let selected = mark.or_else(|| Some(mark_specs[0].mark));
        mark_specs
            .iter()
            .find(|spec| Some(spec.mark) == selected)
            .map(|spec| spec.color_slots)
    }
}

impl Display for KlineIndicator {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(indicators::display_name(*self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u16,
    pub horizontal: HorizontalConfig,
    pub primary_autoscale: bool,
    #[serde(default = "default_candlesticks_visible")]
    pub candlesticks_visible: bool,
    #[serde(default = "default_footprints_visible")]
    pub footprints_visible: bool,
    #[serde(default = "default_trade_data_enabled", alias = "footprints_enabled")]
    pub trade_data_enabled: bool,
    #[serde(default)]
    pub candlestick: CandlestickConfig,
    #[serde(default)]
    pub footprint_cluster_kind: ClusterKind,
    #[serde(default)]
    pub footprint_poc: FootprintPocConfig,
    pub panels: Vec<PanelConfig>,
    pub splits: Vec<f32>,
    pub y_viewports: Vec<ViewportConfig>,
    pub drawings: Vec<DrawingConfig>,
    pub active_drawing_tool: DrawingTool,
    #[serde(default)]
    pub horizontal_line_ray: bool,
    #[serde(default)]
    pub box_extend_right: bool,
    pub studies: Vec<KlineStudy>,
    #[serde(default)]
    pub study_configs: Vec<KlineStudy>,
    #[serde(default)]
    pub indicator_instances: Vec<IndicatorInstance>,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub indicator_configs: Option<IndicatorConfigStore>,
    #[serde(default, skip_serializing)]
    pub rsi: RsiConfig,
    #[serde(default, skip_serializing)]
    pub atr: AtrConfig,
    #[serde(default, skip_serializing)]
    pub moving_averages: MovingAveragesPersistenceConfig,
    #[serde(default, skip_serializing)]
    pub bollinger_bands: BollingerBandsPersistenceConfig,
    #[serde(default, skip_serializing)]
    pub anchored_vwap: AnchoredVwapConfig,
    #[doc(hidden)]
    #[serde(default, rename = "indicator_provider_configs", skip_serializing)]
    pub legacy_indicator_provider_configs: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing)]
    pub indicator_colors: IndicatorColorConfigStore,
    pub series: Vec<SeriesConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 6,
            horizontal: HorizontalConfig::default(),
            primary_autoscale: true,
            candlesticks_visible: true,
            footprints_visible: false,
            trade_data_enabled: false,
            candlestick: CandlestickConfig::default(),
            footprint_cluster_kind: ClusterKind::default(),
            footprint_poc: FootprintPocConfig::default(),
            panels: Vec::new(),
            splits: Vec::new(),
            y_viewports: Vec::new(),
            drawings: Vec::new(),
            active_drawing_tool: DrawingTool::Cursor,
            horizontal_line_ray: false,
            box_extend_right: false,
            studies: Vec::new(),
            study_configs: Vec::new(),
            indicator_instances: Vec::new(),
            indicator_configs: Some(IndicatorConfigStore::default()),
            rsi: RsiConfig::default(),
            atr: AtrConfig::default(),
            moving_averages: MovingAveragesPersistenceConfig::default(),
            bollinger_bands: BollingerBandsPersistenceConfig::default(),
            anchored_vwap: AnchoredVwapConfig::default(),
            legacy_indicator_provider_configs: BTreeMap::new(),
            indicator_colors: IndicatorColorConfigStore::default(),
            series: Vec::new(),
        }
    }
}

impl Config {
    pub fn indicator_instances_for(&self, basis: Basis) -> IndicatorInstanceStore {
        if !self.indicator_instances.is_empty() {
            return IndicatorInstanceStore::from_instances(self.indicator_instances.clone(), basis);
        }

        let configs = self.indicator_configs_for(basis);
        let mut ids = self
            .panels
            .iter()
            .flat_map(|panel| {
                panel
                    .value_id
                    .into_iter()
                    .chain(panel.layers.iter().filter_map(|layer| {
                        let LayerSource::Indicator(id) = layer.source else {
                            return None;
                        };
                        Some(id)
                    }))
            })
            .collect::<Vec<_>>();
        if ids.is_empty() {
            ids.push(IndicatorInstanceId::legacy_for(KlineIndicator::Volume));
        }
        ids.sort_unstable();
        ids.dedup();

        let instances = ids
            .into_iter()
            .filter_map(|id| {
                let kind = id.legacy_kind()?;
                Some(IndicatorInstance {
                    id,
                    kind,
                    config: configs.config_for(kind),
                    colors: self.indicator_colors.config_for(kind),
                    style: IndicatorStyleConfig::default(),
                })
            })
            .collect();
        IndicatorInstanceStore::from_instances(instances, basis)
    }

    pub fn indicator_configs_for(&self, basis: Basis) -> IndicatorConfigStore {
        self.indicator_configs
            .clone()
            .unwrap_or_else(|| self.legacy_indicator_configs())
            .normalized_for_basis(basis)
    }

    fn legacy_indicator_configs(&self) -> IndicatorConfigStore {
        let mut configs = IndicatorConfigStore::default();
        let _ = configs.update(IndicatorConfig::Rsi(self.rsi));
        let _ = configs.update(IndicatorConfig::Atr(self.atr));
        let _ = configs.update(IndicatorConfig::MovingAverages(
            MovingAveragesCalculationConfig {
                fast_period: self.moving_averages.fast_period,
                medium_period: self.moving_averages.medium_period,
                slow_period: self.moving_averages.slow_period,
                method: self.moving_averages.method,
            },
        ));
        let _ = configs.update(IndicatorConfig::BollingerBands(
            BollingerBandsCalculationConfig {
                period: self.bollinger_bands.period,
                method: self.bollinger_bands.method,
                std_dev_multiplier: self.bollinger_bands.std_dev_multiplier,
            },
        ));
        let _ = configs.update(IndicatorConfig::AnchoredVwap(
            self.resolved_anchored_vwap_config(),
        ));
        configs
    }

    fn resolved_anchored_vwap_config(&self) -> AnchoredVwapConfig {
        self.legacy_indicator_provider_configs
            .get("anchored_vwap")
            .and_then(|config| serde_json::from_value(config.clone()).ok())
            .unwrap_or(self.anchored_vwap)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct HorizontalConfig {
    pub cell_width: f32,
    pub scaling: f32,
    pub offset: f32,
}

impl Default for HorizontalConfig {
    fn default() -> Self {
        Self {
            cell_width: 5.0,
            scaling: 1.0,
            offset: 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub id: u32,
    pub role: PanelRole,
    pub presentation: PanelPresentation,
    pub title: Option<String>,
    pub value_id: Option<PanelValueId>,
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
            presentation: PanelPresentation::Plot,
            title: None,
            value_id: None,
            value_precision: None,
            value_label_policy: ValueLabelPolicy::default(),
            base_layer: None,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: ComparisonPolicy::default(),
            layers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerConfig {
    pub id: u32,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
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
            mark: None,
            mark_override: None,
            axis: AxisBinding::Primary,
            visible: true,
            style: LayerStyle::default(),
        }
    }
}

pub use composition::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, LayerStyle, MarkKind, PanelPresentation,
    PanelRole, PanelScaleMode, PanelValueId, PanelValueLabelMode, PanelValueLabelPolicy,
};
pub type ComparisonPolicy = composition::PanelComparisonPolicy;
pub type ValueLabelPolicy = composition::PanelValueLabelPolicy;
pub type ValueLabelMode = composition::PanelValueLabelMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LayerSource {
    #[default]
    Primary,
    Comparison,
    Indicator(PanelValueId),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(i8),
    FixedStep(f32),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ViewportConfig {
    pub panel_id: u32,
    pub min_value: f32,
    pub max_value: f32,
}

pub use drawing::{
    DrawingAnchor, DrawingColor, DrawingEntity, DrawingObject, DrawingStyle, DrawingTool, Rgba,
};
pub type DrawingConfig = DrawingEntity;
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct MovingAveragesPersistenceConfig {
    pub fast_period: u16,
    pub medium_period: u16,
    pub slow_period: u16,
    pub method: MovingAverageMethod,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct BollingerBandsPersistenceConfig {
    pub period: u16,
    pub method: MovingAverageMethod,
    pub std_dev_multiplier: f32,
}

impl Default for MovingAveragesPersistenceConfig {
    fn default() -> Self {
        let config = MovingAveragesCalculationConfig::default();
        Self {
            fast_period: config.fast_period,
            medium_period: config.medium_period,
            slow_period: config.slow_period,
            method: config.method,
        }
    }
}

impl Default for BollingerBandsPersistenceConfig {
    fn default() -> Self {
        let config = BollingerBandsCalculationConfig::default();
        Self {
            period: config.period,
            method: config.method,
            std_dev_multiplier: config.std_dev_multiplier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesConfig {
    pub ticker: Ticker,
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
        title: panel.title.clone(),
        value_id: panel.value_id,
        value_precision: panel.value_precision.map(value_precision_from_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(|layer_id| layer_id.0),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel.layers.iter().map(layer_config_from_spec).collect(),
    }
}

pub fn layer_config_from_spec(layer: &LayerSpec) -> LayerConfig {
    LayerConfig {
        id: layer.id.0,
        name: layer.name.clone(),
        source: layer_source_from_domain(&layer.source),
        data_kind: layer.data_kind,
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
        title: panel.title.clone(),
        value_id: panel.value_id,
        value_precision: panel.value_precision.map(value_precision_into_domain),
        value_label_policy: panel.value_label_policy,
        base_layer: panel.base_layer.map(composition::LayerId),
        preferred_scale: panel.preferred_scale,
        comparison_policy: panel.comparison_policy,
        layers: panel
            .layers
            .iter()
            .map(|layer| layer_spec_from_config(layer, panel.value_id))
            .collect(),
    }
}

pub fn layer_spec_from_config(
    layer: &LayerConfig,
    panel_value_id: Option<composition::PanelValueId>,
) -> LayerSpec {
    let source = layer_source_into_domain(layer.source);
    LayerSpec {
        id: composition::LayerId(layer.id),
        name: layer.name.clone(),
        source: source.clone(),
        data_kind: layer.data_kind,
        mark: layer
            .mark
            .unwrap_or_else(|| default_mark_for_layer(&source, panel_value_id, layer.data_kind)),
        mark_override: layer.mark_override,
        axis: layer.axis,
        visible: layer.visible,
        style: layer.style.clone(),
    }
}

fn default_mark_for_layer(
    source: &DomainLayerSource,
    panel_value_id: Option<composition::PanelValueId>,
    data_kind: LayerDataKind,
) -> MarkKind {
    let value_id = match source {
        DomainLayerSource::RawIndicator { value_id, .. } => Some(*value_id),
        DomainLayerSource::RawKline { .. } => panel_value_id,
    };

    let Some(value_id) = value_id else {
        return composition::default_mark_for_data_kind(data_kind);
    };

    let Some(kind) = value_id.legacy_kind() else {
        return composition::default_mark_for_data_kind(data_kind);
    };
    match crate::chart::kline_v2::indicators::panel_recipe(kind) {
        crate::chart::kline_v2::indicators::IndicatorPanelRecipe::Plot { mark, .. }
        | crate::chart::kline_v2::indicators::IndicatorPanelRecipe::PrimaryOverlay {
            mark, ..
        } => mark,
        crate::chart::kline_v2::indicators::IndicatorPanelRecipe::Table { .. } => {
            composition::default_mark_for_data_kind(data_kind)
        }
    }
}

fn layer_source_from_domain(source: &DomainLayerSource) -> LayerSource {
    match source {
        DomainLayerSource::RawKline {
            source: composition::DataSourceId::Synthetic("Comparison"),
        } => LayerSource::Comparison,
        DomainLayerSource::RawKline { .. } => LayerSource::Primary,
        DomainLayerSource::RawIndicator { value_id, .. } => LayerSource::Indicator(*value_id),
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
        LayerSource::Indicator(value_id) => DomainLayerSource::RawIndicator {
            source: composition::DataSourceId::Primary,
            value_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_legacy_anchored_vwap_provider_config() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "indicator_provider_configs": {
                "anchored_vwap": {
                    "anchor": "Week",
                    "source": "Close"
                }
            }
        }))
        .expect("legacy chart configuration should deserialize");

        let IndicatorConfig::AnchoredVwap(indicator_config) = config
            .indicator_configs_for(Basis::Time(exchange::Timeframe::M1))
            .config_for(KlineIndicator::AnchoredVwap)
        else {
            panic!("Anchored VWAP must retain its typed configuration");
        };
        assert_eq!(
            indicator_config,
            AnchoredVwapConfig {
                anchor: VwapAnchor::Week,
                source: KlinePriceSource::Close,
            }
        );
    }

    #[test]
    fn persists_instances_without_legacy_indicator_maps() {
        let mut instances = IndicatorInstanceStore::default();
        let id = instances.create(KlineIndicator::AnchoredVwap);
        let _ = instances.update_config(
            id,
            IndicatorConfig::AnchoredVwap(AnchoredVwapConfig {
                anchor: VwapAnchor::Month,
                source: KlinePriceSource::Hlc3,
            }),
        );
        let config = Config {
            indicator_instances: instances.instances().to_vec(),
            ..Config::default()
        };

        let serialized =
            serde_json::to_value(config).expect("chart configuration should serialize");

        assert!(serialized.get("anchored_vwap").is_none());
        assert!(serialized.get("indicator_provider_configs").is_none());
        assert!(serialized.get("indicator_configs").is_none());
        assert!(serialized.get("indicator_colors").is_none());

        let IndicatorConfig::AnchoredVwap(config) = serde_json::from_value::<Config>(serialized)
            .expect("chart configuration should deserialize")
            .indicator_instances
            .into_iter()
            .find(|instance| instance.id == id)
            .expect("indicator instance should persist")
            .config
        else {
            panic!("Anchored VWAP must retain its typed configuration");
        };
        assert_eq!(config.anchor, VwapAnchor::Month);
        assert_eq!(config.source, KlinePriceSource::Hlc3);
    }

    #[test]
    fn persists_footprint_poc_settings_at_chart_level() {
        let config = Config {
            footprint_poc: FootprintPocConfig {
                show_poc: false,
                show_npoc: true,
                colors: FootprintPocColorConfig::defaults(),
            },
            ..Config::default()
        };

        let serialized =
            serde_json::to_value(config).expect("chart configuration should serialize");
        assert_eq!(serialized["footprint_poc"]["show_poc"], false);

        let restored: Config =
            serde_json::from_value(serialized).expect("chart configuration should deserialize");
        assert!(!restored.footprint_poc.show_poc);
        assert!(restored.footprint_poc.show_npoc);
    }

    #[test]
    fn migrates_legacy_panel_value_to_an_instance() {
        let config: Config = serde_json::from_value(serde_json::json!({
            "panels": [{ "value_id": "Rsi" }]
        }))
        .expect("legacy panel value should deserialize");

        let instances = config.indicator_instances_for(Basis::Time(exchange::Timeframe::M1));
        let id = IndicatorInstanceId::legacy_for(KlineIndicator::Rsi);
        let instance = instances
            .get(id)
            .expect("legacy panel should create an instance");

        assert_eq!(instance.kind, KlineIndicator::Rsi);
        assert_eq!(instance.config.indicator(), KlineIndicator::Rsi);
    }

    #[test]
    fn duplicate_kinds_keep_independent_configuration() {
        let mut instances = IndicatorInstanceStore::default();
        let first = instances.create(KlineIndicator::Rsi);
        let second = instances.create(KlineIndicator::Rsi);
        let changed = instances.update_config(
            first,
            IndicatorConfig::Rsi(RsiConfig {
                period: 21,
                ..RsiConfig::default()
            }),
        );

        assert!(changed);
        assert_ne!(
            instances.get(first).unwrap().config,
            instances.get(second).unwrap().config
        );
    }

    #[test]
    fn fill_visibility_is_instance_scoped_and_resettable() {
        let mut instances = IndicatorInstanceStore::default();
        let first = instances.create(KlineIndicator::Rsi);
        let second = instances.create(KlineIndicator::Rsi);

        assert!(instances.get(first).unwrap().style.fill_visible);
        assert!(instances.update_fill_visibility(first, false));
        assert!(!instances.get(first).unwrap().style.fill_visible);
        assert!(instances.get(second).unwrap().style.fill_visible);
        assert!(instances.reset_style(first));
        assert!(instances.get(first).unwrap().style.fill_visible);
    }

    #[test]
    fn line_style_is_instance_scoped_and_width_is_clamped() {
        let mut instances = IndicatorInstanceStore::default();
        let first = instances.create(KlineIndicator::Rsi);
        let second = instances.create(KlineIndicator::Rsi);

        assert_eq!(
            instances
                .get(first)
                .unwrap()
                .style
                .line_width_for(IndicatorColorSlot::Main),
            MIN_LINE_WIDTH
        );
        assert!(instances.update_line_width(first, IndicatorColorSlot::Main, 255));
        assert_eq!(
            instances
                .get(first)
                .unwrap()
                .style
                .line_width_for(IndicatorColorSlot::Main),
            MAX_LINE_WIDTH
        );
        assert!(instances.update_stroke_style(
            first,
            IndicatorColorSlot::Main,
            LineStrokeStyle::Dashed,
        ));
        assert_eq!(
            instances
                .get(second)
                .unwrap()
                .style
                .stroke_style_for(IndicatorColorSlot::Main),
            LineStrokeStyle::Solid
        );
        assert!(instances.reset_style(first));
        assert_eq!(
            instances
                .get(first)
                .unwrap()
                .style
                .stroke_style_for(IndicatorColorSlot::Main),
            LineStrokeStyle::Solid
        );
    }

    #[test]
    fn line_channels_supply_a_primary_value_when_no_overlay_is_declared() {
        let volume_channels = KlineIndicator::Volume.line_channels();
        assert_eq!(volume_channels.len(), 1);
        assert!(matches!(
            volume_channels[0].source,
            IndicatorOverlaySource::Primary
        ));

        let rsi_channels = KlineIndicator::Rsi.line_channels();
        assert_eq!(
            rsi_channels
                .iter()
                .filter(|channel| matches!(channel.source, IndicatorOverlaySource::Primary))
                .count(),
            1
        );
    }

    #[test]
    fn only_line_marks_support_line_styling() {
        assert!(MarkKind::Line.supports_line_styling());
        assert!(!MarkKind::Candle.supports_line_styling());
        assert!(!MarkKind::Bar(BarMode::Regular).supports_line_styling());
    }

    #[test]
    fn categorizes_indicators_by_required_input_sources() {
        assert_eq!(KlineIndicator::Rsi.input_source_category(), "Kline data");
        assert_eq!(
            KlineIndicator::CumulativeVolumeDelta.input_source_category(),
            "Kline data"
        );
        assert_eq!(
            KlineIndicator::OpenInterest.input_source_category(),
            "Open interest data"
        );
        assert_eq!(
            KlineIndicator::TradeCount.input_source_category(),
            "Trade data"
        );
    }
}
