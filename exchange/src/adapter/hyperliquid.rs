use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, StreamKind, Ticker, TickerInfo, TickerStats,
        Timeframe, Trade,
        connect::{State, connect_ws},
        de_string_to_f32,
        depth::{DepthPayload, DepthUpdate, LocalDepthCache, Order},
        limiter::{self, RateLimiter},
    },
    AdapterError, Event,
};

use fastwebsockets::{FragmentCollector, Frame, OpCode};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use iced_futures::{
    futures::{SinkExt, Stream},
    stream,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use std::{collections::HashMap, sync::LazyLock, time::Duration};
use tokio::sync::Mutex;

const API_DOMAIN: &str = "https://api.hyperliquid.xyz";
const WS_DOMAIN: &str = "api.hyperliquid.xyz";

// Both Price (px) and Size (sz) have a maximum number of decimals that are accepted.
// Prices can have up to 5 significant figures, but no more than MAX_DECIMALS - szDecimals decimal places where MAX_DECIMALS is 6 for perps and 8 for spot.
// For example, for perps, 1234.5 is valid but 1234.56 is not (too many significant figures).
// 0.001234 is valid, but 0.0012345 is not (more than 6 decimal places).
// For spot, 0.0001234 is valid if szDecimals is 0 or 1, but not if szDecimals is greater than 2 (more than 8-2 decimal places).
// Integer prices are always allowed, regardless of the number of significant figures. E.g. 123456.0 is a valid price even though 12345.6 is not.
// Prices are precise to the lesser of 5 significant figures or 6 decimals.
// Meta request to the info endpoint returns szDecimals for each asset
const _MAX_DECIMALS_SPOT: u8 = 8;
const MAX_DECIMALS_PERP: u8 = 6;

const SIG_FIG_LIMIT: i32 = 5;

#[allow(dead_code)]
const LIMIT: usize = 1200; // Conservative rate limit

#[allow(dead_code)]
const REFILL_RATE: Duration = Duration::from_secs(60);
const LIMITER_BUFFER_PCT: f32 = 0.05;

#[allow(dead_code)]
static HYPERLIQUID_LIMITER: LazyLock<Mutex<HyperliquidLimiter>> =
    LazyLock::new(|| Mutex::new(HyperliquidLimiter::new(LIMIT, REFILL_RATE)));

pub struct HyperliquidLimiter {
    bucket: limiter::FixedWindowBucket,
}

impl HyperliquidLimiter {
    pub fn new(limit: usize, refill_rate: Duration) -> Self {
        let effective_limit = (limit as f32 * (1.0 - LIMITER_BUFFER_PCT)) as usize;
        Self {
            bucket: limiter::FixedWindowBucket::new(effective_limit, refill_rate),
        }
    }
}

impl RateLimiter for HyperliquidLimiter {
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

#[derive(Debug, Deserialize)]
struct HyperliquidAssetInfo {
    name: String,
    #[serde(rename = "szDecimals")]
    sz_decimals: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidTickerStats {
    coin: String,
    #[serde(rename = "markPx", deserialize_with = "de_string_to_f32")]
    mark_price: f32,
    #[serde(rename = "midPx", deserialize_with = "de_string_to_f32")]
    mid_price: f32,
    #[serde(rename = "prevDayPx", deserialize_with = "de_string_to_f32")]
    prev_day_price: f32,
    #[serde(rename = "dayNtlVlm", deserialize_with = "de_string_to_f32")]
    day_notional_volume: f32,
    #[serde(rename = "openInterest", deserialize_with = "de_string_to_f32")]
    open_interest: f32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "T")]
    close_time: u64,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "i")]
    interval: String,
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
    #[serde(rename = "n")]
    trade_count: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidDepth {
    coin: String,
    levels: [Vec<HyperliquidLevel>; 2], // [bids, asks]
    time: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidLevel {
    #[serde(deserialize_with = "de_string_to_f32")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_f32")]
    sz: f32,
    n: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidTrade {
    coin: String,
    side: String,
    #[serde(deserialize_with = "de_string_to_f32")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_f32")]
    sz: f32,
    time: u64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct HyperliquidWSMessage {
    channel: String,
    data: Value,
}

enum StreamData {
    Trade(Vec<HyperliquidTrade>),
    Depth(HyperliquidDepth),
    Kline(HyperliquidKline),
}

pub async fn fetch_ticksize(
    market: MarketKind,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    if market != MarketKind::LinearPerps {
        return Ok(HashMap::new());
    }

    let url = format!("{}/info", API_DOMAIN);

    // Use metaAndAssetCtxs to get both universe and price data in one call
    let body = json!({
        "type": "metaAndAssetCtxs"
    });

    let response_text = limiter::http_request_with_limiter(
        &url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;
    let response_json: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    // Parse the response: [meta, [asset_contexts...]]
    let meta = response_json
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Missing meta data in response".to_string()))?;

    let asset_contexts = response_json
        .get(1)
        .and_then(|arr| arr.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing asset contexts array".to_string()))?;

    let universe = meta
        .get("universe")
        .and_then(|u| u.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing universe in meta data".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    for (index, asset) in universe.iter().enumerate() {
        if let Ok(asset_info) = serde_json::from_value::<HyperliquidAssetInfo>(asset.clone()) {
            let ticker = Ticker::new(&asset_info.name, Exchange::HyperliquidLinear);

            if let Some(asset_ctx) = asset_contexts.get(index) {
                // Prefer midPx then markPx
                let price = ["midPx", "markPx", "oraclePx"].iter().find_map(|k| {
                    asset_ctx
                        .get(k)
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f32>().ok())
                });

                // Skip ticker if we can't determine a ticksize from price
                if let Some(p) = price {
                    let tick_size = compute_tick_size(p, asset_info.sz_decimals, market);
                    let min_qty = 10.0_f32.powi(-(asset_info.sz_decimals as i32));

                    let ticker_info = TickerInfo {
                        ticker,
                        min_ticksize: tick_size,
                        min_qty,
                    };
                    ticker_info_map.insert(ticker, Some(ticker_info));
                }
            };
        }
    }

    Ok(ticker_info_map)
}

fn compute_tick_size(price: f32, sz_decimals: u32, market: MarketKind) -> f32 {
    if price <= 0.0 {
        return 0.001;
    }
    // Integer-only if price >= 100_000
    if price >= 100_000.0 {
        return 1.0;
    }

    let max_system_decimals = match market {
        MarketKind::LinearPerps => MAX_DECIMALS_PERP as i32,
        _ => MAX_DECIMALS_PERP as i32,
    };
    let decimal_cap = (max_system_decimals - sz_decimals as i32).max(0);
    if decimal_cap == 0 {
        return 1.0;
    }

    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };

    // If integer digits already exceed 5 sig figs -> integer only
    if int_digits >= 6 {
        return 1.0;
    }

    let effective_decimals = if int_digits > 0 {
        // Remaining sig figs go directly to fractional digits
        let remaining_sig = (SIG_FIG_LIMIT - int_digits).max(0);
        remaining_sig.min(decimal_cap)
    } else {
        // price < 1: leading zeros after decimal don't count toward sig figs
        // leading_zeros = -floor(log10(price)) - 1
        let leading_zeros = {
            let lg = price.abs().log10().floor() as i32; // negative
            (-lg - 1).max(0)
        };
        // We can have (leading_zeros + SIG_FIG_LIMIT) total decimal places,
        // but cannot exceed decimal_cap
        let allowed = leading_zeros + SIG_FIG_LIMIT;
        allowed.min(decimal_cap)
    };

    if effective_decimals <= 0 {
        1.0
    } else {
        10_f32.powi(-effective_decimals)
    }
}

pub async fn fetch_ticker_prices(
    market: MarketKind,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    if market != MarketKind::LinearPerps {
        return Ok(HashMap::new());
    }

    let url = format!("{}/info", API_DOMAIN);
    let body = json!({
        "type": "allMids"
    });

    let response_text = limiter::http_request_with_limiter(
        &url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;

    let mids: HashMap<String, String> = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    // Get 24hr stats
    let stats_body = json!({
        "type": "metaAndAssetCtxs"
    });

    let stats_response_text = limiter::http_request_with_limiter(
        &url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&stats_body),
    )
    .await?;

    let stats_json: Value = serde_json::from_str(&stats_response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    // Parse metadata and asset contexts - metaAndAssetCtxs returns [meta, [asset_ctx...]]
    let meta = stats_json
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Meta data not found".to_string()))?;
    let asset_ctxs = stats_json
        .get(1)
        .and_then(|v| v.as_array())
        .ok_or_else(|| AdapterError::ParseError("Asset contexts not found".to_string()))?;
    let universe = meta
        .get("universe")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AdapterError::ParseError("Universe not found".to_string()))?;

    let mut ticker_stats_map = HashMap::new();

    // Process each symbol from mids and match with asset context data
    for (symbol, mid_price_str) in &mids {
        // Skip internal asset IDs like @128, @1, etc.
        if symbol.starts_with('@') {
            continue;
        }

        let mid_price = mid_price_str
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse mid price".to_string()))?;

        // Find the asset index for this symbol in the universe
        let asset_index = universe.iter().position(|asset| {
            asset
                .get("name")
                .and_then(|n| n.as_str())
                .map(|name| name == symbol)
                .unwrap_or(false)
        });

        let (daily_price_chg, daily_volume) = if let Some(index) = asset_index {
            if let Some(asset_ctx) = asset_ctxs.get(index) {
                let prev_day_px = asset_ctx
                    .get("prevDayPx")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AdapterError::ParseError("Previous day price not found".to_string())
                    })?
                    .parse::<f32>()
                    .map_err(|_| {
                        AdapterError::ParseError("Failed to parse previous day price".to_string())
                    })?;

                let day_ntl_vlm = asset_ctx
                    .get("dayNtlVlm")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AdapterError::ParseError("Daily volume not found".to_string()))?
                    .parse::<f32>()
                    .map_err(|_| {
                        AdapterError::ParseError("Failed to parse daily volume".to_string())
                    })?;

                let price_change_pct = if prev_day_px > 0.0 {
                    ((mid_price - prev_day_px) / prev_day_px) * 100.0
                } else {
                    0.0
                };

                (price_change_pct, day_ntl_vlm)
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        let ticker_stats = TickerStats {
            mark_price: mid_price,
            daily_price_chg,
            daily_volume,
        };

        ticker_stats_map.insert(
            Ticker::new(symbol, Exchange::HyperliquidLinear),
            ticker_stats,
        );
    }

    Ok(ticker_stats_map)
}

pub async fn fetch_klines(
    ticker: Ticker,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let interval = match timeframe {
        Timeframe::M1 => "1m",
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::M30 => "30m",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        Timeframe::D1 => "1d",
        _ => {
            return Err(AdapterError::InvalidRequest(
                "Unsupported timeframe".to_string(),
            ));
        }
    };

    let url = format!("{}/info", API_DOMAIN);
    let (symbol_str, _) = ticker.to_full_symbol_and_type();

    // Hyperliquid requires startTime and endTime - use provided range or default to 500 candles
    let (start_time, end_time) = if let Some((start, end)) = range {
        (start, end)
    } else {
        // Default to last 500 candles
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let interval_ms = timeframe.to_milliseconds();
        let candles_ago = now - (interval_ms * 500); // 500 candles ago
        (candles_ago, now)
    };

    let body = json!({
        "type": "candleSnapshot",
        "req": {
            "coin": symbol_str,
            "interval": interval,
            "startTime": start_time,
            "endTime": end_time
        }
    });

    let response_text = limiter::http_request_with_limiter(
        &url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;

    let klines_data: Vec<Value> = serde_json::from_str(&response_text).map_err(|e| {
        AdapterError::ParseError(format!(
            "Failed to parse response as Vec<Value>: {}. Response was: {}",
            e, response_text
        ))
    })?;

    let mut klines = Vec::new();
    for kline_data in klines_data {
        if let Ok(hl_kline) = serde_json::from_value::<HyperliquidKline>(kline_data) {
            let kline = Kline {
                time: hl_kline.time,
                open: hl_kline.open,
                high: hl_kline.high,
                low: hl_kline.low,
                close: hl_kline.close,
                volume: (hl_kline.volume, 0.0), // (base_volume, quote_volume)
            };
            klines.push(kline);
        }
    }

    Ok(klines)
}

pub async fn fetch_historical_oi(
    _ticker: Ticker,
    _range: Option<(u64, u64)>,
    _timeframe: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    // Hyperliquid doesn't provide historical OI data in the same way
    // We can only get current OI from the allMids endpoint
    // For now, return empty vector
    Ok(Vec::new())
}

async fn connect_websocket(
    domain: &str,
    path: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    connect_ws(domain, &url).await
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

pub fn connect_market_stream(ticker: Ticker) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;
        let exchange = Exchange::HyperliquidLinear;

        let mut local_depth_cache = LocalDepthCache::default();
        let mut trades_buffer = Vec::new();

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(WS_DOMAIN, "/ws").await {
                        Ok(mut websocket) => {
                            // Subscribe to depth and trades
                            let (symbol_str, _) = ticker.to_full_symbol_and_type();
                            let subscribe_msg = json!({
                                "method": "subscribe",
                                "subscription": {
                                    "type": "l2Book",
                                    "coin": symbol_str
                                }
                            });

                            let trades_subscribe_msg = json!({
                                "method": "subscribe",
                                "subscription": {
                                    "type": "trades",
                                    "coin": symbol_str
                                }
                            });

                            if let Err(_) = websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    subscribe_msg.to_string().as_bytes(),
                                )))
                                .await
                            {
                                continue;
                            }

                            if let Err(_) = websocket
                                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                    trades_subscribe_msg.to_string().as_bytes(),
                                )))
                                .await
                            {
                                continue;
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
                State::Connected(websocket) => {
                    match websocket.read_frame().await {
                        Ok(msg) => match msg.opcode {
                            OpCode::Text => {
                                if let Ok(stream_data) = parse_websocket_message(&msg.payload) {
                                    match stream_data {
                                        StreamData::Trade(trades) => {
                                            for hl_trade in trades {
                                                let trade = Trade {
                                                    time: hl_trade.time,
                                                    is_sell: hl_trade.side == "A", // A for Ask/Sell, B for Bid/Buy
                                                    price: hl_trade.px,
                                                    qty: hl_trade.sz,
                                                };
                                                trades_buffer.push(trade);
                                            }
                                        }
                                        StreamData::Depth(depth) => {
                                            let bids = depth.levels[0]
                                                .iter()
                                                .map(|level| Order {
                                                    price: level.px,
                                                    qty: level.sz,
                                                })
                                                .collect();

                                            let asks = depth.levels[1]
                                                .iter()
                                                .map(|level| Order {
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
                                            local_depth_cache
                                                .update(DepthUpdate::Snapshot(depth_payload));

                                            let stream_kind = StreamKind::DepthAndTrades { ticker };
                                            let current_depth = local_depth_cache.depth.clone();
                                            let trades = std::mem::take(&mut trades_buffer)
                                                .into_boxed_slice();

                                            let _ = output
                                                .send(Event::DepthReceived(
                                                    stream_kind,
                                                    depth.time,
                                                    current_depth,
                                                    trades,
                                                ))
                                                .await;
                                        }
                                        StreamData::Kline(_) => {
                                            // Handle kline data if needed for depth stream
                                        }
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
                    }
                }
            }
        }
    })
}

pub fn connect_kline_stream(
    streams: Vec<(Ticker, Timeframe)>,
    _market: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;
        let exchange = Exchange::HyperliquidLinear;

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(WS_DOMAIN, "/ws").await {
                        Ok(mut websocket) => {
                            // Subscribe to kline streams
                            for (ticker, timeframe) in &streams {
                                let interval = match timeframe {
                                    Timeframe::M1 => "1m",
                                    Timeframe::M5 => "5m",
                                    Timeframe::M15 => "15m",
                                    Timeframe::M30 => "30m",
                                    Timeframe::H1 => "1h",
                                    Timeframe::H4 => "4h",
                                    Timeframe::D1 => "1d",
                                    _ => continue,
                                };

                                let (symbol_str, _) = ticker.to_full_symbol_and_type();
                                let subscribe_msg = json!({
                                    "method": "subscribe",
                                    "subscription": {
                                        "type": "candle",
                                        "coin": symbol_str,
                                        "interval": interval
                                    }
                                });

                                if let Err(_) = websocket
                                    .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                        subscribe_msg.to_string().as_bytes(),
                                    )))
                                    .await
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
                State::Connected(websocket) => {
                    match websocket.read_frame().await {
                        Ok(msg) => match msg.opcode {
                            OpCode::Text => {
                                if let Ok(StreamData::Kline(hl_kline)) =
                                    parse_websocket_message(&msg.payload)
                                {
                                    let ticker = Ticker::new(&hl_kline.symbol, exchange);
                                    let timeframe = match hl_kline.interval.as_str() {
                                        "1m" => Timeframe::M1,
                                        "5m" => Timeframe::M5,
                                        "15m" => Timeframe::M15,
                                        "30m" => Timeframe::M30,
                                        "1h" => Timeframe::H1,
                                        "4h" => Timeframe::H4,
                                        "1d" => Timeframe::D1,
                                        _ => continue,
                                    };

                                    let kline = Kline {
                                        time: hl_kline.time,
                                        open: hl_kline.open,
                                        high: hl_kline.high,
                                        low: hl_kline.low,
                                        close: hl_kline.close,
                                        volume: (hl_kline.volume, 0.0), // (base_volume, quote_volume)
                                    };

                                    let stream_kind = StreamKind::Kline { ticker, timeframe };
                                    let _ =
                                        output.send(Event::KlineReceived(stream_kind, kline)).await;
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
                    }
                }
            }
        }
    })
}
