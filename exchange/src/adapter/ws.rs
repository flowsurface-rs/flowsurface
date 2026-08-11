use crate::adapter::{AdapterError, Event, StreamKind};
use crate::proxy::{Proxy, ProxyStream};
use crate::unit::qty::QtyNormalization;
use crate::{Ticker, TickerInfo, Trade, UnixMs};

use bytes::Bytes;
use fastwebsockets::{FragmentCollector, Frame, OpCode, Payload, WebSocketError};
use http_body_util::Empty;
use hyper::{
    Request,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustc_hash::FxHashMap;
use tokio::time::Instant;
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, OwnedTrustAnchor},
};
use url::Url;

use futures::StreamExt;
#[cfg(not(feature = "unbounded-channel"))]
use futures::channel::mpsc::Sender;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

const HEARTBEAT_SEND_FAILED_REASON: &str = "Failed to send heartbeat ping";
const HEARTBEAT_PONG_FAILED_REASON: &str = "Failed to reply pong";

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

const MAX_DRAIN_PER_TICK: usize = 256;

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

enum AnySender<T> {
    #[cfg(not(feature = "unbounded-channel"))]
    Bounded(Sender<T>),
    Unbounded(UnboundedSender<T>),
}

impl<T> AnySender<T> {
    fn send(&mut self, item: T) -> Result<(), futures::channel::mpsc::TrySendError<T>> {
        match self {
            #[cfg(not(feature = "unbounded-channel"))]
            AnySender::Bounded(tx) => tx.try_send(item),
            AnySender::Unbounded(tx) => tx.unbounded_send(item),
        }
    }
}

enum AnyReceiver<T> {
    #[cfg(not(feature = "unbounded-channel"))]
    Bounded(futures::channel::mpsc::Receiver<T>),
    Unbounded(UnboundedReceiver<T>),
}

impl<T> futures::Stream for AnyReceiver<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.get_mut() {
            #[cfg(not(feature = "unbounded-channel"))]
            AnyReceiver::Bounded(rx) => std::pin::Pin::new(rx).poll_next(cx),
            AnyReceiver::Unbounded(rx) => std::pin::Pin::new(rx).poll_next(cx),
        }
    }
}

impl<T> AnyReceiver<T> {
    fn try_recv(&mut self) -> Option<T> {
        match self {
            #[cfg(not(feature = "unbounded-channel"))]
            AnyReceiver::Bounded(rx) => rx.try_recv().ok(),
            AnyReceiver::Unbounded(rx) => rx.try_recv().ok(),
        }
    }
}

fn channel<T>(_capacity: usize) -> (AnySender<T>, AnyReceiver<T>) {
    #[cfg(not(feature = "unbounded-channel"))]
    {
        let (tx, rx) = futures::channel::mpsc::channel(_capacity);
        (AnySender::Bounded(tx), AnyReceiver::Bounded(rx))
    }
    #[cfg(feature = "unbounded-channel")]
    {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        (AnySender::Unbounded(tx), AnyReceiver::Unbounded(rx))
    }
}

pub(super) struct ChannelStream<T> {
    receiver: AnyReceiver<T>,
    task: tokio::task::JoinHandle<()>,
}

impl<T> futures::Stream for ChannelStream<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.get_mut().receiver).poll_next(cx)
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

/// How often [`on_tick`](WsAdapter::on_tick) fires. Also serves as the time-bucket
/// granularity for trade aggregation — trades within one interval are collapsed
/// into a single [`Event::TradesReceived`].
const ADAPTER_TICK_INTERVAL: Duration = Duration::from_micros(33_333);

pub(super) trait WsAdapter {
    /// Connects to the WebSocket and returns a transport for it.
    /// This will be retried indefinitely until it succeeds, with an exponential backoff
    /// between attempts (base ~500ms, doubling, capped at 30s, with jitter).
    fn connect(&mut self) -> impl std::future::Future<Output = Result<WsTransport, String>> + Send;

