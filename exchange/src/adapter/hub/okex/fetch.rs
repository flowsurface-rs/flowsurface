use crate::{
    Kline, OpenInterest, Price, Qty, Ticker, TickerInfo, TickerStats, Timeframe, UnixMs,
    serde_util,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    HttpHub, MarketKind, OkexLimiter, REST_API_BASE, exchange_from_market_type,
    raw_qty_unit_from_market_type, timeframe_to_okx_bar,
};
use crate::adapter::hub::AdapterError;
use serde_json::Value;
use std::collections::HashMap;

pub(super) async fn fetch_ticker_metadata(
    hub: &mut HttpHub<OkexLimiter>,
    markets: &[MarketKind],
) -> Result<super::super::TickerMetadataMap, AdapterError> {
    let mut map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let url = format!("{REST_API_BASE}/public/instruments?instType=SPOT");

        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;
        let doc: Value = serde_json::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let list = doc["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        let exchange = crate::Exchange::OkexSpot;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if item["state"].as_str().unwrap_or("") != "live" {
                continue;
            }

            if item["quoteCcy"].as_str() != Some("USDT") {
                continue;
            }

            if !exchange.is_symbol_supported(symbol, true) {
                continue;
            }

            let min_ticksize = serde_util::value_as_f32(&item["tickSz"])
                .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;
            let min_qty = serde_util::value_as_f32(&item["lotSz"])
                .ok_or_else(|| AdapterError::ParseError("Lot size not found".to_string()))?;

            let ticker = Ticker::new(symbol, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, None);

            map.insert(ticker, Some(info));
        }
    }

    if include_perps {
        let url = format!("{REST_API_BASE}/public/instruments?instType=SWAP");

        let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;
        let doc: Value = serde_json::from_str(&response_text)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let list = doc["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if item["state"].as_str().unwrap_or("") != "live" {
                continue;
            }

            let market_kind = match item["ctType"].as_str() {
                Some("linear") => {
                    if item["settleCcy"].as_str() != Some("USDT") {
                        continue;
                    }
                    MarketKind::LinearPerps
                }
                Some("inverse") => MarketKind::InversePerps,
                _ => continue,
            };

            if !markets.contains(&market_kind) {
                continue;
            }

            let exchange = exchange_from_market_type(market_kind);

            if !exchange.is_symbol_supported(symbol, true) {
                continue;
            }

            let min_ticksize = serde_util::value_as_f32(&item["tickSz"])
                .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;
            let min_qty = serde_util::value_as_f32(&item["lotSz"])
                .ok_or_else(|| AdapterError::ParseError("Lot size not found".to_string()))?;
            let contract_size = serde_util::value_as_f32(&item["ctVal"]);

            let ticker = Ticker::new(symbol, exchange);
            let info = TickerInfo::new(ticker, min_ticksize, min_qty, contract_size);

            map.insert(ticker, Some(info));
        }
    }

    Ok(map)
}

pub(super) async fn fetch_ticker_stats(
    hub: &mut HttpHub<OkexLimiter>,
    markets: &[MarketKind],
) -> Result<super::super::TickerStatsMap, AdapterError> {
    let mut map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps =
        markets.contains(&MarketKind::LinearPerps) || markets.contains(&MarketKind::InversePerps);

    if include_spot {
        let url = format!("{REST_API_BASE}/market/tickers?instType=SPOT");

        let parsed_response: Value = hub.http_json_with_limiter(&url, 1, None, None).await?;

        let list = parsed_response["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        let exchange = crate::Exchange::OkexSpot;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            let last_trade_price = serde_util::value_as_f32(&item["last"]);
            let open24h = serde_util::value_as_f32(&item["open24h"]);
            let Some(vol24h) = serde_util::value_as_f32(&item["volCcy24h"]) else {
                continue;
            };

            let (last_price, previous_daily_open) =
                if let (Some(last), Some(previous_daily_open)) = (last_trade_price, open24h) {
                    (last, previous_daily_open)
                } else {
                    continue;
                };

            let daily_price_chg = if previous_daily_open > 0.0 {
                (last_price - previous_daily_open) / previous_daily_open * 100.0
            } else {
                0.0
            };

            map.insert(
                Ticker::new(symbol, exchange),
                TickerStats {
                    mark_price: Price::from_f32(last_price),
                    daily_price_chg,
                    daily_volume: Qty::from_f32(vol24h),
                },
            );
        }
    }

    if include_perps {
        let url = format!("{REST_API_BASE}/market/tickers?instType=SWAP");

        let parsed_response: Value = hub.http_json_with_limiter(&url, 1, None, None).await?;

        let list = parsed_response["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in list {
            let symbol = match item["instId"].as_str() {
                Some(s) => s,
                None => continue,
            };

            let perps_market_symbol = if symbol.ends_with("-USDT-SWAP") {
                Some(MarketKind::LinearPerps)
            } else if symbol.ends_with("-USD-SWAP") {
                Some(MarketKind::InversePerps)
            } else {
                None
            };

            let Some(perps_market) = perps_market_symbol else {
                continue;
            };

            if !markets.contains(&perps_market) {
                continue;
            }

            let exchange = exchange_from_market_type(perps_market);

            if !exchange.is_symbol_supported(symbol, false) {
                continue;
            }

            let last_trade_price = serde_util::value_as_f32(&item["last"]);
            let open24h = serde_util::value_as_f32(&item["open24h"]);

            let Some(vol24h) = serde_util::value_as_f32(&item["volCcy24h"]) else {
                continue;
            };

            let (last_price, previous_daily_open) =
                if let (Some(last), Some(previous_daily_open)) = (last_trade_price, open24h) {
                    (last, previous_daily_open)
                } else {
                    continue;
                };
            let daily_price_chg = if previous_daily_open > 0.0 {
                (last_price - previous_daily_open) / previous_daily_open * 100.0
            } else {
                0.0
            };

            map.insert(
                Ticker::new(symbol, exchange),
                TickerStats {
                    mark_price: Price::from_f32(last_price),
                    daily_price_chg,
                    daily_volume: Qty::from_f32(vol24h * last_price),
                },
            );
        }
    }

    Ok(map)
}

