use crate::{
    Kline, Price, Qty, Ticker, TickerInfo, TickerStats, Timeframe, UnixMs, Volume,
    adapter::MarketKind,
    depth::{DeOrder, DepthPayload},
    serde_util::{self, de_string_to_number},
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    FETCH_DOMAIN, HttpHub, MexcLimiter, convert_to_mexc_timeframe, exchange_from_market_type,
    mexc_perps_market_from_symbol, raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct FuturesApiResponse {
    success: bool,
    code: u8,
    data: Value,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct KlineSpot {
    #[serde()]
    open_ts: u64,
    #[serde(deserialize_with = "de_string_to_number")]
    open: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    high: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    low: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    close: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    vol: f32,
    #[serde()]
    close_ts: u64,
    #[serde(deserialize_with = "de_string_to_number")]
    asset_vol: f32,
}

#[derive(Deserialize)]
struct DepthSnapshotResponse {
    #[serde(rename = "data")]
    data: DepthData,
}

#[derive(Deserialize)]
struct DepthData {
    #[serde(rename = "asks")]
    asks: Vec<DeOrder>,
    #[serde(rename = "bids")]
    bids: Vec<DeOrder>,
    #[serde(rename = "version")]
    version: u64,
    #[serde(rename = "timestamp")]
    timestamp: u64,
}

pub(super) async fn fetch_depth_snapshot(
    hub: &mut HttpHub<MexcLimiter>,
    ticker: Ticker,
) -> Result<DepthPayload, AdapterError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    if market_type == MarketKind::Spot {
        return Err(AdapterError::InvalidRequest(
            "MEXC spot depth snapshot is not supported in this stream".to_string(),
        ));
    }

    let url = format!("{FETCH_DOMAIN}/v1/contract/depth/{symbol_str}");
    let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;
    let snapshot: DepthSnapshotResponse =
        sonic_rs::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    Ok(DepthPayload {
        last_update_id: snapshot.data.version,
        time: UnixMs(snapshot.data.timestamp),
        bids: snapshot.data.bids,
        asks: snapshot.data.asks,
    })
}

pub(super) async fn fetch_ticker_metadata(
    hub: &mut HttpHub<MexcLimiter>,
    markets: &[MarketKind],
) -> Result<super::super::TickerMetadataMap, AdapterError> {
    let mut ticker_info_map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let url = format!("{FETCH_DOMAIN}/v3/exchangeInfo");

        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;
        let exchange_info: Value = sonic_rs::from_str(&response_text).map_err(|e| {
            AdapterError::ParseError(format!("Failed to parse MEXC exchange info: {e}"))
        })?;

        let symbols = exchange_info["symbols"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Missing symbols array".to_string()))?;

        let exchange = exchange_from_market_type(MarketKind::Spot);

        for item in symbols {
            if let Some(status) = item["status"].as_str()
                && status != "1"
                && status != "2"
            {
                continue;
            }

            let symbol_str = item["symbol"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Missing symbol".to_string()))?;

            if !exchange.is_symbol_supported(symbol_str, true) {
                continue;
            }

            if let Some(quote_asset) = item["quoteAsset"].as_str()
                && quote_asset != "USDT"
                && quote_asset != "USD"
            {
                continue;
            }

            let min_qty = serde_util::value_as_f32(&item["baseSizePrecision"])
                .ok_or_else(|| AdapterError::ParseError("Missing baseSizePrecision".to_string()))?;

            let quote_asset_precision = item["quoteAssetPrecision"].as_i64().ok_or_else(|| {
                AdapterError::ParseError("Missing quoteAssetPrecision".to_string())
            })?;

            let min_ticksize = 10f32.powi(-quote_asset_precision as i32);

            let ticker = Ticker::new(symbol_str, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, None);
            ticker_info_map.insert(ticker, Some(info));
        }
    }

    if include_perps {
        let url = format!("{FETCH_DOMAIN}/v1/contract/detail");

        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;
        let exchange_info: Value = sonic_rs::from_str(&response_text).map_err(|e| {
            AdapterError::ParseError(format!("Failed to parse MEXC exchange info: {e}"))
        })?;

        let result_list: &Vec<Value> = exchange_info["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in result_list {
            if let Some(state) = item["state"].as_i64()
                && state != 0
            {
                continue;
            }

            let symbol = item["symbol"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Missing symbol".to_string()))?;

            let Some(quote_asset) = item["quoteCoin"].as_str() else {
                return Err(AdapterError::ParseError("Missing quoteCoin".to_string()));
            };

            if quote_asset != "USDT" && quote_asset != "USD" {
                continue;
            }

            let settle_asset = item["settleCoin"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Missing settleCoin".to_string()))?;

            let base_asset = item["baseCoin"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Missing baseCoin".to_string()))?;

            let perps_market = if settle_asset == base_asset {
                MarketKind::InversePerps
            } else if settle_asset == quote_asset {
                MarketKind::LinearPerps
            } else {
                return Err(AdapterError::ParseError(
                    "Unknown contract type".to_string(),
                ));
            };

            if !markets.contains(&perps_market) {
                continue;
            }

            let exchange = exchange_from_market_type(perps_market);

            if !exchange.is_symbol_supported(symbol, true) {
                continue;
            }

            let min_qty_contracts = item["minVol"]
                .as_f64()
                .ok_or_else(|| AdapterError::ParseError("Missing minVol (min_qty)".to_string()))?
                as f32;

            let min_ticksize = item["priceUnit"].as_f64().ok_or_else(|| {
                AdapterError::ParseError("Missing priceUnit (ticksize)".to_string())
            })? as f32;

            let contract_size = item["contractSize"]
                .as_f64()
                .ok_or_else(|| AdapterError::ParseError("Missing contractSize".to_string()))?
                as f32;

            let min_qty = min_qty_contracts * contract_size;

            let ticker = Ticker::new(symbol, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, Some(contract_size));
            ticker_info_map.insert(ticker, Some(info));
        }
    }

    Ok(ticker_info_map)
}

pub(super) async fn fetch_ticker_stats(
    hub: &mut HttpHub<MexcLimiter>,
    markets: &[MarketKind],
    contract_sizes: Option<&HashMap<Ticker, f32>>,
) -> Result<super::super::TickerStatsMap, AdapterError> {
    let mut ticker_prices_map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let exchange = exchange_from_market_type(MarketKind::Spot);
        let url = format!("{FETCH_DOMAIN}/v3/ticker/24hr");
        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

        let parsed_response: Value = sonic_rs::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let result_list: &Vec<Value> = parsed_response
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Data is not an array".to_string()))?;

        for item in result_list {
            let symbol = item["symbol"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            if !symbol.ends_with("USDT") {
                continue;
            }

            let last_price = serde_util::value_as_f32(&item["lastPrice"])
                .ok_or_else(|| AdapterError::ParseError("Last price not found".to_string()))?;

            let price_change_percent = serde_util::value_as_f32(&item["priceChangePercent"])
                .ok_or_else(|| {
                    AdapterError::ParseError("Price change percent not found".to_string())
                })?;

            let volume = serde_util::value_as_f32(&item["volume"])
                .ok_or_else(|| AdapterError::ParseError("Volume not found".to_string()))?;

            let volume_in_usd = if let Some(qv) = serde_util::value_as_f32(&item["quoteVolume"]) {
                qv
            } else {
                volume * last_price
            };

            let daily_price_chg = price_change_percent * 100.0;

            let ticker_stats = TickerStats {
                mark_price: Price::from_f32(last_price),
                daily_price_chg,
                daily_volume: Qty::from_f32(volume_in_usd),
            };

            ticker_prices_map.insert(Ticker::new(symbol, exchange), ticker_stats);
        }
    }

    if include_perps {
        let url = format!("{FETCH_DOMAIN}/v1/contract/ticker");
        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

        let parsed_response: Value = sonic_rs::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let result_list: &Vec<Value> = parsed_response["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Data is not an array".to_string()))?;

        for item in result_list {
            let symbol = item["symbol"]
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

            let Some(perps_market) = mexc_perps_market_from_symbol(symbol, contract_sizes) else {
                continue;
            };

            if !markets.contains(&perps_market) {
                continue;
            }

            let exchange = exchange_from_market_type(perps_market);

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            let ticker = Ticker::new(symbol, exchange);
            let contract_size = contract_sizes.and_then(|sizes| sizes.get(&ticker)).copied();

            let Some(cs) = contract_size else {
                continue;
            };

            let last_price = serde_util::value_as_f32(&item["lastPrice"])
                .ok_or_else(|| AdapterError::ParseError("Last price not found".to_string()))?;

            let rise_fall_rate = serde_util::value_as_f32(&item["riseFallRate"])
                .ok_or_else(|| AdapterError::ParseError("Missing riseFallRate".to_string()))?;

            let volume_24 = serde_util::value_as_f32(&item["volume24"])
                .ok_or_else(|| AdapterError::ParseError("Missing volume24".to_string()))?;

            let volume_in_usd = if perps_market == MarketKind::InversePerps {
                volume_24 * cs
            } else {
                volume_24 * cs * last_price
            };

            let daily_price_chg = rise_fall_rate * 100.0;

            let ticker_stats = TickerStats {
                mark_price: Price::from_f32(last_price),
                daily_price_chg,
                daily_volume: Qty::from_f32(volume_in_usd),
            };

            ticker_prices_map.insert(ticker, ticker_stats);
        }
    }

    Ok(ticker_prices_map)
}

pub(super) async fn fetch_klines(
    hub: &mut HttpHub<MexcLimiter>,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(UnixMs, UnixMs)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    let timeframe_str = convert_to_mexc_timeframe(timeframe, market_type).ok_or_else(|| {
        AdapterError::InvalidRequest(format!(
            "Unsupported MEXC kline timeframe {timeframe} for {market_type}"
        ))
    })?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let mut url = match market_type {
        MarketKind::Spot => format!(
            "{FETCH_DOMAIN}/v3/klines?symbol={}&interval={}",
            symbol_str.to_uppercase(),
            timeframe_str
        ),
        MarketKind::LinearPerps | MarketKind::InversePerps => format!(
            "{FETCH_DOMAIN}/v1/contract/kline/{}?interval={}",
            symbol_str.to_uppercase(),
            timeframe_str
        ),
    };

    if let Some((start_ms, end_ms)) = range {
        let start_ms = start_ms.as_u64();
        let end_ms = end_ms.as_u64();
        match market_type {
            MarketKind::Spot => {
                url.push_str(&format!("&startTime={}&endTime={}", start_ms, end_ms));
            }
            MarketKind::LinearPerps | MarketKind::InversePerps => {
                let start_sec = start_ms / 1000;
                let end_sec = end_ms / 1000;
                url.push_str(&format!("&start={}&end={}", start_sec, end_sec));
            }
        }
    }

    let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

    let klines_result: Result<Vec<Kline>, AdapterError> = match market_type {
        MarketKind::Spot => {
            let parsed_response: Vec<KlineSpot> = sonic_rs::from_str(&response_text)
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            parsed_response
                .iter()
                .map(|kline| {
                    let volume = qty_norm.normalize_qty(kline.vol, kline.close);
                    Ok(Kline::new(
                        kline.close_ts,
                        kline.open,
                        kline.high,
                        kline.low,
                        kline.close,
                        Volume::TotalOnly(volume),
                        ticker_info.min_ticksize,
                    ))
                })
                .collect()
        }
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            let parsed_response: FuturesApiResponse = sonic_rs::from_str(&response_text)
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;

            let data = &parsed_response.data;
            let times = data["time"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Time array not found".to_string()))?;
            let opens = data["open"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Open array not found".to_string()))?;
            let highs = data["high"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("High array not found".to_string()))?;
            let lows = data["low"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Low array not found".to_string()))?;
            let closes = data["close"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Close array not found".to_string()))?;
            let amounts = data["amount"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Amount array not found".to_string()))?;
            let volumes = data["vol"]
                .as_array()
                .ok_or_else(|| AdapterError::ParseError("Vol array not found".to_string()))?;

            (0..times.len())
                .map(|i| {
                    let timestamp = times[i].as_u64().ok_or_else(|| {
                        AdapterError::ParseError("Time value not found".to_string())
                    })? * 1000;

                    let open = opens[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("Open value not found".to_string())
                    })? as f32;
                    let high = highs[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("High value not found".to_string())
                    })? as f32;
                    let low = lows[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("Low value not found".to_string())
                    })? as f32;
                    let close = closes[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("Close value not found".to_string())
                    })? as f32;
                    let _amount = amounts[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("Amount value not found".to_string())
                    })? as f32;
                    let volume = volumes[i].as_f64().ok_or_else(|| {
                        AdapterError::ParseError("Vol value not found".to_string())
                    })? as f32;

                    let normalized_vol = qty_norm.normalize_qty(volume, close);

                    Ok(Kline::new(
                        timestamp,
                        open,
                        high,
                        low,
                        close,
                        Volume::TotalOnly(normalized_vol),
                        ticker_info.min_ticksize,
                    ))
                })
                .collect()
        }
    };

    klines_result
}
