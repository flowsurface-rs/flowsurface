use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::BufReader, path::PathBuf};

use fastwebsockets::{FragmentCollector, OpCode};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use sonic_rs::{FastStr, to_object_iter_unchecked};

use iced_futures::{
    futures::{SinkExt, Stream, channel::mpsc},
    stream,
};

use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, StreamKind, Ticker, TickerInfo, TickerStats,
        Timeframe, Trade,
        connect::{State, setup_tcp_connection, setup_tls_connection, setup_websocket_connection},
        de_string_to_f32,
        depth::{DepthPayload, DepthUpdate, LocalDepthCache, Order},
        str_f32_parse,
    },
    Connection, Event, StreamError,
};

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::BinanceSpot,
        MarketKind::LinearPerps => Exchange::BinanceLinear,
        MarketKind::InversePerps => Exchange::BinanceInverse,
    }
}

#[derive(Deserialize, Clone)]
pub struct FetchedPerpDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "bids")]
    bids: Vec<Order>,
    #[serde(rename = "asks")]
    asks: Vec<Order>,
}

#[derive(Deserialize, Clone)]
pub struct FetchedSpotDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(rename = "bids")]
    bids: Vec<Order>,
    #[serde(rename = "asks")]
    asks: Vec<Order>,
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "o", deserialize_with = "de_string_to_f32")]
    open: f32,
    #[serde(rename = "h", deserialize_with = "de_string_to_f32")]
    high: f32,
    #[serde(rename = "l", deserialize_with = "de_string_to_f32")]
    low: f32,
    #[serde(rename = "c", deserialize_with = "de_string_to_f32")]
    close: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_f32")]
    volume: f32,
    #[serde(rename = "V", deserialize_with = "de_string_to_f32")]
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

#[derive(Serialize, Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_f32")]
    price: f32,
    #[serde(rename = "q", deserialize_with = "de_string_to_f32")]
    qty: f32,
    #[serde(rename = "m")]
    is_sell: bool,
}

enum SonicDepth {
    Spot(SpotDepth),
    Perp(PerpDepth),
}

#[derive(Serialize, Deserialize)]
struct SpotDepth {
    #[serde(rename = "E")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<Order>,
    #[serde(rename = "a")]
    asks: Vec<Order>,
}

#[derive(Serialize, Deserialize)]
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
    bids: Vec<Order>,
    #[serde(rename = "a")]
    asks: Vec<Order>,
}

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

