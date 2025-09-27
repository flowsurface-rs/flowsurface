use crate::{Price, PushFrequency, adapter::StreamTicksize, de_string_to_f32};

use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, SIZE_IN_QUOTE_CURRENCY, StreamKind, Ticker,
        TickerInfo, TickerStats, Timeframe, Trade,
        connect::{State, connect_ws},
        depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
        is_symbol_supported,
        limiter::{self, RateLimiter},
        str_f32_parse,
    },
    AdapterError, Event,
};

use csv::ReaderBuilder;
use fastwebsockets::OpCode;
use iced_futures::{
    futures::{SinkExt, Stream, channel::mpsc},
    stream,
};
use serde::Deserialize;
use sonic_rs::{FastStr, to_object_iter_unchecked};
use tokio::sync::Mutex;

use std::{collections::HashMap, io::BufReader, path::PathBuf, sync::LazyLock, time::Duration};

const SPOT_DOMAIN: &str = "https://api.binance.com";
const LINEAR_PERP_DOMAIN: &str = "https://fapi.binance.com";
const INVERSE_PERP_DOMAIN: &str = "https://dapi.binance.com";

static SPOT_LIMITER: LazyLock<Mutex<BinanceLimiter>> =
    LazyLock::new(|| Mutex::new(BinanceLimiter::new(SPOT_LIMIT, REFILL_RATE)));
static LINEAR_LIMITER: LazyLock<Mutex<BinanceLimiter>> =
    LazyLock::new(|| Mutex::new(BinanceLimiter::new(PERP_LIMIT, REFILL_RATE)));
static INVERSE_LIMITER: LazyLock<Mutex<BinanceLimiter>> =
    LazyLock::new(|| Mutex::new(BinanceLimiter::new(PERP_LIMIT, REFILL_RATE)));

const SPOT_LIMIT: usize = 6000;
const PERP_LIMIT: usize = 2400;

const REFILL_RATE: Duration = Duration::from_secs(60);
const LIMITER_BUFFER_PCT: f32 = 0.03;

pub struct BinanceLimiter {
    bucket: limiter::DynamicBucket,
}

impl BinanceLimiter {
    pub fn new(limit: usize, refill_rate: Duration) -> Self {
        let effective_limit = (limit as f32 * (1.0 - LIMITER_BUFFER_PCT)) as usize;
        BinanceLimiter {
            bucket: limiter::DynamicBucket::new(effective_limit, refill_rate),
        }
    }
}

impl RateLimiter for BinanceLimiter {
    fn prepare_request(&mut self, weight: usize) -> Option<Duration> {
        let (wait_time, _reason) = self.bucket.prepare_request(weight);
        wait_time
    }

    fn update_from_response(&mut self, response: &reqwest::Response, _weight: usize) {
        if let Some(header_value) = response
            .headers()
            .get("x-mbx-used-weight-1m")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
        {
            self.bucket.update_weight(header_value);
        }
    }

    fn should_exit_on_response(&self, response: &reqwest::Response) -> bool {
        let status = response.status();
        status == 429 || status == 418
    }
}

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::BinanceSpot,
        MarketKind::LinearPerps => Exchange::BinanceLinear,
        MarketKind::InversePerps => Exchange::BinanceInverse,
    }
}

fn limiter_from_market_type(market: MarketKind) -> &'static Mutex<BinanceLimiter> {
    match market {
        MarketKind::Spot => &SPOT_LIMITER,
        MarketKind::LinearPerps => &LINEAR_LIMITER,
        MarketKind::InversePerps => &INVERSE_LIMITER,
    }
}

fn ws_domain_from_market_type(market: MarketKind) -> &'static str {
    match market {
        MarketKind::Spot => "stream.binance.com",
        MarketKind::LinearPerps => "fstream.binance.com",
        MarketKind::InversePerps => "dstream.binance.com",
    }
}

#[derive(Deserialize, Clone)]
pub struct FetchedPerpDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "bids")]
    bids: Vec<DeOrder>,
    #[serde(rename = "asks")]
    asks: Vec<DeOrder>,
}

