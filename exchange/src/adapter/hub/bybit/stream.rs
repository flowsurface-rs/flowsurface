use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::connect::{State, channel, connect_ws},
    adapter::{MarketKind, StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{WS_DOMAIN, exchange_from_market_type, raw_qty_unit_from_market_type};
use crate::adapter::hub::AdapterError;
use fastwebsockets::{Frame, OpCode};
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde_json::Value;
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use std::{collections::HashMap, sync::Arc, time::Duration};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);
const HEARTBEAT_TIMEOUT_REASON: &str = "Heartbeat timeout (no websocket activity)";
const HEARTBEAT_SEND_FAILED_REASON: &str = "Failed to send heartbeat ping";
const BYBIT_PING_PAYLOAD: &[u8] = br#"{\"op\":\"ping\"}"#;

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    pub asks: Vec<DeOrder>,
}

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    pub time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_number")]
    pub price: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_number")]
    pub qty: f32,
    #[serde(rename = "S")]
    pub is_sell: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SonicKline {
    #[serde(rename = "start")]
    pub time: u64,
    #[serde(rename = "open", deserialize_with = "de_string_to_number")]
    pub open: f32,
    #[serde(rename = "high", deserialize_with = "de_string_to_number")]
    pub high: f32,
    #[serde(rename = "low", deserialize_with = "de_string_to_number")]
    pub low: f32,
    #[serde(rename = "close", deserialize_with = "de_string_to_number")]
    pub close: f32,
    #[serde(rename = "volume", deserialize_with = "de_string_to_number")]
    pub volume: f32,
    #[serde(rename = "interval")]
    pub interval: String,
}

enum StreamData {
    Trade(Ticker, Vec<SonicTrade>),
    Depth(SonicDepth, String, u64),
    Kline(Ticker, Vec<SonicKline>),
}

#[derive(Debug)]
enum StreamName {
    Depth(Ticker),
    Trade(Ticker),
    Kline(Ticker),
    Unknown,
}

impl StreamName {
    fn from_topic(topic: &str, is_ticker: Option<Ticker>, market_type: MarketKind) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if let Some(ticker_str) = parts.last() {
            let exchange = exchange_from_market_type(market_type);
            let ticker = is_ticker.unwrap_or_else(|| Ticker::new(ticker_str, exchange));

            match parts.first() {
                Some(&"publicTrade") => StreamName::Trade(ticker),
                Some(&"orderbook") => StreamName::Depth(ticker),
                Some(&"kline") => StreamName::Kline(ticker),
                _ => StreamName::Unknown,
            }
        } else {
            StreamName::Unknown
        }
    }
}

#[derive(Debug)]
enum StreamWrapper {
    Trade,
    Depth,
    Kline,
}

#[allow(unused_assignments)]
fn feed_de(
    slice: &[u8],
    ticker: Option<Ticker>,
    market_type: MarketKind,
) -> Result<StreamData, AdapterError> {
    let mut stream_type: Option<StreamWrapper> = None;
    let mut depth_wrap: Option<SonicDepth> = None;

    let mut data_type = String::new();
    let mut topic_ticker: Option<Ticker> = ticker;

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "topic" {
            if let Some(val) = v.as_str() {
                let mut is_ticker = None;

                if let Some(t) = ticker {
                    is_ticker = Some(t);
                }

                match StreamName::from_topic(val, is_ticker, market_type) {
                    StreamName::Depth(t) => {
                        stream_type = Some(StreamWrapper::Depth);
                        topic_ticker = Some(t);
                    }
                    StreamName::Trade(t) => {
                        stream_type = Some(StreamWrapper::Trade);
                        topic_ticker = Some(t);
                    }
                    StreamName::Kline(t) => {
                        stream_type = Some(StreamWrapper::Kline);
                        topic_ticker = Some(t);
                    }
                    _ => {
                        log::error!("Unknown stream name");
                    }
                }
            }
        } else if k == "type" {
            if let Some(value) = v.as_str() {
                value.clone_into(&mut data_type);
            } else {
                return Err(AdapterError::ParseError(
                    "Bybit frame `type` field is not a string".to_string(),
                ));
            }
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade_wrap: Vec<SonicTrade> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    if let Some(t) = topic_ticker {
                        return Ok(StreamData::Trade(t, trade_wrap));
                    } else {
                        return Err(AdapterError::ParseError(
                            "Missing ticker for trade data".to_string(),
                        ));
                    }
                }
                Some(StreamWrapper::Depth) => {
                    if depth_wrap.is_none() {
                        depth_wrap = Some(SonicDepth {
                            update_id: 0,
                            bids: Vec::new(),
                            asks: Vec::new(),
                        });
                    }
                    depth_wrap = Some(
                        sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?,
                    );
                }
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: Vec<SonicKline> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    if let Some(t) = topic_ticker {
                        return Ok(StreamData::Kline(t, kline_wrap));
                    } else {
                        return Err(AdapterError::ParseError(
                            "Missing ticker for kline data".to_string(),
                        ));
                    }
                }
                _ => {
                    log::error!("Unknown stream type");
                }
            }
        } else if k == "cts"
            && let Some(dw) = depth_wrap
        {
            let time: u64 = v
                .as_u64()
                .ok_or_else(|| AdapterError::ParseError("Failed to parse u64".to_string()))?;

            return Ok(StreamData::Depth(dw, data_type.to_string(), time));
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

async fn try_connect(
    streams: &Value,
    market_type: MarketKind,
    stream_scope: &Arc<[StreamKind]>,
    output: &mut mpsc::Sender<Event>,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> State {
    let url = format!(
        "wss://{}/v5/public/{}",
        WS_DOMAIN,
        match market_type {
            MarketKind::Spot => "spot",
            MarketKind::LinearPerps => "linear",
            MarketKind::InversePerps => "inverse",
        }
    );

    match connect_ws(WS_DOMAIN, &url, proxy_cfg).await {
        Ok(mut websocket) => {
            if let Err(e) = websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    streams.to_string().as_bytes(),
                )))
                .await
            {
                let _ = output
                    .send(Event::Disconnected(
                        stream_scope.clone(),
                        format!("Failed subscribing: {e}"),
                    ))
                    .await;
                return State::Disconnected;
            }

            let _ = output.send(Event::Connected(stream_scope.clone())).await;
            State::Connected(websocket)
        }
        Err(err) => {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let _ = output
                .send(Event::Disconnected(
                    stream_scope.clone(),
                    format!("Failed to connect: {err}"),
                ))
                .await;
            State::Disconnected
        }
    }
}

