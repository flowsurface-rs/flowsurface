use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Trade, Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        hub::{TradeBuffer, WsAdapter, WsSession, WsTransport, emit_connected},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{BinanceHandle, exchange_from_market_type, raw_qty_unit_from_market_type};
use crate::adapter::hub::AdapterError;
use futures::{SinkExt, Stream, channel::mpsc};
use serde::Deserialize;
use sonic_rs::{JsonValueTrait, to_object_iter_unchecked};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::oneshot::{self, error::TryRecvError};

const MAX_PENDING_DEPTH_EVENTS: usize = 512;
const BINANCE_OPCODE_PING_PAYLOAD: &[u8] = b"fs";

fn ws_domain_from_market_type(market: MarketKind) -> &'static str {
    match market {
        MarketKind::Spot => "stream.binance.com",
        MarketKind::LinearPerps => "fstream.binance.com",
        MarketKind::InversePerps => "dstream.binance.com",
    }
}

#[derive(Clone, Copy)]
enum WsTrafficKind {
    Public,
    Market,
}

fn ws_stream_path(market: MarketKind, traffic_kind: WsTrafficKind) -> &'static str {
    match market {
        MarketKind::Spot => "stream",
        MarketKind::LinearPerps | MarketKind::InversePerps => match traffic_kind {
            WsTrafficKind::Public => "public/stream",
            WsTrafficKind::Market => "market/stream",
        },
    }
}

async fn connect_stream_socket(
    market: MarketKind,
    traffic_kind: WsTrafficKind,
    stream: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, String> {
    let domain = ws_domain_from_market_type(market);
    let stream_path = ws_stream_path(market, traffic_kind);
    let url = format!("wss://{domain}/{stream_path}?streams={stream}");

    WsTransport::establish(domain, &url, proxy_cfg)
        .await
        .map_err(|e| format!("Failed to connect to websocket: {e}"))
}

enum ApplyDepthResult {
    Applied(u64),
    Skipped,
    NeedsResync(String),
}

enum DepthSyncState {
    /// Unsynced state where we need snapshots to correctly apply diff. updates.
    /// Buffers incoming diff. updates until snapshot is applied, then replays them.
    /// Never emits local orderbook to the caller in this state.
    WaitingSnapshot(oneshot::Receiver<Result<DepthPayload, AdapterError>>),
    /// Synced and applying live diff. updates, without needing snapshots.
    /// Emits local orderbook to the caller only as live diff. updates are applied.
    Live,
}

struct DepthSyncMachine {
    handle: BinanceHandle,
    ticker: Ticker,
    state: DepthSyncState,
    prev_id: u64,
    pending: VecDeque<SonicDepth>,
    current: LocalDepthCache,
}

impl DepthSyncMachine {
    fn new(handle: BinanceHandle, ticker: Ticker) -> Self {
        let mut depth_sync = Self {
            state: DepthSyncState::Live,
            handle,
            ticker,
            prev_id: 0,
            current: LocalDepthCache::default(),
            pending: VecDeque::new(),
        };
        depth_sync.begin_resync();
        depth_sync
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
        self.prev_id = 0;

        while let Some(depth_type) = self.pending.pop_front() {
            match depth_type.apply_depth_diff(
                &mut self.current,
                ticker_info,
                qty_norm,
                &mut self.prev_id,
            ) {
                ApplyDepthResult::Applied(_) => {}
                ApplyDepthResult::Skipped => {}
                ApplyDepthResult::NeedsResync(reason) => {
                    log::warn!("{}", reason);
                    self.begin_resync();
                    return Ok(());
                }
            }
        }

        self.state = DepthSyncState::Live;
        Ok(())
    }

    fn on_live_diff(
        &mut self,
        diff_update: SonicDepth,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        match diff_update.apply_depth_diff(
            &mut self.current,
            ticker_info,
            qty_norm,
            &mut self.prev_id,
        ) {
            ApplyDepthResult::Applied(time) => Ok(Some(time)),
            ApplyDepthResult::Skipped => Ok(None),
            ApplyDepthResult::NeedsResync(reason) => {
                log::warn!("{}", reason);
                self.pending.clear();
                self.pending.push_back(diff_update);
                self.prev_id = 0;
                self.begin_resync();
                Ok(None)
            }
        }
    }

    fn queue_pending_diff(&mut self, diff_update: SonicDepth) {
        if self.pending.len() == MAX_PENDING_DEPTH_EVENTS {
            self.pending.pop_front();
        }

        self.pending.push_back(diff_update);
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
                    return Err("Depth fetch channel error: channel closed".to_string());
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
        diff_update: SonicDepth,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        if matches!(self.state, DepthSyncState::WaitingSnapshot(_)) {
            self.queue_pending_diff(diff_update);
            Ok(None)
        } else {
            self.on_live_diff(diff_update, ticker_info, qty_norm)
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "o", deserialize_with = "de_string_to_number")]
    open: f32,
    #[serde(rename = "h", deserialize_with = "de_string_to_number")]
    high: f32,
    #[serde(rename = "l", deserialize_with = "de_string_to_number")]
    low: f32,
    #[serde(rename = "c", deserialize_with = "de_string_to_number")]
    close: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_number")]
    volume: f32,
    #[serde(rename = "V", deserialize_with = "de_string_to_number")]
    taker_buy_base_asset_volume: f32,
    #[serde(rename = "i")]
    interval: String,
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKlineWrap {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "k")]
    kline: SonicKline,
}

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_number")]
    price: f32,
    #[serde(rename = "q", deserialize_with = "de_string_to_number")]
    qty: f32,
    #[serde(rename = "m")]
    is_sell: bool,
}

