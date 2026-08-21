use super::super::IndicatorUpdateScope;
use super::super::engine::IndicatorConfigStore;
use super::super::values::IndicatorData;
use super::input::{
    CVD_INPUTS, IndicatorInput, IndicatorInputSource, KLINE_INPUTS, OPEN_INTEREST_INPUTS,
};
use super::runtime::IndicatorRuntime;
use super::{
    AvailabilityContext, IndicatorAvailability, IndicatorPanelRecipe, IndicatorRuntimeSet,
    IndicatorRuntimeSnapshot,
};
use crate::chart::kline_v2::composition::{DataSourceId, PanelValuePrecision};
use crate::chart::kline_v2::indicators::input::TRADE_INPUTS;
use crate::chart::kline_v2::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode, PanelValueId,
    PanelValueLabelMode, PanelValueLabelPolicy,
};
use enum_map::Enum;
use exchange::Kline;
use exchange::adapter::MarketKind;

macro_rules! define_indicator_catalog {
    (
        spot: [$($spot:ident),+ $(,)?],
        perps: [$($perps:ident),+ $(,)?],
        entries: {
            $(
                $variant:ident => {
                    state: $state:path,
                    display_name: $display_name:literal,
                    panel_recipe: $panel_recipe:block,
                    inputs: $inputs:expr,
                    config: $config:ident,
                    colors: $colors:path,
                    marks: $marks:path,
                    channels: $channels:path
                }
            ),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Enum)]
        pub enum KlineIndicator {
            $($variant),+
        }

        #[derive(Debug, Clone)]
        pub enum IndicatorRuntimeState {
            $($variant($state)),+
        }

        impl IndicatorRuntimeState {
            pub(super) fn for_indicator(indicator: KlineIndicator) -> Self {
                match indicator {
                    $(KlineIndicator::$variant => Self::$variant(<$state>::default()),)+
                }
            }

            pub(super) fn clear(&mut self) {
                match self {
                    $(Self::$variant(state) => state.clear(),)+
                }
            }

            pub(super) fn on_input_from(
                &mut self,
                input: IndicatorInput<'_>,
                configs: &IndicatorConfigStore,
                scope: IndicatorUpdateScope,
            ) {
                match self {
                    $(Self::$variant(state) => state.on_input_from(
                        input,
                        define_indicator_catalog!(@config $config configs),
                        scope,
                    ),)+
                }
            }

            pub(super) fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
                match self {
                    $(Self::$variant(state) => state.data_for_bar(bar),)+
                }
            }

            pub(super) fn snapshot(&self) -> IndicatorRuntimeSnapshot {
                match self {
                    $(Self::$variant(state) => state.snapshot(),)+
                }
            }
        }

        const ALL_INDICATORS: [KlineIndicator; define_indicator_catalog!(@count $($variant),+)] = [
            $(KlineIndicator::$variant),+
        ];
        const SPOT_INDICATORS: [KlineIndicator; define_indicator_catalog!(@count $($spot),+)] = [
            $(KlineIndicator::$spot),+
        ];
        const PERPS_INDICATORS: [KlineIndicator; define_indicator_catalog!(@count $($perps),+)] = [
            $(KlineIndicator::$perps),+
        ];

        pub fn all_indicators() -> &'static [KlineIndicator] {
            &ALL_INDICATORS
        }

        pub fn indicators_for_market(market: MarketKind) -> &'static [KlineIndicator] {
            match market {
                MarketKind::Spot => &SPOT_INDICATORS,
                MarketKind::LinearPerps | MarketKind::InversePerps => &PERPS_INDICATORS,
            }
        }

        pub fn is_supported_for_market(indicator: KlineIndicator, market: MarketKind) -> bool {
            indicators_for_market(market).contains(&indicator)
        }

        pub fn requires_trade_data(indicator: KlineIndicator) -> bool {
            input_requirements(indicator).contains(&IndicatorInputSource::Trades)
        }

        pub fn display_name(indicator: KlineIndicator) -> &'static str {
            match indicator {
                $(
                    KlineIndicator::$variant =>
                        $display_name,
                )+
            }
        }

        pub fn panel_recipe(indicator: KlineIndicator) -> IndicatorPanelRecipe {
            match indicator {
                $(
                    KlineIndicator::$variant =>
                        $panel_recipe,
                )+
            }
        }

        pub fn input_requirements(indicator: KlineIndicator) -> &'static [IndicatorInputSource] {
            match indicator {
                $(KlineIndicator::$variant => $inputs,)+
            }
        }

        pub fn color_specs(
            indicator: KlineIndicator,
        ) -> &'static [super::super::IndicatorColorSpec] {
            match indicator {
                $(KlineIndicator::$variant => &$colors,)+
            }
        }

        pub fn mark_specs(
            indicator: KlineIndicator,
        ) -> &'static [super::super::IndicatorMarkSpec] {
            match indicator {
                $(KlineIndicator::$variant => &$marks,)+
            }
        }

        pub fn overlay_channels(
            indicator: KlineIndicator,
        ) -> &'static [super::IndicatorOverlayChannelSpec] {
            match indicator {
                $(KlineIndicator::$variant => &$channels,)+
            }
        }

        pub fn table_columns(indicator: KlineIndicator) -> &'static [super::TableColumnSpec] {
            match panel_recipe(indicator) {
                IndicatorPanelRecipe::Table { columns, .. } => columns,
                _ => &[],
            }
        }

        pub fn kline_warmup_bars(indicator: KlineIndicator) -> Option<u64> {
            match indicator {
                $(
                    KlineIndicator::$variant =>
                        <$state as super::runtime::IndicatorDefinition>::warmup_bars(
                            <$state as super::runtime::IndicatorDefinition>::Config::default(),
                        ),
                )+
            }
        }

        pub fn availability(
            indicator: KlineIndicator,
            context: AvailabilityContext,
            base_series_data: Option<&IndicatorRuntimeSet>,
        ) -> IndicatorAvailability {
            if !is_supported_for_market(indicator, context.base_ticker.market_type()) {
                return super::IndicatorAvailability::Unsupported(
                    super::IndicatorUnsupportedReason::SourceNotSupported,
                );
            }

            let snapshot = base_series_data
                .map(|data| data.snapshot_for(indicator))
                .unwrap_or_default();

            match indicator {
                $(
                    KlineIndicator::$variant =>
                        <$state as super::runtime::IndicatorDefinition>::availability(
                            context,
                            snapshot,
                        ),
                )+
            }
        }

    };
    (@count $($item:ident),+) => {
        <[()]>::len(&[$(define_indicator_catalog!(@replace $item ())),+])
    };
    (@replace $_item:ident $value:expr) => {$value};
    (@config unit $configs:ident) => {()};
    (@config rsi $configs:ident) => {$configs.rsi_config()};
    (@config moving_averages $configs:ident) => {
        $configs.moving_averages_calculation_config()
    };
    (@config bollinger_bands $configs:ident) => {
        $configs.bollinger_bands_calculation_config()
    };
    (@config anchored_vwap $configs:ident) => {$configs.anchored_vwap_config()};
}