fn feed_de(slice: &[u8], market: MarketKind) -> Result<StreamData, StreamError> {
    let exchange = exchange_from_market_type(market);

    let mut stream_type: Option<StreamWrapper> = None;
    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| StreamError::ParseError(e.to_string()))?;

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
                Some(StreamWrapper::Depth) => match market {
                    MarketKind::Spot => {
                        let depth: SpotDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| StreamError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Spot(depth)));
                    }
                    MarketKind::LinearPerps | MarketKind::InversePerps => {
                        let depth: PerpDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| StreamError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Perp(depth)));
                    }
                },
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: SonicKlineWrap = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| StreamError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Kline(
                        Ticker::new(kline_wrap.symbol, exchange),
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

async fn connect(
    domain: &str,
    streams: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, StreamError> {
    let tcp_stream = setup_tcp_connection(domain).await?;
    let tls_stream = setup_tls_connection(domain, tcp_stream).await?;
    let url = format!("wss://{domain}/stream?streams={streams}");
    setup_websocket_connection(domain, tls_stream, &url).await
}

async fn try_resync(
    exchange: Exchange,
    ticker: Ticker,
    orderbook: &mut LocalDepthCache,
    state: &mut State,
    output: &mut mpsc::Sender<Event>,
    already_fetching: &mut bool,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    *already_fetching = true;

    tokio::spawn(async move {
        let result = fetch_depth(&ticker).await;
        let _ = tx.send(result);
    });

    match rx.await {
        Ok(Ok(depth)) => {
            orderbook.update(DepthUpdate::Snapshot(depth));
        }
        Ok(Err(e)) => {
            let _ = output
                .send(Event::Disconnected(
                    exchange,
                    format!("Depth fetch failed: {e}"),
                ))
                .await;
        }
        Err(e) => {
            *state = State::Disconnected;

            output
                .send(Event::Disconnected(
                    exchange,
                    format!("Failed to send fetched depth for {ticker}, error: {e}"),
                ))
                .await
                .expect("Trying to send disconnect event...");
        }
    }
    *already_fetching = false;
}

#[allow(unused_assignments)]
pub fn connect_market_stream(ticker: Ticker) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let (symbol_str, market) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market);

        let stream_1 = format!("{}@aggTrade", symbol_str.to_lowercase());
        let stream_2 = format!("{}@depth@100ms", symbol_str.to_lowercase());

        let mut orderbook: LocalDepthCache = LocalDepthCache::new();
        let mut trades_buffer: Vec<Trade> = Vec::new();
        let mut already_fetching: bool = false;
        let mut prev_id: u64 = 0;

        let streams = format!("{stream_1}/{stream_2}");

        let domain = match market {
            MarketKind::Spot => "stream.binance.com",
            MarketKind::LinearPerps => "fstream.binance.com",
            MarketKind::InversePerps => "dstream.binance.com",
        };

        let contract_size = get_contract_size(&ticker, market);

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
                                orderbook.update(DepthUpdate::Snapshot(depth));
                                prev_id = 0;

                                state = State::Connected(websocket);

                                let _ = output.send(Event::Connected(exchange, Connection)).await;
                            }
                            Ok(Err(e)) => {
                                let _ = output
                                    .send(Event::Disconnected(
                                        exchange,
                                        format!("Depth fetch failed: {e}"),
                                    ))
                                    .await;
                            }
                            Err(e) => {
                                let _ = output
                                    .send(Event::Disconnected(
                                        exchange,
                                        format!("Channel error: {e}"),
                                    ))
                                    .await;
                            }
                        }
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
                                if let Ok(data) = feed_de(&msg.payload[..], market) {
                                    match data {
                                        StreamData::Trade(de_trade) => {
                                            let trade = Trade {
                                                time: de_trade.time,
                                                is_sell: de_trade.is_sell,
                                                price: de_trade.price,
                                                qty: contract_size.map_or(de_trade.qty, |size| {
                                                    de_trade.qty * size
                                                }),
                                            };

                                            trades_buffer.push(trade);
                                        }
                                        StreamData::Depth(depth_type) => {
                                            if already_fetching {
                                                log::warn!("Already fetching...\n");
                                                continue;
                                            }

                                            let last_update_id = orderbook.last_update_id;

                                            match depth_type {
                                                SonicDepth::Perp(ref de_depth) => {
                                                    if (de_depth.final_id <= last_update_id)
                                                        || last_update_id == 0
                                                    {
                                                        continue;
                                                    }

                                                    if prev_id == 0
                                                        && (de_depth.first_id > last_update_id + 1)
                                                        || (last_update_id + 1 > de_depth.final_id)
                                                    {
                                                        log::warn!(
                                                            "Out of sync at first event. Trying to resync...\n"
                                                        );

                                                        try_resync(
                                                            exchange,
                                                            ticker,
                                                            &mut orderbook,
                                                            &mut state,
                                                            &mut output,
                                                            &mut already_fetching,
                                                        )
                                                        .await;
                                                    }

                                                    if (prev_id == 0)
                                                        || (prev_id == de_depth.prev_final_id)
                                                    {
                                                        orderbook.update(DepthUpdate::Diff(
                                                            new_depth_cache(
                                                                &depth_type,
                                                                contract_size,
                                                            ),
                                                        ));

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::DepthAndTrades {
                                                                    exchange,
                                                                    ticker,
                                                                },
                                                                de_depth.time,
                                                                orderbook.depth.clone(),
                                                                std::mem::take(&mut trades_buffer)
                                                                    .into_boxed_slice(),
                                                            ))
                                                            .await;

                                                        prev_id = de_depth.final_id;
                                                    } else {
                                                        state = State::Disconnected;
                                                        let _ = output.send(
                                                                Event::Disconnected(
                                                                    exchange,
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
                                                        log::warn!(
                                                            "Out of sync at first event. Trying to resync...\n"
                                                        );

                                                        try_resync(
                                                            exchange,
                                                            ticker,
                                                            &mut orderbook,
                                                            &mut state,
                                                            &mut output,
                                                            &mut already_fetching,
                                                        )
                                                        .await;
                                                    }

                                                    if (prev_id == 0)
                                                        || (prev_id == de_depth.first_id - 1)
                                                    {
                                                        orderbook.update(DepthUpdate::Diff(
                                                            new_depth_cache(
                                                                &depth_type,
                                                                contract_size,
                                                            ),
                                                        ));

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::DepthAndTrades {
                                                                    exchange,
                                                                    ticker,
                                                                },
                                                                de_depth.time,
                                                                orderbook.depth.clone(),
                                                                std::mem::take(&mut trades_buffer)
                                                                    .into_boxed_slice(),
                                                            ))
                                                            .await;

                                                        prev_id = de_depth.final_id;
                                                    } else {
                                                        state = State::Disconnected;
                                                        let _ = output.send(
                                                                Event::Disconnected(
                                                                    exchange,
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
                    };
                }
            }
        }
    })
}

