use crate::{
    Event, Kline, OpenInterest, PushFrequency, TickerInfo, Timeframe,
    adapter::limiter::FixedWindowRateLimiterConfig,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::time::Duration;

pub mod fetch;
pub mod stream;

const WS_DOMAIN: &str = "ws.okx.com";
const REST_API_BASE: &str = "https://www.okx.com/api/v5";
const LIMIT: usize = 20;
const REFILL_RATE: Duration = Duration::from_secs(2);
const LIMITER_BUFFER_PCT: f32 = 0.05;
const DEFAULT_COMMAND_BUFFER_CAPACITY: usize = 128;

fn exchange_from_market_type(market_type: MarketKind) -> Exchange {
    match market_type {
        MarketKind::Spot => Exchange::OkexSpot,
        MarketKind::LinearPerps => Exchange::OkexLinear,
        MarketKind::InversePerps => Exchange::OkexInverse,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot => RawQtyUnit::Base,
        MarketKind::LinearPerps | MarketKind::InversePerps => RawQtyUnit::Contracts,
    }
}

fn timeframe_to_okx_bar(tf: Timeframe) -> Option<&'static str> {
    Some(match tf {
        Timeframe::M1 => "1m",
        Timeframe::M3 => "3m",
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::M30 => "30m",
        Timeframe::H1 => "1H",
        Timeframe::H2 => "2H",
        Timeframe::H4 => "4H",
        Timeframe::H12 => "12Hutc",
        Timeframe::D1 => "1Dutc",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
pub struct OkexConfig {
    pub limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
}

impl Default for OkexConfig {
    fn default() -> Self {
        Self {
            limit: LIMIT,
            refill_rate: REFILL_RATE,
            limiter_buffer_pct: LIMITER_BUFFER_PCT,
        }
    }
}

impl OkexConfig {
    fn limiter_config(self) -> FixedWindowRateLimiterConfig {
        FixedWindowRateLimiterConfig::new(
            self.limit,
            self.refill_rate,
            self.limiter_buffer_pct,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        )
    }
}

pub type OkexLimiter = crate::adapter::limiter::FixedWindowRateLimiter;

type OkexCommand = super::FetchCommand<Vec<MarketKind>>;

#[derive(Clone)]
pub struct OkexHandle {
    request_port: RequestPort<OkexCommand>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl OkexHandle {
    fn new(request_port: RequestPort<OkexCommand>, proxy_cfg: Option<crate::proxy::Proxy>) -> Self {
        Self {
            request_port,
            proxy_cfg,
        }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: Vec<MarketKind>,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| OkexCommand::TickerMetadata {
                market_scope,
                reply,
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market_scope: Vec<MarketKind>,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| OkexCommand::TickerStats {
                market_scope,
                reply,
            })
            .await
    }

    pub async fn fetch_klines(
        &self,
        ticker: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<Kline>, AdapterError> {
        self.request_port
            .request(move |reply| OkexCommand::Klines {
                ticker,
                timeframe,
                range,
                reply,
            })
            .await
    }

    pub async fn fetch_open_interest(
        &self,
        ticker: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<OpenInterest>, AdapterError> {
        self.request_port
            .request(move |reply| OkexCommand::OpenInterest {
                ticker,
                timeframe,
                range,
                reply,
            })
            .await
    }

    pub fn connect_depth_stream(
        self,
        ticker_info: TickerInfo,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_depth_stream(ticker_info, push_freq, self.proxy_cfg)
    }

    pub fn connect_trade_stream(
        self,
        streams: Vec<TickerInfo>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_trade_stream(streams, market_type, self.proxy_cfg)
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
    hub: HttpHub<OkexLimiter>,
}

impl Worker {
    fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = OkexConfig::default();

        let limiter = OkexLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter, network.proxy_cfg)?;

        Ok(Self { hub })
    }
}

impl super::FetchCommandHandler<Vec<MarketKind>> for Worker {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: Vec<MarketKind>,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(async move { fetch::fetch_ticker_metadata(&mut self.hub, &market_scope).await })
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: Vec<MarketKind>,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move { fetch::fetch_ticker_stats(&mut self.hub, &market_scope).await })
    }

    fn fetch_klines(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(
            async move { fetch::fetch_klines(&mut self.hub, ticker_info, timeframe, range).await },
        )
    }

    fn fetch_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<OpenInterest>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_historical_oi(&mut self.hub, ticker_info, range, timeframe).await
        })
    }
}

pub fn spawn_okex_with_network(network: AdapterNetworkConfig) -> Result<OkexHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Worker::new_with_network(network)?;
    let request_port = super::spawn_fetch_worker(DEFAULT_COMMAND_BUFFER_CAPACITY, worker);

    Ok(OkexHandle::new(request_port, proxy_cfg))
}