enum SonicDepth {
    Spot(SpotDepth),
    Perp(PerpDepth),
}

impl SonicDepth {
    fn apply_depth_diff(
        &self,
        orderbook: &mut LocalDepthCache,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
        prev_id: &mut u64,
    ) -> ApplyDepthResult {
        let last_update_id = orderbook.last_update_id;

        match self {
            SonicDepth::Perp(de_depth) => {
                if (de_depth.final_id <= last_update_id) || last_update_id == 0 {
                    return ApplyDepthResult::Skipped;
                }

                let next_expected = last_update_id.saturating_add(1);
                if *prev_id == 0 {
                    if (de_depth.first_id > next_expected) || (next_expected > de_depth.final_id) {
                        return ApplyDepthResult::NeedsResync(format!(
                            "Perp first event out of sync. first_id={}, final_id={}, snapshot_last_id={}",
                            de_depth.first_id, de_depth.final_id, last_update_id
                        ));
                    }
                } else if *prev_id != de_depth.prev_final_id {
                    return ApplyDepthResult::NeedsResync(format!(
                        "Perp out of sync. expected prev_final_id={}, got={}",
                        *prev_id, de_depth.prev_final_id
                    ));
                }

                orderbook.update_with_qty_norm(
                    DepthUpdate::Diff(self.into()),
                    ticker_info.min_ticksize,
                    Some(qty_norm),
                );

                *prev_id = de_depth.final_id;
                ApplyDepthResult::Applied(de_depth.time)
            }
            SonicDepth::Spot(de_depth) => {
                if (de_depth.final_id <= last_update_id) || last_update_id == 0 {
                    return ApplyDepthResult::Skipped;
                }

                let next_expected = last_update_id.saturating_add(1);
                if *prev_id == 0 {
                    if (de_depth.first_id > next_expected) || (next_expected > de_depth.final_id) {
                        return ApplyDepthResult::NeedsResync(format!(
                            "Spot first event out of sync. first_id={}, final_id={}, snapshot_last_id={}",
                            de_depth.first_id, de_depth.final_id, last_update_id
                        ));
                    }
                } else {
                    let expected_prev = de_depth.first_id.saturating_sub(1);
                    if *prev_id != expected_prev {
                        return ApplyDepthResult::NeedsResync(format!(
                            "Spot out of sync. expected prev_id={}, got={}",
                            *prev_id, expected_prev
                        ));
                    }
                }

                orderbook.update_with_qty_norm(
                    DepthUpdate::Diff(self.into()),
                    ticker_info.min_ticksize,
                    Some(qty_norm),
                );

                *prev_id = de_depth.final_id;
                ApplyDepthResult::Applied(de_depth.time)
            }
        }
    }
}

