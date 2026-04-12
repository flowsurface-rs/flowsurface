use super::AdapterError;
use crate::{
    Kline, OpenInterest, Ticker, TickerInfo, TickerStats, Timeframe, Trade,
    limiter::{DynamicBucket, FixedWindowBucket, RateLimiter},
};

use futures::future::BoxFuture;
use serde::de::DeserializeOwned;

use reqwest::{Client, Method, Response, header};
use tokio::sync::{mpsc, oneshot};

use std::{collections::HashMap, future::Future, path::PathBuf, time::Duration};

pub mod binance;
pub mod bybit;
pub mod hyperliquid;
pub mod mexc;
pub mod okex;

const DEFAULT_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
pub struct HttpHubConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for HttpHubConfig {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_HTTP_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_HTTP_REQUEST_TIMEOUT,
        }
    }
}

/// Shared per-handle state that avoids process-wide HTTP and limiter globals.
pub struct HttpHub<L> {
    client: Client,
    limiter: L,
    config: HttpHubConfig,
}

impl<L> HttpHub<L> {
    pub fn new(limiter: L) -> Result<Self, AdapterError> {
        Self::with_config(limiter, HttpHubConfig::default())
    }

    pub fn with_config(limiter: L, config: HttpHubConfig) -> Result<Self, AdapterError> {
        let client = build_http_client(config)?;

        Ok(Self {
            client,
            limiter,
            config,
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn limiter(&self) -> &L {
        &self.limiter
    }

    pub fn limiter_mut(&mut self) -> &mut L {
        &mut self.limiter
    }

    pub fn config(&self) -> HttpHubConfig {
        self.config
    }
}

fn build_http_client(config: HttpHubConfig) -> Result<Client, AdapterError> {
    let builder = Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout);

    let runtime_proxy = crate::proxy::runtime_proxy_cfg();
    let builder = crate::proxy::try_apply_proxy(builder, runtime_proxy.as_ref());

    builder.build().map_err(|error| {
        AdapterError::InvalidRequest(format!("Failed to build worker HTTP client: {error}"))
    })
}

async fn send_request_with_hub_client(
    client: &Client,
    method: Method,
    url: &str,
    json_body: Option<&serde_json::Value>,
) -> Result<Response, reqwest::Error> {
    let mut request_builder = client.request(method, url);

    if let Some(body) = json_body {
        request_builder = request_builder.json(body);
    }

    request_builder.send().await
}

fn body_preview(body: &str, limit: usize) -> String {
    let trimmed = body.trim();
    let mut preview = trimmed.chars().take(limit).collect::<String>();

    if trimmed.chars().count() > limit {
        preview.push_str("...");
    }

    preview
}

async fn read_response_body(
    method: &Method,
    url: &str,
    response: Response,
) -> Result<String, AdapterError> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let body = response.bytes().await.map_err(|error| {
        AdapterError::response_body_failed(method, url, status, &content_type, error)
    })?;

    let body_text = String::from_utf8_lossy(&body).into_owned();

    if !status.is_success() {
        let msg = format!(
            "{} {}: HTTP {} | content-type={} | response_len={} | preview={:?}",
            method,
            url,
            status,
            content_type,
            body.len(),
            body_preview(&body_text, 200)
        );
        log::error!("{}", msg);
        return Err(AdapterError::http_status_failed(status, msg));
    }

    Ok(body_text)
}

pub async fn http_request_with_hub<L>(
    hub: &HttpHub<L>,
    url: &str,
    method: Option<Method>,
    json_body: Option<&serde_json::Value>,
) -> Result<String, AdapterError> {
    let method = method.unwrap_or(Method::GET);
    let request_method = method.clone();

    let response = send_request_with_hub_client(hub.client(), method, url, json_body)
        .await
        .map_err(|error| AdapterError::request_failed(&request_method, url, error))?;

    read_response_body(&request_method, url, response).await
}

pub async fn http_request_with_hub_limiter<L: RateLimiter>(
    hub: &mut HttpHub<L>,
    url: &str,
    weight: usize,
    method: Option<Method>,
    json_body: Option<&serde_json::Value>,
) -> Result<String, AdapterError> {
    let method = method.unwrap_or(Method::GET);
    let request_method = method.clone();

    {
        let limiter = hub.limiter_mut();
        if let Some(wait_time) = limiter.prepare_request(weight) {
            log::warn!("Rate limit hit for: {url}. Waiting for {:?}", wait_time);
            tokio::time::sleep(wait_time).await;
        }
    }

    let response = send_request_with_hub_client(hub.client(), method, url, json_body)
        .await
        .map_err(|error| AdapterError::request_failed(&request_method, url, error))?;

    {
        let limiter = hub.limiter_mut();
        if limiter.should_exit_on_response(&response) {
            let status = response.status();
            let msg = format!(
                "HTTP error {} for: {}. Handle limiter exit status reached.",
                status, url
            );
            log::error!("{}", msg);
            return Err(AdapterError::http_status_failed(status, msg));
        }

        limiter.update_from_response(&response, weight);
    }

    read_response_body(&request_method, url, response).await
}

pub async fn http_parse_with_hub_limiter<L, V>(
    hub: &mut HttpHub<L>,
    url: &str,
    weight: usize,
    method: Option<Method>,
    json_body: Option<&serde_json::Value>,
) -> Result<V, AdapterError>
where
    L: RateLimiter,
    V: DeserializeOwned,
{
    let body = http_request_with_hub_limiter(hub, url, weight, method, json_body).await?;
    let trimmed = body.trim();

    if trimmed.is_empty() {
        let msg = format!("Empty response body | url={url}");
        log::error!("{}", msg);
        return Err(AdapterError::ParseError(msg));
    }
    if trimmed.starts_with('<') {
        let msg = format!(
            "Non-JSON (HTML?) response | url={} | len={} | preview={:?}",
            url,
            body.len(),
            body_preview(&body, 200)
        );
        log::error!("{}", msg);
        return Err(AdapterError::ParseError(msg));
    }

    serde_json::from_str(&body).map_err(|e| {
        let msg = format!(
            "JSON parse failed: {} | url={} | response_len={} | preview={:?}",
            e,
            url,
            body.len(),
            body_preview(&body, 200)
        );
        log::error!("{}", msg);
        AdapterError::ParseError(msg)
    })
}

pub type ResponseTx<T> = oneshot::Sender<Result<T, AdapterError>>;

pub fn reply_once<T>(reply: ResponseTx<T>, result: Result<T, AdapterError>) {
    let _ = reply.send(result);
}

pub type TickerMetadataMap = HashMap<Ticker, Option<TickerInfo>>;
pub type TickerStatsMap = HashMap<Ticker, TickerStats>;

pub enum FetchCommand<M> {
    FetchTickerMetadata {
        market_scope: M,
        reply: ResponseTx<TickerMetadataMap>,
    },
    FetchTickerStats {
        market_scope: M,
        reply: ResponseTx<TickerStatsMap>,
    },
    FetchKlines {
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
        reply: ResponseTx<Vec<Kline>>,
    },
    FetchOpenInterest {
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
        reply: ResponseTx<Vec<OpenInterest>>,
    },
    FetchTrades {
        ticker_info: TickerInfo,
        from_time: u64,
        data_path: Option<PathBuf>,
        reply: ResponseTx<Vec<Trade>>,
    },
}

fn unsupported_fetch(feature: &'static str) -> AdapterError {
    AdapterError::InvalidRequest(format!("{feature} is not supported by this worker"))
}

pub trait FetchCommandHandler<M> {
    fn fetch_ticker_metadata(
        &mut self,
        market_scope: M,
    ) -> BoxFuture<'_, Result<TickerMetadataMap, AdapterError>>;

