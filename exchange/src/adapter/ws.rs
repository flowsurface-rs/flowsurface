use crate::adapter::{AdapterError, Event, StreamKind};
use crate::proxy::{Proxy, ProxyStream};
use crate::unit::qty::QtyNormalization;
use crate::{Ticker, TickerInfo, Trade, UnixMs};

use bytes::Bytes;
use fastwebsockets::{
    FragmentCollectorRead, Frame, OpCode, Payload, WebSocket, WebSocketError, WebSocketWrite,
};
use http_body_util::Empty;
use hyper::{
    Request,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustc_hash::FxHashMap;
use tokio::io::{AsyncRead, AsyncWrite};
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
const HEARTBEAT_TIMEOUT_REASON: &str = "Heartbeat timeout (no websocket activity)";

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
    streams: Arc<[StreamKind]>,
}

/// How often [`on_tick`](WsAdapter::on_tick) fires. Also serves as the time-bucket
/// granularity for trade aggregation — trades within one interval are collapsed
/// into a single [`Event::TradesReceived`].
const ADAPTER_TICK_INTERVAL: Duration = Duration::from_micros(33_333);

pub(super) trait WsAdapter {
    /// Selects the transport heartbeat policy for this adapter.
    ///
    /// The session reads this policy after each successful connection. The
    /// policy owns all venue-specific timing and wire-format decisions, while
    /// the shared transport handles frame activity and control replies.
    fn heartbeat_policy(&self) -> HeartbeatPolicy;

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
    pub(super) fn new(streams: Arc<[StreamKind]>) -> Self {
        Self { streams }
    }

    pub(super) fn run<A: WsAdapter + Send + 'static>(self, mut adapter: A) -> ChannelStream<Event> {
        let (mut event_tx, event_rx) = channel(512);

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

                let heartbeat_policy = adapter.heartbeat_policy();

