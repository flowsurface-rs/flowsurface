mod anchored_vwap;
mod atr;
mod bar_analysis;
mod bollinger;
mod catalog;
mod cvd;
mod input;
mod moving_average;
mod moving_averages;
mod open_interest;
mod rsi;
mod runtime;
mod stoch_rsi;
mod trade_count;
mod volume;

use super::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueLabelPolicy,
    PanelValuePrecision,
};
use super::values::IndicatorDataFieldKey;
use crate::chart::Basis;
use enum_map::Enum;
use exchange::{TickerInfo, UnixMs};
use std::fmt::{Display, Formatter};

pub use crate::chart::kline::values::IndicatorCellValue;
pub use anchored_vwap::{AnchoredVwapConfig, AnchoredVwapState, VwapAnchor};
pub use atr::AtrConfig;
pub use bar_analysis::BarAnalysisState;
pub use bollinger::BollingerBandsCalculationConfig;
pub use catalog::{
    IndicatorConfig, IndicatorConfigStore, IndicatorRuntimeState, KlineIndicator, all_indicators,
    availability, color_specs, display_name, fill_specs, indicators_for_market, input_requirements,
    input_source_category, is_supported_for_market, kline_warmup_bars, mark_specs,
    overlay_channels, panel_recipe, readout_specs, requires_trade_data, table_columns,
    unsupported_reason,
};
pub use input::{
    IndicatorInput, IndicatorInputSource, KLINE_INPUTS, KlinePriceSource, NormalizedTradeBar,
    OPEN_INTEREST_INPUTS, TRADE_INPUTS, TradeBarValues,
};
pub use moving_average::MovingAverageMethod;
pub use moving_averages::MovingAveragesCalculationConfig;
pub use rsi::{RsiConfig, RsiSmoothing};
pub use runtime::{IndicatorRuntime, IndicatorRuntimeSet};
pub use stoch_rsi::StochRsiConfig;

#[derive(Debug, Clone, Copy, Default)]
pub struct IndicatorHistoryRequirement {
    history_start: Option<UnixMs>,
    history_bars: u64,
}

impl IndicatorHistoryRequirement {
    pub fn new(history_start: Option<UnixMs>, history_bars: u64) -> Self {
        Self {
            history_start,
            history_bars,
        }
    }

    pub fn history_start(self) -> Option<UnixMs> {
        self.history_start
    }

    pub fn history_bars(self) -> u64 {
        self.history_bars
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            history_start: match (self.history_start, other.history_start) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(time), None) | (None, Some(time)) => Some(time),
                (None, None) => None,
            },
            history_bars: self.history_bars.max(other.history_bars),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Enum)]
pub enum IndicatorColorSlot {
    Main,
    Fill,
    Bars,
    Positive,
    Negative,
    Signal,
    Upper,
    Lower,
    Fast,
    Medium,
    Slow,
}

