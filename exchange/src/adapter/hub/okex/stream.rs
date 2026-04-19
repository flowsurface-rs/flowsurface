use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::connect::{State, channel, connect_ws},
    adapter::{MarketKind, StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util,
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    WS_DOMAIN, exchange_from_market_type, raw_qty_unit_from_market_type, timeframe_to_okx_bar,
};
use crate::adapter::hub::AdapterError;
use fastwebsockets::{Frame, OpCode};
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "ts", deserialize_with = "de_string_to_number")]
    pub time: u64,
    #[serde(rename = "px", deserialize_with = "de_string_to_number")]
    pub price: f32,
    #[serde(rename = "sz", deserialize_with = "de_string_to_number")]
    pub qty: f32,
    #[serde(rename = "side")]
    pub is_sell: String,
}

struct SonicDepth {
    pub update_id: u64,
    pub bids: Vec<DeOrder>,
    pub asks: Vec<DeOrder>,
}

enum StreamData {
    Trade(String, Vec<SonicTrade>),
    Depth(SonicDepth, String, u64),
}

fn feed_de(slice: &[u8]) -> Result<StreamData, AdapterError> {
    let v: Value =
        serde_json::from_slice(slice).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let mut channel = String::new();
    let mut inst_id = String::new();
    if let Some(arg) = v.get("arg")
        && let Some(ch) = arg.get("channel").and_then(|c| c.as_str())
    {
        channel = ch.to_string();

        if let Some(symbol) = arg.get("instId").and_then(|c| c.as_str()) {
            inst_id = symbol.to_string();
        }
    }

    if let Some(action) = v.get("action").and_then(|a| a.as_str())
        && let Some(data_arr) = v.get("data")
        && let Some(first) = data_arr.get(0)
    {
        let bids: Vec<DeOrder> = if let Some(b) = first.get("bids") {
            serde_json::from_value(b.clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?
        } else {
            Vec::new()
        };
        let asks: Vec<DeOrder> = if let Some(a) = first.get("asks") {
            serde_json::from_value(a.clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?
        } else {
            Vec::new()
        };

        let seq_id = first.get("seqId").and_then(|s| s.as_u64()).unwrap_or(0);

        let time = first
            .get("ts")
            .and_then(serde_util::value_as_u64)
            .unwrap_or(0);

        let depth = SonicDepth {
            update_id: seq_id,
            bids,
            asks,
        };

        match channel.as_str() {
            "books" => {
                let dtype = if action == "update" {
                    "delta"
                } else {
                    "snapshot"
                };
                return Ok(StreamData::Depth(depth, dtype.to_string(), time));
            }
            _ => {
                return Err(AdapterError::ParseError(
                    "Depth message for non-depth subscription".to_string(),
                ));
            }
        }
    }

    if let Some(data_arr) = v.get("data") {
        let trades: Vec<SonicTrade> = serde_json::from_value(data_arr.clone())
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if matches!(channel.as_str(), "trades" | "trade") {
            if inst_id.is_empty() {
                return Err(AdapterError::ParseError(
                    "Missing instId for trade data".to_string(),
                ));
            }

            return Ok(StreamData::Trade(inst_id, trades));
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

async fn try_connect(
    streams: &Value,
    exchange: crate::Exchange,
    output: &mut mpsc::Sender<Event>,
    topic: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> State {
    let url = format!("wss://{WS_DOMAIN}/ws/v5/{topic}");

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
                        exchange,
                        format!("Failed subscribing: {e}"),
                    ))
                    .await;
                return State::Disconnected;
            }

            let _ = output.send(Event::Connected(exchange)).await;
            State::Connected(websocket)
        }
        Err(err) => {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            let _ = output
                .send(Event::Disconnected(
                    exchange,
                    format!("Failed to connect: {err}"),
                ))
                .await;
            State::Disconnected
        }
    }
}

pub fn connect_depth_stream(
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
        let exchange = ticker.exchange;

        let subscribe_message = serde_json::json!({
            "op": "subscribe",
            "args": [
                { "channel": "books",  "instId": symbol_str },
            ],
        });

        let mut orderbook = LocalDepthCache::default();

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
        let qty_norm = QtyNormalization::with_raw_qty_unit(
            size_in_quote_ccy,
            ticker_info,
            raw_qty_unit_from_market_type(market_type),
        );

        loop {
            match &mut state {
                State::Disconnected => {
                    state = try_connect(
                        &subscribe_message,
                        exchange,
                        &mut output,
                        "public",
                        proxy_cfg.as_ref(),
                    )
                    .await;
                }
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(data) = feed_de(&msg.payload[..]) {
                                match data {
                                    StreamData::Depth(de_depth, data_type, time) => {
                                        let depth = DepthPayload {
                                            last_update_id: de_depth.update_id,
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
                                                    StreamKind::Depth {
                                                        ticker_info,
                                                        depth_aggr: StreamTicksize::Client,
                                                        push_freq,
                                                    },
                                                    time,
                                                    orderbook.depth.clone(),
                                                ))
                                                .await;
                                        }
                                    }
                                    StreamData::Trade(_, _) => {}
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
    streams: Vec<TickerInfo>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);

        let args = streams
            .iter()
            .map(|ticker_info| {
                let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();
                serde_json::json!({
                    "channel": "trades",
                    "instId": symbol_str,
                })
            })
            .collect::<Vec<_>>();

        let subscribe_message = serde_json::json!({
            "op": "subscribe",
            "args": args,
        });

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
        let ticker_info_map = streams
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

        let symbol_to_ticker = streams
            .iter()
            .map(|ticker_info| {
                let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();
                (symbol_str, ticker_info.ticker)
            })
            .collect::<FxHashMap<String, Ticker>>();

        let mut trades_buffer_map: FxHashMap<Ticker, Vec<Trade>> = FxHashMap::default();
        let mut last_flush = tokio::time::Instant::now();

        loop {
            match &mut state {
                State::Disconnected => {
                    state = try_connect(
                        &subscribe_message,
                        exchange,
                        &mut output,
                        "public",
                        proxy_cfg.as_ref(),
                    )
                    .await;
                    last_flush = tokio::time::Instant::now();
                }
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Trade(inst_id, de_trade_vec)) =
                                feed_de(&msg.payload[..])
                            {
                                if let Some(ticker) = symbol_to_ticker.get(&inst_id)
                                    && let Some((ticker_info, qty_norm)) =
                                        ticker_info_map.get(ticker)
                                {
                                    let ticker_info = *ticker_info;
                                    let trades_buffer =
                                        trades_buffer_map.entry(*ticker).or_default();

                                    for de_trade in &de_trade_vec {
                                        let price = Price::from_f32(de_trade.price)
                                            .round_to_min_tick(ticker_info.min_ticksize);
                                        let qty =
                                            qty_norm.normalize_qty(de_trade.qty, de_trade.price);

                                        let trade = Trade {
                                            time: de_trade.time,
                                            is_sell: de_trade.is_sell == "sell"
                                                || de_trade.is_sell == "SELL",
                                            price,
                                            qty,
                                        };
                                        trades_buffer.push(trade);
                                    }
                                } else {
                                    log::error!("Ticker info not found for symbol: {}", inst_id);
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
                        flush_trade_buffers(&mut output, &ticker_info_map, &mut trades_buffer_map)
                            .await;

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

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let mut args = Vec::with_capacity(streams.len());
        let mut lookup = HashMap::new();
        for (ticker_info, timeframe) in &streams {
            let ticker = ticker_info.ticker;

            if let Some(bar) = timeframe_to_okx_bar(*timeframe) {
                let (symbol, _mt) = ticker.to_full_symbol_and_type();
                let channel = format!("candle{bar}");
                args.push(serde_json::json!({
                    "channel": channel,
                    "instId": symbol,
                }));
                lookup.insert((channel, symbol), (*ticker_info, *timeframe));
            }
        }

        let exchange = streams
            .first()
            .map(|(t, _)| t.exchange())
            .unwrap_or_else(|| crate::Exchange::OkexSpot);

        let subscribe_message = serde_json::json!({
            "op": "subscribe",
            "args": args,
        });

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        loop {
            match &mut state {
                State::Disconnected => {
                    state = try_connect(
                        &subscribe_message,
                        exchange,
                        &mut output,
                        "business",
                        proxy_cfg.as_ref(),
                    )
                    .await;
                }
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(v) = serde_json::from_slice::<Value>(&msg.payload[..]) {
                                let channel = v["arg"]["channel"].as_str().unwrap_or("");
                                if !channel.starts_with("candle") {
                                    continue;
                                }

                                let inst = match v["arg"]["instId"].as_str() {
                                    Some(s) => s,
                                    None => continue,
                                };
                                let (ticker_info, timeframe) =
                                    match lookup.get(&(channel.to_string(), inst.to_string())) {
                                        Some(t) => *t,
                                        None => continue,
                                    };
                                let qty_norm = QtyNormalization::with_raw_qty_unit(
                                    size_in_quote_ccy,
                                    ticker_info,
                                    raw_qty_unit_from_market_type(market_type),
                                );

                                if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
                                    for row in data {
                                        let time = row.get(0).and_then(serde_util::value_as_u64);
                                        let open = row.get(1).and_then(serde_util::value_as_f32);
                                        let high = row.get(2).and_then(serde_util::value_as_f32);
                                        let low = row.get(3).and_then(serde_util::value_as_f32);
                                        let close = row.get(4).and_then(serde_util::value_as_f32);
                                        let volume = row.get(5).and_then(serde_util::value_as_f32);

                                        let (ts, open, high, low, close) =
                                            match (time, open, high, low, close) {
                                                (
                                                    Some(ts),
                                                    Some(open),
                                                    Some(high),
                                                    Some(low),
                                                    Some(close),
                                                ) => (ts, open, high, low, close),
                                                _ => continue,
                                            };

                                        let volume_in_display = if let Some(vq) = volume {
                                            qty_norm.normalize_qty(vq, close)
                                        } else {
                                            qty_norm.normalize_qty(0.0, close)
                                        };

                                        let kline = Kline::new(
                                            ts,
                                            open,
                                            high,
                                            low,
                                            close,
                                            Volume::TotalOnly(volume_in_display),
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
