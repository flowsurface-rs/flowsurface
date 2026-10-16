use crate::chart::Basis;
use crate::config::theme::{ThemeColor, ThemeColorRole, ThemeColorShade};
use exchange::{
    SizeUnit, TickerInfo, Trade, UnixMs,
    unit::{Price, qty::volume_size_unit},
};

pub const DEFAULT_BIG_TRADES_THRESHOLD: u64 = 2_500_000;
pub const MIN_BIG_TRADES_THRESHOLD: u64 = 10_000;
pub const MAX_BIG_TRADES_THRESHOLD: u64 = 25_000_000;
const BIG_TRADES_MANTISSAS: [u64; 9] = [10, 15, 20, 25, 30, 40, 50, 60, 75];

const fn big_trades_nice_threshold(step: usize) -> u64 {
    let mut scale = MIN_BIG_TRADES_THRESHOLD / BIG_TRADES_MANTISSAS[0];
    let mut decade = step / BIG_TRADES_MANTISSAS.len();
    while decade > 0 {
        scale *= 10;
        decade -= 1;
    }
    BIG_TRADES_MANTISSAS[step % BIG_TRADES_MANTISSAS.len()] * scale
}

pub const fn big_trades_step_count() -> usize {
    let mut count = 0;
    while big_trades_nice_threshold(count) <= MAX_BIG_TRADES_THRESHOLD {
        count += 1;
    }
    count
}

const BIG_TRADES_THRESHOLDS: [u64; big_trades_step_count()] = {
    let mut thresholds = [0_u64; big_trades_step_count()];
    let mut step = 0;
    while step < thresholds.len() {
        thresholds[step] = big_trades_nice_threshold(step);
        step += 1;
    }
    thresholds
};

pub(super) fn default_big_trades_threshold() -> u64 {
    DEFAULT_BIG_TRADES_THRESHOLD
}

pub(super) const fn default_big_trades_radius_scale() -> Option<i32> {
    Some(100)
}

pub(super) const fn default_big_trades_buy_color() -> ThemeColor {
    ThemeColor::new(ThemeColorRole::Success, ThemeColorShade::Soft)
}

pub(super) const fn default_big_trades_sell_color() -> ThemeColor {
    ThemeColor::new(ThemeColorRole::Danger, ThemeColorShade::Soft)
}

pub fn big_trades_threshold_at_step(step: usize) -> u64 {
    BIG_TRADES_THRESHOLDS[step.min(BIG_TRADES_THRESHOLDS.len() - 1)]
}

pub fn big_trades_step_for_threshold(threshold: u64) -> usize {
    let threshold = threshold.clamp(MIN_BIG_TRADES_THRESHOLD, MAX_BIG_TRADES_THRESHOLD);
    let target = (threshold as f64).ln();
    BIG_TRADES_THRESHOLDS
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| {
            distance_in_log_space(**left, target).total_cmp(&distance_in_log_space(**right, target))
        })
        .map(|(index, _)| index)
        .unwrap_or_default()
}

fn distance_in_log_space(candidate: u64, target: f64) -> f64 {
    ((candidate as f64).ln() - target).abs()
}

pub fn big_trades_threshold_label(threshold: u64) -> String {
    format!(">${}", crate::util::abbr_large_numbers(threshold as f64))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigTradesConfig {
    pub threshold: u64,
    pub radius_scale: Option<i32>,
    pub buy_color: ThemeColor,
    pub sell_color: ThemeColor,
}

#[derive(Debug, Clone, Copy)]
pub struct BigTradeEntry {
    pub index: usize,
    pub time: UnixMs,
    pub price: Price,
    pub notional: f64,
    pub is_sell: bool,
}

pub(crate) struct BigTradesCache {
    threshold: u64,
    entries: Vec<BigTradeEntry>,
    consumed: usize,
}

impl BigTradesCache {
    pub(crate) fn new(
        threshold: u64,
        trades: &[Trade],
        ticker_info: &TickerInfo,
        basis: Basis,
    ) -> Self {
        let mut cache = Self {
            threshold,
            entries: Vec::new(),
            consumed: 0,
        };
        cache.fold(trades, ticker_info, basis);
        cache
    }

    pub(crate) fn threshold(&self) -> u64 {
        self.threshold
    }

    pub(crate) fn in_time_range(&self, start: UnixMs, end: UnixMs) -> &[BigTradeEntry] {
        let lo = self.entries.partition_point(|entry| entry.time < start);
        let hi = self.entries.partition_point(|entry| entry.time <= end);
        &self.entries[lo..hi]
    }

    pub(crate) fn in_index_range(&self, lo: usize, hi: usize) -> &[BigTradeEntry] {
        let start = self.entries.partition_point(|entry| entry.index < lo);
        let end = self.entries.partition_point(|entry| entry.index < hi);
        &self.entries[start..end]
    }

    pub(crate) fn fold(&mut self, trades: &[Trade], ticker_info: &TickerInfo, basis: Basis) {
        let start = self.consumed.min(trades.len());
        let previous_len = self.entries.len();
        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
        let market_type = ticker_info.market_type();
        let threshold = self.threshold as f64;
        self.entries.extend(
            trades[start..]
                .iter()
                .enumerate()
                .filter_map(|(offset, trade)| {
                    let notional =
                        market_type.qty_in_quote_value(trade.qty, trade.price, size_in_quote_ccy);
                    (notional >= threshold).then_some(BigTradeEntry {
                        index: start + offset,
                        time: trade.time,
                        price: trade.price,
                        notional,
                        is_sell: trade.is_sell,
                    })
                }),
        );
        if basis.is_time()
            && !self.entries[previous_len.saturating_sub(1)..]
                .is_sorted_by_key(|entry| (entry.time, entry.index))
        {
            self.entries
                .sort_unstable_by_key(|entry| (entry.time, entry.index));
        }
        self.consumed = trades.len();
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.consumed = 0;
    }
}