pub fn connect_kline_stream(
    streams: Vec<(Ticker, Timeframe)>,
    market: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let exchange = exchange_from_market_type(market);

        let stream_str = streams
            .iter()
            .map(|(ticker, timeframe)| {
                let timeframe_str = timeframe.to_string();
                format!(
                    "{}@kline_{timeframe_str}",
                    ticker.to_full_symbol_and_type().0.to_lowercase()
                )
            })
            .collect::<Vec<String>>()
            .join("/");

        loop {
            match &mut state {
                State::Disconnected => {
                    let domain = match market {
                        MarketKind::Spot => "stream.binance.com",
                        MarketKind::LinearPerps => "fstream.binance.com",
                        MarketKind::InversePerps => "dstream.binance.com",
                    };

                    if let Ok(websocket) = connect(domain, stream_str.as_str()).await {
                        state = State::Connected(websocket);
                        let _ = output.send(Event::Connected(exchange, Connection)).await;
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

                                    if let Some(c_size) = get_contract_size(&ticker, market) {
                                        (buy_volume * c_size, sell_volume * c_size)
                                    } else {
                                        (buy_volume, sell_volume)
                                    }
                                };

                                let kline = Kline {
                                    time: de_kline.time,
                                    open: de_kline.open,
                                    high: de_kline.high,
                                    low: de_kline.low,
                                    close: de_kline.close,
                                    volume: (buy_volume, sell_volume),
                                };

                                if let Some(timeframe) = streams
                                    .iter()
                                    .find(|(_, tf)| tf.to_string() == de_kline.interval)
                                {
                                    let _ = output
                                        .send(Event::KlineReceived(
                                            StreamKind::Kline {
                                                exchange,
                                                ticker,
                                                timeframe: timeframe.1,
                                            },
                                            kline,
                                        ))
                                        .await;
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

fn get_contract_size(ticker: &Ticker, market_type: MarketKind) -> Option<f32> {
    match market_type {
        MarketKind::Spot | MarketKind::LinearPerps => None,
        MarketKind::InversePerps => {
            if ticker.to_full_symbol_and_type().0 == "BTCUSD_PERP" {
                Some(100.0)
            } else {
                Some(10.0)
            }
        }
    }
}

fn new_depth_cache(depth: &SonicDepth, contract_size: Option<f32>) -> DepthPayload {
    let (time, final_id, bids, asks) = match depth {
        SonicDepth::Spot(de) => (de.time, de.final_id, &de.bids, &de.asks),
        SonicDepth::Perp(de) => (de.time, de.final_id, &de.bids, &de.asks),
    };

    DepthPayload {
        last_update_id: final_id,
        time,
        bids: bids
            .iter()
            .map(|x| Order {
                price: x.price,
                qty: contract_size.map_or(x.qty, |size| x.qty * size),
            })
            .collect(),
        asks: asks
            .iter()
            .map(|x| Order {
                price: x.price,
                qty: contract_size.map_or(x.qty, |size| x.qty * size),
            })
            .collect(),
    }
}

async fn fetch_depth(ticker: &Ticker) -> Result<DepthPayload, StreamError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let base_url = match market_type {
        MarketKind::Spot => "https://api.binance.com/api/v3/depth",
        MarketKind::LinearPerps => "https://fapi.binance.com/fapi/v1/depth",
        MarketKind::InversePerps => "https://dapi.binance.com/dapi/v1/depth",
    };

    let url = format!(
        "{}?symbol={}&limit=1000",
        base_url,
        symbol_str.to_uppercase()
    );

    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    match market_type {
        MarketKind::Spot => {
            let fetched_depth: FetchedSpotDepth =
                serde_json::from_str(&text).map_err(|e| StreamError::ParseError(e.to_string()))?;

            let depth = DepthPayload {
                last_update_id: fetched_depth.update_id,
                time: chrono::Utc::now().timestamp_millis() as u64,
                bids: fetched_depth.bids,
                asks: fetched_depth.asks,
            };

            Ok(depth)
        }
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            let fetched_depth: FetchedPerpDepth =
                serde_json::from_str(&text).map_err(|e| StreamError::ParseError(e.to_string()))?;

            let depth = DepthPayload {
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
    #[serde(deserialize_with = "de_string_to_f32")] f32,
    #[serde(deserialize_with = "de_string_to_f32")] f32,
    #[serde(deserialize_with = "de_string_to_f32")] f32,
    #[serde(deserialize_with = "de_string_to_f32")] f32,
    #[serde(deserialize_with = "de_string_to_f32")] f32,
    u64,
    String,
    u32,
    #[serde(deserialize_with = "de_string_to_f32")] f32,
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
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, StreamError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    let timeframe_str = timeframe.to_string();

    let base_url = match market_type {
        MarketKind::Spot => "https://api.binance.com/api/v3/klines",
        MarketKind::LinearPerps => "https://fapi.binance.com/fapi/v1/klines",
        MarketKind::InversePerps => "https://dapi.binance.com/dapi/v1/klines",
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&interval={timeframe_str}");

    if let Some((start, end)) = range {
        let interval_ms = timeframe.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(1000);

        if num_intervals < 3 {
            let new_start = start - (interval_ms * 5);
            let new_end = end + (interval_ms * 5);
            let num_intervals = ((new_end - new_start) / interval_ms).min(1000);

            url.push_str(&format!(
                "&startTime={new_start}&endTime={new_end}&limit={num_intervals}"
            ));
        } else {
            url.push_str(&format!(
                "&startTime={start}&endTime={end}&limit={num_intervals}"
            ));
        }
    } else {
        url.push_str("&limit=400");
    }

    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let fetched_klines: Vec<FetchedKlines> = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse klines: {e}")))?;

    let klines: Vec<_> = fetched_klines
        .into_iter()
        .map(|k| Kline {
            time: k.0,
            open: k.1,
            high: k.2,
            low: k.3,
            close: k.4,
            volume: match market_type {
                MarketKind::Spot | MarketKind::LinearPerps => {
                    let sell_volume = k.5 - k.9;
                    (k.9, sell_volume)
                }
                MarketKind::InversePerps => {
                    let contract_size = if symbol_str == "BTCUSD_PERP" {
                        100.0
                    } else {
                        10.0
                    };

                    let sell_volume = k.5 - k.9;
                    (k.9 * contract_size, sell_volume * contract_size)
                }
            },
        })
        .collect();

    Ok(klines)
}

pub async fn fetch_ticksize(
    market: MarketKind,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, StreamError> {
    let exchange = exchange_from_market_type(market);

    let url = match market {
        MarketKind::Spot => "https://api.binance.com/api/v3/exchangeInfo".to_string(),
        MarketKind::LinearPerps => "https://fapi.binance.com/fapi/v1/exchangeInfo".to_string(),
        MarketKind::InversePerps => "https://dapi.binance.com/dapi/v1/exchangeInfo".to_string(),
    };
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let exchange_info: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse exchange info: {e}")))?;

    let rate_limits = exchange_info["rateLimits"]
        .as_array()
        .ok_or_else(|| StreamError::ParseError("Missing rateLimits array".to_string()))?;

    let request_limit = rate_limits
        .iter()
        .find(|x| x["rateLimitType"].as_str().unwrap_or_default() == "REQUEST_WEIGHT")
        .and_then(|x| x["limit"].as_i64())
        .ok_or_else(|| StreamError::ParseError("Missing request weight limit".to_string()))?;

    log::info!(
        "Binance req. weight limit per minute {}: {:?}",
        match market {
            MarketKind::Spot => "Spot",
            MarketKind::LinearPerps => "Linear Perps",
            MarketKind::InversePerps => "Inverse Perps",
        },
        request_limit
    );

    let symbols = exchange_info["symbols"]
        .as_array()
        .ok_or_else(|| StreamError::ParseError("Missing symbols array".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    for item in symbols {
        let symbol_str = item["symbol"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Missing symbol".to_string()))?;

        if let Some(contract_type) = item["contractType"].as_str() {
            if contract_type != "PERPETUAL" {
                continue;
            }
        }

        if let Some(quote_asset) = item["quoteAsset"].as_str() {
            if quote_asset != "USDT" && quote_asset != "USD" {
                continue;
            }
        }

        if let Some(status) = item["status"].as_str() {
            if status != "TRADING" && status != "HALT" {
                continue;
            }
        }

        let filters = item["filters"]
            .as_array()
            .ok_or_else(|| StreamError::ParseError("Missing filters array".to_string()))?;

        let price_filter = filters
            .iter()
            .find(|x| x["filterType"].as_str().unwrap_or_default() == "PRICE_FILTER");

        let min_qty = filters
            .iter()
            .find(|x| x["filterType"].as_str().unwrap_or_default() == "LOT_SIZE")
            .and_then(|x| x["minQty"].as_str())
            .ok_or_else(|| {
                StreamError::ParseError("Missing minQty in LOT_SIZE filter".to_string())
            })?
            .parse::<f32>()
            .map_err(|e| StreamError::ParseError(format!("Failed to parse minQty: {e}")))?;

        if let Some(price_filter) = price_filter {
            let min_ticksize = price_filter["tickSize"]
                .as_str()
                .ok_or_else(|| StreamError::ParseError("tickSize not found".to_string()))?
                .parse::<f32>()
                .map_err(|e| StreamError::ParseError(format!("Failed to parse tickSize: {e}")))?;

            let ticker = Ticker::new(symbol_str, exchange);

            ticker_info_map.insert(
                Ticker::new(symbol_str, exchange),
                Some(TickerInfo {
                    ticker,
                    min_ticksize,
                    min_qty,
                }),
            );
        } else {
            ticker_info_map.insert(Ticker::new(symbol_str, exchange), None);
        }
    }

    Ok(ticker_info_map)
}

const LINEAR_FILTER_VOLUME: f32 = 32_000_000.0;
const INVERSE_FILTER_VOLUME: f32 = 4_000.0;
const SPOT_FILTER_VOLUME: f32 = 9_000_000.0;

pub async fn fetch_ticker_prices(
    market: MarketKind,
) -> Result<HashMap<Ticker, TickerStats>, StreamError> {
    let exhange = exchange_from_market_type(market);

    let url = match market {
        MarketKind::Spot => "https://api.binance.com/api/v3/ticker/24hr".to_string(),
        MarketKind::LinearPerps => "https://fapi.binance.com/fapi/v1/ticker/24hr".to_string(),
        MarketKind::InversePerps => "https://dapi.binance.com/dapi/v1/ticker/24hr".to_string(),
    };
    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;
    let text = response.text().await.map_err(StreamError::FetchError)?;

    let value: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| StreamError::ParseError(format!("Failed to parse prices: {e}")))?;

    let mut ticker_price_map = HashMap::new();

    let volume_threshold = match market {
        MarketKind::Spot => SPOT_FILTER_VOLUME,
        MarketKind::LinearPerps => LINEAR_FILTER_VOLUME,
        MarketKind::InversePerps => INVERSE_FILTER_VOLUME,
    };

    for item in value {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Symbol not found".to_string()))?;

        let last_price = item["lastPrice"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Last price not found".to_string()))?
            .parse::<f32>()
            .map_err(|e| StreamError::ParseError(format!("Failed to parse last price: {e}")))?;

        let price_change_pt = item["priceChangePercent"]
            .as_str()
            .ok_or_else(|| StreamError::ParseError("Price change percent not found".to_string()))?
            .parse::<f32>()
            .map_err(|e| {
                StreamError::ParseError(format!("Failed to parse price change percent: {e}"))
            })?;

        let volume = {
            match market {
                MarketKind::Spot | MarketKind::LinearPerps => item["quoteVolume"]
                    .as_str()
                    .ok_or_else(|| StreamError::ParseError("Quote volume not found".to_string()))?
                    .parse::<f32>()
                    .map_err(|e| {
                        StreamError::ParseError(format!("Failed to parse quote volume: {e}"))
                    })?,
                MarketKind::InversePerps => item["volume"]
                    .as_str()
                    .ok_or_else(|| StreamError::ParseError("Volume not found".to_string()))?
                    .parse::<f32>()
                    .map_err(|e| StreamError::ParseError(format!("Failed to parse volume: {e}")))?,
            }
        };

        if volume < volume_threshold {
            continue;
        }

        let ticker_stats = TickerStats {
            mark_price: last_price,
            daily_price_chg: price_change_pt,
            daily_volume: match market {
                MarketKind::Spot | MarketKind::LinearPerps => volume,
                MarketKind::InversePerps => {
                    let contract_size = if symbol == "BTCUSD_PERP" { 100.0 } else { 10.0 };
                    volume * contract_size
                }
            },
        };

        ticker_price_map.insert(Ticker::new(symbol, exhange), ticker_stats);
    }

    Ok(ticker_price_map)
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(rename = "timestamp")]
    pub time: u64,
    #[serde(rename = "sumOpenInterest", deserialize_with = "de_string_to_f32")]
    pub sum: f32,
}

const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000; // 30 days in milliseconds

pub async fn fetch_historical_oi(
    ticker: Ticker,
    range: Option<(u64, u64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, StreamError> {
    let (ticker_str, market) = ticker.to_full_symbol_and_type();
    let period_str = period.to_string();

    let (domain, pair_str) = match market {
        MarketKind::LinearPerps => (
            "https://fapi.binance.com/futures/data/openInterestHist",
            format!("?symbol={ticker_str}",),
        ),
        MarketKind::InversePerps => (
            "https://dapi.binance.com/futures/data/openInterestHist",
            format!(
                "?pair={}&contractType=PERPETUAL",
                ticker_str.split('_').next().unwrap()
            ),
        ),
        _ => {
            let err_msg = format!("Unsupported market type for open interest: {market:?}");
            log::error!("{}", err_msg);
            return Err(StreamError::UnknownError(err_msg));
        }
    };

    let mut url = format!("{domain}{pair_str}&period={period_str}",);

    if let Some((start, end)) = range {
        // API is limited to 30 days of historical data
        let thirty_days_ago = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Could not get system time")
            .as_millis() as u64
            - THIRTY_DAYS_MS;

        if end < thirty_days_ago {
            let err_msg = format!(
                "Requested end time {end} is before available data (30 days is the API limit)"
            );
            log::error!("{}", err_msg);
            return Err(StreamError::UnknownError(err_msg));
        }

        let adjusted_start = if start < thirty_days_ago {
            log::warn!(
                "Adjusting start time from {} to {} (30 days limit)",
                start,
                thirty_days_ago
            );
            thirty_days_ago
        } else {
            start
        };

        let interval_ms = period.to_milliseconds();
        let num_intervals = ((end - adjusted_start) / interval_ms).min(500);

        url.push_str(&format!(
            "&startTime={adjusted_start}&endTime={end}&limit={num_intervals}"
        ));
    } else {
        url.push_str("&limit=400");
    }

    let response = reqwest::get(&url).await.map_err(|e| {
        log::error!("Failed to fetch from {}: {}", url, e);
        StreamError::FetchError(e)
    })?;

    let text = response.text().await.map_err(|e| {
        log::error!("Failed to get response text from {}: {}", url, e);
        StreamError::FetchError(e)
    })?;

    let binance_oi: Vec<DeOpenInterest> = serde_json::from_str(&text).map_err(|e| {
        log::error!(
            "Failed to parse response from {}: {}\nResponse: {}",
            url,
            e,
            text
        );
        StreamError::ParseError(format!("Failed to parse open interest: {e}"))
    })?;

    let contract_size = get_contract_size(&ticker, market);

    let open_interest = binance_oi
        .iter()
        .map(|x| OpenInterest {
            time: x.time,
            value: contract_size.map_or(x.sum, |size| x.sum * size),
        })
        .collect::<Vec<OpenInterest>>();

    Ok(open_interest)
}

async fn handle_rate_limit(headers: &hyper::HeaderMap, max_limit: f32) -> Result<(), StreamError> {
    let weight = headers
        .get("x-mbx-used-weight-1m")
        .ok_or_else(|| StreamError::ParseError("Missing rate limit header".to_string()))?
        .to_str()
        .map_err(|e| StreamError::ParseError(format!("Invalid header value: {e}")))?
        .parse::<i32>()
        .map_err(|e| StreamError::ParseError(format!("Invalid weight value: {e}")))?;

    let usage_percentage = (weight as f32 / max_limit) * 100.0;

    match usage_percentage {
        p if p >= 95.0 => {
            log::warn!("Rate limit critical ({:.1}%), sleeping for 10s", p);
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
        p if p >= 90.0 => {
            log::warn!("Rate limit high ({:.1}%), sleeping for 5s", p);
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
        p if p >= 80.0 => {
            log::warn!("Rate limit warning ({:.1}%), sleeping for 3s", p);
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        }
        _ => (),
    }

    Ok(())
}

pub async fn fetch_trades(
    ticker: Ticker,
    from_time: u64,
    data_path: PathBuf,
) -> Result<Vec<Trade>, StreamError> {
    let today_midnight = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();

    if from_time as i64 >= today_midnight.timestamp_millis() {
        return fetch_intraday_trades(ticker, from_time).await;
    }

    let from_date = chrono::DateTime::from_timestamp_millis(from_time as i64)
        .ok_or_else(|| StreamError::ParseError("Invalid timestamp".into()))?
        .date_naive();

    match get_hist_trades(ticker, from_date, data_path).await {
        Ok(trades) => Ok(trades),
        Err(e) => {
            log::warn!(
                "Historical trades fetch failed: {}, falling back to intraday fetch",
                e
            );
            fetch_intraday_trades(ticker, from_time).await
        }
    }
}

pub async fn fetch_intraday_trades(ticker: Ticker, from: u64) -> Result<Vec<Trade>, StreamError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    let base_url = match market_type {
        MarketKind::Spot => "https://api.binance.com/api/v3/aggTrades",
        MarketKind::LinearPerps => "https://fapi.binance.com/fapi/v1/aggTrades",
        MarketKind::InversePerps => "https://dapi.binance.com/dapi/v1/aggTrades",
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&limit=1000",);

    url.push_str(&format!("&startTime={from}"));

    let response = reqwest::get(&url).await.map_err(StreamError::FetchError)?;

    handle_rate_limit(
        response.headers(),
        match market_type {
            MarketKind::Spot => 6000.0,
            MarketKind::LinearPerps | MarketKind::InversePerps => 2400.0,
        },
    )
    .await?;

    let text = response.text().await.map_err(StreamError::FetchError)?;

    let trades: Vec<Trade> = {
        let de_trades: Vec<SonicTrade> = sonic_rs::from_str(&text)
            .map_err(|e| StreamError::ParseError(format!("Failed to parse trades: {e}")))?;

        de_trades
            .into_iter()
            .map(|de_trade| Trade {
                time: de_trade.time,
                is_sell: de_trade.is_sell,
                price: de_trade.price,
                qty: de_trade.qty,
            })
            .collect()
    };

    Ok(trades)
}

pub async fn get_hist_trades(
    ticker: Ticker,
    date: chrono::NaiveDate,
    base_path: PathBuf,
) -> Result<Vec<Trade>, StreamError> {
    let (symbol, market_type) = ticker.to_full_symbol_and_type();

    let market_subpath = match market_type {
        MarketKind::Spot => format!("data/spot/daily/aggTrades/{symbol}"),
        MarketKind::LinearPerps => format!("data/futures/um/daily/aggTrades/{symbol}"),
        MarketKind::InversePerps => format!("data/futures/cm/daily/aggTrades/{symbol}"),
    };

    let zip_file_name = format!(
        "{}-aggTrades-{}.zip",
        symbol.to_uppercase(),
        date.format("%Y-%m-%d"),
    );

    let base_path = base_path.join(&market_subpath);

    std::fs::create_dir_all(&base_path)
        .map_err(|e| StreamError::ParseError(format!("Failed to create directories: {e}")))?;

    let zip_path = format!("{market_subpath}/{zip_file_name}",);
    let base_zip_path = base_path.join(&zip_file_name);

    if std::fs::metadata(&base_zip_path).is_ok() {
        log::info!("Using cached {}", zip_path);
    } else {
        let url = format!("https://data.binance.vision/{zip_path}");

        log::info!("Downloading from {}", url);

        let resp = reqwest::get(&url).await.map_err(StreamError::FetchError)?;

        if !resp.status().is_success() {
            return Err(StreamError::InvalidRequest(format!(
                "Failed to fetch from {}: {}",
                url,
                resp.status()
            )));
        }

        let body = resp.bytes().await.map_err(StreamError::FetchError)?;

        std::fs::write(&base_zip_path, &body).map_err(|e| {
            StreamError::ParseError(format!("Failed to write zip file: {e}, {base_zip_path:?}"))
        })?;
    }

    match std::fs::File::open(&base_zip_path) {
        Ok(file) => {
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| StreamError::ParseError(format!("Failed to unzip file: {e}")))?;

            let mut trades = Vec::new();
            for i in 0..archive.len() {
                let csv_file = archive
                    .by_index(i)
                    .map_err(|e| StreamError::ParseError(format!("Failed to read csv: {e}")))?;

                let mut csv_reader = ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(BufReader::new(csv_file));

                trades.extend(csv_reader.records().filter_map(|record| {
                    record.ok().and_then(|record| {
                        let time = record[5].parse::<u64>().ok()?;
                        let is_sell = record[6].parse::<bool>().ok()?;
                        let price = str_f32_parse(&record[1]);
                        let qty = str_f32_parse(&record[2]);

                        Some(Trade {
                            time,
                            is_sell,
                            price,
                            qty,
                        })
                    })
                }));
            }

            if let Some(latest_trade) = trades.last() {
                match fetch_intraday_trades(ticker, latest_trade.time).await {
                    Ok(intraday_trades) => {
                        trades.extend(intraday_trades);
                    }
                    Err(e) => {
                        log::error!("Failed to fetch intraday trades: {}", e);
                    }
                }
            }

            Ok(trades)
        }
        Err(e) => Err(StreamError::ParseError(format!(
            "Failed to open compressed file: {e}"
        ))),
    }
}