define_indicator_catalog! {
    spot: [
        Volume,
        BollingerBands,
        MovingAverages,
        Rsi,
        AnchoredVwap,
        CumulativeVolumeDelta,
        TradeCount,
        BarAnalysis,
    ],
    perps: [
        Volume,
        BollingerBands,
        MovingAverages,
        Rsi,
        AnchoredVwap,
        OpenInterest,
        CumulativeVolumeDelta,
        TradeCount,
        BarAnalysis,
    ],
    entries: {
        Volume => {
            state: super::volume::VolumeState,
            display_name: "Volume",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "Volume",
                    layer_name: "Volume",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Histogram,
                    mark: MarkKind::Bar(
                        BarMode::Histogram(
                            HistogramMode::SignedOverlay,
                        ),
                    ),
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::BaseTickerMinQty,
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Abbreviated,
                        header_mode: PanelValueLabelMode::Commas,
                        max_decimals: None,
                    },
                    preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
                }
            },
            inputs: KLINE_INPUTS,
            config: unit,
            colors: super::volume::COLOR_SPECS,
            marks: super::volume::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS
        },
        BollingerBands => {
            state: super::bollinger::BollingerBandsState,
            display_name: "Bollinger Bands",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Bollinger Basis",
                    source: DataSourceId::Primary,
                    value_id: PanelValueId::BollingerBands,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: bollinger_bands,
            colors: super::bollinger::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::bollinger::OVERLAY_CHANNELS
        },
        MovingAverages => {
            state: super::moving_averages::MovingAveragesState,
            display_name: "Moving Averages",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Moving Averages",
                    source: DataSourceId::Primary,
                    value_id: PanelValueId::MovingAverages,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: moving_averages,
            colors: super::moving_averages::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::moving_averages::OVERLAY_CHANNELS
        },
        Rsi => {
            state: super::rsi::RsiState,
            display_name: "RSI",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "RSI",
                    layer_name: "RSI",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::FixedPower10(
                        exchange::unit::Power10::new(-2),
                    ),
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Compact,
                        header_mode: PanelValueLabelMode::Compact,
                        max_decimals: Some(2),
                    },
                    preferred_scale: PanelScaleMode::FitVisible,
                }
            },
            inputs: KLINE_INPUTS,
            config: rsi,
            colors: super::rsi::COLOR_SPECS,
            marks: super::rsi::MARK_SPECS,
            channels: super::rsi::OVERLAY_CHANNELS
        },
        AnchoredVwap => {
            state: super::anchored_vwap::AnchoredVwapState,
            display_name: "Anchored VWAP",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Anchored VWAP",
                    source: DataSourceId::Primary,
                    value_id: PanelValueId::AnchoredVwap,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: anchored_vwap,
            colors: super::anchored_vwap::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::anchored_vwap::OVERLAY_CHANNELS
        },
        OpenInterest => {
            state: super::open_interest::OpenInterestState,
            display_name: "Open Interest",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "Open Interest",
                    layer_name: "Open Interest",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::BaseTickerMinQty,
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Abbreviated,
                        header_mode: PanelValueLabelMode::Commas,
                        max_decimals: None,
                    },
                    preferred_scale: PanelScaleMode::FitVisible,
                }
            },
            inputs: OPEN_INTEREST_INPUTS,
            config: unit,
            colors: super::open_interest::COLOR_SPECS,
            marks: super::open_interest::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS
        },
        CumulativeVolumeDelta => {
            state: super::cvd::CumulativeVolumeDeltaState,
            display_name: "CVD",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "CVD",
                    layer_name: "Cumulative Volume Delta",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::BaseTickerMinQty,
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Abbreviated,
                        header_mode: PanelValueLabelMode::Commas,
                        max_decimals: None,
                    },
                    preferred_scale: PanelScaleMode::FitVisible,
                }
            },
            inputs: CVD_INPUTS,
            config: unit,
            colors: super::cvd::COLOR_SPECS,
            marks: super::cvd::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS
        },
        TradeCount => {
            state: super::trade_count::TradeCountState,
            display_name: "Trade Count",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "Trade Count",
                    layer_name: "Trades",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Histogram,
                    mark: MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::FixedStep(1.0),
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Abbreviated,
                        header_mode: PanelValueLabelMode::Commas,
                        max_decimals: Some(0),
                    },
                    preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
                }
            },
            inputs: TRADE_INPUTS,
            config: unit,
            colors: super::trade_count::COLOR_SPECS,
            marks: super::trade_count::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS
        },
        BarAnalysis => {
            state: super::bar_analysis::BarAnalysisState,
            display_name: "Bar Analysis",
            panel_recipe: {
                IndicatorPanelRecipe::Table {
                    panel_title: "Bar Analysis",
                    source: DataSourceId::Primary,
                    columns: super::bar_analysis::TABLE_COLUMNS,
                }
            },
            inputs: TRADE_INPUTS,
            config: unit,
            colors: super::bar_analysis::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS
        },
    }
}
