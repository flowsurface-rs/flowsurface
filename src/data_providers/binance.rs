use std::collections::HashMap;

use fastwebsockets::{FragmentCollector, OpCode};
use ::futures::{SinkExt, Stream};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use iced_futures::stream;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sonic_rs::{to_object_iter_unchecked, FastStr};

use super::{
    deserialize_string_to_f32,
    setup_tcp_connection, setup_tls_connection, setup_websocket_connection, 
    Connection, Event, Kline, LocalDepthCache, MarketType, OpenInterest, Order, State, 
    StreamError, Ticker, TickerInfo, TickerStats, Timeframe, Trade, VecLocalDepthCache,
};

async fn connect(
    domain: &str,
    streams: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, StreamError> {
    let tcp_stream = setup_tcp_connection(domain).await?;
    let tls_stream = setup_tls_connection(domain, tcp_stream).await?;
    let url = format!("wss://{domain}/stream?streams={streams}");
    setup_websocket_connection(domain, tls_stream, &url).await
}

mod string_to_f32 {
    use serde::{self, Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<f32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = <&str>::deserialize(deserializer)?;
        s.parse::<f32>().map_err(serde::de::Error::custom)
    }
}

fn str_f32_parse(s: &str) -> f32 {
    s.parse::<f32>().unwrap_or_else(|e| {
        log::error!("Failed to parse float: {}, error: {}", s, e);
        0.0
    })
}

#[derive(Debug, Deserialize, Clone)]
pub struct FetchedPerpDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: i64,
    #[serde(rename = "T")]
    time: i64,
    #[serde(rename = "bids")]
    bids: Vec<Order>,
    #[serde(rename = "asks")]
    asks: Vec<Order>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FetchedSpotDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: i64,
    #[serde(rename = "bids")]
    bids: Vec<Order>,
    #[serde(rename = "asks")]
    asks: Vec<Order>,
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "o")]
    open: String,
    #[serde(rename = "h")]
    high: String,
    #[serde(rename = "l")]
    low: String,
    #[serde(rename = "c")]
    close: String,
    #[serde(rename = "v")]
    volume: String,
    #[serde(rename = "V")]
    taker_buy_base_asset_volume: String,
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

#[derive(Serialize, Deserialize, Debug)]
struct BidAsk {
    #[serde(rename = "0")]
    price: String,
    #[serde(rename = "1")]
    qty: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "p")]
    price: String,
    #[serde(rename = "q")]
    qty: String,
    #[serde(rename = "m")]
    is_sell: bool,
}

#[derive(Debug)]
enum SonicDepth {
    Spot(SpotDepth),
    LinearPerp(LinearPerpDepth),
}

#[derive(Serialize, Deserialize, Debug)]
struct SpotDepth {
    #[serde(rename = "E")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<BidAsk>,
    #[serde(rename = "a")]
    asks: Vec<BidAsk>,
}

#[derive(Serialize, Deserialize, Debug)]
struct LinearPerpDepth {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "pu")]
    prev_final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<BidAsk>,
    #[serde(rename = "a")]
    asks: Vec<BidAsk>,
}

#[derive(Debug)]
enum StreamData {
    Trade(SonicTrade),
    Depth(SonicDepth),
    Kline(Ticker, SonicKline),
}

enum StreamWrapper {
    Trade,
    Depth,
    Kline,
}

impl StreamWrapper {
    fn from_stream_type(stream_type: &FastStr) -> Option<Self> {
        stream_type.split('@').nth(1).and_then(|after_at| {
            match after_at {
                s if s.starts_with("de") => Some(StreamWrapper::Depth),
                s if s.starts_with("ag") => Some(StreamWrapper::Trade),
                s if s.starts_with("kl") => Some(StreamWrapper::Kline),
                _ => None,
            }
        })
    }
}

fn feed_de(slice: &[u8], market: MarketType) -> Result<StreamData, StreamError> {
    let mut stream_type: Option<StreamWrapper> = None;
    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem
            .map_err(|e| StreamError::ParseError(e.to_string()))?;

        if k == "stream" {
            if let Some(s) = StreamWrapper::from_stream_type(&v.as_raw_faststr()) {
                stream_type = Some(s);
            }
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade: SonicTrade = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| StreamError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Trade(trade));
                }
                Some(StreamWrapper::Depth) => {
                    match market {
                        MarketType::Spot => {
                            let depth: SpotDepth = sonic_rs::from_str(&v.as_raw_faststr())
                                .map_err(|e| StreamError::ParseError(e.to_string()))?;

                            return Ok(StreamData::Depth(SonicDepth::Spot(depth)));
                        }
                        MarketType::LinearPerps => {
                            let depth: LinearPerpDepth = sonic_rs::from_str(&v.as_raw_faststr())
                                .map_err(|e| StreamError::ParseError(e.to_string()))?;

                            return Ok(StreamData::Depth(SonicDepth::LinearPerp(depth)));
                        }
                    }
                }
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: SonicKlineWrap = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| StreamError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Kline(
                        Ticker::new(kline_wrap.symbol, market),
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

    Err(StreamError::ParseError(
        "Failed to parse ws data".to_string(),
    ))
}

