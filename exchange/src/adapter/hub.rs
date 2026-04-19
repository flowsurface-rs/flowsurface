pub mod binance;
pub mod bybit;
pub mod hyperliquid;
pub mod mexc;
pub mod okex;

use crate::adapter::AdapterError;
use crate::adapter::limiter::RateLimiter;
use crate::depth::DepthPayload;
use crate::{Kline, OpenInterest, Ticker, TickerInfo, TickerStats, Timeframe, Trade};

use futures::future::BoxFuture;
use reqwest::{Client, Method, Response, header};
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc, oneshot};

use std::{collections::HashMap, path::PathBuf, time::Duration};

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type ResponseTx<T> = oneshot::Sender<Result<T, AdapterError>>;

type TickerMetadataMap = HashMap<Ticker, Option<TickerInfo>>;
type TickerStatsMap = HashMap<Ticker, TickerStats>;

enum FetchCommand<M> {
    TickerMetadata {
        market_scope: M,
        reply: ResponseTx<TickerMetadataMap>,
    },
    TickerStats {
        market_scope: M,
        reply: ResponseTx<TickerStatsMap>,
    },
    Klines {
        ticker: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
        reply: ResponseTx<Vec<Kline>>,
    },
    OpenInterest {
        ticker: TickerInfo,
        timeframe: Timeframe,
        range: Option<(u64, u64)>,
        reply: ResponseTx<Vec<OpenInterest>>,
    },
    DepthSnapshot {
        ticker: Ticker,
        reply: ResponseTx<DepthPayload>,
    },
    Trades {
        ticker: TickerInfo,
        from_time: u64,
        data_path: Option<PathBuf>,
        reply: ResponseTx<Vec<Trade>>,
    },
}

pub struct HttpHub<L> {
    client: Client,
    limiter: L,
}

impl<L: RateLimiter> HttpHub<L> {
    fn new(limiter: L, proxy_cfg: Option<super::Proxy>) -> Result<Self, AdapterError> {
        Self::with_config(limiter, proxy_cfg)
    }

    fn with_config(limiter: L, proxy_cfg: Option<super::Proxy>) -> Result<Self, AdapterError> {
        let builder = Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT);

        let builder = super::proxy::try_apply_proxy(builder, proxy_cfg.as_ref());

        let client = builder.build().map_err(|error| {
            AdapterError::InvalidRequest(format!("Failed to build worker HTTP client: {error}"))
        })?;

