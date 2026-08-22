use super::super::IndicatorUpdateScope;
use super::super::values::{IndicatorData, RSI_SIGNAL_FIELD_KEY};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfig,
    IndicatorConfigValue, IndicatorFillGeometry, IndicatorFillSpec, IndicatorMarkSpec,
    IndicatorOverlayChannelSpec, IndicatorOverlayColorRole, IndicatorOverlaySource,
    IndicatorSetting, IndicatorSettingControl, IndicatorSettingValue, IndicatorSettingsContext,
};
use crate::chart::kline_v2::composition::{BarMode, MarkKind};
use exchange::{Kline, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;

const COLOR_SLOTS: [IndicatorColorSlot; 5] = [
    IndicatorColorSlot::Main,
    IndicatorColorSlot::Fill,
    IndicatorColorSlot::Signal,
    IndicatorColorSlot::Upper,
    IndicatorColorSlot::Lower,
];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 5] = [
    IndicatorColorSpec {
        label: "RSI",
        slot: IndicatorColorSlot::Main,
    },
    IndicatorColorSpec {
        label: "Signal",
        slot: IndicatorColorSlot::Signal,
    },
    IndicatorColorSpec {
        label: "Background fill",
        slot: IndicatorColorSlot::Fill,
    },
    IndicatorColorSpec {
        label: "Overbought",
        slot: IndicatorColorSlot::Upper,
    },
    IndicatorColorSpec {
        label: "Oversold",
        slot: IndicatorColorSlot::Lower,
    },
];

