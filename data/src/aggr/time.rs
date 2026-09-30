use std::collections::BTreeMap;

use rustc_hash::FxHashSet;

use crate::chart::Basis;
use crate::chart::heatmap::HeatmapDataPoint;
use crate::chart::kline::bar::KlineDataPoint;
use crate::chart::kline::footprint::{ClusterKind, KlineTrades};
use crate::chart::kline::poc::NPoc;

use exchange::unit::{Price, PriceStep, Qty};
use exchange::{Kline, Timeframe, Trade, UnixMs};

pub trait DataPoint {
    fn add_trade(&mut self, trade: &Trade, step: PriceStep);

    fn clear_trades(&mut self);
}

pub struct TimeSeries<D: DataPoint> {
    pub datapoints: BTreeMap<UnixMs, D>,
    pub interval: Timeframe,
    pub tick_size: PriceStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrderedChange {
    Tail { from: UnixMs },
    Rebuild,
}

#[derive(Debug, Clone)]
pub(crate) struct KlineInsertReport {
    pub inserted_timestamps: Vec<UnixMs>,
    pub ordered_change: OrderedChange,
}

impl<D: DataPoint> TimeSeries<D> {
    pub fn timerange(&self) -> (UnixMs, UnixMs) {
        let earliest = self
            .datapoints
            .keys()
            .next()
            .copied()
            .unwrap_or(UnixMs::ZERO);
        let latest = self
            .datapoints
            .keys()
            .last()
            .copied()
            .unwrap_or(UnixMs::ZERO);

        (earliest, latest)
    }

    /// Ensures a datapoint bucket exists at `rounded_t` and ingests all trades into it.
    pub fn ingest_trades_bucket(&mut self, rounded_t: UnixMs, trades: &[Trade], step: PriceStep)
    where
        D: Default,
    {
        let bucket = self.datapoints.entry(rounded_t).or_default();

        for trade in trades {
            bucket.add_trade(trade, step);
        }
    }

    pub fn clear_trades(&mut self) {
        for data_point in self.datapoints.values_mut() {
            data_point.clear_trades();
        }
    }

    fn align_down_to_phase(time: UnixMs, phase: UnixMs, interval: u64) -> UnixMs {
        if time >= phase {
            let t = time.as_u64();
            let p = phase.as_u64();
            UnixMs::new(t.saturating_sub((t - p) % interval))
        } else {
            phase
        }
    }
}

impl TimeSeries<KlineDataPoint> {
    pub fn new(interval: Timeframe, tick_size: PriceStep, klines: &[Kline]) -> Self {
        let mut timeseries = Self {
            datapoints: BTreeMap::new(),
            interval,
            tick_size,
        };

        timeseries.insert_klines(klines);
        timeseries
    }

    pub fn insert_klines(&mut self, klines: &[Kline]) {
        let _ = self.insert_klines_inner(klines, false);
    }

    pub(crate) fn insert_klines_report(&mut self, klines: &[Kline]) -> Option<KlineInsertReport> {
        self.insert_klines_inner(klines, true)
    }

    fn insert_klines_inner(
        &mut self,
        klines: &[Kline],
        update_poc_status: bool,
    ) -> Option<KlineInsertReport> {
        if klines.is_empty() {
            return None;
        }

        let previous_latest = self.datapoints.keys().next_back().copied();
        let mut inserted_timestamps = Vec::new();
        let mut changed_existing = false;

        for kline in klines {
            let entry = self.datapoints.entry(kline.time);
            match entry {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(KlineDataPoint {
                        kline: *kline,
                        footprint: KlineTrades::new(),
                    });
                    inserted_timestamps.push(kline.time);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().kline = *kline;
                    changed_existing = true;
                }
            }
        }

        inserted_timestamps.sort_unstable();

        if update_poc_status {
            self.update_poc_status();
        }

        let ordered_change = if !changed_existing
            && inserted_timestamps
                .iter()
                .all(|timestamp| previous_latest.is_none_or(|latest| *timestamp > latest))
        {
            OrderedChange::Tail {
                from: inserted_timestamps
                    .iter()
                    .copied()
                    .min()
                    .unwrap_or_else(|| previous_latest.unwrap_or(UnixMs::ZERO)),
            }
        } else {
            OrderedChange::Rebuild
        };

        Some(KlineInsertReport {
            inserted_timestamps,
            ordered_change,
        })
    }

