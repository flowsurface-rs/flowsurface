use super::{Ticker, Timeframe};
use crate::{Kline, OpenInterest, TickMultiplier, TickerInfo, TickerStats, Trade, depth::Depth};

use enum_map::{Enum, EnumMap};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};

pub mod binance;
pub mod bybit;
pub mod hyperliquid;
pub mod okex;

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedStream {
    /// Streams that are persisted but needs to be resolved for use
    Waiting(Vec<PersistStreamKind>),
    /// Streams that are active and ready to use, but can't persist
    Ready(Vec<StreamKind>),
}

impl ResolvedStream {
    pub fn rebuild_ready_from(&mut self, streams: &[StreamKind]) {
        *self = ResolvedStream::Ready(streams.to_vec());
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        match self {
            ResolvedStream::Ready(existing) => existing.iter().any(|s| s == stream),
            _ => false,
        }
    }

    pub fn ready_iter_mut(&mut self) -> Option<impl Iterator<Item = &mut StreamKind>> {
        match self {
            ResolvedStream::Ready(streams) => Some(streams.iter_mut()),
            _ => None,
        }
    }

    pub fn ready_iter(&self) -> Option<impl Iterator<Item = &StreamKind>> {
        match self {
            ResolvedStream::Ready(streams) => Some(streams.iter()),
            _ => None,
        }
    }

    pub fn find_ready_map<F, T>(&self, f: F) -> Option<T>
    where
        F: FnMut(&StreamKind) -> Option<T>,
    {
        match self {
            ResolvedStream::Ready(streams) => streams.iter().find_map(f),
            _ => None,
        }
    }

    pub fn into_waiting(self) -> Vec<PersistStreamKind> {
        match self {
            ResolvedStream::Waiting(streams) => streams,
            ResolvedStream::Ready(streams) => streams
                .into_iter()
                .map(|s| match s {
                    StreamKind::DepthAndTrades {
                        ticker_info,
                        depth_aggr,
                    } => {
                        let persist_depth = PersistDepth {
                            ticker: ticker_info.ticker,
                            depth_aggr,
                        };
                        PersistStreamKind::DepthAndTrades(persist_depth)
                    }
                    StreamKind::Kline {
                        ticker_info,
                        timeframe,
                    } => {
                        let persist_kline = PersistKline {
                            ticker: ticker_info.ticker,
                            timeframe,
                        };
                        PersistStreamKind::Kline(persist_kline)
                    }
                })
                .collect(),
        }
    }

    pub fn waiting_to_resolve(&self) -> Option<&[PersistStreamKind]> {
        match self {
            ResolvedStream::Waiting(streams) => Some(streams),
            _ => None,
        }
    }
}

impl IntoIterator for &ResolvedStream {
    type Item = StreamKind;
    type IntoIter = std::vec::IntoIter<StreamKind>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            ResolvedStream::Ready(streams) => streams.clone().into_iter(),
            ResolvedStream::Waiting(_) => vec![].into_iter(),
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum MarketKind {
    Spot,
    LinearPerps,
    InversePerps,
}

impl MarketKind {
    pub const ALL: [MarketKind; 3] = [
        MarketKind::Spot,
        MarketKind::LinearPerps,
        MarketKind::InversePerps,
    ];

    pub fn qty_in_quote_value(&self, qty: f32, price: f32, size_in_quote_currency: bool) -> f32 {
        match self {
            MarketKind::InversePerps => qty,
            _ => {
                if size_in_quote_currency {
                    qty
                } else {
                    price * qty
                }
            }
        }
    }
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
        ticker_info: TickerInfo,
        timeframe: Timeframe,
    },
    DepthAndTrades {
        ticker_info: TickerInfo,
        #[serde(default = "default_depth_aggr")]
        depth_aggr: StreamTicksize,
    },
}

impl StreamKind {
    pub fn ticker_info(&self) -> TickerInfo {
        match self {
            StreamKind::Kline { ticker_info, .. }
            | StreamKind::DepthAndTrades { ticker_info, .. } => *ticker_info,
        }
    }

    pub fn as_depth_stream(&self) -> Option<(TickerInfo, StreamTicksize)> {
        match self {
            StreamKind::DepthAndTrades {
                ticker_info,
                depth_aggr,
            } => Some((*ticker_info, *depth_aggr)),
            _ => None,
        }
    }