    fn fetch_ticker_stats(
        &mut self,
        market_scope: M,
    ) -> BoxFuture<'_, Result<TickerStatsMap, AdapterError>>;

    fn fetch_klines(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> BoxFuture<'_, Result<Vec<Kline>, AdapterError>> {
        let _ = (ticker_info, timeframe, range);
        Box::pin(async { Err(unsupported_fetch("Kline fetch")) })
    }

    fn fetch_open_interest(
        &mut self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
    ) -> BoxFuture<'_, Result<Vec<OpenInterest>, AdapterError>> {
        let _ = (ticker_info, timeframe, range);
        Box::pin(async { Err(unsupported_fetch("Open interest fetch")) })
    }

    fn fetch_trades(
        &mut self,
        ticker_info: TickerInfo,
        from_time: u64,
        data_path: Option<PathBuf>,
    ) -> BoxFuture<'_, Result<Vec<Trade>, AdapterError>> {
        let _ = (ticker_info, from_time, data_path);
        Box::pin(async { Err(unsupported_fetch("Trades fetch")) })
    }
}

pub async fn handle_fetch_command<H, M>(handler: &mut H, command: FetchCommand<M>)
where
    H: FetchCommandHandler<M>,
{
    match command {
        FetchCommand::FetchTickerMetadata {
            market_scope,
            reply,
        } => {
            let result = handler.fetch_ticker_metadata(market_scope).await;
            reply_once(reply, result);
        }
        FetchCommand::FetchTickerStats {
            market_scope,
            reply,
        } => {
            let result = handler.fetch_ticker_stats(market_scope).await;
            reply_once(reply, result);
        }
        FetchCommand::FetchKlines {
            ticker_info,
            timeframe,
            range,
            reply,
        } => {
            let result = handler.fetch_klines(ticker_info, timeframe, range).await;
            reply_once(reply, result);
        }
        FetchCommand::FetchOpenInterest {
            ticker_info,
            timeframe,
            range,
            reply,
        } => {
            let result = handler
                .fetch_open_interest(ticker_info, timeframe, range)
                .await;
            reply_once(reply, result);
        }
        FetchCommand::FetchTrades {
            ticker_info,
            from_time,
            data_path,
            reply,
        } => {
            let result = handler
                .fetch_trades(ticker_info, from_time, data_path)
                .await;
            reply_once(reply, result);
        }
    }
}

