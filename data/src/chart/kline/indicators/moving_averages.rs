use super::super::IndicatorUpdateScope;
use super::super::values::{
    IndicatorData, MOVING_AVERAGES_MEDIUM_FIELD_KEY, MOVING_AVERAGES_SLOW_FIELD_KEY,
};
use super::input::IndicatorInput;
use super::moving_average::MovingAverageMethod;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfigValue,
    IndicatorOverlayChannelSpec, IndicatorOverlayColorRole, IndicatorOverlaySource,
    IndicatorSetting, IndicatorSettingChoice, IndicatorSettingControl, IndicatorSettingValue,
    IndicatorSettingsContext,
};
use exchange::{Kline, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 3] = [
    IndicatorColorSpec {
        label: "Fast",
        slot: IndicatorColorSlot::Fast,
    },
    IndicatorColorSpec {
        label: "Medium",
        slot: IndicatorColorSlot::Medium,
    },
    IndicatorColorSpec {
        label: "Slow",
        slot: IndicatorColorSlot::Slow,
    },
];

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 3] = [
    IndicatorOverlayChannelSpec {
        label: "F",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.1,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Fast,
    },
    IndicatorOverlayChannelSpec {
        label: "M",
        source: IndicatorOverlaySource::Field(MOVING_AVERAGES_MEDIUM_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Success,
        color_slot: IndicatorColorSlot::Medium,
    },
    IndicatorOverlayChannelSpec {
        label: "S",
        source: IndicatorOverlaySource::Field(MOVING_AVERAGES_SLOW_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Danger,
        color_slot: IndicatorColorSlot::Slow,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovingAveragesCalculationConfig {
    pub fast_period: u16,
    pub medium_period: u16,
    pub slow_period: u16,
    pub method: MovingAverageMethod,
}

impl Default for MovingAveragesCalculationConfig {
    fn default() -> Self {
        Self {
            fast_period: 50,
            medium_period: 150,
            slow_period: 250,
            method: MovingAverageMethod::Sma,
        }
    }
}

impl MovingAveragesCalculationConfig {
    pub fn normalized(self) -> Self {
        Self {
            fast_period: self.fast_period.max(1),
            medium_period: self.medium_period.max(1),
            slow_period: self.slow_period.max(1),
            method: self.method,
        }
    }

    pub fn periods(self) -> [u16; 3] {
        let config = self.normalized();
        [config.fast_period, config.medium_period, config.slow_period]
    }

    pub fn warmup_bars(self) -> u64 {
        self.periods().into_iter().map(u64::from).max().unwrap_or(1)
    }
}

impl IndicatorConfigValue for MovingAveragesCalculationConfig {
    fn normalized(self) -> Self {
        self.normalized()
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        vec![
            IndicatorSetting {
                id: "method",
                label: "Average method",
                group: None,
                value: IndicatorSettingValue::Choice(moving_average_method_id(self.method)),
                control: IndicatorSettingControl::Choices(moving_average_method_choices()),
            },
            IndicatorSetting {
                id: "fast_period",
                label: "Fast period",
                group: None,
                value: IndicatorSettingValue::U16(self.fast_period),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 500 },
            },
            IndicatorSetting {
                id: "medium_period",
                label: "Medium period",
                group: None,
                value: IndicatorSettingValue::U16(self.medium_period),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 500 },
            },
            IndicatorSetting {
                id: "slow_period",
                label: "Slow period",
                group: None,
                value: IndicatorSettingValue::U16(self.slow_period),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 500 },
            },
        ]
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("method", IndicatorSettingValue::Choice(value)) => Some(Self {
                method: moving_average_method_from_id(value)?,
                ..self
            }),
            ("fast_period", IndicatorSettingValue::U16(fast_period)) => Some(Self {
                fast_period,
                ..self
            }),
            ("medium_period", IndicatorSettingValue::U16(medium_period)) => Some(Self {
                medium_period,
                ..self
            }),
            ("slow_period", IndicatorSettingValue::U16(slow_period)) => Some(Self {
                slow_period,
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

#[derive(Debug, Clone, Default)]
pub struct MovingAveragesState {
    values: BTreeMap<UnixMs, [Option<f32>; 3]>,
}

impl MovingAveragesState {
    fn recompute_from(
        &mut self,
        bars: &[Kline],
        config: MovingAveragesCalculationConfig,
        first_index: usize,
    ) {
        if first_index == 0 {
            self.values.clear();
        } else {
            let Some(first_bar) = bars.get(first_index) else {
                return;
            };
            self.values.retain(|time, _| *time < first_bar.time);
        }

        let periods = config.periods().map(usize::from);
        for (bar_index, bar) in bars.iter().enumerate().skip(first_index) {
            let values =
                periods.map(|period| moving_average_at(bars, bar_index, period, config.method));
            self.values.insert(bar.time, values);
        }
    }
}

fn moving_average_at(
    bars: &[Kline],
    bar_index: usize,
    period: usize,
    method: MovingAverageMethod,
) -> Option<f32> {
    if period == 0 || bar_index + 1 < period {
        return None;
    }

    let window = &bars[(bar_index + 1 - period)..=bar_index];
    let values = window
        .iter()
        .map(|bar| bar.close.to_f32_lossy())
        .collect::<Vec<_>>();
    let weights = window
        .iter()
        .map(|bar| bar.volume.total().to_f32_lossy())
        .collect::<Vec<_>>();
    method.calculate(&values, &weights)
}

impl IndicatorDefinition for MovingAveragesState {
    type Config = MovingAveragesCalculationConfig;

    fn warmup_bars(config: Self::Config) -> Option<u64> {
        Some(config.warmup_bars())
    }

    fn availability(
        _: super::AvailabilityContext,
        _: super::IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        IndicatorAvailability::Available
    }
}

impl IndicatorRuntime for MovingAveragesState {
    fn clear(&mut self) {
        self.values.clear();
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], config: Self::Config) {
        self.recompute_from(bars, config, 0);
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        if let IndicatorInput::Klines { bars, .. } = input {
            self.recompute_from(bars, config, scope.first_index());
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        let [fast, medium, slow] = *self.values.get(&bar.time)?;
        let mut data = IndicatorData::scalar(fast?);
        if let Some(medium) = medium {
            data = data.with_field(MOVING_AVERAGES_MEDIUM_FIELD_KEY, medium);
        }
        if let Some(slow) = slow {
            data = data.with_field(MOVING_AVERAGES_SLOW_FIELD_KEY, slow);
        }
        Some(data)
    }
}