    pub fn as_kline_stream(&self) -> Option<(TickerInfo, Timeframe)> {
        match self {
            StreamKind::Kline {
                ticker_info,
                timeframe,
            } => Some((*ticker_info, *timeframe)),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct UniqueStreams {
    streams: EnumMap<Exchange, Option<FxHashMap<TickerInfo, FxHashSet<StreamKind>>>>,
    specs: EnumMap<Exchange, Option<StreamSpecs>>,
}

impl UniqueStreams {
    pub fn from<'a>(streams: impl Iterator<Item = &'a StreamKind>) -> Self {
        let mut unique_streams = UniqueStreams::default();
        for stream in streams {
            unique_streams.add(*stream);
        }
        unique_streams
    }

    pub fn add(&mut self, stream: StreamKind) {
        let (exchange, ticker_info) = match stream {
            StreamKind::Kline { ticker_info, .. }
            | StreamKind::DepthAndTrades { ticker_info, .. } => {
                (ticker_info.exchange(), ticker_info)
            }
        };

        self.streams[exchange]
            .get_or_insert_with(FxHashMap::default)
            .entry(ticker_info)
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

        self.specs[exchange] = Some(StreamSpecs {
            depth: depth_streams,
            kline: kline_streams,
        });
    }

    fn streams<T, F>(&self, exchange_filter: Option<Exchange>, stream_extractor: F) -> Vec<T>
    where
        F: Fn(Exchange, &StreamKind) -> Option<T>,
    {
        let f = &stream_extractor;

        let per_exchange = |exchange| {
            self.streams[exchange]
                .as_ref()
                .into_iter()
                .flat_map(|ticker_map| ticker_map.values().flatten())
                .filter_map(move |stream| f(exchange, stream))
        };

        match exchange_filter {
            Some(exchange) => per_exchange(exchange).collect(),
            None => Exchange::ALL.into_iter().flat_map(per_exchange).collect(),
        }
    }

    pub fn depth_streams(
        &self,
        exchange_filter: Option<Exchange>,
    ) -> Vec<(TickerInfo, StreamTicksize)> {
        self.streams(exchange_filter, |_, stream| stream.as_depth_stream())
    }

    pub fn kline_streams(&self, exchange_filter: Option<Exchange>) -> Vec<(TickerInfo, Timeframe)> {
        self.streams(exchange_filter, |_, stream| stream.as_kline_stream())
    }

    pub fn combined_used(&self) -> impl Iterator<Item = (Exchange, &StreamSpecs)> {
        self.specs
            .iter()
            .filter_map(|(exchange, specs)| specs.as_ref().map(|stream| (exchange, stream)))
    }

    pub fn combined(&self) -> &EnumMap<Exchange, Option<StreamSpecs>> {
        &self.specs
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum PersistStreamKind {
    Kline(PersistKline),
    DepthAndTrades(PersistDepth),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PersistDepth {
    pub ticker: Ticker,
    #[serde(default = "default_depth_aggr")]
    pub depth_aggr: StreamTicksize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PersistKline {
    pub ticker: Ticker,
    pub timeframe: Timeframe,
}

impl From<StreamKind> for PersistStreamKind {
    fn from(s: StreamKind) -> Self {
        match s {
            StreamKind::Kline {
                ticker_info,
                timeframe,
            } => PersistStreamKind::Kline(PersistKline {
                ticker: ticker_info.ticker,
                timeframe,
            }),
            StreamKind::DepthAndTrades {
                ticker_info,
                depth_aggr,
            } => PersistStreamKind::DepthAndTrades(PersistDepth {
                ticker: ticker_info.ticker,
                depth_aggr,
            }),
        }
    }
}

impl PersistStreamKind {
    /// Try to convert into runtime StreamKind. `resolver` should return Some(TickerInfo) for a ticker string,
    /// otherwise the conversion fails (so caller can trigger a refresh / fetch).
    pub fn into_stream_kind<F>(self, mut resolver: F) -> Result<StreamKind, String>
    where
        F: FnMut(&Ticker) -> Option<TickerInfo>,
    {
        match self {
            PersistStreamKind::Kline(k) => resolver(&k.ticker)
                .map(|ti| StreamKind::Kline {
                    ticker_info: ti,
                    timeframe: k.timeframe,
                })
                .ok_or_else(|| format!("TickerInfo not found for {}", k.ticker)),
            PersistStreamKind::DepthAndTrades(d) => resolver(&d.ticker)
                .map(|ti| StreamKind::DepthAndTrades {
                    ticker_info: ti,
                    depth_aggr: d.depth_aggr,
                })
                .ok_or_else(|| format!("TickerInfo not found for {}", d.ticker)),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum StreamTicksize {
    ServerSide(TickMultiplier),
    #[default]
    Client,
}

fn default_depth_aggr() -> StreamTicksize {
    StreamTicksize::Client
}

#[derive(Debug, Clone, Default)]
pub struct StreamSpecs {
    pub depth: Vec<(TickerInfo, StreamTicksize)>,
    pub kline: Vec<(TickerInfo, Timeframe)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum ExchangeInclusive {
    Bybit,
    Binance,
    Hyperliquid,
    Okex,
}

impl ExchangeInclusive {
    pub const ALL: [ExchangeInclusive; 4] = [
        ExchangeInclusive::Bybit,
        ExchangeInclusive::Binance,
        ExchangeInclusive::Hyperliquid,
        ExchangeInclusive::Okex,
    ];

    pub fn of(ex: Exchange) -> Self {
        match ex {
            Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => Self::Bybit,
            Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
                Self::Binance
            }
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => Self::Hyperliquid,
            Exchange::OkexLinear | Exchange::OkexInverse | Exchange::OkexSpot => Self::Okex,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize, Enum)]
pub enum Exchange {
    BinanceLinear,
    BinanceInverse,
    BinanceSpot,
    BybitLinear,
    BybitInverse,
    BybitSpot,
    HyperliquidLinear,
    HyperliquidSpot,
    OkexLinear,
    OkexInverse,
    OkexSpot,
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
                Exchange::HyperliquidLinear => "Hyperliquid Linear",
                Exchange::HyperliquidSpot => "Hyperliquid Spot",
                Exchange::OkexLinear => "Okex Linear",
                Exchange::OkexInverse => "Okex Inverse",
                Exchange::OkexSpot => "Okex Spot",
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
            "Hyperliquid Linear" => Ok(Exchange::HyperliquidLinear),
            "Hyperliquid Spot" => Ok(Exchange::HyperliquidSpot),
            "Okex Linear" => Ok(Exchange::OkexLinear),
            "Okex Inverse" => Ok(Exchange::OkexInverse),
            "Okex Spot" => Ok(Exchange::OkexSpot),
            _ => Err(format!("Invalid exchange: {}", s)),
        }
    }
}

impl Exchange {
    pub const ALL: [Exchange; 11] = [
        Exchange::BinanceLinear,
        Exchange::BinanceInverse,
        Exchange::BinanceSpot,
        Exchange::BybitLinear,
        Exchange::BybitInverse,
        Exchange::BybitSpot,
        Exchange::HyperliquidLinear,
        Exchange::HyperliquidSpot,
        Exchange::OkexLinear,
        Exchange::OkexInverse,
        Exchange::OkexSpot,
    ];

    pub fn market_type(&self) -> MarketKind {
        match self {
            Exchange::BinanceLinear
            | Exchange::BybitLinear
            | Exchange::HyperliquidLinear
            | Exchange::OkexLinear => MarketKind::LinearPerps,
            Exchange::BinanceInverse | Exchange::BybitInverse | Exchange::OkexInverse => {
                MarketKind::InversePerps
            }
            Exchange::BinanceSpot
            | Exchange::BybitSpot
            | Exchange::HyperliquidSpot
            | Exchange::OkexSpot => MarketKind::Spot,
        }
    }

    pub fn is_depth_client_aggr(&self) -> bool {
        !matches!(
            self,
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot
        )
    }

    pub fn supports_heatmap_timeframe(&self, tf: Timeframe) -> bool {
        match self {
            Exchange::BybitSpot => tf != Timeframe::MS100,
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => {
                tf != Timeframe::MS100 && tf != Timeframe::MS200
            }
            _ => true,
        }
    }

    pub fn is_perps(&self) -> bool {
        matches!(
            self,
            Exchange::BinanceLinear
                | Exchange::BinanceInverse
                | Exchange::BybitLinear
                | Exchange::BybitInverse
                | Exchange::HyperliquidLinear
                | Exchange::OkexLinear
                | Exchange::OkexInverse
        )
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
    pub tick_mltp: Option<TickMultiplier>,
}

impl<I> StreamConfig<I> {
    pub fn new(id: I, exchange: Exchange, tick_mltp: Option<TickMultiplier>) -> Self {
        let market_type = exchange.market_type();
        Self {
            id,
            market_type,
            tick_mltp,
        }
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
        Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => {
            hyperliquid::fetch_ticksize(market_type).await
        }
        Exchange::OkexLinear | Exchange::OkexInverse | Exchange::OkexSpot => {
            okex::fetch_ticksize(market_type).await
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
        Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => {
            hyperliquid::fetch_ticker_prices(market_type).await
        }
        Exchange::OkexLinear | Exchange::OkexInverse | Exchange::OkexSpot => {
            okex::fetch_ticker_prices(market_type).await
        }
    }
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    match ticker_info.ticker.exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
            binance::fetch_klines(ticker_info, timeframe, range).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => {
            bybit::fetch_klines(ticker_info, timeframe, range).await
        }
        Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => {
            hyperliquid::fetch_klines(ticker_info, timeframe, range).await
        }
        Exchange::OkexLinear | Exchange::OkexInverse | Exchange::OkexSpot => {
            okex::fetch_klines(ticker_info, timeframe, range).await
        }
    }
}

pub async fn fetch_open_interest(
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<OpenInterest>, AdapterError> {
    match ticker.exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse => {
            binance::fetch_historical_oi(ticker, range, timeframe).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse => {
            bybit::fetch_historical_oi(ticker, range, timeframe).await
        }
        _ => Err(AdapterError::InvalidRequest("Invalid exchange".to_string())),
    }
}
