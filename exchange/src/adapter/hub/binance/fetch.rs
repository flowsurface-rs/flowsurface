use crate::{
    Kline, OpenInterest, Price, Qty, Ticker, TickerInfo, TickerStats, Timeframe, Trade, UnixMs,
    Volume,
    depth::{DeOrder, DepthPayload},
    serde_util,
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    BinanceLimiter, HttpHub, INVERSE_PERP_DOMAIN, LINEAR_PERP_DOMAIN, MarketKind, SPOT_DOMAIN,
    THIRTY_DAYS_MS, exchange_from_market_type, raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use csv::ReaderBuilder;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, io::BufReader, path::PathBuf, time::UNIX_EPOCH};

#[derive(Deserialize, Debug, Clone)]
struct FetchedKline(
    u64,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    u64,
    String,
    u32,
    #[serde(deserialize_with = "de_string_to_number")] f32,
    String,
    String,
);

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeOpenInterest {
    #[serde(rename = "timestamp")]
    pub time: u64,
    #[serde(rename = "sumOpenInterest", deserialize_with = "de_string_to_number")]
    pub sum: f32,
}

#[derive(Deserialize, Debug)]
struct DeTrade {
    #[serde(rename = "T")]
    time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_number")]
    price: f32,
    #[serde(rename = "q", deserialize_with = "de_string_to_number")]
    qty: f32,
    #[serde(rename = "m")]
    is_sell: bool,
}

#[derive(Deserialize, Clone)]
struct FetchedPerpDepth {
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
struct FetchedSpotDepth {
    #[serde(rename = "lastUpdateId")]
    update_id: u64,
    #[serde(rename = "bids")]
    bids: Vec<DeOrder>,
    #[serde(rename = "asks")]
    asks: Vec<DeOrder>,
}

pub(super) async fn fetch_depth_snapshot(
    hub: &mut HttpHub<BinanceLimiter>,
    ticker: Ticker,
) -> Result<DepthPayload, AdapterError> {
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let base_url = match market_type {
        MarketKind::Spot => format!("{SPOT_DOMAIN}/api/v3/depth"),
        MarketKind::LinearPerps => format!("{LINEAR_PERP_DOMAIN}/fapi/v1/depth"),
        MarketKind::InversePerps => format!("{INVERSE_PERP_DOMAIN}/dapi/v1/depth"),
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
            _ => {
                return Err(AdapterError::InvalidRequest(format!(
                    "Unsupported depth limit for spot market: {depth_limit}"
                )));
            }
        },
        MarketKind::LinearPerps | MarketKind::InversePerps => match depth_limit {
            ..100 => 2,
            100 => 5,
            500 => 10,
            1000 => 20,
            _ => {
                return Err(AdapterError::InvalidRequest(format!(
                    "Unsupported depth limit for perps market: {depth_limit}"
                )));
            }
        },
    };

    let text = hub.http_text_with_limiter(&url, weight, None, None).await?;

    match market_type {
        MarketKind::Spot => {
            let fetched_depth: FetchedSpotDepth =
                serde_json::from_str(&text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

            Ok(DepthPayload {
                last_update_id: fetched_depth.update_id,
                time: (chrono::Utc::now().timestamp_millis() as u64).into(),
                bids: fetched_depth
                    .bids
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
                asks: fetched_depth
                    .asks
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
            })
        }
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            let fetched_depth: FetchedPerpDepth =
                serde_json::from_str(&text).map_err(|e| AdapterError::ParseError(e.to_string()))?;

            Ok(DepthPayload {
                last_update_id: fetched_depth.update_id,
                time: fetched_depth.time.into(),
                bids: fetched_depth
                    .bids
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
                asks: fetched_depth
                    .asks
                    .iter()
                    .map(|x| DeOrder {
                        price: x.price,
                        qty: x.qty,
                    })
                    .collect(),
            })
        }
    }
}

