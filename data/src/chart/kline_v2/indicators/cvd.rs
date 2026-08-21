use super::super::IndicatorUpdateScope;
use super::super::values::IndicatorData;
use super::input::{IndicatorInput, NormalizedTradeBar};
use super::runtime::{IndicatorDefinition, IndicatorRuntime};
use super::{
    IndicatorAvailability, IndicatorColorSlot, IndicatorColorSpec, IndicatorInputCoverage,
    IndicatorMarkSpec, IndicatorRuntimeSnapshot, IndicatorUnsupportedReason,
};
use crate::chart::kline_v2::composition::{BarMode, MarkKind};
use exchange::{Kline, UnixMs, unit::Qty};
use std::collections::{BTreeMap, BTreeSet};

const MIN_DIRECTIONAL_RUN: usize = 2;
const SERIES_MAX_POINTS: usize = 5000;

const COLOR_SLOTS: [IndicatorColorSlot; 1] = [IndicatorColorSlot::Main];

pub(super) const COLOR_SPECS: [IndicatorColorSpec; 1] = [IndicatorColorSpec {
    label: "Line",
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

#[derive(Debug, Clone, Copy, Default)]
struct CumulativeDeltaPoint {
    cumulative: Qty,
    reliable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CumulativeVolumeDeltaState {
    kline_deltas: BTreeMap<UnixMs, Option<Qty>>,
    trade_deltas: BTreeMap<UnixMs, Qty>,
    values: BTreeMap<UnixMs, CumulativeDeltaPoint>,
    has_points: bool,
    input_coverage: IndicatorInputCoverage,
}

impl CumulativeVolumeDeltaState {
    pub fn clear(&mut self) {
        self.kline_deltas.clear();
        self.trade_deltas.clear();
        self.values.clear();
        self.has_points = false;
        self.input_coverage = IndicatorInputCoverage::Unknown;
    }

    fn replace_kline_deltas(&mut self, bars: &[Kline]) {
        self.kline_deltas.clear();
        self.has_points = !bars.is_empty();

        for bar in bars {
            let delta = bar.volume.buy_sell().map(|(buy, sell)| buy - sell);
            self.kline_deltas.insert(bar.time, delta);
        }

        self.rebuild_cumulative();
    }

    fn replace_kline_deltas_from(&mut self, bars: &[Kline], first_index: usize) {
        if first_index == 0 {
            return self.replace_kline_deltas(bars);
        }

        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.kline_deltas.retain(|time, _| *time < first_bar.time);
        for bar in bars.iter().skip(first_index) {
            self.kline_deltas.insert(
                bar.time,
                bar.volume.buy_sell().map(|(buy, sell)| buy - sell),
            );
        }
        self.has_points = !self.kline_deltas.is_empty() || !self.trade_deltas.is_empty();
        self.rebuild_cumulative_from(first_bar.time);
    }

    fn replace_trade_deltas(&mut self, bars: &[NormalizedTradeBar]) {
        self.trade_deltas.clear();
        self.has_points |= !bars.is_empty();

        for bar in bars {
            let Some(values) = bar.values else {
                continue;
            };

            self.trade_deltas
                .insert(bar.time, values.buy_qty - values.sell_qty);
        }

        self.rebuild_cumulative();
    }

    fn replace_trade_deltas_from(&mut self, bars: &[NormalizedTradeBar], first_index: usize) {
        if first_index == 0 {
            return self.replace_trade_deltas(bars);
        }

        let Some(first_bar) = bars.get(first_index) else {
            return;
        };
        self.trade_deltas.retain(|time, _| *time < first_bar.time);
        for bar in bars.iter().skip(first_index) {
            if let Some(values) = bar.values {
                self.trade_deltas
                    .insert(bar.time, values.buy_qty - values.sell_qty);
            } else {
                self.trade_deltas.remove(&bar.time);
            }
        }
        self.has_points = !self.kline_deltas.is_empty() || !self.trade_deltas.is_empty();
        self.rebuild_cumulative_from(first_bar.time);
    }

    fn effective_deltas(&self) -> BTreeMap<UnixMs, Qty> {
        let mut times = BTreeSet::new();
        times.extend(self.kline_deltas.keys().copied());
        times.extend(self.trade_deltas.keys().copied());

        times
            .into_iter()
            .map(|time| {
                let delta = self
                    .kline_deltas
                    .get(&time)
                    .copied()
                    .flatten()
                    .or_else(|| self.trade_deltas.get(&time).copied())
                    .unwrap_or(Qty::ZERO);
                (time, delta)
            })
            .collect()
    }

    fn rebuild_cumulative(&mut self) {
        let deltas = self.effective_deltas();
        let entries: Vec<_> = deltas.iter().collect();
        let reliable = Self::reliable_indices(&entries, MIN_DIRECTIONAL_RUN);

        let mut cumulative = Qty::ZERO;
        self.values = entries
            .iter()
            .enumerate()
            .map(|(index, &(&time, &delta))| {
                cumulative += delta;
                (
                    time,
                    CumulativeDeltaPoint {
                        cumulative,
                        reliable: reliable[index],
                    },
                )
            })
            .collect();

        while self.values.len() > SERIES_MAX_POINTS {
            let Some((&earliest, _)) = self.values.first_key_value() else {
                break;
            };
            self.values.remove(&earliest);
        }

        self.update_coverage();
    }

    fn rebuild_cumulative_from(&mut self, first_time: UnixMs) {
        let deltas = self.effective_deltas();
        let entries: Vec<_> = deltas.into_iter().collect();
        let mut start = entries
            .binary_search_by_key(&first_time, |(time, _)| *time)
            .unwrap_or_else(|index| index);

        while start > 0 && entries[start - 1].1 != Qty::ZERO {
            start -= 1;
        }

        let start_time = entries
            .get(start)
            .map(|(time, _)| *time)
            .unwrap_or(first_time);
        self.values.retain(|time, _| *time < start_time);

        let mut cumulative = self
            .values
            .range(..start_time)
            .next_back()
            .map(|(_, point)| point.cumulative)
            .unwrap_or(Qty::ZERO);
        let mut index = start;

        while index < entries.len() {
            let run_start = index;
            while index < entries.len() && entries[index].1 != Qty::ZERO {
                index += 1;
            }
            let reliable_run = index - run_start >= MIN_DIRECTIONAL_RUN;

            for (run_index, &(time, delta)) in entries[run_start..index].iter().enumerate() {
                cumulative += delta;
                self.values.insert(
                    time,
                    CumulativeDeltaPoint {
                        cumulative,
                        reliable: reliable_run && run_index > 0,
                    },
                );
            }

            if index < entries.len() {
                cumulative += entries[index].1;
                self.values.insert(
                    entries[index].0,
                    CumulativeDeltaPoint {
                        cumulative,
                        reliable: false,
                    },
                );
                index += 1;
            }
        }

        while self.values.len() > SERIES_MAX_POINTS {
            let Some((&earliest, _)) = self.values.first_key_value() else {
                break;
            };
            self.values.remove(&earliest);
        }

        self.update_coverage();
    }

    fn update_coverage(&mut self) {
        let has_directional =
            self.kline_deltas.values().any(Option::is_some) || !self.trade_deltas.is_empty();
        self.input_coverage = if !self.has_points {
            IndicatorInputCoverage::Unknown
        } else if has_directional {
            IndicatorInputCoverage::Complete
        } else {
            IndicatorInputCoverage::MissingRequiredInput
        };
    }

    fn reliable_indices<K>(entries: &[(&K, &Qty)], min_run: usize) -> Vec<bool> {
        let mut reliable = vec![false; entries.len()];
        let mut index = 0;

        while index < entries.len() {
            if *entries[index].1 == Qty::ZERO {
                index += 1;
                continue;
            }

            let run_start = index;
            while index < entries.len() && *entries[index].1 != Qty::ZERO {
                index += 1;
            }

            if index - run_start >= min_run {
                for slot in reliable.iter_mut().take(index).skip(run_start + 1) {
                    *slot = true;
                }
            }
        }

        reliable
    }

    pub fn value_at(&self, time: UnixMs) -> Option<f32> {
        let point = self.values.get(&time)?;
        point.reliable.then(|| point.cumulative.to_f32_lossy())
    }
}

impl IndicatorDefinition for CumulativeVolumeDeltaState {
    type Config = ();

    fn warmup_bars(_config: Self::Config) -> Option<u64> {
        None
    }

    fn availability(
        _context: super::AvailabilityContext,
        snapshot: IndicatorRuntimeSnapshot,
    ) -> IndicatorAvailability {
        match snapshot.input_coverage {
            IndicatorInputCoverage::Unknown => IndicatorAvailability::PendingProbe,
            IndicatorInputCoverage::Complete => IndicatorAvailability::Available,
            IndicatorInputCoverage::MissingRequiredInput => {
                IndicatorAvailability::Unsupported(IndicatorUnsupportedReason::MissingRequiredInput)
            }
        }
    }
}

impl IndicatorRuntime for CumulativeVolumeDeltaState {
    fn clear(&mut self) {
        CumulativeVolumeDeltaState::clear(self);
    }

    fn rebuild_from_bars(&mut self, bars: &[Kline], _config: Self::Config) {
        self.replace_kline_deltas(bars);
    }

    fn on_input(&mut self, input: IndicatorInput<'_>, _config: Self::Config) {
        match input {
            IndicatorInput::Klines { bars, .. } => self.replace_kline_deltas(bars),
            IndicatorInput::Trades { bars } => self.replace_trade_deltas(bars),
            IndicatorInput::OpenInterest { .. } => {}
        }
    }

    fn on_input_from(
        &mut self,
        input: IndicatorInput<'_>,
        _config: Self::Config,
        scope: IndicatorUpdateScope,
    ) {
        match input {
            IndicatorInput::Klines { bars, .. } => {
                self.replace_kline_deltas_from(bars, scope.first_index())
            }
            IndicatorInput::Trades { bars } => {
                self.replace_trade_deltas_from(bars, scope.first_index())
            }
            IndicatorInput::OpenInterest { .. } => {}
        }
    }

    fn snapshot(&self) -> IndicatorRuntimeSnapshot {
        IndicatorRuntimeSnapshot {
            input_coverage: self.input_coverage,
            ..IndicatorRuntimeSnapshot::default()
        }
    }

    fn data_for_bar(&self, bar: &Kline) -> Option<IndicatorData> {
        self.value_at(bar.time).map(IndicatorData::scalar)
    }
}
