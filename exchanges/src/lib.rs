pub mod adapter;
pub mod connect;
pub mod depth;

use adapter::{Exchange, MarketType, StreamType};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{
    fmt::{self, Write},
    hash::Hash,
};

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Timeframe::M1 => "1m",
                Timeframe::M3 => "3m",
                Timeframe::M5 => "5m",
                Timeframe::M15 => "15m",
                Timeframe::M30 => "30m",
                Timeframe::H1 => "1h",
                Timeframe::H2 => "2h",
                Timeframe::H4 => "4h",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Timeframe {
    M1,
    M3,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
}

impl Timeframe {
    pub const ALL: [Timeframe; 8] = [
        Timeframe::M1,
        Timeframe::M3,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H2,
        Timeframe::H4,
    ];

    pub fn to_minutes(self) -> u16 {
        match self {
            Timeframe::M1 => 1,
            Timeframe::M3 => 3,
            Timeframe::M5 => 5,
            Timeframe::M15 => 15,
            Timeframe::M30 => 30,
            Timeframe::H1 => 60,
            Timeframe::H2 => 120,
            Timeframe::H4 => 240,
        }
    }

    pub fn to_milliseconds(self) -> u64 {
        u64::from(self.to_minutes()) * 60_000
    }
}

impl From<Timeframe> for f32 {
    fn from(timeframe: Timeframe) -> f32 {
        timeframe.to_milliseconds() as f32
    }
}

impl From<Timeframe> for u64 {
    fn from(timeframe: Timeframe) -> u64 {
        timeframe.to_milliseconds()
    }
}

impl From<u64> for Timeframe {
    fn from(milliseconds: u64) -> Timeframe {
        match milliseconds {
            60_000 => Timeframe::M1,
            180_000 => Timeframe::M3,
            300_000 => Timeframe::M5,
            900_000 => Timeframe::M15,
            1_800_000 => Timeframe::M30,
            3_600_000 => Timeframe::H1,
            7_200_000 => Timeframe::H2,
            14_400_000 => Timeframe::H4,
            _ => panic!("Invalid timeframe: {milliseconds}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Ticker {
    data: [u64; 2],
    len: u8,
    pub market_type: MarketType,
}

impl Ticker {
    pub fn new<S: AsRef<str>>(ticker: S, market_type: MarketType) -> Self {
        let ticker = ticker.as_ref();
        let base_len = ticker.len();

        assert!(base_len <= 20, "Ticker too long");
        assert!(
            ticker.chars().all(|c| c.is_ascii_alphanumeric()),
            "Invalid character in ticker: {ticker:?}"
        );

        let mut data = [0u64; 2];
        let mut len = 0;

        for (i, c) in ticker.bytes().enumerate() {
            let value = match c {
                b'0'..=b'9' => c - b'0',
                b'A'..=b'Z' => c - b'A' + 10,
                _ => unreachable!(),
            };
            let shift = (i % 10) * 6;
            data[i / 10] |= u64::from(value) << shift;
            len += 1;
        }

        Ticker {
            data,
            len,
            market_type,
        }
    }

    pub fn get_string(&self) -> (String, MarketType) {
        let mut result = String::with_capacity(self.len as usize);
        for i in 0..self.len {
            let value = (self.data[i as usize / 10] >> ((i % 10) * 6)) & 0x3F;
            let c = match value {
                0..=9 => (b'0' + value as u8) as char,
                10..=35 => (b'A' + (value as u8 - 10)) as char,
                _ => unreachable!(),
            };
            result.push(c);
        }

        (result, self.market_type)
    }
}

impl fmt::Display for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Direct formatting without intermediate String allocation
        for i in 0..self.len {
            let value = (self.data[i as usize / 10] >> ((i % 10) * 6)) & 0x3F;
            let c = match value {
                0..=9 => (b'0' + value as u8) as char,
                10..=35 => (b'A' + (value as u8 - 10)) as char,
                _ => unreachable!(),
            };
            f.write_char(c)?;
        }

        Ok(())
    }
}

impl From<(String, MarketType)> for Ticker {
    fn from((s, market_type): (String, MarketType)) -> Self {
        Ticker::new(s, market_type)
    }
}

impl From<(&str, MarketType)> for Ticker {
    fn from((s, market_type): (&str, MarketType)) -> Self {
        Ticker::new(s, market_type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct TickerInfo {
    pub ticker: Ticker,
    #[serde(rename = "tickSize")]
    pub min_ticksize: f32,
}

impl TickerInfo {
    pub fn get_market_type(&self) -> MarketType {
        self.ticker.market_type
    }
}

#[derive(Default, Debug, Clone, Copy, Deserialize)]
pub struct Trade {
    pub time: u64,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_sell: bool,
    pub price: f32,
    pub qty: f32,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Kline {
    pub time: u64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: (f32, f32),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct TickerStats {
    pub mark_price: f32,
    pub daily_price_chg: f32,
    pub daily_volume: f32,
}

fn bool_from_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value.as_i64() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(serde::de::Error::custom("expected 0 or 1")),
    }
}

fn de_string_to_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse::<f32>().map_err(serde::de::Error::custom)
}

fn de_string_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse::<u64>().map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenInterest {
    pub time: u64,
    pub value: f32,
}

fn str_f32_parse(s: &str) -> f32 {
    s.parse::<f32>().unwrap_or_else(|e| {
        log::error!("Failed to parse float: {}, error: {}", s, e);
        0.0
    })
}
