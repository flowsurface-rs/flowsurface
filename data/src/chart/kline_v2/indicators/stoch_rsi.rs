use super::super::IndicatorUpdateScope;
use super::super::values::{IndicatorData, STOCH_RSI_D_FIELD_KEY};
use super::input::{IndicatorInput, KlinePriceSource};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorConfigValue,
    IndicatorMarkSpec, IndicatorOverlayChannelSpec, IndicatorOverlayColorRole,
    IndicatorOverlaySource, IndicatorSetting, IndicatorSettingChoice, IndicatorSettingControl,
    IndicatorSettingValue, IndicatorSettingsContext,
};
use crate::chart::kline_v2::composition::{BarMode, MarkKind};
use exchange::{Kline, UnixMs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

const SERIES_MAX_POINTS: usize = 5000;
const COLOR_SLOTS: [IndicatorColorSlot; 2] = [IndicatorColorSlot::Main, IndicatorColorSlot::Signal];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 2] = [
    IndicatorColorSpec {
        label: "K",
        slot: IndicatorColorSlot::Main,
    },
    IndicatorColorSpec {
        label: "D",
        slot: IndicatorColorSlot::Signal,
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

pub(super) const OVERLAY_CHANNELS: [IndicatorOverlayChannelSpec; 2] = [
    IndicatorOverlayChannelSpec {
        label: "K",
        source: IndicatorOverlaySource::Primary,
        line_width: 1.2,
        color_role: IndicatorOverlayColorRole::Neutral,
        color_slot: IndicatorColorSlot::Main,
    },
    IndicatorOverlayChannelSpec {
        label: "D",
        source: IndicatorOverlaySource::Field(STOCH_RSI_D_FIELD_KEY),
        line_width: 1.0,
        color_role: IndicatorOverlayColorRole::Primary,
        color_slot: IndicatorColorSlot::Signal,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StochRsiConfig {
    pub rsi_length: u16,
    pub stoch_length: u16,
    pub k_smoothing: u16,
    pub d_smoothing: u16,
    pub source: KlinePriceSource,
}

impl Default for StochRsiConfig {
    fn default() -> Self {
        Self {
            rsi_length: 14,
            stoch_length: 14,
            k_smoothing: 3,
            d_smoothing: 3,
            source: KlinePriceSource::Close,
        }
    }
}

impl StochRsiConfig {
    pub fn normalized(self) -> Self {
        Self {
            rsi_length: self.rsi_length.max(2),
            stoch_length: self.stoch_length.max(1),
            k_smoothing: self.k_smoothing.max(1),
            d_smoothing: self.d_smoothing.max(1),
            ..self
        }
    }

    pub fn warmup_bars(self) -> u64 {
        let config = self.normalized();
        u64::from(config.rsi_length)
            .saturating_add(u64::from(config.stoch_length))
            .saturating_add(u64::from(config.k_smoothing))
            .saturating_add(u64::from(config.d_smoothing))
            .saturating_sub(2)
    }
}

impl IndicatorConfigValue for StochRsiConfig {
    fn normalized(self) -> Self {
        self.normalized()
    }

    fn settings(self, _context: IndicatorSettingsContext) -> Vec<IndicatorSetting> {
        vec![
            IndicatorSetting {
                id: "rsi_length",
                label: "RSI length",
                value: IndicatorSettingValue::U16(self.rsi_length),
                control: IndicatorSettingControl::SliderU16 { min: 2, max: 200 },
            },
            IndicatorSetting {
                id: "stoch_length",
                label: "Stoch length",
                value: IndicatorSettingValue::U16(self.stoch_length),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 200 },
            },
            IndicatorSetting {
                id: "k_smoothing",
                label: "K",
                value: IndicatorSettingValue::U16(self.k_smoothing),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 100 },
            },
            IndicatorSetting {
                id: "d_smoothing",
                label: "D",
                value: IndicatorSettingValue::U16(self.d_smoothing),
                control: IndicatorSettingControl::SliderU16 { min: 1, max: 100 },
            },
            IndicatorSetting {
                id: "source",
                label: "RSI source",
                value: IndicatorSettingValue::Choice(self.source.id()),
                control: IndicatorSettingControl::Choices(
                    KlinePriceSource::ALL
                        .into_iter()
                        .map(|source| IndicatorSettingChoice {
                            id: source.id(),
                            label: source.id(),
                        })
                        .collect(),
                ),
            },
        ]
    }

    fn with_setting(self, id: &'static str, value: IndicatorSettingValue) -> Option<Self> {
        match (id, value) {
            ("rsi_length", IndicatorSettingValue::U16(rsi_length)) => {
                Some(Self { rsi_length, ..self })
            }
            ("stoch_length", IndicatorSettingValue::U16(stoch_length)) => Some(Self {
                stoch_length,
                ..self
            }),
            ("k_smoothing", IndicatorSettingValue::U16(k_smoothing)) => Some(Self {
                k_smoothing,
                ..self
            }),
            ("d_smoothing", IndicatorSettingValue::U16(d_smoothing)) => Some(Self {
                d_smoothing,
                ..self
            }),
            ("source", IndicatorSettingValue::Choice(source)) => Some(Self {
                source: KlinePriceSource::from_id(source)?,
                ..self
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StochRsiPoint {
    pub k: f32,
    pub d: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
struct StochRsiValue {
    point: StochRsiPoint,
    rsi: f32,
    average_gain: f32,
    average_loss: f32,
}

struct StochRsiContext {
    average_gain: f32,
    average_loss: f32,
    stoch_window: VecDeque<f32>,
    k_window: VecDeque<f32>,
    d_window: VecDeque<f32>,
}

impl StochRsiContext {
    fn with_averages(average_gain: f32, average_loss: f32) -> Self {
        Self {
            average_gain,
            average_loss,
            stoch_window: VecDeque::new(),
            k_window: VecDeque::new(),
            d_window: VecDeque::new(),
        }
    }

    fn update_averages(&mut self, change: f32, rsi_length: usize) {
        let length = rsi_length as f32;
        self.average_gain = ((self.average_gain * (length - 1.0)) + change.max(0.0)) / length;
        self.average_loss = ((self.average_loss * (length - 1.0)) + (-change).max(0.0)) / length;
    }

    fn record_rsi(&mut self, rsi: f32, config: StochRsiConfig) -> Option<StochRsiPoint> {
        self.stoch_window.push_back(rsi);
        Self::trim_window(&mut self.stoch_window, usize::from(config.stoch_length));
        if self.stoch_window.len() < usize::from(config.stoch_length) {
            return None;
        }

        self.k_window
            .push_back(StochRsiState::stochastic_value(&self.stoch_window));
        Self::trim_window(&mut self.k_window, usize::from(config.k_smoothing));
        if self.k_window.len() < usize::from(config.k_smoothing) {
            return None;
        }

        let k = StochRsiState::average(&self.k_window);
        self.d_window.push_back(k);
        Self::trim_window(&mut self.d_window, usize::from(config.d_smoothing));
        let d = (self.d_window.len() == usize::from(config.d_smoothing))
            .then(|| StochRsiState::average(&self.d_window));
        Some(StochRsiPoint { k, d })
    }

    fn trim_window(window: &mut VecDeque<f32>, maximum_length: usize) {
        while window.len() > maximum_length {
            window.pop_front();
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StochRsiState {
    values: BTreeMap<UnixMs, StochRsiValue>,
}

impl StochRsiState {
    pub(crate) fn recompute_from_bars(&mut self, bars: &[Kline], config: StochRsiConfig) {
        self.values.clear();

        let config = config.normalized();
        let rsi_length = usize::from(config.rsi_length);
        if bars.len() <= rsi_length {
            return;
        }

        let prices = bars
            .iter()
            .map(|bar| config.source.value(bar) as f32)
            .collect::<Vec<_>>();
        let (mut average_gain, mut average_loss) = Self::initial_averages(&prices, rsi_length);
        let mut context = StochRsiContext::with_averages(average_gain, average_loss);

        for index in rsi_length..bars.len() {
            if index > rsi_length {
                let change = prices[index] - prices[index - 1];
                context.update_averages(change, rsi_length);
            }

            average_gain = context.average_gain;
            average_loss = context.average_loss;
            let rsi = Self::rsi(average_gain, average_loss);
            if let Some(point) = context.record_rsi(rsi, config) {
                self.insert_value(bars[index].time, point, rsi, average_gain, average_loss);
            }
        }

        self.trim();
    }

    pub(crate) fn recompute_from_bars_from(
        &mut self,
        bars: &[Kline],
        config: StochRsiConfig,
        first_index: usize,
    ) {
        let config = config.normalized();
        let rsi_length = usize::from(config.rsi_length);
        if first_index <= rsi_length {
            return self.recompute_from_bars(bars, config);
        }

        let Some(mut context) = self.context_before(bars, config, first_index) else {
            return self.recompute_from_bars(bars, config);
        };
        let Some(first_bar) = bars.get(first_index) else {
            return;
        };

        self.values.retain(|time, _| *time < first_bar.time);
        for index in first_index..bars.len() {
            let change = config.source.value(&bars[index]) as f32
                - config.source.value(&bars[index - 1]) as f32;
            context.update_averages(change, rsi_length);
            let rsi = Self::rsi(context.average_gain, context.average_loss);
            if let Some(point) = context.record_rsi(rsi, config) {
                self.insert_value(
                    bars[index].time,
                    point,
                    rsi,
                    context.average_gain,
                    context.average_loss,
                );
            }
        }

        self.trim();
    }

    pub fn value_at(&self, time: UnixMs) -> Option<StochRsiPoint> {
        self.values.get(&time).map(|value| value.point)
    }

    fn context_before(
        &self,
        bars: &[Kline],
        config: StochRsiConfig,
        first_index: usize,
    ) -> Option<StochRsiContext> {
        let previous_index = first_index.checked_sub(1)?;
        let previous = self.values.get(&bars.get(previous_index)?.time)?;
        let stoch_length = usize::from(config.stoch_length);
        let k_smoothing = usize::from(config.k_smoothing);
        let d_smoothing = usize::from(config.d_smoothing);
        let rsi_history_length = stoch_length.saturating_add(k_smoothing).saturating_sub(2);
        let rsi_history_start = first_index.checked_sub(rsi_history_length)?;
        let rsi_history = bars[rsi_history_start..first_index]
            .iter()
            .map(|bar| self.values.get(&bar.time).map(|value| value.rsi))
            .collect::<Option<Vec<_>>>()?;
        let k_history_start = first_index.checked_sub(d_smoothing.saturating_sub(1))?;
        let d_history = bars[k_history_start..first_index]
            .iter()
            .map(|bar| self.values.get(&bar.time).map(|value| value.point.k))
            .collect::<Option<Vec<_>>>()?;

        let mut context =
            StochRsiContext::with_averages(previous.average_gain, previous.average_loss);
        context.stoch_window.extend(
            rsi_history
                .iter()
                .rev()
                .take(stoch_length.saturating_sub(1))
                .copied()
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
        context.k_window.extend(
            rsi_history
                .windows(stoch_length)
                .map(|window| Self::stochastic_value(&VecDeque::from(window.to_vec()))),
        );
        context.d_window.extend(d_history);
        Some(context)
    }

    fn insert_value(
        &mut self,
        time: UnixMs,
        point: StochRsiPoint,
        rsi: f32,
        average_gain: f32,
        average_loss: f32,
    ) {
        self.values.insert(
            time,
            StochRsiValue {
                point,
                rsi,
                average_gain,
                average_loss,
            },
        );
    }

    fn initial_averages(prices: &[f32], length: usize) -> (f32, f32) {
        let (gains, losses) = prices[..=length]
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold((0.0, 0.0), |(gains, losses), change| {
                (gains + change.max(0.0), losses + (-change).max(0.0))
            });
        (gains / length as f32, losses / length as f32)
    }

    fn rsi(average_gain: f32, average_loss: f32) -> f32 {
        const EPSILON: f32 = 1e-6;
        if average_gain <= EPSILON && average_loss <= EPSILON {
            return 50.0;
        }
        if average_loss <= EPSILON {
            return 100.0;
        }
        if average_gain <= EPSILON {
            return 0.0;
        }
        (100.0 - 100.0 / (1.0 + average_gain / average_loss)).clamp(0.0, 100.0)
    }

    fn stochastic_value(window: &VecDeque<f32>) -> f32 {
        let minimum = window.iter().copied().fold(f32::INFINITY, f32::min);
        let maximum = window.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let current = *window.back().expect("stochastic window is non-empty");
        let range = maximum - minimum;
        if range <= 1e-6 {
            50.0
        } else {
            ((current - minimum) / range * 100.0).clamp(0.0, 100.0)
        }
    }

    fn average(window: &VecDeque<f32>) -> f32 {
        window.iter().sum::<f32>() / window.len() as f32
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

impl IndicatorDefinition for StochRsiState {
    type Config = StochRsiConfig;

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

impl IndicatorRuntime for StochRsiState {
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
        let point = self.value_at(bar.time)?;
        let data = IndicatorData::scalar(point.k);
        Some(match point.d {
            Some(d) => data.with_field(STOCH_RSI_D_FIELD_KEY, d),
            None => data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::Volume;
    use exchange::unit::Qty;

    fn bar(time: u64, price: f64) -> Kline {
        Kline::new(
            time,
            price,
            price,
            price,
            price,
            Volume::TotalOnly(Qty::from_f64(1.0)),
            exchange::unit::MinTicksize::new(-2),
        )
    }

    #[test]
    fn normalizes_lengths_and_exposes_its_total_warmup() {
        let config = StochRsiConfig {
            rsi_length: 0,
            stoch_length: 0,
            k_smoothing: 0,
            d_smoothing: 0,
            ..StochRsiConfig::default()
        }
        .normalized();

        assert_eq!(config.rsi_length, 2);
        assert_eq!(config.stoch_length, 1);
        assert_eq!(config.k_smoothing, 1);
        assert_eq!(config.d_smoothing, 1);
        assert_eq!(config.warmup_bars(), 3);
    }

    #[test]
    fn exposes_k_d_lengths_and_source_through_the_generic_setting_schema() {
        let config = StochRsiConfig::default();
        let settings = config.settings(IndicatorSettingsContext {
            basis: crate::chart::Basis::Time(exchange::Timeframe::M1),
        });

        assert_eq!(
            settings
                .iter()
                .map(|setting| setting.id)
                .collect::<Vec<_>>(),
            vec![
                "rsi_length",
                "stoch_length",
                "k_smoothing",
                "d_smoothing",
                "source"
            ]
        );
        assert_eq!(
            config.with_setting("source", IndicatorSettingValue::Choice("Open")),
            Some(StochRsiConfig {
                source: KlinePriceSource::Open,
                ..config
            })
        );
    }

    #[test]
    fn produces_smoothed_k_and_d_after_its_warmup() {
        let bars = (1..=32)
            .map(|time| bar(time, time as f64))
            .collect::<Vec<_>>();
        let mut state = StochRsiState::default();
        state.recompute_from_bars(&bars, StochRsiConfig::default());

        assert_eq!(
            state.value_at(bars[30].time).and_then(|point| point.d),
            None
        );
        let point = state.value_at(bars[31].time).expect("warm indicator value");
        assert!((point.k - 50.0).abs() < 1e-5);
        assert_eq!(point.d, Some(50.0));
    }

    #[test]
    fn incremental_recalculation_matches_a_full_recalculation() {
        let bars = (1..=100)
            .map(|time| bar(time, 100.0 + (time % 11) as f64 - (time % 5) as f64))
            .collect::<Vec<_>>();
        let config = StochRsiConfig::default();
        let mut incremental = StochRsiState::default();
        incremental.recompute_from_bars(&bars[..70], config);
        incremental.recompute_from_bars_from(&bars, config, 70);

        let mut full = StochRsiState::default();
        full.recompute_from_bars(&bars, config);

        for bar in &bars {
            assert_eq!(incremental.value_at(bar.time), full.value_at(bar.time));
        }
    }
}