impl From<&SonicDepth> for DepthPayload {
    fn from(value: &SonicDepth) -> Self {
        let (time, final_id, bids, asks) = match value {
            SonicDepth::Spot(de) => (de.time, de.final_id, &de.bids, &de.asks),
            SonicDepth::Perp(de) => (de.time, de.final_id, &de.bids, &de.asks),
        };

        DepthPayload {
            last_update_id: final_id,
            time: time.into(),
            bids: bids
                .iter()
                .map(|x| DeOrder {
                    price: x.price,
                    qty: x.qty,
                })
                .collect(),
            asks: asks
                .iter()
                .map(|x| DeOrder {
                    price: x.price,
                    qty: x.qty,
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct SpotDepth {
    #[serde(rename = "E")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    asks: Vec<DeOrder>,
}

#[derive(Deserialize)]
struct PerpDepth {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "pu")]
    prev_final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    asks: Vec<DeOrder>,
}

enum StreamData {
    Trade(Ticker, SonicTrade),
    Depth(SonicDepth),
    Kline(Ticker, SonicKline),
}

enum StreamWrapper {
    Trade,
    Depth,
    Kline,
}

impl StreamWrapper {
    fn from_stream_type(stream_type: &str) -> Option<Self> {
        stream_type
            .split('@')
            .nth(1)
            .and_then(|after_at| match after_at {
                s if s.starts_with("de") => Some(StreamWrapper::Depth),
                s if s.starts_with("ag") => Some(StreamWrapper::Trade),
                s if s.starts_with("kl") => Some(StreamWrapper::Kline),
                _ => None,
            })
    }
}

fn feed_de(slice: &[u8], market: MarketKind) -> Result<StreamData, AdapterError> {
    let exchange = exchange_from_market_type(market);

    let mut stream_type: Option<StreamWrapper> = None;
    let mut topic_ticker: Option<Ticker> = None;
    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "stream" {
            let Some(stream_name) = v.as_str() else {
                continue;
            };

            if let Some(s) = StreamWrapper::from_stream_type(stream_name) {
                stream_type = Some(s);
            }

            if let Some(symbol) = stream_name.split('@').next() {
                topic_ticker = Some(Ticker::new(&symbol.to_uppercase(), exchange));
            }
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade: SonicTrade = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    if let Some(t) = topic_ticker {
                        return Ok(StreamData::Trade(t, trade));
                    }

                    return Err(AdapterError::ParseError(
                        "Missing ticker for trade data".to_string(),
                    ));
                }
                Some(StreamWrapper::Depth) => match market {
                    MarketKind::Spot => {
                        let depth: SpotDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Spot(depth)));
                    }
                    MarketKind::LinearPerps | MarketKind::InversePerps => {
                        let depth: PerpDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Perp(depth)));
                    }
                },
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: SonicKlineWrap = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Kline(
                        Ticker::new(&kline_wrap.symbol, exchange),
                        kline_wrap.kline,
                    ));
                }
                _ => {
                    log::error!("Unknown stream type");
                }
            }
        } else {
            log::error!("Unknown data: {:?}", k);
        }
    }

    Err(AdapterError::ParseError(
        "Failed to parse ws data".to_string(),
    ))
}

struct TradeAdapter {
    market: MarketKind,
    buffer: TradeBuffer,
    stream: String,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        connect_stream_socket(
            self.market,
            WsTrafficKind::Market,
            &self.stream,
            self.proxy_cfg.as_ref(),
        )
        .await
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Trade(ticker, de_trade)) = feed_de(payload, self.market) {
            if let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(&ticker) {
                let ticker_info = *ticker_info;
                let price =
                    Price::from_f32(de_trade.price).round_to_min_tick(ticker_info.min_ticksize);

                let trade = Trade {
                    time: de_trade.time.into(),
                    is_sell: de_trade.is_sell,
                    price,
                    qty: qty_norm.normalize_qty(de_trade.qty, de_trade.price),
                };

                self.buffer.push(ticker, trade);
            } else {
                log::error!("Ticker info not found for ticker: {ticker}");
                return Err("Received trade for unknown ticker".to_string());
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
    market: MarketKind,
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

    let stream = tickers
        .iter()
        .map(|ticker_info| {
            format!(
                "{}@aggTrade",
                ticker_info
                    .ticker
                    .to_full_symbol_and_type()
                    .0
                    .to_lowercase()
            )
        })
        .collect::<Vec<_>>()
        .join("/");

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
                        raw_qty_unit_from_market_type(market),
                    ),
                ),
            )
        })
        .collect();

    let adapter = TradeAdapter {
        market,
        buffer: TradeBuffer::new(ticker_info_map),
        stream: stream.clone(),
        proxy_cfg: proxy_cfg.clone(),
    };

    WsSession::with_opcode_ping(BINANCE_OPCODE_PING_PAYLOAD, None, stream_scope).run(adapter)
}

struct DepthAdapter {
    handle: BinanceHandle,
    market: MarketKind,
    ticker_info: TickerInfo,
    qty_norm: QtyNormalization,
    stream: StreamKind,
    stream_scope: Arc<[StreamKind]>,
    ws_stream: String,
    proxy_cfg: Option<crate::proxy::Proxy>,
    sync_machine: DepthSyncMachine,
    stream_ready: bool,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let websocket = connect_stream_socket(
            self.market,
            WsTrafficKind::Public,
            &self.ws_stream,
            self.proxy_cfg.as_ref(),
        )
        .await?;

