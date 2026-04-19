use crate::{
    Event, Kline, Price, PushFrequency, TickMultiplier, Ticker, TickerInfo, Timeframe, Trade,
    Volume,
    adapter::connect::{State, channel, connect_ws},
    adapter::{MarketKind, StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    HyperliquidHandle, WS_DOMAIN, exchange_from_market_type, raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use fastwebsockets::{FragmentCollector, Frame, OpCode};
use futures::{SinkExt, Stream};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const SIG_FIG_LIMIT: i32 = 5;
const ALLOWED_MANTISSA: [i32; 3] = [1, 2, 5];

#[derive(Clone, Copy, Debug)]
struct DepthFeedConfig {
    pub n_sig_figs: Option<i32>,
    pub mantissa: Option<i32>,
}

impl DepthFeedConfig {
    fn new(n_sig_figs: Option<i32>, mantissa: Option<i32>) -> Self {
        Self {
            n_sig_figs,
            mantissa,
        }
    }

    fn full_precision() -> Self {
        Self {
            n_sig_figs: None,
            mantissa: None,
        }
    }
}

fn snap_multiplier_to_125(multiplier: u16) -> (i32, i32) {
    const SQRT2: f32 = std::f32::consts::SQRT_2;
    const SQRT10: f32 = 3.162_277_7;
    const SQRT50: f32 = 7.071_068;

    let m = (multiplier as f32).max(1.0);
    let mut kf = m.log10().floor();
    let rem = m / 10_f32.powf(kf);

    let (mantissa, bump) = if rem < SQRT2 {
        (1, false)
    } else if rem < SQRT10 {
        (2, false)
    } else if rem < SQRT50 {
        (5, false)
    } else {
        (1, true)
    };

    if bump {
        kf += 1.0;
    }

    (kf as i32, mantissa)
}

fn config_from_multiplier(price: f32, multiplier: u16) -> DepthFeedConfig {
    if price <= 0.0 {
        return DepthFeedConfig::full_precision();
    }
    if multiplier <= 1 {
        return DepthFeedConfig::full_precision();
    }

    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };

    let (k, m125) = snap_multiplier_to_125(multiplier);
    let n = if int_digits > SIG_FIG_LIMIT {
        (int_digits - k).clamp(2, SIG_FIG_LIMIT)
    } else {
        (SIG_FIG_LIMIT - k).clamp(2, SIG_FIG_LIMIT)
    };

    let mantissa = if n == SIG_FIG_LIMIT && (m125 == 2 || m125 == 5) {
        Some(m125)
    } else {
        None
    };

    DepthFeedConfig::new(Some(n), mantissa)
}

#[derive(Debug, Deserialize)]
struct HyperliquidDepth {
    levels: [Vec<HyperliquidLevel>; 2],
    time: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidLevel {
    #[serde(deserialize_with = "de_string_to_number")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    sz: f32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidTrade {
    coin: String,
    side: String,
    #[serde(deserialize_with = "de_string_to_number")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    sz: f32,
    time: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "i")]
    interval: String,
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
}

enum StreamData {
    Trade(Vec<HyperliquidTrade>),
    Depth(HyperliquidDepth),
    Kline(HyperliquidKline),
}

async fn connect_websocket(
    domain: &str,
    path: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    connect_ws(domain, &url, proxy_cfg).await
}

fn parse_websocket_message(payload: &[u8]) -> Result<StreamData, AdapterError> {
    let json: Value =
        serde_json::from_slice(payload).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let channel = json
        .get("channel")
        .and_then(|c| c.as_str())
        .ok_or_else(|| AdapterError::ParseError("Missing channel".to_string()))?;

    match channel {
        "trades" => {
            let trades: Vec<HyperliquidTrade> = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Trade(trades))
        }
        "l2Book" => {
            let depth: HyperliquidDepth = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Depth(depth))
        }
        "candle" => {
            let kline: HyperliquidKline = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Kline(kline))
        }
        _ => Err(AdapterError::ParseError(format!(
            "Unknown channel: {}",
            channel
        ))),
    }
}

