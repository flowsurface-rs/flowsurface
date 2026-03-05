// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/91
use crate::aggr;
use crate::chart::kline::{ClusterKind, KlineTrades, NPoc};
use exchange::unit::Qty;
use exchange::unit::price::{Price, PriceStep};
use exchange::{Kline, Trade, Volume};

use std::collections::BTreeMap;

/// Microstructure data from precomputed range bars (ClickHouse).
/// Separate from Kline to avoid polluting the shared exchange type.
/// Serialize: range bar forensic telemetry (--features telemetry)
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct RangeBarMicrostructure {
    pub trade_count: u32,
    pub ofi: f32,
    pub trade_intensity: f32,
}

#[derive(Debug, Clone)]
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,
    pub footprint: KlineTrades,
    pub microstructure: Option<RangeBarMicrostructure>,
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
            microstructure: None,
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

    /// Range bar completion: `|close - open| / open >= threshold_dbps / 1_000_000`
    /// Uses integer Price units (i64, scale 10^8) for precision.
    pub fn is_full_range_bar(&self, threshold_dbps: u32) -> bool {
        let open = self.kline.open.units;
        if open == 0 {
            return false;
        }
        let diff = (self.kline.close.units - open).unsigned_abs();
        // deviation = diff / open, threshold = dbps / 1_000_000
        // => diff * 1_000_000 >= dbps * open
        diff as u128 * 1_000_000 >= threshold_dbps as u128 * open.unsigned_abs() as u128
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
    /// When set, `insert_trades()` uses price-based range bar completion
    /// instead of tick-count. Value is threshold in dbps.
    pub range_bar_threshold_dbps: Option<u32>,
}

