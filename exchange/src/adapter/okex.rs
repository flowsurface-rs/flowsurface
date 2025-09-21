use crate::{
    Price, SIZE_IN_QUOTE_CURRENCY,
    adapter::{StreamKind, StreamTicksize},
    limiter::{self, RateLimiter},
};

use super::{
    super::{
        Exchange, Kline, MarketKind, Ticker, TickerInfo, TickerStats, Timeframe, Trade,
        connect::{State, connect_ws},
        de_string_to_f32, de_string_to_u64, is_symbol_supported,
        limiter::HTTP_CLIENT,
    },
    AdapterError, Event,
};

use super::super::depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache};

use fastwebsockets::{Frame, OpCode};
use iced_futures::{
    futures::{SinkExt, Stream, channel::mpsc},
    stream,
};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock, time::Duration};
use tokio::sync::Mutex;

const WS_DOMAIN: &str = "ws.okx.com";

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

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "ts", deserialize_with = "de_string_to_u64")]
    pub time: u64,
    #[serde(rename = "px", deserialize_with = "de_string_to_f32")]
    pub price: f32,
    #[serde(rename = "sz", deserialize_with = "de_string_to_f32")]
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
    Trade(Vec<SonicTrade>),
    Depth(SonicDepth, String, u64),
}