impl IndicatorColorSlot {
    pub const ALL: [Self; 11] = [
        Self::Main,
        Self::Fill,
        Self::Bars,
        Self::Positive,
        Self::Negative,
        Self::Signal,
        Self::Upper,
        Self::Lower,
        Self::Fast,
        Self::Medium,
        Self::Slow,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndicatorColorSpec {
    pub label: &'static str,
    pub slot: IndicatorColorSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndicatorMarkSpec {
    pub label: &'static str,
    pub mark: MarkKind,
    pub color_slots: &'static [IndicatorColorSlot],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorOverlayColorRole {
    Neutral,
    Success,
    Danger,
    Primary,
}

#[derive(Debug, Clone, Copy)]
pub enum IndicatorOverlaySource {
    Primary,
    Field(IndicatorDataFieldKey),
    Reference(fn(IndicatorConfig) -> f32),
}

impl PartialEq for IndicatorOverlaySource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Primary, Self::Primary) => true,
            (Self::Field(left), Self::Field(right)) => left == right,
            (Self::Reference(left), Self::Reference(right)) => std::ptr::fn_addr_eq(*left, *right),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorOverlayChannelSpec {
    pub label: &'static str,
    pub source: IndicatorOverlaySource,
    pub line_width: f32,
    pub color_role: IndicatorOverlayColorRole,
    pub color_slot: IndicatorColorSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorReadoutScope {
    Always,
    Mark(MarkKind),
}

impl IndicatorReadoutScope {
    pub fn matches(self, mark: MarkKind) -> bool {
        match self {
            Self::Always => true,
            Self::Mark(expected) => expected == mark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorReadoutSpec {
    pub label: &'static str,
    pub source: IndicatorOverlaySource,
    pub color_role: IndicatorOverlayColorRole,
    pub color_slot: IndicatorColorSlot,
    pub scope: IndicatorReadoutScope,
}

impl IndicatorReadoutSpec {
    pub const fn from_overlay_channel(channel: IndicatorOverlayChannelSpec) -> Self {
        Self {
            label: channel.label,
            source: channel.source,
            color_role: channel.color_role,
            color_slot: channel.color_slot,
            scope: IndicatorReadoutScope::Always,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorFillGeometry {
    PanelBand,
    SeriesBand,
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorFillSpec {
    pub upper: IndicatorOverlaySource,
    pub lower: IndicatorOverlaySource,
    pub geometry: IndicatorFillGeometry,
    pub color_slot: IndicatorColorSlot,
    pub opacity: f32,
}

pub(super) const NO_MARK_SPECS: [IndicatorMarkSpec; 0] = [];
pub(super) const NO_OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 0] = [];
pub(super) const NO_READOUT_SPECS: [IndicatorReadoutSpec; 0] = [];
pub(super) const NO_FILL_SPECS: [IndicatorFillSpec; 0] = [];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnValue {
    Primary,
    Field(crate::chart::kline::values::IndicatorDataFieldKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnFormat {
    Quantity,
    Integer,
    Percent,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnColor {
    Neutral,
    Positive,
    Negative,
    Signed,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableColumnSpec {
    pub label: &'static str,
    pub value: TableColumnValue,
    pub format: TableColumnFormat,
    pub color: TableColumnColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndicatorInputCoverage {
    #[default]
    Unknown,
    Complete,
    MissingRequiredInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IndicatorRuntimeSnapshot {
    pub input_coverage: IndicatorInputCoverage,
    pub value_range: Option<(UnixMs, UnixMs)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorUnsupportedReason {
    BasisNotSupported,
    IncompleteHistory,
    SourceNotSupported,
    ResolutionNotSupported,
    InsufficientData { available: usize, required: usize },
    MissingRequiredInput,
}

impl IndicatorUnsupportedReason {
    pub fn indicator_unavailable_tooltip(&self) -> &'static str {
        match self {
            IndicatorUnsupportedReason::BasisNotSupported => {
                "Not available for the selected chart basis"
            }
            IndicatorUnsupportedReason::SourceNotSupported => {
                "Not available for the selected market"
            }
            IndicatorUnsupportedReason::ResolutionNotSupported => {
                "Not available at the selected timeframe"
            }
            IndicatorUnsupportedReason::IncompleteHistory => "Requires complete historical data",
            IndicatorUnsupportedReason::InsufficientData { .. } => {
                "Not enough data to use this indicator"
            }
            IndicatorUnsupportedReason::MissingRequiredInput => {
                "Enable trade data to use this indicator"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorAvailability {
    Available,
    PendingProbe,
    Unsupported(IndicatorUnsupportedReason),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorPanelRecipe {
    Plot {
        panel_title: &'static str,
        layer_name: &'static str,
        source: DataSourceId,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
        value_precision: PanelValuePrecision,
        value_label_policy: PanelValueLabelPolicy,
        preferred_scale: PanelScaleMode,
        readouts: &'static [IndicatorReadoutSpec],
    },
    Table {
        panel_title: &'static str,
        source: DataSourceId,
        columns: &'static [TableColumnSpec],
    },
    PrimaryOverlay {
        layer_name: &'static str,
        source: DataSourceId,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AvailabilityContext {
    pub basis: Basis,
    pub base_ticker: TickerInfo,
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorSettingsContext {
    pub basis: Basis,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndicatorSettingValue {
    U16(u16),
    F32(f32),
    Bool(bool),
    Choice(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndicatorSettingChoice {
    pub id: &'static str,
    pub label: &'static str,
}

impl Display for IndicatorSettingChoice {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndicatorSettingControl {
    SliderU16 { min: u16, max: u16 },
    SliderF32 { min: f32, max: f32, step: f32 },
    Toggle,
    Choices(Vec<IndicatorSettingChoice>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorSetting {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Option<&'static str>,
    pub value: IndicatorSettingValue,
    pub control: IndicatorSettingControl,
}

pub trait IndicatorConfigValue: Copy + Default + PartialEq {
    fn normalized(self) -> Self {
        self
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        Vec::new()
    }

    fn with_setting(self, _id: &'static str, _value: IndicatorSettingValue) -> Option<Self> {
        None
    }

    fn normalized_for_basis(self, _basis: Basis) -> Self {
        self.normalized()
    }

    fn history_requirement(
        self,
        _basis: Basis,
        _reference_time: UnixMs,
    ) -> IndicatorHistoryRequirement {
        IndicatorHistoryRequirement::default()
    }
}

impl IndicatorConfigValue for () {}
