use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        hub::{TradeBuffer, WsAdapter, WsSession, WsTransport},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH, MexcHandle, contract_size_for_market,
    convert_to_mexc_timeframe, exchange_from_market_type, raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use fastwebsockets::Frame;
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde_json::json;
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::oneshot::{self, error::TryRecvError};

const PING_PAYLOAD: &[u8] = br#"{"method":"ping"}"#;

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "p")]
    pub price: f32,
    #[serde(rename = "v")]
    pub qty: f32,
    #[serde(rename = "T")]
    pub direction: u8,
    #[serde(rename = "t")]
    pub time: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct FuturesDepthItem {
    #[serde()]
    pub price: f32,
    #[serde()]
    pub qty: f32,
    #[serde()]
    pub order_count: f32,
}

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "asks")]
    pub asks: Vec<FuturesDepthItem>,
    #[serde(rename = "bids")]
    pub bids: Vec<FuturesDepthItem>,
    #[serde(rename = "version")]
    pub version: u64,
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "o")]
    open: f32,
    #[serde(rename = "h")]
    high: f32,
    #[serde(rename = "l")]
    low: f32,
    #[serde(rename = "c")]
    close: f32,
    #[serde(rename = "q")]
    quote_volume: f32,
    #[serde(rename = "a")]
    _amount: f32,
    #[serde(rename = "interval")]
    interval: String,
    #[serde(rename = "symbol")]
    symbol: String,
}

#[allow(dead_code)]
enum StreamData {
    Trade(Ticker, Vec<SonicTrade>, u64),
    Depth(SonicDepth, u64),
    Kline(Ticker, Vec<SonicKline>),
    Pong(u64),
    Subscription(String),
}

#[derive(Debug)]
enum StreamName {
    Depth,
    Trade,
    Kline,
    Subscription(String),
    Error,
    Pong,
    Unknown,
}

impl StreamName {
    fn from_topic(topic: &str) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if parts.first() == Some(&"pong") {
            return StreamName::Pong;
        }

        match parts.get(1) {
            Some(&"sub") => {
                StreamName::Subscription(parts.get(2).map(|s| s.to_string()).unwrap_or_default())
            }
            Some(&"deal") => StreamName::Trade,
            Some(&"depth") => StreamName::Depth,
            Some(&"kline") => StreamName::Kline,
            Some(&"error") => StreamName::Error,
            _ => StreamName::Unknown,
        }
    }
}

