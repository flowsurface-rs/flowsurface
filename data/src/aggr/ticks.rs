use crate::aggr;
use crate::chart::kline::{ClusterKind, KlineTrades, NPoc};
use exchange::util::{Price, PriceStep};
use exchange::{Kline, Trade};

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,
    pub footprint: KlineTrades,
}

impl TickAccumulation {
    pub fn new(trade: &Trade, step: PriceStep) -> Self {
        let mut footprint = KlineTrades::new();
        footprint.add_trade_to_nearest_bin(trade, step);

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

    pub fn update_with_trade(&mut self, trade: &Trade, step: PriceStep) {
        self.tick_count += 1;
        self.kline.high = self.kline.high.max(trade.price);
        self.kline.low = self.kline.low.min(trade.price);
        self.kline.close = trade.price;

        if trade.is_sell {
            self.kline.volume.1 += trade.qty;
        } else {
            self.kline.volume.0 += trade.qty;
        }

        self.add_trade(trade, step);
    }

    fn add_trade(&mut self, trade: &Trade, step: PriceStep) {
        self.footprint.add_trade_to_nearest_bin(trade, step);
    }

    pub fn max_cluster_qty(&self, cluster_kind: ClusterKind, highest: Price, lowest: Price) -> f32 {
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

    pub fn is_full(&self, interval: aggr::TickCount) -> bool {
        self.tick_count >= interval.0 as usize
    }

    pub fn poc_price(&self) -> Option<Price> {
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
    pub datapoints: Vec<TickAccumulation>,
    pub interval: aggr::TickCount,
    pub tick_size: PriceStep,
}

impl TickAggr {
    pub fn new(interval: aggr::TickCount, tick_size: PriceStep, raw_trades: &[Trade]) -> Self {
        let mut tick_aggr = Self {
            datapoints: Vec::new(),
            interval,
            tick_size,
        };

        if !raw_trades.is_empty() {
            tick_aggr.insert_trades(raw_trades);
        }

        tick_aggr
    }

    pub fn change_tick_size(&mut self, tick_size: f32, raw_trades: &[Trade]) {
        self.tick_size = PriceStep::from_f32(tick_size);

        self.datapoints.clear();

        if !raw_trades.is_empty() {
            self.insert_trades(raw_trades);
        }
    }

    /// return latest data point and its index
    pub fn latest_dp(&self) -> Option<(&TickAccumulation, usize)> {
        self.datapoints
            .last()
            .map(|dp| (dp, self.datapoints.len() - 1))
    }

    pub fn volume_data(&self) -> BTreeMap<u64, (f32, f32)> {
        self.into()
    }

    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        let mut updated_indices = Vec::new();

        for trade in buffer {
            if self.datapoints.is_empty() {
                self.datapoints
                    .push(TickAccumulation::new(trade, self.tick_size));
                updated_indices.push(0);
            } else {
                let last_idx = self.datapoints.len() - 1;

                if self.datapoints[last_idx].is_full(self.interval) {
                    self.datapoints
                        .push(TickAccumulation::new(trade, self.tick_size));
                    updated_indices.push(self.datapoints.len() - 1);
                } else {
                    self.datapoints[last_idx].update_with_trade(trade, self.tick_size);
                    if !updated_indices.contains(&last_idx) {
                        updated_indices.push(last_idx);
                    }
                }
            }
        }

        for idx in updated_indices {
            if idx < self.datapoints.len() {
                self.datapoints[idx].calculate_poc();
            }
        }

        self.update_poc_status();
    }

    pub fn update_poc_status(&mut self) {
        let updates = self
            .datapoints
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| dp.poc_price().map(|price| (idx, price)))
            .collect::<Vec<_>>();

        let total_points = self.datapoints.len();

        for (current_idx, poc_price) in updates {
            let mut npoc = NPoc::default();

            for next_idx in (current_idx + 1)..total_points {
                let next_dp = &self.datapoints[next_idx];

                let next_dp_low = next_dp.kline.low.round_to_side_step(true, self.tick_size);
                let next_dp_high = next_dp.kline.high.round_to_side_step(false, self.tick_size);

                if next_dp_low <= poc_price && next_dp_high >= poc_price {
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
                let data_point = &mut self.datapoints[current_idx];
                data_point.set_poc_status(npoc);
            }
        }
    }

    pub fn min_max_price_in_range_prices(
        &self,
        earliest: usize,
        latest: usize,
    ) -> Option<(Price, Price)> {
        if earliest > latest {
            return None;
        }

        let mut min_p: Option<Price> = None;
        let mut max_p: Option<Price> = None;

        self.datapoints
            .iter()
            .rev()
            .enumerate()
            .filter(|(idx, _)| *idx >= earliest && *idx <= latest)
            .for_each(|(_, dp)| {
                let low = dp.kline.low;
                let high = dp.kline.high;

                min_p = Some(match min_p {
                    Some(value) => value.min(low),
                    None => low,
                });
                max_p = Some(match max_p {
                    Some(value) => value.max(high),
                    None => high,
                });
            });

        match (min_p, max_p) {
            (Some(low), Some(high)) => Some((low, high)),
            _ => None,
        }
    }

    pub fn min_max_price_in_range(&self, earliest: usize, latest: usize) -> Option<(f32, f32)> {
        self.min_max_price_in_range_prices(earliest, latest)
            .map(|(min_p, max_p)| (min_p.to_f32(), max_p.to_f32()))
    }

    pub fn max_qty_idx_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: usize,
        latest: usize,
        highest: Price,
        lowest: Price,
    ) -> f32 {
        let mut max_cluster_qty: f32 = 0.0;

        self.datapoints
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
            .datapoints
            .iter()
            .enumerate()
            .map(|(idx, dp)| (idx as u64, (dp.kline.volume.0, dp.kline.volume.1)))
            .collect()
    }
}
