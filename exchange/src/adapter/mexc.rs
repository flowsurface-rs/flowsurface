use crate::{
    Volume,
    adapter::{TRADE_BUCKET_INTERVAL, flush_trade_buffers},
    unit::qty::{QtyNormalization, RawQtyUnit},
};

use super::{
    super::{
        Exchange, Kline, MarketKind, Price, PushFrequency, Qty, SizeUnit, StreamKind, Ticker,
        TickerInfo, TickerStats, Timeframe, Trade,
        connect::{State, channel, connect_ws},
        depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
        limiter::{self},
        serde_util,
        serde_util::de_string_to_number,
        unit::qty::volume_size_unit,
    },
    AdapterError, Event, StreamTicksize,
};

use fastwebsockets::{FragmentCollector, Frame, OpCode};
use futures::{SinkExt, Stream};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};

use std::{collections::HashMap, time::Duration};

const FETCH_DOMAIN: &str = "https://api.mexc.com/api";

// const MEXC_SPOT_WS_DOMAIN: &str = "wbs-api.mexc.com";
// const MEXC_SPOT_WS_PATH: &str = "/ws";
const MEXC_FUTURES_WS_DOMAIN: &str = "contract.mexc.com";
const MEXC_FUTURES_WS_PATH: &str = "/edge";

const PING_INTERVAL: u64 = 15;

/// REST API response for depth snapshot
#[derive(Deserialize)]
struct DepthSnapshotResponse {
    #[serde(rename = "data")]
    data: DepthData,
}

#[derive(Deserialize)]
struct DepthData {
    #[serde(rename = "asks")]
    asks: Vec<FuturesDepthItem>,
    #[serde(rename = "bids")]
    bids: Vec<FuturesDepthItem>,
    #[serde(rename = "version")]
    version: u64,
    #[serde(rename = "timestamp")]
    timestamp: u64,
}

// MEXC Futures new format structures
#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "p")]
    pub price: f32,
    #[serde(rename = "v")]
    pub qty: f32,
    #[serde(rename = "T")]
    pub direction: u8, // 1 = buy, 2 = sell
    #[serde(rename = "t")]
    pub time: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct FuturesDepthItem {
    #[serde()]
    pub price: f32,
    #[serde()]
    pub qty: f32,
    #[serde()]
    pub order_count: f32,
}

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "asks")]
    pub asks: Vec<FuturesDepthItem>,
    #[serde(rename = "bids")]
    pub bids: Vec<FuturesDepthItem>,
    #[serde(rename = "version")]
    pub version: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SonicKline {
    #[serde(rename = "t")]
    pub time: u64,
    #[serde(rename = "o")]
    pub open: f32,
    #[serde(rename = "h")]
    pub high: f32,
    #[serde(rename = "l")]
    pub low: f32,
    #[serde(rename = "c")]
    pub close: f32,
    #[serde(rename = "q")]
    pub quote_volume: f32,
    #[serde(rename = "a")]
    pub amount: f32,
    #[serde(rename = "interval")]
    pub interval: String,
    #[serde(rename = "symbol")]
    pub symbol: String,
}

#[allow(dead_code)]
enum StreamData {
    Trade(Ticker, Vec<SonicTrade>, u64),
    Depth(SonicDepth, u64),
    Kline(Ticker, Vec<SonicKline>),
    Pong(u64),
    Subscription(String),
}

fn exchange_from_market_type(market: MarketKind) -> Exchange {
    match market {
        MarketKind::Spot => Exchange::MexcSpot,
        MarketKind::LinearPerps => Exchange::MexcLinear,
        MarketKind::InversePerps => Exchange::MexcInverse,
    }
}

fn raw_qty_unit_from_market_type(market: MarketKind) -> RawQtyUnit {
    match market {
        MarketKind::Spot => RawQtyUnit::Base,
        MarketKind::LinearPerps | MarketKind::InversePerps => RawQtyUnit::Contracts,
    }
}

fn contract_size_for_market(
    ticker_info: TickerInfo,
    market: MarketKind,
    context: &str,
) -> Result<f32, AdapterError> {
    match market {
        MarketKind::Spot => Ok(1.0),
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            ticker_info.contract_size.map(f32::from).ok_or_else(|| {
                AdapterError::ParseError(format!(
                    "Missing contract size for {} in {context}",
                    ticker_info.ticker
                ))
            })
        }
    }
}