impl TickAggr {
    pub fn new(interval: aggr::TickCount, tick_size: PriceStep, raw_trades: &[Trade]) -> Self {
        let mut tick_aggr = Self {
            datapoints: Vec::new(),
            interval,
            tick_size,
            range_bar_threshold_dbps: None,
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

    pub fn volume_data(&self) -> BTreeMap<u64, exchange::Volume> {
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

                let bar_complete = match self.range_bar_threshold_dbps {
                    Some(dbps) => self.datapoints[last_idx].is_full_range_bar(dbps),
                    None => self.datapoints[last_idx].is_full(self.interval),
                };
                if bar_complete {
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

    /// Create a TickAggr from precomputed klines (e.g., range bars from ClickHouse).
    /// Each kline becomes one TickAccumulation entry.
    /// The `interval` is set to TickCount(1) since each entry is already a complete bar.
    ///
    /// Klines are stored in ascending order (oldest at index 0, newest at end).
    /// `render_data_source` calls `.iter().rev().enumerate()` which gives
    /// (0, newest), (1, ...), ..., (N-1, oldest) — matching `interval_to_x`
    /// where index 0 maps to x=0 (rightmost = newest).
    pub fn from_klines(tick_size: PriceStep, klines: &[Kline]) -> Self {
        let datapoints: Vec<TickAccumulation> = klines
            .iter()
            .map(|kline| TickAccumulation {
                tick_count: 1,
                kline: *kline,
                footprint: KlineTrades::new(),
                microstructure: None,
            })
            .collect();

        Self {
            datapoints,
            interval: aggr::TickCount(1),
            tick_size,
            range_bar_threshold_dbps: None,
        }
    }

    /// Create a TickAggr from precomputed klines with microstructure sidecar data.
    /// `micro` must be the same length as `klines`.
    pub fn from_klines_with_microstructure(
        tick_size: PriceStep,
        klines: &[Kline],
        micro: &[Option<RangeBarMicrostructure>],
    ) -> Self {
        let datapoints: Vec<TickAccumulation> = klines
            .iter()
            .zip(micro.iter())
            .map(|(kline, m)| TickAccumulation {
                tick_count: 1,
                kline: *kline,
                footprint: KlineTrades::new(),
                microstructure: *m,
            })
            .collect();

        Self {
            datapoints,
            interval: aggr::TickCount(1),
            tick_size,
            range_bar_threshold_dbps: None,
        }
    }

    /// Delta (buy_volume - sell_volume) per bar, keyed by index.
    pub fn delta_data(&self) -> BTreeMap<u64, f32> {
        self.datapoints
            .iter()
            .enumerate()
            .map(|(idx, dp)| {
                let delta = dp
                    .kline
                    .volume
                    .buy_sell()
                    .map(|(b, s)| f32::from(b) - f32::from(s))
                    .unwrap_or(0.0);
                (idx as u64, delta)
            })
            .collect()
    }

    /// Individual trade count per bar from microstructure sidecar.
    pub fn trade_count_data(&self) -> BTreeMap<u64, f32> {
        self.datapoints
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| {
                dp.microstructure
                    .map(|m| (idx as u64, m.trade_count as f32))
            })
            .collect()
    }

    /// Order flow imbalance [-1, 1] per bar from microstructure sidecar.
    pub fn ofi_data(&self) -> BTreeMap<u64, f32> {
        self.datapoints
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| dp.microstructure.map(|m| (idx as u64, m.ofi)))
            .collect()
    }

    /// Trade intensity (trades/sec) per bar from microstructure sidecar.
    pub fn trade_intensity_data(&self) -> BTreeMap<u64, f32> {
        self.datapoints
            .iter()
            .enumerate()
            .filter_map(|(idx, dp)| dp.microstructure.map(|m| (idx as u64, m.trade_intensity)))
            .collect()
    }

    /// Prepend older klines to the datapoints (for historical data loading).
    /// Klines should be in ascending timestamp order.
    /// Older data goes at the beginning (index 0) since datapoints is oldest-first.
    /// Filters out any klines whose timestamp already exists in datapoints.
    pub fn prepend_klines(&mut self, klines: &[Kline]) {
        self.prepend_klines_with_microstructure(klines, None);
    }

    pub fn prepend_klines_with_microstructure(
        &mut self,
        klines: &[Kline],
        micro: Option<&[Option<RangeBarMicrostructure>]>,
    ) {
        // Deduplicate: only prepend klines older than the current oldest bar
        let oldest_existing_ts = self.datapoints.first().map(|dp| dp.kline.time);
        let new_entries: Vec<TickAccumulation> = klines
            .iter()
            .enumerate()
            .filter(|(_, kline)| oldest_existing_ts.is_none_or(|oldest| kline.time < oldest))
            .map(|(i, kline)| TickAccumulation {
                tick_count: 1,
                kline: *kline,
                footprint: KlineTrades::new(),
                microstructure: micro.and_then(|m| m.get(i).copied().flatten()),
            })
            .collect();

        if !new_entries.is_empty() {
            self.datapoints.splice(0..0, new_entries);
        }
    }

    /// Append a newer kline to the end of datapoints (for streaming updates).
    /// Skips if the timestamp already exists (dedup against streaming overlap).
    pub fn append_kline(&mut self, kline: &Kline) {
        let newest_ts = self.datapoints.last().map(|dp| dp.kline.time);
        if newest_ts.is_none_or(|ts| kline.time > ts) {
            self.datapoints.push(TickAccumulation {
                tick_count: 1,
                kline: *kline,
                footprint: KlineTrades::new(),
                microstructure: None,
            });
        }
    }

    /// Replace the last datapoint if timestamps match, otherwise append.
    /// Used for reconciling ClickHouse completed bars with locally-built forming bars.
    /// - Same timestamp → replace OHLCV with authoritative ClickHouse data
    /// - Newer timestamp → append as new completed bar
    /// - Older timestamp → ignore (stale)
    pub fn replace_or_append_kline(&mut self, kline: &Kline) {
        #[cfg(feature = "telemetry")]
        {
            use crate::telemetry::{self, KlineSnapshot, ReconcileAction, TelemetryEvent};
            let action = match self.datapoints.last() {
                Some(last) if kline.time == last.kline.time => ReconcileAction::Replace,
                Some(last) if kline.time > last.kline.time => ReconcileAction::Append,
                Some(_) => ReconcileAction::Drop,
                None => ReconcileAction::AppendEmpty,
            };
            let existing_last = self
                .datapoints
                .last()
                .map(|dp| KlineSnapshot::from_kline(&dp.kline));
            let micro_before = self.datapoints.last().and_then(|dp| dp.microstructure);

            // Emit MicroLoss if we're about to replace a bar that has microstructure
            if matches!(action, ReconcileAction::Replace)
                && let Some(micro) = micro_before
            {
                telemetry::emit(TelemetryEvent::MicroLoss {
                    ts_ms: telemetry::now_ms(),
                    bar_time_ms: kline.time,
                    micro_before: micro,
                });
            }

            telemetry::emit(TelemetryEvent::Reconcile {
                ts_ms: telemetry::now_ms(),
                action,
                incoming: KlineSnapshot::from_kline(kline),
                existing_last,
                micro_before,
            });
        }

        if let Some(last) = self.datapoints.last_mut() {
            if kline.time == last.kline.time {
                let preserved_micro = last.microstructure;
                last.kline = *kline;
                // Preserve locally-built microstructure when ClickHouse bar
                // replaces the forming bar (CH doesn't carry micro in poll stream).
                last.microstructure = preserved_micro;
            } else if kline.time > last.kline.time {
                self.datapoints.push(TickAccumulation {
                    tick_count: 1,
                    kline: *kline,
                    footprint: KlineTrades::new(),
                    microstructure: None,
                });
            }
        } else {
            self.datapoints.push(TickAccumulation {
                tick_count: 1,
                kline: *kline,
                footprint: KlineTrades::new(),
                microstructure: None,
            });
        }
    }

    pub fn min_max_price_in_range_prices(
        &self,
        earliest: usize,
        latest: usize,
    ) -> Option<(Price, Price)> {
        let (earliest, latest) = if earliest > latest {
            (latest, earliest)
        } else {
            (earliest, latest)
        };

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
    ) -> Qty {
        let (earliest, latest) = if earliest > latest {
            (latest, earliest)
        } else {
            (earliest, latest)
        };
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

impl From<&TickAggr> for BTreeMap<u64, exchange::Volume> {
    /// Converts datapoints into a map of timestamps and volume data
    fn from(tick_aggr: &TickAggr) -> Self {
        tick_aggr
            .datapoints
            .iter()
            .enumerate()
            .map(|(idx, dp)| (idx as u64, dp.kline.volume))
            .collect()
    }
}
