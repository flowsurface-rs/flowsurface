use crate::{
    Kline, OpenInterest, Price, Qty, Ticker, TickerInfo, TickerStats, Timeframe, UnixMs,
    adapter::hub::TickerMetadataMap,
    serde_util,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    BybitLimiter, FETCH_DOMAIN, HttpHub, MarketKind, exchange_from_market_type,
    raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(
        rename = "openInterest",
        deserialize_with = "serde_util::de_string_to_number"
    )]
    pub value: f32,
    #[serde(deserialize_with = "serde_util::de_string_to_number")]
    pub timestamp: u64,
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

fn parse_kline_field<T: std::str::FromStr>(field: Option<&str>) -> Result<T, AdapterError> {
    field
        .ok_or_else(|| AdapterError::ParseError("Failed to parse kline".to_string()))
        .and_then(|s| {
            s.parse::<T>()
                .map_err(|_| AdapterError::ParseError("Failed to parse kline".to_string()))
        })
}

pub(super) async fn fetch_ticker_metadata(
    hub: &mut HttpHub<BybitLimiter>,
    market_type: MarketKind,
) -> Result<TickerMetadataMap, AdapterError> {
    let exchange = exchange_from_market_type(market_type);

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let url = format!("{FETCH_DOMAIN}/v5/market/instruments-info?category={market}&limit=1000",);
    let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

    let exchange_info: Value =
        sonic_rs::from_str(&response_text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let result_list: &Vec<Value> = exchange_info["result"]["list"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_info_map = HashMap::new();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !exchange.is_symbol_supported(symbol, true) {
            continue;
        }

        if let Some(contract_type) = item["contractType"].as_str()
            && contract_type != "LinearPerpetual"
            && contract_type != "InversePerpetual"
        {
            continue;
        }

        if let Some(quote_asset) = item["quoteCoin"].as_str()
            && quote_asset != "USDT"
            && quote_asset != "USD"
        {
            continue;
        }

        let lot_size_filter = item["lotSizeFilter"]
            .as_object()
            .ok_or_else(|| AdapterError::ParseError("Lot size filter not found".to_string()))?;

        let min_qty = serde_util::value_as_f32(&lot_size_filter["minOrderQty"])
            .ok_or_else(|| AdapterError::ParseError("Min order qty not found".to_string()))?;

        let price_filter = item["priceFilter"]
            .as_object()
            .ok_or_else(|| AdapterError::ParseError("Price filter not found".to_string()))?;

        let min_ticksize = serde_util::value_as_f32(&price_filter["tickSize"])
            .ok_or_else(|| AdapterError::ParseError("Tick size not found".to_string()))?;

        let ticker = Ticker::new(symbol, exchange);
        let info = TickerInfo::new(ticker, min_ticksize, min_qty, None);

        ticker_info_map.insert(ticker, Some(info));
    }

    Ok(ticker_info_map)
}

pub(super) async fn fetch_ticker_stats(
    hub: &mut HttpHub<BybitLimiter>,
    market_type: MarketKind,
) -> Result<super::super::TickerStatsMap, AdapterError> {
    let exchange = exchange_from_market_type(market_type);

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let url = format!("{FETCH_DOMAIN}/v5/market/tickers?category={market}");
    let parsed_response: Value = hub.http_json_with_limiter(&url, 1, None, None).await?;

    let result_list: &Vec<Value> = parsed_response["result"]["list"]
        .as_array()
        .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

    let mut ticker_prices_map = HashMap::new();

    for item in result_list {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !exchange.is_symbol_supported(symbol, false) {
            continue;
        }

        let mark_price = serde_util::value_as_f32(&item["lastPrice"])
            .ok_or_else(|| AdapterError::ParseError("Mark price not found".to_string()))?;

        let daily_price_chg = serde_util::value_as_f32(&item["price24hPcnt"])
            .ok_or_else(|| AdapterError::ParseError("Daily price change not found".to_string()))?;

        let daily_volume = serde_util::value_as_f32(&item["volume24h"])
            .ok_or_else(|| AdapterError::ParseError("Daily volume not found".to_string()))?;

        let volume_in_usd = if market_type == MarketKind::InversePerps {
            daily_volume
        } else {
            daily_volume * mark_price
        };

        let ticker_stats = TickerStats {
            mark_price: Price::from_f32(mark_price),
            daily_price_chg: daily_price_chg * 100.0,
            daily_volume: Qty::from_f32(volume_in_usd),
        };

        ticker_prices_map.insert(Ticker::new(symbol, exchange), ticker_stats);
    }

    Ok(ticker_prices_map)
}

pub(super) async fn fetch_klines(
    hub: &mut HttpHub<BybitLimiter>,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(UnixMs, UnixMs)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = &ticker.to_full_symbol_and_type();
    let timeframe_str = {
        if Timeframe::D1 == timeframe {
            "D".to_string()
        } else {
            timeframe.to_minutes().to_string()
        }
    };

    let market = match market_type {
        MarketKind::Spot => "spot",
        MarketKind::LinearPerps => "linear",
        MarketKind::InversePerps => "inverse",
    };

    let mut url = format!(
        "{FETCH_DOMAIN}/v5/market/kline?category={}&symbol={}&interval={}",
        market,
        symbol_str.to_uppercase(),
        timeframe_str
    );

    if let Some((start, end)) = range {
        let start = start.as_u64();
        let end = end.as_u64();
        let interval_ms = timeframe.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(1000);

        url.push_str(&format!("&start={start}&end={end}&limit={num_intervals}"));
    }

    let response: ApiResponse = hub.http_json_with_limiter(&url, 1, None, None).await?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(*market_type),
    );

    let klines: Result<Vec<Kline>, AdapterError> = response
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
            let volume = qty_norm.normalize_qty(volume, close);

            let kline = Kline::new(
                time,
                open,
                high,
                low,
                close,
                crate::Volume::TotalOnly(volume),
                ticker_info.min_ticksize,
            );

            Ok(kline)
        })
        .collect();

    klines
}

