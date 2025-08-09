use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use exchange::{adapter::MarketKind, depth::Depth};

use super::Basis;
use super::aggr::time::DataPoint;

pub const CLEANUP_THRESHOLD: usize = 4800;
const GRACE_PERIOD_MS: u64 = 500;

#[derive(Debug, Copy, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub trade_size_filter: f32,
    pub order_size_filter: f32,
    pub trade_size_scale: Option<i32>,
    pub coalescing: Option<CoalesceKind>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
            order_size_filter: 0.0,
            trade_size_scale: Some(100),
            coalescing: Some(CoalesceKind::Average(0.15)),
        }
    }
}

pub struct HeatmapDataPoint {
    pub grouped_trades: Box<[GroupedTrade]>,
    pub buy_sell: (f32, f32),
}

impl DataPoint for HeatmapDataPoint {
    fn add_trade(&mut self, trade: &exchange::Trade, tick_size: f32) {
        let grouped_price = if trade.is_sell {
            (trade.price * (1.0 / tick_size)).floor() * tick_size
        } else {
            (trade.price * (1.0 / tick_size)).ceil() * tick_size
        };

        match self
            .grouped_trades
            .binary_search_by(|probe| probe.compare_with(trade.price, trade.is_sell))
        {
            Ok(index) => self.grouped_trades[index].qty += trade.qty,
            Err(index) => {
                let mut trades = self.grouped_trades.to_vec();
                trades.insert(
                    index,
                    GroupedTrade {
                        is_sell: trade.is_sell,
                        price: grouped_price,
                        qty: trade.qty,
                    },
                );
                self.grouped_trades = trades.into_boxed_slice();
            }
        }

        if trade.is_sell {
            self.buy_sell.1 += trade.qty;
        } else {
            self.buy_sell.0 += trade.qty;
        }
    }

    fn clear_trades(&mut self) {
        self.grouped_trades = Box::new([]);
        self.buy_sell = (0.0, 0.0);
    }

    fn last_trade_time(&self) -> Option<u64> {
        None
    }

    fn first_trade_time(&self) -> Option<u64> {
        None
    }

    fn last_price(&self) -> f32 {
        self.grouped_trades.last().map_or(0.0, |t| t.price)
    }

    fn kline(&self) -> Option<&exchange::Kline> {
        None
    }

    fn value_high(&self) -> f32 {
        self.grouped_trades
            .iter()
            .map(|t| t.price)
            .fold(f32::MIN, f32::max)
    }

