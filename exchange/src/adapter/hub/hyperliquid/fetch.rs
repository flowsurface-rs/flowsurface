use crate::{
    Kline, Price, Qty, Ticker, TickerInfo, TickerStats, Timeframe, Volume,
    adapter::{Exchange, MarketKind},
    depth::{DeOrder, DepthPayload},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    API_DOMAIN, HttpHub, HyperliquidLimiter, MAX_DECIMALS_PERP, SIG_FIG_LIMIT,
    raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use reqwest::Method;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct HyperliquidAssetInfo {
    name: String,
    #[serde(rename = "szDecimals")]
    sz_decimals: u32,
    #[serde(default)]
    index: u32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidSpotPair {
    name: String,
    tokens: [u32; 2],
    index: u32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidSpotMeta {
    tokens: Vec<HyperliquidAssetInfo>,
    universe: Vec<HyperliquidSpotPair>,
}

#[derive(Debug, Deserialize)]
struct HyperliquidAssetContext {
    #[serde(rename = "dayNtlVlm", deserialize_with = "de_string_to_number")]
    day_notional_volume: f32,
    #[serde(rename = "markPx", deserialize_with = "de_string_to_number")]
    mark_price: f32,
    #[serde(rename = "midPx", deserialize_with = "de_string_to_number")]
    mid_price: f32,
    #[serde(rename = "prevDayPx", deserialize_with = "de_string_to_number")]
    prev_day_price: f32,
}

impl HyperliquidAssetContext {
    fn price(&self) -> f32 {
        if self.mid_price > 0.0 {
            self.mid_price
        } else {
            self.mark_price
        }
    }
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
    #[serde(rename = "o", deserialize_with = "de_string_to_number")]
    open: f32,
    #[serde(rename = "h", deserialize_with = "de_string_to_number")]
    high: f32,
    #[serde(rename = "l", deserialize_with = "de_string_to_number")]
    low: f32,
    #[serde(rename = "c", deserialize_with = "de_string_to_number")]
    close: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_number")]
    volume: f32,
    #[serde(rename = "n")]
    trade_count: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidDepth {
    levels: [Vec<HyperliquidLevel>; 2],
    time: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidLevel {
    #[serde(deserialize_with = "de_string_to_number")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    sz: f32,
}

type TickerMetadata = (
    HashMap<Ticker, Option<TickerInfo>>,
    HashMap<Ticker, TickerStats>,
);

async fn post_info<T: DeserializeOwned>(
    hub: &mut HttpHub<HyperliquidLimiter>,
    body: &Value,
) -> Result<T, AdapterError> {
    let url = format!("{}/info", API_DOMAIN);
    let response_text = hub
        .http_text_with_limiter(&url, 1, Some(Method::POST), Some(body))
        .await?;

    serde_json::from_str::<T>(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))
}

async fn fetch_metadata(
    hub: &mut HttpHub<HyperliquidLimiter>,
    market: MarketKind,
) -> Result<TickerMetadata, AdapterError> {
    match market {
        MarketKind::LinearPerps => fetch_perps_metadata(hub).await,
        MarketKind::Spot => fetch_spot_metadata(hub).await,
        _ => Err(AdapterError::InvalidRequest(format!(
            "Hyperliquid metadata fetch not supported for {market:?}"
        ))),
    }
}

async fn fetch_meta_for_dex(
    hub: &mut HttpHub<HyperliquidLimiter>,
    dex_name: Option<&str>,
) -> Result<TickerMetadata, AdapterError> {
    let body = match dex_name {
        Some(name) => json!({ "type": "metaAndAssetCtxs", "dex": name }),
        None => json!({ "type": "metaAndAssetCtxs" }),
    };

    let response_json: Value = post_info(hub, &body).await?;

    let metadata = response_json
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Missing metadata".to_string()))?;
    let asset_contexts = response_json
        .get(1)
        .and_then(|arr| arr.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing asset contexts array".to_string()))?;

    process_perp_assets(metadata, asset_contexts, Exchange::HyperliquidLinear)
}

async fn fetch_perps_metadata(
    hub: &mut HttpHub<HyperliquidLimiter>,
) -> Result<TickerMetadata, AdapterError> {
    let dexes_json: Value = post_info(hub, &json!({ "type": "perpDexs" })).await?;

    let dexes = dexes_json
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Missing dexes array".to_string()))?;

    let dex_names: Vec<Option<String>> = dexes
        .iter()
        .map(|dex| match dex {
            Value::Null => None,
            _ => dex.get("name").and_then(|n| n.as_str()).map(str::to_owned),
        })
        .collect();

    let mut combined_info = HashMap::new();
    let mut combined_stats = HashMap::new();

    for dex_name in dex_names {
        match fetch_meta_for_dex(hub, dex_name.as_deref()).await {
            Ok((info_map, stats_map)) => {
                combined_info.extend(info_map);
                combined_stats.extend(stats_map);
            }
            Err(e) => {
                log::warn!(
                    "Failed to fetch metadata for DEX {:?}: {}",
                    dex_name.as_deref().unwrap_or("default"),
                    e
                );
            }
        }
    }

    Ok((combined_info, combined_stats))
}

async fn fetch_spot_metadata(
    hub: &mut HttpHub<HyperliquidLimiter>,
) -> Result<TickerMetadata, AdapterError> {
    let body = json!({"type": "spotMetaAndAssetCtxs"});
    let response_json: Value = post_info(hub, &body).await?;

    let metadata = response_json
        .get(0)
        .ok_or_else(|| AdapterError::ParseError("Missing metadata".to_string()))?;
    let asset_contexts = response_json
        .get(1)
        .and_then(|arr| arr.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing asset contexts array".to_string()))?;

    process_spot_assets(metadata, asset_contexts, Exchange::HyperliquidSpot)
}

fn insert_ticker_from_ctx(
    ticker: Ticker,
    sz_decimals: u32,
    ctx: &HyperliquidAssetContext,
    ticker_info_map: &mut HashMap<Ticker, Option<TickerInfo>>,
    ticker_stats_map: &mut HashMap<Ticker, TickerStats>,
) {
    let price = ctx.price();
    if price <= 0.0 {
        return;
    }

    let ticker_info = create_ticker_info(ticker, price, sz_decimals);
    ticker_info_map.insert(ticker, Some(ticker_info));

    ticker_stats_map.insert(
        ticker,
        TickerStats {
            mark_price: Price::from_f32(ctx.mark_price),
            daily_price_chg: daily_price_chg_pct(price, ctx.prev_day_price),
            daily_volume: Qty::from_f32(ctx.day_notional_volume),
        },
    );
}

fn process_perp_assets(
    metadata: &Value,
    asset_contexts: &[Value],
    exchange: Exchange,
) -> Result<TickerMetadata, AdapterError> {
    let universe = metadata
        .get("universe")
        .and_then(|u| u.as_array())
        .ok_or_else(|| AdapterError::ParseError("Missing universe in metadata".to_string()))?;

    let mut ticker_info_map = HashMap::new();
    let mut ticker_stats_map = HashMap::new();

    for (index, asset) in universe.iter().enumerate() {
        if let Ok(asset_info) = serde_json::from_value::<HyperliquidAssetInfo>(asset.clone())
            && let Some(asset_ctx) = asset_contexts.get(index)
            && let Ok(ctx) = serde_json::from_value::<HyperliquidAssetContext>(asset_ctx.clone())
        {
            let ticker = Ticker::new(&asset_info.name, exchange);
            insert_ticker_from_ctx(
                ticker,
                asset_info.sz_decimals,
                &ctx,
                &mut ticker_info_map,
                &mut ticker_stats_map,
            );
        }
    }

    Ok((ticker_info_map, ticker_stats_map))
}

fn process_spot_assets(
    metadata: &Value,
    asset_contexts: &[Value],
    exchange: Exchange,
) -> Result<TickerMetadata, AdapterError> {
    let spot_meta: HyperliquidSpotMeta = serde_json::from_value(metadata.clone())
        .map_err(|e| AdapterError::ParseError(format!("Failed to parse spot meta: {}", e)))?;

    let mut ticker_info_map = HashMap::new();
    let mut ticker_stats_map = HashMap::new();

    for pair in &spot_meta.universe {
        if let Some(asset_ctx) = asset_contexts.get(pair.index as usize)
            && let Ok(ctx) = serde_json::from_value::<HyperliquidAssetContext>(asset_ctx.clone())
            && let Some(base_token) = spot_meta.tokens.iter().find(|t| t.index == pair.tokens[0])
        {
            let display_symbol = create_display_symbol(&pair.name, &spot_meta.tokens, &pair.tokens);
            let ticker = Ticker::new_with_display(&pair.name, exchange, Some(&display_symbol));

            insert_ticker_from_ctx(
                ticker,
                base_token.sz_decimals,
                &ctx,
                &mut ticker_info_map,
                &mut ticker_stats_map,
            );
        }
    }

    Ok((ticker_info_map, ticker_stats_map))
}

fn create_ticker_info(ticker: Ticker, price: f32, sz_decimals: u32) -> TickerInfo {
    let market = ticker.market_type();

    let tick_size = compute_tick_size(price, sz_decimals, market);
    let min_qty = 10.0_f32.powi(-(sz_decimals as i32));

    TickerInfo::new(ticker, tick_size, min_qty, None)
}

fn create_display_symbol(
    pair_name: &str,
    tokens: &[HyperliquidAssetInfo],
    token_indices: &[u32; 2],
) -> String {
    if pair_name.starts_with('@') {
        let base_token = tokens.iter().find(|t| t.index == token_indices[0]);
        let quote_token = tokens.iter().find(|t| t.index == token_indices[1]);

        if let (Some(base), Some(quote)) = (base_token, quote_token) {
            format!("{}{}", base.name, quote.name)
        } else {
            pair_name.to_string()
        }
    } else {
        pair_name.replace('/', "")
    }
}

fn daily_price_chg_pct(price: f32, prev_day_price: f32) -> f32 {
    if prev_day_price > 0.0 {
        ((price - prev_day_price) / prev_day_price) * 100.0
    } else {
        0.0
    }
}

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

    if price >= 1.0 {
        let remaining_sig = (SIG_FIG_LIMIT - int_digits).max(0);
        if remaining_sig == 0 || decimal_cap == 0 {
            1.0
        } else {
            10_f32.powi(-remaining_sig.min(decimal_cap))
        }
    } else {
        let lg = price.abs().log10().floor() as i32;
        let leading_zeros = (-lg - 1).max(0);
        let total_decimals = (leading_zeros + SIG_FIG_LIMIT).min(decimal_cap);
        if total_decimals <= 0 {
            1.0
        } else {
            10_f32.powi(-total_decimals)
        }
    }
}

pub(super) async fn fetch_depth_snapshot(
    hub: &mut HttpHub<HyperliquidLimiter>,
    ticker: Ticker,
) -> Result<DepthPayload, AdapterError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    if market_type == MarketKind::InversePerps {
        return Err(AdapterError::InvalidRequest(
            "Hyperliquid inverse market is not supported".to_string(),
        ));
    }

    let url = format!("{}/info", API_DOMAIN);
    let body = json!({
        "type": "l2Book",
        "coin": symbol_str,
    });

    let response_text = hub
        .http_text_with_limiter(&url, 1, Some(Method::POST), Some(&body))
        .await?;

    let depth: HyperliquidDepth = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let bids = depth.levels[0]
        .iter()
        .map(|level| DeOrder {
            price: level.px,
            qty: level.sz,
        })
        .collect();
    let asks = depth.levels[1]
        .iter()
        .map(|level| DeOrder {
            price: level.px,
            qty: level.sz,
        })
        .collect();

    Ok(DepthPayload {
        last_update_id: depth.time,
        time: depth.time,
        bids,
        asks,
    })
}

pub(super) async fn fetch_ticker_metadata(
    hub: &mut HttpHub<HyperliquidLimiter>,
    market: MarketKind,
) -> Result<super::super::TickerMetadataMap, AdapterError> {
    let (ticker_info_map, _) = fetch_metadata(hub, market).await?;
    Ok(ticker_info_map)
}

pub(super) async fn fetch_ticker_stats(
    hub: &mut HttpHub<HyperliquidLimiter>,
    market: MarketKind,
) -> Result<super::super::TickerStatsMap, AdapterError> {
    let (_, ticker_stats_map) = fetch_metadata(hub, market).await?;
    Ok(ticker_stats_map)
}

pub(super) async fn fetch_klines(
    hub: &mut HttpHub<HyperliquidLimiter>,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;
    let interval = timeframe.to_string();

    let url = format!("{}/info", API_DOMAIN);
    let (symbol_str, _) = ticker.to_full_symbol_and_type();

    let (start_time, end_time) = if let Some((start, end)) = range {
        (start, end)
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let interval_ms = timeframe.to_milliseconds();
        let candles_ago = now - (interval_ms * 500);
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

    let klines_data: Vec<Value> = hub
        .http_json_with_limiter(&url, 1, Some(Method::POST), Some(&body))
        .await?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(ticker_info.market_type()),
    );

    let mut klines = Vec::new();
    for kline_data in klines_data {
        if let Ok(hl_kline) = serde_json::from_value::<HyperliquidKline>(kline_data) {
            let volume = qty_norm.normalize_qty(hl_kline.volume, hl_kline.close);

            let kline = Kline::new(
                hl_kline.time,
                hl_kline.open,
                hl_kline.high,
                hl_kline.low,
                hl_kline.close,
                Volume::TotalOnly(volume),
                ticker_info.min_ticksize,
            );
            klines.push(kline);
        }
    }

    Ok(klines)
}
