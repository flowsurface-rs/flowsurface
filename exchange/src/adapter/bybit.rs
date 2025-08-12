use crate::adapter::StreamTicksize;

use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, SIZE_IN_QUOTE_CURRENCY, StreamKind, Ticker,
        TickerInfo, TickerStats, Timeframe, Trade,
        connect::{State, connect_ws},
        de_string_to_f32, de_string_to_u64,
        depth::{DepthPayload, DepthUpdate, LocalDepthCache, Order},
        is_symbol_supported,
        limiter::{self, http_request_with_limiter},
    },
    AdapterError, Event,
};

use fastwebsockets::{Frame, OpCode};
use iced_futures::{
    futures::{SinkExt, Stream, channel::mpsc},
    stream,
};
use serde_json::{Value, json};
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use tokio::sync::Mutex;

use std::{collections::HashMap, sync::LazyLock, time::Duration};

const WS_DOMAIN: &str = "stream.bybit.com";
const FETCH_DOMAIN: &str = "https://api.bybit.com";

static BYBIT_LIMITER: LazyLock<Mutex<BybitLimiter>> =
    LazyLock::new(|| Mutex::new(BybitLimiter::new(LIMIT, REFILL_RATE)));

const LIMIT: usize = 600;

const REFILL_RATE: Duration = Duration::from_secs(5);
const LIMITER_BUFFER_PCT: f32 = 0.05;

pub struct BybitLimiter {
    bucket: limiter::FixedWindowBucket,
}

impl BybitLimiter {
    pub fn new(limit: usize, refill_rate: Duration) -> Self {
        let effective_limit = (limit as f32 * (1.0 - LIMITER_BUFFER_PCT)) as usize;
        Self {
            bucket: limiter::FixedWindowBucket::new(effective_limit, refill_rate),
        }
    }
}

impl limiter::RateLimiter for BybitLimiter {
    fn prepare_request(&mut self, weight: usize) -> Option<Duration> {
        self.bucket.calculate_wait_time(weight)
    }

    fn update_from_response(&mut self, _response: &reqwest::Response, weight: usize) {
        self.bucket.consume_tokens(weight);
    }

    fn should_exit_on_response(&self, response: &reqwest::Response) -> bool {
        response.status() == 403
    }
}

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::BybitSpot,
        MarketKind::LinearPerps => Exchange::BybitLinear,
        MarketKind::InversePerps => Exchange::BybitInverse,
    }
}

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<Order>,
    #[serde(rename = "a")]
    pub asks: Vec<Order>,
}

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    pub time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_f32")]
    pub price: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_f32")]
    pub qty: f32,
    #[serde(rename = "S")]
    pub is_sell: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SonicKline {
    #[serde(rename = "start")]
    pub time: u64,
    #[serde(rename = "open", deserialize_with = "de_string_to_f32")]
    pub open: f32,
    #[serde(rename = "high", deserialize_with = "de_string_to_f32")]
    pub high: f32,
    #[serde(rename = "low", deserialize_with = "de_string_to_f32")]
    pub low: f32,
    #[serde(rename = "close", deserialize_with = "de_string_to_f32")]
    pub close: f32,
    #[serde(rename = "volume", deserialize_with = "de_string_to_f32")]
    pub volume: f32,
    #[serde(rename = "interval")]
    pub interval: String,
}

