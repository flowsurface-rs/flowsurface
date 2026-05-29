use crate::{
    Event,
    adapter::{StreamKind, connect::WsTransport},
};

use fastwebsockets::{Frame, OpCode, Payload};
use futures::{SinkExt, channel::mpsc};
use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, Interval};

pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
pub const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(1);
pub const HEARTBEAT_SEND_FAILED_REASON: &str = "Failed to send heartbeat ping";
pub const HEARTBEAT_PONG_FAILED_REASON: &str = "Failed to reply pong";

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
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
#[allow(dead_code)]
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

pub async fn emit_disconnected(
    output: &mut mpsc::Sender<Event>,
    stream_scope: &Arc<[StreamKind]>,
    reason: impl Into<String>,
) {
    let _ = output
        .send(Event::Disconnected(stream_scope.clone(), reason.into()))
        .await;
}

enum WsState {
    Disconnected,
    Connected(WsTransport),
}
