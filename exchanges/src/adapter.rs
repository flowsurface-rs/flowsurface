use crate::{Kline, OpenInterest, Trade, depth::Depth};

use super::{Ticker, Timeframe};
use serde::{Deserialize, Serialize};

pub mod binance;
pub mod bybit;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum MarketType {
    Spot,
    LinearPerps,
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

#[derive(Debug, Clone)]
pub struct Connection;

#[derive(Debug, Clone)]
pub enum Event {
    Connected(Exchange, Connection),
    Disconnected(Exchange, String),
    DepthReceived(StreamType, u64, Depth, Box<[Trade]>),
    KlineReceived(StreamType, Kline),
}

#[derive(Debug, Clone, Hash)]
pub struct StreamConfig<I> {
    pub id: I,
    pub market_type: MarketType,
}

impl<I> StreamConfig<I> {
    pub fn new(id: I, exchange: Exchange) -> Self {
        let market_type = match exchange {
            Exchange::BinanceFutures | Exchange::BybitLinear => MarketType::LinearPerps,
            Exchange::BinanceSpot | Exchange::BybitSpot => MarketType::Spot,
        };

        Self { id, market_type }
    }
}

pub async fn fetch_klines(
    exchange: Exchange,
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, StreamError> {
    match exchange {
        Exchange::BinanceFutures | Exchange::BinanceSpot => {
            binance::fetch_klines(ticker, timeframe, range).await
        }
        Exchange::BybitLinear | Exchange::BybitSpot => {
            bybit::fetch_klines(ticker, timeframe, range).await
        }
    }
}

pub async fn fetch_open_interest(
    exchange: Exchange,
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<OpenInterest>, StreamError> {
    match exchange {
        Exchange::BinanceFutures => binance::fetch_historical_oi(ticker, range, timeframe).await,
        Exchange::BybitLinear => bybit::fetch_historical_oi(ticker, range, timeframe).await,
        _ => Err(StreamError::InvalidRequest("Invalid exchange".to_string())),
    }
}