pub async fn run_fetch_loop<H, M>(handler: &mut H, command_rx: &mut mpsc::Receiver<FetchCommand<M>>)
where
    H: FetchCommandHandler<M>,
{
    while let Some(command) = command_rx.recv().await {
        handle_fetch_command(handler, command).await;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FixedWindowRateLimiterConfig {
    pub limit: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
    pub exit_status: reqwest::StatusCode,
}

impl FixedWindowRateLimiterConfig {
    pub fn new(
        limit: usize,
        refill_rate: Duration,
        limiter_buffer_pct: f32,
        exit_status: reqwest::StatusCode,
    ) -> Self {
        Self {
            limit,
            refill_rate,
            limiter_buffer_pct,
            exit_status,
        }
    }
}

pub struct FixedWindowRateLimiter {
    bucket: FixedWindowBucket,
    exit_status: reqwest::StatusCode,
}

impl FixedWindowRateLimiter {
    pub fn new(config: FixedWindowRateLimiterConfig) -> Self {
        let keep_ratio = (1.0 - config.limiter_buffer_pct).clamp(0.0, 1.0);
        let effective_limit = (config.limit as f32 * keep_ratio) as usize;

        Self {
            bucket: FixedWindowBucket::new(effective_limit, config.refill_rate),
            exit_status: config.exit_status,
        }
    }
}

impl RateLimiter for FixedWindowRateLimiter {
    fn prepare_request(&mut self, weight: usize) -> Option<Duration> {
        self.bucket.calculate_wait_time(weight)
    }

    fn update_from_response(&mut self, _response: &reqwest::Response, weight: usize) {
        self.bucket.consume_tokens(weight);
    }

    fn should_exit_on_response(&self, response: &reqwest::Response) -> bool {
        response.status() == self.exit_status
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DynamicRateLimiterConfig {
    pub max_weight: usize,
    pub refill_rate: Duration,
    pub limiter_buffer_pct: f32,
    pub used_weight_header: &'static str,
    pub exit_status: reqwest::StatusCode,
    pub extra_exit_status: Option<reqwest::StatusCode>,
}

impl DynamicRateLimiterConfig {
    pub fn new(
        max_weight: usize,
        refill_rate: Duration,
        limiter_buffer_pct: f32,
        used_weight_header: &'static str,
        exit_status: reqwest::StatusCode,
        extra_exit_status: Option<reqwest::StatusCode>,
    ) -> Self {
        Self {
            max_weight,
            refill_rate,
            limiter_buffer_pct,
            used_weight_header,
            exit_status,
            extra_exit_status,
        }
    }
}

pub struct HeaderDynamicRateLimiter {
    bucket: DynamicBucket,
    used_weight_header: &'static str,
    exit_status: reqwest::StatusCode,
    extra_exit_status: Option<reqwest::StatusCode>,
}

impl HeaderDynamicRateLimiter {
    pub fn new(config: DynamicRateLimiterConfig) -> Self {
        let keep_ratio = (1.0 - config.limiter_buffer_pct).clamp(0.0, 1.0);
        let effective_limit = (config.max_weight as f32 * keep_ratio) as usize;

        Self {
            bucket: DynamicBucket::new(effective_limit, config.refill_rate),
            used_weight_header: config.used_weight_header,
            exit_status: config.exit_status,
            extra_exit_status: config.extra_exit_status,
        }
    }
}

impl RateLimiter for HeaderDynamicRateLimiter {
    fn prepare_request(&mut self, weight: usize) -> Option<Duration> {
        let (wait_time, _reason) = self.bucket.prepare_request(weight);
        wait_time
    }

    fn update_from_response(&mut self, response: &reqwest::Response, _weight: usize) {
        if let Some(header_value) = response
            .headers()
            .get(self.used_weight_header)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.bucket.update_weight(header_value);
        }
    }

    fn should_exit_on_response(&self, response: &reqwest::Response) -> bool {
        let status = response.status();
        status == self.exit_status || Some(status) == self.extra_exit_status
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopRateLimiter;

impl RateLimiter for NoopRateLimiter {
    fn prepare_request(&mut self, _weight: usize) -> Option<Duration> {
        None
    }

    fn update_from_response(&mut self, _response: &reqwest::Response, _weight: usize) {}

    fn should_exit_on_response(&self, _response: &reqwest::Response) -> bool {
        false
    }
}

pub struct RequestPort<C> {
    sender: mpsc::Sender<C>,
}

impl<C> Clone for RequestPort<C> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

impl<C> RequestPort<C> {
    pub fn new(sender: mpsc::Sender<C>) -> Self {
        Self { sender }
    }

    pub fn sender(&self) -> mpsc::Sender<C> {
        self.sender.clone()
    }

    pub async fn request<T>(
        &self,
        build: impl FnOnce(ResponseTx<T>) -> C,
    ) -> Result<T, AdapterError> {
        let (reply_tx, reply_rx) = oneshot::channel();

        self.sender
            .send(build(reply_tx))
            .await
            .map_err(|_| AdapterError::WebsocketError("Request port is closed".to_string()))?;

        reply_rx
            .await
            .map_err(|_| AdapterError::WebsocketError("Response channel dropped".to_string()))?
    }
}

pub fn spawn_request_port<C, F, Fut>(command_buffer_capacity: usize, run: F) -> RequestPort<C>
where
    C: Send + 'static,
    F: FnOnce(mpsc::Receiver<C>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel(command_buffer_capacity);
    tokio::spawn(run(receiver));

    RequestPort::new(sender)
}
