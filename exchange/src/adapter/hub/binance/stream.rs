use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::connect::{State, channel, connect_ws},
    adapter::{MarketKind, StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{BinanceHandle, exchange_from_market_type, raw_qty_unit_from_market_type};
use crate::adapter::hub::AdapterError;
use fastwebsockets::OpCode;
use futures::{SinkExt, Stream};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use sonic_rs::{JsonValueTrait, to_object_iter_unchecked};
use std::collections::{HashMap, VecDeque};
use tokio::sync::oneshot;

const MAX_PENDING_DEPTH_EVENTS: usize = 512;

fn ws_domain_from_market_type(market: MarketKind) -> &'static str {
    match market {
        MarketKind::Spot => "stream.binance.com",
        MarketKind::LinearPerps => "fstream.binance.com",
        MarketKind::InversePerps => "dstream.binance.com",
    }
}

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

enum DepthReaderMsg {
    Depth(SonicDepth),
    Disconnected(String),
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
        snapshot_result: Result<Result<DepthPayload, AdapterError>, oneshot::error::RecvError>,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        let snapshot = match snapshot_result {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(e)) => return Err(format!("Depth fetch failed: {e}")),
            Err(e) => return Err(format!("Depth fetch channel error: {e}")),
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
                    return Ok(None);
                }
            }
        }

        self.state = DepthSyncState::Live;
        Ok(None)
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

    fn handle_depth_message(
        &mut self,
        depth_msg: DepthReaderMsg,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        match depth_msg {
            DepthReaderMsg::Depth(diff_update) => {
                if matches!(self.state, DepthSyncState::WaitingSnapshot(_)) {
                    self.queue_pending_diff(diff_update);
                    Ok(None)
                } else {
                    self.on_live_diff(diff_update, ticker_info, qty_norm)
                }
            }
            DepthReaderMsg::Disconnected(reason) => Err(reason),
        }
    }

    /// Ticks the state machine with the next diff. update. Returns:
    /// - `Ok(Some(`time`))` if a diff. update was successfully applied and the local orderbook was updated.
    ///   `time` is the event time of the applied diff.
    /// - `Ok(None)` if no update was applied and the update was buffered or skipped.
    /// - `Err(reason)` if the stream should be considered disconnected and reconnected.
    async fn tick(
        &mut self,
        websocket: &mut fastwebsockets::FragmentCollector<
            hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>,
        >,
        market: MarketKind,
        ticker_info: TickerInfo,
        qty_norm: QtyNormalization,
    ) -> Result<Option<u64>, String> {
        if matches!(self.state, DepthSyncState::WaitingSnapshot(_)) {
            let depth_msg = {
                let DepthSyncState::WaitingSnapshot(snapshot_rx) = &mut self.state else {
                    unreachable!("state must be WaitingSnapshot")
                };

                tokio::select! {
                    snapshot_result = snapshot_rx => {
                        return self.handle_snapshot_result(snapshot_result, ticker_info, qty_norm);
                    }
                    depth_msg = read_next_depth_message(websocket, market) => depth_msg,
                }
            };

            self.handle_depth_message(depth_msg, ticker_info, qty_norm)
        } else {
            let depth_msg = read_next_depth_message(websocket, market).await;
            self.handle_depth_message(depth_msg, ticker_info, qty_norm)
        }
    }
}

