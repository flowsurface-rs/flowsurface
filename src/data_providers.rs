use std::{
    collections::BTreeMap,
    fmt::{self, Write},
    hash::Hash,
};

use ordered_float::OrderedFloat;
use rust_decimal::{
    prelude::{FromPrimitive, ToPrimitive},
    Decimal,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub mod binance;
pub mod bybit;
pub mod fetcher;

#[allow(clippy::large_enum_variant)]
pub enum State {
    Disconnected,
    Connected(FragmentCollector<TokioIo<Upgraded>>),
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected(Connection),
    Disconnected(String),
    DepthReceived(Ticker, i64, Depth, Vec<Trade>),
    KlineReceived(Ticker, Kline, Timeframe),
}

#[derive(Debug, Clone)]
pub struct Connection;

#[allow(dead_code)]
#[derive(thiserror::Error, Debug)]
pub enum StreamError {
    #[error("Fetchrror: {0}")]
    FetchError(#[from] reqwest::Error),
    #[error("Parsing error: {0}")]
    ParseError(String),
    #[error("Stream error: {0}")]
    WebsocketError(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("{0}")]
    UnknownError(String),
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum StreamType {
    Kline {
        exchange: Exchange,
        ticker: Ticker,
        timeframe: Timeframe,
    },
    DepthAndTrades {
        exchange: Exchange,
        ticker: Ticker,
    },
    None,
}

// data types
#[derive(Debug, Clone, Copy, Default)]
struct Order {
    price: f32,
    qty: f32,
}

impl<'de> Deserialize<'de> for Order {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let arr: Vec<&str> = Vec::<&str>::deserialize(deserializer)?;
        let price: f32 = arr[0].parse::<f32>().map_err(serde::de::Error::custom)?;
        let qty: f32 = arr[1].parse::<f32>().map_err(serde::de::Error::custom)?;
        Ok(Order { price, qty })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Depth {
    pub bids: BTreeMap<OrderedFloat<f32>, f32>,
    pub asks: BTreeMap<OrderedFloat<f32>, f32>,
}

#[derive(Debug, Clone, Default)]
struct VecLocalDepthCache {
    last_update_id: i64,
    time: i64,
    bids: Vec<Order>,
    asks: Vec<Order>,
}

#[derive(Debug, Clone, Default)]
struct LocalDepthCache {
    last_update_id: i64,
    time: i64,
    bids: BTreeMap<OrderedFloat<f32>, f32>,
    asks: BTreeMap<OrderedFloat<f32>, f32>,
}

impl LocalDepthCache {
    fn new() -> Self {
        Self {
            last_update_id: 0,
            time: 0,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    fn fetched(&mut self, new_depth: &VecLocalDepthCache) {
        self.last_update_id = new_depth.last_update_id;
        self.time = new_depth.time;

        self.bids.clear();
        new_depth.bids.iter().for_each(|order| {
            self.bids.insert(OrderedFloat(order.price), order.qty);
        });
        self.asks.clear();
        new_depth.asks.iter().for_each(|order| {
            self.asks.insert(OrderedFloat(order.price), order.qty);
        });
    }

    fn update_depth_cache(&mut self, new_depth: &VecLocalDepthCache) {
        self.last_update_id = new_depth.last_update_id;
        self.time = new_depth.time;

        new_depth.bids.iter().for_each(|order| {
            if order.qty == 0.0 {
                self.bids.remove((&order.price).into());
            } else {
                self.bids.insert(OrderedFloat(order.price), order.qty);
            }
        });
        new_depth.asks.iter().for_each(|order| {
            if order.qty == 0.0 {
                self.asks.remove((&order.price).into());
            } else {
                self.asks.insert(OrderedFloat(order.price), order.qty);
            }
        });
    }

    fn get_depth(&self) -> Depth {
        Depth {
            bids: self.bids.clone(),
            asks: self.asks.clone(),
        }
    }

    fn get_fetch_id(&self) -> i64 {
        self.last_update_id
    }
}

#[derive(Default, Debug, Clone, Copy, Deserialize)]
pub struct Trade {
    pub time: i64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct TickMultiplier(pub u16);

impl std::fmt::Display for TickMultiplier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}x", self.0)
    }
}

impl TickMultiplier {
    pub const ALL: [TickMultiplier; 8] = [
        TickMultiplier(1),
        TickMultiplier(2),
        TickMultiplier(5),
        TickMultiplier(10),
        TickMultiplier(25),
        TickMultiplier(50),
        TickMultiplier(100),
        TickMultiplier(200),
    ];

    /// Returns the final tick size after applying the user selected multiplier
    ///
    /// Usually used for price steps in chart scales
    pub fn multiply_with_min_tick_size(&self, ticker_info: TickerInfo) -> f32 {
        let min_tick_size = ticker_info.min_ticksize;

        let multiplier = if let Some(m) = Decimal::from_f32(f32::from(self.0)) {
            m
        } else {
            log::error!("Failed to convert multiplier: {}", self.0);
            return f32::from(self.0) * min_tick_size;
        };

        let decimal_min_tick_size = if let Some(d) = Decimal::from_f32(min_tick_size) {
            d
        } else {
            log::error!("Failed to convert min_tick_size: {}", min_tick_size);
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

// connection types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Exchange {
    BinanceFutures,
    BinanceSpot,
    BybitLinear,
    BybitSpot,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Exchange::BinanceFutures => "Binance Futures",
                Exchange::BinanceSpot => "Binance Spot",
                Exchange::BybitLinear => "Bybit Linear",
                Exchange::BybitSpot => "Bybit Spot",
            }
        )
    }
}
impl Exchange {
    pub const MARKET_TYPES: [(Exchange, MarketType); 4] = [
        (Exchange::BinanceFutures, MarketType::LinearPerps),
        (Exchange::BybitLinear, MarketType::LinearPerps),
        (Exchange::BinanceSpot, MarketType::Spot),
        (Exchange::BybitSpot, MarketType::Spot),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum MarketType {
    Spot,
    LinearPerps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Ticker {
    data: [u64; 2],
    len: u8,
    market_type: MarketType,
}

impl Default for Ticker {
    fn default() -> Self {
        Ticker::new("", MarketType::Spot)
    }
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

        Ticker { data, len, market_type }
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

fn deserialize_string_to_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse::<f32>().map_err(serde::de::Error::custom)
}

fn deserialize_string_to_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse::<i64>().map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenInterest {
    pub time: i64,
    pub value: f32,
}

// other helpers
pub fn format_with_commas(num: f32) -> String {
    let s = format!("{num:.0}");
    
    // Handle special case for small numbers
    if s.len() <= 4 && s.starts_with('-') {
        return s;  // Return as-is if it's a small negative number
    }
    
    let mut result = String::with_capacity(s.len() + (s.len() - 1) / 3);
    let (sign, digits) = if s.starts_with('-') {
        ("-", &s[1..])  // Split into sign and digits
    } else {
        ("", &s[..])
    };
    
    let mut i = digits.len();
    while i > 0 {
        if !result.is_empty() {
            result.insert(0, ',');
        }
        let start = if i >= 3 { i - 3 } else { 0 };
        result.insert_str(0, &digits[start..i]);
        i = start;
    }
    
    // Add sign at the start if negative
    if !sign.is_empty() {
        result.insert_str(0, sign);
    }
    
    result
}

// websocket
use bytes::Bytes;
use tokio::net::TcpStream;
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use fastwebsockets::FragmentCollector;
use hyper::{
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
    Request,
};
use tokio_rustls::{
    rustls::{ClientConfig, OwnedTrustAnchor},
    TlsConnector,
};

struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
    Fut: std::future::Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::task::spawn(fut);
    }
}

pub fn tls_connector() -> Result<TlsConnector, StreamError> {
    let mut root_store = tokio_rustls::rustls::RootCertStore::empty();

    root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(std::sync::Arc::new(config)))
}

async fn setup_tcp_connection(domain: &str) -> Result<TcpStream, StreamError> {
    let addr = format!("{domain}:443");
    TcpStream::connect(&addr)
        .await
        .map_err(|e| StreamError::WebsocketError(e.to_string()))
}

async fn setup_tls_connection(
    domain: &str,
    tcp_stream: TcpStream,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, StreamError> {
    let tls_connector: TlsConnector = tls_connector()?;
    let domain: tokio_rustls::rustls::ServerName =
        tokio_rustls::rustls::ServerName::try_from(domain)
            .map_err(|_| StreamError::ParseError("invalid dnsname".to_string()))?;
    tls_connector
        .connect(domain, tcp_stream)
        .await
        .map_err(|e| StreamError::WebsocketError(e.to_string()))
}

async fn setup_websocket_connection(
    domain: &str,
    tls_stream: tokio_rustls::client::TlsStream<TcpStream>,
    url: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, StreamError> {
    let req: Request<Empty<Bytes>> = Request::builder()
        .method("GET")
        .uri(url)
        .header("Host", domain)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header(
            "Sec-WebSocket-Key",
            fastwebsockets::handshake::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .body(Empty::<Bytes>::new())
        .map_err(|e| StreamError::WebsocketError(e.to_string()))?;

    let (ws, _) = fastwebsockets::handshake::client(&SpawnExecutor, req, tls_stream)
        .await
        .map_err(|e| StreamError::WebsocketError(e.to_string()))?;

    Ok(FragmentCollector::new(ws))
}

#[allow(unused_imports)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_bybit_tickers_with_rate_limits() -> Result<(), StreamError> {
        let url = "https://api.bybit.com/v5/market/tickers?category=spot".to_string();
        let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;

        println!("{:?}", response.headers());

        let _text = response.text().await.map_err(StreamError::FetchError)?;

        Ok(())
    }

    #[tokio::test]
    async fn fetch_binance_tickers_with_rate_limits() -> Result<(), StreamError> {
        let url = "https://fapi.binance.com/fapi/v1/ticker/24hr".to_string();
        let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;

        println!("{:?}", response.headers());

        let _text = response.text().await.map_err(StreamError::FetchError)?;

        Ok(())
    }
}