                let (frame_tx, mut frame_rx) = {
                    let (tx, rx) = futures::channel::mpsc::unbounded();
                    (AnySender::Unbounded(tx), AnyReceiver::Unbounded(rx))
                };
                let io_handle = tokio::spawn(transport.read_frame(heartbeat_policy, frame_tx));

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

/// Wire representation for an outbound heartbeat payload.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PingPayload {
    /// An application-level text message.
    Text(&'static [u8]),
    /// A WebSocket control Ping frame.
    OpCode(&'static [u8]),
}

/// Transport-level keepalive strategy for a WebSocket adapter.
///
/// Activity is recorded for every complete inbound frame, including control
/// frames. The policy determines when the connection sends a heartbeat and
/// when a lack of inbound activity is treated as a dead connection.
#[derive(Clone, Copy, Debug)]
pub(super) enum HeartbeatPolicy {
    /// Send an application or control ping at a fixed cadence.
    ///
    /// The next ping is scheduled after the previous write succeeds. The
    /// connection is closed when no complete inbound frame arrives within
    /// `silence_timeout`.
    PeriodicClientPing {
        ping: PingPayload,
        every: Duration,
        silence_timeout: Duration,
    },
    /// Send one application-level ping after inbound traffic has been idle.
    ///
    /// A complete inbound frame returns the policy to idle monitoring. After
    /// a ping is written, the connection waits `response_timeout` for any
    /// complete inbound frame before closing.
    PingAfterIdle {
        ping: PingPayload,
        idle_for: Duration,
        response_timeout: Duration,
    },
    /// Rely on protocol-level pings sent by the server.
    ///
    /// The transport still replies to incoming WebSocket Ping frames, but it
    /// never sends a proactive heartbeat.
    ServerDriven { silence_timeout: Duration },
}

impl HeartbeatPolicy {
    /// Creates a [`PeriodicClientPing`](HeartbeatPolicy::PeriodicClientPing) policy
    /// that sends an application-level text ping.
    pub(super) const fn periodic_text(
        payload: &'static [u8],
        every: Duration,
        silence_timeout: Duration,
    ) -> Self {
        Self::PeriodicClientPing {
            ping: PingPayload::Text(payload),
            every,
            silence_timeout,
        }
    }

    /// Creates a [`PeriodicClientPing`](HeartbeatPolicy::PeriodicClientPing) policy
    /// that sends a WebSocket control Ping frame.
    #[allow(dead_code)]
    pub(crate) const fn periodic_opcode(
        payload: &'static [u8],
        every: Duration,
        silence_timeout: Duration,
    ) -> Self {
        Self::PeriodicClientPing {
            ping: PingPayload::OpCode(payload),
            every,
            silence_timeout,
        }
    }

    /// Creates a [`PingAfterIdle`](HeartbeatPolicy::PingAfterIdle) policy
    /// that sends an application-level text ping.
    pub(super) const fn ping_after_idle_text(
        payload: &'static [u8],
        idle_for: Duration,
        response_timeout: Duration,
    ) -> Self {
        Self::PingAfterIdle {
            ping: PingPayload::Text(payload),
            idle_for,
            response_timeout,
        }
    }

    /// Creates a [`ServerDriven`](HeartbeatPolicy::ServerDriven) policy
    /// that only observes server-driven activity.
    pub(super) const fn server_driven(silence_timeout: Duration) -> Self {
        Self::ServerDriven { silence_timeout }
    }
}

enum HeartbeatState {
    Periodic { next_ping: Instant },
    IdleMonitoring,
    IdleAwaitingResponse { deadline: Instant },
    ServerDriven,
}

enum HeartbeatAction {
    SendPing(PingPayload),
    Timeout,
    Wait,
}

struct WsHeartbeat {
    policy: HeartbeatPolicy,
    last_activity: Instant,
    state: HeartbeatState,
}

impl WsHeartbeat {
    fn new(policy: HeartbeatPolicy, now: Instant) -> Self {
        let state = match policy {
            HeartbeatPolicy::PeriodicClientPing { every, .. } => HeartbeatState::Periodic {
                next_ping: now + every,
            },
            HeartbeatPolicy::PingAfterIdle { .. } => HeartbeatState::IdleMonitoring,
            HeartbeatPolicy::ServerDriven { .. } => HeartbeatState::ServerDriven,
        };

        Self {
            policy,
            last_activity: now,
            state,
        }
    }

    fn next_deadline(&self) -> Instant {
        match (self.policy, &self.state) {
            (
                HeartbeatPolicy::PeriodicClientPing {
                    silence_timeout, ..
                },
                HeartbeatState::Periodic { next_ping },
            ) => (self.last_activity + silence_timeout).min(*next_ping),
            (HeartbeatPolicy::PingAfterIdle { idle_for, .. }, HeartbeatState::IdleMonitoring) => {
                self.last_activity + idle_for
            }
            (
                HeartbeatPolicy::PingAfterIdle { .. },
                HeartbeatState::IdleAwaitingResponse { deadline },
            ) => *deadline,
            (HeartbeatPolicy::ServerDriven { silence_timeout }, HeartbeatState::ServerDriven) => {
                self.last_activity + silence_timeout
            }
            (
                HeartbeatPolicy::PeriodicClientPing {
                    silence_timeout, ..
                },
                _,
            ) => self.last_activity + silence_timeout,
            (HeartbeatPolicy::PingAfterIdle { idle_for, .. }, _) => self.last_activity + idle_for,
            (HeartbeatPolicy::ServerDriven { silence_timeout }, _) => {
                self.last_activity + silence_timeout
            }
        }
    }

    fn deadline_action(&self, now: Instant) -> HeartbeatAction {
        match (self.policy, &self.state) {
            (
                HeartbeatPolicy::PeriodicClientPing {
                    ping,
                    silence_timeout,
                    ..
                },
                HeartbeatState::Periodic { next_ping },
            ) => {
                if now >= self.last_activity + silence_timeout {
                    HeartbeatAction::Timeout
                } else if now >= *next_ping {
                    HeartbeatAction::SendPing(ping)
                } else {
                    HeartbeatAction::Wait
                }
            }
            (
                HeartbeatPolicy::PingAfterIdle { ping, idle_for, .. },
                HeartbeatState::IdleMonitoring,
            ) => {
                if now >= self.last_activity + idle_for {
                    HeartbeatAction::SendPing(ping)
                } else {
                    HeartbeatAction::Wait
                }
            }
            (
                HeartbeatPolicy::PingAfterIdle { .. },
                HeartbeatState::IdleAwaitingResponse { deadline },
            ) => {
                if now >= *deadline {
                    HeartbeatAction::Timeout
                } else {
                    HeartbeatAction::Wait
                }
            }
            (HeartbeatPolicy::ServerDriven { silence_timeout }, HeartbeatState::ServerDriven) => {
                if now >= self.last_activity + silence_timeout {
                    HeartbeatAction::Timeout
                } else {
                    HeartbeatAction::Wait
                }
            }
            _ => HeartbeatAction::Wait,
        }
    }

    fn mark_activity(&mut self, now: Instant) {
        self.last_activity = now;
        if matches!(self.state, HeartbeatState::IdleAwaitingResponse { .. }) {
            self.state = HeartbeatState::IdleMonitoring;
        }
    }

    fn record_ping_sent(&mut self, now: Instant) {
        match (self.policy, &self.state) {
            (
                HeartbeatPolicy::PeriodicClientPing { every, .. },
                HeartbeatState::Periodic { .. },
            ) => {
                self.state = HeartbeatState::Periodic {
                    next_ping: now + every,
                };
            }
            (
                HeartbeatPolicy::PingAfterIdle {
                    response_timeout, ..
                },
                HeartbeatState::IdleMonitoring,
            ) => {
                self.state = HeartbeatState::IdleAwaitingResponse {
                    deadline: now + response_timeout,
                };
            }
            _ => {}
        }
    }
}

struct WsConnection<R, W> {
    reader: FragmentCollectorRead<R>,
    writer: WebSocketWrite<W>,
    heartbeat: WsHeartbeat,
    frame_tx: AnySender<Result<Vec<u8>, String>>,
}

impl<R, W> WsConnection<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    fn new(
        reader: FragmentCollectorRead<R>,
        writer: WebSocketWrite<W>,
        heartbeat_policy: HeartbeatPolicy,
        frame_tx: AnySender<Result<Vec<u8>, String>>,
    ) -> Self {
        Self {
            reader,
            writer,
            heartbeat: WsHeartbeat::new(heartbeat_policy, Instant::now()),
            frame_tx,
        }
    }

    async fn run(self) {
        let Self {
            mut reader,
            mut writer,
            mut heartbeat,
            mut frame_tx,
        } = self;

        loop {
            let mut no_control_send =
                |_frame: Frame<'_>| std::future::ready(Ok::<(), std::io::Error>(()));
            let mut read_future = Box::pin(reader.read_frame(&mut no_control_send));

            let outcome = loop {
                let deadline = heartbeat.next_deadline();

                tokio::select! {
                    biased;
                    result = &mut read_future => break Ok(result),
                    _ = tokio::time::sleep_until(deadline) => {
                        if let Err(reason) = apply_heartbeat_action(&mut heartbeat, &mut writer).await {
                            break Err(reason);
                        }
                    }
                }
            };

            drop(read_future);

            let message = match outcome {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    let _ = frame_tx.send(Err(format!("Error reading frame: {error}")));
                    break;
                }
                Err(reason) => {
                    let _ = frame_tx.send(Err(reason.to_string()));
                    break;
                }
            };

            heartbeat.mark_activity(Instant::now());

            let keep_connection = match message.opcode {
                OpCode::Text => {
                    let payload = Vec::from(&message.payload[..]);
                    frame_tx.send(Ok(payload)).is_ok()
                }
                OpCode::Ping => {
                    let payload = Vec::from(message.payload);
                    if writer
                        .write_frame(Frame::pong(Payload::Owned(payload)))
                        .await
                        .is_err()
                    {
                        let _ = frame_tx.send(Err(HEARTBEAT_PONG_FAILED_REASON.into()));
                        false
                    } else {
                        frame_tx.send(Ok(Vec::new())).is_ok()
                    }
                }
                OpCode::Close => {
                    let payload = Vec::from(message.payload);
                    let close_reason = format_close_frame_reason(&payload);
                    let _ = writer
                        .write_frame(Frame::close_raw(Payload::Owned(payload)))
                        .await;
                    let _ = frame_tx.send(Err(close_reason));
                    false
                }
                _ => true,
            };

            if !keep_connection {
                break;
            }

            if let Err(reason) = apply_heartbeat_action(&mut heartbeat, &mut writer).await {
                let _ = frame_tx.send(Err(reason.to_string()));
                break;
            }
        }
    }
}

