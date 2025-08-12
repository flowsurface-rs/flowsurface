use crate::TickMultiplier;

use super::{
    super::{
        Exchange, Kline, MarketKind, OpenInterest, SIZE_IN_QUOTE_CURRENCY, StreamKind, Ticker,
        TickerInfo, TickerStats, Timeframe, Trade,
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

    let mut ticker_prices = HashMap::new();

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

                    ticker_prices.insert(ticker, p);
                }
            };
        }
    }

    Ok(ticker_info_map)
}

#[derive(Clone, Copy, Debug)]
pub struct DepthFeedConfig {
    pub n_sig_figs: i32,
    pub mantissa: i32,
}

impl DepthFeedConfig {
    pub fn new(n_sig_figs: i32, mantissa: i32) -> Self {
        Self {
            n_sig_figs,
            mantissa,
        }
    }
}

impl Default for DepthFeedConfig {
    fn default() -> Self {
        Self {
            n_sig_figs: SIG_FIG_LIMIT,
            mantissa: 1,
        }
    }
}

const ALLOWED_MANTISSA: [i32; 3] = [1, 2, 5];
fn config_from_multiplier(price: f32, multiplier: u16) -> DepthFeedConfig {
    if price <= 0.0 {
        return DepthFeedConfig::default();
    }

    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        return DepthFeedConfig::new(SIG_FIG_LIMIT, 0);
    };

    // Integer region
    if int_digits > SIG_FIG_LIMIT {
        // Scale determines the coarse base tick unit (exchange internal)
        let scale = 10_f32.powi(int_digits - SIG_FIG_LIMIT); // e.g. 118_000 => scale = 10
        let target = multiplier as f32;

        let ratio = target / scale;

        //   ratio < 2  -> base mantissa = 1 (omit sending to avoid issues)
        //   2 <= ratio < 5 -> mantissa = 2
        //   ratio >= 5 -> mantissa = 5
        let mantissa = if ratio < 2.0 {
            0 // omit (implies 1)
        } else if ratio < 5.0 {
            2
        } else {
            5
        };
        return DepthFeedConfig::new(SIG_FIG_LIMIT, mantissa);
    }

    // Fractional / boundary region
    let finest_decimals = SIG_FIG_LIMIT - int_digits;
    let finest_tick = if finest_decimals > 0 {
        10_f32.powi(-finest_decimals)
    } else {
        1.0
    };
    let desired_tick = finest_tick * multiplier as f32;

    // Enumerate n_sig_figs candidates
    let mut best_leq: Option<(i32, f32)> = None;
    let mut best_gt: Option<(i32, f32)> = None;
    for n in 1..=SIG_FIG_LIMIT {
        let decimals = (n - int_digits).max(0);
        let tick = if decimals == 0 {
            1.0
        } else {
            10_f32.powi(-decimals)
        };
        if tick <= desired_tick {
            match best_leq {
                None => best_leq = Some((n, tick)),
                Some((_, prev_tick)) if tick > prev_tick => best_leq = Some((n, tick)),
                _ => {}
            }
        } else {
            match best_gt {
                None => best_gt = Some((n, tick)),
                Some((_, prev_tick)) if tick < prev_tick => best_gt = Some((n, tick)),
                _ => {}
            }
        }
    }
    let chosen = best_leq.or(best_gt).unwrap();
    DepthFeedConfig::new(chosen.0, 0)
}

