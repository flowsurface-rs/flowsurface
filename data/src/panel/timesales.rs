use std::{collections::VecDeque, time::Duration};

use exchange::{
    SizeUnit, TickerInfo, Trade, UnixMs,
    unit::{Qty, qty::volume_size_unit},
};
use serde::{Deserialize, Serialize};

use crate::util::ok_or_default;

const TRADE_RETENTION_MS: u64 = 120_000;

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(deserialize_with = "ok_or_default", default)]
    pub ticker_display: TickerDisplay,
    #[serde(deserialize_with = "ok_or_default", default)]
    pub time_format: TimeFormat,
    pub trade_size_filter: f32,
    #[serde(default = "default_buffer_filter")]
    pub trade_retention: Duration,
    #[serde(deserialize_with = "ok_or_default", default)]
    pub stacked_bar: Option<StackedBar>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ticker_display: TickerDisplay::default(),
            time_format: TimeFormat::default(),
            trade_size_filter: 0.0,
            trade_retention: Duration::from_millis(TRADE_RETENTION_MS),
            stacked_bar: StackedBar::Compact(StackedBarRatio::default()).into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickerDisplay {
    #[default]
    Icon,
    IconAndTicker,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    #[default]
    MinutesSecondsMillis,
    HoursMinutesSecondsMillis,
    HoursMinutesSeconds,
}

impl TimeFormat {
    pub const ALL: [Self; 3] = [
        Self::MinutesSecondsMillis,
        Self::HoursMinutesSecondsMillis,
        Self::HoursMinutesSeconds,
    ];

    pub fn pattern(self) -> &'static str {
        match self {
            Self::MinutesSecondsMillis => "%M:%S.%3f",
            Self::HoursMinutesSecondsMillis => "%H:%M:%S.%3f",
            Self::HoursMinutesSeconds => "%H:%M:%S",
        }
    }
}

impl std::fmt::Display for TimeFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MinutesSecondsMillis => "mm:ss.ms",
            Self::HoursMinutesSecondsMillis => "hh:mm:ss.ms",
            Self::HoursMinutesSeconds => "hh:mm:ss",
        })
    }
}

fn default_buffer_filter() -> Duration {
    Duration::from_millis(TRADE_RETENTION_MS)
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

#[derive(Clone, Copy)]
pub struct TapeTrade {
    trade: Trade,
    ticker_info: TickerInfo,
    notional: Qty,
}

impl std::ops::Deref for TapeTrade {
    type Target = Trade;

    fn deref(&self) -> &Trade {
        &self.trade
    }
}

impl TapeTrade {
    pub fn new(ticker_info: TickerInfo, trade: Trade, size_unit: SizeUnit) -> Self {
        let notional = Qty::from_f64(ticker_info.market_type().qty_in_quote_value(
            trade.qty,
            trade.price,
            size_unit == SizeUnit::Quote,
        ));
        Self {
            trade,
            ticker_info,
            notional,
        }
    }

    pub fn ticker_info(&self) -> TickerInfo {
        self.ticker_info
    }

    pub fn quantity_text(&self, multiple_tickers: bool) -> String {
        format!(
            "{}{}",
            self.quantity_prefix(multiple_tickers, volume_size_unit()),
            crate::util::abbr_large_numbers(self.quantity(multiple_tickers).to_f64())
        )
    }

    pub fn quantity(&self, multiple_tickers: bool) -> Qty {
        if multiple_tickers {
            self.notional
        } else {
            self.qty
        }
    }

    pub fn passes_filter(&self, threshold: f32) -> bool {
        self.notional.to_f32_lossy() >= threshold
    }

    pub fn quantity_prefix(&self, multiple_tickers: bool, size_unit: SizeUnit) -> &'static str {
        if multiple_tickers
            || size_unit == SizeUnit::Quote
            || self.ticker_info.market_type() == exchange::adapter::MarketKind::InversePerps
        {
            "$"
        } else {
            ""
        }
    }

    pub fn merge_into(target: &mut VecDeque<Self>, mut incoming: Vec<Self>) {
        if incoming.is_empty() {
            return;
        }
        incoming.sort_by_key(|entry| entry.time);
        if target
            .back()
            .is_none_or(|last| last.time <= incoming[0].time)
        {
            target.extend(incoming);
            return;
        }
        let merge_start = target.partition_point(|entry| entry.time <= incoming[0].time);
        let mut previous = target.split_off(merge_start);
        for entry in incoming {
            while previous.front().is_some_and(|old| old.time <= entry.time) {
                target.push_back(previous.pop_front().unwrap());
            }
            target.push_back(entry);
        }
        target.append(&mut previous);
    }
}

