use crate::{
    Event, Kline, OpenInterest, PushFrequency, Ticker, TickerInfo, Timeframe, Trade,
    adapter::{Exchange, MarketKind},
    depth::DepthPayload,
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::collections::HashMap;
use tokio::sync::mpsc;

pub mod fetch;
pub mod stream;

const FETCH_DOMAIN: &str = "https://api.mexc.com/api";
const MEXC_FUTURES_WS_DOMAIN: &str = "contract.mexc.com";
const MEXC_FUTURES_WS_PATH: &str = "/edge";
const PING_INTERVAL: u64 = 15;
const DEFAULT_COMMAND_BUFFER_CAPACITY: usize = 128;

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::MexcSpot,
        MarketKind::LinearPerps => Exchange::MexcLinear,
        MarketKind::InversePerps => Exchange::MexcInverse,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot => RawQtyUnit::Base,
        MarketKind::LinearPerps | MarketKind::InversePerps => RawQtyUnit::Contracts,
    }
}

fn contract_size_for_market(
    ticker_info: TickerInfo,
    market: MarketKind,
    context: &str,
) -> Result<f32, AdapterError> {
    match market {
        MarketKind::Spot => Ok(1.0),
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            ticker_info.contract_size.map(f32::from).ok_or_else(|| {
                AdapterError::ParseError(format!(
                    "Missing contract size for {} in {context}",
                    ticker_info.ticker
                ))
            })
        }
    }
}

fn mexc_perps_market_from_symbol(
    symbol: &str,
    contract_sizes: Option<&HashMap<Ticker, f32>>,
) -> Option<MarketKind> {
    if symbol.ends_with("USDT") {
        return Some(MarketKind::LinearPerps);
    }
    if symbol.ends_with("USD") {
        return Some(MarketKind::InversePerps);
    }

    let contract_sizes = contract_sizes?;

    let has_linear = contract_sizes.contains_key(&Ticker::new(symbol, Exchange::MexcLinear));
    let has_inverse = contract_sizes.contains_key(&Ticker::new(symbol, Exchange::MexcInverse));

    match (has_linear, has_inverse) {
        (true, false) => Some(MarketKind::LinearPerps),
        (false, true) => Some(MarketKind::InversePerps),
        _ => None,
    }
}