    fn value_low(&self) -> f32 {
        self.grouped_trades
            .iter()
            .map(|t| t.price)
            .fold(f32::MAX, f32::min)
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
                Basis::Time(interval) => interval.into(),
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
                if time > last_run.until_time + GRACE_PERIOD_MS {
                    last_run.until_time = last_run.start_time + self.aggr_time;

                    price_level.push(OrderRun {
                        start_time: time,
                        until_time: time + self.aggr_time,
                        qty: OrderedFloat(qty),
                        is_bid,
                    });
                    return;
                }

                let last_qty = last_run.qty.0;

                let qty_diff_pct = if last_qty > 0.0 {
                    (qty - last_qty).abs() / last_qty
                } else {
                    f32::INFINITY
                };

                if qty_diff_pct <= self.min_order_qty || last_run.qty == OrderedFloat(qty) {
                    last_run.until_time = time + self.aggr_time;
                } else {
                    if last_run.until_time > time {
                        last_run.until_time = time;
                    }
                    price_level.push(OrderRun {
                        start_time: time,
                        until_time: time + self.aggr_time,
                        qty: OrderedFloat(qty),
                        is_bid,
                    });
                }
            }
            Some(last_run) => {
                // side has flipped, end the previous run
                if last_run.until_time > time {
                    last_run.until_time = time;
                }
                // start a new run for the new side
                price_level.push(OrderRun {
                    start_time: time,
                    until_time: time + self.aggr_time,
                    qty: OrderedFloat(qty),
                    is_bid,
                });
            }
            None => {
                // no previous runs at this price level
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
            runs.retain(|run| run.until_time >= oldest_time);
        });

        self.price_levels.retain(|_, runs| !runs.is_empty());
    }

    pub fn coalesced_runs(
        &self,
        earliest: u64,
        latest: u64,
        highest: f32,
        lowest: f32,
        market_type: MarketKind,
        order_size_filter: f32,
        coalesce_kind: CoalesceKind,
    ) -> Vec<(OrderedFloat<f32>, OrderRun)> {
        let mut result_runs = Vec::new();

        let threshold_pct = match coalesce_kind {
            CoalesceKind::Average(t) | CoalesceKind::First(t) | CoalesceKind::Max(t) => t,
        };

        let size_in_quote_currency = exchange::SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

        for (price_at_level, runs_at_price_level) in
            self.iter_time_filtered(earliest, latest, highest, lowest)
        {
            let candidate_runs = runs_at_price_level
                .iter()
                .filter(|run_ref| {
                    if !(run_ref.until_time >= earliest && run_ref.start_time <= latest) {
                        return false;
                    }
                    let order_size = market_type.qty_in_quote_value(
                        run_ref.qty(),
                        **price_at_level,
                        size_in_quote_currency,
                    );
                    order_size > order_size_filter
                })
                .collect::<Vec<&OrderRun>>();

            if candidate_runs.is_empty() {
                continue;
            }

            let mut current_accumulator_opt: Option<CoalescingRun> = None;

            for run_to_process_ref in candidate_runs {
                let run_to_process = *run_to_process_ref;

                if let Some(current_accumulator) = current_accumulator_opt.as_mut() {
                    let comparison_base_qty = current_accumulator.comparison_qty(&coalesce_kind);

                    let qty_diff_pct = if comparison_base_qty > FRACTIONAL_THRESHOLD {
                        (run_to_process.qty() - comparison_base_qty).abs() / comparison_base_qty
                    } else if run_to_process.qty() > FRACTIONAL_THRESHOLD {
                        f32::INFINITY
                    } else {
                        0.0
                    };

                    if run_to_process.start_time <= current_accumulator.until_time
                        && run_to_process.is_bid == current_accumulator.is_bid
                        && qty_diff_pct <= threshold_pct
                    {
                        current_accumulator.merge_run(&run_to_process);
                    } else {
                        result_runs.push((
                            *price_at_level,
                            current_accumulator.to_order_run(&coalesce_kind),
                        ));
                        current_accumulator_opt = Some(CoalescingRun::new(&run_to_process));
                    }
                } else {
                    current_accumulator_opt = Some(CoalescingRun::new(&run_to_process));
                }
            }

            if let Some(accumulator) = current_accumulator_opt {
                result_runs.push((*price_at_level, accumulator.to_order_run(&coalesce_kind)));
            }
        }
        result_runs
    }

    pub fn query_grid_qtys(
        &self,
        center_time: u64,
        center_price: f32,
        time_interval_offsets: &[i64],
        price_tick_offsets: &[i64],
        market_type: MarketKind,
        order_size_filter: f32,
        coalesce_kind: Option<CoalesceKind>,
    ) -> HashMap<(u64, OrderedFloat<f32>), (f32, bool)> {
        let aggr_time = self.aggr_time;
        let tick_size: f32 = self.tick_size;

        let query_earliest_time = time_interval_offsets
            .iter()
            .map(|offset| center_time.saturating_add_signed(*offset * aggr_time as i64))
            .min()
            .unwrap_or(center_time);

        let query_latest_time = time_interval_offsets
            .iter()
            .map(|offset| center_time.saturating_add_signed(*offset * aggr_time as i64))
            .max()
            .map_or(center_time, |t| t.saturating_add(aggr_time));

        let query_lowest_price = price_tick_offsets
            .iter()
            .map(|offset| center_price + (*offset as f32 * tick_size))
            .fold(f32::INFINITY, f32::min)
            - 0.1 * tick_size;
        let query_highest_price = price_tick_offsets
            .iter()
            .map(|offset| center_price + (*offset as f32 * tick_size))
            .fold(f32::NEG_INFINITY, f32::max)
            + 0.1 * tick_size;

        let runs_in_vicinity = if let Some(ck) = coalesce_kind {
            self.coalesced_runs(
                query_earliest_time,
                query_latest_time,
                query_highest_price,
                query_lowest_price,
                market_type,
                order_size_filter,
                ck,
            )
        } else {
            self.iter_time_filtered(
                query_earliest_time,
                query_latest_time,
                query_highest_price,
                query_lowest_price,
            )
            .flat_map(|(price_level, runs_at_price)| {
                runs_at_price.iter().map(move |run| (*price_level, *run))
            })
            .collect()
        };

        let capacity = time_interval_offsets.len() * price_tick_offsets.len();
        let mut grid_quantities: HashMap<(u64, OrderedFloat<f32>), (f32, bool)> =
            HashMap::with_capacity(capacity);

        for price_offset in price_tick_offsets {
            let target_price_val = center_price + (*price_offset as f32 * tick_size);
            let target_price_key = OrderedFloat(target_price_val);
            for time_offset in time_interval_offsets {
                let target_time_val =
                    center_time.saturating_add_signed(*time_offset * aggr_time as i64);

                let current_grid_key = (target_time_val, target_price_key);

                for (run_price_level, run_data) in &runs_in_vicinity {
                    if (run_price_level.into_inner() - target_price_val).abs() < tick_size * 0.1
                        && run_data.start_time <= target_time_val
                        && run_data.until_time > target_time_val
                    {
                        grid_quantities.insert(current_grid_key, (run_data.qty(), run_data.is_bid));
                        break;
                    }
                }
            }
        }
        grid_quantities
    }

    pub fn max_depth_qty_in_range(
        &self,
        earliest: u64,
        latest: u64,
        highest: f32,
        lowest: f32,
        market_type: MarketKind,
        order_size_filter: f32,
    ) -> f32 {
        let mut max_depth_qty = 0.0f32;

        self.iter_time_filtered(earliest, latest, highest, lowest)
            .for_each(|(price, runs)| {
                runs.iter()
                    .filter_map(|run| {
                        let visible_run = run.with_range(earliest, latest)?;

                        let order_size = market_type.qty_in_quote_value(
                            visible_run.qty(),
                            **price,
                            exchange::SIZE_IN_QUOTE_CURRENCY.get() == Some(&true),
                        );

                        if order_size > order_size_filter {
                            Some(visible_run)
                        } else {
                            None
                        }
                    })
                    .for_each(|run| {
                        max_depth_qty = max_depth_qty.max(run.qty());
                    });
            });

        max_depth_qty
    }
}

