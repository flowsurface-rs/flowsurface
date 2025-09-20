use crate::{Price, TickMultiplier};

use super::{
    super::{
        Exchange, Kline, MarketKind, SIZE_IN_QUOTE_CURRENCY, StreamKind, Ticker, TickerInfo,
        TickerStats, Timeframe, Trade,
        connect::{State, connect_ws},
        de_string_to_f32,
        depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
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

const ALLOWED_MANTISSA: [i32; 3] = [1, 2, 5];
const SIG_FIG_LIMIT: i32 = 5;

const MULTS_OVERFLOW: &[u16] = &[1, 10, 20, 50, 100, 1000, 10000];
const MULTS_FRACTIONAL: &[u16] = &[1, 2, 5, 10, 100, 1000];

// safe intersection when base_ticksize == 1.0 but we can't disambiguate
const MULTS_SAFE: &[u16] = &[1, 10, 100, 1000];

pub fn allowed_multipliers_for_base_tick(base_ticksize: f32) -> &'static [u16] {
    if base_ticksize < 1.0 {
        // int_digits <= 4 (fractional/boundary region)
        MULTS_FRACTIONAL
    } else if base_ticksize > 1.0 {
        MULTS_OVERFLOW
    } else {
        // base_ticksize == 1.0: could be exactly 5 digits or overflow (>=6).
        MULTS_SAFE
    }
}

pub fn exact_multipliers_for_price(price: f32) -> &'static [u16] {
    if price <= 0.0 {
        return MULTS_FRACTIONAL;
    }
    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };
    if int_digits > SIG_FIG_LIMIT {
        MULTS_OVERFLOW
    } else {
        MULTS_FRACTIONAL
    }
}

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

