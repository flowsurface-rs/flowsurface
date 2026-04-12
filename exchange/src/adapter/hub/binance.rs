use crate::{
    Event, Kline, OpenInterest, PushFrequency, Ticker, TickerInfo, Timeframe, Trade,
    adapter::{Exchange, MarketKind},
    depth::DepthPayload,
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::sync::mpsc;

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
    fn limiter_config_for_market(self, market: MarketKind) -> super::DynamicRateLimiterConfig {
        let max_weight = match market {
            MarketKind::Spot => self.spot_limit,
            MarketKind::LinearPerps | MarketKind::InversePerps => self.perps_limit,
        };

        super::DynamicRateLimiterConfig::new(
            max_weight,
            self.refill_rate,
            self.limiter_buffer_pct,
            USED_WEIGHT_HEADER,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(reqwest::StatusCode::IM_A_TEAPOT),
        )
    }
}

pub type BinanceLimiter = super::HeaderDynamicRateLimiter;

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

pub enum BinanceCommand {
    Fetch(super::FetchCommand<BinanceMarketScope>),
    FetchDepthSnapshot {
        ticker: Ticker,
        reply: super::ResponseTx<DepthPayload>,
    },
}

#[derive(Clone)]
pub struct BinanceHandle {
    request_port: RequestPort<BinanceCommand>,
}

impl BinanceHandle {
    pub fn new(request_port: RequestPort<BinanceCommand>) -> Self {
        Self { request_port }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: BinanceMarketScope,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| {
                BinanceCommand::Fetch(super::FetchCommand::FetchTickerMetadata {
                    market_scope,
                    reply,
                })
            })
            .await
    }

    pub async fn fetch_ticker_stats(
        &self,
        market_scope: BinanceMarketScope,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.request_port
            .request(move |reply| {
                BinanceCommand::Fetch(super::FetchCommand::FetchTickerStats {
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
                BinanceCommand::Fetch(super::FetchCommand::FetchKlines {
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
                BinanceCommand::Fetch(super::FetchCommand::FetchOpenInterest {
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
        data_path: Option<PathBuf>,
    ) -> Result<Vec<Trade>, AdapterError> {
        self.request_port
            .request(move |reply| {
                BinanceCommand::Fetch(super::FetchCommand::FetchTrades {
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
            .request(move |reply| BinanceCommand::FetchDepthSnapshot { ticker, reply })
            .await
    }

    pub async fn fetch_ticker_metadata_for_market(
        &self,
        market: MarketKind,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.fetch_ticker_metadata(BinanceMarketScope::metadata(market))
            .await
    }

    pub async fn fetch_ticker_stats_for_market(
        &self,
        market: MarketKind,
        contract_sizes: Option<HashMap<Ticker, f32>>,
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.fetch_ticker_stats(BinanceMarketScope::stats(market, contract_sizes))
            .await
    }

    pub fn sender(&self) -> mpsc::Sender<BinanceCommand> {
        self.request_port.sender()
    }

    pub fn depth_stream(
        &self,
        ticker_info: TickerInfo,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_depth_stream(self.clone(), ticker_info, push_freq)
    }
}

pub struct Binance {
    spot_hub: HttpHub<BinanceLimiter>,
    linear_hub: HttpHub<BinanceLimiter>,
    inverse_hub: HttpHub<BinanceLimiter>,
}

impl Binance {
    pub fn new() -> Result<Self, AdapterError> {
        Self::with_config(BinanceConfig::default())
    }

    pub fn with_config(config: BinanceConfig) -> Result<Self, AdapterError> {
        let spot_hub = HttpHub::new(BinanceLimiter::new(
            config.limiter_config_for_market(MarketKind::Spot),
        ))?;
        let linear_hub = HttpHub::new(BinanceLimiter::new(
            config.limiter_config_for_market(MarketKind::LinearPerps),
        ))?;
        let inverse_hub = HttpHub::new(BinanceLimiter::new(
            config.limiter_config_for_market(MarketKind::InversePerps),
        ))?;

        Ok(Self {
            spot_hub,
            linear_hub,
            inverse_hub,
        })
    }

    pub fn http_hub(&self) -> &HttpHub<BinanceLimiter> {
        self.http_hub_for_market(MarketKind::Spot)
    }

    pub fn http_hub_mut(&mut self) -> &mut HttpHub<BinanceLimiter> {
        self.http_hub_for_market_mut(MarketKind::Spot)
    }

    pub fn http_hub_for_market(&self, market: MarketKind) -> &HttpHub<BinanceLimiter> {
        match market {
            MarketKind::Spot => &self.spot_hub,
            MarketKind::LinearPerps => &self.linear_hub,
            MarketKind::InversePerps => &self.inverse_hub,
        }
    }

    pub fn http_hub_for_market_mut(&mut self, market: MarketKind) -> &mut HttpHub<BinanceLimiter> {
        match market {
            MarketKind::Spot => &mut self.spot_hub,
            MarketKind::LinearPerps => &mut self.linear_hub,
            MarketKind::InversePerps => &mut self.inverse_hub,
        }
    }

    pub async fn run(mut self, mut command_rx: mpsc::Receiver<BinanceCommand>) {
        while let Some(command) = command_rx.recv().await {
            fetch::handle_command(&mut self, command).await;
        }
    }
}

pub fn spawn_default_binance() -> Result<BinanceHandle, AdapterError> {
    let worker = Binance::new()?;
    let request_port =
        super::spawn_request_port(DEFAULT_COMMAND_BUFFER_CAPACITY, move |receiver| {
            worker.run(receiver)
        });

    Ok(BinanceHandle::new(request_port))
}
