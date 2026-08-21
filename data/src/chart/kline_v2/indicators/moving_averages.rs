use super::super::IndicatorUpdateScope;
use super::super::values::{
    IndicatorData, MOVING_AVERAGES_MEDIUM_FIELD_KEY, MOVING_AVERAGES_SLOW_FIELD_KEY,
};
use super::input::IndicatorInput;
use super::moving_average::MovingAverageMethod;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorOverlayChannelSpec,
    IndicatorOverlayColorRole, IndicatorOverlaySource,
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

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::Volume;
    use exchange::unit::Qty;

    fn bar(time: u64, close: f64) -> Kline {
        Kline::new(
            time,
            close,
            close,
            close,
            close,
            Volume::TotalOnly(Qty::from_f64(1.0)),
            exchange::unit::MinTicksize::new(-2),
        )
    }

    #[test]
    fn computes_all_configured_periods() {
        let bars = [bar(1, 10.0), bar(2, 20.0), bar(3, 30.0)];
        let config = MovingAveragesCalculationConfig {
            fast_period: 1,
            medium_period: 2,
            slow_period: 3,
            method: MovingAverageMethod::Sma,
        };
        let mut state = MovingAveragesState::default();

        state.rebuild_from_bars(&bars, config);

        let data = state
            .data_for_bar(&bars[2])
            .expect("third bar has all averages");
        assert_eq!(data.value(), 30.0);
        assert_eq!(data.field(MOVING_AVERAGES_MEDIUM_FIELD_KEY), Some(25.0));
        assert_eq!(data.field(MOVING_AVERAGES_SLOW_FIELD_KEY), Some(20.0));
    }
}
