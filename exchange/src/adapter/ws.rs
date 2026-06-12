use crate::adapter::{AdapterError, Event, StreamKind};
use crate::proxy::{Proxy, ProxyStream};
use crate::unit::qty::QtyNormalization;
use crate::{Ticker, TickerInfo, Trade, UnixMs};

use bytes::Bytes;
use fastwebsockets::{FragmentCollector, Frame, OpCode, Payload, WebSocketError};
use futures::SinkExt;
use http_body_util::Empty;
use hyper::{
    Request,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustc_hash::FxHashMap;
use tokio::time::{Instant, Interval};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, OwnedTrustAnchor},
};
use url::Url;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

const HEARTBEAT_SEND_FAILED_REASON: &str = "Failed to send heartbeat ping";
const HEARTBEAT_PONG_FAILED_REASON: &str = "Failed to reply pong";

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) static TLS_CONNECTOR: LazyLock<TlsConnector> = LazyLock::new(|| {
    let mut root_store = tokio_rustls::rustls::RootCertStore::empty();

    root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(
            ta.subject,
            ta.spki,
            ta.name_constraints,
        )
    }));

    let config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
});

type Receiver<T> = futures::channel::mpsc::Receiver<T>;
type Sender<T> = futures::channel::mpsc::Sender<T>;

pub(super) struct ChannelStream<T> {
    receiver: Receiver<T>,
    task: tokio::task::JoinHandle<()>,
}

impl<T> futures::Stream for ChannelStream<T> {
    type Item = T;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl<T> Drop for ChannelStream<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Debug)]
pub(super) struct WsSession {
    ping_payload: PingPayload,
    streams: Arc<[StreamKind]>,
}

pub(super) trait WsAdapter {
    /// Connects to the WebSocket and returns a transport for it.
    /// This will be retried indefinitely until it succeeds, with an exponential backoff
    /// between attempts (base ~500ms, doubling, capped at 30s, with jitter).
    fn connect(&mut self) -> impl std::future::Future<Output = Result<WsTransport, String>> + Send;