fn mexc_perps_market_from_symbol(
    symbol: &str,
    contract_sizes: Option<&HashMap<Ticker, f32>>,
) -> Option<MarketKind> {
    if symbol.ends_with("USDT") {
        return Some(MarketKind::LinearPerps);
    }
    if symbol.ends_with("USD") {
        return Some(MarketKind::InversePerps);
    }

    let contract_sizes = contract_sizes?;

    let has_linear = contract_sizes.contains_key(&Ticker::new(symbol, Exchange::MexcLinear));
    let has_inverse = contract_sizes.contains_key(&Ticker::new(symbol, Exchange::MexcInverse));

    match (has_linear, has_inverse) {
        (true, false) => Some(MarketKind::LinearPerps),
        (false, true) => Some(MarketKind::InversePerps),
        _ => None,
    }
}

pub async fn fetch_ticker_metadata(
    markets: &[MarketKind],
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
    let mut ticker_info_map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let url = format!("{FETCH_DOMAIN}/v3/exchangeInfo");

        let response_text = limiter::http_request(&url, None, None).await?;
        let exchange_info: Value = sonic_rs::from_str(&response_text).map_err(|e| {
            AdapterError::ParseError(format!("Failed to parse MEXC exchange info: {e}"))
        })?;

        let symbols = exchange_info["symbols"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Missing symbols array".to_string()))?;

        let exchange = exchange_from_market_type(MarketKind::Spot);

        for item in symbols {
            // status: 1 - online, 2 - Pause, 3 - offline
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

        let response_text = limiter::http_request(&url, None, None).await?;
        let exchange_info: Value = sonic_rs::from_str(&response_text).map_err(|e| {
            AdapterError::ParseError(format!("Failed to parse MEXC exchange info: {e}"))
        })?;

        let result_list: &Vec<Value> = exchange_info["data"]
            .as_array()
            .ok_or_else(|| AdapterError::ParseError("Result list is not an array".to_string()))?;

        for item in result_list {
            // Status: 0 enabled, 1 delivery, 2 delivered, 3 offline, 4 paused
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

pub async fn fetch_ticker_stats(
    markets: &[MarketKind],
    contract_sizes: Option<&HashMap<Ticker, f32>>,
) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
    let mut ticker_prices_map = HashMap::new();

    let include_spot = markets.contains(&MarketKind::Spot);
    let include_perps = markets
        .iter()
        .any(|m| matches!(m, MarketKind::LinearPerps | MarketKind::InversePerps));

    if include_spot {
        let exchange = exchange_from_market_type(MarketKind::Spot);
        let url = format!("{FETCH_DOMAIN}/v3/ticker/24hr");
        let response_text = limiter::http_request(&url, None, None).await?;

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
        let response_text = limiter::http_request(&url, None, None).await?;

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
    #[serde(deserialize_with = "de_string_to_number ")]
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

fn convert_to_mexc_timeframe(timeframe: Timeframe, market: MarketKind) -> Option<&'static str> {
    if market == MarketKind::Spot {
        match timeframe {
            Timeframe::M1 => Some("1m"),
            Timeframe::M5 => Some("5m"),
            Timeframe::M15 => Some("15m"),
            Timeframe::M30 => Some("30m"),
            Timeframe::H1 => Some("60m"),
            Timeframe::H4 => Some("4h"),
            Timeframe::D1 => Some("1d"),
            _ => None,
        }
    } else {
        match timeframe {
            Timeframe::M1 => Some("Min1"),
            Timeframe::M5 => Some("Min5"),
            Timeframe::M15 => Some("Min15"),
            Timeframe::M30 => Some("Min30"),
            Timeframe::H1 => Some("Min60"),
            Timeframe::H4 => Some("Hour4"),
            Timeframe::D1 => Some("Day1"),
            _ => None,
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
        match market_type {
            MarketKind::Spot => {
                // Spot uses startTime and endTime in milliseconds
                url.push_str(&format!("&startTime={}&endTime={}", start_ms, end_ms));
            }
            MarketKind::LinearPerps | MarketKind::InversePerps => {
                // Futures uses start and end in seconds
                let start_sec = start_ms / 1000;
                let end_sec = end_ms / 1000;
                url.push_str(&format!("&start={}&end={}", start_sec, end_sec));
            }
        }
    }
    let response_text = limiter::http_request(&url, None, None).await?;

    // Parse the klines based on market type
    let klines_result: Result<Vec<Kline>, AdapterError> = match market_type {
        MarketKind::Spot => {
            let parsed_response: Vec<KlineSpot> = sonic_rs::from_str(&response_text)
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            let klines: Result<Vec<Kline>, AdapterError> = parsed_response
                .iter()
                .map(|kline| {
                    let volume = qty_norm.normalize_qty(kline.vol, kline.close);
                    let res = Kline::new(
                        kline.close_ts,
                        kline.open,
                        kline.high,
                        kline.low,
                        kline.close,
                        Volume::TotalOnly(volume),
                        ticker_info.min_ticksize,
                    );
                    Ok(res)
                })
                .collect();
            klines
        }
        MarketKind::LinearPerps | MarketKind::InversePerps => {
            let parsed_response: FuturesApiResponse = sonic_rs::from_str(&response_text)
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            // Futures returns { "success": true, "code": 0, "data": { "time": [...], "open": [...], ... } }
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
                    })? * 1000; // Convert seconds to ms

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

/// Fetch depth snapshot from REST API for initial orderbook state
pub async fn fetch_depth_snapshot(symbol: &str) -> Result<DepthPayload, AdapterError> {
    let url = format!("{FETCH_DOMAIN}/v1/contract/depth/{symbol}");
    let response_text = limiter::http_request(&url, None, None).await?;
    let snapshot: DepthSnapshotResponse = sonic_rs::from_str(&response_text).map_err(|e| {
        log::error!("Failed to parse MEXC depth snapshot: {}", e);
        AdapterError::ParseError(e.to_string())
    })?;

    let parse_orders = |arr: &Vec<FuturesDepthItem>| -> Vec<DeOrder> {
        arr.iter()
            .map(|x| DeOrder {
                price: x.price,
                qty: x.qty,
            })
            .collect()
    };

    let bids = parse_orders(&snapshot.data.bids);
    let asks = parse_orders(&snapshot.data.asks);

    let time = snapshot.data.timestamp;

    Ok(DepthPayload {
        last_update_id: snapshot.data.version,
        time,
        bids,
        asks,
    })
}

async fn send_ping(
    websocket: &mut FragmentCollector<TokioIo<Upgraded>>,
) -> Result<(), &'static str> {
    let ping_msg = json!({"method": "ping"});
    if websocket
        .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
            ping_msg.to_string().as_bytes(),
        )))
        .await
        .is_err()
    {
        log::error!("Failed to send ping");
        return Err("Failed to send ping");
    }
    Ok(())
}

pub fn connect_depth_stream(
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let ticker = ticker_info.ticker;

        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();
        let exchange = exchange_from_market_type(market_type);

        let mut orderbook = LocalDepthCache::default();

        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(market_type),
        );

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));
        let mut snapshot_time: u64 = 0;

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH).await {
                        Ok(mut websocket) => {
                            let depth_subscription = json!({
                                "method": "sub.depth",
                                "param": {
                                    "symbol": symbol_str,
                                }
                            });

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

                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    "Failed to connect to websocket".to_string(),
                                ))
                                .await;
                            continue;
                        }
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }

                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
                                    OpCode::Text => {
                                        match feed_de(&msg.payload[..], Some(ticker), market_type) {
                                            Ok(data) => {
                                                match data {
                                                    StreamData::Pong(_) => {}
                                                    StreamData::Subscription(stream_name) => {
                                                        if stream_name == "depth" {
                                                            match fetch_depth_snapshot(&symbol_str).await {
                                                                Ok(snapshot) => {
                                                                    snapshot_time = snapshot.time;
                                                                    orderbook.update_with_qty_norm(
                                                                        DepthUpdate::Snapshot(snapshot),
                                                                        ticker_info.min_ticksize,
                                                                        Some(qty_norm),
                                                                    );
                                                                }
                                                                Err(e) => {
                                                                    log::error!("Failed to fetch depth snapshot for {symbol_str}: {}", e);
                                                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                                                    continue;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    StreamData::Depth(de_depth, time) => {
                                                        if time < snapshot_time {
                                                            continue;
                                                        }
                                                        let depth = DepthPayload {
                                                            last_update_id: de_depth.version,
                                                            time,
                                                            bids: de_depth
                                                                .bids
                                                                .iter()
                                                                .map(|x| DeOrder {
                                                                    price: x.price,
                                                                    qty: x.qty,
                                                                })
                                                                .collect(),
                                                            asks: de_depth
                                                                .asks
                                                                .iter()
                                                                .map(|x| DeOrder {
                                                                    price: x.price,
                                                                    qty: x.qty,
                                                                })
                                                                .collect(),
                                                        };

                                                        orderbook.update_with_qty_norm(
                                                            DepthUpdate::Diff(depth),
                                                            ticker_info.min_ticksize,
                                                            Some(qty_norm),
                                                        );

                                                        let _ = output
                                                            .send(Event::DepthReceived(
                                                                StreamKind::Depth {
                                                                    ticker_info,
                                                                    depth_aggr: StreamTicksize::Client,
                                                                    push_freq,
                                                                },
                                                                time,
                                                                orderbook.depth.clone(),
                                                            ))
                                                            .await;
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("Failed to parse MEXC depth message: {}", e);
                                            }
                                        }
                                    }
                                    OpCode::Close => {
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                    state = State::Disconnected;
                                    }
                                    _ => {}
                                },
                                Err(_) => {
                                    let _ = output
                                        .send(Event::Disconnected(
                                            exchange,
                                            "Error reading frame".to_string(),
                                        ))
                                        .await;
                                    state = State::Disconnected;
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn connect_trade_stream(
    tickers: Vec<TickerInfo>,
    market_type: MarketKind,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let mut state: State = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);
        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        let ticker_info_map = tickers
            .iter()
            .map(|ticker_info| {
                (
                    ticker_info.ticker,
                    (
                        *ticker_info,
                        QtyNormalization::with_raw_qty_unit(
                            size_in_quote_ccy,
                            *ticker_info,
                            raw_qty_unit_from_market_type(market_type),
                        ),
                    ),
                )
            })
            .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let mut trades_buffer_map: FxHashMap<Ticker, Vec<Trade>> = FxHashMap::default();
        let mut last_flush = tokio::time::Instant::now();

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH).await {
                        Ok(mut websocket) => {
                            for ticker_info in &tickers {
                                let symbol = ticker_info.ticker.to_full_symbol_and_type().0;
                                let deal_subscription = json!({
                                    "method": "sub.deal",
                                    "param": {
                                        "symbol": symbol,
                                    }
                                });

                                if websocket
                                    .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                        deal_subscription.to_string().as_bytes(),
                                    )))
                                    .await
                                    .is_err()
                                {
                                    log::error!("Failed to subscribe to trade stream");
                                    continue;
                                }
                            }

                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
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
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }

                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
                                    OpCode::Text => {
                                        match feed_de(&msg.payload[..], None, market_type) {
                                            Ok(data) => {
                                                match data {
                                                    StreamData::Pong(_) => {}
                                                    StreamData::Subscription(_) => {}
                                                    StreamData::Trade(ticker, mut de_trades, _) => {
                                                        if let Some((ticker_info, qty_norm)) = ticker_info_map.get(&ticker) {
                                                            let ticker_info = *ticker_info;

                                                            de_trades.sort_unstable_by_key(|t| t.time);
                                                            for trade in &de_trades {
                                                                let price = Price::from_f32(trade.price)
                                                                    .round_to_min_tick(ticker_info.min_ticksize);

                                                                let trade_entity = Trade {
                                                                    time: trade.time,
                                                                    is_sell: trade.direction == 2,
                                                                    price,
                                                                    qty: qty_norm
                                                                        .normalize_qty(trade.qty, trade.price),
                                                                };

                                                                let trades_buffer = trades_buffer_map.entry(ticker).or_default();
                                                                trades_buffer.push(trade_entity);
                                                            }
                                                        } else {
                                                            log::error!("Ticker info not found for ticker: {}", ticker);
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("Failed to parse MEXC trade message: {}", e);
                                            }
                                        }

                                        if last_flush.elapsed() >= TRADE_BUCKET_INTERVAL {
                                            flush_trade_buffers(
                                                &mut output,
                                                &ticker_info_map,
                                                &mut trades_buffer_map,
                                            )
                                            .await;
                                            last_flush = tokio::time::Instant::now();
                                        }
                                    }
                                    OpCode::Close => {
                                        flush_trade_buffers(
                                            &mut output,
                                            &ticker_info_map,
                                            &mut trades_buffer_map,
                                        )
                                        .await;
                                        let _ = output
                                            .send(Event::Disconnected(
                                                exchange,
                                                "Connection closed".to_string(),
                                            ))
                                            .await;
                                        state = State::Disconnected;
                                    }
                                    _ => {}
                                },
                                Err(_) => {
                                    flush_trade_buffers(
                                        &mut output,
                                        &ticker_info_map,
                                        &mut trades_buffer_map,
                                    )
                                    .await;
                                    let _ = output
                                        .send(Event::Disconnected(
                                            exchange,
                                            "Error reading frame".to_string(),
                                        ))
                                        .await;
                                    state = State::Disconnected;
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

#[derive(Debug)]
enum StreamName {
    Depth,
    Trade,
    Kline,
    Subscription(String),
    Error,
    Pong,
    Unknown,
}

impl StreamName {
    fn from_topic(topic: &str) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if parts.first() == Some(&"pong") {
            return StreamName::Pong;
        }

        match parts.get(1) {
            Some(&"sub") => {
                StreamName::Subscription(parts.get(2).map(|s| s.to_string()).unwrap_or_default())
            }
            Some(&"deal") => StreamName::Trade,
            Some(&"depth") => StreamName::Depth,
            Some(&"kline") => StreamName::Kline,
            Some(&"error") => StreamName::Error,
            _ => StreamName::Unknown,
        }
    }
}

fn feed_de(
    slice: &[u8],
    ticker: Option<Ticker>,
    market_type: MarketKind,
) -> Result<StreamData, AdapterError> {
    let mut stream_type: Option<StreamName> = None;
    let mut ts: Option<u64> = None;
    let mut data_faststr: Option<sonic_rs::FastStr> = None;

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    let mut topic_ticker: Option<Ticker> = ticker;

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "channel" {
            if let Some(val) = v.as_str() {
                stream_type = Some(StreamName::from_topic(val));
            }
        } else if k == "data" {
            data_faststr = Some(v.as_raw_faststr().clone());
        } else if k == "ts" {
            ts = Some(
                v.as_u64()
                    .ok_or_else(|| AdapterError::ParseError("ts not found".to_string()))?,
            );
        } else if k == "symbol" {
            let ticker_str = v
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("symbol does not exist".to_string()))?;
            if topic_ticker.is_none() {
                topic_ticker = Some(Ticker::new(
                    ticker_str,
                    exchange_from_market_type(market_type),
                ));
            }
        }
    }

    if let Some(data) = data_faststr {
        match stream_type {
            Some(StreamName::Kline) => {
                let mut kline_data: SonicKline = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                kline_data.time *= 1000;

                let ticker =
                    Ticker::new(&kline_data.symbol, exchange_from_market_type(market_type));
                return Ok(StreamData::Kline(ticker, vec![kline_data]));
            }
            Some(StreamName::Trade) => {
                let deals_data: Vec<SonicTrade> = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                let trade_ticker = topic_ticker.ok_or_else(|| {
                    AdapterError::ParseError("Missing ticker for trade data".to_string())
                })?;
                return Ok(StreamData::Trade(
                    trade_ticker,
                    deals_data,
                    ts.unwrap_or_default(),
                ));
            }
            Some(StreamName::Depth) => {
                let depth = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                return Ok(StreamData::Depth(depth, ts.unwrap_or_default()));
            }
            Some(StreamName::Pong) => {
                return Ok(StreamData::Pong(ts.unwrap_or_default()));
            }
            Some(StreamName::Subscription(name)) => {
                return Ok(StreamData::Subscription(name));
            }
            Some(StreamName::Error) => {
                log::error!("Error: {data}");
            }
            _ => {
                log::error!("Unknown stream type");
            }
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    match interval {
        "Min1" => Some(Timeframe::M1),
        "Min5" => Some(Timeframe::M5),
        "Min15" => Some(Timeframe::M15),
        "Min30" => Some(Timeframe::M30),
        "Min60" => Some(Timeframe::H1),
        "Hour4" => Some(Timeframe::H4),
        "Day1" => Some(Timeframe::D1),
        _ => None,
    }
}

async fn connect_websocket(
    domain: &str,
    path: &str,
) -> Result<FragmentCollector<TokioIo<Upgraded>>, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    connect_ws(domain, &url).await
}

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
) -> impl Stream<Item = Event> {
    channel(100, async move |mut output| {
        let mut state = State::Disconnected;

        let exchange = exchange_from_market_type(market_type);
        let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

        if market_type == MarketKind::Spot {
            todo!();
        }

        let ticker_info_map = streams
            .iter()
            .map(|(ticker_info, _)| {
                contract_size_for_market(*ticker_info, market_type, "connect_kline_stream").map(
                    |_| {
                        (
                            ticker_info.ticker,
                            (
                                *ticker_info,
                                QtyNormalization::with_raw_qty_unit(
                                    size_in_quote_ccy,
                                    *ticker_info,
                                    raw_qty_unit_from_market_type(market_type),
                                ),
                            ),
                        )
                    },
                )
            })
            .collect::<Result<HashMap<Ticker, (TickerInfo, QtyNormalization)>, AdapterError>>();

        let ticker_info_map = match ticker_info_map {
            Ok(ticker_info_map) => ticker_info_map,
            Err(err) => {
                let _ = output
                    .send(Event::Disconnected(exchange, err.to_string()))
                    .await;
                return;
            }
        };

        let mut ping_interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL));

        loop {
            match &mut state {
                State::Disconnected => {
                    match connect_websocket(MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH).await {
                        Ok(mut websocket) => {
                            let mut subscribed_any = false;

                            for (ticker_info, timeframe) in &streams {
                                let ticker = ticker_info.ticker;
                                let symbol = ticker.to_full_symbol_and_type().0;
                                let Some(interval) =
                                    convert_to_mexc_timeframe(*timeframe, market_type)
                                else {
                                    log::error!(
                                        "Unsupported MEXC kline timeframe requested: {} ({})",
                                        timeframe,
                                        ticker
                                    );
                                    continue;
                                };
                                let subscribe_msg = serde_json::json!({
                                    "method": "sub.kline",
                                    "param": {
                                        "symbol": symbol.to_uppercase(),
                                        "interval": interval,
                                    },
                                    "gzip": false,
                                });

                                if websocket
                                    .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                                        subscribe_msg.to_string().as_bytes(),
                                    )))
                                    .await
                                    .is_err()
                                {
                                    continue;
                                }

                                subscribed_any = true;
                            }

                            if !subscribed_any {
                                let _ = output
                                    .send(Event::Disconnected(
                                        exchange,
                                        "No supported MEXC kline timeframes requested".to_string(),
                                    ))
                                    .await;
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                continue;
                            }

                            let _ = output.send(Event::Connected(exchange)).await;
                            state = State::Connected(websocket);
                        }
                        Err(err) => {
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                            let _ = output
                                .send(Event::Disconnected(
                                    exchange,
                                    format!("Failed to connect: {err}"),
                                ))
                                .await;
                        }
                    }
                }
                State::Connected(websocket) => {
                    tokio::select! {
                        _ = ping_interval.tick() => {
                            if send_ping(websocket).await.is_err() {
                                state = State::Disconnected;
                            }
                        }
                        msg = websocket.read_frame() => {
                            match msg {
                                Ok(msg) => match msg.opcode {
                                    OpCode::Text => {
                                        if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
                                            feed_de(&msg.payload[..], None, market_type)
                                        {
                                            for de_kline in &de_kline_vec {
                                                if let Some(timeframe) = string_to_timeframe(&de_kline.interval)
                                                {
                                                    if let Some((ticker_info, qty_norm)) =
                                                        ticker_info_map.get(&ticker)
                                                    {
                                                        let ticker_info = *ticker_info;

                                                        let volume = qty_norm.normalize_qty(
                                                            de_kline.quote_volume,
                                                            de_kline.close,
                                                        );

                                                        let kline = Kline::new(
                                                            de_kline.time,
                                                            de_kline.open,
                                                            de_kline.high,
                                                            de_kline.low,
                                                            de_kline.close,
                                                            Volume::TotalOnly(volume),
                                                            ticker_info.min_ticksize,
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
                                                        log::error!(
                                                            "Ticker info not found for ticker: {}",
                                                            ticker
                                                        );
                                                    }
                                                } else {
                                                    log::error!(
                                                        "Failed to find timeframe: {}, {:?}",
                                                        &de_kline.interval,
                                                        streams
                                                    );
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
                            }
                        }
                    }
                }
            }
        }
    })
}
