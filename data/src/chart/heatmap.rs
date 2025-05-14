use std::collections::BTreeMap;

use exchange::depth::Depth;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use super::Basis;

pub const MIN_SCALING: f32 = 0.6;
pub const MAX_SCALING: f32 = 1.2;

pub const MAX_CELL_WIDTH: f32 = 12.0;
pub const MIN_CELL_WIDTH: f32 = 1.0;

pub const MAX_CELL_HEIGHT: f32 = 10.0;
pub const MIN_CELL_HEIGHT: f32 = 1.0;

pub const DEFAULT_CELL_WIDTH: f32 = 3.0;

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
    pub order_size_filter: f32,
    pub dynamic_sized_trades: bool,
    pub trade_size_scale: i32,
    pub smoothing_pct: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
            order_size_filter: 0.0,
            dynamic_sized_trades: true,
            trade_size_scale: 100,
            smoothing_pct: Some(0.15),
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct OrderRun {
    pub start_time: u64,
    pub until_time: u64,
    qty: OrderedFloat<f32>,
    pub is_bid: bool,
}

impl OrderRun {
    pub fn qty(&self) -> f32 {
        self.qty.into_inner()
    }

    pub fn with_range(&self, earliest: u64, latest: u64) -> Option<&OrderRun> {
        if self.start_time <= latest && self.until_time >= earliest {
            Some(self)
        } else {
            None
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct HistoricalDepth {
    price_levels: BTreeMap<OrderedFloat<f32>, Vec<OrderRun>>,
    aggr_time: u64,
    tick_size: f32,
    min_order_qty: f32,
}

impl HistoricalDepth {
    pub fn new(min_order_qty: f32, tick_size: f32, basis: Basis) -> Self {
        Self {
            price_levels: BTreeMap::new(),
            aggr_time: match basis {
                Basis::Time(interval) => interval,
                Basis::Tick(_) => unimplemented!(),
            },
            tick_size,
            min_order_qty,
        }
    }

    pub fn insert_latest_depth(&mut self, depth: &Depth, time: u64) {
        let tick_size = self.tick_size;

        self.process_side(&depth.bids, time, true, |price| {
            ((price * (1.0 / tick_size)).floor()) * tick_size
        });
        self.process_side(&depth.asks, time, false, |price| {
            ((price * (1.0 / tick_size)).ceil()) * tick_size
        });
    }

    fn process_side<F>(
        &mut self,
        side: &BTreeMap<OrderedFloat<f32>, f32>,
        time: u64,
        is_bid: bool,
        round_price: F,
    ) where
        F: Fn(f32) -> f32,
    {
        let mut current_price = None;
        let mut current_qty = 0.0;

        for (price, qty) in side {
            let rounded_price = round_price(price.into_inner());

            if Some(rounded_price) == current_price {
                current_qty += qty;
            } else {
                if let Some(price) = current_price {
                    self.update_price_level(time, price, current_qty, is_bid);
                }
                current_price = Some(rounded_price);
                current_qty = *qty;
            }
        }

        if let Some(price) = current_price {
            self.update_price_level(time, price, current_qty, is_bid);
        }
    }

    fn update_price_level(&mut self, time: u64, price: f32, qty: f32, is_bid: bool) {
        let price_level = self.price_levels.entry(OrderedFloat(price)).or_default();

        match price_level.last_mut() {
            Some(last_run) if last_run.is_bid == is_bid => {
                let last_qty = last_run.qty.0;
                let qty_diff_pct = if last_qty > 0.0 {
                    (qty - last_qty).abs() / last_qty
                } else {
                    f32::INFINITY
                };

                if qty_diff_pct <= self.min_order_qty || last_run.qty == OrderedFloat(qty) {
                    last_run.until_time = time + self.aggr_time;
                } else {
                    price_level.push(OrderRun {
                        start_time: time,
                        until_time: time + self.aggr_time,
                        qty: OrderedFloat(qty),
                        is_bid,
                    });
                }
            }
            _ => {
                price_level.push(OrderRun {
                    start_time: time,
                    until_time: time + self.aggr_time,
                    qty: OrderedFloat(qty),
                    is_bid,
                });
            }
        }
    }

    pub fn iter_time_filtered(
        &self,
        earliest: u64,
        latest: u64,
        highest: f32,
        lowest: f32,
    ) -> impl Iterator<Item = (&OrderedFloat<f32>, &Vec<OrderRun>)> {
        self.price_levels
            .range(OrderedFloat(lowest)..=OrderedFloat(highest))
            .filter(move |(_, runs)| {
                runs.iter()
                    .any(|run| run.until_time >= earliest && run.start_time <= latest)
            })
    }

    pub fn latest_order_runs(
        &self,
        highest: f32,
        lowest: f32,
        latest_timestamp: u64,
    ) -> impl Iterator<Item = (&OrderedFloat<f32>, &OrderRun)> {
        self.price_levels
            .range(OrderedFloat(lowest)..=OrderedFloat(highest))
            .filter_map(move |(price, runs)| {
                runs.last()
                    .filter(|run| run.until_time >= latest_timestamp)
                    .map(|run| (price, run))
            })
    }

    pub fn cleanup_old_price_levels(&mut self, oldest_time: u64) {
        self.price_levels.iter_mut().for_each(|(_, runs)| {
            runs.retain(|run| run.start_time >= oldest_time);
        });

        self.price_levels.retain(|_, runs| !runs.is_empty());
    }
}

#[derive(Default)]
pub struct QtyScale {
    pub max_trade_qty: f32,
    pub max_aggr_volume: f32,
    pub max_depth_qty: f32,
}

#[derive(Debug, Clone)]
pub struct GroupedTrade {
    pub is_sell: bool,
    pub price: f32,
    pub qty: f32,
}

impl GroupedTrade {
    pub fn compare_with(&self, price: f32, is_sell: bool) -> std::cmp::Ordering {
        if self.is_sell == is_sell {
            self.price
                .partial_cmp(&price)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            self.is_sell.cmp(&is_sell)
        }
    }
}