#[derive(Deserialize, Clone)]
pub struct FetchedSpotDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(rename = "bids")]
    bids: Vec<DeOrder>,
    #[serde(rename = "asks")]
    asks: Vec<DeOrder>,
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

#[derive(Deserialize, Debug)]
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

#[derive(Deserialize)]
struct SpotDepth {
    #[serde(rename = "E")]
    time: u64,
    #[serde(rename = "U")]
    first_id: u64,
    #[serde(rename = "u")]
    final_id: u64,
    #[serde(rename = "b")]
    bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    asks: Vec<DeOrder>,
}

#[derive(Deserialize)]
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
    bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    asks: Vec<DeOrder>,
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

fn feed_de(slice: &[u8], market: MarketKind) -> Result<StreamData, AdapterError> {
    let exchange = exchange_from_market_type(market);

    let mut stream_type: Option<StreamWrapper> = None;
    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "stream" {
            if let Some(s) = StreamWrapper::from_stream_type(&v.as_raw_faststr()) {
                stream_type = Some(s);
            }
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade: SonicTrade = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Trade(trade));
                }
                Some(StreamWrapper::Depth) => match market {
                    MarketKind::Spot => {
                        let depth: SpotDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Spot(depth)));
                    }
                    MarketKind::LinearPerps | MarketKind::InversePerps => {
                        let depth: PerpDepth = sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                        return Ok(StreamData::Depth(SonicDepth::Perp(depth)));
                    }
                },
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: SonicKlineWrap = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    return Ok(StreamData::Kline(
                        Ticker::new(&kline_wrap.symbol, exchange),
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

    Err(AdapterError::ParseError(
        "Failed to parse ws data".to_string(),
    ))
}

async fn try_resync(
    exchange: Exchange,
    ticker_info: TickerInfo,
    contract_size: Option<f32>,
    orderbook: &mut LocalDepthCache,
    state: &mut State,
    output: &mut mpsc::Sender<Event>,
    already_fetching: &mut bool,
) {
    let ticker = ticker_info.ticker;

    let (tx, rx) = tokio::sync::oneshot::channel();
    *already_fetching = true;

    tokio::spawn(async move {
        let result = fetch_depth(&ticker, contract_size).await;
        let _ = tx.send(result);
    });

    match rx.await {
        Ok(Ok(depth)) => {
            orderbook.update(DepthUpdate::Snapshot(depth), ticker_info.min_ticksize);
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
pub fn connect_market_stream(
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market);

        let mut orderbook: LocalDepthCache = LocalDepthCache::default();
        let mut trades_buffer: Vec<Trade> = Vec::new();
        let mut already_fetching: bool = false;
        let mut prev_id: u64 = 0;

        let contract_size = get_contract_size(&ticker, market);
        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream_1 = format!("{}@aggTrade", symbol_str.to_lowercase());
                    let stream_2 = format!("{}@depth@100ms", symbol_str.to_lowercase());

                    let domain = ws_domain_from_market_type(market);
                    let streams = format!("{stream_1}/{stream_2}");
                    let url = format!("wss://{domain}/stream?streams={streams}");

                    if let Ok(websocket) = connect_ws(domain, &url).await {
                        let (tx, rx) = tokio::sync::oneshot::channel();

                        tokio::spawn(async move {
                            let result = fetch_depth(&ticker, contract_size).await;
                            let _ = tx.send(result);
                        });
                        match rx.await {
                            Ok(Ok(depth)) => {
                                orderbook
                                    .update(DepthUpdate::Snapshot(depth), ticker_info.min_ticksize);
                                prev_id = 0;

                                state = State::Connected(websocket);

                                let _ = output.send(Event::Connected(exchange)).await;
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
                                            let price = Price::from_f32(de_trade.price)
                                                .round_to_min_tick(ticker_info.min_ticksize);
                                            let qty = contract_size.map_or(
                                                if size_in_quote_currency {
                                                    (de_trade.qty * de_trade.price).round()
                                                } else {
                                                    de_trade.qty
                                                },
                                                |size| de_trade.qty * size,
                                            );

                                            let trade = Trade {
                                                time: de_trade.time,
                                                is_sell: de_trade.is_sell,
                                                price,
                                                qty,
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
                                                            ticker_info,
                                                            contract_size,
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
                                                        orderbook.update(
                                                            DepthUpdate::Diff(new_depth_cache(
                                                                &depth_type,
                                                                contract_size,
                                                            )),
                                                            ticker_info.min_ticksize,
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::DepthAndTrades {
                                                                    ticker_info,
                                                                    depth_aggr:
                                                                        StreamTicksize::Client,
                                                                    push_freq,
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
                                                            ticker_info,
                                                            contract_size,
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
                                                        orderbook.update(
                                                            DepthUpdate::Diff(new_depth_cache(
                                                                &depth_type,
                                                                contract_size,
                                                            )),
                                                            ticker_info.min_ticksize,
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::DepthAndTrades {
                                                                    ticker_info,
                                                                    depth_aggr:
                                                                        StreamTicksize::Client,
                                                                    push_freq,
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
    streams: Vec<(TickerInfo, Timeframe)>,
    market: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;
        let exchange = exchange_from_market_type(market);

        let ticker_info_map = streams
            .iter()
            .map(|(ticker_info, _)| (ticker_info.ticker, *ticker_info))
            .collect::<HashMap<Ticker, TickerInfo>>();

        loop {
            match &mut state {
                State::Disconnected => {
                    let stream_str = streams
                        .iter()
                        .map(|(ticker_info, timeframe)| {
                            let ticker = ticker_info.ticker;
                            format!(
                                "{}@kline_{}",
                                ticker.to_full_symbol_and_type().0.to_lowercase(),
                                timeframe
                            )
                        })
                        .collect::<Vec<String>>()
                        .join("/");

                    let domain = ws_domain_from_market_type(market);
                    let url = format!("wss://{domain}/stream?streams={stream_str}");

                    if let Ok(websocket) = connect_ws(domain, &url).await {
                        state = State::Connected(websocket);
                        let _ = output.send(Event::Connected(exchange)).await;
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
                                    } else if SIZE_IN_QUOTE_CURRENCY.get() == Some(&true) {
                                        (
                                            (buy_volume * de_kline.close).round(),
                                            (sell_volume * de_kline.close).round(),
                                        )
                                    } else {
                                        (buy_volume, sell_volume)
                                    }
                                };

                                if let Some((_, tf)) = streams
                                    .iter()
                                    .find(|(_, tf)| tf.to_string() == de_kline.interval)
                                {
                                    if let Some(info) = ticker_info_map.get(&ticker) {
                                        let ticker_info = *info;
                                        let timeframe = *tf;

                                        let kline = Kline::new(
                                            de_kline.time,
                                            de_kline.open,
                                            de_kline.high,
                                            de_kline.low,
                                            de_kline.close,
                                            (buy_volume, sell_volume),
                                            info.min_ticksize,
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
                                        log::error!("Ticker info not found for ticker: {}", ticker);
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

    let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

    DepthPayload {
        last_update_id: final_id,
        time,
        bids: bids
            .iter()
            .map(|x| DeOrder {
                price: x.price,
                qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
            })
            .collect(),
        asks: asks
            .iter()
            .map(|x| DeOrder {
                price: x.price,
                qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
            })
            .collect(),
    }
}

async fn fetch_depth(
    ticker: &Ticker,
    contract_size: Option<f32>,
) -> Result<DepthPayload, AdapterError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let base_url = match market_type {
        MarketKind::Spot => SPOT_DOMAIN.to_string() + "/api/v3/depth",
        MarketKind::LinearPerps => LINEAR_PERP_DOMAIN.to_string() + "/fapi/v1/depth",
        MarketKind::InversePerps => INVERSE_PERP_DOMAIN.to_string() + "/dapi/v1/depth",
    };

    let depth_limit = match market_type {
        MarketKind::Spot => 5000,
        MarketKind::LinearPerps | MarketKind::InversePerps => 1000,
    };

    let url = format!(
        "{}?symbol={}&limit={}",
        base_url,
        symbol_str.to_uppercase(),
        depth_limit
    );

    let weight = match market_type {
        MarketKind::Spot => match depth_limit {
            ..=100_i32 => 5,
            101_i32..=500_i32 => 25,
            501_i32..=1000_i32 => 50,
            1001_i32..=5000_i32 => 250,
            _ => panic!("Invalid depth limit for Spot market"),
        },
        MarketKind::LinearPerps | MarketKind::InversePerps => match depth_limit {
            ..100 => 2,
            100 => 5,
            500 => 10,
            1000 => 20,
            _ => panic!("Invalid depth limit for Perp market"),
        },
    };

    let limiter = limiter_from_market_type(market_type);
    let text = crate::limiter::http_request_with_limiter(&url, limiter, weight, None, None).await?;

    let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

    match market_type {
        MarketKind::Spot => {
            let fetched_depth: FetchedSpotDepth =
                serde_json::from_str(&text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

            let depth = DepthPayload {
                last_update_id: fetched_depth.update_id,
                time: chrono::Utc::now().timestamp_millis() as u64,
                bids: fetched_depth
                    .bids
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
                    })
                    .collect(),
                asks: fetched_depth
                    .asks
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
                    })
                    .collect(),
            };

            Ok(depth)
        }
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            let fetched_depth: FetchedPerpDepth =
                serde_json::from_str(&text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

            let depth = DepthPayload {
                last_update_id: fetched_depth.update_id,
                time: fetched_depth.time,
                bids: fetched_depth
                    .bids
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
                    })
                    .collect(),
                asks: fetched_depth
                    .asks
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: calc_qty(x.qty, x.price, contract_size, size_in_quote_currency),
                    })
                    .collect(),
            };

            Ok(depth)
        }
    }
}

fn calc_qty(qty: f32, price: f32, contract_size: Option<f32>, size_in_quote_currency: bool) -> f32 {
    match contract_size {
        Some(size) => qty * size,
        None => {
            if size_in_quote_currency {
                (qty * price).round()
            } else {
                qty
            }
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
            open: Price::from_f32(fetched.1),
            high: Price::from_f32(fetched.2),
            low: Price::from_f32(fetched.3),
            close: Price::from_f32(fetched.4),
            volume: (fetched.9, sell_volume),
        }
    }
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    let timeframe_str = timeframe.to_string();

    let base_url = match market_type {
        MarketKind::Spot => SPOT_DOMAIN.to_string() + "/api/v3/klines",
        MarketKind::LinearPerps => LINEAR_PERP_DOMAIN.to_string() + "/fapi/v1/klines",
        MarketKind::InversePerps => INVERSE_PERP_DOMAIN.to_string() + "/dapi/v1/klines",
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&interval={timeframe_str}");

    let limit_param = if let Some((start, end)) = range {
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
        num_intervals
    } else {
        let num_intervals = 400;
        url.push_str(&format!("&limit={num_intervals}",));
        num_intervals
    };

    let weight = match market_type {
        MarketKind::Spot => 2,
        MarketKind::LinearPerps | MarketKind::InversePerps => match limit_param {
            1..=100 => 1,
            101..=500 => 2,
            501..=1000 => 5,
            1001..=1500 => 10,
            _ => panic!("Invalid limit for Inverse Perps market"),
        },
    };

    let limiter = limiter_from_market_type(market_type);
    let text = crate::limiter::http_request_with_limiter(&url, limiter, weight, None, None).await?;

    let fetched_klines: Vec<FetchedKlines> = serde_json::from_str(&text)
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse klines: {e}")))?;

    let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

    let klines: Vec<_> = fetched_klines
        .into_iter()
        .map(|k| Kline {
            time: k.0,
            open: Price::from_f32(k.1).round_to_min_tick(ticker_info.min_ticksize),
            high: Price::from_f32(k.2).round_to_min_tick(ticker_info.min_ticksize),
            low: Price::from_f32(k.3).round_to_min_tick(ticker_info.min_ticksize),
            close: Price::from_f32(k.4).round_to_min_tick(ticker_info.min_ticksize),
            volume: match market_type {
                MarketKind::Spot | MarketKind::LinearPerps => {
                    let sell_volume = if size_in_quote_currency {
                        ((k.5 - k.9) * k.4).round()
                    } else {
                        k.5 - k.9
                    };
                    let buy_volume = if size_in_quote_currency {
                        (k.9 * k.4).round()
                    } else {
                        k.9
                    };
                    (buy_volume, sell_volume)
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
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let (url, _weight) = match market {
        MarketKind::Spot => (SPOT_DOMAIN.to_string() + "/api/v3/exchangeInfo", 20),
        MarketKind::LinearPerps => (LINEAR_PERP_DOMAIN.to_string() + "/fapi/v1/exchangeInfo", 1),
        MarketKind::InversePerps => (INVERSE_PERP_DOMAIN.to_string() + "/dapi/v1/exchangeInfo", 1),
    };

    let response_text = crate::limiter::HTTP_CLIENT
        .get(&url)
        .send()
        .await
        .map_err(AdapterError::FetchError)?
        .text()
        .await
        .map_err(AdapterError::FetchError)?;

    let exchange_info: serde_json::Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse exchange info: {e}")))?;

    let symbols = exchange_info["symbols"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Missing symbols array".to_string()))?;

    let exchange = exchange_from_market_type(market);
    let mut ticker_info_map = HashMap::new();

    for item in symbols {
        let symbol_str = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Missing symbol".to_string()))?;

        if !is_symbol_supported(symbol_str, exchange, true) {
            continue;
        }

        if let Some(contract_type) = item["contractType"].as_str()
            && contract_type != "PERPETUAL"
        {
            continue;
        }
        if let Some(quote_asset) = item["quoteAsset"].as_str()
            && quote_asset != "USDT"
            && quote_asset != "USD"
        {
            continue;
        }
        if let Some(status) = item["status"].as_str()
            && status != "TRADING"
            && status != "HALT"
        {
            continue;
        }

        let filters = item["filters"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Missing filters array".to_string()))?;

        let price_filter = filters
            .iter()
            .find(|x| x["filterType"].as_str().unwrap_or_default() == "PRICE_FILTER");

        let min_qty = filters
            .iter()
            .find(|x| x["filterType"].as_str().unwrap_or_default() == "LOT_SIZE")
            .and_then(|x| x["minQty"].as_str())
            .ok_or_else(|| {
                AdapterError::ParseError("Missing minQty in LOT_SIZE filter".to_string())
            })?
            .parse::<f32>()
            .map_err(|e| AdapterError::ParseError(format!("Failed to parse minQty: {e}")))?;

        let contract_size = item["contractSize"].as_f64().map(|v| v as f32);

        let ticker = Ticker::new(symbol_str, exchange);

        if let Some(price_filter) = price_filter {
            let min_ticksize = price_filter["tickSize"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("tickSize not found".to_string()))?
                .parse::<f32>()
                .map_err(|e| AdapterError::ParseError(format!("Failed to parse tickSize: {e}")))?;

            let info = TickerInfo::new(ticker, min_ticksize, min_qty, contract_size);

            ticker_info_map.insert(ticker, Some(info));
        } else {
            ticker_info_map.insert(ticker, None);
        }
    }

    Ok(ticker_info_map)
}

pub async fn fetch_ticker_prices(
    market: MarketKind,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let (url, weight) = match market {
        MarketKind::Spot => (SPOT_DOMAIN.to_string() + "/api/v3/ticker/24hr", 80),
        MarketKind::LinearPerps => (LINEAR_PERP_DOMAIN.to_string() + "/fapi/v1/ticker/24hr", 40),
        MarketKind::InversePerps => (INVERSE_PERP_DOMAIN.to_string() + "/dapi/v1/ticker/24hr", 40),
    };

    let limiter = limiter_from_market_type(market);
    let text = crate::limiter::http_request_with_limiter(&url, limiter, weight, None, None).await?;

    let value: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse prices: {e}")))?;

    let exchange = exchange_from_market_type(market);
    let mut ticker_price_map = HashMap::new();

    for item in value {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !is_symbol_supported(symbol, exchange, false) {
            continue;
        }

        let last_price = item["lastPrice"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Last price not found".to_string()))?
            .parse::<f32>()
            .map_err(|e| AdapterError::ParseError(format!("Failed to parse last price: {e}")))?;

        let price_change_pt = item["priceChangePercent"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Price change percent not found".to_string()))?
            .parse::<f32>()
            .map_err(|e| {
                AdapterError::ParseError(format!("Failed to parse price change percent: {e}"))
            })?;

        let volume = {
            match market {
                MarketKind::Spot | MarketKind::LinearPerps => item["quoteVolume"]
                    .as_str()
                    .ok_or_else(|| AdapterError::ParseError("Quote volume not found".to_string()))?
                    .parse::<f32>()
                    .map_err(|e| {
                        AdapterError::ParseError(format!("Failed to parse quote volume: {e}"))
                    })?,
                MarketKind::InversePerps => item["volume"]
                    .as_str()
                    .ok_or_else(|| AdapterError::ParseError("Volume not found".to_string()))?
                    .parse::<f32>()
                    .map_err(|e| {
                        AdapterError::ParseError(format!("Failed to parse volume: {e}"))
                    })?,
            }
        };

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

        ticker_price_map.insert(Ticker::new(symbol, exchange), ticker_stats);
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

/// # Panics
///
/// Will panic if the `period` is not one of the supported timeframes for open interest
pub async fn fetch_historical_oi(
    ticker: Ticker,
    range: Option<(u64, u64)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let (ticker_str, market) = ticker.to_full_symbol_and_type();
    let period_str = period.to_string();

    let (base_url, pair_str, weight) = match market {
        MarketKind::LinearPerps => (
            LINEAR_PERP_DOMAIN.to_string() + "/futures/data/openInterestHist",
            format!("?symbol={ticker_str}",),
            12,
        ),
        MarketKind::InversePerps => (
            INVERSE_PERP_DOMAIN.to_string() + "/futures/data/openInterestHist",
            format!(
                "?pair={}&contractType=PERPETUAL",
                ticker_str
                    .split('_')
                    .next()
                    .expect("Ticker format not supported"),
            ),
            1,
        ),
        _ => {
            let err_msg = format!("Unsupported market type for open interest: {market:?}");
            log::error!("{}", err_msg);
            return Err(AdapterError::InvalidRequest(err_msg));
        }
    };

    let mut url = format!("{base_url}{pair_str}&period={period_str}",);

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
            return Err(AdapterError::InvalidRequest(err_msg));
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

    let limiter = limiter_from_market_type(market);
    let text = crate::limiter::http_request_with_limiter(&url, limiter, weight, None, None).await?;

    let binance_oi: Vec<DeOpenInterest> = serde_json::from_str(&text).map_err(|e| {
        log::error!(
            "Failed to parse response from {}: {}\nResponse: {}",
            url,
            e,
            text
        );
        AdapterError::ParseError(format!("Failed to parse open interest: {e}"))
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

pub async fn fetch_trades(
    ticker_info: TickerInfo,
    from_time: u64,
    data_path: PathBuf,
) -> Result<Vec<Trade>, AdapterError> {
    let today_midnight = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();

    if from_time as i64 >= today_midnight.timestamp_millis() {
        return fetch_intraday_trades(ticker_info, from_time).await;
    }

    let from_date = chrono::DateTime::from_timestamp_millis(from_time as i64)
        .ok_or_else(|| AdapterError::ParseError("Invalid timestamp".into()))?
        .date_naive();

    match get_hist_trades(ticker_info, from_date, data_path).await {
        Ok(trades) => Ok(trades),
        Err(e) => {
            log::warn!(
                "Historical trades fetch failed: {}, falling back to intraday fetch",
                e
            );
            fetch_intraday_trades(ticker_info, from_time).await
        }
    }
}

pub async fn fetch_intraday_trades(
    ticker_info: TickerInfo,
    from: u64,
) -> Result<Vec<Trade>, AdapterError> {
    let ticker = ticker_info.ticker;
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let (base_url, weight) = match market_type {
        MarketKind::Spot => (SPOT_DOMAIN.to_string() + "/api/v3/aggTrades", 4),
        MarketKind::LinearPerps => (LINEAR_PERP_DOMAIN.to_string() + "/fapi/v1/aggTrades", 20),
        MarketKind::InversePerps => (INVERSE_PERP_DOMAIN.to_string() + "/dapi/v1/aggTrades", 20),
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&limit=1000",);
    url.push_str(&format!("&startTime={from}"));

    let limiter = limiter_from_market_type(market_type);
    let text = crate::limiter::http_request_with_limiter(&url, limiter, weight, None, None).await?;

    let trades: Vec<Trade> = {
        let de_trades: Vec<SonicTrade> = sonic_rs::from_str(&text)
            .map_err(|e| AdapterError::ParseError(format!("Failed to parse trades: {e}")))?;

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

        de_trades
            .into_iter()
            .map(|de_trade| Trade {
                time: de_trade.time,
                is_sell: de_trade.is_sell,
                price: Price::from_f32(de_trade.price).round_to_min_tick(ticker_info.min_ticksize),
                qty: if size_in_quote_currency {
                    (de_trade.qty * de_trade.price).round()
                } else {
                    de_trade.qty
                },
            })
            .collect()
    };

    Ok(trades)
}

pub async fn get_hist_trades(
    ticker_info: TickerInfo,
    date: chrono::NaiveDate,
    base_path: PathBuf,
) -> Result<Vec<Trade>, AdapterError> {
    let ticker = ticker_info.ticker;
    let (symbol, market_type) = ticker.to_full_symbol_and_type();

    let market_subpath = match market_type {
        MarketKind::Spot => format!("data/spot/daily/aggTrades/{symbol}"),
        MarketKind::LinearPerps => {
            format!("data/futures/um/daily/aggTrades/{symbol}")
        }
        MarketKind::InversePerps => {
            format!("data/futures/cm/daily/aggTrades/{symbol}")
        }
    };

    let zip_file_name = format!(
        "{}-aggTrades-{}.zip",
        symbol.to_uppercase(),
        date.format("%Y-%m-%d"),
    );

    let base_path = base_path.join(&market_subpath);

    std::fs::create_dir_all(&base_path)
        .map_err(|e| AdapterError::ParseError(format!("Failed to create directories: {e}")))?;

    let zip_path = format!("{market_subpath}/{zip_file_name}",);
    let base_zip_path = base_path.join(&zip_file_name);

    if std::fs::metadata(&base_zip_path).is_ok() {
        log::info!("Using cached {}", zip_path);
    } else {
        let url = format!("https://data.binance.vision/{zip_path}");

        log::info!("Downloading from {}", url);

        let resp = reqwest::get(&url).await.map_err(AdapterError::FetchError)?;

        if !resp.status().is_success() {
            return Err(AdapterError::InvalidRequest(format!(
                "Failed to fetch from {}: {}",
                url,
                resp.status()
            )));
        }

        let body = resp.bytes().await.map_err(AdapterError::FetchError)?;

        std::fs::write(&base_zip_path, &body).map_err(|e| {
            AdapterError::ParseError(format!("Failed to write zip file: {e}, {base_zip_path:?}"))
        })?;
    }

    match std::fs::File::open(&base_zip_path) {
        Ok(file) => {
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| AdapterError::ParseError(format!("Failed to unzip file: {e}")))?;

            let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

            let mut trades = Vec::new();
            for i in 0..archive.len() {
                let csv_file = archive
                    .by_index(i)
                    .map_err(|e| AdapterError::ParseError(format!("Failed to read csv: {e}")))?;

                let mut csv_reader = ReaderBuilder::new()
                    .has_headers(false)
                    .from_reader(BufReader::new(csv_file));

                trades.extend(csv_reader.records().filter_map(|record| {
                    record.ok().and_then(|record| {
                        let time = record[5].parse::<u64>().ok()?;
                        let is_sell = record[6].parse::<bool>().ok()?;
                        let price_f32 = str_f32_parse(&record[1]);

                        let price =
                            Price::from_f32(price_f32).round_to_min_tick(ticker_info.min_ticksize);

                        let mut qty = str_f32_parse(&record[2]);

                        qty = if size_in_quote_currency {
                            (qty * price_f32).round()
                        } else {
                            qty
                        };

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
                match fetch_intraday_trades(ticker_info, latest_trade.time).await {
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
        Err(e) => Err(AdapterError::ParseError(format!(
            "Failed to open compressed file: {e}"
        ))),
    }
}