pub fn connect_depth_stream(
    handle: HyperliquidHandle,
    ticker_info: TickerInfo,
    tick_multiplier: Option<TickMultiplier>,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let ticker = ticker_info.ticker;
        let exchange = ticker.exchange;

        let mut local_depth_cache = LocalDepthCache::default();
        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(ticker_info.market_type()),
        );
        let user_multiplier = tick_multiplier.unwrap_or(TickMultiplier(1)).0;

        let (symbol_str, _) = ticker.to_full_symbol_and_type();

        loop {
            match &mut state {
                State::Disconnected => {
                    let snapshot = match handle.fetch_depth_snapshot(ticker).await {
                        Ok(snapshot) => snapshot,
                        Err(e) => {
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    format!("Failed to fetch depth snapshot: {e}"),
                                ))
                                .await;
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    };

                    let Some(best_bid_price) = snapshot.bids.first().map(|o| o.price) else {
                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                "Depth snapshot missing bids".to_string(),
                            ))
                            .await;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    };

                    let depth_cfg = config_from_multiplier(best_bid_price, user_multiplier);
                    let snapshot_time = snapshot.time;

                    local_depth_cache.update_with_qty_norm(
                        DepthUpdate::Snapshot(snapshot),
                        ticker_info.min_ticksize,
                        Some(qty_norm),
                    );

                    match connect_websocket(WS_DOMAIN, "/ws", proxy_cfg.as_ref()).await {
                        Ok(mut websocket) => {
                            let mut depth_subscription = json!({
                                "method": "subscribe",
                                "subscription": {
                                    "type": "l2Book",
                                    "coin": symbol_str,
                                }
                            });

                            if let Some(n) = depth_cfg.n_sig_figs {
                                depth_subscription["subscription"]["nSigFigs"] = json!(n);
                            }
                            if let (Some(m), Some(5)) = (depth_cfg.mantissa, depth_cfg.n_sig_figs)
                                && m != 1
                                && ALLOWED_MANTISSA.contains(&m)
                            {
                                depth_subscription["subscription"]["mantissa"] = json!(m);
                            }

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

                            state = State::Connected(websocket);
                            let _ = output.send(Event::Connected(exchange)).await;

                            let _ = output
                                .send(Event::DepthReceived(
                                    StreamKind::Depth {
                                        ticker_info,
                                        depth_aggr: StreamTicksize::ServerSide(TickMultiplier(
                                            user_multiplier,
                                        )),
                                        push_freq,
                                    },
                                    snapshot_time,
                                    local_depth_cache.depth.clone(),
                                ))
                                .await;
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
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(stream_data) = parse_websocket_message(&msg.payload) {
                                match stream_data {
                                    StreamData::Depth(depth) => {
                                        let bids = depth.levels[0]
                                            .iter()
                                            .map(|level| DeOrder {
                                                price: level.px,
                                                qty: level.sz,
                                            })
                                            .collect();
                                        let asks = depth.levels[1]
                                            .iter()
                                            .map(|level| DeOrder {
                                                price: level.px,
                                                qty: level.sz,
                                            })
                                            .collect();

                                        let depth_payload = DepthPayload {
                                            last_update_id: depth.time,
                                            time: depth.time,
                                            bids,
                                            asks,
                                        };
                                        local_depth_cache.update_with_qty_norm(
                                            DepthUpdate::Snapshot(depth_payload),
                                            ticker_info.min_ticksize,
                                            Some(qty_norm),
                                        );

                                        let stream_kind = StreamKind::Depth {
                                            ticker_info,
                                            depth_aggr: StreamTicksize::ServerSide(TickMultiplier(
                                                user_multiplier,
                                            )),
                                            push_freq,
                                        };

                                        let _ = output
                                            .send(Event::DepthReceived(
                                                stream_kind,
                                                depth.time,
                                                local_depth_cache.depth.clone(),
                                            ))
                                            .await;
                                    }
                                    StreamData::Trade(_) => {}
                                    StreamData::Kline(_) => {}
                                }
                            }
                        }
                        OpCode::Close => {
                            state = State::Disconnected;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "WebSocket closed".to_string(),
                                ))
                                .await;
                        }
                        OpCode::Ping => {
                            let _ = websocket.write_frame(Frame::pong(msg.payload)).await;
                        }
                        _ => {}
                    },
                    Err(e) => {
                        state = State::Disconnected;
                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                format!("WebSocket error: {}", e),
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
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

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
                            raw_qty_unit_from_market_type(ticker_info.market_type()),
                        ),
                    ),
                )
            })
            .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let symbol_to_ticker = tickers
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
                    match connect_websocket(WS_DOMAIN, "/ws", proxy_cfg.as_ref()).await {
                        Ok(mut websocket) => {
                            let mut subscribe_ok = true;
                            for ticker_info in &tickers {
                                let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();

                                let trades_subscribe_msg = json!({
                                    "method": "subscribe",
                                    "subscription": {
                                        "type": "trades",
                                        "coin": symbol_str
                                    }
                                });

                                if websocket
                                    .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                        trades_subscribe_msg.to_string().as_bytes(),
                                    )))
                                    .await
                                    .is_err()
                                {
                                    subscribe_ok = false;
                                    break;
                                }
                            }

                            if !subscribe_ok {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }

                            state = State::Connected(websocket);
                            last_flush = tokio::time::Instant::now();
                            let _ = output.send(Event::Connected(exchange)).await;
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
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Trade(trades)) =
                                parse_websocket_message(&msg.payload)
                            {
                                for hl_trade in trades {
                                    if let Some(ticker) = symbol_to_ticker.get(&hl_trade.coin)
                                        && let Some((ticker_info, qty_norm)) =
                                            ticker_info_map.get(ticker)
                                    {
                                        let ticker_info = *ticker_info;
                                        let price = Price::from_f32(hl_trade.px)
                                            .round_to_min_tick(ticker_info.min_ticksize);

                                        let trade = Trade {
                                            time: hl_trade.time,
                                            is_sell: hl_trade.side == "A",
                                            price,
                                            qty: qty_norm.normalize_qty(hl_trade.sz, hl_trade.px),
                                        };
                                        trades_buffer_map.entry(*ticker).or_default().push(trade);
                                    } else {
                                        log::error!(
                                            "Ticker info not found for Hyperliquid coin: {}",
                                            hl_trade.coin
                                        );
                                    }
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
                                    "WebSocket closed".to_string(),
                                ))
                                .await;
                        }
                        OpCode::Ping => {
                            let _ = websocket.write_frame(Frame::pong(msg.payload)).await;
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
                                format!("WebSocket error: {}", e),
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
    _market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let exchange = streams
            .first()
            .map(|(t, _)| t.exchange())
            .unwrap_or(exchange_from_market_type(MarketKind::LinearPerps));

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(WS_DOMAIN, "/ws", proxy_cfg.as_ref()).await {
                        Ok(mut websocket) => {
                            for (ticker_info, timeframe) in &streams {
                                let ticker = ticker_info.ticker;
                                let interval = timeframe.to_string();

                                let (symbol_str, _) = ticker.to_full_symbol_and_type();
                                let subscribe_msg = json!({
                                    "method": "subscribe",
                                    "subscription": {
                                        "type": "candle",
                                        "coin": symbol_str,
                                        "interval": interval
                                    }
                                });

                                if (websocket
                                    .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                        subscribe_msg.to_string().as_bytes(),
                                    )))
                                    .await)
                                    .is_err()
                                {
                                    break;
                                }
                            }

                            state = State::Connected(websocket);
                            let _ = output.send(Event::Connected(exchange)).await;
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
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Kline(hl_kline)) =
                                parse_websocket_message(&msg.payload)
                                && let Some((ticker_info, timeframe)) =
                                    streams.iter().find(|(t, tf)| {
                                        t.ticker.as_str() == hl_kline.symbol
                                            && tf.to_string() == hl_kline.interval.as_str()
                                    })
                            {
                                let qty_norm = QtyNormalization::with_raw_qty_unit(
                                    size_in_quote_ccy,
                                    *ticker_info,
                                    raw_qty_unit_from_market_type(ticker_info.market_type()),
                                );
                                let volume =
                                    qty_norm.normalize_qty(hl_kline.volume, hl_kline.close);

                                let kline = Kline::new(
                                    hl_kline.time,
                                    hl_kline.open,
                                    hl_kline.high,
                                    hl_kline.low,
                                    hl_kline.close,
                                    Volume::TotalOnly(volume),
                                    ticker_info.min_ticksize,
                                );

                                let stream_kind = StreamKind::Kline {
                                    ticker_info: *ticker_info,
                                    timeframe: *timeframe,
                                };
                                let _ = output.send(Event::KlineReceived(stream_kind, kline)).await;
                            }
                        }
                        OpCode::Close => {
                            state = State::Disconnected;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "WebSocket closed".to_string(),
                                ))
                                .await;
                        }
                        OpCode::Ping => {
                            let _ = websocket.write_frame(Frame::pong(msg.payload)).await;
                        }
                        _ => {}
                    },
                    Err(e) => {
                        state = State::Disconnected;
                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                format!("WebSocket error: {}", e),
                            ))
                            .await;
                    }
                },
            }
        }
    })
}