fn feed_de(
    slice: &[u8],
    ticker: Option<Ticker>,
    market_type: MarketKind,
) -> Result<StreamData, AdapterError> {
    let mut stream_type: Option<StreamName> = None;
    let mut ts: Option<u64> = None;
    let mut data_faststr: Option<sonic_rs::FastStr> = None;

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    let mut topic_ticker: Option<Ticker> = ticker;

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "channel" {
            if let Some(val) = v.as_str() {
                stream_type = Some(StreamName::from_topic(val));
            }
        } else if k == "data" {
            data_faststr = Some(v.as_raw_faststr().clone());
        } else if k == "ts" {
            ts = Some(
                v.as_u64()
                    .ok_or_else(|| AdapterError::ParseError("ts not found".to_string()))?,
            );
        } else if k == "symbol" {
            let ticker_str = v
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("symbol does not exist".to_string()))?;
            if topic_ticker.is_none() {
                topic_ticker = Some(Ticker::new(
                    ticker_str,
                    exchange_from_market_type(market_type),
                ));
            }
        }
    }

    if let Some(data) = data_faststr {
        match stream_type {
            Some(StreamName::Kline) => {
                let mut kline_data: SonicKline = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                kline_data.time *= 1000;

                let ticker =
                    Ticker::new(&kline_data.symbol, exchange_from_market_type(market_type));
                return Ok(StreamData::Kline(ticker, vec![kline_data]));
            }
            Some(StreamName::Trade) => {
                let deals_data: Vec<SonicTrade> = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                let trade_ticker = topic_ticker.ok_or_else(|| {
                    AdapterError::ParseError("Missing ticker for trade data".to_string())
                })?;
                return Ok(StreamData::Trade(
                    trade_ticker,
                    deals_data,
                    ts.unwrap_or_default(),
                ));
            }
            Some(StreamName::Depth) => {
                let depth = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                return Ok(StreamData::Depth(depth, ts.unwrap_or_default()));
            }
            Some(StreamName::Pong) => {
                return Ok(StreamData::Pong(ts.unwrap_or_default()));
            }
            Some(StreamName::Subscription(name)) => {
                return Ok(StreamData::Subscription(name));
            }
            Some(StreamName::Error) => {
                log::error!("Error: {data}");
            }
            _ => {
                log::error!("Unknown stream type");
            }
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    match interval {
        "Min1" => Some(Timeframe::M1),
        "Min5" => Some(Timeframe::M5),
        "Min15" => Some(Timeframe::M15),
        "Min30" => Some(Timeframe::M30),
        "Min60" => Some(Timeframe::H1),
        "Hour4" => Some(Timeframe::H4),
        "Day1" => Some(Timeframe::D1),
        _ => None,
    }
}

async fn connect_websocket(
    domain: &str,
    path: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    WsTransport::establish(domain, &url, proxy_cfg).await
}

struct TradeAdapter {
    market_type: MarketKind,
    tickers: Vec<TickerInfo>,
    buffer: TradeBuffer,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|_| "Failed to connect to websocket".to_string())?;

        for ticker_info in &self.tickers {
            let symbol = ticker_info.ticker.to_full_symbol_and_type().0;
            let deal_subscription = json!({
                "method": "sub.deal",
                "param": {
                    "symbol": symbol,
                }
            });

            if websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    deal_subscription.to_string().as_bytes(),
                )))
                .await
                .is_err()
            {
                log::error!("Failed to subscribe to trade stream for {}", symbol);
            }
        }

        Ok(websocket)
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Trade(ticker, mut de_trades, _)) =
            feed_de(payload, None, self.market_type)
        {
            if let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(&ticker) {
                let ticker_info = *ticker_info;
                let qty_norm = *qty_norm;

                de_trades.sort_unstable_by_key(|t| t.time);
                for trade in &de_trades {
                    let price =
                        Price::from_f32(trade.price).round_to_min_tick(ticker_info.min_ticksize);
                    self.buffer.push(
                        ticker,
                        Trade {
                            time: trade.time.into(),
                            is_sell: trade.direction == 2,
                            price,
                            qty: qty_norm.normalize_qty(trade.qty, trade.price),
                        },
                    );
                }
            } else {
                log::error!("Ticker info not found for ticker: {}", ticker);
            }
        }

        self.buffer.flush_if_ready(output).await;
        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }
}

