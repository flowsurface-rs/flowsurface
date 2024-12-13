use crate::data_providers::deserialize_string_to_f32;
use crate::data_providers::deserialize_string_to_i64;
use std::collections::HashMap;

use iced::{
    stream, 
    futures::{sink::SinkExt, Stream},
};

use regex::Regex;
use serde_json::json;
use serde_json::Value;

use sonic_rs::{JsonValueTrait, Deserialize, Serialize};
use sonic_rs::to_object_iter_unchecked;

use fastwebsockets::{Frame, FragmentCollector, OpCode};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;

use crate::data_providers::{
    setup_tcp_connection, setup_tls_connection, setup_websocket_connection, 
    Connection, Event, Kline, LocalDepthCache, MarketType, Order, State, 
    StreamError, TickerInfo, TickerStats, Trade, VecLocalDepthCache,
};
use crate::{Ticker, Timeframe};

use super::OpenInterest;

#[derive(Serialize, Deserialize, Debug)]
struct SonicDepth {
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<BidAsk>,
    #[serde(rename = "a")]
    pub asks: Vec<BidAsk>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BidAsk {
    #[serde(rename = "0")]
    pub price: String,
    #[serde(rename = "1")]
    pub qty: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    pub time: u64,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "v")]
    pub qty: String,
    #[serde(rename = "S")]
    pub is_sell: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SonicKline {
    #[serde(rename = "start")]
    pub time: u64,
    #[serde(rename = "open")]
    pub open: String,
    #[serde(rename = "high")]
    pub high: String,
    #[serde(rename = "low")]
    pub low: String,
    #[serde(rename = "close")]
    pub close: String,
    #[serde(rename = "volume")]
    pub volume: String,
    #[serde(rename = "interval")]
    pub interval: String,
}

#[derive(Debug)]
enum StreamData {
    Trade(Vec<SonicTrade>),
    Depth(SonicDepth, String, i64),
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
    fn from_topic(topic: &str, is_ticker: Option<Ticker>, market_type: MarketType) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if let Some(ticker_str) = parts.last() {
            let ticker = is_ticker.unwrap_or_else(|| Ticker::new(ticker_str, market_type));

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
    market_type: MarketType
) -> Result<StreamData, StreamError> {
    let mut stream_type: Option<StreamWrapper> = None;
    let mut depth_wrap: Option<SonicDepth> = None;

    let mut data_type = String::new();
    let mut topic_ticker = Ticker::default();

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| StreamError::ParseError(e.to_string()))?;
        
        if k == "topic" {
            if let Some(val) = v.as_str() {
                let mut is_ticker = None;

                if let Some(ticker) = ticker {
                    is_ticker = Some(ticker);
                }

                match StreamName::from_topic(val, is_ticker, market_type) {
                    StreamName::Depth(ticker) => {
                        stream_type = Some(StreamWrapper::Depth);

                        topic_ticker = ticker;
                    }
                    StreamName::Trade(ticker) => {
                        stream_type = Some(StreamWrapper::Trade);

                        topic_ticker = ticker;
                    }
                    StreamName::Kline(ticker) => {
                        stream_type = Some(StreamWrapper::Kline);

                        topic_ticker = ticker;
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
                        .map_err(|e| StreamError::ParseError(e.to_string()))?;

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
                            .map_err(|e| StreamError::ParseError(e.to_string()))?,
                    );
                }
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: Vec<SonicKline> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| StreamError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Kline(topic_ticker, kline_wrap));
                }
                _ => {
                    log::error!("Unknown stream type");
                }
            }
        } else if k == "cts" {
            if let Some(dw) = depth_wrap {
                let time: u64 = v
                    .as_u64()
                    .ok_or_else(|| StreamError::ParseError("Failed to parse u64".to_string()))?;

                return Ok(StreamData::Depth(dw, data_type.to_string(), time as i64));
            }
        }
    }

    Err(StreamError::UnknownError("Unknown data".to_string()))
}

async fn connect(domain: &str, market_type: MarketType) -> Result<FragmentCollector<TokioIo<Upgraded>>, StreamError> {
    let tcp_stream = setup_tcp_connection(domain).await?;
    let tls_stream = setup_tls_connection(domain, tcp_stream).await?;
    let url = format!(
        "wss://stream.bybit.com/v5/public/{}",
        match market_type {
            MarketType::Spot => "spot",
            MarketType::LinearPerps => "linear",
        }
    );
    setup_websocket_connection(domain, tls_stream, &url).await
}

