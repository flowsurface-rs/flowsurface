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

const WS_DOMAIN: &str = "stream.bybit.com";
const FETCH_DOMAIN: &str = "https://api.bybit.com";
const LIMIT: usize = 600;
const REFILL_RATE: Duration = Duration::from_secs(5);
const LIMITER_BUFFER_PCT: f32 = 0.05;
const DEFAULT_COMMAND_BUFFER_CAPACITY: usize = 128;

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::BybitSpot,
        MarketKind::LinearPerps => Exchange::BybitLinear,
        MarketKind::InversePerps => Exchange::BybitInverse,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot | MarketKind::LinearPerps => RawQtyUnit::Base,
        MarketKind::InversePerps => RawQtyUnit::Quote,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BybitConfig {
    pub limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
}

impl Default for BybitConfig {
    fn default() -> Self {
        Self {
            limit: LIMIT,
            refill_rate: REFILL_RATE,
            limiter_buffer_pct: LIMITER_BUFFER_PCT,
        }
    }
}

impl BybitConfig {
    fn limiter_config(self) -> FixedWindowRateLimiterConfig {
        FixedWindowRateLimiterConfig::new(
            self.limit,
            self.refill_rate,
            self.limiter_buffer_pct,
            reqwest::StatusCode::FORBIDDEN,
        )
    }
}

pub type BybitLimiter = crate::adapter::limiter::FixedWindowRateLimiter;

type BybitCommand = super::FetchCommand<MarketKind>;

#[derive(Clone)]
pub struct BybitHandle {
    request_port: RequestPort<BybitCommand>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl BybitHandle {
    fn new(
        request_port: RequestPort<BybitCommand>,
        proxy_cfg: Option<crate::proxy::Proxy>,
    ) -> Self {
        Self {
            request_port,
            proxy_cfg,
        }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market: MarketKind,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| BybitCommand::TickerMetadata {
                market_scope: market,
                reply,
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market: MarketKind,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| BybitCommand::TickerStats {
                market_scope: market,
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
            .request(move |reply| BybitCommand::Klines {
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
            .request(move |reply| BybitCommand::OpenInterest {
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
    hub: HttpHub<BybitLimiter>,
}

impl Worker {
    fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = BybitConfig::default();

        let limiter = BybitLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter, network.proxy_cfg)?;

        Ok(Self { hub })
    }
}

impl super::FetchCommandHandler<MarketKind> for Worker {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(async move { fetch::fetch_ticker_metadata(&mut self.hub, market_scope).await })
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move { fetch::fetch_ticker_stats(&mut self.hub, market_scope).await })
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

pub fn spawn_bybit_with_network(
    network: AdapterNetworkConfig,
) -> Result<BybitHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Worker::new_with_network(network)?;
    let request_port = super::spawn_fetch_worker(DEFAULT_COMMAND_BUFFER_CAPACITY, worker);

    Ok(BybitHandle::new(request_port, proxy_cfg))
}
