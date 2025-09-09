pub mod adapter;
pub mod connect;
pub mod depth;
pub mod fetcher;
mod limiter;

pub use adapter::Event;
use adapter::{Exchange, MarketKind, StreamKind};

use rust_decimal::{
    Decimal,
    prelude::{FromPrimitive, ToPrimitive},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use std::sync::OnceLock;
use std::{fmt, hash::Hash};

pub static SIZE_IN_QUOTE_CURRENCY: OnceLock<bool> = OnceLock::new();

pub fn is_flag_enabled() -> bool {
    *SIZE_IN_QUOTE_CURRENCY.get().unwrap_or(&false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum PreferredCurrency {
    Quote,
    Base,
}

pub fn set_size_in_quote_currency(preferred: PreferredCurrency) {
    let enabled = match preferred {
        PreferredCurrency::Quote => true,
        PreferredCurrency::Base => false,
    };

    SIZE_IN_QUOTE_CURRENCY
        .set(enabled)
        .expect("Failed to set SIZE_IN_QUOTE_CURRENCY");
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Timeframe::MS100 => "100ms",
                Timeframe::MS200 => "200ms",
                Timeframe::MS500 => "500ms",
                Timeframe::MS1000 => "1s",
                Timeframe::M1 => "1m",
                Timeframe::M3 => "3m",
                Timeframe::M5 => "5m",
                Timeframe::M15 => "15m",
                Timeframe::M30 => "30m",
                Timeframe::H1 => "1h",
                Timeframe::H2 => "2h",
                Timeframe::H4 => "4h",
                Timeframe::H6 => "6h",
                Timeframe::H12 => "12h",
                Timeframe::D1 => "1d",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, PartialOrd, Ord)]
pub enum Timeframe {
    MS100,
    MS200,
    MS500,
    MS1000,
    M1,
    M3,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
    H6,
    H12,
    D1,
}

impl Timeframe {
    pub const KLINE: [Timeframe; 11] = [
        Timeframe::M1,
        Timeframe::M3,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H2,
        Timeframe::H4,
        Timeframe::H6,
        Timeframe::H12,
        Timeframe::D1,
    ];

    pub const HEATMAP: [Timeframe; 4] = [
        Timeframe::MS100,
        Timeframe::MS200,
        Timeframe::MS500,
        Timeframe::MS1000,
    ];

    /// # Panics
    ///
    /// Will panic if the `Timeframe` is not one of the defined variants
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
            Timeframe::H6 => 360,
            Timeframe::H12 => 720,
            Timeframe::D1 => 1440,
            _ => panic!("Invalid timeframe: {:?}", self),
        }
    }

    pub fn to_milliseconds(self) -> u64 {
        match self {
            Timeframe::MS100 => 100,
            Timeframe::MS200 => 200,
            Timeframe::MS500 => 500,
            Timeframe::MS1000 => 1_000,
            _ => {
                let minutes = self.to_minutes();
                u64::from(minutes) * 60_000
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTimeframe(pub u64);

impl fmt::Display for InvalidTimeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid milliseconds value for Timeframe: {}", self.0)
    }
}

impl std::error::Error for InvalidTimeframe {}

/// Serializable version of `(Exchange, Ticker)` tuples that is used for keys in maps
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SerTicker {
    pub exchange: Exchange,
    pub ticker: Ticker,
}

impl SerTicker {
    pub fn new(exchange: Exchange, ticker_str: &str) -> Self {
        let ticker = Ticker::new(ticker_str, exchange);
        Self { exchange, ticker }
    }

    pub fn from_parts(exchange: Exchange, ticker: Ticker) -> Self {
        assert_eq!(
            ticker.market_type(),
            exchange.market_type(),
            "Ticker market type must match Exchange market type"
        );

        Self { exchange, ticker }
    }

    fn exchange_to_string(exchange: Exchange) -> &'static str {
        match exchange {
            Exchange::BinanceLinear => "BinanceLinear",
            Exchange::BinanceInverse => "BinanceInverse",
            Exchange::BinanceSpot => "BinanceSpot",
            Exchange::BybitLinear => "BybitLinear",
            Exchange::BybitInverse => "BybitInverse",
            Exchange::BybitSpot => "BybitSpot",
            Exchange::HyperliquidLinear => "HyperliquidLinear",
            Exchange::HyperliquidSpot => "HyperliquidSpot",
        }
    }

    fn string_to_exchange(s: &str) -> Result<Exchange, String> {
        match s {
            "BinanceLinear" => Ok(Exchange::BinanceLinear),
            "BinanceInverse" => Ok(Exchange::BinanceInverse),
            "BinanceSpot" => Ok(Exchange::BinanceSpot),
            "BybitLinear" => Ok(Exchange::BybitLinear),
            "BybitInverse" => Ok(Exchange::BybitInverse),
            "BybitSpot" => Ok(Exchange::BybitSpot),
            "HyperliquidLinear" => Ok(Exchange::HyperliquidLinear),
            "HyperliquidSpot" => Ok(Exchange::HyperliquidSpot),
            _ => Err(format!("Unknown exchange: {}", s)),
        }
    }
}

impl Serialize for SerTicker {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (ticker_str, _) = self.ticker.to_full_symbol_and_type();
        let exchange_str = Self::exchange_to_string(self.exchange);
        let combined = format!("{}:{}", exchange_str, ticker_str);
        serializer.serialize_str(&combined)
    }
}

