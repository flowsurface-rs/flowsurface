mod anchored_vwap;
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
mod trade_count;
mod volume;

use super::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueId,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use super::engine::IndicatorConfigStore;
use super::values::IndicatorDataFieldKey;
use crate::chart::Basis;
use enum_map::Enum;
use exchange::{TickerInfo, UnixMs};

pub use crate::chart::kline_v2::values::IndicatorCellValue;
pub use anchored_vwap::{AnchoredVwapConfig, AnchoredVwapState, VwapAnchor};
pub use bar_analysis::BarAnalysisState;
pub use bollinger::BollingerBandsCalculationConfig;
pub use catalog::{
    IndicatorRuntimeState, KlineIndicator, all_indicators, availability, color_specs, display_name,
    indicators_for_market, input_requirements, is_supported_for_market, kline_warmup_bars,
    mark_specs, overlay_channels, panel_recipe, requires_trade_data, table_columns,
};
pub use input::{
    IndicatorInput, IndicatorInputSource, KLINE_INPUTS, KlinePriceSource, NormalizedTradeBar,
    OPEN_INTEREST_INPUTS, TRADE_INPUTS, TradeBarValues,
};
pub use moving_average::MovingAverageMethod;
pub use moving_averages::MovingAveragesCalculationConfig;
pub use rsi::{RsiConfig, RsiSmoothing};
pub use runtime::{IndicatorRuntime, IndicatorRuntimeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Enum)]
pub enum IndicatorColorSlot {
    Main,
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
    pub const ALL: [Self; 10] = [
        Self::Main,
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
    Reference(fn(&IndicatorConfigStore) -> f32),
}

#[derive(Debug, Clone, Copy)]
pub struct IndicatorOverlayChannelSpec {
    pub label: &'static str,
    pub source: IndicatorOverlaySource,
    pub line_width: f32,
    pub color_role: IndicatorOverlayColorRole,
    pub color_slot: IndicatorColorSlot,
}

pub(super) const NO_MARK_SPECS: [IndicatorMarkSpec; 0] = [];
pub(super) const NO_OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 0] = [];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnValue {
    Primary,
    Field(crate::chart::kline_v2::values::IndicatorDataFieldKey),
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
    SourceNotSupported,
    ResolutionNotSupported,
    InsufficientData { available: usize, required: usize },
    MissingRequiredInput,
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
    },
    Table {
        panel_title: &'static str,
        source: DataSourceId,
        columns: &'static [TableColumnSpec],
    },
    PrimaryOverlay {
        layer_name: &'static str,
        source: DataSourceId,
        value_id: PanelValueId,
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