pub fn connect_depth_stream(
    ticker_info: TickerInfo,
    depth_aggr: StreamTicksize,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let stream = StreamKind::Depth {
            ticker_info,
            depth_aggr,
            push_freq,
        };
        let stream_scope = Arc::from(vec![stream].into_boxed_slice());
        let mut state: State = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

        let mut orderbook = LocalDepthCache::default();

        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(market_type),
        );

        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut last_transport_activity = tokio::time::Instant::now();

        loop {
            match &mut state {
                State::Disconnected => {
                    let depth_level = if let PushFrequency::Custom(tf) = push_freq {
                        match market_type {
                            MarketKind::Spot => match tf {
                                Timeframe::MS200 => "200",
                                Timeframe::MS300 => "1000",
                                _ => "200",
                            },
                            MarketKind::LinearPerps | MarketKind::InversePerps => match tf {
                                Timeframe::MS100 => "200",
                                Timeframe::MS300 => "1000",
                                _ => "200",
                            },
                        }
                    } else {
                        "200"
                    };

                    let stream = format!("orderbook.{depth_level}.{symbol_str}");

                    let subscribe_message = serde_json::json!({
                        "op": "subscribe",
                        "args": [stream]
                    });
                    state = try_connect(
                        &subscribe_message,
                        market_type,
                        &stream_scope,
                        &mut output,
                        proxy_cfg.as_ref(),
                    )
                    .await;

                    if matches!(state, State::Connected(_)) {
                        last_transport_activity = tokio::time::Instant::now();
                        heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = heartbeat_interval.tick() => {
                            if last_transport_activity.elapsed() >= HEARTBEAT_TIMEOUT {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_TIMEOUT_REASON.to_string(),
                                    ))
                                    .await;
                                continue;
                            }

                            if websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    BYBIT_PING_PAYLOAD,
                                )))
                                .await
                                .is_err()
                            {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_SEND_FAILED_REASON.to_string(),
                                    ))
                                    .await;
                            }
                        }
                        frame = websocket.read_frame() => match frame {
                            Ok(msg) => {
                                last_transport_activity = tokio::time::Instant::now();

                                match msg.opcode {
                                    OpCode::Text => {
                                        if let Ok(data) = feed_de(&msg.payload[..], Some(ticker), market_type) {
                                            match data {
                                                StreamData::Depth(de_depth, data_type, time) => {
                                                    let depth = DepthPayload {
                                                        last_update_id: de_depth.update_id,
                                                        time: time.into(),
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

                                                    if (data_type == "snapshot") || (depth.last_update_id == 1)
                                                    {
                                                        orderbook.update_with_qty_norm(
                                                            DepthUpdate::Snapshot(depth),
                                                            ticker_info.min_ticksize,
                                                            Some(qty_norm),
                                                        );
                                                    } else if data_type == "delta" {
                                                        orderbook.update_with_qty_norm(
                                                            DepthUpdate::Diff(depth),
                                                            ticker_info.min_ticksize,
                                                            Some(qty_norm),
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                stream,
                                                                time.into(),
                                                                orderbook.depth.clone(),
                                                            ))
                                                            .await;
                                                    }
                                                }
                                                _ => {
                                                    log::warn!("Unknown data received");
                                                }
                                            }
                                        }
                                    }
                                    OpCode::Ping => {
                                        if websocket.write_frame(Frame::pong(msg.payload)).await.is_err() {
                                            state = State::Disconnected;
                                            let _ = output
                                                .send(Event::Disconnected(
                                                    stream_scope.clone(),
                                                    "Failed to reply pong".to_string(),
                                                ))
                                                .await;
                                        }
                                    }
                                    OpCode::Pong => {}
                                    OpCode::Close => {
                                        state = State::Disconnected;
                                        let _ = output
                                            .send(Event::Disconnected(
                                                stream_scope.clone(),
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        "Error reading frame: ".to_string() + &e.to_string(),
                                    ))
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn connect_trade_stream(
    tickers: Vec<TickerInfo>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let stream_scope: Arc<[StreamKind]> = Arc::from(
            tickers
                .iter()
                .map(|ticker_info| StreamKind::Trades {
                    ticker_info: *ticker_info,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );

        if tickers.is_empty() {
            let _ = output
                .send(Event::Disconnected(
                    stream_scope,
                    "Empty Bybit trade stream payload".to_string(),
                ))
                .await;
            return;
        }

        let mut state: State = State::Disconnected;
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
                            raw_qty_unit_from_market_type(market_type),
                        ),
                    ),
                )
            })
            .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let mut last_flush = tokio::time::Instant::now();
        let mut trades_buffer_map: FxHashMap<Ticker, Vec<Trade>> = FxHashMap::default();
        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut last_transport_activity = tokio::time::Instant::now();

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream = tickers
                        .iter()
                        .map(|ticker_info| {
                            format!(
                                "publicTrade.{}",
                                ticker_info.ticker.to_full_symbol_and_type().0
                            )
                        })
                        .collect::<Vec<_>>();

                    let subscribe_message = serde_json::json!({
                        "op": "subscribe",
                        "args": stream
                    });

                    state = try_connect(
                        &subscribe_message,
                        market_type,
                        &stream_scope,
                        &mut output,
                        proxy_cfg.as_ref(),
                    )
                    .await;
                    last_flush = tokio::time::Instant::now();

                    if matches!(state, State::Connected(_)) {
                        last_transport_activity = tokio::time::Instant::now();
                        heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = heartbeat_interval.tick() => {
                            if last_transport_activity.elapsed() >= HEARTBEAT_TIMEOUT {
                                flush_trade_buffers(
                                    &mut output,
                                    &ticker_info_map,
                                    &mut trades_buffer_map,
                                )
                                .await;

                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_TIMEOUT_REASON.to_string(),
                                    ))
                                    .await;
                                continue;
                            }

                            if websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    BYBIT_PING_PAYLOAD,
                                )))
                                .await
                                .is_err()
                            {
                                flush_trade_buffers(
                                    &mut output,
                                    &ticker_info_map,
                                    &mut trades_buffer_map,
                                )
                                .await;

                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_SEND_FAILED_REASON.to_string(),
                                    ))
                                    .await;
                            }
                        }
                        frame = websocket.read_frame() => match frame {
                            Ok(msg) => {
                                last_transport_activity = tokio::time::Instant::now();

                                match msg.opcode {
                                    OpCode::Text => {
                                        if let Ok(StreamData::Trade(ticker, de_trade_vec)) =
                                            feed_de(&msg.payload[..], None, market_type)
                                        {
                                            if let Some((ticker_info, qty_norm)) = ticker_info_map.get(&ticker)
                                            {
                                                let ticker_info = *ticker_info;

                                                let trades_buffer =
                                                    trades_buffer_map.entry(ticker).or_default();
                                                for de_trade in &de_trade_vec {
                                                    let price = Price::from_f32(de_trade.price)
                                                        .round_to_min_tick(ticker_info.min_ticksize);

                                                    let trade = Trade {
                                                        time: de_trade.time.into(),
                                                        is_sell: de_trade.is_sell == "Sell",
                                                        price,
                                                        qty: qty_norm
                                                            .normalize_qty(de_trade.qty, de_trade.price),
                                                    };

                                                    trades_buffer.push(trade);
                                                }
                                            } else {
                                                log::error!("Ticker info not found for ticker: {}", ticker);
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
                                    OpCode::Ping => {
                                        if websocket.write_frame(Frame::pong(msg.payload)).await.is_err() {
                                            flush_trade_buffers(
                                                &mut output,
                                                &ticker_info_map,
                                                &mut trades_buffer_map,
                                            )
                                            .await;

                                            state = State::Disconnected;
                                            let _ = output
                                                .send(Event::Disconnected(
                                                    stream_scope.clone(),
                                                    "Failed to reply pong".to_string(),
                                                ))
                                                .await;
                                        }
                                    }
                                    OpCode::Pong => {}
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
                                                stream_scope.clone(),
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                flush_trade_buffers(&mut output, &ticker_info_map, &mut trades_buffer_map)
                                    .await;

                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        "Error reading frame: ".to_string() + &e.to_string(),
                                    ))
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
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

        if streams.is_empty() {
            let _ = output
                .send(Event::Disconnected(
                    stream_scope,
                    "Empty Bybit kline stream payload".to_string(),
                ))
                .await;
            return;
        }

        let mut state = State::Disconnected;
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
                            raw_qty_unit_from_market_type(market_type),
                        ),
                    ),
                )
            })
            .collect::<HashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let mut heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut last_transport_activity = tokio::time::Instant::now();

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream_str = streams
                        .iter()
                        .map(|(ticker_info, timeframe)| {
                            let ticker = ticker_info.ticker;
                            let timeframe_str = {
                                if Timeframe::D1 == *timeframe {
                                    "D".to_string()
                                } else {
                                    timeframe.to_minutes().to_string()
                                }
                            };
                            format!(
                                "kline.{timeframe_str}.{}",
                                ticker.to_full_symbol_and_type().0
                            )
                        })
                        .collect::<Vec<String>>();
                    let subscribe_message = serde_json::json!({
                        "op": "subscribe",
                        "args": stream_str
                    });

                    state = try_connect(
                        &subscribe_message,
                        market_type,
                        &stream_scope,
                        &mut output,
                        proxy_cfg.as_ref(),
                    )
                    .await;

                    if matches!(state, State::Connected(_)) {
                        last_transport_activity = tokio::time::Instant::now();
                        heartbeat_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = heartbeat_interval.tick() => {
                            if last_transport_activity.elapsed() >= HEARTBEAT_TIMEOUT {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_TIMEOUT_REASON.to_string(),
                                    ))
                                    .await;
                                continue;
                            }

                            if websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    BYBIT_PING_PAYLOAD,
                                )))
                                .await
                                .is_err()
                            {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        HEARTBEAT_SEND_FAILED_REASON.to_string(),
                                    ))
                                    .await;
                            }
                        }
                        frame = websocket.read_frame() => match frame {
                            Ok(msg) => {
                                last_transport_activity = tokio::time::Instant::now();

                                match msg.opcode {
                                    OpCode::Text => {
                                        if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
                                            feed_de(&msg.payload[..], None, market_type)
                                        {
                                            for de_kline in &de_kline_vec {
                                                if let Some(timeframe) = string_to_timeframe(&de_kline.interval)
                                                {
                                                    if let Some((ticker_info, qty_norm)) =
                                                        ticker_info_map.get(&ticker)
                                                    {
                                                        let ticker_info = *ticker_info;

                                                        let volume = qty_norm
                                                            .normalize_qty(de_kline.volume, de_kline.close);

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
                                                    } else {
                                                        log::error!(
                                                            "Ticker info not found for ticker: {}",
                                                            ticker
                                                        );
                                                    }
                                                } else {
                                                    log::error!(
                                                        "Failed to find timeframe: {}, {:?}",
                                                        &de_kline.interval,
                                                        streams
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    OpCode::Ping => {
                                        if websocket.write_frame(Frame::pong(msg.payload)).await.is_err() {
                                            state = State::Disconnected;
                                            let _ = output
                                                .send(Event::Disconnected(
                                                    stream_scope.clone(),
                                                    "Failed to reply pong".to_string(),
                                                ))
                                                .await;
                                        }
                                    }
                                    OpCode::Pong => {}
                                    OpCode::Close => {
                                        state = State::Disconnected;
                                        let _ = output
                                            .send(Event::Disconnected(
                                                stream_scope.clone(),
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                state = State::Disconnected;
                                let _ = output
                                    .send(Event::Disconnected(
                                        stream_scope.clone(),
                                        "Error reading frame: ".to_string() + &e.to_string(),
                                    ))
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    })
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    Timeframe::KLINE
        .iter()
        .find(|&tf| {
            tf.to_minutes().to_string() == interval || {
                if tf == &Timeframe::D1 {
                    interval == "D"
                } else {
                    false
                }
            }
        })
        .copied()
}