impl<'de> Deserialize<'de> for SerTicker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let parts: Vec<&str> = s.split(':').collect();

        if parts.len() != 2 {
            return Err(serde::de::Error::custom(format!(
                "Invalid ExchangeTicker format: expected 'Exchange:Ticker', got '{}'",
                s
            )));
        }

        let exchange_str = parts[0];
        let exchange = Self::string_to_exchange(exchange_str).map_err(serde::de::Error::custom)?;

        let ticker_str = parts[1];
        let ticker = Ticker::new(ticker_str, exchange);

        Ok(SerTicker { exchange, ticker })
    }
}

impl fmt::Display for SerTicker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (ticker_str, _) = self.ticker.to_full_symbol_and_type();
        let exchange_str = Self::exchange_to_string(self.exchange);
        write!(f, "{}:{}", exchange_str, ticker_str)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticker {
    bytes: [u8; Ticker::MAX_LEN as usize],
    pub exchange: Exchange,
    // Optional display symbol for UI, mainly used for Hyperliquid spot markets
    // to show "HYPEUSDC" instead of "@107"
    display_bytes: [u8; Ticker::MAX_LEN as usize],
    has_display_symbol: bool,
}

impl Ticker {
    const MAX_LEN: u8 = 28;

    pub fn new(ticker: &str, exchange: Exchange) -> Self {
        Self::new_with_display(ticker, exchange, None)
    }

    pub fn new_with_display(
        ticker: &str,
        exchange: Exchange,
        display_symbol: Option<&str>,
    ) -> Self {
        assert!(ticker.len() <= Self::MAX_LEN as usize, "Ticker too long");
        assert!(
            ticker.is_ascii()
                && ticker
                    .as_bytes()
                    .iter()
                    .all(|&b| b.is_ascii_graphic() && b != b':' && b != b'|'),
            "Ticker must be printable ASCII and must not contain ':' or '|': {ticker:?}"
        );

        let mut bytes = [0u8; Self::MAX_LEN as usize];
        bytes[..ticker.len()].copy_from_slice(ticker.as_bytes());

        let mut display_bytes = [0u8; Self::MAX_LEN as usize];
        let has_display_symbol = if let Some(display) = display_symbol {
            assert!(
                display.len() <= Self::MAX_LEN as usize,
                "Display symbol too long"
            );
            assert!(
                display.is_ascii()
                    && display
                        .as_bytes()
                        .iter()
                        .all(|&b| b.is_ascii_graphic() && b != b':' && b != b'|'),
                "Display symbol must be printable ASCII and must not contain ':' or '|': {display:?}"
            );
            display_bytes[..display.len()].copy_from_slice(display.as_bytes());
            true
        } else {
            false
        };

        Ticker {
            bytes,
            exchange,
            display_bytes,
            has_display_symbol,
        }
    }

    #[inline]
    fn as_str(&self) -> &str {
        let end = self
            .bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(Self::MAX_LEN as usize);
        std::str::from_utf8(&self.bytes[..end]).unwrap()
    }

    #[inline]
    fn display_as_str(&self) -> &str {
        if self.has_display_symbol {
            let end = self
                .display_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(Self::MAX_LEN as usize);
            std::str::from_utf8(&self.display_bytes[..end]).unwrap()
        } else {
            self.as_str()
        }
    }

    /// Get the display symbol if it exists, otherwise None
    pub fn display_symbol(&self) -> Option<&str> {
        if self.has_display_symbol {
            Some(self.display_as_str())
        } else {
            None
        }
    }

    pub fn to_full_symbol_and_type(&self) -> (String, MarketKind) {
        (self.as_str().to_owned(), self.market_type())
    }

