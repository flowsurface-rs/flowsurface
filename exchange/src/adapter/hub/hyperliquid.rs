use crate::{
    Event, Kline, OpenInterest, PushFrequency, TickMultiplier, TickerInfo, Timeframe, Trade,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    depth::DepthPayload,
    limiter::FixedWindowRateLimiterConfig,
    unit::{MinTicksize, qty::RawQtyUnit},
};

use super::{AdapterError, HttpHub, RequestPort};
use std::time::Duration;
use tokio::sync::mpsc;

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

pub type HyperliquidLimiter = crate::limiter::FixedWindowRateLimiter;

enum HyperliquidCommand {
    Fetch(super::FetchCommand<MarketKind>),
    FetchDepthSnapshot {
        ticker: crate::Ticker,
        reply: super::ResponseTx<DepthPayload>,
    },
}

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
            .request(move |reply| {
                HyperliquidCommand::Fetch(super::FetchCommand::TickerMetadata {
                    market_scope: market,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market: MarketKind,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| {
                HyperliquidCommand::Fetch(super::FetchCommand::TickerStats {
                    market_scope: market,
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
                HyperliquidCommand::Fetch(super::FetchCommand::Klines {
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
                HyperliquidCommand::Fetch(super::FetchCommand::OpenInterest {
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
                HyperliquidCommand::Fetch(super::FetchCommand::Trades {
                    ticker_info,
                    from_time,
                    data_path,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_depth_snapshot(
        &self,
        ticker: crate::Ticker,
    ) -> Result<DepthPayload, AdapterError> {
        self.request_port
            .request(move |reply| HyperliquidCommand::FetchDepthSnapshot { ticker, reply })
            .await
    }

    pub fn connect_depth_stream(
        &self,
        ticker_info: TickerInfo,
        tick_multiplier: Option<TickMultiplier>,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_depth_stream(
            self.clone(),
            ticker_info,
            tick_multiplier,
            push_freq,
            self.proxy_cfg.clone(),
        )
    }

    pub fn connect_trade_stream(
        &self,
        tickers: Vec<TickerInfo>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_trade_stream(tickers, market_type, self.proxy_cfg.clone())
    }

    pub fn connect_kline_stream(
        &self,
        streams: Vec<(TickerInfo, Timeframe)>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_kline_stream(streams, market_type, self.proxy_cfg.clone())
    }
}

pub struct Hyperliquid {
    hub: HttpHub<HyperliquidLimiter>,
}

impl Hyperliquid {
    pub fn new() -> Result<Self, AdapterError> {
        Self::new_with_network(AdapterNetworkConfig::default())
    }

    pub fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        Self::with_config_and_network(HyperliquidConfig::default(), network)
    }

    pub fn with_config(config: HyperliquidConfig) -> Result<Self, AdapterError> {
        Self::with_config_and_network(config, AdapterNetworkConfig::default())
    }

    pub fn with_config_and_network(
        config: HyperliquidConfig,
        network: AdapterNetworkConfig,
    ) -> Result<Self, AdapterError> {
        let limiter = HyperliquidLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter, network.proxy_cfg)?;

        Ok(Self { hub })
    }

    pub fn http_hub(&self) -> &HttpHub<HyperliquidLimiter> {
        &self.hub
    }

    pub fn http_hub_mut(&mut self) -> &mut HttpHub<HyperliquidLimiter> {
        &mut self.hub
    }

    async fn run(mut self, mut command_rx: mpsc::Receiver<HyperliquidCommand>) {
        while let Some(command) = command_rx.recv().await {
            fetch::handle_command(&mut self, command).await;
        }
    }
}

impl super::FetchCommandHandler<MarketKind> for Hyperliquid {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_metadata_with_hub(self.http_hub_mut(), market_scope).await
        })
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: MarketKind,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_stats_with_hub(self.http_hub_mut(), market_scope).await
        })
    }

    fn fetch_klines(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_klines_with_hub(self.http_hub_mut(), ticker_info, timeframe, range).await
        })
    }
}

pub fn spawn_default_hyperliquid() -> Result<HyperliquidHandle, AdapterError> {
    spawn_hyperliquid_with_network(AdapterNetworkConfig::default())
}

pub fn spawn_hyperliquid_with_network(
    network: AdapterNetworkConfig,
) -> Result<HyperliquidHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Hyperliquid::new_with_network(network)?;
    let request_port =
        super::spawn_request_port(DEFAULT_COMMAND_BUFFER_CAPACITY, move |receiver| {
            worker.run(receiver)
        });

    Ok(HyperliquidHandle::new(request_port, proxy_cfg))
}