    /// Called when a connection is established.
    /// This is called on every successful connection, including after reconnects.
    fn on_connected(
        &mut self,
        output: &mut Sender<Event>,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Called when a text message is received.
    fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut Sender<Event>,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;

    /// Called when the connection is closed or a fatal error occurs.
    fn on_disconnected(
        &mut self,
        reason: &str,
        output: &mut Sender<Event>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

impl WsSession {
    pub(super) fn with_text_ping(ping_payload: &'static [u8], streams: Arc<[StreamKind]>) -> Self {
        Self {
            ping_payload: PingPayload::Text(ping_payload),
            streams,
        }
    }

    pub(super) fn with_opcode_ping(
        ping_payload: &'static [u8],
        streams: Arc<[StreamKind]>,
    ) -> Self {
        Self {
            ping_payload: PingPayload::OpCode(ping_payload),
            streams,
        }
    }

    pub(super) fn run<A: WsAdapter + Send + 'static>(self, mut adapter: A) -> ChannelStream<Event> {
        let (mut output, receiver) = futures::channel::mpsc::channel(100);

        let ping_payload = self.ping_payload;
        let streams = Arc::clone(&self.streams);

        let task = tokio::spawn(async move {
            if streams.is_empty() {
                emit_disconnected(&mut output, &streams, "Empty stream payload".to_string()).await;
            } else {
                let mut state = State::Disconnected;
                let mut heartbeat = WsHeartbeat::default();
                let mut backoff = ReconnectBackoff::new();

                loop {
                    match &mut state {
                        State::Disconnected => match adapter.connect().await {
                            Ok(websocket) => {
                                state = State::Connected(websocket);
                                heartbeat.reset();
                                backoff.reset();

                                adapter.on_connected(&mut output).await;
                                emit_connected(&mut output, &streams).await;
                            }
                            Err(reason) => {
                                emit_disconnected(&mut output, &streams, reason).await;
                                tokio::time::sleep(backoff.delay()).await;
                                backoff.record_failure();
                            }
                        },
                        State::Connected(websocket) => {
                            let disconnect_reason = tokio::select! {
                                _ = heartbeat.interval_mut().tick() => {
                                    if heartbeat.timed_out() {
                                        Some("Heartbeat timeout (no websocket activity)".to_string())
                                    } else if websocket
                                        .send_heartbeat_ping(ping_payload)
                                        .await
                                        .is_err()
                                    {
                                        Some(HEARTBEAT_SEND_FAILED_REASON.to_string())
                                    } else {
                                        None
                                    }
                                }
                                frame = websocket.read_frame() => match frame {
                                    Ok(msg) => {
                                        heartbeat.mark_activity();

                                        match msg.opcode {
                                            OpCode::Text => {
                                                adapter.on_text(&msg.payload[..], &mut output).await.err()
                                            }
                                            OpCode::Ping => {
                                                let payload = Vec::from(msg.payload);
                                                if websocket.reply_pong(Payload::Owned(payload)).await.is_err() {
                                                    Some(HEARTBEAT_PONG_FAILED_REASON.to_string())
                                                } else {
                                                    None
                                                }
                                            }
                                            OpCode::Close => Some("Connection closed".to_string()),
                                            _ => None,
                                        }
                                    }
                                    Err(e) => Some(format!("Error reading frame: {e}")),
                                }
                            };

                            if let Some(reason) = disconnect_reason {
                                adapter.on_disconnected(&reason, &mut output).await;
                                state = State::Disconnected;
                                emit_disconnected(&mut output, &streams, reason).await;
                            }
                        }
                    }
                }
            }
        });

        ChannelStream { receiver, task }
    }
}

pub(super) struct WsTransport(FragmentCollector<TokioIo<Upgraded>>);

impl WsTransport {
    pub(super) async fn establish(
        domain: &str,
        url: &str,
        proxy_cfg: Option<&Proxy>,
    ) -> Result<Self, AdapterError> {
        let parsed = Url::parse(url).map_err(|e| AdapterError::InvalidRequest(e.to_string()))?;

        let url_host = parsed.host_str().ok_or_else(|| {
            AdapterError::InvalidRequest("Missing host in websocket URL".to_string())
        })?;

        if !url_host.eq_ignore_ascii_case(domain) {
            return Err(AdapterError::InvalidRequest(format!(
                "WebSocket URL host mismatch: url_host={url_host}, domain_arg={domain}"
            )));
        }

        let target_port = parsed.port_or_known_default().ok_or_else(|| {
            AdapterError::InvalidRequest("Missing port for websocket URL".to_string())
        })?;

        let tcp_stream = ProxyStream::connect_tcp(domain, target_port, proxy_cfg).await?;

        match parsed.scheme() {
            "wss" => {
                let tls_stream = tokio::time::timeout(
                    TLS_HANDSHAKE_TIMEOUT,
                    Self::upgrade_to_tls(tcp_stream, domain),
                )
                .await
                .map_err(|_| {
                    AdapterError::WebsocketError("TLS handshake to target timed out".to_string())
                })??;

                tokio::time::timeout(
                    WS_HANDSHAKE_TIMEOUT,
                    Self::handshake_tls(tls_stream, domain, &parsed),
                )
                .await
                .map_err(|_| {
                    AdapterError::WebsocketError("WebSocket handshake timed out".to_string())
                })?
            }
            "ws" => tokio::time::timeout(
                WS_HANDSHAKE_TIMEOUT,
                Self::handshake_tcp(tcp_stream, domain, &parsed),
            )
            .await
            .map_err(|_| {
                AdapterError::WebsocketError("WebSocket handshake timed out".to_string())
            })?,
            _ => Err(AdapterError::InvalidRequest(
                "Invalid scheme for websocket URL".to_string(),
            )),
        }
    }

    async fn read_frame(&mut self) -> Result<Frame<'_>, WebSocketError> {
        self.0.read_frame().await
    }

    pub(super) async fn write_frame(&mut self, frame: Frame<'_>) -> Result<(), WebSocketError> {
        self.0.write_frame(frame).await
    }

    async fn reply_pong(&mut self, payload: Payload<'_>) -> Result<(), &'static str> {
        self.write_frame(Frame::pong(payload))
            .await
            .map_err(|_| HEARTBEAT_PONG_FAILED_REASON)
    }

    async fn send_heartbeat_ping(&mut self, ping_payload: PingPayload) -> Result<(), &'static str> {
        let frame = match ping_payload {
            PingPayload::Text(payload) => Frame::text(Payload::Borrowed(payload)),
            PingPayload::OpCode(payload) => {
                Frame::new(true, OpCode::Ping, None, Payload::Borrowed(payload))
            }
        };

        self.write_frame(frame)
            .await
            .map_err(|_| HEARTBEAT_SEND_FAILED_REASON)
    }

    async fn upgrade_to_tls(
        stream: ProxyStream,
        domain: &str,
    ) -> Result<Box<tokio_rustls::client::TlsStream<ProxyStream>>, AdapterError> {
        let server_name = tokio_rustls::rustls::ServerName::try_from(domain)
            .map_err(|_| AdapterError::ParseError("invalid dnsname".to_string()))?;

        let tls_stream = TLS_CONNECTOR
            .connect(server_name, stream)
            .await
            .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

        Ok(Box::new(tls_stream))
    }

    async fn handshake_tcp(
        stream: ProxyStream,
        domain: &str,
        parsed: &Url,
    ) -> Result<Self, AdapterError> {
        let req = Self::build_ws_request(domain, parsed)?;
        let exec = TokioExecutor::new();
        let (ws, _http_resp) = fastwebsockets::handshake::client(&exec, req, stream)
            .await
            .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;
        Ok(Self(FragmentCollector::new(ws)))
    }

    async fn handshake_tls(
        tls: Box<tokio_rustls::client::TlsStream<ProxyStream>>,
        domain: &str,
        parsed: &Url,
    ) -> Result<Self, AdapterError> {
        let req = Self::build_ws_request(domain, parsed)?;
        let exec = TokioExecutor::new();
        let (ws, _http_resp) = fastwebsockets::handshake::client(&exec, req, tls)
            .await
            .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;
        Ok(Self(FragmentCollector::new(ws)))
    }

    fn build_ws_request(domain: &str, parsed: &Url) -> Result<Request<Empty<Bytes>>, AdapterError> {
        let mut path_and_query = parsed.path().to_string();
        if let Some(q) = parsed.query() {
            path_and_query.push('?');
            path_and_query.push_str(q);
        }
        if path_and_query.is_empty() {
            path_and_query.push('/');
        }

        let host_header = match parsed.port() {
            Some(explicit_port) => {
                let default_port = parsed.port_or_known_default().unwrap_or(explicit_port);
                if explicit_port != default_port {
                    format!("{domain}:{explicit_port}")
                } else {
                    domain.to_string()
                }
            }
            None => domain.to_string(),
        };

        Request::builder()
            .method("GET")
            .uri(path_and_query)
            .header("Host", host_header)
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "upgrade")
            .header(
                "Sec-WebSocket-Key",
                fastwebsockets::handshake::generate_key(),
            )
            .header("Sec-WebSocket-Version", "13")
            .body(Empty::<Bytes>::new())
            .map_err(|e| AdapterError::WebsocketError(e.to_string()))
    }
}