fn str_f32_parse(s: &str) -> f32 {
    s.parse::<f32>().unwrap_or_else(|e| {
        log::error!("Failed to parse float: {}, error: {}", s, e);
        0.0
    })
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    Timeframe::ALL
        .iter()
        .find(|&tf| tf.to_minutes().to_string() == interval)
        .copied()
}

pub fn connect_market_stream(ticker: Ticker) -> impl Stream<Item = Event> {
    stream::channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let mut trades_buffer: Vec<Trade> = Vec::new();

        let (symbol_str, market_type) = ticker.get_string();

        let stream_1 = format!("publicTrade.{symbol_str}");
        let stream_2 = format!(
            "orderbook.{}.{}",
            match market_type {
                MarketType::Spot => "200",
                MarketType::LinearPerps => "500",
            },
            symbol_str,
        );

        let mut orderbook: LocalDepthCache = LocalDepthCache::new();

        loop {
            match &mut state {
                State::Disconnected => {
                    let domain: &str = "stream.bybit.com";

                    if let Ok(mut websocket) = connect(domain, market_type).await {
                        let subscribe_message: String = serde_json::json!({
                            "op": "subscribe",
                            "args": [stream_1, stream_2]
                        })
                        .to_string();

                        if let Err(e) = websocket
                            .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                subscribe_message.as_bytes(),
                            )))
                            .await
                        {
                            let _ = output
                                .send(Event::Disconnected(format!("Failed subscribing: {e}")))
                                .await;

                            continue;
                        }

                        state = State::Connected(websocket);
                        let _ = output.send(Event::Connected(Connection)).await;
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                        let _ = output
                            .send(Event::Disconnected(
                                "Failed to connect to websocket".to_string(),
                            ))
                            .await;
                    }
                }
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(data) = feed_de(&msg.payload[..], Some(ticker), market_type) {
                                match data {
                                    StreamData::Trade(de_trade_vec) => {
                                        for de_trade in &de_trade_vec {
                                            let trade = Trade {
                                                time: de_trade.time as i64,
                                                is_sell: de_trade.is_sell == "Sell",
                                                price: str_f32_parse(&de_trade.price),
                                                qty: str_f32_parse(&de_trade.qty),
                                            };

                                            trades_buffer.push(trade);
                                        }
                                    }
                                    StreamData::Depth(de_depth, data_type, time) => {
                                        let depth_update = VecLocalDepthCache {
                                            last_update_id: de_depth.update_id as i64,
                                            time,
                                            bids: de_depth
                                                .bids
                                                .iter()
                                                .map(|x| Order {
                                                    price: str_f32_parse(&x.price),
                                                    qty: str_f32_parse(&x.qty),
                                                })
                                                .collect(),
                                            asks: de_depth
                                                .asks
                                                .iter()
                                                .map(|x| Order {
                                                    price: str_f32_parse(&x.price),
                                                    qty: str_f32_parse(&x.qty),
                                                })
                                                .collect(),
                                        };

                                        if (data_type == "snapshot")
                                            || (depth_update.last_update_id == 1)
                                        {
                                            orderbook.fetched(&depth_update);
                                        } else if data_type == "delta" {
                                            orderbook.update_depth_cache(&depth_update);

                                            let _ = output
                                                .send(Event::DepthReceived(
                                                    ticker,
                                                    time,
                                                    orderbook.get_depth(),
                                                    std::mem::take(&mut trades_buffer),
                                                ))
                                                .await;
                                        }
                                    }
                                    _ => {
                                        log::warn!("Unknown data: {:?}", &data);
                                    }
                                }
                            }
                        }
                        OpCode::Close => {
                            state = State::Disconnected;
                            let _ = output
                                .send(Event::Disconnected("Connection closed".to_string()))
                                .await;
                        }
                        _ => {}
                    },
                    Err(e) => {
                        state = State::Disconnected;
                        let _ = output
                            .send(Event::Disconnected(
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
    market_type: MarketType
) -> impl Stream<Item = Event> {
    stream::channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let stream_str = streams
            .iter()
            .map(|(ticker, timeframe)| {
                let timeframe_str = timeframe.to_minutes().to_string();
                format!("kline.{timeframe_str}.{}", ticker.get_string().0)
            })
            .collect::<Vec<String>>();

        loop {
            match &mut state {
                State::Disconnected => {
                    let domain = "stream.bybit.com";

                    if let Ok(mut websocket) = connect(domain, market_type).await {
                        let subscribe_message = serde_json::json!({
                            "op": "subscribe",
                            "args": stream_str
                        })
                        .to_string();

                        if let Err(e) = websocket
                            .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                subscribe_message.as_bytes(),
                            )))
                            .await
                        {
                            let _ = output
                                .send(Event::Disconnected(format!("Failed subscribing: {e}")))
                                .await;

                            continue;
                        }

                        state = State::Connected(websocket);
                        let _ = output.send(Event::Connected(Connection)).await;
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                        let _ = output
                            .send(Event::Disconnected(
                                "Failed to connect to websocket".to_string(),
                            ))
                            .await;
                    }
                }
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => {
                        if msg.opcode == OpCode::Text {
                            if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
                                feed_de(&msg.payload[..], None, market_type)
                            {
                                for de_kline in &de_kline_vec {
                                    let kline = Kline {
                                        time: de_kline.time,
                                        open: str_f32_parse(&de_kline.open),
                                        high: str_f32_parse(&de_kline.high),
                                        low: str_f32_parse(&de_kline.low),
                                        close: str_f32_parse(&de_kline.close),
                                        volume: (-1.0, str_f32_parse(&de_kline.volume)),
                                    };

                                    if let Some(timeframe) = string_to_timeframe(&de_kline.interval)
                                    {
                                        let _ = output
                                            .send(Event::KlineReceived(ticker, kline, timeframe))
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
                    }
                    Err(e) => {
                        state = State::Disconnected;
                        let _ = output
                            .send(Event::Disconnected(
                                "Error reading frame: ".to_string() + &e.to_string(),
                            ))
                            .await;
                    }
                },
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(rename = "openInterest", deserialize_with = "deserialize_string_to_f32")]
    pub value: f32,
    #[serde(deserialize_with = "deserialize_string_to_i64")]
    pub timestamp: i64,
}

pub async fn fetch_historical_oi(
    ticker: Ticker, 
    range: Option<(i64, i64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, StreamError> {
    let ticker_str = ticker.get_string().0.to_uppercase();
    let period_str = match period {
        Timeframe::M5 => "5min",
        Timeframe::M15 => "15min",
        Timeframe::M30 => "30min",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        _ => {
            let err_msg = format!("Unsupported timeframe for open interest: {}", period);
            log::error!("{}", err_msg);
            return Err(StreamError::UnknownError(err_msg));
        }
    };

    let mut url = format!(
        "https://api.bybit.com/v5/market/open-interest?category=linear&symbol={}&intervalTime={}",
        ticker_str, period_str,
    );

    if let Some((start, end)) = range {
        let interval_ms = period.to_milliseconds() as i64;
        let num_intervals = ((end - start) / interval_ms).min(200);

        url.push_str(&format!("&startTime={start}&endTime={end}&limit={num_intervals}"));
    } else {
        url.push_str(&format!("&limit={}", 200));
    }

    let response = reqwest::get(&url)
        .await
        .map_err(|e| {
            log::error!("Failed to fetch from {}: {}", url, e);
            StreamError::FetchError(e)
        })?;
        
    let text = response.text()
        .await
        .map_err(|e| {
            log::error!("Failed to get response text from {}: {}", url, e);
            StreamError::FetchError(e)
        })?;

    let content: Value = sonic_rs::from_str(&text)
        .map_err(|e| {
            log::error!("Failed to parse JSON from {}: {}\nResponse: {}", url, e, text);
            StreamError::ParseError(e.to_string())
        })?;

    let result_list = content["result"]["list"]
        .as_array()
        .ok_or_else(|| {
            log::error!("Result list is not an array in response: {}", text);
            StreamError::ParseError("Result list is not an array".to_string())
        })?;
    
    let bybit_oi: Vec<DeOpenInterest> = serde_json::from_value(json!(result_list))
        .map_err(|e| {
            log::error!("Failed to parse open interest array: {}\nResponse: {}", e, text);
            StreamError::ParseError(format!("Failed to parse open interest: {e}"))
        })?;

    let open_interest = bybit_oi
        .into_iter()
        .map(|x| OpenInterest {
            time: x.timestamp,
            value: x.value,
        })
        .collect();

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

pub async fn fetch_klines(
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(i64, i64)>,
) -> Result<Vec<Kline>, StreamError> {
    let (symbol_str, market_type) = &ticker.get_string();
    let timeframe_str = timeframe.to_minutes().to_string();

    fn parse_kline_field<T: std::str::FromStr>(field: Option<&str>) -> Result<T, StreamError> {
        field
            .ok_or_else(|| StreamError::ParseError("Failed to parse kline".to_string()))
            .and_then(|s| {
                s.parse::<T>()
                    .map_err(|_| StreamError::ParseError("Failed to parse kline".to_string()))
            })
    }

    let market = match market_type {
        MarketType::Spot => "spot",
        MarketType::LinearPerps => "linear",
    };

    let mut url = format!(
        "https://api.bybit.com/v5/market/kline?category={}&symbol={}&interval={}",
        market, symbol_str.to_uppercase(), timeframe_str
    );

    if let Some((start, end)) = range {
        let interval_ms = timeframe.to_milliseconds() as i64;
        let num_intervals = ((end - start) / interval_ms).min(1000);

        url.push_str(&format!("&start={start}&end={end}&limit={num_intervals}"));
    } else {
        url.push_str(&format!("&limit={}", 200));
    }

    let response: reqwest::Response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let api_response: ApiResponse =
        sonic_rs::from_str(&text).map_err(|e| StreamError::ParseError(e.to_string()))?;

    let klines: Result<Vec<Kline>, StreamError> = api_response
        .result
        .list
        .iter()
        .map(|kline| {
            let time = parse_kline_field::<u64>(kline[0].as_str())?;
            let open = parse_kline_field::<f32>(kline[1].as_str())?;
            let high = parse_kline_field::<f32>(kline[2].as_str())?;
            let low = parse_kline_field::<f32>(kline[3].as_str())?;
            let close = parse_kline_field::<f32>(kline[4].as_str())?;
            let volume = parse_kline_field::<f32>(kline[5].as_str())?;

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

pub async fn fetch_ticksize(market_type: MarketType) -> Result<HashMap<Ticker, Option<TickerInfo>>, StreamError> {
    let market = match market_type {
        MarketType::Spot => "spot",
        MarketType::LinearPerps => "linear",
    };

    let url = format!("https://api.bybit.com/v5/market/instruments-info?category={market}");
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let exchange_info: Value =
        sonic_rs::from_str(&text).map_err(|e| StreamError::ParseError(e.to_string()))?;

    let result_list: &Vec<Value> = exchange_info["result"]["list"]
        .as_array()
        .ok_or_else(|| StreamError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    let re = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Symbol not found".to_string()))?;

        if !re.is_match(symbol) {
            continue;
        }

        let price_filter = item["priceFilter"]
            .as_object()
            .ok_or_else(|| StreamError::ParseError("Price filter not found".to_string()))?;

        let tick_size = price_filter["tickSize"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Tick size not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| StreamError::ParseError("Failed to parse tick size".to_string()))?;

        ticker_info_map.insert(Ticker::new(symbol, market_type), Some(TickerInfo { tick_size }));
    }

    Ok(ticker_info_map)
}

pub async fn fetch_ticker_prices(market_type: MarketType) -> Result<HashMap<Ticker, TickerStats>, StreamError> {
    let market = match market_type {
        MarketType::Spot => "spot",
        MarketType::LinearPerps => "linear",
    };

    let url = format!("https://api.bybit.com/v5/market/tickers?category={market}");
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let exchange_info: Value =
        sonic_rs::from_str(&text).map_err(|e| StreamError::ParseError(e.to_string()))?;

    let result_list: &Vec<Value> = exchange_info["result"]["list"]
        .as_array()
        .ok_or_else(|| StreamError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_prices_map = HashMap::new();

    let re = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Symbol not found".to_string()))?;

        if !re.is_match(symbol) {
            continue;
        }

        let mark_price = item["lastPrice"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Mark price not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| StreamError::ParseError("Failed to parse mark price".to_string()))?;

        let daily_price_chg = item["price24hPcnt"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Daily price change not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| {
                StreamError::ParseError("Failed to parse daily price change".to_string())
            })?;

        let daily_volume = item["volume24h"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Daily volume not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| StreamError::ParseError("Failed to parse daily volume".to_string()))?;

        let quote_volume = daily_volume * mark_price;

        if quote_volume < 4_000_000.0 {
            continue;
        }

        let ticker_stats = TickerStats {
            mark_price,
            daily_price_chg: daily_price_chg * 100.0,
            daily_volume: quote_volume,
        };

        ticker_prices_map.insert(Ticker::new(symbol, market_type), ticker_stats);
    }

    Ok(ticker_prices_map)
}