pub fn prune_trades(trades: &mut VecDeque<TapeTrade>, retention: Duration, now: UnixMs) -> bool {
    let retention_ms = retention.as_millis() as u64;
    let prune_slack_ms = retention_ms / 10;
    let retention_cutoff = now.saturating_sub(retention_ms);
    let slack_cutoff = now.saturating_sub(retention_ms.saturating_add(prune_slack_ms));

    if trades
        .front()
        .is_none_or(|oldest| oldest.time >= slack_cutoff)
    {
        return false;
    }

    let previous_len = trades.len();
    while trades
        .front()
        .is_some_and(|trade| trade.time < retention_cutoff)
    {
        trades.pop_front();
    }
    trades.len() != previous_len
}

#[derive(Debug, Clone, Copy)]
pub enum HistAggValues {
    Count { buy: u64, sell: u64 },
    Qty { buy: Qty, sell: Qty },
}

#[derive(Default)]
pub struct HistAgg {
    buy_count: u64,
    sell_count: u64,
    buy_sum: Qty,
    sell_sum: Qty,
}

impl HistAgg {
    fn average_qty(sum: Qty, count: u64) -> Qty {
        if count == 0 {
            return Qty::ZERO;
        }

        let c = count.min(i64::MAX as u64) as i64;
        let half = c / 2;
        let rounded = if sum.units >= 0 {
            sum.units.saturating_add(half).div_euclid(c)
        } else {
            sum.units.saturating_sub(half).div_euclid(c)
        };

        Qty::from_units(rounded)
    }

    pub fn add(&mut self, is_sell: bool, qty: Qty) {
        if is_sell {
            self.sell_count += 1;
            self.sell_sum = Qty::from_units(self.sell_sum.units.saturating_add(qty.units));
        } else {
            self.buy_count += 1;
            self.buy_sum = Qty::from_units(self.buy_sum.units.saturating_add(qty.units));
        }
    }

    pub fn values_for(&self, ratio_kind: StackedBarRatio) -> Option<HistAggValues> {
        match ratio_kind {
            StackedBarRatio::Count => {
                let buy = self.buy_count;
                let sell = self.sell_count;
                let total = buy.saturating_add(sell);

                if total == 0 {
                    return None;
                }

                Some(HistAggValues::Count { buy, sell })
            }
            StackedBarRatio::Volume => {
                let buy = self.buy_sum.to_f32_lossy();
                let sell = self.sell_sum.to_f32_lossy();
                let total = buy + sell;

                if total <= 0.0 {
                    return None;
                }

                Some(HistAggValues::Qty {
                    buy: self.buy_sum,
                    sell: self.sell_sum,
                })
            }
            StackedBarRatio::AverageSize => {
                let buy_avg = Self::average_qty(self.buy_sum, self.buy_count);
                let sell_avg = Self::average_qty(self.sell_sum, self.sell_count);

                let denom = buy_avg.units.saturating_add(sell_avg.units);
                if denom <= 0 {
                    return None;
                }

                Some(HistAggValues::Qty {
                    buy: buy_avg,
                    sell: sell_avg,
                })
            }
        }
    }
}