fn feed_de(slice: &[u8], _ticker: Ticker) -> Result<StreamData, AdapterError> {
    let v: Value =
        serde_json::from_slice(slice).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let mut channel = String::new();
    if let Some(arg) = v.get("arg")
        && let Some(ch) = arg.get("channel").and_then(|c| c.as_str())
    {
        channel = ch.to_string();
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
            .and_then(|t| t.as_str())
            .and_then(|s| s.parse::<u64>().ok())
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
            return Ok(StreamData::Trade(trades));
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

pub fn connect_market_stream(ticker_info: TickerInfo) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state: State = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
        let exchange = ticker.exchange;

        let subscribe_message = serde_json::json!({
            "op": "subscribe",
            "args": [
                { "channel": "trades", "instId": symbol_str },
                { "channel": "books",  "instId": symbol_str },
            ],
        });

        let mut trades_buffer: Vec<Trade> = vec![];
        let mut orderbook = LocalDepthCache::default();

        let size_in_quote = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);
        let contract_size = ticker_info.contract_size.map(f32::from);

        loop {
            match &mut state {
                State::Disconnected => {
                    state = try_connect(&subscribe_message, exchange, &mut output, "public").await;
                }
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(data) = feed_de(&msg.payload[..], ticker) {
                                match data {
                                    StreamData::Trade(de_trade_vec) => {
                                        for de_trade in &de_trade_vec {
                                            let price = Price::from_f32(de_trade.price)
                                                .round_to_min_tick(ticker_info.min_ticksize);
                                            let qty = calc_qty(
                                                de_trade.qty,
                                                de_trade.price,
                                                size_in_quote,
                                                contract_size,
                                                market_type,
                                            );

                                            let trade = Trade {
                                                time: de_trade.time,
                                                is_sell: de_trade.is_sell == "sell"
                                                    || de_trade.is_sell == "SELL",
                                                price,
                                                qty,
                                            };
                                            trades_buffer.push(trade);
                                        }
                                    }
                                    StreamData::Depth(de_depth, data_type, time) => {
                                        let depth = DepthPayload {
                                            last_update_id: de_depth.update_id,
                                            time,
                                            bids: de_depth
                                                .bids
                                                .iter()
                                                .map(|x| DeOrder {
                                                    price: x.price,
                                                    qty: calc_qty(
                                                        x.qty,
                                                        x.price,
                                                        size_in_quote,
                                                        contract_size,
                                                        market_type,
                                                    ),
                                                })
                                                .collect(),
                                            asks: de_depth
                                                .asks
                                                .iter()
                                                .map(|x| DeOrder {
                                                    price: x.price,
                                                    qty: calc_qty(
                                                        x.qty,
                                                        x.price,
                                                        size_in_quote,
                                                        contract_size,
                                                        market_type,
                                                    ),
                                                })
                                                .collect(),
                                        };

                                        if (data_type == "snapshot") || (depth.last_update_id == 1)
                                        {
                                            orderbook.update(
                                                DepthUpdate::Snapshot(depth),
                                                ticker_info.min_ticksize,
                                            );
                                        } else if data_type == "delta" {
                                            orderbook.update(
                                                DepthUpdate::Diff(depth),
                                                ticker_info.min_ticksize,
                                            );

                                            let _ = output
                                                .send(Event::DepthReceived(
                                                    StreamKind::DepthAndTrades {
                                                        ticker_info,
                                                        depth_aggr: StreamTicksize::Client,
                                                    },
                                                    time,
                                                    orderbook.depth.clone(),
                                                    std::mem::take(&mut trades_buffer)
                                                        .into_boxed_slice(),
                                                ))
                                                .await;
                                        }
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

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let mut args: Vec<Value> = Vec::with_capacity(streams.len());
        let mut lookup: HashMap<String, (TickerInfo, Timeframe)> = HashMap::new();
        for (ticker_info, timeframe) in &streams {
            let ticker = ticker_info.ticker;

            if let Some(bar) = timeframe_to_okx_bar(*timeframe) {
                let (symbol, _mt) = ticker.to_full_symbol_and_type();
                args.push(serde_json::json!({
                    "channel": format!("candle{bar}"),
                    "instId": symbol,
                }));
                lookup.insert(symbol, (*ticker_info, *timeframe));
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

        let size_in_quote = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

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
                                let (ticker_info, timeframe) = match lookup.get(inst) {
                                    Some(t) => *t,
                                    None => continue,
                                };
                                let contract_size = ticker_info.contract_size.map(f32::from);

                                if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
                                    for row in data {
                                        let time = row
                                            .get(0)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<u64>().ok());
                                        let open = row
                                            .get(1)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<f32>().ok());
                                        let high = row
                                            .get(2)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<f32>().ok());
                                        let low = row
                                            .get(3)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<f32>().ok());
                                        let close = row
                                            .get(4)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<f32>().ok());
                                        let volume = row
                                            .get(5)
                                            .and_then(|x| x.as_str())
                                            .and_then(|s| s.parse::<f32>().ok());

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
                                            calc_qty(
                                                vq,
                                                close,
                                                size_in_quote,
                                                contract_size,
                                                market_type,
                                            )
                                        } else {
                                            0.0
                                        };

                                        let kline = Kline::new(
                                            ts,
                                            open,
                                            high,
                                            low,
                                            close,
                                            (-1.0, volume_in_display),
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

fn calc_qty(
    qty: f32,
    price: f32,
    size_in_quote_currency: bool,
    contract_size: Option<f32>,
    market: MarketKind,
) -> f32 {
    let is_inverse = matches!(market, MarketKind::InversePerps);

    match contract_size {
        Some(cs) => {
            if is_inverse {
                if size_in_quote_currency {
                    qty * cs
                } else {
                    qty
                }
            } else if size_in_quote_currency {
                qty * cs * price
            } else {
                qty * cs
            }
        }
        None => {
            if size_in_quote_currency {
                qty * price
            } else {
                qty
            }
        }
    }
}

fn okx_inst_type(m: MarketKind) -> &'static str {
    match m {
        MarketKind::Spot => "SPOT",
        MarketKind::LinearPerps | MarketKind::InversePerps => "SWAP",
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
        Timeframe::H6 => "6H",
        Timeframe::H12 => "12H",
        Timeframe::D1 => "1D",
        _ => return None,
    })
}

pub async fn fetch_ticksize(
    market_type: MarketKind,
) -> Result<std::collections::HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let inst_type = okx_inst_type(market_type);
    let url = format!(
        "https://www.okx.com/api/v5/public/instruments?instType={}",
        inst_type
    );

    let response_text = HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(AdapterError::FetchError)?
        .text()
        .await
        .map_err(AdapterError::FetchError)?;

    let doc: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let exchange = match market_type {
        MarketKind::Spot => Exchange::OkexSpot,
        MarketKind::LinearPerps => Exchange::OkexLinear,
        MarketKind::InversePerps => Exchange::OkexInverse,
    };

    let mut map = std::collections::HashMap::new();

    for item in list {
        let symbol = match item["instId"].as_str() {
            Some(s) => s,
            None => continue,
        };

        if item["state"].as_str().unwrap_or("") != "live" {
            continue;
        }

        let accept = match market_type {
            MarketKind::Spot => item["quoteCcy"].as_str() == Some("USDT"),
            MarketKind::LinearPerps => {
                item["ctType"].as_str() == Some("linear")
                    && (item["settleCcy"].as_str() == Some("USDT"))
            }
            MarketKind::InversePerps => item["ctType"].as_str() == Some("inverse"),
        };
        if !accept {
            continue;
        }

        if !is_symbol_supported(symbol, exchange, true) {
            continue;
        }

        let min_ticksize = item["tickSz"]
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
            .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;
        let min_qty = item["lotSz"]
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
            .ok_or_else(|| AdapterError::ParseError("Lot size not found".to_string()))?;
        let contract_size = if market_type == MarketKind::Spot {
            None
        } else {
            item["ctVal"].as_str().and_then(|s| s.parse::<f32>().ok())
        };

        let ticker = Ticker::new(symbol, exchange);
        let info = TickerInfo::new(ticker, min_ticksize, min_qty, contract_size);

        map.insert(ticker, Some(info));
    }

    Ok(map)
}

pub async fn fetch_ticker_prices(
    market_type: MarketKind,
) -> Result<std::collections::HashMap<Ticker, TickerStats>, AdapterError> {
    let inst_type = okx_inst_type(market_type);
    let url = format!(
        "https://www.okx.com/api/v5/market/tickers?instType={}",
        inst_type
    );

    let response_text =
        limiter::http_request_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

    let doc: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let exchange = match market_type {
        MarketKind::Spot => Exchange::OkexSpot,
        MarketKind::LinearPerps => Exchange::OkexLinear,
        MarketKind::InversePerps => Exchange::OkexInverse,
    };

    let mut map = std::collections::HashMap::new();

    for item in list {
        let symbol = match item["instId"].as_str() {
            Some(s) => s,
            None => continue,
        };

        if !is_symbol_supported(symbol, exchange, false) {
            continue;
        }

        let last_trade_price = item["last"].as_str().and_then(|s| s.parse::<f32>().ok());
        let open24h = item["open24h"].as_str().and_then(|s| s.parse::<f32>().ok());

        let Some(vol24h) = item["volCcy24h"]
            .as_str()
            .and_then(|s| s.parse::<f32>().ok())
        else {
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

        let volume_usd =
            if market_type == MarketKind::LinearPerps || market_type == MarketKind::InversePerps {
                vol24h * last_price
            } else {
                vol24h
            };

        map.insert(
            Ticker::new(symbol, exchange),
            TickerStats {
                mark_price: last_price,
                daily_price_chg,
                daily_volume: volume_usd,
            },
        );
    }

    Ok(map)
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market) = ticker.to_full_symbol_and_type();
    let contract_size = ticker_info.contract_size.map(f32::from);

    let bar = timeframe_to_okx_bar(timeframe).ok_or_else(|| {
        AdapterError::InvalidRequest(format!("Unsupported timeframe: {timeframe}"))
    })?;

    let mut url = format!(
        "https://www.okx.com/api/v5/market/history-candles?instId={}&bar={}&limit={}",
        symbol_str,
        bar,
        match range {
            Some((start, end)) => {
                ((end - start) / timeframe.to_milliseconds()).clamp(1, 1000)
            }
            None => 200,
        }
    );

    if let Some((start, end)) = range {
        url.push_str(&format!("&before={start}&after={end}"));
    }

    let response_text =
        limiter::http_request_with_limiter(&url, &OKEX_LIMITER, 1, None, None).await?;

    let doc: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Kline result is not an array".to_string()))?;

    let size_in_quote = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

    let mut klines: Vec<Kline> = Vec::with_capacity(list.len());

    for row in list {
        let time = row
            .get(0)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());
        let open = row
            .get(1)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok());
        let high = row
            .get(2)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok());
        let low = row
            .get(3)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok());
        let close = row
            .get(4)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok());
        let volume = row
            .get(5)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok());

        let (ts, open, high, low, close) = match (time, open, high, low, close) {
            (Some(ts), Some(o), Some(h), Some(l), Some(c)) => (ts, o, h, l, c),
            _ => continue,
        };
        let volume_in_display = if let Some(vq) = volume {
            calc_qty(vq, close, size_in_quote, contract_size, market)
        } else {
            0.0
        };

        let kline = Kline::new(
            ts,
            open,
            high,
            low,
            close,
            (-1.0, volume_in_display),
            ticker_info.min_ticksize,
        );

        klines.push(kline);
    }

    klines.sort_by_key(|k| k.time);
    Ok(klines)
}