pub(super) async fn fetch_ticker_metadata(
    hub: &mut HttpHub<BinanceLimiter>,
    market: MarketKind,
) -> Result<super::super::TickerMetadataMap, AdapterError> {
    let (url, weight) = match market {
        MarketKind::Spot => (format!("{SPOT_DOMAIN}/api/v3/exchangeInfo"), 20),
        MarketKind::LinearPerps => (format!("{LINEAR_PERP_DOMAIN}/fapi/v1/exchangeInfo"), 1),
        MarketKind::InversePerps => (format!("{INVERSE_PERP_DOMAIN}/dapi/v1/exchangeInfo"), 1),
    };

    let response_text = hub.http_text_with_limiter(&url, weight, None, None).await?;

    let exchange_info: Value = serde_json::from_str(&response_text)
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

        if !exchange.is_symbol_supported(symbol_str, true) {
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
            .ok_or_else(|| {
                AdapterError::ParseError("Missing minQty in LOT_SIZE filter".to_string())
            })
            .and_then(|x| {
                serde_util::value_as_f32(&x["minQty"])
                    .ok_or_else(|| AdapterError::ParseError("Failed to parse minQty".to_string()))
            })?;

        let contract_size = serde_util::value_as_f32(&item["contractSize"]);

        let ticker = Ticker::new(symbol_str, exchange);

        if let Some(price_filter) = price_filter {
            let min_ticksize = serde_util::value_as_f32(&price_filter["tickSize"])
                .ok_or_else(|| AdapterError::ParseError("tickSize not found".to_string()))?;

            let info = TickerInfo::new(ticker, min_ticksize, min_qty, contract_size);
            ticker_info_map.insert(ticker, Some(info));
        } else {
            ticker_info_map.insert(ticker, None);
        }
    }

    Ok(ticker_info_map)
}

pub(super) async fn fetch_ticker_stats(
    hub: &mut HttpHub<BinanceLimiter>,
    market: MarketKind,
    contract_sizes: Option<&HashMap<Ticker, f32>>,
) -> Result<super::super::TickerStatsMap, AdapterError> {
    let (url, weight) = match market {
        MarketKind::Spot => (format!("{SPOT_DOMAIN}/api/v3/ticker/24hr"), 80),
        MarketKind::LinearPerps => (format!("{LINEAR_PERP_DOMAIN}/fapi/v1/ticker/24hr"), 40),
        MarketKind::InversePerps => (format!("{INVERSE_PERP_DOMAIN}/dapi/v1/ticker/24hr"), 40),
    };

    let parsed_response: Vec<Value> = hub.http_json_with_limiter(&url, weight, None, None).await?;

    let exchange = exchange_from_market_type(market);
    let mut ticker_price_map = HashMap::new();

    for item in parsed_response {
        let symbol = item["symbol"]
            .as_str()
            .ok_or_else(|| AdapterError::ParseError("Symbol not found".to_string()))?;

        if !exchange.is_symbol_supported(symbol, false) {
            continue;
        }

        let ticker = Ticker::new(symbol, exchange);

        let last_price = serde_util::value_as_f32(&item["lastPrice"])
            .ok_or_else(|| AdapterError::ParseError("Last price not found".to_string()))?;

        let price_change_pt =
            serde_util::value_as_f32(&item["priceChangePercent"]).ok_or_else(|| {
                AdapterError::ParseError("Price change percent not found".to_string())
            })?;

        let volume = match market {
            MarketKind::Spot | MarketKind::LinearPerps => {
                serde_util::value_as_f32(&item["quoteVolume"])
                    .ok_or_else(|| AdapterError::ParseError("Quote volume not found".to_string()))?
            }
            MarketKind::InversePerps => serde_util::value_as_f32(&item["volume"])
                .ok_or_else(|| AdapterError::ParseError("Volume not found".to_string()))?,
        };

        let daily_volume = match market {
            MarketKind::Spot | MarketKind::LinearPerps => Qty::from_f32(volume),
            MarketKind::InversePerps => {
                let contract_size = match contract_sizes
                    .and_then(|sizes| sizes.get(&ticker))
                    .copied()
                {
                    Some(size) => size,
                    None => {
                        log::debug!("Missing contract size for {ticker}, skipping ticker in stats");
                        continue;
                    }
                };

                Qty::from_f32(volume * contract_size)
            }
        };

        let ticker_stats = TickerStats {
            mark_price: Price::from_f32(last_price),
            daily_price_chg: price_change_pt,
            daily_volume,
        };

        ticker_price_map.insert(ticker, ticker_stats);
    }

    Ok(ticker_price_map)
}

