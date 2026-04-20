use crate::{
    Event, Kline, PushFrequency, TickMultiplier, TickerInfo, Timeframe, Trade, UnixMs,
    adapter::limiter::FixedWindowRateLimiterConfig,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    depth::DepthPayload,
    unit::{MinTicksize, qty::RawQtyUnit},
};

use super::{AdapterError, HttpHub, RequestPort};
use std::time::Duration;

pub mod fetch;
pub mod stream;

const API_DOMAIN: &str = "https://api.hyperliquid.xyz";
const WS_DOMAIN: &str = "api.hyperliquid.xyz";
const MAX_DECIMALS_PERP: u8 = 6;
const SIG_FIG_LIMIT: i32 = 5;

const LIMIT: usize = 1200;
const REFILL_RATE: Duration = Duration::from_secs(60);
const LIMITER_BUFFER_PCT: f32 = 0.05;
const DEFAULT_COMMAND_BUFFER_CAPACITY: usize = 128;

const _MAX_DECIMALS_SPOT: u8 = 8;

const MULTS_OVERFLOW: &[u16] = &[1, 10, 20, 50, 100, 1000, 10000];
const MULTS_FRACTIONAL: &[u16] = &[1, 2, 5, 10, 100, 1000];

// safe intersection when base tick is exactly 1 (cannot disambiguate boundary case)
const MULTS_SAFE: &[u16] = &[1, 10, 100, 1000];

pub fn allowed_multipliers_for_min_tick(min_ticksize: MinTicksize) -> &'static [u16] {
    if min_ticksize.power < 0 {
        // int_digits <= 4 (fractional/boundary region)
        MULTS_FRACTIONAL
    } else if min_ticksize.power > 0 {
        MULTS_OVERFLOW
    } else {
        // min tick == 1: could be exactly 5 digits or overflow (>=6).
        MULTS_SAFE
    }
}

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::HyperliquidSpot,
        MarketKind::LinearPerps | MarketKind::InversePerps => Exchange::HyperliquidLinear,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot | MarketKind::LinearPerps | MarketKind::InversePerps => RawQtyUnit::Base,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HyperliquidConfig {
    pub limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
}

impl Default for HyperliquidConfig {
    fn default() -> Self {
        Self {
            limit: LIMIT,
            refill_rate: REFILL_RATE,
            limiter_buffer_pct: LIMITER_BUFFER_PCT,
        }
    }
}

impl HyperliquidConfig {
    fn limiter_config(self) -> FixedWindowRateLimiterConfig {
        FixedWindowRateLimiterConfig::new(
            self.limit,
            self.refill_rate,
            self.limiter_buffer_pct,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        )
    }
}

pub type HyperliquidLimiter = crate::adapter::limiter::FixedWindowRateLimiter;

type HyperliquidCommand = super::FetchCommand<MarketKind>;

#[derive(Clone)]
pub struct HyperliquidHandle {
    request_port: RequestPort<HyperliquidCommand>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl HyperliquidHandle {
    fn new(
        request_port: RequestPort<HyperliquidCommand>,
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
            .request(move |reply| HyperliquidCommand::TickerMetadata {
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
            .request(move |reply| HyperliquidCommand::TickerStats {
                market_scope: market,
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
            .request(move |reply| HyperliquidCommand::Klines {
                ticker,
                timeframe,
                range,
                reply,
            })
            .await
    }

    pub async fn fetch_trades(
        &self,
        ticker: TickerInfo,
        from_time: UnixMs,
        data_path: Option<std::path::PathBuf>,
    ) -> Result<Vec<Trade>, AdapterError> {
        self.request_port
            .request(move |reply| HyperliquidCommand::Trades {
                ticker,
                from_time,
                data_path,
                reply,
            })
            .await
    }

    pub async fn fetch_depth_snapshot(
        &self,
        ticker: crate::Ticker,
    ) -> Result<DepthPayload, AdapterError> {
        self.request_port
            .request(move |reply| HyperliquidCommand::DepthSnapshot { ticker, reply })
            .await
    }

    pub fn connect_depth_stream(
        self,
        ticker_info: TickerInfo,
        tick_multiplier: Option<TickMultiplier>,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        let proxy_cfg = self.proxy_cfg.clone();
        stream::connect_depth_stream(self, ticker_info, tick_multiplier, push_freq, proxy_cfg)
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
    hub: HttpHub<HyperliquidLimiter>,
}

impl Worker {
    pub fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = HyperliquidConfig::default();

        let limiter = HyperliquidLimiter::new(config.limiter_config());
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
        range: Option<(UnixMs, UnixMs)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(
            async move { fetch::fetch_klines(&mut self.hub, ticker_info, timeframe, range).await },
        )
    }

    fn fetch_depth_snapshot(
        &mut self,
        ticker: crate::Ticker,
    ) -> futures::future::BoxFuture<'_, Result<DepthPayload, AdapterError>> {
        Box::pin(async move { fetch::fetch_depth_snapshot(&mut self.hub, ticker).await })
    }
}

pub fn spawn_hyperliquid_with_network(
    network: AdapterNetworkConfig,
) -> Result<HyperliquidHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Worker::new_with_network(network)?;
    let request_port = super::spawn_fetch_worker(DEFAULT_COMMAND_BUFFER_CAPACITY, worker);

    Ok(HyperliquidHandle::new(request_port, proxy_cfg))
}
