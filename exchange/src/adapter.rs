use super::{Ticker, Timeframe};
use crate::{Kline, OpenInterest, TickerInfo, TickerStats, Trade, depth::Depth};

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

pub mod binance;
pub mod bybit;

#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
    #[error("{0}")]
    FetchError(#[from] reqwest::Error),
    #[error("Parsing: {0}")]
    ParseError(String),
    #[error("Stream: {0}")]
    WebsocketError(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum MarketKind {
    Spot,
    LinearPerps,
    InversePerps,
}

impl std::fmt::Display for MarketKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                MarketKind::Spot => "Spot",
                MarketKind::LinearPerps => "Linear",
                MarketKind::InversePerps => "Inverse",
            }
        )
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum StreamKind {
    Kline {
        exchange: Exchange,
        ticker: Ticker,
        timeframe: Timeframe,
    },
    DepthAndTrades {
        exchange: Exchange,
        ticker: Ticker,
    },
}

impl StreamKind {
    pub fn exchange_and_ticker(&self) -> (Exchange, Ticker) {
        match self {
            StreamKind::Kline {
                exchange, ticker, ..
            }
            | StreamKind::DepthAndTrades { exchange, ticker } => (*exchange, *ticker),
        }
    }

    pub fn as_depth_stream(&self) -> Option<(Exchange, Ticker)> {
        match self {
            StreamKind::DepthAndTrades { exchange, ticker } => Some((*exchange, *ticker)),
            _ => None,
        }
    }

    pub fn as_kline_stream(&self) -> Option<(Exchange, Ticker, Timeframe)> {
        match self {
            StreamKind::Kline {
                exchange,
                ticker,
                timeframe,
            } => Some((*exchange, *ticker, *timeframe)),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct UniqueStreams {
    streams: HashMap<Exchange, HashMap<Ticker, HashSet<StreamKind>>>,
    specs: HashMap<Exchange, StreamSpecs>,
}

impl UniqueStreams {
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
            specs: HashMap::new(),
        }
    }

    pub fn from<'a>(streams: impl Iterator<Item = &'a StreamKind>) -> Self {
        let mut unique_streams = UniqueStreams::new();
        for stream in streams {
            unique_streams.add(*stream);
        }
        unique_streams
    }

    pub fn add(&mut self, stream: StreamKind) {
        let (exchange, ticker) = match stream {
            StreamKind::Kline {
                exchange, ticker, ..
            }
            | StreamKind::DepthAndTrades { exchange, ticker } => (exchange, ticker),
        };

        self.streams
            .entry(exchange)
            .or_default()
            .entry(ticker)
            .or_default()
            .insert(stream);

        self.update_specs_for_exchange(exchange);
    }

    pub fn extend<'a>(&mut self, streams: impl IntoIterator<Item = &'a StreamKind>) {
        for stream in streams {
            self.add(*stream);
        }
    }

    fn update_specs_for_exchange(&mut self, exchange: Exchange) {
        let depth_streams = self.depth_streams(Some(exchange));
        let kline_streams = self.kline_streams(Some(exchange));

        self.specs.insert(
            exchange,
            StreamSpecs {
                depth: depth_streams,
                kline: kline_streams,
            },
        );
    }

    fn streams<T, F>(&self, exchange_filter: Option<Exchange>, stream_extractor: F) -> Vec<T>
    where
        F: Fn(Exchange, &StreamKind) -> Option<T>,
    {
        match exchange_filter {
            Some(exchange) => self.streams.get(&exchange).map_or(vec![], |ticker_map| {
                ticker_map
                    .values()
                    .flatten()
                    .filter_map(|stream| stream_extractor(exchange, stream))
                    .collect()
            }),
            None => self
                .streams
                .iter()
                .flat_map(|(exchange, ticker_map)| {
                    ticker_map
                        .values()
                        .flatten()
                        .filter_map(|stream| stream_extractor(*exchange, stream))
                        .collect::<Vec<_>>()
                })
                .collect(),
        }
    }

    pub fn depth_streams(&self, exchange_filter: Option<Exchange>) -> Vec<(Exchange, Ticker)> {
        self.streams(exchange_filter, |exchange, stream| {
            stream
                .as_depth_stream()
                .map(|(_, ticker)| (exchange, ticker))
        })
    }

    pub fn kline_streams(
        &self,
        exchange_filter: Option<Exchange>,
    ) -> Vec<(Exchange, Ticker, Timeframe)> {
        self.streams(exchange_filter, |exchange, stream| {
            stream
                .as_kline_stream()
                .map(|(_, ticker, timeframe)| (exchange, ticker, timeframe))
        })
    }

    pub fn combined(&self) -> &HashMap<Exchange, StreamSpecs> {
        &self.specs
    }
}