    pub fn display_symbol_and_type(&self) -> (String, MarketKind) {
        let market_kind = self.market_type();

        let result = if self.has_display_symbol {
            // Use the custom display symbol (e.g., "HYPEUSDC" for Hyperliquid spot)
            self.display_as_str().to_owned()
        } else {
            let mut result = self.as_str().to_owned();
            // Transform Hyperliquid symbols to standardized display format
            if matches!(self.exchange, Exchange::HyperliquidLinear)
                && market_kind == MarketKind::LinearPerps
            {
                // For Hyperliquid Linear Perps, append USDT to match other exchanges' format
                // The "P" suffix will be added later in compute_display_data for all perpetual contracts
                result.push_str("USDT");
            }
            result
        };

        (result, market_kind)
    }

    pub fn market_type(&self) -> MarketKind {
        self.exchange.market_type()
    }
}

impl fmt::Display for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (sym, kind) = self.display_symbol_and_type();
        let internal_sym = self.as_str();
        if self.has_display_symbol && internal_sym != sym {
            write!(
                f,
                "Ticker({}:{}[{}], {:?})",
                SerTicker::exchange_to_string(self.exchange),
                sym,
                internal_sym,
                kind
            )
        } else {
            write!(
                f,
                "Ticker({}:{}, {:?})",
                SerTicker::exchange_to_string(self.exchange),
                sym,
                kind
            )
        }
    }
}

impl Serialize for Ticker {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let internal = self.as_str();
        let exchange = SerTicker::exchange_to_string(self.exchange);
        let s = if self.has_display_symbol {
            let display = self.display_as_str();
            format!("{exchange}:{internal}|{display}")
        } else {
            format!("{exchange}:{internal}")
        };
        serializer.serialize_str(&s)
    }
}

/// Backwards compatible deserializer for Ticker so it won't break old persistent states
#[derive(Deserialize)]
#[serde(untagged)]
enum TickerDe {
    Str(String),
    // Old packed format
    Old {
        data: [u64; 2],
        len: u8,
        exchange: String,
    },
}

impl<'de> Deserialize<'de> for Ticker {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match TickerDe::deserialize(deserializer)? {
            TickerDe::Str(s) => {
                let (exchange_str, rest) = s
                    .split_once(':')
                    .ok_or_else(|| serde::de::Error::custom("expected \"Exchange:Symbol\""))?;
                let exchange = SerTicker::string_to_exchange(exchange_str)
                    .map_err(serde::de::Error::custom)?;

                let (symbol, display) = if let Some((sym, disp)) = rest.split_once('|') {
                    (sym, Some(disp))
                } else {
                    (rest, None)
                };
                Ok(Ticker::new_with_display(symbol, exchange, display))
            }
            TickerDe::Old {
                data,
                len,
                exchange,
            } => {
                // Decode old 6-bit packed symbol
                if len as usize > 20 {
                    return Err(serde::de::Error::custom("old Ticker.len > 20"));
                }

                let mut symbol = String::with_capacity(len as usize);
                for i in 0..(len as usize) {
                    let shift = (i % 10) * 6;
                    let v = ((data[i / 10] >> shift) & 0x3F) as u8;
                    let ch = match v {
                        0..=9 => (b'0' + v) as char,
                        10..=35 => (b'A' + (v - 10)) as char,
                        36 => '_',
                        _ => {
                            return Err(serde::de::Error::custom(format!(
                                "invalid old char code {}",
                                v
                            )));
                        }
                    };
                    symbol.push(ch);
                }

                let exchange_enum =
                    SerTicker::string_to_exchange(&exchange).map_err(serde::de::Error::custom)?;

                Ok(Ticker::new(&symbol, exchange_enum))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq)]
pub struct Power10<const MIN: i8, const MAX: i8> {
    pub power: i8,
}

impl<const MIN: i8, const MAX: i8> Power10<MIN, MAX> {
    #[inline]
    pub fn new(power: i8) -> Self {
        Self {
            power: power.clamp(MIN, MAX),
        }
    }

    #[inline]
    pub fn as_f32(self) -> f32 {
        10f32.powi(self.power as i32)
    }
}

impl<const MIN: i8, const MAX: i8> From<Power10<MIN, MAX>> for f32 {
    fn from(v: Power10<MIN, MAX>) -> Self {
        v.as_f32()
    }
}

impl<const MIN: i8, const MAX: i8> From<f32> for Power10<MIN, MAX> {
    fn from(value: f32) -> Self {
        if value <= 0.0 {
            return Self { power: 0 };
        }
        let log10 = value.abs().log10();
        let rounded = log10.round() as i8;
        let power = rounded.clamp(MIN, MAX);
        Self { power }
    }
}

impl<const MIN: i8, const MAX: i8> serde::Serialize for Power10<MIN, MAX> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // serialize as a plain numeric (e.g. 0.1, 1, 10)
        let v: f32 = (*self).into();
        serializer.serialize_f32(v)
    }
}