        self.sync_machine = DepthSyncMachine::new(self.handle.clone(), self.ticker_info.ticker);
        self.stream_ready = false;

        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        self.sync_machine
            .poll_snapshot_if_ready(self.ticker_info, self.qty_norm)?;

        if let Ok(StreamData::Depth(depth_type)) = feed_de(payload, self.market)
            && let Some(time) = self.sync_machine.handle_depth_update(
                depth_type,
                self.ticker_info,
                self.qty_norm,
            )?
        {
            if !self.stream_ready {
                self.stream_ready = true;
                emit_connected(output, &self.stream_scope).await;
            }

            let _ = output
                .send(Event::DepthReceived(
                    self.stream,
                    time.into(),
                    self.sync_machine.current.depth.clone(),
                ))
                .await;
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_depth_stream(
    handle: BinanceHandle,
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
    let (symbol_str, market) = ticker.to_full_symbol_and_type();

    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(market),
    );

    let ws_stream = format!("{}@depth@100ms", symbol_str.to_lowercase());

    let adapter = DepthAdapter {
        handle: handle.clone(),
        market,
        ticker_info,
        qty_norm,
        stream,
        stream_scope: stream_scope.clone(),
        ws_stream: ws_stream.clone(),
        proxy_cfg,
        sync_machine: DepthSyncMachine::new(handle, ticker),
        stream_ready: false,
    };

    WsSession::with_opcode_ping(BINANCE_OPCODE_PING_PAYLOAD, None, stream_scope.clone())
        .run(adapter)
}

struct KlineAdapter {
    market: MarketKind,
    ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)>,
    timeframe_by_interval: HashMap<String, crate::Timeframe>,
    stream_str: String,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        connect_stream_socket(
            self.market,
            WsTrafficKind::Market,
            &self.stream_str,
            self.proxy_cfg.as_ref(),
        )
        .await
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Kline(ticker, de_kline)) = feed_de(payload, self.market) {
            let Some(timeframe) = self.timeframe_by_interval.get(&de_kline.interval) else {
                return Ok(());
            };

            if let Some((ticker_info, qty_norm)) = self.ticker_info_map.get(&ticker) {
                let ticker_info = *ticker_info;

                let buy_volume_raw = de_kline.taker_buy_base_asset_volume;
                let sell_volume_raw = de_kline.volume - buy_volume_raw;

                let buy_volume = qty_norm.normalize_qty(buy_volume_raw, de_kline.close);
                let sell_volume = qty_norm.normalize_qty(sell_volume_raw, de_kline.close);

                let kline = Kline::new(
                    de_kline.time,
                    de_kline.open,
                    de_kline.high,
                    de_kline.low,
                    de_kline.close,
                    Volume::BuySell(buy_volume, sell_volume),
                    ticker_info.min_ticksize,
                );

                let _ = output
                    .send(Event::KlineReceived(
                        StreamKind::Kline {
                            ticker_info,
                            timeframe: *timeframe,
                        },
                        kline,
                    ))
                    .await;
            } else {
                log::error!("Ticker info not found for ticker: {ticker}");
                return Err("Received kline for unknown ticker".to_string());
            }
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, crate::Timeframe)>,
    market: MarketKind,
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

    let stream_str = streams
        .iter()
        .map(|(ticker_info, timeframe)| {
            let ticker = ticker_info.ticker;
            format!(
                "{}@kline_{}",
                ticker.to_full_symbol_and_type().0.to_lowercase(),
                timeframe
            )
        })
        .collect::<Vec<String>>()
        .join("/");

    let ticker_info_map = streams
        .iter()
        .map(|(ticker_info, _)| {
            (
                ticker_info.ticker,
                (
                    *ticker_info,
                    QtyNormalization::with_raw_qty_unit(
                        volume_size_unit() == SizeUnit::Quote,
                        *ticker_info,
                        raw_qty_unit_from_market_type(market),
                    ),
                ),
            )
        })
        .collect();

    let timeframe_by_interval = streams
        .iter()
        .map(|(_, timeframe)| (timeframe.to_string(), *timeframe))
        .collect();

    let adapter = KlineAdapter {
        market,
        ticker_info_map,
        timeframe_by_interval,
        stream_str: stream_str.clone(),
        proxy_cfg: proxy_cfg.clone(),
    };

    WsSession::with_opcode_ping(BINANCE_OPCODE_PING_PAYLOAD, None, stream_scope).run(adapter)
}
