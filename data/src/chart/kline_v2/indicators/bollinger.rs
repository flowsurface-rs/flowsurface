use super::super::IndicatorUpdateScope;
use super::super::values::{BOLLINGER_LOWER_FIELD_KEY, BOLLINGER_UPPER_FIELD_KEY, IndicatorData};
use super::moving_average::MovingAverageMethod;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfigValue,
    IndicatorFillGeometry, IndicatorFillSpec, IndicatorOverlayChannelSpec,
    IndicatorOverlayColorRole, IndicatorOverlaySource, IndicatorSetting, IndicatorSettingChoice,
    IndicatorSettingControl, IndicatorSettingValue, IndicatorSettingsContext,
};
use exchange::{Kline, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

const SERIES_MAX_POINTS: usize = 5000;

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 4] = [
    IndicatorColorSpec {
        label: "Basis",
        slot: IndicatorColorSlot::Main,
    },
    IndicatorColorSpec {
        label: "Upper band",
        slot: IndicatorColorSlot::Upper,
    },
    IndicatorColorSpec {
        label: "Lower band",
        slot: IndicatorColorSlot::Lower,
    },
    IndicatorColorSpec {
        label: "Background fill",
        slot: IndicatorColorSlot::Fill,
    },
];

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 3] = [
    IndicatorOverlayChannelSpec {
        label: "BB",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.1,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Main,
    },
    IndicatorOverlayChannelSpec {
        label: "U",
        source: IndicatorOverlaySource::Field(BOLLINGER_UPPER_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Success,
        color_slot: IndicatorColorSlot::Upper,
    },
    IndicatorOverlayChannelSpec {
        label: "L",
        source: IndicatorOverlaySource::Field(BOLLINGER_LOWER_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Danger,
        color_slot: IndicatorColorSlot::Lower,
    },
];

pub(super) const FILL_SPECS: [IndicatorFillSpec; 1] = [IndicatorFillSpec {
    upper: IndicatorOverlaySource::Field(BOLLINGER_UPPER_FIELD_KEY),
    lower: IndicatorOverlaySource::Field(BOLLINGER_LOWER_FIELD_KEY),
    geometry: IndicatorFillGeometry::SeriesBand,
    color_slot: IndicatorColorSlot::Fill,
    opacity: 0.12,
}];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BollingerBandsCalculationConfig {
    pub period: u16,
    pub method: MovingAverageMethod,
    pub std_dev_multiplier: f32,
}

impl Default for BollingerBandsCalculationConfig {
    fn default() -> Self {
        Self {
            period: 20,
            method: MovingAverageMethod::Sma,
            std_dev_multiplier: 2.0,
        }
    }
}

impl BollingerBandsCalculationConfig {
    pub fn normalized(self) -> Self {
        let std_dev_multiplier = if self.std_dev_multiplier.is_finite() {
            self.std_dev_multiplier.max(0.0)
        } else {
            Self::default().std_dev_multiplier
        };

        Self {
            period: self.period.max(2),
            std_dev_multiplier,
            ..self
        }
    }

    pub fn warmup_bars(self) -> u64 {
        u64::from(self.normalized().period)
    }
}

impl IndicatorConfigValue for BollingerBandsCalculationConfig {
    fn normalized(self) -> Self {
        self.normalized()
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        vec![
            IndicatorSetting {
                id: "period",
                label: "Period",
                value: IndicatorSettingValue::U16(self.period),
                control: IndicatorSettingControl::SliderU16 { min: 2, max: 200 },
            },
            IndicatorSetting {
                id: "method",
                label: "Average method",
                value: IndicatorSettingValue::Choice(moving_average_method_id(self.method)),
                control: IndicatorSettingControl::Choices(moving_average_method_choices()),
            },
            IndicatorSetting {
                id: "std_dev_multiplier",
                label: "Std. deviation",
                value: IndicatorSettingValue::F32(self.std_dev_multiplier),
                control: IndicatorSettingControl::SliderF32 {
                    min: 0.0,
                    max: 5.0,
                    step: 0.1,
                },
            },
        ]
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("period", IndicatorSettingValue::U16(period)) => Some(Self { period, ..self }),
            ("method", IndicatorSettingValue::Choice(value)) => Some(Self {
                method: moving_average_method_from_id(value)?,
                ..self
            }),
            ("std_dev_multiplier", IndicatorSettingValue::F32(std_dev_multiplier)) => Some(Self {
                std_dev_multiplier,
                ..self
            }),
            _ => None,
        }
    }
}

fn moving_average_method_choices() -> Vec<IndicatorSettingChoice> {
    vec![
        IndicatorSettingChoice {
            id: "sma",
            label: "SMA",
        },
        IndicatorSettingChoice {
            id: "ema",
            label: "EMA",
        },
        IndicatorSettingChoice {
            id: "vwma",
            label: "VWMA",
        },
    ]
}

fn moving_average_method_id(method: MovingAverageMethod) -> &'static str {
    match method {
        MovingAverageMethod::Sma => "sma",
        MovingAverageMethod::Ema => "ema",
        MovingAverageMethod::Vwma => "vwma",
    }
}

