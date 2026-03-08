use super::{Ticker, Timeframe};
use crate::{
    Kline, OpenInterest, Price, PushFrequency, TickMultiplier, TickerInfo, TickerStats, Trade,
    depth::Depth,
};

use enum_map::{Enum, EnumMap};
use futures::SinkExt;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

pub mod binance;
pub mod bybit;
pub mod hyperliquid;
pub mod mexc;
pub mod okex;

/// Buffer trades and flush in this interval
const TRADE_BUCKET_INTERVAL: Duration = Duration::from_micros(33_333);

async fn flush_trade_buffers<V>(
    output: &mut futures::channel::mpsc::Sender<Event>,
    ticker_info_map: &FxHashMap<Ticker, (TickerInfo, V)>,
    trade_buffers_map: &mut FxHashMap<Ticker, Vec<Trade>>,
) {
    let interval_ms = TRADE_BUCKET_INTERVAL.as_millis() as u64;

    for (ticker, trades_buffer) in trade_buffers_map.iter_mut() {
        if trades_buffer.is_empty() {
            continue;
        }

        let bucket_update_t = trades_buffer
            .iter()
            .map(|t| t.time)
            .max()
            .map(|t| (t / interval_ms) * interval_ms);

        if let Some((ticker_info, _)) = ticker_info_map.get(ticker)
            && let Some(update_t) = bucket_update_t
        {
            let _ = output
                .send(Event::TradesReceived(
                    StreamKind::Trades {
                        ticker_info: *ticker_info,
                    },
                    update_t,
                    std::mem::take(trades_buffer).into_boxed_slice(),
                ))
                .await;
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

    pub fn qty_in_quote_value<T>(&self, qty: T, price: Price, size_in_quote_ccy: bool) -> f32
    where
        T: Into<f32>,
    {
        let qty = qty.into();

        match self {
            MarketKind::InversePerps => qty,
            _ => {
                if size_in_quote_ccy {
                    qty
                } else {
                    price.to_f32() * qty
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
    Depth {
        ticker_info: TickerInfo,
        #[serde(default = "default_depth_aggr")]
        depth_aggr: StreamTicksize,
        push_freq: PushFrequency,
    },
    Trades {
        ticker_info: TickerInfo,
    },
}

impl StreamKind {
    pub fn ticker_info(&self) -> TickerInfo {
        match self {
            StreamKind::Kline { ticker_info, .. }
            | StreamKind::Depth { ticker_info, .. }
            | StreamKind::Trades { ticker_info, .. } => *ticker_info,
        }
    }

    pub fn as_depth_stream(&self) -> Option<(TickerInfo, StreamTicksize, PushFrequency)> {
        match self {
            StreamKind::Depth {
                ticker_info,
                depth_aggr,
                push_freq,
            } => Some((*ticker_info, *depth_aggr, *push_freq)),
            _ => None,
        }
    }

    pub fn as_trade_stream(&self) -> Option<TickerInfo> {
        match self {
            StreamKind::Trades { ticker_info } => Some(*ticker_info),
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
            | StreamKind::Depth { ticker_info, .. }
            | StreamKind::Trades { ticker_info, .. } => (ticker_info.exchange(), ticker_info),
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
        let trade_streams = self.trade_streams(Some(exchange));
        let kline_streams = self.kline_streams(Some(exchange));

        self.specs[exchange] = Some(StreamSpecs {
            depth: depth_streams,
            trade: trade_streams,
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
    ) -> Vec<(TickerInfo, StreamTicksize, PushFrequency)> {
        self.streams(exchange_filter, |_, stream| stream.as_depth_stream())
    }

    pub fn kline_streams(&self, exchange_filter: Option<Exchange>) -> Vec<(TickerInfo, Timeframe)> {
        self.streams(exchange_filter, |_, stream| stream.as_kline_stream())
    }

    pub fn trade_streams(&self, exchange_filter: Option<Exchange>) -> Vec<TickerInfo> {
        self.streams(exchange_filter, |_, stream| stream.as_trade_stream())
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
    pub depth: Vec<(TickerInfo, StreamTicksize, PushFrequency)>,
    pub trade: Vec<TickerInfo>,
    pub kline: Vec<(TickerInfo, Timeframe)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Venue {
    Bybit,
    Binance,
    Hyperliquid,
    Okex,
    Mexc,
}

impl Venue {
    pub const ALL: [Venue; 5] = [
        Venue::Bybit,
        Venue::Binance,
        Venue::Hyperliquid,
        Venue::Okex,
        Venue::Mexc,
    ];
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
    MexcLinear,
    MexcInverse,
    MexcSpot,
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
                Exchange::MexcLinear => "Mexc Linear",
                Exchange::MexcInverse => "Mexc Inverse",
                Exchange::MexcSpot => "Mexc Spot",
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
            "Mexc Linear" => Ok(Exchange::MexcLinear),
            "Mexc Inverse" => Ok(Exchange::MexcInverse),
            "Mexc Spot" => Ok(Exchange::MexcSpot),
            _ => Err(format!("Invalid exchange: {}", s)),
        }
    }
}

impl Exchange {
    pub const ALL: [Exchange; 14] = [
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
        Exchange::MexcLinear,
        Exchange::MexcInverse,
        Exchange::MexcSpot,
    ];

    pub fn market_type(&self) -> MarketKind {
        match self {
            Exchange::BinanceLinear
            | Exchange::BybitLinear
            | Exchange::HyperliquidLinear
            | Exchange::OkexLinear
            | Exchange::MexcLinear => MarketKind::LinearPerps,
            Exchange::BinanceInverse
            | Exchange::BybitInverse
            | Exchange::OkexInverse
            | Exchange::MexcInverse => MarketKind::InversePerps,
            Exchange::BinanceSpot
            | Exchange::BybitSpot
            | Exchange::HyperliquidSpot
            | Exchange::OkexSpot
            | Exchange::MexcSpot => MarketKind::Spot,
        }
    }

    pub fn venue(&self) -> Venue {
        match self {
            Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot => Venue::Bybit,
            Exchange::BinanceLinear | Exchange::BinanceInverse | Exchange::BinanceSpot => {
                Venue::Binance
            }
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => Venue::Hyperliquid,
            Exchange::OkexLinear | Exchange::OkexInverse | Exchange::OkexSpot => Venue::Okex,
            Exchange::MexcLinear | Exchange::MexcInverse | Exchange::MexcSpot => Venue::Mexc,
        }
    }

    pub fn is_depth_client_aggr(&self) -> bool {
        !matches!(
            self,
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot
        )
    }

    pub fn is_custom_push_freq(&self) -> bool {
        matches!(
            self,
            Exchange::BybitLinear | Exchange::BybitInverse | Exchange::BybitSpot
        )
    }

    pub fn allowed_push_freqs(&self) -> &[PushFrequency] {
        match self {
            Exchange::BybitLinear | Exchange::BybitInverse => &[
                PushFrequency::Custom(Timeframe::MS100),
                PushFrequency::Custom(Timeframe::MS300),
            ],
            Exchange::BybitSpot => &[
                PushFrequency::Custom(Timeframe::MS200),
                PushFrequency::Custom(Timeframe::MS300),
            ],
            _ => &[PushFrequency::ServerDefault],
        }
    }

    pub fn supports_heatmap_timeframe(&self, tf: Timeframe) -> bool {
        match self {
            Exchange::BybitSpot => tf != Timeframe::MS100,
            Exchange::BybitLinear | Exchange::BybitInverse => tf != Timeframe::MS200,
            Exchange::HyperliquidLinear | Exchange::HyperliquidSpot => {
                tf != Timeframe::MS100 && tf != Timeframe::MS200 && tf != Timeframe::MS300
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

    pub fn stream_ticksize(
        &self,
        multiplier: Option<TickMultiplier>,
        server_fallback: TickMultiplier,
    ) -> StreamTicksize {
        if self.is_depth_client_aggr() {
            StreamTicksize::Client
        } else {
            StreamTicksize::ServerSide(multiplier.unwrap_or(server_fallback))
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected(Exchange),
    Disconnected(Exchange, String),
    DepthReceived(StreamKind, u64, Arc<Depth>),
    TradesReceived(StreamKind, u64, Box<[Trade]>),
    KlineReceived(StreamKind, Kline),
}

#[derive(Debug, Clone, Hash)]
pub struct StreamConfig<I> {
    pub id: I,
    pub exchange: Exchange,
    pub tick_mltp: Option<TickMultiplier>,
    pub push_freq: PushFrequency,
}

impl<I> StreamConfig<I> {
    pub fn new(
        id: I,
        exchange: Exchange,
        tick_mltp: Option<TickMultiplier>,
        push_freq: PushFrequency,
    ) -> Self {
        Self {
            id,
            exchange,
            tick_mltp,
            push_freq,
        }
    }
}

pub async fn fetch_ticker_metadata(
    exchange: Exchange,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let market_type = exchange.market_type();

    match exchange.venue() {
        Venue::Binance => binance::fetch_ticker_metadata(market_type).await,
        Venue::Bybit => bybit::fetch_ticker_metadata(market_type).await,
        Venue::Hyperliquid => hyperliquid::fetch_ticker_metadata(market_type).await,
        Venue::Okex => okex::fetch_ticker_metadata(market_type).await,
        Venue::Mexc => mexc::fetch_ticker_metadata(market_type).await,
    }
}

pub async fn fetch_ticker_stats(
    exchange: Exchange,
    contract_sizes: Option<HashMap<Ticker, f32>>,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let market_type = exchange.market_type();

    match exchange.venue() {
        Venue::Binance => binance::fetch_ticker_stats(market_type, contract_sizes).await,
        Venue::Bybit => bybit::fetch_ticker_stats(market_type).await,
        Venue::Hyperliquid => hyperliquid::fetch_ticker_stats(market_type).await,
        Venue::Okex => okex::fetch_ticker_stats(market_type).await,
        Venue::Mexc => mexc::fetch_ticker_prices(market_type).await,
    }
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    match ticker_info.ticker.exchange.venue() {
        Venue::Binance => binance::fetch_klines(ticker_info, timeframe, range).await,
        Venue::Bybit => bybit::fetch_klines(ticker_info, timeframe, range).await,
        Venue::Hyperliquid => hyperliquid::fetch_klines(ticker_info, timeframe, range).await,
        Venue::Okex => okex::fetch_klines(ticker_info, timeframe, range).await,
        Venue::Mexc => mexc::fetch_klines(ticker_info, timeframe, range).await,
    }
}

pub async fn fetch_open_interest(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let exchange = ticker_info.ticker.exchange;

    match exchange {
        Exchange::BinanceLinear | Exchange::BinanceInverse => {
            binance::fetch_historical_oi(ticker_info, range, timeframe).await
        }
        Exchange::BybitLinear | Exchange::BybitInverse => {
            bybit::fetch_historical_oi(ticker_info, range, timeframe).await
        }
        Exchange::OkexLinear | Exchange::OkexInverse => {
            okex::fetch_historical_oi(ticker_info, range, timeframe).await
        }
        _ => Err(AdapterError::InvalidRequest(format!(
            "Open interest data not available for {exchange}"
        ))),
    }
}
