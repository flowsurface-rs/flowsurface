use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, Price, PushFrequency, Qty, Ticker, TickerInfo,
        TickerStats, Timeframe, Trade, Volume,
        adapter::{StreamKind, StreamTicksize, TRADE_BUCKET_INTERVAL, flush_trade_buffers},
        connect::{State, channel, connect_ws},
        depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
        limiter::{self, RateLimiter},
        serde_util,
        serde_util::de_string_to_number,
        unit::qty::{QtyNormalization, RawQtyUnit, SizeUnit, volume_size_unit},
    },
    AdapterError, Event,
};

use fastwebsockets::{Frame, OpCode};
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock, time::Duration};
use tokio::sync::Mutex;

const WS_DOMAIN: &str = "ws.okx.com";
const REST_API_BASE: &str = "https://www.okx.com/api/v5";

const LIMIT: usize = 20;

const REFILL_RATE: Duration = Duration::from_secs(2);
const LIMITER_BUFFER_PCT: f32 = 0.05;

static OKEX_LIMITER: LazyLock<Mutex<OkexLimiter>> =
    LazyLock::new(|| Mutex::new(OkexLimiter::new(LIMIT, REFILL_RATE)));

pub struct OkexLimiter {
    bucket: limiter::FixedWindowBucket,
}

impl OkexLimiter {
    pub fn new(limit: usize, refill_rate: Duration) -> Self {
        let effective_limit = (limit as f32 * (1.0 - LIMITER_BUFFER_PCT)) as usize;
        Self {
            bucket: limiter::FixedWindowBucket::new(effective_limit, refill_rate),
        }
    }
}

impl RateLimiter for OkexLimiter {
    fn prepare_request(&mut self, weight: usize) -> Option<Duration> {
        self.bucket.calculate_wait_time(weight)
    }

    fn update_from_response(&mut self, _response: &reqwest::Response, weight: usize) {
        self.bucket.consume_tokens(weight);
    }