fn moving_average_method_from_id(id: &str) -> Option<MovingAverageMethod> {
    match id {
        "sma" => Some(MovingAverageMethod::Sma),
        "ema" => Some(MovingAverageMethod::Ema),
        "vwma" => Some(MovingAverageMethod::Vwma),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerBandsPoint {
    pub basis: f32,
    pub upper: f32,
    pub lower: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BollingerBandsState {
    values: BTreeMap<UnixMs, BollingerBandsPoint>,
}

impl BollingerBandsState {
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn recompute_from_bars(&mut self, bars: &[Kline], config: BollingerBandsCalculationConfig) {
        self.values.clear();

        let config = config.normalized();
        let period = usize::from(config.period);

        let mut window = VecDeque::with_capacity(period + 1);

        for bar in bars {
            let close = bar.close.to_f32_lossy();
            let volume = bar.volume.total().to_f32_lossy();
            window.push_back((close, volume));

            if window.len() > period {
                window.pop_front();
            }

            if window.len() == period {
                let values: Vec<f32> = window.iter().map(|(value, _)| *value).collect();
                let weights: Vec<f32> = window.iter().map(|(_, weight)| *weight).collect();
                let basis = config.method.calculate(&values, &weights).unwrap_or(close);
                let mean = values.iter().sum::<f32>() / values.len() as f32;
                let variance = values
                    .iter()
                    .map(|value| {
                        let difference = *value - mean;
                        difference * difference
                    })
                    .sum::<f32>()
                    / values.len() as f32;
                let std_dev = variance.sqrt();
                let delta = std_dev * config.std_dev_multiplier;

                self.values.insert(
                    bar.time,
                    BollingerBandsPoint {
                        basis,
                        upper: basis + delta,
                        lower: basis - delta,
                    },
                );
            }
        }

        self.trim();
    }

    pub fn recompute_from_bars_from(
        &mut self,
        bars: &[Kline],
        config: BollingerBandsCalculationConfig,
        first_index: usize,
    ) {
        if first_index == 0 {
            return self.recompute_from_bars(bars, config);
        }

        let config = config.normalized();
        let period = usize::from(config.period);
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.values.retain(|time, _| *time < first_bar.time);

        let window_start = first_index.saturating_sub(period.saturating_sub(1));
        let mut window = VecDeque::with_capacity(period + 1);

        for (index, bar) in bars.iter().enumerate().skip(window_start) {
            let close = bar.close.to_f32_lossy();
            let volume = bar.volume.total().to_f32_lossy();
            window.push_back((close, volume));

            if window.len() > period {
                window.pop_front();
            }

            if index >= first_index && window.len() == period {
                let values: Vec<f32> = window.iter().map(|(value, _)| *value).collect();
                let weights: Vec<f32> = window.iter().map(|(_, weight)| *weight).collect();
                let basis = config.method.calculate(&values, &weights).unwrap_or(close);
                let mean = values.iter().sum::<f32>() / values.len() as f32;
                let variance = values
                    .iter()
                    .map(|value| {
                        let difference = *value - mean;
                        difference * difference
                    })
                    .sum::<f32>()
                    / values.len() as f32;
                let delta = variance.sqrt() * config.std_dev_multiplier;

                self.values.insert(
                    bar.time,
                    BollingerBandsPoint {
                        basis,
                        upper: basis + delta,
                        lower: basis - delta,
                    },
                );
            }
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<BollingerBandsPoint> {
        self.values.get(&time).copied()
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

impl IndicatorDefinition for BollingerBandsState {
    type Config = BollingerBandsCalculationConfig;

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

impl IndicatorRuntime for BollingerBandsState {
    fn clear(&mut self) {
        BollingerBandsState::clear(self);
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
        let bands = self.value_at(bar.time)?;
        Some(
            IndicatorData::scalar(bands.basis)
                .with_field(BOLLINGER_UPPER_FIELD_KEY, bands.upper)
                .with_field(BOLLINGER_LOWER_FIELD_KEY, bands.lower),
        )
    }
}