async fn read_next_depth_message(
    websocket: &mut fastwebsockets::FragmentCollector<
        hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>,
    >,
    market: MarketKind,
) -> DepthReaderMsg {
    loop {
        match websocket.read_frame().await {
            Ok(msg) => match msg.opcode {
                OpCode::Text => {
                    if let Ok(StreamData::Depth(depth_type)) = feed_de(&msg.payload[..], market) {
                        return DepthReaderMsg::Depth(depth_type);
                    }
                }
                OpCode::Close => {
                    return DepthReaderMsg::Disconnected("Connection closed".to_string());
                }
                _ => {}
            },
            Err(e) => {
                return DepthReaderMsg::Disconnected(
                    "Error reading frame: ".to_string() + &e.to_string(),
                );
            }
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

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;
        let exchange = exchange_from_market_type(market);

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        let ticker_info_map = streams
            .iter()
            .map(|(ticker_info, _)| {
                (
                    ticker_info.ticker,
                    (
                        *ticker_info,
                        QtyNormalization::with_raw_qty_unit(
                            size_in_quote_ccy,
                            *ticker_info,
                            raw_qty_unit_from_market_type(market),
                        ),
                    ),
                )
            })
            .collect::<HashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        loop {
            match &mut state {
                State::Disconnected => {
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

                    let domain = ws_domain_from_market_type(market);
                    let stream_path = ws_stream_path(market, WsTrafficKind::Market);
                    let url = format!("wss://{domain}/{stream_path}?streams={stream_str}");

                    if let Ok(websocket) = connect_ws(domain, &url, proxy_cfg.as_ref()).await {
                        state = State::Connected(websocket);
                        let _ = output.send(Event::Connected(exchange)).await;
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                "Failed to connect to websocket".to_string(),
                            ))
                            .await;
                    }
                }
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Kline(ticker, de_kline)) =
                                feed_de(&msg.payload[..], market)
                            {
                                let (buy_volume, sell_volume) = {
                                    let buy_volume = de_kline.taker_buy_base_asset_volume;
                                    let sell_volume = de_kline.volume - buy_volume;
                                    (buy_volume, sell_volume)
                                };

                                if let Some((_, tf)) = streams
                                    .iter()
                                    .find(|(_, tf)| tf.to_string() == de_kline.interval)
                                {
                                    if let Some((ticker_info, qty_norm)) =
                                        ticker_info_map.get(&ticker)
                                    {
                                        let ticker_info = *ticker_info;
                                        let timeframe = *tf;

                                        let buy_volume =
                                            qty_norm.normalize_qty(buy_volume, de_kline.close);
                                        let sell_volume =
                                            qty_norm.normalize_qty(sell_volume, de_kline.close);

                                        let volume = Volume::BuySell(buy_volume, sell_volume);

                                        let kline = Kline::new(
                                            de_kline.time,
                                            de_kline.open,
                                            de_kline.high,
                                            de_kline.low,
                                            de_kline.close,
                                            volume,
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
                                    } else {
                                        log::error!("Ticker info not found for ticker: {ticker}");
                                        state = State::Disconnected;
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Received kline for unknown ticker".to_string(),
                                            ))
                                            .await;
                                    }
                                }
                            }
                        }
                        OpCode::Close => {
                            state = State::Disconnected;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "Connection closed".to_string(),
                                ))
                                .await;
                        }
                        _ => {}
                    },
                    Err(e) => {
                        state = State::Disconnected;
                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                "Error reading frame: ".to_string() + &e.to_string(),
                            ))
                            .await;
                    }
                },
            }
        }
    })
}