enum StreamData {
    Trade(Vec<SonicTrade>),
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
            v.as_str().unwrap().clone_into(&mut data_type);
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade_wrap: Vec<SonicTrade> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Trade(trade_wrap));
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
    output: &mut mpsc::Sender<Event>,
) -> State {
    let exchange = match market_type {
        MarketKind::Spot => Exchange::BybitSpot,
        MarketKind::LinearPerps => Exchange::BybitLinear,
        MarketKind::InversePerps => Exchange::BybitInverse,
    };
    let url = format!(
        "wss://{}/v5/public/{}",
        WS_DOMAIN,
        match market_type {
            MarketKind::Spot => "spot",
            MarketKind::LinearPerps => "linear",
            MarketKind::InversePerps => "inverse",
        }
    );

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

pub fn connect_market_stream(ticker: Ticker) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state: State = State::Disconnected;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market_type);

        let mut trades_buffer: Vec<Trade> = Vec::new();
        let mut orderbook = LocalDepthCache::default();

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream_1 = format!("publicTrade.{symbol_str}");
                    let stream_2 = format!(
                        "orderbook.{}.{}",
                        match market_type {
                            MarketKind::Spot => "200",
                            MarketKind::LinearPerps | MarketKind::InversePerps => "500",
                        },
                        symbol_str,
                    );
                    let subscribe_message = serde_json::json!({
                        "op": "subscribe",
                        "args": [stream_1, stream_2]
                    });

                    state = try_connect(&subscribe_message, market_type, &mut output).await;
                }
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(data) = feed_de(&msg.payload[..], Some(ticker), market_type) {
                                match data {
                                    StreamData::Trade(de_trade_vec) => {
                                        for de_trade in &de_trade_vec {
                                            let trade = Trade {
                                                time: de_trade.time,
                                                is_sell: de_trade.is_sell == "Sell",
                                                price: de_trade.price,
                                                qty: if size_in_quote_currency {
                                                    (de_trade.qty * de_trade.price).round()
                                                } else {
                                                    de_trade.qty
                                                },
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
                                                .map(|x| Order {
                                                    price: x.price,
                                                    qty: if size_in_quote_currency {
                                                        (x.qty * x.price).round()
                                                    } else {
                                                        x.qty
                                                    },
                                                })
                                                .collect(),
                                            asks: de_depth
                                                .asks
                                                .iter()
                                                .map(|x| Order {
                                                    price: x.price,
                                                    qty: if size_in_quote_currency {
                                                        (x.qty * x.price).round()
                                                    } else {
                                                        x.qty
                                                    },
                                                })
                                                .collect(),
                                        };

                                        if (data_type == "snapshot") || (depth.last_update_id == 1)
                                        {
                                            orderbook.update(DepthUpdate::Snapshot(depth));
                                        } else if data_type == "delta" {
                                            orderbook.update(DepthUpdate::Diff(depth));

                                            let _ = output
                                                .send(Event::DepthReceived(
                                                    StreamKind::DepthAndTrades {
                                                        ticker,
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
                                    _ => {
                                        log::warn!("Unknown data received");
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
    streams: Vec<(Ticker, Timeframe)>,
    market_type: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream_str = streams
                        .iter()
                        .map(|(ticker, timeframe)| {
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

                    state = try_connect(&subscribe_message, market_type, &mut output).await;
                }
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
                                feed_de(&msg.payload[..], None, market_type)
                            {
                                for de_kline in &de_kline_vec {
                                    let volume = if SIZE_IN_QUOTE_CURRENCY.get() == Some(&true) {
                                        (de_kline.volume * de_kline.close).round()
                                    } else {
                                        de_kline.volume
                                    };

                                    let kline = Kline {
                                        time: de_kline.time,
                                        open: de_kline.open,
                                        high: de_kline.high,
                                        low: de_kline.low,
                                        close: de_kline.close,
                                        volume: (-1.0, volume),
                                    };

                                    if let Some(timeframe) = string_to_timeframe(&de_kline.interval)
                                    {
                                        let _ = output
                                            .send(Event::KlineReceived(
                                                StreamKind::Kline { ticker, timeframe },
                                                kline,
                                            ))
                                            .await;
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
                },
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

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(rename = "openInterest", deserialize_with = "de_string_to_f32")]
    pub value: f32,
    #[serde(deserialize_with = "de_string_to_u64")]
    pub timestamp: u64,
}

/// # Panics
///
/// Will panic if the `period` is not one of the supported timeframes for open interest
pub async fn fetch_historical_oi(
    ticker: Ticker,
    range: Option<(u64, u64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let ticker_str = ticker.to_full_symbol_and_type().0.to_uppercase();
    let period_str = match period {
        Timeframe::M5 => "5min",
        Timeframe::M15 => "15min",
        Timeframe::M30 => "30min",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        Timeframe::D1 => "1d",
        _ => panic!("Unsupported timeframe for open interest: {period}"),
    };

    let mut url = format!(
        "{FETCH_DOMAIN}/v5/market/open-interest?category=linear&symbol={ticker_str}&intervalTime={period_str}",
    );

    if let Some((start, end)) = range {
        let interval_ms = period.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(200);

        url.push_str(&format!(
            "&startTime={start}&endTime={end}&limit={num_intervals}"
        ));
    } else {
        url.push_str("&limit=200");
    }

    let response_text = http_request_with_limiter(&url, &BYBIT_LIMITER, 1, None, None).await?;

    let content: Value = sonic_rs::from_str(&response_text).map_err(|e| {
        log::error!(
            "Failed to parse JSON from {}: {}\nResponse: {}",
            url,
            e,
            response_text
        );
        AdapterError::ParseError(e.to_string())
    })?;

    let result_list = content["result"]["list"].as_array().ok_or_else(|| {
        log::error!("Result list is not an array in response: {}", response_text);
        AdapterError::ParseError("Result list is not an array".to_string())
    })?;

    let bybit_oi: Vec<DeOpenInterest> =
        serde_json::from_value(json!(result_list)).map_err(|e| {
            log::error!(
                "Failed to parse open interest array: {}\nResponse: {}",
                e,
                response_text
            );
            AdapterError::ParseError(format!("Failed to parse open interest: {e}"))
        })?;

    let open_interest: Vec<OpenInterest> = bybit_oi
        .into_iter()
        .map(|x| OpenInterest {
            time: x.timestamp,
            value: x.value,
        })
        .collect();

    if open_interest.is_empty() {
        log::warn!(
            "No open interest data found for {}, from url: {}",
            ticker_str,
            url
        );
    }

    Ok(open_interest)
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ApiResponse {
    #[serde(rename = "retCode")]
    ret_code: u32,
    #[serde(rename = "retMsg")]
    ret_msg: String,
    result: ApiResult,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ApiResult {
    symbol: String,
    category: String,
    list: Vec<Vec<Value>>,
}

fn parse_kline_field<T: std::str::FromStr>(field: Option<&str>) -> Result<T, AdapterError> {
    field
        .ok_or_else(|| AdapterError::ParseError("Failed to parse kline".to_string()))
        .and_then(|s| {
            s.parse::<T>()
                .map_err(|_| AdapterError::ParseError("Failed to parse kline".to_string()))
        })
}

pub async fn fetch_klines(
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let (symbol_str, market_type) = &ticker.to_full_symbol_and_type();
    let timeframe_str = {
        if Timeframe::D1 == timeframe {
            "D".to_string()
        } else {
            timeframe.to_minutes().to_string()
        }
    };

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let mut url = format!(
        "{FETCH_DOMAIN}/v5/market/kline?category={}&symbol={}&interval={}",
        market,
        symbol_str.to_uppercase(),
        timeframe_str
    );

    if let Some((start, end)) = range {
        let interval_ms = timeframe.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(1000);

        url.push_str(&format!("&start={start}&end={end}&limit={num_intervals}"));
    } else {
        url.push_str(&format!("&limit={}", 200));
    }

    let response_text = http_request_with_limiter(&url, &BYBIT_LIMITER, 1, None, None).await?;

    let value: ApiResponse =
        sonic_rs::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

    let klines: Result<Vec<Kline>, AdapterError> = value
        .result
        .list
        .iter()
        .map(|kline| {
            let time = parse_kline_field::<u64>(kline[0].as_str())?;

            let open = parse_kline_field::<f32>(kline[1].as_str())?;
            let high = parse_kline_field::<f32>(kline[2].as_str())?;
            let low = parse_kline_field::<f32>(kline[3].as_str())?;
            let close = parse_kline_field::<f32>(kline[4].as_str())?;

            let mut volume = parse_kline_field::<f32>(kline[5].as_str())?;
            volume = if size_in_quote_currency {
                (volume * close).round()
            } else {
                volume
            };

            Ok(Kline {
                time,
                open,
                high,
                low,
                close,
                volume: (-1.0, volume),
            })
        })
        .collect();

    klines
}

pub async fn fetch_ticksize(
    market_type: MarketKind,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let exchange = exchange_from_market_type(market_type);

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let url = format!("{FETCH_DOMAIN}/v5/market/instruments-info?category={market}&limit=1000",);

    let response_text = crate::limiter::HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(AdapterError::FetchError)?
        .text()
        .await
        .map_err(AdapterError::FetchError)?;

    let exchange_info: Value =
        sonic_rs::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let result_list: &Vec<Value> = exchange_info["result"]["list"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !is_symbol_supported(symbol, exchange, true) {
            continue;
        }

        if let Some(contract_type) = item["contractType"].as_str()
            && contract_type != "LinearPerpetual"
            && contract_type != "InversePerpetual"
        {
            continue;
        }

        if let Some(quote_asset) = item["quoteCoin"].as_str()
            && quote_asset != "USDT"
            && quote_asset != "USD"
        {
            continue;
        }

        let lot_size_filter = item["lotSizeFilter"]
            .as_object()
            .ok_or_else(|| AdapterError::ParseError("Lot size filter not found".to_string()))?;

        let min_qty = lot_size_filter["minOrderQty"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Min order qty not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse min order qty".to_string()))?;

        let price_filter = item["priceFilter"]
            .as_object()
            .ok_or_else(|| AdapterError::ParseError("Price filter not found".to_string()))?;

        let min_ticksize = price_filter["tickSize"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse tick size".to_string()))?;

        let ticker = Ticker::new(symbol, exchange);

        ticker_info_map.insert(
            ticker,
            Some(TickerInfo {
                ticker,
                min_ticksize,
                min_qty,
            }),
        );
    }

    Ok(ticker_info_map)
}

pub async fn fetch_ticker_prices(
    market_type: MarketKind,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let exchange = exchange_from_market_type(market_type);

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let url = format!("{FETCH_DOMAIN}/v5/market/tickers?category={market}");

    let response_text = http_request_with_limiter(&url, &BYBIT_LIMITER, 1, None, None).await?;

    let exchange_info: Value =
        sonic_rs::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let result_list: &Vec<Value> = exchange_info["result"]["list"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_prices_map = HashMap::new();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !is_symbol_supported(symbol, exchange, false) {
            continue;
        }

        let mark_price = item["lastPrice"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Mark price not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse mark price".to_string()))?;

        let daily_price_chg = item["price24hPcnt"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Daily price change not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| {
                AdapterError::ParseError("Failed to parse daily price change".to_string())
            })?;

        let daily_volume = item["volume24h"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Daily volume not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse daily volume".to_string()))?;

        let volume_in_usd = if market_type == MarketKind::InversePerps {
            daily_volume
        } else {
            daily_volume * mark_price
        };

        let ticker_stats = TickerStats {
            mark_price,
            daily_price_chg: daily_price_chg * 100.0,
            daily_volume: volume_in_usd,
        };

        ticker_prices_map.insert(Ticker::new(symbol, exchange), ticker_stats);
    }

    Ok(ticker_prices_map)
}
