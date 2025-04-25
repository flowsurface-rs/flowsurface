use crate::chart::kline::{ClusterKind, KlineTrades, NPoc};
use exchange::{Kline, Timeframe, Trade};

use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

pub struct DataPoint {
    pub kline: Kline,
    pub footprint: KlineTrades,
}

impl DataPoint {
    pub fn max_cluster_qty(
        &self,
        cluster_kind: ClusterKind,
        highest: OrderedFloat<f32>,
        lowest: OrderedFloat<f32>,
    ) -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk => self
                .footprint
                .max_qty_by(highest, lowest, |buy, sell| buy.max(sell)),
            ClusterKind::DeltaProfile => self
                .footprint
                .max_qty_by(highest, lowest, |buy, sell| (buy - sell).abs()),
            ClusterKind::VolumeProfile => {
                self.footprint
                    .max_qty_by(highest, lowest, |buy, sell| buy + sell)
            }
        }
    }

    pub fn add_trade(&mut self, trade: &Trade, tick_size: f32) {
        self.footprint.add_trade_at_price_level(trade, tick_size);
    }

    pub fn poc_price(&self) -> Option<f32> {
        self.footprint.poc_price()
    }

    pub fn set_poc_status(&mut self, status: NPoc) {
        self.footprint.set_poc_status(status);
    }

    pub fn clear_trades(&mut self) {
        self.footprint.clear();
    }

    pub fn calculate_poc(&mut self) -> bool {
        self.footprint.calculate_poc()
    }
}

pub struct TimeSeries {
    pub data_points: BTreeMap<u64, DataPoint>,
    pub interval: Timeframe,
    pub tick_size: f32,
}

impl TimeSeries {
    pub fn new(
        interval: Timeframe,
        tick_size: f32,
        raw_trades: &[Trade],
        klines: &[Kline],
    ) -> Self {
        let mut timeseries = Self {
            data_points: BTreeMap::new(),
            interval,
            tick_size,
        };

        timeseries.insert_klines(klines);

        if !raw_trades.is_empty() {
            timeseries.insert_trades(raw_trades, None);
        }

        timeseries
    }

    pub fn base_price(&self) -> f32 {
        self.data_points
            .values()
            .last()
            .map_or(0.0, |dp| dp.kline.close)
    }

    pub fn latest_timestamp(&self) -> Option<u64> {
        self.data_points.keys().last().copied()
    }

    pub fn latest_kline(&self) -> Option<&Kline> {
        self.data_points.values().last().map(|dp| &dp.kline)
    }

    pub fn price_scale(&self, lookback: usize) -> (f32, f32) {
        let mut scale_high = 0.0f32;
        let mut scale_low = f32::MAX;

        self.data_points
            .iter()
            .rev()
            .take(lookback)
            .for_each(|(_, data_point)| {
                scale_high = scale_high.max(data_point.kline.high);
                scale_low = scale_low.min(data_point.kline.low);
            });

        (scale_high, scale_low)
    }

    pub fn volume_data(&self) -> BTreeMap<u64, (f32, f32)> {
        self.into()
    }

    pub fn kline_timerange(&self) -> (u64, u64) {
        let earliest = self.data_points.keys().next().copied().unwrap_or(0);
        let latest = self.data_points.keys().last().copied().unwrap_or(0);

        (earliest, latest)
    }

    pub fn change_tick_size(&mut self, tick_size: f32, all_raw_trades: &[Trade]) {
        self.tick_size = tick_size;
        self.clear_trades();

        if !all_raw_trades.is_empty() {
            self.insert_trades(all_raw_trades, None);
        }
    }

    pub fn insert_klines(&mut self, klines: &[Kline]) {
        for kline in klines {
            let entry = self
                .data_points
                .entry(kline.time)
                .or_insert_with(|| DataPoint {
                    kline: *kline,
                    footprint: KlineTrades::new(),
                });

            entry.kline = *kline;
        }

        self.update_poc_status();
    }

    pub fn insert_trades(&mut self, buffer: &[Trade], update_t: Option<u64>) {
        if buffer.is_empty() {
            return;
        }

        let aggregate_time = self.interval.to_milliseconds();
        let rounded_update_t = update_t.map(|t| (t / aggregate_time) * aggregate_time);

        let mut updated_times = Vec::new();

        buffer.iter().for_each(|trade| {
            let rounded_time =
                rounded_update_t.unwrap_or((trade.time / aggregate_time) * aggregate_time);

            if !updated_times.contains(&rounded_time) {
                updated_times.push(rounded_time);
            }

            let entry = self
                .data_points
                .entry(rounded_time)
                .or_insert_with(|| DataPoint {
                    kline: Kline {
                        time: rounded_time,
                        open: trade.price,
                        high: trade.price,
                        low: trade.price,
                        close: trade.price,
                        volume: (0.0, 0.0),
                    },
                    footprint: KlineTrades::new(),
                });

            entry.add_trade(trade, self.tick_size);
        });

        for time in updated_times {
            if let Some(data_point) = self.data_points.get_mut(&time) {
                data_point.calculate_poc();
            }
        }
    }

    pub fn update_poc_status(&mut self) {
        let updates = self
            .data_points
            .iter()
            .filter_map(|(&time, dp)| dp.poc_price().map(|price| (time, price)))
            .collect::<Vec<_>>();

        for (current_time, poc_price) in updates {
            let mut npoc = NPoc::default();

            for (&next_time, next_dp) in self.data_points.range((current_time + 1)..) {
                if next_dp.kline.low <= poc_price && next_dp.kline.high >= poc_price {
                    npoc.filled(next_time);
                    break;
                }
            }

            if let Some(data_point) = self.data_points.get_mut(&current_time) {
                data_point.set_poc_status(npoc);
            }
        }
    }

    pub fn max_qty_ts_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: u64,
        latest: u64,
        highest: OrderedFloat<f32>,
        lowest: OrderedFloat<f32>,
    ) -> f32 {
        let mut max_cluster_qty: f32 = 0.0;

        self.data_points
            .range(earliest..=latest)
            .for_each(|(_, dp)| {
                max_cluster_qty =
                    max_cluster_qty.max(dp.max_cluster_qty(cluster_kind, highest, lowest));
            });

        max_cluster_qty
    }

    pub fn clear_trades(&mut self) {
        for data_point in self.data_points.values_mut() {
            data_point.clear_trades();
        }
    }

    pub fn check_integrity(&self, earliest: u64, latest: u64, interval: u64) -> Option<Vec<u64>> {
        let mut time = earliest;
        let mut missing_count = 0;

        while time < latest {
            if !self.data_points.contains_key(&time) {
                missing_count += 1;
                break;
            }
            time += interval;
        }

        if missing_count > 0 {
            let mut missing_keys = Vec::with_capacity(((latest - earliest) / interval) as usize);
            let mut time = earliest;

            while time < latest {
                if !self.data_points.contains_key(&time) {
                    missing_keys.push(time);
                }
                time += interval;
            }

            log::warn!(
                "Integrity check failed: missing {} klines",
                missing_keys.len()
            );
            return Some(missing_keys);
        }

        None
    }
}

impl From<&TimeSeries> for BTreeMap<u64, (f32, f32)> {
    /// Converts datapoints into a map of timestamps and volume data
    fn from(timeseries: &TimeSeries) -> Self {
        timeseries
            .data_points
            .iter()
            .map(|(time, dp)| (*time, (dp.kline.volume.0, dp.kline.volume.1)))
            .collect()
    }
}
