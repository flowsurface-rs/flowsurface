use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::IndicatorInput;
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfigValue,
    IndicatorMarkSpec, IndicatorOverlayChannelSpec, IndicatorOverlayColorRole,
    IndicatorOverlaySource, IndicatorSetting, IndicatorSettingControl, IndicatorSettingValue,
    IndicatorSettingsContext,
};
use crate::chart::kline::composition::{BarMode, MarkKind};
use exchange::{Kline, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SERIES_MAX_POINTS: usize = 5000;
const COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Main];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 1] = [IndicatorColorSpec {
    label: "ATR",
    slot: IndicatorColorSlot::Main,
}];

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

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 1] =
    [IndicatorOverlayChannelSpec {
        label: "ATR",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.2,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Main,
    }];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AtrConfig {
    pub period: u16,
}

impl Default for AtrConfig {
    fn default() -> Self {
        Self { period: 14 }
    }
}

impl AtrConfig {
    pub fn normalized(self) -> Self {
        Self {
            period: self.period.max(1),
        }
    }

    pub fn warmup_bars(self) -> u64 {
        u64::from(self.normalized().period)
    }
}

impl IndicatorConfigValue for AtrConfig {
    fn normalized(self) -> Self {
        self.normalized()
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        vec![IndicatorSetting {
            id: "period",
            label: "Period",
            group: None,
            value: IndicatorSettingValue::U16(self.period),
            control: IndicatorSettingControl::SliderU16 { min: 1, max: 200 },
        }]
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("period", IndicatorSettingValue::U16(period)) => Some(Self { period }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AtrState {
    values: BTreeMap<UnixMs, f32>,
}

impl AtrState {
    pub(crate) fn recompute_from_bars(&mut self, bars: &[Kline], config: AtrConfig) {
        self.values.clear();

        let period = usize::from(config.normalized().period);
        if bars.len() < period {
            return;
        }

        let mut average = bars
            .iter()
            .enumerate()
            .take(period)
            .map(|(index, bar)| Self::true_range(bar, bars.get(index.saturating_sub(1))))
            .sum::<f32>()
            / period as f32;
        self.values.insert(bars[period - 1].time, average);

        for (index, bar) in bars.iter().enumerate().skip(period) {
            let range = Self::true_range(bar, bars.get(index - 1));
            average = ((average * (period - 1) as f32) + range) / period as f32;
            self.values.insert(bar.time, average);
        }

        self.trim();
    }

    pub(crate) fn recompute_from_bars_from(
        &mut self,
        bars: &[Kline],
        config: AtrConfig,
        first_index: usize,
    ) {
        let period = usize::from(config.normalized().period);
        if first_index < period {
            return self.recompute_from_bars(bars, config);
        }

        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        let Some(mut average) = bars
            .get(first_index - 1)
            .and_then(|bar| self.values.get(&bar.time).copied())
        else {
            return self.recompute_from_bars(bars, config);
        };

        self.values.retain(|time, _| *time < first_bar.time);
        for (index, bar) in bars.iter().enumerate().skip(first_index) {
            let range = Self::true_range(bar, bars.get(index - 1));
            average = ((average * (period - 1) as f32) + range) / period as f32;
            self.values.insert(bar.time, average);
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<f32> {
        self.values.get(&time).copied()
    }

    fn true_range(bar: &Kline, previous: Option<&Kline>) -> f32 {
        let high = bar.high.to_f32_lossy();
        let low = bar.low.to_f32_lossy();
        let range = high - low;
        let Some(previous) = previous else {
            return range;
        };

        let previous_close = previous.close.to_f32_lossy();
        range
            .max((high - previous_close).abs())
            .max((low - previous_close).abs())
    }

    fn trim(&mut self) {
        while self.values.len() > SERIES_MAX_POINTS {
            let Some((&earliest, _)) = self.values.first_key_value() else {
                break;
            };
            self.values.remove(&earliest);
        }
    }
}

impl IndicatorDefinition for AtrState {
    type Config = AtrConfig;

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

impl IndicatorRuntime for AtrState {
    fn clear(&mut self) {
        self.values.clear();
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], config: Self::Config) {
        self.recompute_from_bars(bars, config);
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        let IndicatorInput::Klines { bars, .. } = input else {
            return;
        };
        self.recompute_from_bars_from(bars, config, scope.first_index());
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.value_at(bar.time).map(IndicatorData::scalar)
    }
}
