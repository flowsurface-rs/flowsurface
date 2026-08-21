use crate::chart::Basis;
use exchange::unit::Qty;
use exchange::{Kline, OpenInterest, Trade, UnixMs};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KlinePriceSource {
    Open,
    High,
    Low,
    Close,
    Hlc3,
    Ohlc4,
}

impl KlinePriceSource {
    pub const ALL: [Self; 6] = [
        Self::Open,
        Self::High,
        Self::Low,
        Self::Close,
        Self::Hlc3,
        Self::Ohlc4,
    ];

    pub fn value(self, bar: &Kline) -> f64 {
        let open = bar.open.to_f64();
        let high = bar.high.to_f64();
        let low = bar.low.to_f64();
        let close = bar.close.to_f64();

        match self {
            Self::Open => open,
            Self::High => high,
            Self::Low => low,
            Self::Close => close,
            Self::Hlc3 => (high + low + close) / 3.0,
            Self::Ohlc4 => (open + high + low + close) / 4.0,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::High => "High",
            Self::Low => "Low",
            Self::Close => "Close",
            Self::Hlc3 => "HLC3",
            Self::Ohlc4 => "OHLC4",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "Open" => Some(Self::Open),
            "High" => Some(Self::High),
            "Low" => Some(Self::Low),
            "Close" => Some(Self::Close),
            "HLC3" => Some(Self::Hlc3),
            "OHLC4" => Some(Self::Ohlc4),
            _ => None,
        }
    }
}

impl Default for KlinePriceSource {
    fn default() -> Self {
        Self::Close
    }
}

impl Display for KlinePriceSource {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndicatorInputSource {
    Klines,
    Trades,
    OpenInterest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TradeBarValues {
    pub trade_count: u64,
    pub buy_qty: Qty,
    pub sell_qty: Qty,
}

impl TradeBarValues {
    pub fn add_trade(&mut self, trade: &Trade) {
        self.trade_count += 1;
        if trade.is_sell {
            self.sell_qty += trade.qty;
        } else {
            self.buy_qty += trade.qty;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizedTradeBar {
    pub time: UnixMs,
    pub values: Option<TradeBarValues>,
}

pub const KLINE_INPUTS: &[IndicatorInputSource] = &[IndicatorInputSource::Klines];
pub const CVD_INPUTS: &[IndicatorInputSource] =
    &[IndicatorInputSource::Klines, IndicatorInputSource::Trades];
pub const TRADE_INPUTS: &[IndicatorInputSource] = &[IndicatorInputSource::Trades];
pub const OPEN_INTEREST_INPUTS: &[IndicatorInputSource] = &[IndicatorInputSource::OpenInterest];

#[derive(Debug, Clone, Copy)]
pub enum IndicatorInput<'a> {
    Klines {
        bars: &'a [Kline],
        times: Option<&'a [UnixMs]>,
    },
    Trades {
        bars: &'a [NormalizedTradeBar],
    },
    OpenInterest {
        data: &'a [OpenInterest],
        basis: Basis,
    },
}

impl IndicatorInput<'_> {
    pub const fn source(self) -> IndicatorInputSource {
        match self {
            Self::Klines { .. } => IndicatorInputSource::Klines,
            Self::Trades { .. } => IndicatorInputSource::Trades,
            Self::OpenInterest { .. } => IndicatorInputSource::OpenInterest,
        }
    }
}
