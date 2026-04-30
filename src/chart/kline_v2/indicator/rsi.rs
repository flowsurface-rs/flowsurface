use super::IndicatorAvailability;
use super::IndicatorPanelRecipe;
use crate::widget::chart::kline::composition::{
    AxisBinding, DataSourceId, LayerDataKind, MarkKind, PanelScaleMode, PanelValueLabelMode,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use data::chart::Basis;
use exchange::unit::Power10;
use exchange::{Kline, UnixMs};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsiSmoothing {
    None,
    Ema(u16),
}

impl Default for RsiSmoothing {
    fn default() -> Self {
        Self::Ema(9)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RsiConfig {
    pub period: u16,
    pub smoothing: RsiSmoothing,
    pub upper_band: f32,
    pub lower_band: f32,
}

impl Default for RsiConfig {
    fn default() -> Self {
        Self {
            period: 14,
            smoothing: RsiSmoothing::default(),
            upper_band: 70.0,
            lower_band: 30.0,
        }
    }
}

impl RsiConfig {
    pub fn normalized(self) -> Self {
        let period = self.period.max(2);

        let smoothing = match self.smoothing {
            RsiSmoothing::None => RsiSmoothing::None,
            RsiSmoothing::Ema(period) => {
                if period <= 1 {
                    RsiSmoothing::None
                } else {
                    RsiSmoothing::Ema(period)
                }
            }
        };

        let mut upper_band = self.upper_band.clamp(0.0, 100.0);
        let mut lower_band = self.lower_band.clamp(0.0, 100.0);

        if upper_band <= lower_band {
            let midpoint = (upper_band + lower_band) * 0.5;
            lower_band = (midpoint - 10.0).clamp(0.0, 99.0);
            upper_band = (midpoint + 10.0).clamp(1.0, 100.0);
            if upper_band <= lower_band {
                upper_band = (lower_band + 1.0).min(100.0);
            }
        }

        Self {
            period,
            smoothing,
            upper_band,
            lower_band,
        }
    }

    pub fn warmup_bars(self) -> u64 {
        let config = self.normalized();
        let base = u64::from(config.period).saturating_add(1);

        let smoothing = match config.smoothing {
            RsiSmoothing::None => 0,
            RsiSmoothing::Ema(period) => u64::from(period),
        };

        base.saturating_add(smoothing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RsiPoint {
    pub value: f32,
    pub signal: Option<f32>,
    pub upper_band: f32,
    pub lower_band: f32,
}

#[derive(Debug, Clone)]
pub struct RsiState {
    config: RsiConfig,
    values: BTreeMap<UnixMs, RsiPoint>,
}

impl Default for RsiState {
    fn default() -> Self {
        Self {
            config: RsiConfig::default().normalized(),
            values: BTreeMap::new(),
        }
    }
}

impl RsiState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn set_config(&mut self, config: RsiConfig) -> bool {
        let normalized = config.normalized();
        if self.config == normalized {
            return false;
        }

        self.config = normalized;
        true
    }

    pub fn recompute_from_bars(&mut self, bars: &[Kline]) {
        self.values.clear();

        let config = self.config.normalized();
        let period = usize::from(config.period);

        if period < 2 || bars.len() <= period {
            return;
        }

        let closes: Vec<f32> = bars.iter().map(|bar| bar.close.to_f32()).collect();
        let mut initial_gains = 0.0f32;
        let mut initial_losses = 0.0f32;

        for idx in 1..=period {
            let delta = closes[idx] - closes[idx - 1];
            if delta >= 0.0 {
                initial_gains += delta;
            } else {
                initial_losses += -delta;
            }
        }

        let period_f = period as f32;
        let mut avg_gain = initial_gains / period_f;
        let mut avg_loss = initial_losses / period_f;

        let mut raw_rsi_values: Vec<(UnixMs, f32)> = Vec::with_capacity(bars.len() - period);
        raw_rsi_values.push((bars[period].time, Self::compute_rsi(avg_gain, avg_loss)));

        for idx in (period + 1)..bars.len() {
            let delta = closes[idx] - closes[idx - 1];
            let gain = delta.max(0.0);
            let loss = (-delta).max(0.0);

            avg_gain = ((avg_gain * (period_f - 1.0)) + gain) / period_f;
            avg_loss = ((avg_loss * (period_f - 1.0)) + loss) / period_f;

            raw_rsi_values.push((bars[idx].time, Self::compute_rsi(avg_gain, avg_loss)));
        }

        let mut signal_ema: Option<f32> = None;
        let signal_period = match config.smoothing {
            RsiSmoothing::None => None,
            RsiSmoothing::Ema(period) => Some(usize::from(period)),
        };

        let alpha = signal_period
            .map(|period| 2.0 / (period as f32 + 1.0))
            .unwrap_or(0.0);

        for (idx, (time, rsi_value)) in raw_rsi_values.iter().copied().enumerate() {
            let signal = if let Some(period) = signal_period {
                signal_ema = Some(match signal_ema {
                    Some(prev) => prev + alpha * (rsi_value - prev),
                    None => rsi_value,
                });

                if idx + 1 >= period { signal_ema } else { None }
            } else {
                None
            };

            self.values.insert(
                time,
                RsiPoint {
                    value: rsi_value,
                    signal,
                    upper_band: config.upper_band,
                    lower_band: config.lower_band,
                },
            );
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<RsiPoint> {
        self.values.get(&time).copied()
    }

    fn compute_rsi(avg_gain: f32, avg_loss: f32) -> f32 {
        let eps = 1e-6;

        if avg_gain <= eps && avg_loss <= eps {
            return 50.0;
        }

        if avg_loss <= eps {
            return 100.0;
        }

        if avg_gain <= eps {
            return 0.0;
        }

        let rs = avg_gain / avg_loss;
        (100.0 - (100.0 / (1.0 + rs))).clamp(0.0, 100.0)
    }

    fn trim(&mut self) {
        while self.values.len() > SERIES_MAX_POINTS {
            if let Some((&earliest, _)) = self.values.first_key_value() {
                self.values.remove(&earliest);
            } else {
                break;
            }
        }
    }
}

pub fn panel_recipe() -> IndicatorPanelRecipe {
    IndicatorPanelRecipe::AuxPanel {
        panel_title: "RSI",
        layer_name: "RSI",
        source: DataSourceId::Primary,
        data_kind: LayerDataKind::Scalar,
        mark: MarkKind::Line,
        axis: AxisBinding::Secondary,
        value_precision: PanelValuePrecision::FixedPower10(Power10::new(-2)),
        value_label_policy: PanelValueLabelPolicy {
            axis_mode: PanelValueLabelMode::Compact,
            header_mode: PanelValueLabelMode::Compact,
            max_decimals: Some(2),
        },
        preferred_scale: PanelScaleMode::FitVisible,
    }
}

pub fn kline_warmup_bars(config: RsiConfig) -> u64 {
    config.warmup_bars()
}

pub fn availability(_basis: Basis) -> IndicatorAvailability {
    IndicatorAvailability::Available
}
