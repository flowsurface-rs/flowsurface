use crate::{
    Event, Kline, OpenInterest, PushFrequency, TickerInfo, Timeframe, Trade,
    adapter::{Exchange, MarketKind},
    unit::qty::RawQtyUnit,
};

use super::{AdapterError, HttpHub, RequestPort};
use std::time::Duration;
use tokio::sync::mpsc;

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
    fn limiter_config(self) -> super::FixedWindowRateLimiterConfig {
        super::FixedWindowRateLimiterConfig::new(
            self.limit,
            self.refill_rate,
            self.limiter_buffer_pct,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        )
    }
}

pub type OkexLimiter = super::FixedWindowRateLimiter;

pub type OkexCommand = super::FetchCommand<Vec<MarketKind>>;

#[derive(Clone)]
pub struct OkexHandle {
    request_port: RequestPort<OkexCommand>,
}

impl OkexHandle {
    pub fn new(request_port: RequestPort<OkexCommand>) -> Self {
        Self { request_port }
    }

    pub async fn fetch_ticker_metadata(
        &self,
        market_scope: Vec<MarketKind>,
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.request_port
            .request(move |reply| OkexCommand::FetchTickerMetadata {
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
            .request(move |reply| OkexCommand::FetchTickerStats {
                market_scope,
                reply,
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
            .request(move |reply| OkexCommand::FetchKlines {
                ticker_info,
                timeframe,
                range,
                reply,
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
            .request(move |reply| OkexCommand::FetchOpenInterest {
                ticker_info,
                timeframe,
                range,
                reply,
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
            .request(move |reply| OkexCommand::FetchTrades {
                ticker_info,
                from_time,
                data_path,
                reply,
            })
            .await
    }

    pub async fn fetch_ticker_metadata_for_markets(
        &self,
        markets: &[MarketKind],
    ) -> Result<super::TickerMetadataMap, AdapterError> {
        self.fetch_ticker_metadata(markets.to_vec()).await
    }

    pub async fn fetch_ticker_stats_for_markets(
        &self,
        markets: &[MarketKind],
    ) -> Result<super::TickerStatsMap, AdapterError> {
        self.fetch_ticker_stats(markets.to_vec()).await
    }

    pub fn connect_depth_stream(
        &self,
        ticker_info: TickerInfo,
        push_freq: PushFrequency,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_depth_stream(ticker_info, push_freq)
    }

    pub fn connect_trade_stream(
        &self,
        streams: Vec<TickerInfo>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_trade_stream(streams, market_type)
    }

    pub fn connect_kline_stream(
        &self,
        streams: Vec<(TickerInfo, Timeframe)>,
        market_type: MarketKind,
    ) -> impl futures::Stream<Item = Event> {
        stream::connect_kline_stream(streams, market_type)
    }

    pub fn sender(&self) -> mpsc::Sender<OkexCommand> {
        self.request_port.sender()
    }
}

pub struct Okex {
    hub: HttpHub<OkexLimiter>,
}

impl Okex {
    pub fn new() -> Result<Self, AdapterError> {
        Self::with_config(OkexConfig::default())
    }

    pub fn with_config(config: OkexConfig) -> Result<Self, AdapterError> {
        let limiter = OkexLimiter::new(config.limiter_config());
        let hub = HttpHub::new(limiter)?;

        Ok(Self { hub })
    }

    pub fn http_hub(&self) -> &HttpHub<OkexLimiter> {
        &self.hub
    }

    pub fn http_hub_mut(&mut self) -> &mut HttpHub<OkexLimiter> {
        &mut self.hub
    }

    pub async fn run(mut self, mut command_rx: mpsc::Receiver<OkexCommand>) {
        super::run_fetch_loop(&mut self, &mut command_rx).await;
    }
}

impl super::FetchCommandHandler<Vec<MarketKind>> for Okex {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: Vec<MarketKind>,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerMetadataMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_metadata_with_hub(self.http_hub_mut(), &market_scope).await
        })
    }

    fn fetch_ticker_stats(
        &mut self,
        market_scope: Vec<MarketKind>,
    ) -> futures::future::BoxFuture<'_, Result<super::TickerStatsMap, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_ticker_stats_with_hub(self.http_hub_mut(), &market_scope).await
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

    fn fetch_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> futures::future::BoxFuture<'_, Result<Vec<OpenInterest>, AdapterError>> {
        Box::pin(async move {
            fetch::fetch_historical_oi_with_hub(self.http_hub_mut(), ticker_info, range, timeframe)
                .await
        })
    }
}

pub fn spawn_default_okex() -> Result<OkexHandle, AdapterError> {
    let worker = Okex::new()?;
    let request_port =
        super::spawn_request_port(DEFAULT_COMMAND_BUFFER_CAPACITY, move |receiver| {
            worker.run(receiver)
        });

    Ok(OkexHandle::new(request_port))
}