pub(super) async fn fetch_klines(
    hub: &mut HttpHub<BinanceLimiter>,
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Option<(UnixMs, UnixMs)>,
) -> Result<Vec<Kline>, AdapterError> {
    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
    let timeframe_str = timeframe.to_string();

    let base_url = match market_type {
        MarketKind::Spot => format!("{SPOT_DOMAIN}/api/v3/klines"),
        MarketKind::LinearPerps => format!("{LINEAR_PERP_DOMAIN}/fapi/v1/klines"),
        MarketKind::InversePerps => format!("{INVERSE_PERP_DOMAIN}/dapi/v1/klines"),
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&interval={timeframe_str}");

    let limit_param = if let Some((start, end)) = range {
        let start = start.as_u64();
        let end = end.as_u64();
        let interval_ms = timeframe.to_milliseconds();
        let num_intervals = ((end - start) / interval_ms).min(1000);

        if num_intervals < 3 {
            let new_start = start - (interval_ms * 5);
            let new_end = end + (interval_ms * 5);
            let num_intervals = ((new_end - new_start) / interval_ms).min(1000);

            url.push_str(&format!(
                "&startTime={new_start}&endTime={new_end}&limit={num_intervals}"
            ));
            num_intervals
        } else {
            url.push_str(&format!(
                "&startTime={start}&endTime={end}&limit={num_intervals}"
            ));
            num_intervals
        }
    } else {
        let num_intervals = 400;
        url.push_str(&format!("&limit={num_intervals}"));
        num_intervals
    };

    let weight = match market_type {
        MarketKind::Spot => 2,
        MarketKind::LinearPerps | MarketKind::InversePerps => match limit_param {
            1..=100 => 1,
            101..=500 => 2,
            501..=1000 => 5,
            1001..=1500 => 10,
            _ => {
                return Err(AdapterError::InvalidRequest(format!(
                    "Unsupported kline limit parameter for perps market: {limit_param}"
                )));
            }
        },
    };

    let fetched_klines: Vec<FetchedKline> =
        hub.http_json_with_limiter(&url, weight, None, None).await?;

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );
    let min_ticksize = ticker_info.min_ticksize;

    let klines: Vec<_> = fetched_klines
        .into_iter()
        .map(|k| {
            let FetchedKline(
                time,
                open,
                high,
                low,
                close,
                volume,
                _close_time,
                _quote_asset_volume,
                _number_of_trades,
                taker_buy_base_asset_volume,
                _taker_buy_quote_asset_volume,
                _ignore,
            ) = k;

            let buy_volume = taker_buy_base_asset_volume;
            let sell_volume = volume - buy_volume;

            let buy_volume = qty_norm.normalize_qty(buy_volume, close);
            let sell_volume = qty_norm.normalize_qty(sell_volume, close);

            Kline::new(
                time,
                open,
                high,
                low,
                close,
                Volume::BuySell(buy_volume, sell_volume),
                min_ticksize,
            )
        })
        .collect();

    Ok(klines)
}

