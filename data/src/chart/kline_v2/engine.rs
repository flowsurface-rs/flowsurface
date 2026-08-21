use super::IndicatorUpdateScope;
use super::KlineIndicator;
use super::indicators::IndicatorInput;
use super::indicators::{
    AnchoredVwapConfig, BollingerBandsCalculationConfig, MovingAveragesCalculationConfig, RsiConfig,
};
use super::series::KlineSeries;
use crate::chart::Basis;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum IndicatorConfig {
    Rsi(RsiConfig),
    MovingAverages(MovingAveragesCalculationConfig),
    BollingerBands(BollingerBandsCalculationConfig),
    AnchoredVwap(AnchoredVwapConfig),
}

impl IndicatorConfig {
    pub fn normalized(self) -> Self {
        match self {
            Self::Rsi(config) => Self::Rsi(config.normalized()),
            Self::MovingAverages(config) => Self::MovingAverages(config.normalized()),
            Self::BollingerBands(config) => Self::BollingerBands(config.normalized()),
            Self::AnchoredVwap(config) => Self::AnchoredVwap(config),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorConfigStore {
    rsi: RsiConfig,
    moving_averages: MovingAveragesCalculationConfig,
    bollinger_bands: BollingerBandsCalculationConfig,
    anchored_vwap: AnchoredVwapConfig,
}

impl Default for IndicatorConfigStore {
    fn default() -> Self {
        Self {
            rsi: RsiConfig::default().normalized(),
            moving_averages: MovingAveragesCalculationConfig::default().normalized(),
            bollinger_bands: BollingerBandsCalculationConfig::default().normalized(),
            anchored_vwap: AnchoredVwapConfig::default(),
        }
    }
}

impl IndicatorConfigStore {
    pub fn config_for(&self, indicator: KlineIndicator) -> Option<IndicatorConfig> {
        match indicator {
            KlineIndicator::Rsi => Some(IndicatorConfig::Rsi(self.rsi)),
            KlineIndicator::MovingAverages => {
                Some(IndicatorConfig::MovingAverages(self.moving_averages))
            }
            KlineIndicator::BollingerBands => {
                Some(IndicatorConfig::BollingerBands(self.bollinger_bands))
            }
            KlineIndicator::AnchoredVwap => Some(IndicatorConfig::AnchoredVwap(self.anchored_vwap)),
            _ => None,
        }
    }

    pub fn update(&mut self, config: IndicatorConfig) -> bool {
        let config = config.normalized();
        match config {
            IndicatorConfig::Rsi(config) => replace_if_changed(&mut self.rsi, config),
            IndicatorConfig::MovingAverages(config) => {
                replace_if_changed(&mut self.moving_averages, config)
            }
            IndicatorConfig::BollingerBands(config) => {
                replace_if_changed(&mut self.bollinger_bands, config)
            }
            IndicatorConfig::AnchoredVwap(config) => {
                replace_if_changed(&mut self.anchored_vwap, config)
            }
        }
    }

    pub fn reset(&mut self, indicator: KlineIndicator) -> bool {
        let Some(default_config) = Self::default_config(indicator) else {
            return false;
        };
        self.update(default_config)
    }

    fn default_config(indicator: KlineIndicator) -> Option<IndicatorConfig> {
        match indicator {
            KlineIndicator::Rsi => Some(IndicatorConfig::Rsi(RsiConfig::default().normalized())),
            KlineIndicator::MovingAverages => Some(IndicatorConfig::MovingAverages(
                MovingAveragesCalculationConfig::default().normalized(),
            )),
            KlineIndicator::BollingerBands => Some(IndicatorConfig::BollingerBands(
                BollingerBandsCalculationConfig::default().normalized(),
            )),
            KlineIndicator::AnchoredVwap => {
                Some(IndicatorConfig::AnchoredVwap(AnchoredVwapConfig::default()))
            }
            _ => None,
        }
    }

    pub fn warmup_bars(&self, indicator: KlineIndicator) -> Option<u64> {
        match indicator {
            KlineIndicator::Rsi => Some(self.rsi.warmup_bars()),
            KlineIndicator::MovingAverages => Some(self.moving_averages.warmup_bars()),
            KlineIndicator::BollingerBands => Some(self.bollinger_bands.warmup_bars()),
            _ => super::indicators::kline_warmup_bars(indicator),
        }
    }

    pub fn rsi_config(&self) -> RsiConfig {
        self.rsi
    }

    pub fn moving_averages_calculation_config(&self) -> MovingAveragesCalculationConfig {
        self.moving_averages
    }

    pub fn bollinger_bands_calculation_config(&self) -> BollingerBandsCalculationConfig {
        self.bollinger_bands
    }

    pub fn anchored_vwap_config(&self) -> AnchoredVwapConfig {
        self.anchored_vwap
    }
}

fn replace_if_changed<T: PartialEq>(current: &mut T, replacement: T) -> bool {
    if *current == replacement {
        return false;
    }
    *current = replacement;
    true
}

#[derive(Debug, Clone, Default)]
pub struct IndicatorEngine {
    configs: IndicatorConfigStore,
}

impl IndicatorEngine {
    pub fn configs(&self) -> &IndicatorConfigStore {
        &self.configs
    }

    pub fn config_for(&self, indicator: KlineIndicator) -> Option<IndicatorConfig> {
        self.configs.config_for(indicator)
    }

    pub fn update_config(&mut self, config: IndicatorConfig) -> bool {
        self.configs.update(config)
    }

    pub fn reset_config(&mut self, indicator: KlineIndicator) -> bool {
        self.configs.reset(indicator)
    }

    pub fn rsi_config(&self) -> RsiConfig {
        self.configs.rsi_config()
    }

    pub fn moving_averages_calculation_config(&self) -> MovingAveragesCalculationConfig {
        self.configs.moving_averages_calculation_config()
    }

    pub fn bollinger_bands_calculation_config(&self) -> BollingerBandsCalculationConfig {
        self.configs.bollinger_bands_calculation_config()
    }

    pub fn anchored_vwap_config(&self) -> AnchoredVwapConfig {
        self.configs.anchored_vwap_config()
    }

    pub fn configure_series(&self, series: &mut KlineSeries, basis: Basis) {
        series.refresh_indicator_inputs(&self.configs);
        let trade_bars = series.normalized_trade_bars(basis);
        series.insert_indicator_input(IndicatorInput::Trades { bars: &trade_bars }, &self.configs);
    }

    pub fn update_series_after_trades(
        &self,
        series: &mut KlineSeries,
        basis: Basis,
        scope: IndicatorUpdateScope,
    ) {
        if matches!(basis, Basis::Tick(_)) {
            series.refresh_indicator_inputs_from(&self.configs, scope);
        }

        let trade_bars = series.normalized_trade_bars(basis);
        series.insert_indicator_input_from(
            IndicatorInput::Trades { bars: &trade_bars },
            &self.configs,
            scope,
        );
    }

    pub fn update_series_after_klines(
        &self,
        series: &mut KlineSeries,
        scope: IndicatorUpdateScope,
    ) {
        series.refresh_indicator_inputs_from(&self.configs, scope);
    }
}