// Only when mantissa (1,2,5) is provided does tick become mantissa * 10^(int_digits - SIG_FIG_LIMIT).
fn compute_tick_size(price: f32, sz_decimals: u32, market: MarketKind) -> f32 {
    if price <= 0.0 {
        return 0.001;
    }

    let max_system_decimals = match market {
        MarketKind::LinearPerps => MAX_DECIMALS_PERP as i32,
        _ => MAX_DECIMALS_PERP as i32,
    };
    let decimal_cap = (max_system_decimals - sz_decimals as i32).max(0);

    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };

    if int_digits > SIG_FIG_LIMIT {
        return 1.0;
    }

    // int_digits <= SIG_FIG_LIMIT: fractional (or boundary) region
    if price >= 1.0 {
        let remaining_sig = (SIG_FIG_LIMIT - int_digits).max(0);
        if remaining_sig == 0 || decimal_cap == 0 {
            1.0
        } else {
            10_f32.powi(-remaining_sig.min(decimal_cap))
        }
    } else {
        let lg = price.abs().log10().floor() as i32; // negative
        let leading_zeros = (-lg - 1).max(0);
        let total_decimals = (leading_zeros + SIG_FIG_LIMIT).min(decimal_cap);
        if total_decimals <= 0 {
            1.0
        } else {
            10_f32.powi(-total_decimals)
        }
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
            let volume = if SIZE_IN_QUOTE_CURRENCY.get() == Some(&true) {
                (hl_kline.volume * hl_kline.close).round()
            } else {
                hl_kline.volume
            };

            let kline = Kline {
                time: hl_kline.time,
                open: hl_kline.open,
                high: hl_kline.high,
                low: hl_kline.low,
                close: hl_kline.close,
                // -1.0 for the sources that don't provide individual buy/sell volume,
                // negative value indicates the other field is the total volume
                volume: (-1.0, volume),
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

pub fn connect_market_stream(
    ticker: Ticker,
    tick_multiplier: Option<TickMultiplier>,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;
        let exchange = Exchange::HyperliquidLinear;

        let mut local_depth_cache = LocalDepthCache::default();
        let mut trades_buffer = Vec::new();

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);
        let user_multiplier = tick_multiplier.unwrap_or(TickMultiplier(1)).0;

        loop {
            match &mut state {
                State::Disconnected => {
                    let price = match fetch_orderbook(ticker).await {
                        Ok(depth) => depth.bids.first().map(|o| o.price),
                        Err(e) => {
                            log::error!("Failed to fetch orderbook for price: {}", e);
                            None
                        }
                    };
                    if price.is_none() {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    let price = price.unwrap();

                    log::debug!(
                        "Connecting to Hyperliquid market stream with price {} and multiplier {}",
                        price,
                        user_multiplier
                    );

                    let depth_cfg = config_from_multiplier(price, user_multiplier);

                    match connect_websocket(WS_DOMAIN, "/ws").await {
                        Ok(mut websocket) => {
                            let (symbol_str, _) = ticker.to_full_symbol_and_type();
                            let mut depth_subscription = json!({
                                "method": "subscribe",
                                "subscription": {
                                    "type": "l2Book",
                                    "coin": symbol_str,
                                    "nSigFigs": depth_cfg.n_sig_figs,
                                }
                            });

                            // Only attach mantissa if > 0 (we now treat 0 as “omit / implies 1”)
                            if depth_cfg.mantissa > 0
                                && ALLOWED_MANTISSA.contains(&depth_cfg.mantissa)
                                && let Some(obj) = depth_subscription
                                    .get_mut("subscription")
                                    .and_then(|v| v.as_object_mut())
                            {
                                obj.insert("mantissa".to_string(), json!(depth_cfg.mantissa));
                            }

                            log::debug!("Subscribing to depth stream: {}", &depth_subscription);

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
                                tokio::time::sleep(Duration::from_secs(1)).await;
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
                                                    qty: if size_in_quote_currency {
                                                        (hl_trade.sz * hl_trade.px).round()
                                                    } else {
                                                        hl_trade.sz
                                                    },
                                                };
                                                trades_buffer.push(trade);
                                            }
                                        }
                                        StreamData::Depth(depth) => {
                                            let bids = depth.levels[0]
                                                .iter()
                                                .map(|level| Order {
                                                    price: level.px,
                                                    qty: if size_in_quote_currency {
                                                        (level.sz * level.px).round()
                                                    } else {
                                                        level.sz
                                                    },
                                                })
                                                .collect();
                                            let asks = depth.levels[1]
                                                .iter()
                                                .map(|level| Order {
                                                    price: level.px,
                                                    qty: if size_in_quote_currency {
                                                        (level.sz * level.px).round()
                                                    } else {
                                                        level.sz
                                                    },
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

                                            let stream_kind = StreamKind::DepthAndTrades {
                                                ticker,
                                                depth_aggr: super::StreamTicksize::ServerSide(
                                                    TickMultiplier(user_multiplier),
                                                ),
                                            };
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

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

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

                                let volume = if size_in_quote_currency {
                                    (hl_kline.volume * hl_kline.close).round()
                                } else {
                                    hl_kline.volume
                                };

                                let kline = Kline {
                                    time: hl_kline.time,
                                    open: hl_kline.open,
                                    high: hl_kline.high,
                                    low: hl_kline.low,
                                    close: hl_kline.close,
                                    volume: (-1.0, volume),
                                };

                                let stream_kind = StreamKind::Kline { ticker, timeframe };
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

pub async fn fetch_orderbook(ticker: Ticker) -> Result<DepthPayload, AdapterError> {
    let url = format!("{}/info", API_DOMAIN);
    let (symbol_str, _) = ticker.to_full_symbol_and_type();

    let body = json!({
        "type": "l2Book",
        "coin": symbol_str,
    });

    let response_text = limiter::http_request_with_limiter(
        &url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;

    let depth: HyperliquidDepth = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let bids = depth.levels[0]
        .iter()
        .map(|level| Order {
            price: level.px,
            qty: if SIZE_IN_QUOTE_CURRENCY.get() == Some(&true) {
                (level.sz * level.px).round()
            } else {
                level.sz
            },
        })
        .collect();
    let asks = depth.levels[1]
        .iter()
        .map(|level| Order {
            price: level.px,
            qty: if SIZE_IN_QUOTE_CURRENCY.get() == Some(&true) {
                (level.sz * level.px).round()
            } else {
                level.sz
            },
        })
        .collect();

    Ok(DepthPayload {
        last_update_id: depth.time,
        time: depth.time,
        bids,
        asks,
    })
}
