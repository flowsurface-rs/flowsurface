use crate::aggr;
use crate::chart::kline::footprint::{ClusterKind, KlineTrades};
use crate::chart::kline::poc::NPoc;
use exchange::unit::Qty;
use exchange::unit::price::{Price, PriceStep};
use exchange::{Kline, Trade, Volume};

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
            volume: Volume::empty_buy_sell().add_trade_qty(trade.is_sell, trade.qty),
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

        self.kline.volume = self.kline.volume.add_trade_qty(trade.is_sell, trade.qty);

        self.add_trade(trade, step);
    }

    fn add_trade(&mut self, trade: &Trade, step: PriceStep) {
        self.footprint.add_trade_to_nearest_bin(trade, step);
    }

    pub fn max_cluster_qty(&self, cluster_kind: ClusterKind, highest: Price, lowest: Price) -> Qty {
        self.footprint
            .max_cluster_qty(cluster_kind, highest, lowest)
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

#[derive(Debug, Clone)]
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

    pub fn change_tick_size(&mut self, tick_size: PriceStep, raw_trades: &[Trade]) {
        self.tick_size = tick_size;

        self.datapoints.clear();

        if !raw_trades.is_empty() {
            self.insert_trades(raw_trades);
        }
    }

    pub fn insert_trades(&mut self, buffer: &[Trade]) -> Option<usize> {
        if buffer.is_empty() {
            return None;
        }
        let first_updated = match self.datapoints.last() {
            Some(last) if !last.is_full(self.interval) => self.datapoints.len() - 1,
            _ => self.datapoints.len(),
        };

        for trade in buffer {
            if let Some(last) = self
                .datapoints
                .last_mut()
                .filter(|last| !last.is_full(self.interval))
            {
                last.update_with_trade(trade, self.tick_size);
            } else {
                self.datapoints
                    .push(TickAccumulation::new(trade, self.tick_size));
            }
        }
        for point in &mut self.datapoints[first_updated..] {
            point.calculate_poc();
        }
        Some(first_updated)
    }

    pub fn max_qty_idx_range(
        &self,
        cluster_kind: ClusterKind,
        earliest: usize,
        latest: usize,
        highest: Price,
        lowest: Price,
    ) -> Qty {
        let mut max_cluster_qty: Qty = Qty::default();

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