const FRACTIONAL_THRESHOLD: f32 = 0.00001;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum CoalesceKind {
    First(f32),
    Average(f32),
    Max(f32),
}

impl CoalesceKind {
    pub fn threshold(&self) -> f32 {
        match self {
            CoalesceKind::Average(t) | CoalesceKind::First(t) | CoalesceKind::Max(t) => *t,
        }
    }

    pub fn with_threshold(&self, threshold: f32) -> Self {
        match self {
            CoalesceKind::First(_) => CoalesceKind::First(threshold),
            CoalesceKind::Average(_) => CoalesceKind::Average(threshold),
            CoalesceKind::Max(_) => CoalesceKind::Max(threshold),
        }
    }
}

impl PartialEq for CoalesceKind {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for CoalesceKind {}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct CoalescingRun {
    pub start_time: u64,
    pub until_time: u64,
    pub is_bid: bool,
    pub qty_sum: f32,
    pub run_count: u32,
    first_qty: f32,
    max_qty: f32,
}

impl CoalescingRun {
    pub fn new(run: &OrderRun) -> Self {
        let run_qty = run.qty();
        CoalescingRun {
            start_time: run.start_time,
            until_time: run.until_time,
            is_bid: run.is_bid,
            qty_sum: run_qty,
            run_count: 1,
            first_qty: run_qty,
            max_qty: run_qty,
        }
    }

    pub fn merge_run(&mut self, run: &OrderRun) {
        self.until_time = self.until_time.max(run.until_time);
        let run_qty = run.qty();
        self.qty_sum += run_qty;
        self.run_count += 1;
        self.max_qty = self.max_qty.max(run_qty);
    }

    pub fn comparison_qty(&self, kind: &CoalesceKind) -> f32 {
        match kind {
            CoalesceKind::Average(_) => self.current_average_qty(),
            CoalesceKind::Max(_) | CoalesceKind::First(_) => self.first_qty,
        }
    }

    pub fn current_average_qty(&self) -> f32 {
        if self.run_count == 0 {
            0.0
        } else {
            self.qty_sum / self.run_count as f32
        }
    }

    pub fn to_order_run(&self, kind: &CoalesceKind) -> OrderRun {
        let final_qty = match kind {
            CoalesceKind::Average(_) => self.current_average_qty(),
            CoalesceKind::First(_) => self.first_qty,
            CoalesceKind::Max(_) => self.max_qty,
        };
        OrderRun {
            start_time: self.start_time,
            until_time: self.until_time,
            qty: OrderedFloat(final_qty),
            is_bid: self.is_bid,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum HeatmapStudy {
    VolumeProfile(ProfileKind),
}

impl HeatmapStudy {
    pub const ALL: [HeatmapStudy; 1] = [HeatmapStudy::VolumeProfile(ProfileKind::VisibleRange)];
}

impl std::fmt::Display for HeatmapStudy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeatmapStudy::VolumeProfile(kind) => {
                write!(f, "Volume Profile ({})", kind)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ProfileKind {
    FixedWindow(usize),
    VisibleRange,
}

impl std::fmt::Display for ProfileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileKind::FixedWindow(_) => write!(f, "Fixed window"),
            ProfileKind::VisibleRange => write!(f, "Visible range"),
        }
    }
}