pub fn connect_trade_stream(
    tickers: Vec<TickerInfo>,
    market: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;
        let exchange = exchange_from_market_type(market);

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
        let ticker_info_map = tickers
            .iter()
            .map(|ticker_info| {
                (
                    ticker_info.ticker,
                    (
                        *ticker_info,
                        QtyNormalization::with_raw_qty_unit(
                            size_in_quote_ccy,
                            *ticker_info,
                            raw_qty_unit_from_market_type(market),
                        ),
                    ),
                )
            })
            .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let mut trades_buffer_map: FxHashMap<Ticker, Vec<Trade>> = FxHashMap::default();
        let mut last_flush = tokio::time::Instant::now();

        loop {
            match &mut state {
                State::Disconnected => {
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

                    let domain = ws_domain_from_market_type(market);
                    let stream_path = ws_stream_path(market, WsTrafficKind::Market);
                    let url = format!("wss://{domain}/{stream_path}?streams={stream}");

                    if let Ok(websocket) = connect_ws(domain, &url, proxy_cfg.as_ref()).await {
                        state = State::Connected(websocket);
                        last_flush = tokio::time::Instant::now();

                        let _ = output.send(Event::Connected(exchange)).await;
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                "Failed to connect to websocket".to_string(),
                            ))
                            .await;
                    }
                }
                State::Connected(ws) => {
                    match ws.read_frame().await {
                        Ok(msg) => match msg.opcode {
                            OpCode::Text => {
                                if let Ok(StreamData::Trade(ticker, de_trade)) =
                                    feed_de(&msg.payload[..], market)
                                {
                                    if let Some((ticker_info, qty_norm)) =
                                        ticker_info_map.get(&ticker)
                                    {
                                        let ticker_info = *ticker_info;
                                        let price = Price::from_f32(de_trade.price)
                                            .round_to_min_tick(ticker_info.min_ticksize);

                                        let trade = Trade {
                                            time: de_trade.time.into(),
                                            is_sell: de_trade.is_sell,
                                            price,
                                            qty: qty_norm
                                                .normalize_qty(de_trade.qty, de_trade.price),
                                        };

                                        trades_buffer_map.entry(ticker).or_default().push(trade);
                                    } else {
                                        log::error!("Ticker info not found for ticker: {ticker}");
                                        state = State::Disconnected;
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Received trade for unknown ticker".to_string(),
                                            ))
                                            .await;
                                    }
                                }

                                if last_flush.elapsed() >= TRADE_BUCKET_INTERVAL {
                                    flush_trade_buffers(
                                        &mut output,
                                        &ticker_info_map,
                                        &mut trades_buffer_map,
                                    )
                                    .await;
                                    last_flush = tokio::time::Instant::now();
                                }
                            }
                            OpCode::Close => {
                                flush_trade_buffers(
                                    &mut output,
                                    &ticker_info_map,
                                    &mut trades_buffer_map,
                                )
                                .await;

                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        exchange,
                                        "Connection closed".to_string(),
                                    ))
                                    .await;
                            }
                            _ => {}
                        },
                        Err(e) => {
                            flush_trade_buffers(
                                &mut output,
                                &ticker_info_map,
                                &mut trades_buffer_map,
                            )
                            .await;

                            state = State::Disconnected;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "Error reading frame: ".to_string() + &e.to_string(),
                                ))
                                .await;
                        }
                    };
                }
            }
        }
    })
}

pub fn connect_depth_stream(
    handle: BinanceHandle,
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let ticker = ticker_info.ticker;
        let (symbol_str, market) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market);

        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(market),
        );

        let stream_kind = StreamKind::Depth {
            ticker_info,
            depth_aggr: StreamTicksize::Client,
            push_freq,
        };

        loop {
            let stream = format!("{}@depth@100ms", symbol_str.to_lowercase());
            let domain = ws_domain_from_market_type(market);
            let stream_path = ws_stream_path(market, WsTrafficKind::Public);
            let url = format!("wss://{domain}/{stream_path}?streams={stream}");

            let mut websocket = match connect_ws(domain, &url, proxy_cfg.as_ref()).await {
                Ok(ws) => ws,
                Err(_) => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    let _ = output
                        .send(Event::Disconnected(
                            exchange,
                            "Failed to connect to websocket".to_string(),
                        ))
                        .await;
                    continue;
                }
            };

            let mut sync_machine = DepthSyncMachine::new(handle.clone(), ticker);
            let mut connected_sent = false;

            let disconnect_reason = loop {
                match sync_machine
                    .tick(&mut websocket, market, ticker_info, qty_norm)
                    .await
                {
                    Ok(Some(time)) => {
                        if !connected_sent {
                            connected_sent = true;
                            let _ = output.send(Event::Connected(exchange)).await;
                        }

                        let synced_book = sync_machine.current.depth.clone();

                        let _ = output
                            .send(Event::DepthReceived(stream_kind, time.into(), synced_book))
                            .await;
                    }
                    Ok(None) => {}
                    Err(reason) => break reason,
                }
            };
            let _ = output
                .send(Event::Disconnected(exchange, disconnect_reason))
                .await;

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    })
}
