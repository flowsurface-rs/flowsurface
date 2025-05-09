use exchange::{Kline, Trade};
use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

use crate::chart::kline::{ClusterKind, KlineTrades, NPoc};
use crate::util::round_to_tick;

#[derive(Debug, Clone)]
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,
    pub footprint: KlineTrades,
}

impl TickAccumulation {
    pub fn new(trade: &Trade, tick_size: f32) -> Self {
        let mut footprint = KlineTrades::new();
        footprint.add_trade_at_price_level(trade, tick_size);

        let kline = Kline {
            time: trade.time,
            open: trade.price,
            high: trade.price,
            low: trade.price,
            close: trade.price,
            volume: (
                if trade.is_sell { 0.0 } else { trade.qty },
                if trade.is_sell { trade.qty } else { 0.0 },
            ),
        };

        Self {
            tick_count: 1,
            kline,
            footprint,
        }
    }

    pub fn update_with_trade(&mut self, trade: &Trade, tick_size: f32) {
        self.tick_count += 1;
        self.kline.high = self.kline.high.max(trade.price);
        self.kline.low = self.kline.low.min(trade.price);
        self.kline.close = trade.price;

        if trade.is_sell {
            self.kline.volume.1 += trade.qty;
        } else {
            self.kline.volume.0 += trade.qty;
        }

        self.add_trade(trade, tick_size);
    }

    fn add_trade(&mut self, trade: &Trade, tick_size: f32) {
        self.footprint.add_trade_at_price_level(trade, tick_size);
    }

    pub fn max_cluster_qty(
        &self,
        cluster_kind: ClusterKind,
        highest: OrderedFloat<f32>,
        lowest: OrderedFloat<f32>,
    ) -> f32 {
        match cluster_kind {
            ClusterKind::BidAsk => self.footprint.max_qty_by(highest, lowest, f32::max),
            ClusterKind::DeltaProfile => self
                .footprint
                .max_qty_by(highest, lowest, |buy, sell| (buy - sell).abs()),
            ClusterKind::VolumeProfile => {
                self.footprint
                    .max_qty_by(highest, lowest, |buy, sell| buy + sell)
            }
        }
    }

    pub fn is_full(&self, interval: u64) -> bool {
        self.tick_count >= interval as usize
    }

    pub fn poc_price(&self) -> Option<f32> {
        self.footprint.poc_price()
    }

    pub fn set_poc_status(&mut self, status: NPoc) {
        self.footprint.set_poc_status(status);
    }

    pub fn calculate_poc(&mut self) {
        self.footprint.calculate_poc();
    }
}

pub struct TickAggr {
    pub data_points: Vec<TickAccumulation>,
    pub interval: u64,
    pub tick_size: f32,
}

impl TickAggr {
    pub fn new(interval: u64, tick_size: f32, raw_trades: &[Trade]) -> Self {
        let mut tick_aggr = Self {
            data_points: Vec::new(),
            interval,
            tick_size,
        };

        if !raw_trades.is_empty() {
            tick_aggr.insert_trades(raw_trades);
        }

        tick_aggr
    }

    pub fn change_tick_size(&mut self, tick_size: f32, raw_trades: &[Trade]) {
        self.tick_size = tick_size;

        self.data_points.clear();

        if !raw_trades.is_empty() {
            self.insert_trades(raw_trades);
        }
    }

    /// return latest data point and its index
    pub fn latest_dp(&self) -> Option<(&TickAccumulation, usize)> {
        self.data_points
            .last()
            .map(|dp| (dp, self.data_points.len() - 1))
    }

    pub fn volume_data(&self) -> BTreeMap<u64, (f32, f32)> {
        self.into()
    }

    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        let mut updated_indices = Vec::new();

        for trade in buffer {
            if self.data_points.is_empty() {
                self.data_points
                    .push(TickAccumulation::new(trade, self.tick_size));
                updated_indices.push(0);
            } else {
                let last_idx = self.data_points.len() - 1;

                if self.data_points[last_idx].is_full(self.interval) {
                    self.data_points
                        .push(TickAccumulation::new(trade, self.tick_size));
                    updated_indices.push(self.data_points.len() - 1);
                } else {
                    self.data_points[last_idx].update_with_trade(trade, self.tick_size);
                    if !updated_indices.contains(&last_idx) {
                        updated_indices.push(last_idx);
                    }
                }
            }
        }

        for idx in updated_indices {
            if idx < self.data_points.len() {
                self.data_points[idx].calculate_poc();
            }
        }

        self.update_poc_status();
    }

    pub fn update_poc_status(&mut self) {
        let updates = self
            .data_points
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| dp.poc_price().map(|price| (idx, price)))
            .collect::<Vec<_>>();

        let total_points = self.data_points.len();

        for (current_idx, poc_price) in updates {
            let mut npoc = NPoc::default();

            for next_idx in (current_idx + 1)..total_points {
                let next_dp = &self.data_points[next_idx];
                if round_to_tick(next_dp.kline.low, self.tick_size) <= poc_price
                    && round_to_tick(next_dp.kline.high, self.tick_size) >= poc_price
                {
                    // on render we reverse the order of the points
                    // as it is easier to just take the idx=0 as latest candle for coords
                    let reversed_idx = (total_points - 1) - next_idx;
                    npoc.filled(reversed_idx as u64);
                    break;
                } else {
                    npoc.unfilled();
                }
            }

            if current_idx < total_points {
                let data_point = &mut self.data_points[current_idx];
                data_point.set_poc_status(npoc);
            }
        }
    }

    pub fn max_qty_idx_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: usize,
        latest: usize,
        highest: OrderedFloat<f32>,
        lowest: OrderedFloat<f32>,
    ) -> f32 {
        let mut max_cluster_qty: f32 = 0.0;

        self.data_points
            .iter()
            .rev()
            .enumerate()
            .filter(|(index, _)| *index <= latest && *index >= earliest)
            .for_each(|(_, dp)| {
                max_cluster_qty =
                    max_cluster_qty.max(dp.max_cluster_qty(cluster_kind, highest, lowest));
            });

        max_cluster_qty
    }
}

impl From<&TickAggr> for BTreeMap<u64, (f32, f32)> {
    /// Converts datapoints into a map of timestamps and volume data
    fn from(tick_aggr: &TickAggr) -> Self {
        tick_aggr
            .data_points
            .iter()
            .enumerate()
            .map(|(idx, dp)| (idx as u64, (dp.kline.volume.0, dp.kline.volume.1)))
            .collect()
    }
}