enum State {
    Disconnected,
    Connected(WsTransport),
}

#[derive(Clone, Copy, Debug)]
enum PingPayload {
    Text(&'static [u8]),
    OpCode(&'static [u8]),
}

struct WsHeartbeat {
    interval: Duration,
    timeout: Duration,
    heartbeat_interval: Interval,
    last_transport_activity: Instant,
}

impl WsHeartbeat {
    const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
    const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

    fn new(interval: Duration, timeout: Duration) -> Self {
        Self {
            interval,
            timeout,
            heartbeat_interval: tokio::time::interval(interval),
            last_transport_activity: Instant::now(),
        }
    }

    fn reset(&mut self) {
        self.last_transport_activity = Instant::now();
        self.heartbeat_interval = tokio::time::interval(self.interval);
    }

    fn interval_mut(&mut self) -> &mut Interval {
        &mut self.heartbeat_interval
    }

    fn mark_activity(&mut self) {
        self.last_transport_activity = Instant::now();
    }

    fn timed_out(&self) -> bool {
        self.last_transport_activity.elapsed() >= self.timeout
    }
}

impl Default for WsHeartbeat {
    fn default() -> Self {
        Self::new(
            Self::DEFAULT_HEARTBEAT_INTERVAL,
            Self::DEFAULT_HEARTBEAT_TIMEOUT,
        )
    }
}

/// Exponential backoff for WebSocket reconnection attempts.
///
/// Starts at ~500ms, doubles each failure, caps at 30s, and adds ±25% jitter
/// to spread reconnections across streams when multiple disconnect at once.
struct ReconnectBackoff {
    current: Duration,
    max_delay: Duration,
    multiplier: f32,
    jitter: f32,
}

impl ReconnectBackoff {
    const INITIAL: Duration = Duration::from_millis(500);
    const MAX: Duration = Duration::from_secs(30);

    fn new() -> Self {
        Self {
            current: Self::INITIAL,
            max_delay: Self::MAX,
            multiplier: 2.0,
            jitter: 0.25,
        }
    }

    /// Returns the delay before the next reconnect attempt, with ±jitter applied.
    fn delay(&self) -> Duration {
        let jitter_range = self.current.mul_f32(self.jitter);
        let jitter = Duration::from_secs_f32(
            (rand::random::<f32>() * 2.0 - 1.0) * jitter_range.as_secs_f32(),
        );
        (self.current + jitter).clamp(Duration::ZERO, self.max_delay)
    }

    /// Advances the backoff after a failed attempt (multiplies delay, capped at max).
    fn record_failure(&mut self) {
        self.current = (self.current.mul_f32(self.multiplier)).min(self.max_delay);
    }

    /// Resets back to the initial delay after a successful connection.
    fn reset(&mut self) {
        self.current = Self::INITIAL;
    }
}

async fn emit_connected(output: &mut Sender<Event>, streams: &Arc<[StreamKind]>) {
    let _ = output.send(Event::Connected(streams.clone())).await;
}

async fn emit_disconnected(
    output: &mut Sender<Event>,
    streams: &Arc<[StreamKind]>,
    reason: impl Into<String>,
) {
    let _ = output
        .send(Event::Disconnected(streams.clone(), reason.into()))
        .await;
}

pub(super) struct TradeBuffer {
    buffer_map: FxHashMap<Ticker, Vec<Trade>>,
    ticker_info_map: FxHashMap<Ticker, (TickerInfo, QtyNormalization)>,
    last_flush: Instant,
}

impl TradeBuffer {
    /// Buffer trades and flush in this interval
    const TRADE_BUCKET_INTERVAL: Duration = Duration::from_micros(33_333);

    pub(super) fn new(ticker_info_map: FxHashMap<Ticker, (TickerInfo, QtyNormalization)>) -> Self {
        Self {
            buffer_map: FxHashMap::default(),
            ticker_info_map,
            last_flush: Instant::now(),
        }
    }

    pub(super) fn ticker_info(&self, ticker: &Ticker) -> Option<&(TickerInfo, QtyNormalization)> {
        self.ticker_info_map.get(ticker)
    }

    pub(super) fn push(&mut self, ticker: Ticker, trade: Trade) {
        self.buffer_map.entry(ticker).or_default().push(trade);
    }

    pub(super) async fn flush_if_ready(&mut self, output: &mut Sender<Event>) {
        if self.last_flush.elapsed() >= Self::TRADE_BUCKET_INTERVAL {
            self.flush(output).await;
        }
    }

    pub(super) async fn flush(&mut self, output: &mut Sender<Event>) {
        let interval_ms = Self::TRADE_BUCKET_INTERVAL.as_millis() as u64;

        for (ticker, trades_buffer) in self.buffer_map.iter_mut() {
            if trades_buffer.is_empty() {
                continue;
            }

            let bucket_update_t = trades_buffer
                .iter()
                .map(|t| t.time.as_u64())
                .max()
                .map(|t| UnixMs::new((t / interval_ms) * interval_ms));

            if let Some((ticker_info, _)) = self.ticker_info_map.get(ticker)
                && let Some(update_t) = bucket_update_t
            {
                let _ = output
                    .send(Event::TradesReceived(
                        StreamKind::Trades {
                            ticker_info: *ticker_info,
                        },
                        update_t,
                        std::mem::take(trades_buffer).into_boxed_slice(),
                    ))
                    .await;
            }
        }

        self.last_flush = Instant::now();
    }
}