    fn should_exit_on_response(&self, response: &reqwest::Response) -> bool {
        response.status() == 429
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot => RawQtyUnit::Base,
        MarketKind::LinearPerps | MarketKind::InversePerps => RawQtyUnit::Contracts,
    }
}

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
    exchange: Exchange,
    output: &mut mpsc::Sender<Event>,
    topic: &str,
) -> State {
    let url = format!("wss://{WS_DOMAIN}/ws/v5/{topic}");

    match connect_ws(WS_DOMAIN, &url).await {
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
                    state = try_connect(&subscribe_message, exchange, &mut output, "public").await;
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
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let exchange = match market_type {
            MarketKind::Spot => Exchange::OkexSpot,
            MarketKind::LinearPerps => Exchange::OkexLinear,
            MarketKind::InversePerps => Exchange::OkexInverse,
        };

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
                    state = try_connect(&subscribe_message, exchange, &mut output, "public").await;
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
            .unwrap_or_else(|| Exchange::OkexSpot);

        let subscribe_message = serde_json::json!({
            "op": "subscribe",
            "args": args,
        });

        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        loop {
            match &mut state {
                State::Disconnected => {
                    state =
                        try_connect(&subscribe_message, exchange, &mut output, "business").await;
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

fn exchange_from_market_type(market_type: MarketKind) -> Exchange {
    match market_type {
        MarketKind::Spot => Exchange::OkexSpot,
        MarketKind::LinearPerps => Exchange::OkexLinear,
        MarketKind::InversePerps => Exchange::OkexInverse,
    }
}

fn timeframe_to_okx_bar(tf: Timeframe) -> Option<&'static str> {
    Some(match tf {
        Timeframe::M1 => "1m",
        Timeframe::M3 => "3m",
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::M30 => "30m",
        Timeframe::H1 => "1H",
        Timeframe::H2 => "2H",
        Timeframe::H4 => "4H",
        Timeframe::H12 => "12Hutc",
        Timeframe::D1 => "1Dutc",
        _ => return None,
    })
}

pub async fn fetch_ticker_metadata(
    markets: &[MarketKind],
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let mut map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let url = format!("{REST_API_BASE}/public/instruments?instType=SPOT");

        let response_text = limiter::http_request(&url, None, None).await?;
        let doc: Value = serde_json::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let list = doc["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        let exchange = Exchange::OkexSpot;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if item["state"].as_str().unwrap_or("") != "live" {
                continue;
            }

            if item["quoteCcy"].as_str() != Some("USDT") {
                continue;
            }

            if !exchange.is_symbol_supported(symbol, true) {
                continue;
            }

            let min_ticksize = serde_util::value_as_f32(&item["tickSz"])
                .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;
            let min_qty = serde_util::value_as_f32(&item["lotSz"])
                .ok_or_else(|| AdapterError::ParseError("Lot size not found".to_string()))?;

            let ticker = Ticker::new(symbol, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, None);

            map.insert(ticker, Some(info));
        }
    }

    if include_perps {
        let url = format!("{REST_API_BASE}/public/instruments?instType=SWAP");

        let response_text = limiter::http_request(&url, None, None).await?;
        let doc: Value = serde_json::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let list = doc["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if item["state"].as_str().unwrap_or("") != "live" {
                continue;
            }

            let market_kind = match item["ctType"].as_str() {
                Some("linear") => {
                    if item["settleCcy"].as_str() != Some("USDT") {
                        continue;
                    }
                    MarketKind::LinearPerps
                }
                Some("inverse") => MarketKind::InversePerps,
                _ => continue,
            };

            if !markets.contains(&market_kind) {
                continue;
            }

            let exchange = exchange_from_market_type(market_kind);

            if !exchange.is_symbol_supported(symbol, true) {
                continue;
            }

            let min_ticksize = serde_util::value_as_f32(&item["tickSz"])
                .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;
            let min_qty = serde_util::value_as_f32(&item["lotSz"])
                .ok_or_else(|| AdapterError::ParseError("Lot size not found".to_string()))?;
            let contract_size = serde_util::value_as_f32(&item["ctVal"]);

            let ticker = Ticker::new(symbol, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, contract_size);

            map.insert(ticker, Some(info));
        }
    }

    Ok(map)
}

pub async fn fetch_ticker_stats(
    markets: &[MarketKind],
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let mut map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps =
        markets.contains(&MarketKind::LinearPerps) || markets.contains(&MarketKind::InversePerps);

    if include_spot {
        let url = format!("{REST_API_BASE}/market/tickers?instType=SPOT");

        let parsed_response: Value =
            limiter::http_parse_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

        let list = parsed_response["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        let exchange = Exchange::OkexSpot;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            let last_trade_price = serde_util::value_as_f32(&item["last"]);
            let open24h = serde_util::value_as_f32(&item["open24h"]);
            let Some(vol24h) = serde_util::value_as_f32(&item["volCcy24h"]) else {
                continue;
            };

            let (last_price, previous_daily_open) =
                if let (Some(last), Some(previous_daily_open)) = (last_trade_price, open24h) {
                    (last, previous_daily_open)
                } else {
                    continue;
                };

            let daily_price_chg = if previous_daily_open > 0.0 {
                (last_price - previous_daily_open) / previous_daily_open * 100.0
            } else {
                0.0
            };

            map.insert(
                Ticker::new(symbol, exchange),
                TickerStats {
                    mark_price: Price::from_f32(last_price),
                    daily_price_chg,
                    daily_volume: Qty::from_f32(vol24h),
                },
            );
        }
    }

    if include_perps {
        let url = format!("{REST_API_BASE}/market/tickers?instType=SWAP");

        let parsed_response: Value =
            limiter::http_parse_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

        let list = parsed_response["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            let perps_market_symbol = if symbol.ends_with("-USDT-SWAP") {
                Some(MarketKind::LinearPerps)
            } else if symbol.ends_with("-USD-SWAP") {
                Some(MarketKind::InversePerps)
            } else {
                None
            };

            let Some(perps_market) = perps_market_symbol else {
                continue;
            };

            if !markets.contains(&perps_market) {
                continue;
            }

            let exchange = exchange_from_market_type(perps_market);

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            let last_trade_price = serde_util::value_as_f32(&item["last"]);
            let open24h = serde_util::value_as_f32(&item["open24h"]);

            let Some(vol24h) = serde_util::value_as_f32(&item["volCcy24h"]) else {
                continue;
            };

            let (last_price, previous_daily_open) =
                if let (Some(last), Some(previous_daily_open)) = (last_trade_price, open24h) {
                    (last, previous_daily_open)
                } else {
                    continue;
                };
            let daily_price_chg = if previous_daily_open > 0.0 {
                (last_price - previous_daily_open) / previous_daily_open * 100.0
            } else {
                0.0
            };

            map.insert(
                Ticker::new(symbol, exchange),
                TickerStats {
                    mark_price: Price::from_f32(last_price),
                    daily_price_chg,
                    daily_volume: Qty::from_f32(vol24h * last_price),
                },
            );
        }
    }

    Ok(map)
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let bar = timeframe_to_okx_bar(timeframe).ok_or_else(|| {
        AdapterError::InvalidRequest(format!("Unsupported timeframe: {timeframe}"))
    })?;

    let mut url = format!(
        "{REST_API_BASE}/market/history-candles?instId={}&bar={}&limit={}",
        symbol_str,
        bar,
        match range {
            Some((start, end)) => {
                ((end - start) / timeframe.to_milliseconds()).clamp(1, 300)
            }
            None => 300,
        }
    );

    if let Some((start, end)) = range {
        url.push_str(&format!("&before={start}&after={end}"));
    }

    let doc: Value = limiter::http_parse_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Kline result is not an array".to_string()))?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let mut klines: Vec<Kline> = Vec::with_capacity(list.len());

    for row in list {
        let time = row.get(0).and_then(serde_util::value_as_u64);
        let open = row.get(1).and_then(serde_util::value_as_f32);
        let high = row.get(2).and_then(serde_util::value_as_f32);
        let low = row.get(3).and_then(serde_util::value_as_f32);
        let close = row.get(4).and_then(serde_util::value_as_f32);
        let volume = row.get(5).and_then(serde_util::value_as_f32);

        let (ts, open, high, low, close) = match (time, open, high, low, close) {
            (Some(ts), Some(o), Some(h), Some(l), Some(c)) => (ts, o, h, l, c),
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

        klines.push(kline);
    }

    klines.sort_by_key(|k| k.time);
    Ok(klines)
}

pub async fn fetch_historical_oi(
    ticker_info: TickerInfo,
    range: Option<(u64, u64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let (ticker_str, _market) = ticker_info.ticker.to_full_symbol_and_type();

    let bar = timeframe_to_okx_bar(period)
        .ok_or_else(|| AdapterError::InvalidRequest(format!("Unsupported timeframe: {period}")))?;

    let mut url = format!(
        "{REST_API_BASE}/rubik/stat/contracts/open-interest-history?instId={ticker_str}&period={bar}"
    );

    if let Some((start, end)) = range {
        url.push_str(&format!("&begin={start}&end={end}"));
    }

    let response_text =
        limiter::http_request_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

    let doc: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Fetch result is not an array".to_string()))?;

    // data = [ [ts, oi, oiCcy, oiUsd], ... ]
    let open_interest: Vec<OpenInterest> = list
        .iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            let ts = serde_util::value_as_u64(arr.first()?)?;
            let oi_ccy = serde_util::value_as_f32(arr.get(2)?)?;
            Some(OpenInterest {
                time: ts,
                value: oi_ccy,
            })
        })
        .collect();

    Ok(open_interest)
}