    pub(crate) fn missing_kline_range(
        &self,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Option<(UnixMs, UnixMs)> {
        if self.datapoints.is_empty() {
            return None;
        }

        let interval = self.interval.to_milliseconds();
        if interval == 0 {
            return None;
        }

        let (series_earliest, series_latest) = self.timerange();
        let phase = UnixMs::new(series_earliest.as_u64() % interval);
        let check_earliest =
            Self::align_down_to_phase(earliest.max(series_earliest), phase, interval)
                .max(series_earliest);
        let check_latest = Self::align_down_to_phase(latest.min(series_latest), phase, interval)
            .min(series_latest);

        if check_earliest >= check_latest {
            return None;
        }

        let mut first_missing = None;
        let mut last_missing = None;
        let mut time = check_earliest;
        const MAX_INTEGRITY_SCAN: u64 = 1_000_000;

        for _ in 0..MAX_INTEGRITY_SCAN {
            if time >= check_latest {
                break;
            }
            if !self.datapoints.contains_key(&time) {
                first_missing.get_or_insert(time);
                last_missing = Some(time);
            }
            time = time.saturating_add(interval);
        }

        first_missing.zip(last_missing)
    }

    pub fn insert_trades_existing_buckets(&mut self, buffer: &[Trade]) {
        if buffer.is_empty() {
            return;
        }
        let mut updated_times = FxHashSet::default();

        for run in buffer.chunk_by(|left, right| {
            left.time.floor_to(self.interval) == right.time.floor_to(self.interval)
        }) {
            let rounded_time = run[0].time.floor_to(self.interval);
            if let Some(entry) = self.datapoints.get_mut(&rounded_time) {
                updated_times.insert(rounded_time);
                for trade in run {
                    entry.add_trade(trade, self.tick_size);
                }
            }
        }

        for time in updated_times {
            if let Some(data_point) = self.datapoints.get_mut(&time) {
                data_point.calculate_poc();
            }
        }
    }

    pub fn backfill_new_buckets(&mut self, trades: &[Trade], newborn: &[UnixMs]) {
        if trades.is_empty() || newborn.is_empty() {
            return;
        }

        let newborn_times = newborn.iter().copied().collect::<FxHashSet<UnixMs>>();
        let mut updated_times = FxHashSet::default();

        for trade in trades {
            let rounded_time = trade.time.floor_to(self.interval);
            if newborn_times.contains(&rounded_time)
                && let Some(entry) = self.datapoints.get_mut(&rounded_time)
            {
                updated_times.insert(rounded_time);
                entry.add_trade(trade, self.tick_size);
            }
        }

        for time in updated_times {
            if let Some(data_point) = self.datapoints.get_mut(&time) {
                data_point.calculate_poc();
            }
        }
    }

    pub fn change_tick_size(&mut self, tick_size: PriceStep, raw_trades: &[Trade]) {
        self.tick_size = tick_size;

        self.clear_trades();

        if !raw_trades.is_empty() {
            self.insert_trades_existing_buckets(raw_trades);
        }

        self.update_poc_status();
    }

    pub fn update_poc_status(&mut self) {
        let updates = self
            .datapoints
            .iter()
            .filter_map(|(&time, dp)| dp.poc_price().map(|price| (time, price)))
            .collect::<Vec<_>>();

        for (current_time, poc_price) in updates {
            let mut npoc = NPoc::default();

            for (&next_time, next_dp) in self.datapoints.range(current_time.saturating_add(1)..) {
                let next_dp_low = next_dp.kline.low.round_to_step(self.tick_size);
                let next_dp_high = next_dp.kline.high.round_to_step(self.tick_size);

                if next_dp_low <= poc_price && next_dp_high >= poc_price {
                    npoc.filled(next_time.as_u64());
                    break;
                } else {
                    npoc.unfilled();
                }
            }

            if let Some(data_point) = self.datapoints.get_mut(&current_time) {
                data_point.set_poc_status(npoc);
            }
        }
    }

    pub fn suggest_trade_fetch_range(
        &self,
        visible_earliest: UnixMs,
        visible_latest: UnixMs,
    ) -> Option<(UnixMs, UnixMs)> {
        if self.datapoints.is_empty() {
            return None;
        }

        self.find_trade_gap()
            .and_then(|(last_t_before_gap, first_t_after_gap)| {
                if last_t_before_gap.is_none() && first_t_after_gap.is_none() {
                    return None;
                }
                let (data_earliest, data_latest) = self.timerange();

                let gap_start = last_t_before_gap.map_or(data_earliest, |t| t.saturating_add(1));

                let fetch_from = gap_start
                    .max(visible_earliest)
                    .floor_to(self.interval)
                    .max(gap_start);

                let fetch_to = match first_t_after_gap {
                    Some(t) => t.saturating_sub(1),
                    None => data_latest.min(visible_latest),
                };

                if fetch_from < fetch_to {
                    if gap_start < visible_earliest {
                        let interval_ms = self.interval.to_milliseconds();
                        if fetch_to.as_u64().saturating_sub(fetch_from.as_u64()) < interval_ms {
                            return None;
                        }
                    }
                    Some((fetch_from, fetch_to))
                } else {
                    None
                }
            })
    }

    fn find_trade_gap(&self) -> Option<(Option<UnixMs>, Option<UnixMs>)> {
        let empty_kline_time = self
            .datapoints
            .iter()
            .rev()
            .find(|(_, dp)| dp.footprint.trades.is_empty())
            .map(|(&time, _)| time);

        if let Some(target_time) = empty_kline_time {
            let last_t_before_gap = self
                .datapoints
                .range(..target_time)
                .rev()
                .find_map(|(_, dp)| dp.last_trade_time());

            let first_t_after_gap = self
                .datapoints
                .range(target_time.saturating_add(1)..)
                .find_map(|(_, dp)| dp.first_trade_time());

            Some((last_t_before_gap, first_t_after_gap))
        } else {
            None
        }
    }

    pub fn max_qty_ts_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: UnixMs,
        latest: UnixMs,
        highest: Price,
        lowest: Price,
    ) -> Qty {
        let mut max_cluster_qty: Qty = Qty::default();

        self.datapoints
            .range(earliest..=latest)
            .for_each(|(_, dp)| {
                max_cluster_qty =
                    max_cluster_qty.max(dp.max_cluster_qty(cluster_kind, highest, lowest));
            });

        max_cluster_qty
    }
}

impl TimeSeries<HeatmapDataPoint> {
    pub fn new(basis: Basis, tick_size: PriceStep) -> Self {
        let timeframe = match basis {
            Basis::Time(interval) => interval,
            Basis::Tick(_) => unimplemented!(),
        };

        Self {
            datapoints: BTreeMap::new(),
            interval: timeframe,
            tick_size,
        }
    }
}