fn convert_to_mexc_timeframe(timeframe: Timeframe, market: MarketKind) -> Option<&'static str> {
    if market == MarketKind::Spot {
        match timeframe {
            Timeframe::M1 => Some("1m"),
            Timeframe::M5 => Some("5m"),
            Timeframe::M15 => Some("15m"),
            Timeframe::M30 => Some("30m"),
            Timeframe::H1 => Some("60m"),
            Timeframe::H4 => Some("4h"),
            Timeframe::D1 => Some("1d"),
            _ => None,
        }
    } else {
        match timeframe {
            Timeframe::M1 => Some("Min1"),
            Timeframe::M5 => Some("Min5"),
            Timeframe::M15 => Some("Min15"),
            Timeframe::M30 => Some("Min30"),
            Timeframe::H1 => Some("Min60"),
            Timeframe::H4 => Some("Hour4"),
            Timeframe::D1 => Some("Day1"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MexcConfig;

pub type MexcLimiter = super::NoopRateLimiter;

#[derive(Debug, Clone, Default)]
pub struct MexcMarketScope {
    pub markets: Vec<MarketKind>,
    pub contract_sizes: Option<HashMap<Ticker, f32>>,
}

impl MexcMarketScope {
    pub fn metadata(markets: &[MarketKind]) -> Self {
        Self {
            markets: markets.to_vec(),
            contract_sizes: None,
        }
    }

    pub fn stats(markets: &[MarketKind], contract_sizes: Option<HashMap<Ticker, f32>>) -> Self {
        Self {
            markets: markets.to_vec(),
            contract_sizes,
        }
    }
}

pub enum MexcCommand {
    Fetch(super::FetchCommand<MexcMarketScope>),
    FetchDepthSnapshot {
        ticker: Ticker,
        reply: super::ResponseTx<DepthPayload>,
    },
}

#[derive(Clone)]
pub struct MexcHandle {
    request_port: RequestPort<MexcCommand>,
}

impl MexcHandle {
    pub fn new(request_port: RequestPort<MexcCommand>) -> Self {
        Self { request_port }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: MexcMarketScope,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| {
                MexcCommand::Fetch(super::FetchCommand::FetchTickerMetadata {
                    market_scope,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market_scope: MexcMarketScope,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| {
                MexcCommand::Fetch(super::FetchCommand::FetchTickerStats {
                    market_scope,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_klines(
        &self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<Kline>, AdapterError> {
        self.request_port
            .request(move |reply| {
                MexcCommand::Fetch(super::FetchCommand::FetchKlines {
                    ticker_info,
                    timeframe,
                    range,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_open_interest(
        &self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<OpenInterest>, AdapterError> {
        self.request_port
            .request(move |reply| {
                MexcCommand::Fetch(super::FetchCommand::FetchOpenInterest {
                    ticker_info,
                    timeframe,
                    range,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_trades(
        &self,
        ticker_info: TickerInfo,
        from_time: u64,
        data_path: Option<std::path::PathBuf>,
    ) -> Result<Vec<Trade>, AdapterError> {
        self.request_port
            .request(move |reply| {
                MexcCommand::Fetch(super::FetchCommand::FetchTrades {
                    ticker_info,
                    from_time,
                    data_path,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_depth_snapshot(&self, ticker: Ticker) -> Result<DepthPayload, AdapterError> {
        self.request_port
            .request(move |reply| MexcCommand::FetchDepthSnapshot { ticker, reply })
            .await
    }

    pub async fn fetch_ticker_metadata_for_markets(
        &self,
        markets: &[MarketKind],
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.fetch_ticker_metadata(MexcMarketScope::metadata(markets))
            .await
    }

    pub async fn fetch_ticker_stats_for_markets(
        &self,
        markets: &[MarketKind],
        contract_sizes: Option<HashMap<Ticker, f32>>,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.fetch_ticker_stats(MexcMarketScope::stats(markets, contract_sizes))
            .await
    }

    pub fn connect_depth_stream(
        &self,
        ticker_info: TickerInfo,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_depth_stream(self.clone(), ticker_info, push_freq)
    }

    pub fn connect_trade_stream(
        &self,
        tickers: Vec<TickerInfo>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_trade_stream(tickers, market_type)
    }

    pub fn connect_kline_stream(
        &self,
        streams: Vec<(TickerInfo, Timeframe)>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_kline_stream(streams, market_type)
    }

    pub fn sender(&self) -> mpsc::Sender<MexcCommand> {
        self.request_port.sender()
    }
}

pub struct Mexc {
    hub: HttpHub<MexcLimiter>,
}

impl Mexc {
    pub fn new() -> Result<Self, AdapterError> {
        Self::with_config(MexcConfig)
    }

    pub fn with_config(_config: MexcConfig) -> Result<Self, AdapterError> {
        let hub = HttpHub::new(MexcLimiter::default())?;
        Ok(Self { hub })
    }

    pub fn http_hub(&self) -> &HttpHub<MexcLimiter> {
        &self.hub
    }

    pub fn http_hub_mut(&mut self) -> &mut HttpHub<MexcLimiter> {
        &mut self.hub
    }

    pub async fn run(mut self, mut command_rx: mpsc::Receiver<MexcCommand>) {
        while let Some(command) = command_rx.recv().await {
            fetch::handle_command(&mut self, command).await;
        }
    }
}

impl super::FetchCommandHandler<MexcMarketScope> for Mexc {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: MexcMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_metadata_with_hub(self.http_hub(), &market_scope.markets).await
        })
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: MexcMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_stats_with_hub(
                self.http_hub(),
                &market_scope.markets,
                market_scope.contract_sizes.as_ref(),
            )
            .await
        })
    }

    fn fetch_klines(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_klines_with_hub(self.http_hub(), ticker_info, timeframe, range).await
        })
    }
}

pub fn spawn_default_mexc() -> Result<MexcHandle, AdapterError> {
    let worker = Mexc::new()?;
    let request_port =
        super::spawn_request_port(DEFAULT_COMMAND_BUFFER_CAPACITY, move |receiver| {
            worker.run(receiver)
        });

    Ok(MexcHandle::new(request_port))
}