async fn apply_heartbeat_action<W>(
    heartbeat: &mut WsHeartbeat,
    writer: &mut WebSocketWrite<W>,
) -> Result<(), &'static str>
where
    W: AsyncWrite + Unpin,
{
    match heartbeat.deadline_action(Instant::now()) {
        HeartbeatAction::SendPing(ping) => {
            write_heartbeat_ping(writer, ping)
                .await
                .map_err(|_| HEARTBEAT_SEND_FAILED_REASON)?;
            heartbeat.record_ping_sent(Instant::now());
            Ok(())
        }
        HeartbeatAction::Timeout => Err(HEARTBEAT_TIMEOUT_REASON),
        HeartbeatAction::Wait => Ok(()),
    }
}

async fn write_heartbeat_ping<W>(
    writer: &mut WebSocketWrite<W>,
    ping_payload: PingPayload,
) -> Result<(), WebSocketError>
where
    W: AsyncWrite + Unpin,
{
    let frame = match ping_payload {
        PingPayload::Text(payload) => Frame::text(Payload::Borrowed(payload)),
        PingPayload::OpCode(payload) => {
            Frame::new(true, OpCode::Ping, None, Payload::Borrowed(payload))
        }
    };

    writer.write_frame(frame).await
}

fn format_close_frame_reason(payload: &[u8]) -> String {
    const MAX_CLOSE_REASON_CHARS: usize = 512;
    match payload {
        [] => "Connection closed by peer: no status code or reason".to_string(),
        [_] => "Connection closed by peer: invalid close payload (one byte)".to_string(),
        [code_high, code_low, reason @ ..] => {
            let code = u16::from_be_bytes([*code_high, *code_low]);
            let close_code = fastwebsockets::CloseCode::from(code);
            let reason = match std::str::from_utf8(reason) {
                Ok("") => "no reason".to_string(),
                Ok(reason) => {
                    let mut chars = reason.chars();
                    let preview: String = chars.by_ref().take(MAX_CLOSE_REASON_CHARS).collect();
                    let suffix = if chars.next().is_some() {
                        " (truncated)"
                    } else {
                        ""
                    };
                    format!("reason={preview:?}{suffix}")
                }
                Err(_) => format!("reason=<invalid UTF-8, {} bytes>", reason.len()),
            };

            format!("Connection closed by peer: code={code} ({close_code:?}), {reason}")
        }
    }
}

pub(super) struct WsTransport(WebSocket<TokioIo<Upgraded>>);

impl WsTransport {
    /// Reads frames, handles heartbeat and Ping/Pong at transport level,
    /// forwards text frames to the processor task.
    async fn read_frame(
        self,
        heartbeat_policy: HeartbeatPolicy,
        frame_tx: AnySender<Result<Vec<u8>, String>>,
    ) {
        let (mut reader, writer) = self.0.split(tokio::io::split);
        reader.set_auto_pong(false);
        reader.set_auto_close(false);

        let reader = FragmentCollectorRead::new(reader);
        WsConnection::new(reader, writer, heartbeat_policy, frame_tx)
            .run()
            .await;
    }

    pub(super) async fn write_frame(&mut self, frame: Frame<'_>) -> Result<(), WebSocketError> {
        self.0.write_frame(frame).await
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
        Ok(Self(ws))
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
        Ok(Self(ws))
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
