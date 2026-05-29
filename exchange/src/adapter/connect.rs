use crate::error::AdapterError;

use crate::{Event, adapter::StreamKind};
use bytes::Bytes;
use fastwebsockets::FragmentCollector;
use futures::{Stream as FuturesStream, channel::mpsc};
use http_body_util::Empty;
use hyper::{
    Request,
    header::{CONNECTION, UPGRADE},
    upgrade::Upgraded,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{
    future::Future,
    pin::Pin,
    sync::LazyLock,
    task::{Context, Poll},
    time::Duration,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, OwnedTrustAnchor},
};
use url::Url;

use fastwebsockets::{Frame, OpCode, Payload};
use futures::SinkExt;
use std::sync::Arc;
use tokio::time::{Instant, Interval};

pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
pub const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(1);
pub const HEARTBEAT_SEND_FAILED_REASON: &str = "Failed to send heartbeat ping";
pub const HEARTBEAT_PONG_FAILED_REASON: &str = "Failed to reply pong";

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

pub static TLS_CONNECTOR: LazyLock<TlsConnector> = LazyLock::new(tls_connector);

pub type WsTransport = FragmentCollector<TokioIo<Upgraded>>;

fn tls_connector() -> TlsConnector {
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

    TlsConnector::from(std::sync::Arc::new(config))
}

struct ChannelStream<T> {
    receiver: mpsc::Receiver<T>,
    task: tokio::task::JoinHandle<()>,
}

impl<T> FuturesStream for ChannelStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl<T> Drop for ChannelStream<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn channel<T, Fut, F>(buffer: usize, f: F) -> impl futures::Stream<Item = T>
where
    T: Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
    F: FnOnce(mpsc::Sender<T>) -> Fut + Send + 'static,
{
    let (sender, receiver) = mpsc::channel(buffer);
    let task = tokio::spawn(async move {
        f(sender).await;
    });

    ChannelStream { receiver, task }
}

enum WsState {
    Disconnected,
    Connected(WsTransport),
}

pub async fn connect_ws(
    domain: &str,
    url: &str,
    proxy_cfg: Option<&super::proxy::Proxy>,
) -> Result<WsTransport, AdapterError> {
    let parsed = Url::parse(url).map_err(|e| AdapterError::InvalidRequest(e.to_string()))?;

    let url_host = parsed
        .host_str()
        .ok_or_else(|| AdapterError::InvalidRequest("Missing host in websocket URL".to_string()))?;

    if !url_host.eq_ignore_ascii_case(domain) {
        return Err(AdapterError::InvalidRequest(format!(
            "WebSocket URL host mismatch: url_host={url_host}, domain_arg={domain}"
        )));
    }

    let target_port = parsed.port_or_known_default().ok_or_else(|| {
        AdapterError::InvalidRequest("Missing port for websocket URL".to_string())
    })?;

    let stream = setup_tcp(domain, target_port, proxy_cfg).await?;

    match parsed.scheme() {
        "wss" => {
            let tls_stream =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, upgrade_to_tls(domain, stream))
                    .await
                    .map_err(|_| {
                        AdapterError::WebsocketError(
                            "TLS handshake to target timed out".to_string(),
                        )
                    })??;

            tokio::time::timeout(
                WS_HANDSHAKE_TIMEOUT,
                upgrade_to_websocket(domain, tls_stream, &parsed),
            )
            .await
            .map_err(|_| {
                AdapterError::WebsocketError("WebSocket handshake timed out".to_string())
            })?
        }
        "ws" => tokio::time::timeout(
            WS_HANDSHAKE_TIMEOUT,
            upgrade_to_websocket(domain, stream, &parsed),
        )
        .await
        .map_err(|_| AdapterError::WebsocketError("WebSocket handshake timed out".to_string()))?,
        _ => Err(AdapterError::InvalidRequest(
            "Invalid scheme for websocket URL".to_string(),
        )),
    }
}

