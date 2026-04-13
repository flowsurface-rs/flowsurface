use crate::{
    Kline, OpenInterest, TickerInfo, Timeframe,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    limiter::FixedWindowRateLimiterConfig,
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::time::Duration;
use tokio::sync::mpsc;

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

pub type BybitLimiter = crate::limiter::FixedWindowRateLimiter;

type BybitCommand = super::FetchCommand<MarketKind>;

#[derive(Clone)]
pub struct BybitHandle {
    request_port: RequestPort<BybitCommand>,
}

impl BybitHandle {
    fn new(request_port: RequestPort<BybitCommand>) -> Self {
        Self { request_port }
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
}

pub struct Bybit {
    hub: HttpHub<BybitLimiter>,
}

impl Bybit {
    pub fn new() -> Result<Self, AdapterError> {
        Self::new_with_network(AdapterNetworkConfig::default())
    }

    pub fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = BybitConfig::default();

        let limiter = BybitLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter, network.proxy_cfg)?;

        Ok(Self { hub })
    }

    pub fn http_hub_mut(&mut self) -> &mut HttpHub<BybitLimiter> {
        &mut self.hub
    }

    async fn run(mut self, mut command_rx: mpsc::Receiver<BybitCommand>) {
        while let Some(command) = command_rx.recv().await {
            super::handle_fetch_command(&mut self, command).await;
        }
    }
}

impl super::FetchCommandHandler<MarketKind> for Bybit {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(
            async move { fetch::fetch_ticker_metadata(self.http_hub_mut(), market_scope).await },
        )
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move { fetch::fetch_ticker_stats(self.http_hub_mut(), market_scope).await })
    }

    fn fetch_klines(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_klines(self.http_hub_mut(), ticker_info, timeframe, range).await
        })
    }

    fn fetch_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<OpenInterest>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_historical_oi(self.http_hub_mut(), ticker_info, range, timeframe).await
        })
    }
}

pub fn spawn_bybit_with_network(
    network: AdapterNetworkConfig,
) -> Result<BybitHandle, AdapterError> {
    let worker = Bybit::new_with_network(network)?;
    let request_port =
        super::spawn_request_port(DEFAULT_COMMAND_BUFFER_CAPACITY, move |receiver| {
            worker.run(receiver)
        });

    Ok(BybitHandle::new(request_port))
}