pub(super) const MARK_SPECS: [IndicatorMarkSpec; 2] = [
    IndicatorMarkSpec {
        label: "Line",
        mark: MarkKind::Line,
        color_slots: &COLOR_SLOTS,
    },
    IndicatorMarkSpec {
        label: "Bars",
        mark: MarkKind::Bar(BarMode::Regular),
        color_slots: &COLOR_SLOTS,
    },
];

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 4] = [
    IndicatorOverlayChannelSpec {
        label: "RSI",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.2,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Main,
    },
    IndicatorOverlayChannelSpec {
        label: "S",
        source: IndicatorOverlaySource::Field(RSI_SIGNAL_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Primary,
        color_slot: IndicatorColorSlot::Signal,
    },
    IndicatorOverlayChannelSpec {
        label: "OB",
        source: IndicatorOverlaySource::Reference(rsi_upper_band),
        line_width: 0.9,
        color_role: IndicatorOverlayColorRole::Success,
        color_slot: IndicatorColorSlot::Upper,
    },
    IndicatorOverlayChannelSpec {
        label: "OS",
        source: IndicatorOverlaySource::Reference(rsi_lower_band),
        line_width: 0.9,
        color_role: IndicatorOverlayColorRole::Danger,
        color_slot: IndicatorColorSlot::Lower,
    },
];

pub(super) const FILL_SPECS: [IndicatorFillSpec; 1] = [IndicatorFillSpec {
    upper: IndicatorOverlaySource::Reference(rsi_upper_band),
    lower: IndicatorOverlaySource::Reference(rsi_lower_band),
    geometry: IndicatorFillGeometry::PanelBand,
    color_slot: IndicatorColorSlot::Fill,
    opacity: 0.12,
}];

fn rsi_upper_band(config: IndicatorConfig) -> f32 {
    let IndicatorConfig::Rsi(config) = config else {
        unreachable!("RSI must retain its own configuration");
    };
    config.upper_band
}

fn rsi_lower_band(config: IndicatorConfig) -> f32 {
    let IndicatorConfig::Rsi(config) = config else {
        unreachable!("RSI must retain its own configuration");
    };
    config.lower_band
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RsiSmoothing {
    None,
    Ema(u16),
}

impl Default for RsiSmoothing {
    fn default() -> Self {
        Self::Ema(9)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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

        let defaults = Self::default();
        let mut upper_band = if self.upper_band.is_finite() {
            self.upper_band.clamp(0.0, 100.0)
        } else {
            defaults.upper_band
        };
        let mut lower_band = if self.lower_band.is_finite() {
            self.lower_band.clamp(0.0, 100.0)
        } else {
            defaults.lower_band
        };
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

impl IndicatorConfigValue for RsiConfig {
    fn normalized(self) -> Self {
        self.normalized()
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        let mut settings = vec![
            IndicatorSetting {
                id: "period",
                label: "Period",
                value: IndicatorSettingValue::U16(self.period),
                control: IndicatorSettingControl::SliderU16 { min: 2, max: 100 },
            },
            IndicatorSetting {
                id: "ema_signal",
                label: "EMA signal",
                value: IndicatorSettingValue::Bool(matches!(self.smoothing, RsiSmoothing::Ema(_))),
                control: IndicatorSettingControl::Toggle,
            },
        ];
        if let RsiSmoothing::Ema(period) = self.smoothing {
            settings.push(IndicatorSetting {
                id: "ema_period",
                label: "EMA period",
                value: IndicatorSettingValue::U16(period),
                control: IndicatorSettingControl::SliderU16 { min: 2, max: 50 },
            });
        }
        settings.extend([
            IndicatorSetting {
                id: "upper_band",
                label: "Upper band",
                value: IndicatorSettingValue::F32(self.upper_band),
                control: IndicatorSettingControl::SliderF32 {
                    min: 1.0,
                    max: 99.0,
                    step: 1.0,
                },
            },
            IndicatorSetting {
                id: "lower_band",
                label: "Lower band",
                value: IndicatorSettingValue::F32(self.lower_band),
                control: IndicatorSettingControl::SliderF32 {
                    min: 1.0,
                    max: 99.0,
                    step: 1.0,
                },
            },
        ]);
        settings
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("period", IndicatorSettingValue::U16(period)) => Some(Self { period, ..self }),
            ("ema_signal", IndicatorSettingValue::Bool(enabled)) => Some(Self {
                smoothing: if enabled {
                    RsiSmoothing::Ema(match self.smoothing {
                        RsiSmoothing::Ema(period) => period.max(2),
                        RsiSmoothing::None => 9,
                    })
                } else {
                    RsiSmoothing::None
                },
                ..self
            }),
            ("ema_period", IndicatorSettingValue::U16(period)) => Some(Self {
                smoothing: RsiSmoothing::Ema(period),
                ..self
            }),
            ("upper_band", IndicatorSettingValue::F32(upper_band)) => {
                Some(Self { upper_band, ..self })
            }
            ("lower_band", IndicatorSettingValue::F32(lower_band)) => {
                Some(Self { lower_band, ..self })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RsiPoint {
    pub value: f32,
    pub signal: Option<f32>,
    avg_gain: f32,
    avg_loss: f32,
    signal_ema: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct RsiState {
    values: BTreeMap<UnixMs, RsiPoint>,
}

impl RsiState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub(crate) fn recompute_from_bars(&mut self, bars: &[Kline], config: RsiConfig) {
        self.values.clear();

        let config = config.normalized();
        let period = usize::from(config.period);

        if period < 2 || bars.len() <= period {
            return;
        }

        let closes: Vec<f32> = bars.iter().map(|bar| bar.close.to_f32_lossy()).collect();
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

        let mut raw_rsi_values: Vec<(UnixMs, f32, f32, f32)> =
            Vec::with_capacity(bars.len() - period);
        raw_rsi_values.push((
            bars[period].time,
            Self::compute_rsi(avg_gain, avg_loss),
            avg_gain,
            avg_loss,
        ));

        for idx in (period + 1)..bars.len() {
            let delta = closes[idx] - closes[idx - 1];
            let gain = delta.max(0.0);
            let loss = (-delta).max(0.0);

            avg_gain = ((avg_gain * (period_f - 1.0)) + gain) / period_f;
            avg_loss = ((avg_loss * (period_f - 1.0)) + loss) / period_f;

            raw_rsi_values.push((
                bars[idx].time,
                Self::compute_rsi(avg_gain, avg_loss),
                avg_gain,
                avg_loss,
            ));
        }

        let mut signal_ema: Option<f32> = None;
        let signal_period = match config.smoothing {
            RsiSmoothing::None => None,
            RsiSmoothing::Ema(period) => Some(usize::from(period)),
        };

        let alpha = signal_period
            .map(|period| 2.0 / (period as f32 + 1.0))
            .unwrap_or(0.0);

        for (idx, (time, rsi_value, point_avg_gain, point_avg_loss)) in
            raw_rsi_values.iter().copied().enumerate()
        {
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
                    avg_gain: point_avg_gain,
                    avg_loss: point_avg_loss,
                    signal_ema,
                },
            );
        }

        self.trim();
    }

    fn recompute_from_bars_from(&mut self, bars: &[Kline], config: RsiConfig, first_index: usize) {
        let config = config.normalized();
        let period = usize::from(config.period);
        if first_index <= period {
            return self.recompute_from_bars(bars, config);
        }

        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        let Some(previous) = bars
            .get(first_index - 1)
            .and_then(|bar| self.values.get(&bar.time).copied())
        else {
            return self.recompute_from_bars(bars, config);
        };

        self.values.retain(|time, _| *time < first_bar.time);

        let period_f = period as f32;
        let signal_period = match config.smoothing {
            RsiSmoothing::None => None,
            RsiSmoothing::Ema(period) => Some(usize::from(period)),
        };
        let alpha = signal_period
            .map(|period| 2.0 / (period as f32 + 1.0))
            .unwrap_or(0.0);
        let mut avg_gain = previous.avg_gain;
        let mut avg_loss = previous.avg_loss;
        let mut signal_ema = previous.signal_ema;

        for idx in first_index..bars.len() {
            let delta = bars[idx].close.to_f32_lossy() - bars[idx - 1].close.to_f32_lossy();
            let gain = delta.max(0.0);
            let loss = (-delta).max(0.0);

            avg_gain = ((avg_gain * (period_f - 1.0)) + gain) / period_f;
            avg_loss = ((avg_loss * (period_f - 1.0)) + loss) / period_f;
            let value = Self::compute_rsi(avg_gain, avg_loss);

            let signal = if let Some(period) = signal_period {
                signal_ema = Some(match signal_ema {
                    Some(previous) => previous + alpha * (value - previous),
                    None => value,
                });
                if idx - period + 1 >= period {
                    signal_ema
                } else {
                    None
                }
            } else {
                None
            };

            self.values.insert(
                bars[idx].time,
                RsiPoint {
                    value,
                    signal,
                    avg_gain,
                    avg_loss,
                    signal_ema,
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

impl IndicatorDefinition for RsiState {
    type Config = RsiConfig;

    fn warmup_bars(config: Self::Config) -> Option<u64> {
        Some(config.warmup_bars())
    }

    fn availability(
        _context: super::AvailabilityContext,
        _snapshot: super::IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for RsiState {
    fn clear(&mut self) {
        RsiState::clear(self);
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], config: Self::Config) {
        self.recompute_from_bars(bars, config);
    }

    fn on_input_from(
        &mut self,
        input: super::input::IndicatorInput<'_>,
        config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        let super::input::IndicatorInput::Klines { bars, .. } = input else {
            return;
        };
        self.recompute_from_bars_from(bars, config, scope.first_index());
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        let point = self.value_at(bar.time)?;
        let data = IndicatorData::scalar(point.value);
        Some(match point.signal {
            Some(signal) => data.with_field(RSI_SIGNAL_FIELD_KEY, signal),
            None => data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_calculation_and_band_settings_together() {
        let config = RsiConfig {
            period: 1,
            smoothing: RsiSmoothing::Ema(1),
            upper_band: 20.0,
            lower_band: 80.0,
        }
        .normalized();

        assert_eq!(config.period, 2);
        assert_eq!(config.smoothing, RsiSmoothing::None);
        assert_eq!(config.upper_band, 60.0);
        assert_eq!(config.lower_band, 40.0);
    }

    #[test]
    fn replaces_non_finite_bands_with_defaults() {
        let config = RsiConfig {
            upper_band: f32::NAN,
            lower_band: f32::INFINITY,
            ..RsiConfig::default()
        }
        .normalized();

        assert_eq!(config.upper_band, RsiConfig::default().upper_band);
        assert_eq!(config.lower_band, RsiConfig::default().lower_band);
    }
}