#[derive(Debug, Clone, Default)]
pub struct StreamSpecs {
    pub depth: Vec<(Exchange, Ticker)>,
    pub kline: Vec<(Exchange, Ticker, Timeframe)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Exchange {
    BinanceLinear,
    BinanceInverse,
    BinanceSpot,
    BybitLinear,
    BybitInverse,
    BybitSpot,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Exchange::BinanceLinear => "Binance Linear",
                Exchange::BinanceInverse => "Binance Inverse",
                Exchange::BinanceSpot => "Binance Spot",
                Exchange::BybitLinear => "Bybit Linear",
                Exchange::BybitInverse => "Bybit Inverse",
                Exchange::BybitSpot => "Bybit Spot",
            }
        )
    }
}

impl FromStr for Exchange {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Binance Linear" => Ok(Exchange::BinanceLinear),
            "Binance Inverse" => Ok(Exchange::BinanceInverse),
            "Binance Spot" => Ok(Exchange::BinanceSpot),
            "Bybit Linear" => Ok(Exchange::BybitLinear),
            "Bybit Inverse" => Ok(Exchange::BybitInverse),
            "Bybit Spot" => Ok(Exchange::BybitSpot),
            _ => Err(format!("Invalid exchange: {}", s)),
        }
    }
}

impl Exchange {
    pub const ALL: [Exchange; 6] = [
        Exchange::BinanceLinear,
        Exchange::BinanceInverse,
        Exchange::BinanceSpot,
        Exchange::BybitLinear,
        Exchange::BybitInverse,
        Exchange::BybitSpot,
    ];

    pub fn market_type(&self) -> MarketKind {
        match self {
            Exchange::BinanceLinear | Exchange::BybitLinear => MarketKind::LinearPerps,
            Exchange::BinanceInverse | Exchange::BybitInverse => MarketKind::InversePerps,
            Exchange::BinanceSpot | Exchange::BybitSpot => MarketKind::Spot,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected(Exchange),
    Disconnected(Exchange, String),
    DepthReceived(StreamKind, u64, Depth, Box<[Trade]>),
    KlineReceived(StreamKind, Kline),
}

#[derive(Debug, Clone, Hash)]
pub struct StreamConfig<I> {
    pub id: I,
    pub market_type: MarketKind,
}

impl<I> StreamConfig<I> {
    pub fn new(id: I, exchange: Exchange) -> Self {
        let market_type = exchange.market_type();
        Self { id, market_type }
    }
}

pub async fn fetch_ticker_info(
    exchange: Exchange,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let market_type = exchange.market_type();

    match exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
            binance::fetch_ticksize(market_type).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => {
            bybit::fetch_ticksize(market_type).await
        }
    }
}

pub async fn fetch_ticker_prices(
    exchange: Exchange,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let market_type = exchange.market_type();

    match exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
            binance::fetch_ticker_prices(market_type).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => {
            bybit::fetch_ticker_prices(market_type).await
        }
    }
}

pub async fn fetch_klines(
    exchange: Exchange,
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    match exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
            binance::fetch_klines(ticker, timeframe, range).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => {
            bybit::fetch_klines(ticker, timeframe, range).await
        }
    }
}

pub async fn fetch_open_interest(
    exchange: Exchange,
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<OpenInterest>, AdapterError> {
    match exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse => {
            binance::fetch_historical_oi(ticker, range, timeframe).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse => {
            bybit::fetch_historical_oi(ticker, range, timeframe).await
        }
        _ => Err(AdapterError::InvalidRequest("Invalid exchange".to_string())),
    }
}