// Unified structure for both perp and spot asset info
#[derive(Debug, Deserialize)]
struct HyperliquidAssetInfo {
    name: String,
    #[serde(rename = "szDecimals")]
    sz_decimals: u32,
    #[serde(default)] // For perp assets that don't have index
    index: u32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidSpotPair {
    name: String,
    tokens: [u32; 2], // [base_token_index, quote_token_index]
    index: u32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidSpotMeta {
    tokens: Vec<HyperliquidAssetInfo>,
    universe: Vec<HyperliquidSpotPair>,
}

// Unified asset context structure for price/volume data
#[derive(Debug, Deserialize)]
struct HyperliquidAssetContext {
    #[serde(rename = "dayNtlVlm", deserialize_with = "de_string_to_f32")]
    day_notional_volume: f32,
    #[serde(rename = "markPx", deserialize_with = "de_string_to_f32")]
    mark_price: f32,
    #[serde(rename = "midPx", deserialize_with = "de_string_to_f32")]
    mid_price: f32,
    #[serde(rename = "prevDayPx", deserialize_with = "de_string_to_f32")]
    prev_day_price: f32,
    // TODO: Add open interest
    // #[serde(rename = "openInterest", deserialize_with = "de_string_to_f32", default)]
    // open_interest: f32, // Only available for perps
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
    let url = format!("{}/info", API_DOMAIN);

    let (endpoint_type, exchange) = match market {
        MarketKind::LinearPerps => ("metaAndAssetCtxs", Exchange::HyperliquidLinear),
        MarketKind::Spot => ("spotMetaAndAssetCtxs", Exchange::HyperliquidSpot),
        _ => return Ok(HashMap::new()),
    };

    let body = json!({"type": endpoint_type});

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

    // Both endpoints return [metadata, [asset_contexts...]]
    let metadata = response_json
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Missing metadata".to_string()))?;
    let asset_contexts = response_json
        .get(1)
        .and_then(|arr| arr.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing asset contexts array".to_string()))?;

    match market {
        MarketKind::LinearPerps => process_perp_assets(metadata, asset_contexts, exchange).await,
        MarketKind::Spot => process_spot_assets(metadata, asset_contexts, exchange).await,
        _ => unreachable!(),
    }
}

async fn process_perp_assets(
    metadata: &Value,
    asset_contexts: &[Value],
    exchange: Exchange,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let universe = metadata
        .get("universe")
        .and_then(|u| u.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing universe in metadata".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    for (index, asset) in universe.iter().enumerate() {
        if let Ok(asset_info) = serde_json::from_value::<HyperliquidAssetInfo>(asset.clone()) {
            let ticker = Ticker::new(&asset_info.name, exchange);

            if let Some(asset_ctx) = asset_contexts.get(index)
                && let Some(price) = extract_price_from_context(asset_ctx)
            {
                let ticker_info = create_ticker_info(
                    ticker,
                    price,
                    asset_info.sz_decimals,
                    MarketKind::LinearPerps,
                );
                ticker_info_map.insert(ticker, Some(ticker_info));
            }
        }
    }

    Ok(ticker_info_map)
}

async fn process_spot_assets(
    metadata: &Value,
    asset_contexts: &[Value],
    exchange: Exchange,
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let spot_meta: HyperliquidSpotMeta = serde_json::from_value(metadata.clone())
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse spot meta: {}", e)))?;

    let mut ticker_info_map = HashMap::new();

    for pair in &spot_meta.universe {
        if let Some(asset_ctx) = asset_contexts.get(pair.index as usize)
            && let Ok(ctx) = serde_json::from_value::<HyperliquidAssetContext>(asset_ctx.clone())
        {
            let price = if ctx.mid_price > 0.0 {
                ctx.mid_price
            } else {
                ctx.mark_price
            };

            if price > 0.0
                && let Some(base_token) =
                    spot_meta.tokens.iter().find(|t| t.index == pair.tokens[0])
            {
                let display_symbol =
                    create_display_symbol(&pair.name, &spot_meta.tokens, &pair.tokens);

                let ticker = Ticker::new_with_display(&pair.name, exchange, Some(&display_symbol));

                let ticker_info =
                    create_ticker_info(ticker, price, base_token.sz_decimals, MarketKind::Spot);
                ticker_info_map.insert(ticker, Some(ticker_info));
            }
        }
    }

    Ok(ticker_info_map)
}

// Helper functions
fn extract_price_from_context(asset_ctx: &Value) -> Option<f32> {
    ["midPx", "markPx", "oraclePx"].iter().find_map(|k| {
        asset_ctx
            .get(k)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f32>().ok())
    })
}

fn create_ticker_info(
    ticker: Ticker,
    price: f32,
    sz_decimals: u32,
    market: MarketKind,
) -> TickerInfo {
    let tick_size = compute_tick_size(price, sz_decimals, market);
    let min_qty = 10.0_f32.powi(-(sz_decimals as i32));

    TickerInfo::new(ticker, tick_size, min_qty, None)
}

// Helper function to create display symbols
fn create_display_symbol(
    pair_name: &str,
    tokens: &[HyperliquidAssetInfo],
    token_indices: &[u32; 2],
) -> String {
    if pair_name.starts_with('@') {
        // For @index pairs, create symbol from base+quote token names
        let base_token = tokens.iter().find(|t| t.index == token_indices[0]);
        let quote_token = tokens.iter().find(|t| t.index == token_indices[1]);

        if let (Some(base), Some(quote)) = (base_token, quote_token) {
            format!("{}{}", base.name, quote.name)
        } else {
            pair_name.to_string() // Fallback
        }
    } else {
        // For named pairs like "PURR/USDC" → "PURRUSDC"
        pair_name.replace('/', "")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DepthFeedConfig {
    // allowed significant figures (2..=5)
    pub n_sig_figs: Option<i32>,
    // only allowed if n_sig_figs is set
    // can be 1, 2, or 5
    pub mantissa: Option<i32>,
}

impl DepthFeedConfig {
    pub fn new(n_sig_figs: Option<i32>, mantissa: Option<i32>) -> Self {
        Self {
            n_sig_figs,
            mantissa,
        }
    }
    pub fn full_precision() -> Self {
        Self {
            n_sig_figs: None,
            mantissa: None,
        }
    }
    pub fn is_full(&self) -> bool {
        self.n_sig_figs.is_none()
    }
}

impl Default for DepthFeedConfig {
    fn default() -> Self {
        Self {
            n_sig_figs: Some(SIG_FIG_LIMIT),
            mantissa: Some(1),
        }
    }
}

pub fn depth_tick_from_cfg(price: f32, cfg: DepthFeedConfig) -> f32 {
    if price <= 0.0 {
        return 0.0;
    }
    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };

    if cfg.is_full() {
        // server's "full precision"
        if int_digits > SIG_FIG_LIMIT {
            return 1.0;
        }
        if price >= 1.0 {
            let remaining = (SIG_FIG_LIMIT - int_digits).max(0);
            return 10_f32.powi(-remaining);
        } else {
            // price < 1: account for leading zeros before first significant digit
            let lg = price.abs().log10().floor() as i32; // negative
            let leading_zeros = (-lg - 1).max(0);
            let total_decimals = leading_zeros + SIG_FIG_LIMIT;
            return 10_f32.powi(-total_decimals);
        }
    }

    let n_sig = cfg.n_sig_figs.unwrap();

    // significant-figures tick rule
    // n < int_digits  -> coarsen integer part: 10^(int_digits - n)
    // n == int_digits -> 1
    // n > int_digits  -> fractional:
    //   - price >= 1: 10^-(n - int_digits)
    //   - price < 1:  10^-(leading_zeros + (n - int_digits))
    let mut tick = if n_sig < int_digits {
        10_f32.powi(int_digits - n_sig)
    } else if n_sig == int_digits {
        1.0
    } else {
        let frac_power = n_sig - int_digits;
        if price >= 1.0 {
            10_f32.powi(-frac_power)
        } else {
            let lg = price.abs().log10().floor() as i32; // negative
            let leading_zeros = (-lg - 1).max(0);
            10_f32.powi(-(leading_zeros + frac_power))
        }
    };

    if n_sig == SIG_FIG_LIMIT
        && let Some(m) = cfg.mantissa.filter(|m| ALLOWED_MANTISSA.contains(m))
    {
        tick *= m as f32;
    }

    tick
}

// snap to nearest 1–2–5 × 10^k
fn snap_multiplier_to_125(multiplier: u16) -> (i32, i32) {
    // boundaries between {1,2,5,10} in log-space
    const SQRT2: f32 = std::f32::consts::SQRT_2;
    const SQRT10: f32 = 3.162_277_7;
    const SQRT50: f32 = 7.071_068;

    let m = (multiplier as f32).max(1.0);
    let mut kf = m.log10().floor();
    let rem = m / 10_f32.powf(kf);

    // nearest of {1,2,5,10} using boundaries
    let (mantissa, bump) = if rem < SQRT2 {
        (1, false)
    } else if rem < SQRT10 {
        (2, false)
    } else if rem < SQRT50 {
        (5, false)
    } else {
        (1, true) // closer to 10: bump decade
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

    // Decompose multiplier into mantissa ∈ {1,2,5} and decade k
    let (k, m125) = snap_multiplier_to_125(multiplier);

    // Multiplier mapping (unchanged for 10^k):
    // - overflow (int_digits > 5): n = int_digits - k
    // - fractional/boundary (int_digits <= 5): n = 5 - k
    let mut n = if int_digits > SIG_FIG_LIMIT {
        int_digits - k
    } else {
        SIG_FIG_LIMIT - k
    };
    n = n.clamp(2, SIG_FIG_LIMIT);

    // Only set mantissa when n == 5 and m ∈ {2,5}. Otherwise omit.
    let mantissa = if n == SIG_FIG_LIMIT && (m125 == 2 || m125 == 5) {
        Some(m125)
    } else {
        None
    };

    DepthFeedConfig::new(Some(n), mantissa)
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
    let url = format!("{}/info", API_DOMAIN);

    // Step 1: Get all mid prices (contains both perp and spot)
    let mids = fetch_all_mids(&url).await?;

    // Step 2: Get market-specific metadata for stats
    let (metadata_type, exchange) = match market {
        MarketKind::LinearPerps => ("metaAndAssetCtxs", Exchange::HyperliquidLinear),
        MarketKind::Spot => ("spotMetaAndAssetCtxs", Exchange::HyperliquidSpot),
        _ => return Ok(HashMap::new()),
    };

    let metadata = fetch_metadata(&url, metadata_type).await?;

    // Step 3: Process based on market type
    match market {
        MarketKind::LinearPerps => process_perp_ticker_stats(&mids, &metadata, exchange).await,
        MarketKind::Spot => process_spot_ticker_stats(&mids, &metadata, exchange).await,
        _ => unreachable!(),
    }
}

// Shared helper to fetch all mid prices
async fn fetch_all_mids(url: &str) -> Result<HashMap<String, String>, AdapterError> {
    let body = json!({"type": "allMids"});
    let response_text = limiter::http_request_with_limiter(
        url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;

    serde_json::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))
}

// Shared helper to fetch metadata
async fn fetch_metadata(url: &str, metadata_type: &str) -> Result<Value, AdapterError> {
    let body = json!({"type": metadata_type});
    let response_text = limiter::http_request_with_limiter(
        url,
        &HYPERLIQUID_LIMITER,
        1,
        Some(Method::POST),
        Some(&body),
    )
    .await?;

    serde_json::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))
}

async fn process_perp_ticker_stats(
    mids: &HashMap<String, String>,
    metadata: &Value,
    exchange: Exchange,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    // Parse metadata for perps: [meta, [asset_contexts...]]
    let meta = metadata
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Meta data not found".to_string()))?;
    let asset_ctxs = metadata
        .get(1)
        .and_then(|v| v.as_array())
        .ok_or_else(|| AdapterError::ParseError("Asset contexts not found".to_string()))?;
    let universe = meta
        .get("universe")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AdapterError::ParseError("Universe not found".to_string()))?;