        Ok(Self { client, limiter })
    }

    fn client(&self) -> &Client {
        &self.client
    }

    fn limiter_mut(&mut self) -> &mut L {
        &mut self.limiter
    }

    /// Lowest-level HTTP layer.
    ///
    /// Applies limiter pre-wait, performs the request, updates limiter state from
    /// the response, and returns the raw response for callers that need custom
    /// decoding/parsing logic.
    async fn http_response_with_limiter(
        &mut self,
        url: &str,
        weight: usize,
        method: Method,
        json_body: Option<&serde_json::Value>,
    ) -> Result<Response, AdapterError> {
        let request_method = method.clone();

        {
            let limiter = self.limiter_mut();
            if let Some(wait_time) = limiter.prepare_request(weight) {
                log::warn!("Rate limit hit for: {url}. Waiting for {:?}", wait_time);
                tokio::time::sleep(wait_time).await;
            }
        }

        let response = Self::send_request_client(self.client(), method, url, json_body)
            .await
            .map_err(|error| AdapterError::request_failed(&request_method, url, error))?;

        {
            let limiter = self.limiter_mut();
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

        Ok(response)
    }

    /// Text-response layer.
    ///
    /// Builds on `http_response_with_limiter`, decodes the response body into UTF-8
    /// text, and emits enriched HTTP/status diagnostics on failure.
    async fn http_text_with_limiter(
        &mut self,
        url: &str,
        weight: usize,
        method: Option<Method>,
        json_body: Option<&serde_json::Value>,
    ) -> Result<String, AdapterError> {
        let method = method.unwrap_or(Method::GET);

        let response = self
            .http_response_with_limiter(url, weight, method.clone(), json_body)
            .await?;

        Self::read_response_body(&method, url, response).await
    }

    /// JSON layer.
    ///
    /// Builds on `http_text_with_limiter`, validates response shape, and
    /// deserializes JSON into the target type with parse diagnostics.
    async fn http_json_with_limiter<V>(
        &mut self,
        url: &str,
        weight: usize,
        method: Option<Method>,
        json_body: Option<&serde_json::Value>,
    ) -> Result<V, AdapterError>
    where
        V: DeserializeOwned,
    {
        let body = self
            .http_text_with_limiter(url, weight, method, json_body)
            .await?;
        Self::parse_json_body(url, &body)
    }

    async fn send_request_client(
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

    fn parse_json_body<V>(url: &str, body: &str) -> Result<V, AdapterError>
    where
        V: DeserializeOwned,
    {
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
                Self::body_preview(body, 200)
            );
            log::error!("{}", msg);
            return Err(AdapterError::ParseError(msg));
        }

        serde_json::from_str(body).map_err(|error| {
            let msg = format!(
                "JSON parse failed: {} | url={} | response_len={} | preview={:?}",
                error,
                url,
                body.len(),
                Self::body_preview(body, 200)
            );
            log::error!("{}", msg);
            AdapterError::ParseError(msg)
        })
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
                Self::body_preview(&body_text, 200)
            );
            log::error!("{}", msg);
            return Err(AdapterError::http_status_failed(status, msg));
        }

        Ok(body_text)
    }
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

    fn fetch_depth_snapshot(
        &mut self,
        ticker: Ticker,
    ) -> BoxFuture<'_, Result<DepthPayload, AdapterError>> {
        let _ = ticker;
        Box::pin(async { Err(unsupported_fetch("Depth snapshot fetch")) })
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

fn spawn_fetch_worker<H, M>(
    command_buffer_capacity: usize,
    mut worker: H,
) -> RequestPort<FetchCommand<M>>
where
    H: FetchCommandHandler<M> + Send + 'static,
    M: Send + 'static,
{
    let (sender, mut receiver) = mpsc::channel(command_buffer_capacity);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            handle_fetch_command(&mut worker, command).await;
        }
    });
    RequestPort::new(sender)
}

fn unsupported_fetch(feature: &'static str) -> AdapterError {
    AdapterError::InvalidRequest(format!("{feature} is not supported by this worker"))
}

async fn handle_fetch_command<H, M>(handler: &mut H, command: FetchCommand<M>)
where
    H: FetchCommandHandler<M>,
{
    match command {
        FetchCommand::TickerMetadata {
            market_scope,
            reply,
        } => {
            let result = handler.fetch_ticker_metadata(market_scope).await;
            let _ = reply.send(result);
        }
        FetchCommand::TickerStats {
            market_scope,
            reply,
        } => {
            let result = handler.fetch_ticker_stats(market_scope).await;
            let _ = reply.send(result);
        }
        FetchCommand::Klines {
            ticker,
            timeframe,
            range,
            reply,
        } => {
            let result = handler.fetch_klines(ticker, timeframe, range).await;
            let _ = reply.send(result);
        }
        FetchCommand::OpenInterest {
            ticker,
            timeframe,
            range,
            reply,
        } => {
            let result = handler.fetch_open_interest(ticker, timeframe, range).await;
            let _ = reply.send(result);
        }
        FetchCommand::DepthSnapshot { ticker, reply } => {
            let result = handler.fetch_depth_snapshot(ticker).await;
            let _ = reply.send(result);
        }
        FetchCommand::Trades {
            ticker,
            from_time,
            data_path,
            reply,
        } => {
            let result = handler.fetch_trades(ticker, from_time, data_path).await;
            let _ = reply.send(result);
        }
    }
}

struct RequestPort<C> {
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
    fn new(sender: mpsc::Sender<C>) -> Self {
        Self { sender }
    }

    async fn request<T>(&self, build: impl FnOnce(ResponseTx<T>) -> C) -> Result<T, AdapterError> {
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