async fn setup_tcp(
    domain: &str,
    target_port: u16,
    proxy_cfg: Option<&super::proxy::Proxy>,
) -> Result<super::proxy::ProxyStream, AdapterError> {
    if let Some(proxy) = proxy_cfg {
        log::info!("Using proxy for WS: {}", proxy);
        return proxy.connect_tcp(domain, target_port).await;
    }

    let addr = format!("{domain}:{target_port}");
    let tcp = tokio::time::timeout(TCP_CONNECT_TIMEOUT, tokio::net::TcpStream::connect(&addr))
        .await
        .map_err(|_| AdapterError::WebsocketError(format!("TCP connect timeout: {addr}")))?
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    Ok(super::proxy::ProxyStream::Plain(tcp))
}

async fn upgrade_to_tls<S>(
    domain: &str,
    stream: S,
) -> Result<tokio_rustls::client::TlsStream<S>, AdapterError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let domain: tokio_rustls::rustls::ServerName =
        tokio_rustls::rustls::ServerName::try_from(domain)
            .map_err(|_| AdapterError::ParseError("invalid dnsname".to_string()))?;

    TLS_CONNECTOR
        .connect(domain, stream)
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))
}

async fn upgrade_to_websocket<S>(
    domain: &str,
    stream: S,
    parsed: &Url,
) -> Result<WsTransport, AdapterError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
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

    let req: Request<Empty<Bytes>> = Request::builder()
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
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    let exec = TokioExecutor::new();
    let (ws, _) = fastwebsockets::handshake::client(&exec, req, stream)
        .await
        .map_err(|e| AdapterError::WebsocketError(e.to_string()))?;

    Ok(FragmentCollector::new(ws))
}

#[derive(Clone, Copy, Debug)]
pub enum PingPayload {
    Text(&'static [u8]),
    OpCode(&'static [u8]),
}

impl PingPayload {
    pub async fn send_heartbeat_ping(
        &self,
        websocket: &mut WsTransport,
    ) -> Result<(), &'static str> {
        let frame = match self {
            PingPayload::Text(payload) => Frame::text(Payload::Borrowed(payload)),
            PingPayload::OpCode(payload) => {
                Frame::new(true, OpCode::Ping, None, Payload::Borrowed(payload))
            }
        };

        websocket
            .write_frame(frame)
            .await
            .map_err(|_| HEARTBEAT_SEND_FAILED_REASON)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ConnectedEventMode {
    Immediate,
    AdapterManaged,
}

#[derive(Clone, Debug)]
pub struct WsControlConfig {
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub reconnect_delay: Duration,
    pub ping_payload: PingPayload,
    pub text_pong_payload: Option<&'static [u8]>,
    pub connected_event_mode: ConnectedEventMode,
    pub stream_scope: Arc<[StreamKind]>,
}

pub trait WsAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String>;
    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>);
    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String>;
    async fn on_disconnected(&mut self, reason: &str, output: &mut mpsc::Sender<Event>);
}

impl WsControlConfig {
    pub fn with_text_ping(
        ping_payload: &'static [u8],
        text_pong_payload: Option<&'static [u8]>,
        stream_scope: Arc<[StreamKind]>,
    ) -> Self {
        Self {
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
            reconnect_delay: DEFAULT_RECONNECT_DELAY,
            ping_payload: PingPayload::Text(ping_payload),
            text_pong_payload,
            connected_event_mode: ConnectedEventMode::Immediate,
            stream_scope,
        }
    }

    pub fn with_opcode_ping(
        ping_payload: &'static [u8],
        text_pong_payload: Option<&'static [u8]>,
        stream_scope: Arc<[StreamKind]>,
    ) -> Self {
        Self {
            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL,
            heartbeat_timeout: DEFAULT_HEARTBEAT_TIMEOUT,
            reconnect_delay: DEFAULT_RECONNECT_DELAY,
            ping_payload: PingPayload::OpCode(ping_payload),
            text_pong_payload,
            connected_event_mode: ConnectedEventMode::Immediate,
            stream_scope,
        }
    }