    let mut ticker_stats_map = HashMap::new();

    // Process perp symbols from mids (skip spot symbols)
    for (symbol, mid_price_str) in mids {
        // Skip spot symbols: @xxx or PURR/USDC
        if symbol.starts_with('@') || symbol.contains('/') {
            continue;
        }

        let mid_price = mid_price_str
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse mid price".to_string()))?;

        // Find asset in universe and get stats
        if let Some(stats) = find_asset_stats(symbol, universe, asset_ctxs, mid_price)? {
            ticker_stats_map.insert(Ticker::new(symbol, exchange), stats);
        }
    }

    Ok(ticker_stats_map)
}

async fn process_spot_ticker_stats(
    mids: &HashMap<String, String>,
    metadata: &Value,
    exchange: Exchange,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let spot_meta = metadata
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Missing spot meta data".to_string()))?;
    let asset_contexts = metadata
        .get(1)
        .and_then(|arr| arr.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing asset contexts array".to_string()))?;

    let spot_meta: HyperliquidSpotMeta = serde_json::from_value(spot_meta.clone())
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse spot meta: {}", e)))?;

    let mut ticker_stats_map = HashMap::new();

    // use mids for verification
    for pair in &spot_meta.universe {
        if mids.contains_key(&pair.name)
            && let Some(asset_ctx) = asset_contexts.get(pair.index as usize)
            && let Ok(ctx) = serde_json::from_value::<HyperliquidAssetContext>(asset_ctx.clone())
        {
            let display_symbol = create_display_symbol(&pair.name, &spot_meta.tokens, &pair.tokens);

            let daily_price_chg = if ctx.prev_day_price > 0.0 {
                ((ctx.mid_price - ctx.prev_day_price) / ctx.prev_day_price) * 100.0
            } else {
                0.0
            };

            let ticker = Ticker::new_with_display(&pair.name, exchange, Some(&display_symbol));

            ticker_stats_map.insert(
                ticker,
                TickerStats {
                    mark_price: ctx.mark_price,
                    daily_price_chg,
                    daily_volume: ctx.day_notional_volume,
                },
            );
        }
    }

    Ok(ticker_stats_map)
}

// Helper to find asset stats in universe
fn find_asset_stats(
    symbol: &str,
    universe: &[Value],
    asset_ctxs: &[Value],
    mid_price: f32,
) -> Result<Option<TickerStats>, AdapterError> {
    let asset_index = universe.iter().position(|asset| {
        asset
            .get("name")
            .and_then(|n| n.as_str())
            .map(|name| name == symbol)
            .unwrap_or(false)
    });

    if let Some(index) = asset_index
        && let Some(asset_ctx) = asset_ctxs.get(index)
    {
        let prev_day_px = asset_ctx
            .get("prevDayPx")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::ParseError("Previous day price not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| {
                AdapterError::ParseError("Failed to parse previous day price".to_string())
            })?;

        let day_ntl_vlm = asset_ctx
            .get("dayNtlVlm")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::ParseError("Daily volume not found".to_string()))?
            .parse::<f32>()
            .map_err(|_| AdapterError::ParseError("Failed to parse daily volume".to_string()))?;

        let daily_price_chg = if prev_day_px > 0.0 {
            ((mid_price - prev_day_px) / prev_day_px) * 100.0
        } else {
            0.0
        };

        return Ok(Some(TickerStats {
            mark_price: mid_price,
            daily_price_chg,
            daily_volume: day_ntl_vlm,
        }));
    }

    Ok(None)
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

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
    // Use the internal symbol (e.g., "@107" for spot, "BTC" for perps)
    let (symbol_str, _) = ticker.to_full_symbol_and_type();

    log::debug!("Fetching klines for ticker symbol: '{}'", symbol_str);

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

            let kline = Kline::new(
                hl_kline.time,
                hl_kline.open,
                hl_kline.high,
                hl_kline.low,
                hl_kline.close,
                (-1.0, volume),
                ticker_info.min_ticksize,
            );
            klines.push(kline);
        }
    }

    Ok(klines)
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
    ticker_info: TickerInfo,
    tick_multiplier: Option<TickMultiplier>,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let ticker = ticker_info.ticker;
        let exchange = ticker.exchange;

        let mut local_depth_cache = LocalDepthCache::default();
        let mut trades_buffer = Vec::new();

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);
        let user_multiplier = tick_multiplier.unwrap_or(TickMultiplier(1)).0;

        let (symbol_str, _) = ticker.to_full_symbol_and_type();

        log::debug!(
            "Connecting market stream for ticker symbol: '{}'",
            symbol_str
        );

        loop {
            match &mut state {
                State::Disconnected => {
                    let price = match fetch_orderbook(&symbol_str, None).await {
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
                            if let (Some(m), Some(5)) = (depth_cfg.mantissa, depth_cfg.n_sig_figs) {
                                depth_subscription["subscription"]["mantissa"] = json!(m);
                            }

                            log::debug!(
                                "Hyperliquid WS Depth Subscription: {}",
                                serde_json::to_string_pretty(&depth_subscription)
                                    .unwrap_or_else(|_| "Failed to serialize".to_string())
                            );

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

                            log::debug!(
                                "Hyperliquid WS Trades Subscription: {}",
                                serde_json::to_string_pretty(&trades_subscribe_msg)
                                    .unwrap_or_else(|_| "Failed to serialize".to_string())
                            );

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
                                                let price = Price::from_f32(hl_trade.px)
                                                    .round_to_min_tick(ticker_info.min_ticksize);
                                                let qty = if size_in_quote_currency {
                                                    (hl_trade.sz * hl_trade.px).round()
                                                } else {
                                                    hl_trade.sz
                                                };

                                                let trade = Trade {
                                                    time: hl_trade.time,
                                                    is_sell: hl_trade.side == "A", // A for Ask/Sell, B for Bid/Buy
                                                    price,
                                                    qty,
                                                };
                                                trades_buffer.push(trade);
                                            }
                                        }
                                        StreamData::Depth(depth) => {
                                            let bids = depth.levels[0]
                                                .iter()
                                                .map(|level| DeOrder {
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
                                                .map(|level| DeOrder {
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
                                            local_depth_cache.update(
                                                DepthUpdate::Snapshot(depth_payload),
                                                ticker_info.min_ticksize,
                                            );

                                            let stream_kind = StreamKind::DepthAndTrades {
                                                ticker_info,
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
    streams: Vec<(TickerInfo, Timeframe)>,
    _market: MarketKind,
) -> impl Stream<Item = Event> {
    stream::channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let exchange = streams
            .first()
            .map(|(t, _)| t.exchange())
            .unwrap_or(Exchange::HyperliquidLinear);

        let size_in_quote_currency = SIZE_IN_QUOTE_CURRENCY.get() == Some(&true);

        loop {
            match &mut state {
                State::Disconnected => match connect_websocket(WS_DOMAIN, "/ws").await {
                    Ok(mut websocket) => {
                        for (ticker_info, timeframe) in &streams {
                            let ticker = ticker_info.ticker;

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

                            log::debug!(
                                "Hyperliquid WS Kline Subscription: {}",
                                serde_json::to_string_pretty(&subscribe_msg)
                                    .unwrap_or_else(|_| "Failed to serialize".to_string())
                            );

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
                },
                State::Connected(websocket) => match websocket.read_frame().await {
                    Ok(msg) => match msg.opcode {
                        OpCode::Text => {
                            if let Ok(StreamData::Kline(hl_kline)) =
                                parse_websocket_message(&msg.payload)
                            {
                                let ticker_info = streams
                                    .iter()
                                    .find(|(t, _)| t.ticker.as_str() == hl_kline.symbol)
                                    .map(|(t, _)| *t);

                                if let Some(info) = ticker_info {
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

                                    let kline = Kline::new(
                                        hl_kline.time,
                                        hl_kline.open,
                                        hl_kline.high,
                                        hl_kline.low,
                                        hl_kline.close,
                                        (-1.0, volume),
                                        info.min_ticksize,
                                    );

                                    let stream_kind = StreamKind::Kline {
                                        ticker_info: info,
                                        timeframe,
                                    };
                                    let _ =
                                        output.send(Event::KlineReceived(stream_kind, kline)).await;
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

async fn fetch_orderbook(
    symbol: &str,
    cfg: Option<DepthFeedConfig>,
) -> Result<DepthPayload, AdapterError> {
    log::debug!("Fetching orderbook for symbol: '{}'", symbol);
    let url = format!("{}/info", API_DOMAIN);

    let mut body = json!({
        "type": "l2Book",
        "coin": symbol,
    });

    if let Some(cfg) = cfg
        && let Some(obj) = body.as_object_mut()
    {
        if let Some(n) = cfg.n_sig_figs {
            obj.insert("nSigFigs".into(), json!(n));
        }
        // Only send mantissa if:
        // - nSigFigs == 5
        // - mantissa is 2 or 5
        // (mantissa=1 is redundant and can trigger null responses on some assets)
        if let (Some(m), Some(5)) = (cfg.mantissa, cfg.n_sig_figs)
            && m != 1
            && ALLOWED_MANTISSA.contains(&m)
        {
            obj.insert("mantissa".into(), json!(m));
        }
    }

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
        .map(|level| DeOrder {
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
        .map(|level| DeOrder {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn smallest_positive_gap(mut prices: Vec<f32>) -> Option<f32> {
        prices.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let mut best: Option<f32> = None;
        for w in prices.windows(2) {
            if w[0] != w[1] {
                let gap = (w[0] - w[1]).abs();
                if gap > 0.0 && (best.is_none() || gap < best.unwrap()) {
                    best = Some(gap);
                }
            }
        }
        best
    }

    #[tokio::test]
    async fn manual_depth_cfg() {
        let symbol = "BTC";
        let depth_config = DepthFeedConfig::new(Some(5), Some(1));

        let depth = fetch_orderbook(symbol, Some(depth_config))
            .await
            .expect("Failed to fetch orderbook with config");

        for (i, order) in depth.bids.iter().take(5).enumerate() {
            println!("Bid {}: Price: {}", i + 1, order.price);
        }
    }

    #[tokio::test]
    async fn e2e_depth_config_precision() {
        let symbols = ["BTC", "ETH", "HYPE"];
        let multipliers = [1u16, 2u16, 5u16, 10u16, 25u16, 50u16, 100u16];

        // Tolerances for floating errors
        const REL_EPS: f32 = 5e-3; // 0.5%
        const ABS_EPS_MIN: f32 = 1e-6; // floor to ignore tiny fp noise

        for sym in symbols {
            let baseline = fetch_orderbook(sym, None).await.expect("baseline fetch");
            let top_price = match baseline.bids.first() {
                Some(o) => o.price,
                None => continue,
            };
            for m in multipliers {
                let cfg = super::config_from_multiplier(top_price, m);
                let constrained = match fetch_orderbook(sym, Some(cfg)).await {
                    Ok(c) => c,
                    Err(e) => {
                        println!("SYM {sym} m {m} cfg {:?} fetch error: {e}", cfg);
                        continue;
                    }
                };

                let bid_prices: Vec<f32> =
                    constrained.bids.iter().take(25).map(|o| o.price).collect();
                if bid_prices.len() < 2 {
                    println!("SYM {sym} m {m} cfg {:?} insufficient levels", cfg);
                    continue;
                }

                let expected_tick = depth_tick_from_cfg(top_price, cfg);
                if expected_tick == 0.0 {
                    println!("SYM {sym} m {m} cfg {:?} expected_tick=0 skipped", cfg);
                    continue;
                }

                if let Some(gap) = smallest_positive_gap(bid_prices.clone()) {
                    let abs_diff = (gap - expected_tick).abs();
                    let rel_diff = abs_diff / expected_tick;
                    let passes = abs_diff <= ABS_EPS_MIN.max(expected_tick * REL_EPS);

                    let status = if passes { "OK" } else { "DRIFT" };
                    println!(
                        "SYM {sym:>6} m {m:>2} cfg {:?} top_px {:>12.6} exp_tick {:>10.8} gap {:>10.8} abs_diff {:>10.8} rel_diff {:>8.5} {status}",
                        cfg, top_price, expected_tick, gap, abs_diff, rel_diff
                    );
                } else {
                    println!("SYM {sym} m {m} cfg {:?} no distinct gap found", cfg);
                }
            }
        }
    }
}