impl<'de, const MIN: i8, const MAX: i8> serde::Deserialize<'de> for Power10<MIN, MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = f32::deserialize(deserializer)?;
        Ok(Self::from(v))
    }
}

pub type ContractSize = Power10<-1, 6>;
pub type MinTicksize = Power10<-8, 2>;
pub type MinQtySize = Power10<-6, 8>;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Hash, Eq)]
pub struct TickerInfo {
    pub ticker: Ticker,
    #[serde(rename = "tickSize")]
    pub min_ticksize: MinTicksize,
    pub min_qty: MinQtySize,
    pub contract_size: Option<ContractSize>,
}

impl TickerInfo {
    pub fn new(
        ticker: Ticker,
        min_ticksize: f32,
        min_qty: f32,
        contract_size: Option<f32>,
    ) -> Self {
        Self {
            ticker,
            min_ticksize: MinTicksize::from(min_ticksize),
            min_qty: MinQtySize::from(min_qty),
            contract_size: contract_size.map(ContractSize::from),
        }
    }

    pub fn market_type(&self) -> MarketKind {
        self.ticker.market_type()
    }

    pub fn is_perps(&self) -> bool {
        let market_type = self.ticker.market_type();
        market_type == MarketKind::LinearPerps || market_type == MarketKind::InversePerps
    }

    pub fn exchange(&self) -> Exchange {
        self.ticker.exchange
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Trade {
    pub time: u64,
    #[serde(deserialize_with = "bool_from_int")]
    pub is_sell: bool,
    pub price: f32,
    pub qty: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Kline {
    pub time: u64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: (f32, f32),
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct TickerStats {
    pub mark_price: f32,
    pub daily_price_chg: f32,
    pub daily_volume: f32,
}

pub fn is_symbol_supported(symbol: &str, exchange: Exchange, log: bool) -> bool {
    let valid_symbol = symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_');

    if valid_symbol {
        return true;
    } else if log {
        log::warn!("Unsupported ticker: '{}': {:?}", exchange, symbol,);
    }
    false
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
pub struct TickMultiplier(pub u16);

impl std::fmt::Display for TickMultiplier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}x", self.0)
    }
}

impl TickMultiplier {
    pub const ALL: [TickMultiplier; 9] = [
        TickMultiplier(1),
        TickMultiplier(2),
        TickMultiplier(5),
        TickMultiplier(10),
        TickMultiplier(25),
        TickMultiplier(50),
        TickMultiplier(100),
        TickMultiplier(200),
        TickMultiplier(500),
    ];

    pub fn is_custom(&self) -> bool {
        !Self::ALL.contains(self)
    }

    pub fn base(&self, scaled_value: f32) -> f32 {
        let decimals = (-scaled_value.log10()).ceil() as i32 + 2;
        let multiplier = 10f32.powi(decimals);

        ((scaled_value * multiplier) / f32::from(self.0)).round() / multiplier
    }

    /// Returns the final tick size after applying the user selected multiplier
    ///
    /// Usually used for price steps in chart scales
    pub fn multiply_with_min_tick_size(&self, ticker_info: TickerInfo) -> f32 {
        let min_tick_size: f32 = ticker_info.min_ticksize.into();

        let Some(multiplier) = Decimal::from_f32(f32::from(self.0)) else {
            log::error!("Failed to convert multiplier: {}", self.0);
            return f32::from(self.0) * min_tick_size;
        };

        let Some(decimal_min_tick_size) = Decimal::from_f32(min_tick_size) else {
            log::error!("Failed to convert min_tick_size: {min_tick_size}",);
            return f32::from(self.0) * min_tick_size;
        };

        let normalized = multiplier * decimal_min_tick_size.normalize();
        if let Some(tick_size) = normalized.to_f32() {
            let decimal_places = calculate_decimal_places(min_tick_size);
            round_to_decimal_places(tick_size, decimal_places)
        } else {
            log::error!("Failed to calculate tick size for multiplier: {}", self.0);
            f32::from(self.0) * min_tick_size
        }
    }
}

// ticksize rounding helpers
fn calculate_decimal_places(value: f32) -> u32 {
    let s = value.to_string();
    if let Some(decimal_pos) = s.find('.') {
        (s.len() - decimal_pos - 1) as u32
    } else {
        0
    }
}
fn round_to_decimal_places(value: f32, places: u32) -> f32 {
    let factor = 10.0f32.powi(places as i32);
    (value * factor).round() / factor
}