pub(super) async fn fetch_historical_oi(
    hub: &mut HttpHub<BinanceLimiter>,
    ticker_info: TickerInfo,
    range: Option<(UnixMs, UnixMs)>,
    period: Timeframe,
) -> Result<Vec<OpenInterest>, AdapterError> {
    let (ticker_str, market) = ticker_info.ticker.to_full_symbol_and_type();
    let period_str = period.to_string();

    let (base_url, pair_str, weight) = match market {
        MarketKind::LinearPerps => (
            format!("{LINEAR_PERP_DOMAIN}/futures/data/openInterestHist"),
            format!("?symbol={ticker_str}"),
            12,
        ),
        MarketKind::InversePerps => (
            format!("{INVERSE_PERP_DOMAIN}/futures/data/openInterestHist"),
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

    let mut url = format!("{base_url}{pair_str}&period={period_str}");

    if let Some((start, end)) = range {
        let start = start.as_u64();
        let end = end.as_u64();
        let thirty_days_ago = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
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

    let binance_oi: Vec<DeOpenInterest> =
        hub.http_json_with_limiter(&url, weight, None, None).await?;

    let contract_size = ticker_info.contract_size;
    let open_interest = binance_oi
        .iter()
        .map(|x| OpenInterest {
            time: x.time.into(),
            value: contract_size.map_or(x.sum, |size| x.sum * size.as_f32()),
        })
        .collect::<Vec<OpenInterest>>();

    Ok(open_interest)
}

async fn fetch_intraday_trades(
    hub: &mut HttpHub<BinanceLimiter>,
    ticker_info: TickerInfo,
    from: UnixMs,
) -> Result<Vec<Trade>, AdapterError> {
    let ticker = ticker_info.ticker;
    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let (base_url, weight) = match market_type {
        MarketKind::Spot => (format!("{SPOT_DOMAIN}/api/v3/aggTrades"), 4),
        MarketKind::LinearPerps => (format!("{LINEAR_PERP_DOMAIN}/fapi/v1/aggTrades"), 20),
        MarketKind::InversePerps => (format!("{INVERSE_PERP_DOMAIN}/dapi/v1/aggTrades"), 20),
    };

    let mut url = format!("{base_url}?symbol={symbol_str}&limit=1000");
    url.push_str(&format!("&startTime={}", from.as_u64()));

    let de_trades: Vec<DeTrade> = hub.http_json_with_limiter(&url, weight, None, None).await?;

    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let trades = de_trades
        .into_iter()
        .map(|de_trade| Trade {
            time: de_trade.time.into(),
            is_sell: de_trade.is_sell,
            price: Price::from_f32(de_trade.price).round_to_min_tick(ticker_info.min_ticksize),
            qty: qty_norm.normalize_qty(de_trade.qty, de_trade.price),
        })
        .collect();

    Ok(trades)
}

async fn get_hist_trades_with_client(
    client: &reqwest::Client,
    ticker_info: TickerInfo,
    date: chrono::NaiveDate,
    base_path: PathBuf,
) -> Result<Vec<Trade>, AdapterError> {
    let ticker = ticker_info.ticker;
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
        .map_err(|e| AdapterError::ParseError(format!("Failed to create directories: {e}")))?;

    let zip_path = format!("{market_subpath}/{zip_file_name}");
    let base_zip_path = base_path.join(&zip_file_name);

    if std::fs::metadata(&base_zip_path).is_ok() {
        log::info!("Using cached {}", zip_path);
    } else {
        let url = format!("https://data.binance.vision/{zip_path}");

        log::info!("Downloading from {}", url);

        let resp = client.get(&url).send().await.map_err(AdapterError::from)?;

        if !resp.status().is_success() {
            return Err(AdapterError::InvalidRequest(format!(
                "Failed to fetch from {}: {}",
                url,
                resp.status()
            )));
        }

        let body = resp.bytes().await.map_err(AdapterError::from)?;

        std::fs::write(&base_zip_path, &body).map_err(|e| {
            AdapterError::ParseError(format!("Failed to write zip file: {e}, {base_zip_path:?}"))
        })?;
    }

    let file = std::fs::File::open(&base_zip_path)
        .map_err(|e| AdapterError::ParseError(format!("Failed to open compressed file: {e}")))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AdapterError::ParseError(format!("Failed to unzip file: {e}")))?;

    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

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
                let price_f32 = record[1].parse::<f32>().ok()?;

                let price = Price::from_f32(price_f32).round_to_min_tick(ticker_info.min_ticksize);
                let qty = qty_norm.normalize_qty(record[2].parse::<f32>().ok()?, price_f32);

                Some(Trade {
                    time: time.into(),
                    is_sell,
                    price,
                    qty,
                })
            })
        }));
    }

    Ok(trades)
}

pub(super) async fn fetch_trades(
    hub: &mut HttpHub<BinanceLimiter>,
    ticker_info: TickerInfo,
    from_time: UnixMs,
    data_path: Option<PathBuf>,
) -> Result<Vec<Trade>, AdapterError> {
    let Some(data_path) = data_path else {
        return Err(AdapterError::InvalidRequest(
            "Binance trades fetch requires data_path".to_string(),
        ));
    };

    let today_midnight = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight should always be valid")
        .and_utc();

    if from_time.as_u64() as i64 >= today_midnight.timestamp_millis() {
        return fetch_intraday_trades(hub, ticker_info, from_time).await;
    }

    let from_date = chrono::DateTime::from_timestamp_millis(from_time.as_u64() as i64)
        .ok_or_else(|| AdapterError::ParseError("Invalid timestamp".into()))?
        .date_naive();

    let client = hub.client().clone();

    match get_hist_trades_with_client(&client, ticker_info, from_date, data_path).await {
        Ok(mut trades) => {
            if let Some(latest_trade) = trades.last().copied() {
                match fetch_intraday_trades(hub, ticker_info, latest_trade.time).await {
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
        Err(e) => {
            log::warn!(
                "Historical trades fetch failed: {}, falling back to intraday fetch",
                e
            );
            fetch_intraday_trades(hub, ticker_info, from_time).await
        }
    }
}
