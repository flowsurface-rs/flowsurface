use crate::{
    Event, Kline, OpenInterest, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, UnixMs,
    adapter::limiter::DynamicRateLimiterConfig,
    adapter::{AdapterNetworkConfig, Exchange, MarketKind},
    depth::DepthPayload,
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::{collections::HashMap, path::PathBuf, time::Duration};

pub mod fetch;
pub mod stream;

const SPOT_DOMAIN: &str = "https://api.binance.com";
const LINEAR_PERP_DOMAIN: &str = "https://fapi.binance.com";
const INVERSE_PERP_DOMAIN: &str = "https://dapi.binance.com";

const SPOT_LIMIT: usize = 6000;
const PERPS_LIMIT: usize = 2400;
const REFILL_RATE: Duration = Duration::from_secs(60);
const LIMITER_BUFFER_PCT: f32 = 0.03;
const USED_WEIGHT_HEADER: &str = "x-mbx-used-weight-1m";
const DEFAULT_COMMAND_BUFFER_CAPACITY: usize = 128;
const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000;

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::BinanceSpot,
        MarketKind::LinearPerps => Exchange::BinanceLinear,
        MarketKind::InversePerps => Exchange::BinanceInverse,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot | MarketKind::LinearPerps => RawQtyUnit::Base,
        MarketKind::InversePerps => RawQtyUnit::Contracts,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BinanceConfig {
    pub spot_limit: usize,
    pub perps_limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            spot_limit: SPOT_LIMIT,
            perps_limit: PERPS_LIMIT,
            refill_rate: REFILL_RATE,
            limiter_buffer_pct: LIMITER_BUFFER_PCT,
        }
    }
}

impl BinanceConfig {
    fn limiter_config_for_market(self, market: MarketKind) -> DynamicRateLimiterConfig {
        let max_weight = match market {
            MarketKind::Spot => self.spot_limit,
            MarketKind::LinearPerps | MarketKind::InversePerps => self.perps_limit,
        };

        DynamicRateLimiterConfig::new(
            max_weight,
            self.refill_rate,
            self.limiter_buffer_pct,
            USED_WEIGHT_HEADER,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(reqwest::StatusCode::IM_A_TEAPOT),
        )
    }
}

pub type BinanceLimiter = crate::adapter::limiter::HeaderDynamicRateLimiter;

#[derive(Debug, Clone)]
pub struct BinanceMarketScope {
    pub market: MarketKind,
    pub contract_sizes: Option<HashMap<Ticker, f32>>,
}

impl BinanceMarketScope {
    pub fn metadata(market: MarketKind) -> Self {
        Self {
            market,
            contract_sizes: None,
        }
    }

    pub fn stats(market: MarketKind, contract_sizes: Option<HashMap<Ticker, f32>>) -> Self {
        Self {
            market,
            contract_sizes,
        }
    }
}

type BinanceCommand = super::FetchCommand<BinanceMarketScope>;

#[derive(Clone)]
pub struct BinanceHandle {
    request_port: RequestPort<BinanceCommand>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl BinanceHandle {
    fn new(
        request_port: RequestPort<BinanceCommand>,
        proxy_cfg: Option<crate::proxy::Proxy>,
    ) -> Self {
        Self {
            request_port,
            proxy_cfg,
        }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: BinanceMarketScope,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| BinanceCommand::TickerMetadata {
                market_scope,
                reply,
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market_scope: BinanceMarketScope,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| BinanceCommand::TickerStats {
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
            .request(move |reply| BinanceCommand::Klines {
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
        range: Option<(UnixMs, UnixMs)>,
    ) -> Result<Vec<OpenInterest>, AdapterError> {
        self.request_port
            .request(move |reply| BinanceCommand::OpenInterest {
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
        data_path: Option<PathBuf>,
    ) -> Result<Vec<Trade>, AdapterError> {
        self.request_port
            .request(move |reply| BinanceCommand::Trades {
                ticker,
                from_time,
                data_path,
                reply,
            })
            .await
    }

    pub async fn fetch_depth_snapshot(&self, ticker: Ticker) -> Result<DepthPayload, AdapterError> {
        self.request_port
            .request(move |reply| BinanceCommand::DepthSnapshot { ticker, reply })
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
    spot_hub: HttpHub<BinanceLimiter>,
    linear_hub: HttpHub<BinanceLimiter>,
    inverse_hub: HttpHub<BinanceLimiter>,
}

impl Worker {
    fn new_with_network(network: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        let config = BinanceConfig::default();

        let spot_hub = HttpHub::new(
            BinanceLimiter::new(config.limiter_config_for_market(MarketKind::Spot)),
            network.proxy_cfg.clone(),
        )?;
        let linear_hub = HttpHub::new(
            BinanceLimiter::new(config.limiter_config_for_market(MarketKind::LinearPerps)),
            network.proxy_cfg.clone(),
        )?;
        let inverse_hub = HttpHub::new(
            BinanceLimiter::new(config.limiter_config_for_market(MarketKind::InversePerps)),
            network.proxy_cfg,
        )?;

        Ok(Self {
            spot_hub,
            linear_hub,
            inverse_hub,
        })
    }

    fn hub_for_market(&mut self, market: MarketKind) -> &mut HttpHub<BinanceLimiter> {
        match market {
            MarketKind::Spot => &mut self.spot_hub,
            MarketKind::LinearPerps => &mut self.linear_hub,
            MarketKind::InversePerps => &mut self.inverse_hub,
        }
    }
}

impl super::FetchCommandHandler<BinanceMarketScope> for Worker {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: BinanceMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        let market = market_scope.market;
        Box::pin(
            async move { fetch::fetch_ticker_metadata(self.hub_for_market(market), market).await },
        )
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: BinanceMarketScope,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        let market = market_scope.market;
        Box::pin(async move {
            fetch::fetch_ticker_stats(
                self.hub_for_market(market),
                market,
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
        let market = ticker_info.market_type();
        Box::pin(async move {
            fetch::fetch_klines(self.hub_for_market(market), ticker_info, timeframe, range).await
        })
    }

    fn fetch_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(UnixMs, UnixMs)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<OpenInterest>, AdapterError>> {
        let market = ticker_info.market_type();
        Box::pin(async move {
            fetch::fetch_historical_oi(self.hub_for_market(market), ticker_info, range, timeframe)
                .await
        })
    }

    fn fetch_depth_snapshot(
        &mut self,
        ticker: Ticker,
    ) -> futures::future::BoxFuture<'_, Result<DepthPayload, AdapterError>> {
        let market = ticker.market_type();
        Box::pin(
            async move { fetch::fetch_depth_snapshot(self.hub_for_market(market), ticker).await },
        )
    }

    fn fetch_trades(
        &mut self,
        ticker_info: TickerInfo,
        from_time: UnixMs,
        data_path: Option<PathBuf>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Trade>, AdapterError>> {
        let market = ticker_info.market_type();
        Box::pin(async move {
            fetch::fetch_trades(
                self.hub_for_market(market),
                ticker_info,
                from_time,
                data_path,
            )
            .await
        })
    }
}

pub fn spawn_binance_with_network(
    network: AdapterNetworkConfig,
) -> Result<BinanceHandle, AdapterError> {
    let proxy_cfg = network.proxy_cfg.clone();
    let worker = Worker::new_with_network(network)?;
    let request_port = super::spawn_fetch_worker(DEFAULT_COMMAND_BUFFER_CAPACITY, worker);

    Ok(BinanceHandle::new(request_port, proxy_cfg))
}