    /// Tick interval controlling how often [`on_tick`](WsAdapter::on_tick) is called
    /// and the time-bucket granularity for trade aggregation.
    fn tick_interval(&self) -> Duration {
        ADAPTER_TICK_INTERVAL
    }

    /// Called when a connection is established.
    /// This is called on every successful connection, including after reconnects.
    fn on_connected(&mut self) -> impl std::future::Future<Output = Vec<Event>> + Send;

    /// Called periodically while connected, at the cadence returned by
    /// [`tick_interval`].
    ///
    /// This is where **trade adapters** flush their [`TradeBuffer`] — trades are
    /// batched across one tick interval and emerge as a single
    /// [`Event::TradesReceived`]. Non-trade adapters leave this as the default
    /// no-op because they push events directly in [`on_text`](Self::on_text).
    fn on_tick(&mut self) -> impl std::future::Future<Output = Vec<Event>> + Send {
        async { Vec::new() }
    }

    /// Called when a text message is received.
    ///
    /// Adapters parse incoming data and return resulting `Event`s.
    /// The session loop sends them to the output channel.
    /// If the output channel is full, events are silently dropped
    ///
    /// **Flush model**: non-trade adapters return events here directly.
    /// Trade adapters only buffer into [`TradeBuffer`] here and return
    /// events later in [`on_tick`](Self::on_tick).
    fn on_text(
        &mut self,
        payload: &[u8],
    ) -> impl std::future::Future<Output = Result<Vec<Event>, String>> + Send;

    /// Called when the connection is closed or a fatal error occurs.
    fn on_disconnected(
        &mut self,
        reason: &str,
    ) -> impl std::future::Future<Output = Vec<Event>> + Send;
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
        let (mut event_tx, event_rx) = channel(512);

        let ping_payload = self.ping_payload;
        let streams = Arc::clone(&self.streams);