#[allow(unused_assignments)]
pub fn connect_market_stream(ticker: Ticker) -> impl Stream<Item = Event> {
    stream::channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let (symbol_str, market) = ticker.get_string();
    
        let stream_1 = format!("{}@aggTrade", symbol_str.to_lowercase());
        let stream_2 = format!("{}@depth@100ms", symbol_str.to_lowercase());

        let mut orderbook: LocalDepthCache = LocalDepthCache::new();
        let mut trades_buffer: Vec<Trade> = Vec::new();
        let mut already_fetching: bool = false;
        let mut prev_id: u64 = 0;

        let streams = format!("{stream_1}/{stream_2}");

        let domain = match market {
            MarketType::Spot => "stream.binance.com",
            MarketType::LinearPerps => "fstream.binance.com",
        };

        loop {
            match &mut state {
                State::Disconnected => {
                    if let Ok(websocket) = connect(domain, streams.as_str()).await {
                        let (tx, rx) = tokio::sync::oneshot::channel();

                        tokio::spawn(async move {
                            let result = fetch_depth(&ticker).await;
                            let _ = tx.send(result);
                        });
                        match rx.await {
                            Ok(Ok(depth)) => {
                                orderbook.fetched(&depth);
                                prev_id = 0;

                                state = State::Connected(websocket);

                                let _ = output
                                    .send(Event::Connected(Connection)).await;
                            }
                            Ok(Err(e)) => {
                                let _ = output
                                    .send(Event::Disconnected(format!("Depth fetch failed: {}", e))).await;
                            }
                            Err(e) => {
                                let _ = output
                                    .send(Event::Disconnected(format!("Channel error: {}", e))).await;
                            }
                        }
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                        let _ = output
                            .send(Event::Disconnected(
                                "Failed to connect to websocket".to_string(),
                            ))
                            .await;
                    }
                }
                State::Connected(ws) => {
                    match ws.read_frame().await {
                        Ok(msg) => match msg.opcode {
                            OpCode::Text => {
                                if let Ok(data) = feed_de(&msg.payload[..], market) {            
                                    match data {
                                        StreamData::Trade(de_trade) => {
                                            let trade = Trade {
                                                time: de_trade.time as i64,
                                                is_sell: de_trade.is_sell,
                                                price: str_f32_parse(&de_trade.price),
                                                qty: str_f32_parse(&de_trade.qty),
                                            };

                                            trades_buffer.push(trade);
                                        }
                                        StreamData::Depth(depth_type) => {
                                            if already_fetching {
                                                log::warn!("Already fetching...\n");
                                                continue;
                                            }

                                            let last_update_id = orderbook.get_fetch_id() as u64;

                                            match depth_type {
                                                SonicDepth::LinearPerp(ref de_depth) => {
                                                    if (de_depth.final_id <= last_update_id)
                                                        || last_update_id == 0
                                                    {
                                                        continue;
                                                    }

                                                    if prev_id == 0
                                                        && (de_depth.first_id > last_update_id + 1)
                                                        || (last_update_id + 1 > de_depth.final_id)
                                                    {
                                                        log::warn!("Out of sync at first event. Trying to resync...\n");

                                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                                        already_fetching = true;

                                                        tokio::spawn(async move {
                                                            let result = fetch_depth(&ticker).await;
                                                            let _ = tx.send(result);
                                                        });
                                                        match rx.await {
                                                            Ok(Ok(depth)) => {
                                                                orderbook.fetched(&depth);
                                                            }
                                                            Ok(Err(e)) => {
                                                                let _ = output
                                                                    .send(Event::Disconnected(format!("Depth fetch failed: {}", e))).await;
                                                            }
                                                            Err(e) => {
                                                                state = State::Disconnected;
                                                                output.send(Event::Disconnected(
                                                                        format!("Failed to send fetched depth for {symbol_str}, error: {e}")
                                                                    )).await.expect("Trying to send disconnect event...");
                                                            }
                                                        }
                                                        already_fetching = false;
                                                    }

                                                    if (prev_id == 0) || (prev_id == de_depth.prev_final_id)
                                                    {
                                                        let time = de_depth.time as i64;

                                                        orderbook.update_depth_cache(
                                                            &new_depth_cache(&depth_type)
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                ticker,
                                                                time,
                                                                orderbook.get_depth(),
                                                                std::mem::take(&mut trades_buffer),
                                                            ))
                                                            .await;

                                                        prev_id = de_depth.final_id;
                                                    } else {
                                                        state = State::Disconnected;
                                                        let _ = output.send(
                                                                Event::Disconnected(
                                                                    format!("Out of sync. Expected update_id: {}, got: {}", de_depth.prev_final_id, prev_id)
                                                                )
                                                            ).await;
                                                    }
                                                }
                                                SonicDepth::Spot(ref de_depth) => {
                                                    if (de_depth.final_id <= last_update_id)
                                                        || last_update_id == 0
                                                    {
                                                        continue;
                                                    }

                                                    if prev_id == 0
                                                        && (de_depth.first_id > last_update_id + 1)
                                                        || (last_update_id + 1 > de_depth.final_id)
                                                    {
                                                        log::warn!("Out of sync at first event. Trying to resync...\n");

                                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                                        already_fetching = true;

                                                        tokio::spawn(async move {
                                                            let result = fetch_depth(&ticker).await;
                                                            let _ = tx.send(result);
                                                        });
                                                        match rx.await {
                                                            Ok(Ok(depth)) => {
                                                                orderbook.fetched(&depth);
                                                            }
                                                            Ok(Err(e)) => {
                                                                let _ = output
                                                                    .send(Event::Disconnected(format!("Depth fetch failed: {}", e))).await;
                                                            }
                                                            Err(e) => {
                                                                state = State::Disconnected;
                                                                output.send(Event::Disconnected(
                                                                        format!("Failed to send fetched depth for {symbol_str}, error: {e}")
                                                                    )).await.expect("Trying to send disconnect event...");
                                                            }
                                                        }
                                                        already_fetching = false;
                                                    }

                                                    if (prev_id == 0) || (prev_id == de_depth.first_id - 1)
                                                    {
                                                        let time = de_depth.time as i64;

                                                        orderbook.update_depth_cache(
                                                            &new_depth_cache(&depth_type)
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                ticker,
                                                                time,
                                                                orderbook.get_depth(),
                                                                std::mem::take(&mut trades_buffer),
                                                            ))
                                                            .await;

                                                        prev_id = de_depth.final_id;
                                                    } else {
                                                        state = State::Disconnected;
                                                        let _ = output.send(
                                                                Event::Disconnected(
                                                                    format!("Out of sync. Expected update_id: {}, got: {}", de_depth.final_id, prev_id)
                                                                )
                                                            ).await;
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
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
                    };
                }
            }
        }
    })
}

pub fn connect_kline_stream(
    streams: Vec<(Ticker, Timeframe)>,
    market: MarketType,
) -> impl Stream<Item = super::Event> {
    stream::channel(100, move |mut output| async move {
        let mut state = State::Disconnected;

        let stream_str = streams
            .iter()
            .map(|(ticker, timeframe)| {
                let timeframe_str = timeframe.to_string();
                format!(
                    "{}@kline_{timeframe_str}",
                    ticker.get_string().0.to_lowercase()
                )
            })
            .collect::<Vec<String>>()
            .join("/");

        loop {
            match &mut state {
                State::Disconnected => {
                    let domain = match market {
                        MarketType::Spot => "stream.binance.com",
                        MarketType::LinearPerps => "fstream.binance.com",
                    };

                    if let Ok(websocket) = connect(domain, stream_str.as_str()).await {
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
                State::Connected(ws) => match ws.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Kline(ticker, de_kline)) = feed_de(&msg.payload[..], market) {
                                let buy_volume =
                                    str_f32_parse(&de_kline.taker_buy_base_asset_volume);
                                let sell_volume = str_f32_parse(&de_kline.volume) - buy_volume;

                                let kline = Kline {
                                    time: de_kline.time,
                                    open: str_f32_parse(&de_kline.open),
                                    high: str_f32_parse(&de_kline.high),
                                    low: str_f32_parse(&de_kline.low),
                                    close: str_f32_parse(&de_kline.close),
                                    volume: (buy_volume, sell_volume),
                                };

                                if let Some(timeframe) = streams
                                    .iter()
                                    .find(|(_, tf)| tf.to_string() == de_kline.interval)
                                {
                                    let _ = output
                                        .send(Event::KlineReceived(ticker, kline, timeframe.1))
                                        .await;
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

fn new_depth_cache(depth: &SonicDepth) -> VecLocalDepthCache {
    match depth {
        SonicDepth::Spot(de) => VecLocalDepthCache {
            last_update_id: de.final_id as i64,
            time: de.time as i64,
            bids: de.bids.iter().map(|x| Order {
                price: str_f32_parse(&x.price),
                qty: str_f32_parse(&x.qty),
            }).collect(),
            asks: de.asks.iter().map(|x| Order {
                price: str_f32_parse(&x.price),
                qty: str_f32_parse(&x.qty),
            }).collect(),
        },
        SonicDepth::LinearPerp(de) => VecLocalDepthCache {
            last_update_id: de.final_id as i64,
            time: de.time as i64,
            bids: de.bids.iter().map(|x| Order {
                price: str_f32_parse(&x.price),
                qty: str_f32_parse(&x.qty),
            }).collect(),
            asks: de.asks.iter().map(|x| Order {
                price: str_f32_parse(&x.price),
                qty: str_f32_parse(&x.qty),
            }).collect(),
        }
    }
}

async fn fetch_depth(ticker: &Ticker) -> Result<VecLocalDepthCache, StreamError> {
    let (symbol_str, market_type) = ticker.get_string();

    let base_url = match market_type {
        MarketType::Spot => "https://api.binance.com/api/v3/depth",
        MarketType::LinearPerps => "https://fapi.binance.com/fapi/v1/depth",
    };

    let url = format!(
        "{}?symbol={}&limit=1000", 
        base_url,
        symbol_str.to_uppercase()
    );

    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    match market_type {
        MarketType::Spot => {
            let fetched_depth: FetchedSpotDepth = serde_json::from_str(&text)
                .map_err(|e| StreamError::ParseError(e.to_string()))?;

            let depth: VecLocalDepthCache = VecLocalDepthCache {
                last_update_id: fetched_depth.update_id,
                time: chrono::Utc::now().timestamp_millis(),
                bids: fetched_depth.bids,
                asks: fetched_depth.asks,
            };

            Ok(depth)
        }
        MarketType::LinearPerps => {
            let fetched_depth: FetchedPerpDepth = serde_json::from_str(&text)
                .map_err(|e| StreamError::ParseError(e.to_string()))?;

            let depth: VecLocalDepthCache = VecLocalDepthCache {
                last_update_id: fetched_depth.update_id,
                time: fetched_depth.time,
                bids: fetched_depth.bids,
                asks: fetched_depth.asks,
            };

            Ok(depth)
        }
    }
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
struct FetchedKlines(
    u64,
    #[serde(with = "string_to_f32")] f32,
    #[serde(with = "string_to_f32")] f32,
    #[serde(with = "string_to_f32")] f32,
    #[serde(with = "string_to_f32")] f32,
    #[serde(with = "string_to_f32")] f32,
    u64,
    String,
    u32,
    #[serde(with = "string_to_f32")] f32,
    String,
    String,
);

impl From<FetchedKlines> for Kline {
    fn from(fetched: FetchedKlines) -> Self {
        let sell_volume = fetched.5 - fetched.9;

        Self {
            time: fetched.0,
            open: fetched.1,
            high: fetched.2,
            low: fetched.3,
            close: fetched.4,
            volume: (fetched.9, sell_volume),
        }
    }
}

pub async fn fetch_klines(
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(i64, i64)>,
) -> Result<Vec<Kline>, StreamError> {
    let (symbol_str, market_type) = ticker.get_string();
    let timeframe_str = timeframe.to_string();

    let base_url = match market_type {
        MarketType::Spot => "https://api.binance.com/api/v3/klines",
        MarketType::LinearPerps => "https://fapi.binance.com/fapi/v1/klines",
    };

    let mut url = format!(
        "{base_url}?symbol={symbol_str}&interval={timeframe_str}"
    );

    if let Some((start, end)) = range {
        let interval_ms = timeframe.to_milliseconds() as i64;
        let num_intervals = ((end - start) / interval_ms).min(1000);

        url.push_str(&format!(
            "&startTime={start}&endTime={end}&limit={num_intervals}"
        ));
    } else {
        url.push_str(&format!("&limit={}", 200));
    }

    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let fetched_klines: Vec<FetchedKlines> = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse klines: {e}")))?;

    let klines: Vec<_> = fetched_klines.into_iter().map(Kline::from).collect();

    Ok(klines)
}

pub async fn fetch_ticksize(market: MarketType) -> Result<HashMap<Ticker, Option<TickerInfo>>, StreamError> {
    let url = match market {
        MarketType::Spot => "https://api.binance.com/api/v3/exchangeInfo".to_string(),
        MarketType::LinearPerps => "https://fapi.binance.com/fapi/v1/exchangeInfo".to_string(),
    };
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let exchange_info: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse exchange info: {e}")))?;

    let symbols = exchange_info["symbols"]
        .as_array()
        .ok_or_else(|| StreamError::ParseError("Missing symbols array".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    let re = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();

    for symbol in symbols {
        let ticker = symbol["symbol"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Missing symbol".to_string()))?
            .to_string();

        if !re.is_match(&ticker) {
            continue;
        }
        
        if !ticker.ends_with("USDT") {
            continue;
        }

        let filters = symbol["filters"]
            .as_array()
            .ok_or_else(|| StreamError::ParseError("Missing filters array".to_string()))?;

        let price_filter = filters
            .iter()
            .find(|x| x["filterType"].as_str().unwrap_or_default() == "PRICE_FILTER");

        if let Some(price_filter) = price_filter {
            let tick_size = price_filter["tickSize"]
                .as_str()
                .ok_or_else(|| StreamError::ParseError("tickSize not found".to_string()))?
                .parse::<f32>()
                .map_err(|e| StreamError::ParseError(format!("Failed to parse tickSize: {e}")))?;

            ticker_info_map.insert(Ticker::new(ticker, market), Some(TickerInfo { tick_size }));
        } else {
            ticker_info_map.insert(Ticker::new(ticker, market), None);
        }
    }

    Ok(ticker_info_map)
}

pub async fn fetch_ticker_prices(market: MarketType) -> Result<HashMap<Ticker, TickerStats>, StreamError> {
    let url = match market {
        MarketType::Spot => "https://api.binance.com/api/v3/ticker/24hr".to_string(),
        MarketType::LinearPerps => "https://fapi.binance.com/fapi/v1/ticker/24hr".to_string(),
    };
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let value: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse prices: {e}")))?;

    let mut ticker_price_map = HashMap::new();

    let re = Regex::new(r"^[a-zA-Z0-9]+$").unwrap();

    let volume_threshold = match market {
        MarketType::Spot => 9_000_000.0,
        MarketType::LinearPerps => 29_000_000.0,
    };

    for item in value {
        if let (Some(symbol), Some(last_price), Some(price_change_pt), Some(volume)) = (
            item.get("symbol").and_then(|v| v.as_str()),
            item.get("lastPrice")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<f32>().ok()),
            item.get("priceChangePercent")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<f32>().ok()),
            item.get("quoteVolume")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse::<f32>().ok()),
        ) {
            if !re.is_match(symbol) {
                continue;
            }

            if !symbol.ends_with("USDT") {
                continue;
            }

            if volume < volume_threshold {
                continue;
            }

            let ticker_stats = TickerStats {
                mark_price: last_price,
                daily_price_chg: price_change_pt,
                daily_volume: volume,
            };

            ticker_price_map.insert(Ticker::new(symbol, market), ticker_stats);
        }
    }

    Ok(ticker_price_map)
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(rename = "timestamp")]
    pub time: i64,
    #[serde(rename = "sumOpenInterest", deserialize_with = "deserialize_string_to_f32")]
    pub sum: f32,
}

pub async fn fetch_historical_oi(
    ticker: Ticker, 
    range: Option<(i64, i64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, StreamError> {
    let ticker_str = ticker.get_string().0.to_uppercase();
    let period_str = match period {
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::M30 => "30m",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        _ => {
            let err_msg = format!("Unsupported timeframe for open interest: {}", period);
            log::error!("{}", err_msg);
            return Err(StreamError::UnknownError(err_msg));
        }
    };

    let mut url = format!(
        "https://fapi.binance.com/futures/data/openInterestHist?symbol={}&period={}",
        ticker_str, period_str,
    );

    if let Some((start, end)) = range {
        let interval_ms = period.to_milliseconds() as i64;
        let num_intervals = ((end - start) / interval_ms).min(500);

        url.push_str(&format!(
            "&startTime={start}&endTime={end}&limit={num_intervals}"
        ));
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

    let binance_oi: Vec<DeOpenInterest> = serde_json::from_str(&text)
        .map_err(|e| {
            log::error!("Failed to parse response from {}: {}\nResponse: {}", url, e, text);
            StreamError::ParseError(format!("Failed to parse open interest: {e}"))
        })?;

    let open_interest: Vec<OpenInterest> = binance_oi
        .iter()
        .map(|x| OpenInterest {
            time: x.time,
            value: x.sum,
        })
        .collect();

    Ok(open_interest)
}