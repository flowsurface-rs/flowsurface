use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, UnixMs, Volume,
    adapter::connect::{State, channel, connect_ws},
    adapter::{MarketKind, StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH, MexcHandle, PING_INTERVAL,
    contract_size_for_market, convert_to_mexc_timeframe, exchange_from_market_type,
    raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use fastwebsockets::{FragmentCollector, Frame, OpCode};
use futures::{SinkExt, Stream};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use rustc_hash::FxHashMap;
use serde_json::json;
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use std::{collections::HashMap, time::Duration};

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
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    connect_ws(domain, &url, proxy_cfg).await
}

async fn send_ping(
    websocket: &mut FragmentCollector<TokioIo<Upgraded>>,
) -> Result<(), &'static str> {
    let ping_msg = json!({"method": "ping"});
    if websocket
        .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
            ping_msg.to_string().as_bytes(),
        )))
        .await
        .is_err()
    {
        log::error!("Failed to send ping");
        return Err("Failed to send ping");
    }
    Ok(())
}

pub fn connect_depth_stream(
    handle: MexcHandle,
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market_type);

        let mut orderbook = LocalDepthCache::default();
        let mut snapshot_ready = false;
        let mut snapshot_time = UnixMs::ZERO;

        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(market_type),
        );

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(
                        MEXC_FUTURES_WS_DOMAIN,
                        MEXC_FUTURES_WS_PATH,
                        proxy_cfg.as_ref(),
                    )
                    .await
                    {
                        Ok(mut websocket) => {
                            let depth_subscription = json!({
                                "method": "sub.depth",
                                "param": {
                                    "symbol": symbol_str,
                                }
                            });

                            if websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    depth_subscription.to_string().as_bytes(),
                                )))
                                .await
                                .is_err()
                            {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }

                            snapshot_ready = false;
                            snapshot_time = UnixMs::ZERO;
                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "Failed to connect to websocket".to_string(),
                                ))
                                .await;
                            continue;
                        }
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }

                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
                                    OpCode::Text => {
                                        match feed_de(&msg.payload[..], Some(ticker), market_type) {
                                            Ok(data) => {
                                                match data {
                                                    StreamData::Pong(_) => {}
                                                    StreamData::Subscription(stream_name) => {
                                                        if stream_name == "depth" {
                                                            match handle.fetch_depth_snapshot(ticker).await {
                                                                Ok(snapshot) => {
                                                                    snapshot_time = snapshot.time;
                                                                    snapshot_ready = true;
                                                                    orderbook.update_with_qty_norm(
                                                                        DepthUpdate::Snapshot(snapshot),
                                                                        ticker_info.min_ticksize,
                                                                        Some(qty_norm),
                                                                    );
                                                                }
                                                                Err(e) => {
                                                                    let _ = output
                                                                        .send(Event::Disconnected(
                                                                            exchange,
                                                                            format!("Failed to fetch depth snapshot: {e}"),
                                                                        ))
                                                                        .await;
                                                                    state = State::Disconnected;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    StreamData::Depth(de_depth, time) => {
                                                        if !snapshot_ready || time < snapshot_time.as_u64() {
                                                            continue;
                                                        }

                                                        let depth = DepthPayload {
                                                            last_update_id: de_depth.version,
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

                                                        orderbook.update_with_qty_norm(
                                                            DepthUpdate::Diff(depth),
                                                            ticker_info.min_ticksize,
                                                            Some(qty_norm),
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::Depth {
                                                                    ticker_info,
                                                                    depth_aggr: StreamTicksize::Client,
                                                                    push_freq,
                                                                },
                                                                time.into(),
                                                                orderbook.depth.clone(),
                                                            ))
                                                            .await;
                                                    }
                                                    StreamData::Trade(_, _, _) => {}
                                                    StreamData::Kline(_, _) => {}
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("Failed to parse MEXC depth message: {}", e);
                                            }
                                        }
                                    }
                                    OpCode::Close => {
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                        state = State::Disconnected;
                                    }
                                    _ => {}
                                },
                                Err(_) => {
                                    let _ = output
                                        .send(Event::Disconnected(
                                            exchange,
                                            "Error reading frame".to_string(),
                                        ))
                                        .await;
                                    state = State::Disconnected;
                                }
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
        let mut state: State = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);
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

        let mut trades_buffer_map: FxHashMap<Ticker, Vec<Trade>> = FxHashMap::default();
        let mut last_flush = tokio::time::Instant::now();

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(
                        MEXC_FUTURES_WS_DOMAIN,
                        MEXC_FUTURES_WS_PATH,
                        proxy_cfg.as_ref(),
                    )
                    .await
                    {
                        Ok(mut websocket) => {
                            for ticker_info in &tickers {
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
                                    log::error!("Failed to subscribe to trade stream");
                                    continue;
                                }
                            }

                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "Failed to connect to websocket".to_string(),
                                ))
                                .await;
                        }
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }

                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
                                    OpCode::Text => {
                                        match feed_de(&msg.payload[..], None, market_type) {
                                            Ok(data) => {
                                                match data {
                                                    StreamData::Pong(_) => {}
                                                    StreamData::Subscription(_) => {}
                                                    StreamData::Trade(ticker, mut de_trades, _) => {
                                                        if let Some((ticker_info, qty_norm)) = ticker_info_map.get(&ticker) {
                                                            let ticker_info = *ticker_info;

                                                            de_trades.sort_unstable_by_key(|t| t.time);
                                                            for trade in &de_trades {
                                                                let price = Price::from_f32(trade.price)
                                                                    .round_to_min_tick(ticker_info.min_ticksize);

                                                                let trade_entity = Trade {
                                                                    time: trade.time.into(),
                                                                    is_sell: trade.direction == 2,
                                                                    price,
                                                                    qty: qty_norm
                                                                        .normalize_qty(trade.qty, trade.price),
                                                                };

                                                                let trades_buffer = trades_buffer_map.entry(ticker).or_default();
                                                                trades_buffer.push(trade_entity);
                                                            }
                                                        } else {
                                                            log::error!("Ticker info not found for ticker: {}", ticker);
                                                        }
                                                    }
                                                    StreamData::Depth(_, _) => {}
                                                    StreamData::Kline(_, _) => {}
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("Failed to parse MEXC trade message: {}", e);
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
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                        state = State::Disconnected;
                                    }
                                    _ => {}
                                },
                                Err(_) => {
                                    flush_trade_buffers(
                                        &mut output,
                                        &ticker_info_map,
                                        &mut trades_buffer_map,
                                    )
                                    .await;
                                    let _ = output
                                        .send(Event::Disconnected(
                                            exchange,
                                            "Error reading frame".to_string(),
                                        ))
                                        .await;
                                    state = State::Disconnected;
                                }
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
        let mut state = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);
        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        if market_type == MarketKind::Spot {
            let _ = output
                .send(Event::Disconnected(
                    exchange,
                    "MEXC spot kline websocket stream is not supported".to_string(),
                ))
                .await;
            return;
        }

        let ticker_info_map = streams
            .iter()
            .map(|(ticker_info, _)| {
                contract_size_for_market(*ticker_info, market_type, "connect_kline_stream").map(
                    |_| {
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
                    },
                )
            })
            .collect::<Result<HashMap<Ticker, (TickerInfo, QtyNormalization)>, AdapterError>>();

        let ticker_info_map = match ticker_info_map {
            Ok(ticker_info_map) => ticker_info_map,
            Err(err) => {
                let _ = output
                    .send(Event::Disconnected(exchange, err.to_string()))
                    .await;
                return;
            }
        };

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(
                        MEXC_FUTURES_WS_DOMAIN,
                        MEXC_FUTURES_WS_PATH,
                        proxy_cfg.as_ref(),
                    )
                    .await
                    {
                        Ok(mut websocket) => {
                            let mut subscribed_any = false;

                            for (ticker_info, timeframe) in &streams {
                                let ticker = ticker_info.ticker;
                                let symbol = ticker.to_full_symbol_and_type().0;
                                let Some(interval) =
                                    convert_to_mexc_timeframe(*timeframe, market_type)
                                else {
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
                                    .is_err()
                                {
                                    continue;
                                }

                                subscribed_any = true;
                            }

                            if !subscribed_any {
                                let _ = output
                                    .send(Event::Disconnected(
                                        exchange,
                                        "No supported MEXC kline timeframes requested".to_string(),
                                    ))
                                    .await;
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }

                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
                        }
                        Err(err) => {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    format!("Failed to connect: {err}"),
                                ))
                                .await;
                        }
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }
                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
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

                                                        let volume = qty_norm.normalize_qty(
                                                            de_kline.quote_volume,
                                                            de_kline.close,
                                                        );

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
                            }
                        }
                    }
                }
            }
        }
    })
}