pub fn connect_trade_stream(
    tickers: Vec<TickerInfo>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        tickers
            .iter()
            .map(|ticker_info| StreamKind::Trades {
                ticker_info: *ticker_info,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let ticker_info_map = tickers
        .iter()
        .map(|ticker_info| {
            (
                ticker_info.ticker,
                (
                    *ticker_info,
                    QtyNormalization::with_raw_qty_unit(
                        volume_size_unit() == SizeUnit::Quote,
                        *ticker_info,
                        raw_qty_unit_from_market_type(market_type),
                    ),
                ),
            )
        })
        .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

    let adapter = TradeAdapter {
        market_type,
        tickers,
        buffer: TradeBuffer::new(ticker_info_map),
        proxy_cfg,
    };

    WsSession::with_text_ping(PING_PAYLOAD, None, stream_scope).run(adapter)
}

struct DepthAdapter {
    handle: MexcHandle,
    ticker_info: TickerInfo,
    market_type: MarketKind,
    symbol_str: String,
    stream: StreamKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
    qty_norm: QtyNormalization,
    sync_machine: DepthSyncMachine,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|_| "Failed to connect to websocket".to_string())?;

        let depth_subscription = json!({
            "method": "sub.depth",
            "param": {
                "symbol": self.symbol_str,
            }
        });

        websocket
            .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                depth_subscription.to_string().as_bytes(),
            )))
            .await
            .map_err(|_| "Failed to send depth subscription frame".to_string())?;

        self.sync_machine = DepthSyncMachine::new(self.handle.clone(), self.ticker_info.ticker);
        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {
        self.sync_machine.begin_resync();
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        self.sync_machine
            .poll_snapshot_if_ready(self.ticker_info, self.qty_norm)?;

        let ticker = self.ticker_info.ticker;

        if let Ok(StreamData::Depth(de_depth, time)) =
            feed_de(payload, Some(ticker), self.market_type)
        {
            let diff = DepthDiff {
                version: de_depth.version,
                time,
                bids: de_depth
                    .bids
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
                asks: de_depth
                    .asks
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
            };

            if let Some(update_time) =
                self.sync_machine
                    .handle_depth_update(diff, self.ticker_info, self.qty_norm)?
            {
                let _ = output
                    .send(Event::DepthReceived(
                        self.stream,
                        update_time.into(),
                        self.sync_machine.current.depth.clone(),
                    ))
                    .await;
            }
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

/// A buffered depth diff from the WebSocket, queued while the snapshot is being fetched.
struct DepthDiff {
    version: u64,
    time: u64,
    bids: Vec<DeOrder>,
    asks: Vec<DeOrder>,
}

enum DepthSyncState {
    /// Fetching the initial snapshot; diffs received in the meantime are buffered.
    WaitingSnapshot(oneshot::Receiver<Result<DepthPayload, AdapterError>>),
    /// Fully synced - applying live diffs and emitting orderbook updates.
    Live,
}

struct DepthSyncMachine {
    handle: MexcHandle,
    ticker: Ticker,
    state: DepthSyncState,
    local_last_version: u64,
    pending: VecDeque<DepthDiff>,
    current: LocalDepthCache,
}

impl DepthSyncMachine {
    const MAX_PENDING_DEPTH_EVENTS: usize = 512;

    fn new(handle: MexcHandle, ticker: Ticker) -> Self {
        Self {
            state: DepthSyncState::Live,
            handle,
            ticker,
            local_last_version: 0,
            current: LocalDepthCache::default(),
            pending: VecDeque::new(),
        }
    }

    fn begin_resync(&mut self) {
        let fetch_snapshot = {
            let handle = self.handle.clone();
            let ticker = self.ticker;
            let (tx, rx) = oneshot::channel();

            tokio::spawn(async move {
                let result = handle.fetch_depth_snapshot(ticker).await;
                let _ = tx.send(result);
            });

            rx
        };

        self.local_last_version = 0;
        self.state = DepthSyncState::WaitingSnapshot(fetch_snapshot);
    }

    fn handle_snapshot_result(
        &mut self,
        snapshot_result: Result<DepthPayload, AdapterError>,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<(), String> {
        let snapshot = match snapshot_result {
            Ok(snapshot) => snapshot,
            Err(e) => return Err(format!("Depth fetch failed: {e}")),
        };

        self.current.update_with_qty_norm(
            DepthUpdate::Snapshot(snapshot),
            ticker_info.min_ticksize,
            Some(qty_norm),
        );
        self.local_last_version = self.current.last_update_id;

        let mut pending: Vec<DepthDiff> = self.pending.drain(..).collect();
        pending.sort_unstable_by_key(|d| d.version);

        for diff in pending {
            if diff.version <= self.local_last_version {
                continue;
            }

            let depth = DepthPayload {
                last_update_id: diff.version,
                time: diff.time.into(),
                bids: diff.bids,
                asks: diff.asks,
            };

            self.current.update_with_qty_norm(
                DepthUpdate::Diff(depth),
                ticker_info.min_ticksize,
                Some(qty_norm),
            );
            self.local_last_version = diff.version;
        }

        self.state = DepthSyncState::Live;
        Ok(())
    }

    fn on_live_diff(
        &mut self,
        diff: DepthDiff,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        if diff.version <= self.local_last_version {
            return Ok(None);
        }

        let depth = DepthPayload {
            last_update_id: diff.version,
            time: diff.time.into(),
            bids: diff.bids,
            asks: diff.asks,
        };

        self.current.update_with_qty_norm(
            DepthUpdate::Diff(depth),
            ticker_info.min_ticksize,
            Some(qty_norm),
        );
        self.local_last_version = diff.version;
        Ok(Some(diff.time))
    }

    fn queue_pending_diff(&mut self, diff: DepthDiff) {
        if self.pending.len() == Self::MAX_PENDING_DEPTH_EVENTS {
            self.pending.pop_front();
        }
        self.pending.push_back(diff);
    }

    fn poll_snapshot_if_ready(
        &mut self,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<(), String> {
        let snapshot_result = {
            let DepthSyncState::WaitingSnapshot(snapshot_rx) = &mut self.state else {
                return Ok(());
            };

            match snapshot_rx.try_recv() {
                Ok(snapshot_result) => Some(snapshot_result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Closed) => {
                    return Err("Depth fetch channel closed".to_string());
                }
            }
        };

        if let Some(snapshot_result) = snapshot_result {
            self.handle_snapshot_result(snapshot_result, ticker_info, qty_norm)?;
        }

        Ok(())
    }

    fn handle_depth_update(
        &mut self,
        diff: DepthDiff,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        if matches!(self.state, DepthSyncState::WaitingSnapshot(_)) {
            self.queue_pending_diff(diff);
            Ok(None)
        } else {
            self.on_live_diff(diff, ticker_info, qty_norm)
        }
    }
}

pub fn connect_depth_stream(
    handle: MexcHandle,
    ticker_info: TickerInfo,
    depth_aggr: StreamTicksize,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream = StreamKind::Depth {
        ticker_info,
        depth_aggr,
        push_freq,
    };
    let stream_scope: Arc<[StreamKind]> = Arc::from(vec![stream].into_boxed_slice());
    let ticker = ticker_info.ticker;
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let adapter = DepthAdapter {
        handle: handle.clone(),
        ticker_info,
        market_type,
        symbol_str,
        stream,
        proxy_cfg,
        qty_norm,
        sync_machine: DepthSyncMachine::new(handle, ticker),
    };

    WsSession::with_text_ping(PING_PAYLOAD, None, stream_scope).run(adapter)
}

struct KlineAdapter {
    market_type: MarketKind,
    streams: Vec<(TickerInfo, Timeframe)>,
    ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|e| format!("Failed to connect: {e}"))?;

        let mut subscribed_any = false;

        for (ticker_info, timeframe) in &self.streams {
            let ticker = ticker_info.ticker;
            let symbol = ticker.to_full_symbol_and_type().0;

            let Some(interval) = convert_to_mexc_timeframe(*timeframe, self.market_type) else {
                log::error!(
                    "Unsupported MEXC kline timeframe requested: {} ({})",
                    timeframe,
                    ticker
                );
                continue;
            };

            let subscribe_msg = json!({
                "method": "sub.kline",
                "param": {
                    "symbol": symbol.to_uppercase(),
                    "interval": interval,
                },
                "gzip": false,
            });

            if websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    subscribe_msg.to_string().as_bytes(),
                )))
                .await
                .is_ok()
            {
                subscribed_any = true;
            }
        }

        if !subscribed_any {
            return Err("No supported MEXC kline timeframes requested".to_string());
        }

        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
            feed_de(payload, None, self.market_type)
        {
            for de_kline in &de_kline_vec {
                let Some(timeframe) = string_to_timeframe(&de_kline.interval) else {
                    log::error!(
                        "Failed to find timeframe: {}, {:?}",
                        &de_kline.interval,
                        self.streams
                    );
                    continue;
                };

                let Some((ticker_info, qty_norm)) = self.ticker_info_map.get(&ticker) else {
                    log::error!("Ticker info not found for ticker: {}", ticker);
                    continue;
                };

                let ticker_info = *ticker_info;
                let volume = qty_norm.normalize_qty(de_kline.quote_volume, de_kline.close);

                let kline = Kline::new(
                    de_kline.time,
                    de_kline.open,
                    de_kline.high,
                    de_kline.low,
                    de_kline.close,
                    Volume::TotalOnly(volume),
                    ticker_info.min_ticksize,
                );

                let _ = output
                    .send(Event::KlineReceived(
                        StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        },
                        kline,
                    ))
                    .await;
            }
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        streams
            .iter()
            .map(|(ticker_info, timeframe)| StreamKind::Kline {
                ticker_info: *ticker_info,
                timeframe: *timeframe,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)> = streams
        .iter()
        .filter_map(|(ticker_info, _)| {
            let _ =
                contract_size_for_market(*ticker_info, market_type, "connect_kline_stream").ok()?;
            Some((
                ticker_info.ticker,
                (
                    *ticker_info,
                    QtyNormalization::with_raw_qty_unit(
                        volume_size_unit() == SizeUnit::Quote,
                        *ticker_info,
                        raw_qty_unit_from_market_type(market_type),
                    ),
                ),
            ))
        })
        .collect();

    let adapter = KlineAdapter {
        market_type,
        streams,
        ticker_info_map,
        proxy_cfg,
    };

    WsSession::with_text_ping(PING_PAYLOAD, None, stream_scope).run(adapter)
}