        let task = tokio::spawn(async move {
            if streams.is_empty() {
                let _ = event_tx.send(Event::Disconnected(
                    streams,
                    "Empty stream payload".to_string(),
                ));
                return;
            }

            let mut backoff = ReconnectBackoff::new();

            loop {
                let transport = match adapter.connect().await {
                    Ok(t) => t,
                    Err(reason) => {
                        let _ = event_tx.send(Event::Disconnected(Arc::clone(&streams), reason));
                        tokio::time::sleep(backoff.delay()).await;
                        backoff.record_failure();
                        continue;
                    }
                };

                let (frame_tx, mut frame_rx) = {
                    let (tx, rx) = futures::channel::mpsc::unbounded();
                    (AnySender::Unbounded(tx), AnyReceiver::Unbounded(rx))
                };
                let io_handle = tokio::spawn(transport.read_frame(ping_payload, frame_tx));

                for event in adapter.on_connected().await {
                    let _ = event_tx.send(event);
                }
                let _ = event_tx.send(Event::Connected(Arc::clone(&streams)));

                let tick_interval = adapter.tick_interval();
                let tick_sleep = tokio::time::sleep(tick_interval);
                tokio::pin!(tick_sleep);

                let disconnect_reason = loop {
                    tokio::select! {
                        biased;
                        frame = frame_rx.next() => {
                            let mut disconnect_reason: Option<String> = None;

                            match frame {
                                Some(Ok(payload)) => {
                                    backoff.record_success();

                                    if !payload.is_empty() {
                                        match adapter.on_text(&payload).await {
                                            Ok(events) => {
                                                for event in events {
                                                    let _ = event_tx.send(event);
                                                }
                                            }
                                            Err(reason) => {
                                                disconnect_reason = Some(reason);
                                            }
                                        }
                                    }

                                    if disconnect_reason.is_none() {
                                       let mut drained = 0;
                                        while drained < MAX_DRAIN_PER_TICK {
                                            let Some(drain) = frame_rx.try_recv() else { break };
                                            drained += 1;
                                            match drain {
                                                Ok(payload) => {
                                                    match adapter.on_text(&payload).await {
                                                        Ok(events) => {
                                                            for event in events {
                                                                let _ = event_tx.send(event);
                                                            }
                                                        }
                                                        Err(reason) => {
                                                            disconnect_reason = Some(reason);
                                                            break;
                                                        }
                                                    }
                                                }
                                                Err(reason) => {
                                                    disconnect_reason = Some(reason);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Err(reason)) => {
                                    disconnect_reason = Some(reason);
                                }
                                None => {
                                    disconnect_reason = Some(
                                        "I/O task exited".to_string(),
                                    );
                                }
                            }

                            if let Some(reason) = disconnect_reason {
                                break Some(reason);
                            }
                        }
                        _ = &mut tick_sleep => {
                            for event in adapter.on_tick().await {
                                let _ = event_tx.send(event);
                            }
                            tick_sleep
                                .as_mut()
                                .reset(tokio::time::Instant::now() + tick_interval);
                        }
                    }
                };

                if io_handle.is_finished() {
                    if let Err(e) = io_handle.await {
                        log::error!("WebSocket I/O task panicked (reconnecting): {e}");
                    }
                } else {
                    io_handle.abort();
                }

                if let Some(reason) = disconnect_reason {
                    for event in adapter.on_disconnected(&reason).await {
                        let _ = event_tx.send(event);
                    }
                    let _ = event_tx.send(Event::Disconnected(Arc::clone(&streams), reason));
                }

                tokio::time::sleep(backoff.delay()).await;
                backoff.record_failure();
            }
        });

        ChannelStream {
            receiver: event_rx,
            task,
        }
    }
}

pub(super) struct WsTransport(FragmentCollector<TokioIo<Upgraded>>);

impl WsTransport {
    /// Reads frames, handles heartbeat and Ping/Pong at transport level,
    /// forwards text frames to the processor task.
    async fn read_frame(
        mut self,
        ping_payload: PingPayload,
        mut frame_tx: AnySender<Result<Vec<u8>, String>>,
    ) {
        let mut heartbeat = WsHeartbeat::default();

        loop {
            let read_timeout = heartbeat
                .time_until_next_ping()
                .max(Duration::from_millis(1));

            match tokio::time::timeout(read_timeout, self.0.read_frame()).await {
                Ok(Ok(msg)) => {
                    heartbeat.mark_activity();

                    match msg.opcode {
                        OpCode::Text => {
                            let payload = Vec::from(&msg.payload[..]);
                            if frame_tx.send(Ok(payload)).is_err() {
                                break;
                            }
                        }
                        OpCode::Ping => {
                            let payload = Vec::from(msg.payload);
                            let _ = self.reply_pong(Payload::Owned(payload)).await;
                            let _ = frame_tx.send(Ok(Vec::new()));
                        }
                        OpCode::Close => {
                            let _ = frame_tx.send(Err("Connection closed".into()));
                            break;
                        }
                        _ => {}
                    }
                }
                Ok(Err(e)) => {
                    let _ = frame_tx.send(Err(format!("Error reading frame: {e}")));
                    break;
                }
                Err(_elapsed) => {
                    if heartbeat.timed_out() {
                        let _ =
                            frame_tx.send(Err("Heartbeat timeout (no websocket activity)".into()));
                        break;
                    }

                    if heartbeat.should_send_ping() {
                        if self.send_heartbeat_ping(ping_payload).await.is_err() {
                            let _ = frame_tx.send(Err(HEARTBEAT_SEND_FAILED_REASON.into()));
                            break;
                        }
                        heartbeat.record_ping_sent();
                    }
                }
            }
        }
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

#[derive(Clone, Copy, Debug)]
enum PingPayload {
    Text(&'static [u8]),
    OpCode(&'static [u8]),
}

struct WsHeartbeat {
    interval: Duration,
    timeout: Duration,
    last_transport_activity: Instant,
    last_ping_sent: Instant,
}

impl WsHeartbeat {
    const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
    const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

    fn new(interval: Duration, timeout: Duration) -> Self {
        let now = Instant::now();
        Self {
            interval,
            timeout,
            last_transport_activity: now,
            last_ping_sent: now,
        }
    }

    fn mark_activity(&mut self) {
        self.last_transport_activity = Instant::now();
    }

    fn timed_out(&self) -> bool {
        self.last_transport_activity.elapsed() >= self.timeout
    }

    /// Returns true when the ping interval has elapsed since the last ping was sent.
    fn should_send_ping(&self) -> bool {
        self.last_ping_sent.elapsed() >= self.interval
    }

    /// Call after successfully sending a heartbeat ping.
    fn record_ping_sent(&mut self) {
        self.last_ping_sent = Instant::now();
    }

    /// How long until the next ping is due (used as the read_frame timeout so
    /// heartbeats are checked even on idle connections).
    fn time_until_next_ping(&self) -> Duration {
        self.interval.saturating_sub(self.last_ping_sent.elapsed())
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
/// Delay doubles on each failure, resets to the initial 500ms on success.
/// Capped at 30s with ±25% multiplicative jitter to spread reconnections
/// across streams when multiple disconnect at once.
struct ReconnectBackoff {
    current: Duration,
}

impl ReconnectBackoff {
    const INITIAL: Duration = Duration::from_millis(500);
    const MAX: Duration = Duration::from_secs(30);
    const JITTER: f32 = 0.25;

    fn new() -> Self {
        Self {
            current: Self::INITIAL,
        }
    }

    /// Returns the delay before the next reconnect attempt, with ±jitter applied.
    fn delay(&self) -> Duration {
        let factor = 1.0 + (rand::random::<f32>() * 2.0 - 1.0) * Self::JITTER;
        let secs = self.current.as_secs_f32() * factor;
        Duration::from_secs_f32(secs.max(0.0)).min(Self::MAX)
    }

    /// Doubles the delay (capped) after a failed attempt.
    fn record_failure(&mut self) {
        self.current = (self.current.mul_f32(2.0)).min(Self::MAX);
    }

    /// Resets the delay to the initial value after genuine success
    /// (real market-data events were produced by the connection).
    fn record_success(&mut self) {
        self.current = Self::INITIAL;
    }
}

pub(super) struct TradeBuffer {
    buffer_map: FxHashMap<Ticker, Vec<Trade>>,
    ticker_info_map: FxHashMap<Ticker, (TickerInfo, QtyNormalization)>,
}

impl TradeBuffer {
    pub(super) fn new(ticker_info_map: FxHashMap<Ticker, (TickerInfo, QtyNormalization)>) -> Self {
        Self {
            buffer_map: FxHashMap::default(),
            ticker_info_map,
        }
    }

    pub(super) fn ticker_info(&self, ticker: &Ticker) -> Option<&(TickerInfo, QtyNormalization)> {
        self.ticker_info_map.get(ticker)
    }

    pub(super) fn push(&mut self, ticker: Ticker, trade: Trade) {
        self.buffer_map.entry(ticker).or_default().push(trade);
    }

    /// Drain all buffered trades, clearing internal buffers.
    ///
    /// Each ticker's trades are collapsed into a single [`Event::TradesReceived`]
    /// keyed by the most recent trade's time rounded down to the nearest
    /// [`ADAPTER_TICK_INTERVAL`] bucket.
    pub(super) fn flush(&mut self) -> Vec<Event> {
        let interval_ms = ADAPTER_TICK_INTERVAL.as_millis() as u64;
        let mut events = Vec::new();

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
                events.push(Event::TradesReceived(
                    StreamKind::Trades {
                        ticker_info: *ticker_info,
                    },
                    update_t,
                    std::mem::take(trades_buffer).into_boxed_slice(),
                ));
            }
        }

        events
    }
}