pub(super) async fn fetch_klines(
    hub: &mut HttpHub<OkexLimiter>,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(UnixMs, UnixMs)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let bar = timeframe_to_okx_bar(timeframe).ok_or_else(|| {
        AdapterError::InvalidRequest(format!("Unsupported timeframe: {timeframe}"))
    })?;

    let mut url = format!(
        "{REST_API_BASE}/market/history-candles?instId={}&bar={}&limit={}",
        symbol_str,
        bar,
        match range {
            Some((start, end)) => {
                ((end.as_u64() - start.as_u64()) / timeframe.to_milliseconds()).clamp(1, 300)
            }
            None => 300,
        }
    );

    if let Some((start, end)) = range {
        url.push_str(&format!(
            "&before={}&after={}",
            start.as_u64(),
            end.as_u64()
        ));
    }

    let doc: Value = hub.http_json_with_limiter(&url, 1, None, None).await?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Kline result is not an array".to_string()))?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let mut klines: Vec<Kline> = Vec::with_capacity(list.len());

    for row in list {
        let time = row.get(0).and_then(serde_util::value_as_u64);
        let open = row.get(1).and_then(serde_util::value_as_f32);
        let high = row.get(2).and_then(serde_util::value_as_f32);
        let low = row.get(3).and_then(serde_util::value_as_f32);
        let close = row.get(4).and_then(serde_util::value_as_f32);
        let volume = row.get(5).and_then(serde_util::value_as_f32);

        let (ts, open, high, low, close) = match (time, open, high, low, close) {
            (Some(ts), Some(o), Some(h), Some(l), Some(c)) => (ts, o, h, l, c),
            _ => continue,
        };
        let volume_in_display = if let Some(vq) = volume {
            qty_norm.normalize_qty(vq, close)
        } else {
            qty_norm.normalize_qty(0.0, close)
        };

        let kline = Kline::new(
            ts,
            open,
            high,
            low,
            close,
            crate::Volume::TotalOnly(volume_in_display),
            ticker_info.min_ticksize,
        );

        klines.push(kline);
    }

    klines.sort_by_key(|k| k.time);
    Ok(klines)
}

pub(super) async fn fetch_historical_oi(
    hub: &mut HttpHub<OkexLimiter>,
    ticker_info: TickerInfo,
    range: Option<(UnixMs, UnixMs)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let (ticker_str, _market) = ticker_info.ticker.to_full_symbol_and_type();

    let bar = timeframe_to_okx_bar(period)
        .ok_or_else(|| AdapterError::InvalidRequest(format!("Unsupported timeframe: {period}")))?;

    let mut url = format!(
        "{REST_API_BASE}/rubik/stat/contracts/open-interest-history?instId={ticker_str}&period={bar}"
    );

    if let Some((start, end)) = range {
        url.push_str(&format!("&begin={}&end={}", start.as_u64(), end.as_u64()));
    }

    let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

    let doc: Value = serde_json::from_str(&response_text)
        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let list = doc["data"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Fetch result is not an array".to_string()))?;

    let open_interest: Vec<OpenInterest> = list
        .iter()
        .filter_map(|row| {
            let arr = row.as_array()?;
            let ts = serde_util::value_as_u64(arr.first()?)?;
            let oi_ccy = serde_util::value_as_f32(arr.get(2)?)?;
            Some(OpenInterest {
                time: ts.into(),
                value: oi_ccy,
            })
        })
        .collect();

    Ok(open_interest)
}
