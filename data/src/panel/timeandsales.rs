use std::time::Duration;

use exchange::util::Price;
use serde::{Deserialize, Serialize};

use crate::util::ok_or_default;

const TRADE_RETENTION_MS: u64 = 120_000;

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub trade_size_filter: f32,
    #[serde(default = "default_buffer_filter")]
    pub trade_retention: Duration,
    #[serde(deserialize_with = "ok_or_default", default)]
    pub stacked_bar: Option<StackedBar>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trade_size_filter: 0.0,
            trade_retention: Duration::from_millis(TRADE_RETENTION_MS),
            stacked_bar: StackedBar::Compact(StackedBarRatio::default()).into(),
        }
    }
}

fn default_buffer_filter() -> Duration {
    Duration::from_millis(TRADE_RETENTION_MS)
}

#[derive(Debug, Clone)]
pub struct TradeDisplay {
    pub time_str: String,
    pub price: Price,
    pub qty: f32,
    pub is_sell: bool,
}

#[derive(Debug, Clone)]
pub struct TradeEntry {
    pub ts_ms: u64,
    pub display: TradeDisplay,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Copy)]
pub enum StackedBar {
    Compact(StackedBarRatio),
    Full(StackedBarRatio),
}

impl StackedBar {
    pub fn ratio(self) -> StackedBarRatio {
        match self {
            StackedBar::Compact(r) | StackedBar::Full(r) => r,
        }
    }

    pub fn with_ratio(self, r: StackedBarRatio) -> Self {
        match self {
            StackedBar::Compact(_) => StackedBar::Compact(r),
            StackedBar::Full(_) => StackedBar::Full(r),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default, Copy)]
pub enum StackedBarRatio {
    Count,
    #[default]
    Volume,
    AverageSize,
}

impl std::fmt::Display for StackedBarRatio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StackedBarRatio::Count => write!(f, "Count"),
            StackedBarRatio::AverageSize => write!(f, "Average trade size"),
            StackedBarRatio::Volume => write!(f, "Volume"),
        }
    }
}

impl StackedBarRatio {
    pub const ALL: [StackedBarRatio; 3] = [
        StackedBarRatio::Count,
        StackedBarRatio::Volume,
        StackedBarRatio::AverageSize,
    ];
}

#[derive(Default)]
pub struct HistAgg {
    buy_count: u64,
    sell_count: u64,
    buy_sum: f64,
    sell_sum: f64,
}

impl HistAgg {
    pub fn add(&mut self, trade: &TradeDisplay) {
        let qty = trade.qty as f64;

        if trade.is_sell {
            self.sell_count += 1;
            self.sell_sum += qty;
        } else {
            self.buy_count += 1;
            self.buy_sum += qty;
        }
    }

    pub fn remove(&mut self, trade: &TradeDisplay) {
        let qty = trade.qty as f64;

        if trade.is_sell {
            self.sell_count = self.sell_count.saturating_sub(1);
            self.sell_sum -= qty;
        } else {
            self.buy_count = self.buy_count.saturating_sub(1);
            self.buy_sum -= qty;
        }
    }

    pub fn values_for(&self, ratio_kind: StackedBarRatio) -> Option<(f64, f64, f32)> {
        match ratio_kind {
            StackedBarRatio::Count => {
                let buy = self.buy_count as f64;
                let sell = self.sell_count as f64;
                let total = buy + sell;

                if total <= 0.0 {
                    return None;
                }
                let buy_ratio = (buy / total) as f32;

                Some((buy, sell, buy_ratio))
            }
            StackedBarRatio::Volume => {
                let buy = self.buy_sum;
                let sell = self.sell_sum;
                let total = buy + sell;

                if total <= 0.0 {
                    return None;
                }
                let buy_ratio = (buy / total) as f32;

                Some((buy, sell, buy_ratio))
            }
            StackedBarRatio::AverageSize => {
                let buy_avg = if self.buy_count > 0 {
                    self.buy_sum / self.buy_count as f64
                } else {
                    0.0
                };
                let sell_avg = if self.sell_count > 0 {
                    self.sell_sum / self.sell_count as f64
                } else {
                    0.0
                };

                let denom = buy_avg + sell_avg;
                if denom <= 0.0 {
                    return None;
                }
                let buy_ratio = (buy_avg / denom) as f32;

                Some((buy_avg, sell_avg, buy_ratio))
            }
        }
    }
}