pub(super) async fn fetch_historical_oi(
    hub: &mut HttpHub<BybitLimiter>,
    ticker_info: TickerInfo,
    range: Option<(UnixMs, UnixMs)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let ticker_str = ticker_info
        .ticker
        .to_full_symbol_and_type()
        .0
        .to_uppercase();
    let period_str = match period {
        Timeframe::M5 => "5min",
        Timeframe::M15 => "15min",
        Timeframe::M30 => "30min",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        Timeframe::D1 => "1d",
        _ => {
            return Err(AdapterError::InvalidRequest(format!(
                "Unsupported timeframe for open interest: {period}"
            )));
        }
    };

    let mut url = format!(
        "{FETCH_DOMAIN}/v5/market/open-interest?category=linear&symbol={ticker_str}&intervalTime={period_str}",
    );

    if let Some((start, end)) = range {
        let start = start.as_u64();
        let end = end.as_u64();
        let interval_ms = period.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(200);

        url.push_str(&format!(
            "&startTime={start}&endTime={end}&limit={num_intervals}"
        ));
    } else {
        url.push_str("&limit=200");
    }

    let response_text = hub.http_text_with_limiter(&url, 1, None, None).await?;

    let content: Value = sonic_rs::from_str(&response_text).map_err(|e| {
        log::error!(
            "Failed to parse JSON from {}: {}\nResponse: {}",
            url,
            e,
            response_text
        );
        AdapterError::ParseError(e.to_string())
    })?;

    let result_list = content["result"]["list"].as_array().ok_or_else(|| {
        log::error!("Result list is not an array in response: {}", response_text);
        AdapterError::ParseError("Result list is not an array".to_string())
    })?;

    let bybit_oi: Vec<DeOpenInterest> =
        serde_json::from_value(json!(result_list)).map_err(|e| {
            log::error!(
                "Failed to parse open interest array: {}\nResponse: {}",
                e,
                response_text
            );
            AdapterError::ParseError(format!("Failed to parse open interest: {e}"))
        })?;

    let open_interest: Vec<OpenInterest> = bybit_oi
        .into_iter()
        .map(|x| OpenInterest {
            time: x.timestamp.into(),
            value: x.value,
        })
        .collect();

    if open_interest.is_empty() {
        log::warn!(
            "No open interest data found for {}, from url: {}",
            ticker_str,
            url
        );
    }

    Ok(open_interest)
}