    pub fn with_connected_event_mode(mut self, mode: ConnectedEventMode) -> Self {
        self.connected_event_mode = mode;
        self
    }

    pub async fn run<A: WsAdapter>(&self, adapter: &mut A, output: &mut mpsc::Sender<Event>) -> ! {
        let mut ws_state = WsState::Disconnected;
        let mut heartbeat = WsHeartbeat::new(self.heartbeat_interval, self.heartbeat_timeout);
        let stream_scope = self.stream_scope.clone();

        loop {
            match &mut ws_state {
                WsState::Disconnected => match adapter.connect().await {
                    Ok(websocket) => {
                        ws_state = WsState::Connected(websocket);
                        heartbeat.reset();

                        if matches!(self.connected_event_mode, ConnectedEventMode::Immediate) {
                            emit_connected(output, &stream_scope).await;
                        }

                        adapter.on_connected(output).await;
                    }
                    Err(reason) => {
                        emit_disconnected(output, &stream_scope, reason).await;
                        tokio::time::sleep(self.reconnect_delay).await;
                    }
                },
                WsState::Connected(websocket) => {
                    let disconnect_reason = tokio::select! {
                        _ = heartbeat.interval_mut().tick() => {
                            if heartbeat.timed_out() {
                                Some("Heartbeat timeout (no websocket activity)".to_string())
                            } else if self.ping_payload.send_heartbeat_ping(websocket).await.is_err() {
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
                                        if self.text_pong_payload
                                            .is_some_and(|pong| &msg.payload[..] == pong)
                                        {
                                            None
                                        } else {
                                            adapter.on_text(&msg.payload[..], output).await.err()
                                        }
                                    }
                                    OpCode::Ping => {
                                        if reply_pong(websocket, msg.payload).await.is_err() {
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
                        adapter.on_disconnected(&reason, output).await;
                        ws_state = WsState::Disconnected;
                        emit_disconnected(output, &stream_scope, reason).await;
                    }
                }
            }
        }
    }
}

pub struct WsHeartbeat {
    interval: Duration,
    timeout: Duration,
    heartbeat_interval: Interval,
    last_transport_activity: Instant,
}

impl WsHeartbeat {
    pub fn new(interval: Duration, timeout: Duration) -> Self {
        Self {
            interval,
            timeout,
            heartbeat_interval: tokio::time::interval(interval),
            last_transport_activity: Instant::now(),
        }
    }

    pub fn reset(&mut self) {
        self.last_transport_activity = Instant::now();
        self.heartbeat_interval = tokio::time::interval(self.interval);
    }

    pub fn interval_mut(&mut self) -> &mut Interval {
        &mut self.heartbeat_interval
    }

    pub fn mark_activity(&mut self) {
        self.last_transport_activity = Instant::now();
    }

    pub fn timed_out(&self) -> bool {
        self.last_transport_activity.elapsed() >= self.timeout
    }
}

pub async fn reply_pong(
    websocket: &mut WsTransport,
    payload: Payload<'_>,
) -> Result<(), &'static str> {
    websocket
        .write_frame(Frame::pong(payload))
        .await
        .map_err(|_| HEARTBEAT_PONG_FAILED_REASON)
}

pub async fn emit_connected(output: &mut mpsc::Sender<Event>, stream_scope: &Arc<[StreamKind]>) {
    let _ = output.send(Event::Connected(stream_scope.clone())).await;
}

async fn emit_disconnected(
    output: &mut mpsc::Sender<Event>,
    stream_scope: &Arc<[StreamKind]>,
    reason: impl Into<String>,
) {
    let _ = output
        .send(Event::Disconnected(stream_scope.clone(), reason.into()))
        .await;
}
