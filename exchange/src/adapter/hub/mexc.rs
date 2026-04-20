use crate::{
    Event, Kline, PushFrequency, Ticker, TickerInfo, Timeframe, UnixMs,
    adapter::limiter::FixedWindowRateLimiterConfig,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    depth::DepthPayload,
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::{collections::HashMap, time::Duration};

pub mod fetch;
pub mod stream;

const FETCH_DOMAIN: &str = "https://api.mexc.com/api";
const MEXC_FUTURES_WS_DOMAIN: &str = "contract.mexc.com";
const MEXC_FUTURES_WS_PATH: &str = "/edge";
const PING_INTERVAL: u64 = 15;
const LIMIT: usize = 10;
const REFILL_RATE: Duration = Duration::from_secs(2);
const LIMITER_BUFFER_PCT: f32 = 0.0;
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

#[derive(Debug, Clone, Copy)]
pub struct MexcConfig {
    pub limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
}

impl Default for MexcConfig {
    fn default() -> Self {
        Self {
            limit: LIMIT,
            refill_rate: REFILL_RATE,
            limiter_buffer_pct: LIMITER_BUFFER_PCT,
        }
    }
}

impl MexcConfig {
    fn limiter_config(self) -> FixedWindowRateLimiterConfig {
        FixedWindowRateLimiterConfig::new(
            self.limit,
            self.refill_rate,
            self.limiter_buffer_pct,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        )
    }
}

pub type MexcLimiter = crate::adapter::limiter::FixedWindowRateLimiter;

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

type MexcCommand = super::FetchCommand<MexcMarketScope>;

#[derive(Clone)]
pub struct MexcHandle {
    request_port: RequestPort<MexcCommand>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl MexcHandle {
    fn new(request_port: RequestPort<MexcCommand>, proxy_cfg: Option<crate::proxy::Proxy>) -> Self {
        Self {
            request_port,
            proxy_cfg,
        }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: MexcMarketScope,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| MexcCommand::TickerMetadata {
                market_scope,
                reply,
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market_scope: MexcMarketScope,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| MexcCommand::TickerStats {
                market_scope,
                reply,
            })
            .await
    }

    pub async fn fetch_klines(
        &self,
        ticker: TickerInfo,
        timeframe: Timeframe,
        range: Option<(UnixMs, UnixMs)>,
    ) -> Result<Vec<Kline>, AdapterError> {
        self.request_port
            .request(move |reply| MexcCommand::Klines {
                ticker,
                timeframe,
                range,
                reply,
            })
            .await
    }

    pub async fn fetch_depth_snapshot(&self, ticker: Ticker) -> Result<DepthPayload, AdapterError> {
        self.request_port
            .request(move |reply| MexcCommand::DepthSnapshot { ticker, reply })
            .await
    }

    pub fn connect_depth_stream(
        self,
        ticker_info: TickerInfo,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        let proxy_cfg = self.proxy_cfg.clone();
        stream::connect_depth_stream(self, ticker_info, push_freq, proxy_cfg)
    }

    pub fn connect_trade_stream(
        self,
        tickers: Vec<TickerInfo>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_trade_stream(tickers, market_type, self.proxy_cfg)
    }

    pub fn connect_kline_stream(
        self,
        streams: Vec<(TickerInfo, Timeframe)>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_kline_stream(streams, market_type, self.proxy_cfg)
    }
}

struct Worker {
    hub: HttpHub<MexcLimiter>,
}

impl Worker {
    fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = MexcConfig::default();

        let limiter = MexcLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter, network.proxy_cfg)?;

        Ok(Self { hub })
    }
}

impl super::FetchCommandHandler<MexcMarketScope> for Worker {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: MexcMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(
            async move { fetch::fetch_ticker_metadata(&mut self.hub, &market_scope.markets).await },
        )
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: MexcMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_stats(
                &mut self.hub,
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
        range: Option<(UnixMs, UnixMs)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(
            async move { fetch::fetch_klines(&mut self.hub, ticker_info, timeframe, range).await },
        )
    }

    fn fetch_depth_snapshot(
        &mut self,
        ticker: Ticker,
    ) -> futures::future::BoxFuture<'_, Result<DepthPayload, AdapterError>> {
        Box::pin(async move { fetch::fetch_depth_snapshot(&mut self.hub, ticker).await })
    }
}

pub fn spawn_mexc_with_network(network: AdapterNetworkConfig) -> Result<MexcHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Worker::new_with_network(network)?;
    let request_port = super::spawn_fetch_worker(DEFAULT_COMMAND_BUFFER_CAPACITY, worker);

    Ok(MexcHandle::new(request_port, proxy_cfg))
}
