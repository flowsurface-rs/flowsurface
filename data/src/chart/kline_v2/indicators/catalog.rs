use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::{
    CVD_INPUTS, IndicatorInput, IndicatorInputSource, KLINE_INPUTS, OPEN_INTEREST_INPUTS,
};
use super::runtime::IndicatorRuntime;
use super::{
    AvailabilityContext, IndicatorAvailability, IndicatorConfigValue, IndicatorPanelRecipe,
    IndicatorRuntimeSet, IndicatorRuntimeSnapshot, IndicatorSetting, IndicatorSettingValue,
    IndicatorSettingsContext,
};
use crate::chart::Basis;
use crate::chart::kline_v2::composition::{DataSourceId, PanelValuePrecision};
use crate::chart::kline_v2::indicators::input::TRADE_INPUTS;
use crate::chart::kline_v2::{
    AxisBinding, BarMode, HistogramMode, LayerDataKind, MarkKind, PanelScaleMode,
    PanelValueLabelMode, PanelValueLabelPolicy,
};
use enum_map::{Enum, EnumMap};
use exchange::adapter::MarketKind;
use exchange::{Kline, UnixMs};

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
                    config: $config:ty,
                    colors: $colors:path,
                    marks: $marks:path,
                    channels: $channels:path,
                    fills: $fills:path,
                    uses_directional_volume: $uses_directional_volume:expr
                }
            ),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, Enum)]
        pub enum KlineIndicator {
            $($variant),+
        }

        #[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
        pub enum IndicatorConfig {
            $($variant($config)),+
        }

        impl Default for IndicatorConfig {
            fn default() -> Self {
                define_indicator_catalog!(@default_config $($variant: $config),+)
            }
        }

        impl IndicatorConfig {
            pub fn indicator(self) -> KlineIndicator {
                match self {
                    $(Self::$variant(_) => KlineIndicator::$variant),+
                }
            }

            pub fn normalized(self) -> Self {
                match self {
                    $(Self::$variant(config) => Self::$variant(
                        < $config as IndicatorConfigValue >::normalized(config)
                    )),+
                }
            }

            pub fn normalized_for_basis(self, basis: Basis) -> Self {
                match self {
                    $(Self::$variant(config) => Self::$variant(
                        < $config as IndicatorConfigValue >::normalized_for_basis(config, basis)
                    )),+
                }
            }

            pub fn history_requirement(
                self,
                basis: Basis,
                reference_time: UnixMs,
            ) -> super::IndicatorHistoryRequirement {
                match self {
                    $(Self::$variant(config) =>
                        < $config as IndicatorConfigValue >::history_requirement(
                            config,
                            basis,
                            reference_time,
                        )
                    ),+
                }
            }

            pub fn settings(self, context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
                match self {
                    $(Self::$variant(config) => < $config as IndicatorConfigValue >::settings(
                        config,
                        context,
                    )),+
                }
            }

            pub fn with_setting(
                self,
                id: &'static str,
                value: IndicatorSettingValue,
            ) -> Option<Self> {
                match self {
                    $(Self::$variant(config) =>
                        < $config as IndicatorConfigValue >::with_setting(config, id, value)
                            .map(Self::$variant)
                    ),+
                }
            }

            pub fn default_for(indicator: KlineIndicator) -> Self {
                match indicator {
                    $(KlineIndicator::$variant => Self::$variant(
                        < $config as Default >::default()
                    )),+
                }
            }

            pub fn warmup_bars(self) -> Option<u64> {
                match self {
                    $(Self::$variant(config) =>
                        <$state as super::runtime::IndicatorDefinition>::warmup_bars(config)
                    ),+
                }
            }
        }

        #[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
        pub struct IndicatorConfigStore {
            configs: EnumMap<KlineIndicator, IndicatorConfig>,
        }

        impl Default for IndicatorConfigStore {
            fn default() -> Self {
                let mut configs = EnumMap::default();
                $(configs[KlineIndicator::$variant] = IndicatorConfig::$variant(
                    < $config as Default >::default()
                );)+
                Self { configs }
            }
        }

        impl IndicatorConfigStore {
            pub fn normalized(mut self) -> Self {
                for indicator in all_indicators() {
                    self.configs[*indicator] = self.configs[*indicator].normalized();
                }
                self
            }

            pub fn normalized_for_basis(mut self, basis: Basis) -> Self {
                for indicator in all_indicators() {
                    self.configs[*indicator] = self.configs[*indicator].normalized_for_basis(basis);
                }
                self
            }

            pub fn config_for(&self, indicator: KlineIndicator) -> IndicatorConfig {
                self.configs[indicator]
            }

            pub fn update(&mut self, config: IndicatorConfig) -> bool {
                let config = config.normalized();
                let indicator = config.indicator();
                if self.configs[indicator] == config {
                    return false;
                }
                self.configs[indicator] = config;
                true
            }

            pub fn reset(&mut self, indicator: KlineIndicator) -> bool {
                self.update(IndicatorConfig::default_for(indicator))
            }

            pub fn warmup_bars(&self, indicator: KlineIndicator) -> Option<u64> {
                match self.config_for(indicator) {
                    $(IndicatorConfig::$variant(config) =>
                        < $state as super::runtime::IndicatorDefinition >::warmup_bars(config)
                    ),+
                }
            }

            pub fn history_requirement(
                &self,
                active: &[KlineIndicator],
                basis: Basis,
                reference_time: UnixMs,
            ) -> super::IndicatorHistoryRequirement {
                active.iter().fold(
                    super::IndicatorHistoryRequirement::default(),
                    |requirement, indicator| {
                        requirement.merge(
                            self.config_for(*indicator)
                                .history_requirement(basis, reference_time),
                        )
                    },
                )
            }
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
                config: IndicatorConfig,
                scope: IndicatorUpdateScope,
            ) {
                match self {
                    $(Self::$variant(state) => {
                        let IndicatorConfig::$variant(config) = config
                        else {
                            unreachable!("indicator configuration must match its runtime state");
                        };
                        state.on_input_from(input, config, scope);
                    }),+
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

        pub fn uses_directional_volume(indicator: KlineIndicator) -> bool {
            match indicator {
                $(KlineIndicator::$variant => $uses_directional_volume,)+
            }
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

        pub fn input_source_category(indicator: KlineIndicator) -> &'static str {
            let inputs = input_requirements(indicator);
            let has_klines = inputs.contains(&IndicatorInputSource::Klines);
            let has_trades = inputs.contains(&IndicatorInputSource::Trades);
            let has_open_interest = inputs.contains(&IndicatorInputSource::OpenInterest);

            match (has_klines, has_trades, has_open_interest) {
                (true, false, false) => "Kline data",
                (false, true, false) => "Trade data",
                (false, false, true) => "Open interest data",
                (true, true, false) => "Kline + trade data",
                (true, false, true) => "Kline + open interest data",
                (false, true, true) => "Trade + open interest data",
                (true, true, true) => "Kline + trade + open interest data",
                (false, false, false) => "Other",
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

        pub fn fill_specs(
            indicator: KlineIndicator,
        ) -> &'static [super::IndicatorFillSpec] {
            match indicator {
                $(KlineIndicator::$variant => &$fills,)+
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
                            <$config as Default>::default(),
                        ),
                )+
            }
        }

        pub fn availability(
            instance_id: super::super::IndicatorInstanceId,
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
                .map(|data| data.snapshot_for(instance_id))
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
    (@default_config $variant:ident: $config:ty $(, $rest_variant:ident: $rest_config:ty)*) => {
        Self::$variant(<$config as Default>::default())
    };
}

define_indicator_catalog! {
    spot: [
        Volume,
        BollingerBands,
        MovingAverages,
        Rsi,
        StochRsi,
        Atr,
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
        StochRsi,
        Atr,
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
            config: (),
            colors: super::volume::COLOR_SPECS,
            marks: super::volume::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: true
        },
        BollingerBands => {
            state: super::bollinger::BollingerBandsState,
            display_name: "Bollinger Bands",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Bollinger Basis",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: super::bollinger::BollingerBandsCalculationConfig,
            colors: super::bollinger::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::bollinger::OVERLAY_CHANNELS,
            fills: super::bollinger::FILL_SPECS,
            uses_directional_volume: false
        },
        MovingAverages => {
            state: super::moving_averages::MovingAveragesState,
            display_name: "Moving Averages",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Moving Averages",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: super::moving_averages::MovingAveragesCalculationConfig,
            colors: super::moving_averages::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::moving_averages::OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
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
            config: super::rsi::RsiConfig,
            colors: super::rsi::COLOR_SPECS,
            marks: super::rsi::MARK_SPECS,
            channels: super::rsi::OVERLAY_CHANNELS,
            fills: super::rsi::FILL_SPECS,
            uses_directional_volume: false
        },
        StochRsi => {
            state: super::stoch_rsi::StochRsiState,
            display_name: "Stoch RSI",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "Stoch RSI",
                    layer_name: "Stoch RSI",
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
            config: super::stoch_rsi::StochRsiConfig,
            colors: super::stoch_rsi::COLOR_SPECS,
            marks: super::stoch_rsi::MARK_SPECS,
            channels: super::stoch_rsi::OVERLAY_CHANNELS,
            fills: super::stoch_rsi::FILL_SPECS,
            uses_directional_volume: false
        },
        Atr => {
            state: super::atr::AtrState,
            display_name: "ATR",
            panel_recipe: {
                IndicatorPanelRecipe::Plot {
                    panel_title: "ATR",
                    layer_name: "Average True Range",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Secondary,
                    value_precision: PanelValuePrecision::BaseTickerMinTick,
                    value_label_policy: PanelValueLabelPolicy {
                        axis_mode: PanelValueLabelMode::Compact,
                        header_mode: PanelValueLabelMode::Compact,
                        max_decimals: None,
                    },
                    preferred_scale: PanelScaleMode::FitVisibleIncludeZero,
                }
            },
            inputs: KLINE_INPUTS,
            config: super::atr::AtrConfig,
            colors: super::atr::COLOR_SPECS,
            marks: super::atr::MARK_SPECS,
            channels: super::atr::OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
        },
        AnchoredVwap => {
            state: super::anchored_vwap::AnchoredVwapState,
            display_name: "Anchored VWAP",
            panel_recipe: {
                IndicatorPanelRecipe::PrimaryOverlay {
                    layer_name: "Anchored VWAP",
                    source: DataSourceId::Primary,
                    data_kind: LayerDataKind::Scalar,
                    mark: MarkKind::Line,
                    axis: AxisBinding::Primary,
                }
            },
            inputs: KLINE_INPUTS,
            config: super::anchored_vwap::AnchoredVwapConfig,
            colors: super::anchored_vwap::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::anchored_vwap::OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
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
            config: (),
            colors: super::open_interest::COLOR_SPECS,
            marks: super::open_interest::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
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
            config: (),
            colors: super::cvd::COLOR_SPECS,
            marks: super::cvd::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
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
            config: (),
            colors: super::trade_count::COLOR_SPECS,
            marks: super::trade_count::MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
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
            config: (),
            colors: super::bar_analysis::COLOR_SPECS,
            marks: super::NO_MARK_SPECS,
            channels: super::NO_OVERLAY_CHANNELS,
            fills: super::NO_FILL_SPECS,
            uses_directional_volume: false
        },
    }
}
