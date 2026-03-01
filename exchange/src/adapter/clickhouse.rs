//! ClickHouse adapter for precomputed range bars from rangebar-py cache.
//!
//! Reads from `rangebar_cache.range_bars` table via ClickHouse HTTP interface.
//! Tickers come from real exchanges (e.g. Binance) — the symbol in ClickHouse
//! is just the base symbol name like "BTCUSDT".
//!
//! Environment variables:
//!   FLOWSURFACE_CH_HOST (default: "bigblack")
//!   FLOWSURFACE_CH_PORT (default: 8123)

use super::{
    super::{Kline, TickerInfo, Trade, Volume},
    AdapterError, Event, StreamKind,
};
use crate::unit::{MinTicksize, Qty};

use iced_futures::{
    futures::{SinkExt, Stream},
    stream,
};
use serde::Deserialize;

use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

pub use rangebar_core::{FixedPoint, RangeBar, RangeBarProcessor};

/// Microstructure fields from ClickHouse range bar cache.
/// Kept in exchange crate to avoid circular dependency with data crate.
#[derive(Debug, Clone, Copy)]
pub struct ChMicrostructure {
    pub trade_count: u32,
    pub ofi: f32,
    pub trade_intensity: f32,
}

// === rangebar-core in-process integration ===
// GitHub Issue: https://github.com/terrylica/rangebar-py/issues/97

/// Convert a flowsurface Trade into a rangebar-core AggTrade.
///
/// Both Price and FixedPoint use i64 with 10^8 scale, so price conversion
/// is a direct copy of the underlying units. Volume uses f32→FixedPoint
/// via string round-trip for precision.
pub fn trade_to_agg_trade(trade: &Trade, seq_id: i64) -> rangebar_core::AggTrade {
    // Binance WebSocket trades have millisecond timestamps. rangebar-core uses
    // microseconds and has a same-timestamp gate (prevent_same_timestamp_close)
    // that blocks bar closure when trade.timestamp == bar.open_time.
    // Add sub-millisecond offset from seq_id so trades within the same ms batch
    // get unique µs timestamps, preventing the gate from stalling bar completion.
    let base_us = (trade.time as i64) * 1000;
    let sub_ms_offset = seq_id % 1000; // 0-999 µs within the millisecond
    rangebar_core::AggTrade {
        agg_trade_id: seq_id,
        price: FixedPoint(trade.price.units),
        volume: FixedPoint((f32::from(trade.qty) as f64 * 1e8) as i64),
        first_trade_id: seq_id,
        last_trade_id: seq_id,
        timestamp: base_us + sub_ms_offset,
        is_buyer_maker: trade.is_sell,
        is_best_match: None,
    }
}

/// Convert a completed rangebar-core RangeBar into a flowsurface Kline.
pub fn range_bar_to_kline(bar: &RangeBar, min_tick: MinTicksize) -> Kline {
    let scale = rangebar_core::fixed_point::SCALE as f64;
    Kline::new(
        (bar.close_time / 1000) as u64, // µs → ms
        bar.open.to_f64() as f32,
        bar.high.to_f64() as f32,
        bar.low.to_f64() as f32,
        bar.close.to_f64() as f32,
        Volume::BuySell(
            Qty::from((bar.buy_volume as f64 / scale) as f32),
            Qty::from((bar.sell_volume as f64 / scale) as f32),
        ),
        min_tick,
    )
}

/// Extract microstructure indicators from a completed RangeBar.
pub fn range_bar_to_microstructure(bar: &RangeBar) -> ChMicrostructure {
    ChMicrostructure {
        trade_count: bar.individual_trade_count,
        ofi: bar.ofi as f32,
        trade_intensity: bar.trade_intensity as f32,
    }
}

/// Range bar symbols fetched from ClickHouse at startup.
/// Populated by `init_range_bar_symbols()`, accessed synchronously from view code.
static RANGE_BAR_SYMBOLS: OnceLock<Vec<String>> = OnceLock::new();

/// Fetch available range bar symbols from ClickHouse and cache them.
/// Called once at startup; gracefully returns empty vec on failure.
pub async fn init_range_bar_symbols() -> Vec<String> {
    let sql = "SELECT DISTINCT symbol FROM rangebar_cache.range_bars ORDER BY symbol FORMAT TabSeparated";
    match query(sql).await {
        Ok(body) => {
            let symbols: Vec<String> = body
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect();
            let count = symbols.len();
            if RANGE_BAR_SYMBOLS.set(symbols).is_err() {
                log::warn!("range bar symbol cache already initialized");
            } else {
                log::info!("cached {count} range bar symbols from ClickHouse");
            }
            // Non-blocking schema coherence check after successful connection
            validate_schema().await;
        }
        Err(e) => {
            log::warn!("failed to fetch range bar symbols from ClickHouse: {e}");
        }
    }
    RANGE_BAR_SYMBOLS.get().cloned().unwrap_or_default()
}

/// Startup schema coherence check — logs column presence and rangebar-py version.
/// Non-fatal: logs warnings on mismatch, never blocks startup.
async fn validate_schema() {
    // Check expected columns exist in the range_bars table
    let expected_cols = [
        "close_time_ms", "open_time_ms", "open", "high", "low", "close",
        "buy_volume", "sell_volume", "individual_trade_count", "ofi", "trade_intensity",
    ];
    let col_sql = "SELECT name FROM system.columns \
                   WHERE database = 'rangebar_cache' AND table = 'range_bars' \
                   FORMAT TabSeparated";
    match query(col_sql).await {
        Ok(body) => {
            let actual: Vec<&str> = body.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
            let missing: Vec<&str> = expected_cols.iter()
                .filter(|c| !actual.iter().any(|a| a == *c))
                .copied()
                .collect();
            if missing.is_empty() {
                log::info!("[CH schema] all {}/{} expected columns present",
                    expected_cols.len(), expected_cols.len());
            } else {
                log::warn!("[CH schema] MISSING columns: {missing:?} — indicators may show no data");
            }
        }
        Err(e) => {
            log::warn!("[CH schema] column check failed: {e}");
        }
    }

    // Query rangebar_version from most recent bar (silent if column absent)
    let ver_sql = "SELECT rangebar_version FROM rangebar_cache.range_bars \
                   ORDER BY close_time_ms DESC LIMIT 1 FORMAT TabSeparated";
    match query(ver_sql).await {
        Ok(body) => {
            if let Some(version) = body.lines().next().map(|l| l.trim()).filter(|l| !l.is_empty()) {
                log::info!("[CH schema] rangebar-py version: {version}");
            }
        }
        Err(_) => {
            // rangebar_version column may not exist on older schemas — silently skip
        }
    }
}

/// Returns the range bar symbol allowlist, or None if not yet loaded or empty.
pub fn range_bar_symbol_filter() -> Option<&'static [String]> {
    RANGE_BAR_SYMBOLS
        .get()
        .filter(|v| !v.is_empty())
        .map(|v| v.as_slice())
}

static CLICKHOUSE_HOST: LazyLock<String> = LazyLock::new(|| {
    std::env::var("FLOWSURFACE_CH_HOST").unwrap_or_else(|_| "bigblack".to_string())
});

static CLICKHOUSE_PORT: LazyLock<u16> = LazyLock::new(|| {
    std::env::var("FLOWSURFACE_CH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8123)
});

fn base_url() -> String {
    format!("http://{}:{}", *CLICKHOUSE_HOST, *CLICKHOUSE_PORT)
}

/// Shared HTTP client — reuses connections through the SSH tunnel instead of
/// creating a new TCP handshake per request.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(2)
        .build()
        .expect("reqwest client build")
});

async fn query(sql: &str) -> Result<String, AdapterError> {
    let url = base_url();
    let sql_preview: String = sql.chars().take(120).collect();
    log::debug!("[CH] POST {url} — {sql_preview}…");

    let resp = HTTP_CLIENT
        .post(&url)
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| {
            log::error!("[CH] reqwest failed: {e} (is_timeout={}, is_connect={}, url={url})",
                e.is_timeout(), e.is_connect());
            AdapterError::FetchError(e)
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        log::error!("[CH] HTTP {status}: {body} — SQL: {sql_preview}…");
        return Err(AdapterError::ParseError(format!(
            "ClickHouse HTTP {}: {}",
            status, body
        )));
    }

    resp.text().await.map_err(|e| {
        log::error!("[CH] response body read failed: {e}");
        AdapterError::from(e)
    })
}

/// Extract the bare symbol name from a ticker (e.g. "BTCUSDT" from a BinanceLinear ticker).
/// ClickHouse stores symbols without exchange suffixes.
fn bare_symbol(ticker_info: &TickerInfo) -> String {
    ticker_info.ticker.to_string()
}

// -- Kline data --

#[derive(Debug, Deserialize)]
struct ChKline {
    close_time_ms: i64,
    #[serde(default, rename = "open_time_ms")]
    _open_time_ms: Option<i64>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    buy_volume: f64,
    sell_volume: f64,
    #[serde(default)]
    individual_trade_count: Option<u32>,
    #[serde(default)]
    ofi: Option<f64>,
    #[serde(default)]
    trade_intensity: Option<f64>,
}

pub async fn fetch_klines(
    ticker_info: TickerInfo,
    threshold_dbps: u32,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, AdapterError> {
    let symbol = bare_symbol(&ticker_info);
    let min_tick = ticker_info.min_ticksize;

    let sql = build_range_bar_sql(&symbol, threshold_dbps, range);

    let body = query(&sql).await?;
    let mut klines = Vec::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ck: ChKline = serde_json::from_str(line)
            .map_err(|e| AdapterError::ParseError(format!("ClickHouse kline parse: {e}")))?;

        klines.push(Kline::new(
            ck.close_time_ms as u64,
            ck.open as f32,
            ck.high as f32,
            ck.low as f32,
            ck.close as f32,
            Volume::BuySell(Qty::from(ck.buy_volume as f32), Qty::from(ck.sell_volume as f32)),
            min_tick,
        ));
    }

    // DESC order → reverse to ascending (oldest first)
    klines.reverse();

    Ok(klines)
}

/// Shared SQL builder for range bar queries (includes microstructure columns).
///
/// The initial fetch limit is scaled inversely with threshold so all thresholds
/// show a similar time window. BPR25 (250 dbps) is the reference at 500 bars;
/// BPR50 gets ~250, BPR100 gets ~125.
fn build_range_bar_sql(symbol: &str, threshold_dbps: u32, range: Option<(u64, u64)>) -> String {
    // Both paths use DESC ordering + reverse to get the N most recent bars
    // within the requested window. ASC ordering would return bars from the
    // beginning of time, creating gaps when loading historical data.
    let cols = "close_time_ms, open_time_ms, open, high, low, close, buy_volume, sell_volume, \
                individual_trade_count, ofi, trade_intensity";
    if let Some((start, end)) = range {
        format!(
            "SELECT {cols} \
             FROM rangebar_cache.range_bars \
             WHERE symbol = '{symbol}' AND threshold_decimal_bps = {threshold_dbps} \
               AND close_time_ms BETWEEN {start} AND {end} \
             ORDER BY close_time_ms DESC \
             LIMIT 2000 \
             FORMAT JSONEachRow"
        )
    } else {
        // Scale limit inversely with threshold: BPR25 gets 20,000 bars;
        // all thresholds get a minimum of 13,000 bars to fully populate
        // a 7,000-bar intensity lookback window from the first render.
        let reference_dbps = 250u32;
        let reference_limit = 20_000u32;
        let limit = ((reference_limit as f64) * (reference_dbps as f64) / (threshold_dbps as f64))
            .round()
            .max(13_000.0) as u32;
        format!(
            "SELECT {cols} \
             FROM rangebar_cache.range_bars \
             WHERE symbol = '{symbol}' AND threshold_decimal_bps = {threshold_dbps} \
             ORDER BY close_time_ms DESC \
             LIMIT {limit} \
             FORMAT JSONEachRow"
        )
    }
}

fn parse_microstructure(ck: &ChKline) -> Option<ChMicrostructure> {
    match (ck.individual_trade_count, ck.ofi, ck.trade_intensity) {
        (Some(tc), Some(ofi), Some(ti)) => Some(ChMicrostructure {
            trade_count: tc,
            ofi: ofi as f32,
            trade_intensity: ti as f32,
        }),
        _ => None,
    }
}

/// Fetch klines + microstructure sidecar from ClickHouse range bar cache.
pub async fn fetch_klines_with_microstructure(
    ticker_info: TickerInfo,
    threshold_dbps: u32,
    range: Option<(u64, u64)>,
) -> Result<(Vec<Kline>, Vec<Option<ChMicrostructure>>), AdapterError> {
    let symbol = bare_symbol(&ticker_info);
    let min_tick = ticker_info.min_ticksize;
    let sql = build_range_bar_sql(&symbol, threshold_dbps, range);

    let body = query(&sql).await?;
    let mut klines = Vec::new();
    let mut micro = Vec::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ck: ChKline = serde_json::from_str(line)
            .map_err(|e| AdapterError::ParseError(format!("ClickHouse kline parse: {e}")))?;

        klines.push(Kline::new(
            ck.close_time_ms as u64,
            ck.open as f32,
            ck.high as f32,
            ck.low as f32,
            ck.close as f32,
            Volume::BuySell(Qty::from(ck.buy_volume as f32), Qty::from(ck.sell_volume as f32)),
            min_tick,
        ));
        micro.push(parse_microstructure(&ck));
    }

    // DESC order → reverse to ascending (oldest first)
    klines.reverse();
    micro.reverse();

    Ok((klines, micro))
}

// -- Backfill request (Issue #97: on-demand trigger for rangebar-py) --

/// Request a backfill by inserting into the backfill_requests table.
/// Returns Ok(true) if the request was inserted, Ok(false) if a recent
/// pending/running request already exists (dedup within 5 minutes).
pub async fn request_backfill(
    symbol: &str,
    threshold_dbps: u32,
) -> Result<bool, AdapterError> {
    // Check for recent pending/running request to avoid spam
    let check_sql = format!(
        "SELECT count() as cnt \
         FROM rangebar_cache.backfill_requests FINAL \
         WHERE symbol = '{symbol}' AND status IN ('pending', 'running') \
           AND requested_at > now64(3) - INTERVAL 5 MINUTE \
         FORMAT JSONEachRow"
    );

    let body = query(&check_sql).await?;
    let existing: u64 = body
        .lines()
        .find_map(|line| {
            serde_json::from_str::<serde_json::Value>(line.trim())
                .ok()
                .and_then(|v| v["cnt"].as_u64())
        })
        .unwrap_or(0);

    if existing > 0 {
        log::info!("[CH backfill] request already pending for {symbol}");
        return Ok(false);
    }

    let insert_sql = format!(
        "INSERT INTO rangebar_cache.backfill_requests \
         (symbol, threshold_decimal_bps, source, ouroboros_mode) VALUES \
         ('{symbol}', {threshold_dbps}, 'flowsurface', 'month')"
    );

    query(&insert_sql).await?;
    log::info!("[CH backfill] requested backfill for {symbol} @ {threshold_dbps} dbps");
    Ok(true)
}

// -- Streaming (polling) --

pub fn connect_kline_stream(
    ticker_info: TickerInfo,
    threshold_dbps: u32,
) -> impl Stream<Item = Event> {
    // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/91
    log::info!(
        "[CH poll] connect_kline_stream STARTED: {} @{} dbps",
        ticker_info.ticker, threshold_dbps
    );
    stream::channel(16, async move |mut output| {
        let exchange = ticker_info.exchange();
        let _ = output.send(Event::Connected(exchange)).await;

        let stream_kind = StreamKind::RangeBarKline {
            ticker_info,
            threshold_dbps,
        };

        let symbol = bare_symbol(&ticker_info);

        // Initialize last_ts to the latest bar's timestamp so the first poll
        // doesn't re-fetch bars already loaded by the initial fetch_klines().
        let mut last_ts: u64 = {
            let sql = format!(
                "SELECT max(close_time_ms) AS ts FROM rangebar_cache.range_bars \
                 WHERE symbol = '{}' AND threshold_decimal_bps = {} FORMAT JSONEachRow",
                symbol, threshold_dbps
            );
            match query(&sql).await {
                Ok(body) => {
                    let ts = body.lines()
                        .find_map(|line| {
                            serde_json::from_str::<serde_json::Value>(line.trim())
                                .ok()
                                .and_then(|v| v["ts"].as_u64())
                        })
                        .unwrap_or(0);
                    log::info!("[CH poll] init last_ts={} for {} @{}", ts, symbol, threshold_dbps);
                    ts
                }
                Err(e) => {
                    log::warn!("[CH poll] init query failed for {} @{}: {}", symbol, threshold_dbps, e);
                    0
                }
            }
        };

        let mut logged_micro_warning = false;

        loop {
            // GitHub Issue: https://github.com/terrylica/rangebar-py/issues/91
            // 5s polling for near-real-time range bar updates (from 60s)
            tokio::time::sleep(Duration::from_secs(5)).await;

            let sql = format!(
                "SELECT close_time_ms, open_time_ms, open, high, low, close, buy_volume, sell_volume, \
                        individual_trade_count, ofi, trade_intensity \
                 FROM rangebar_cache.range_bars \
                 WHERE symbol = '{}' AND threshold_decimal_bps = {} \
                   AND close_time_ms > {} \
                 ORDER BY close_time_ms ASC \
                 LIMIT 100 \
                 FORMAT JSONEachRow",
                symbol, threshold_dbps, last_ts
            );

            match query(&sql).await {
                Ok(body) => {
                    let mut count = 0u32;
                    for line in body.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(ck) = serde_json::from_str::<ChKline>(line) {
                            let ts = ck.close_time_ms as u64;
                            if ts > last_ts {
                                last_ts = ts;
                            }
                            let kline = Kline::new(
                                ts,
                                ck.open as f32,
                                ck.high as f32,
                                ck.low as f32,
                                ck.close as f32,
                                Volume::BuySell(Qty::from(ck.buy_volume as f32), Qty::from(ck.sell_volume as f32)),
                                ticker_info.min_ticksize,
                            );
                            let _ = output.send(Event::KlineReceived(stream_kind, kline)).await;
                            count += 1;
                        }
                    }
                    if count > 0 {
                        log::info!(
                            "[CH poll] {} @{}: {} new bars, last_ts={}",
                            symbol, threshold_dbps, count, last_ts
                        );
                        // One-time warning if first polled bar lacks microstructure
                        if !logged_micro_warning {
                            logged_micro_warning = true;
                            if let Some(first_line) = body.lines().find(|l| !l.trim().is_empty()) {
                                if let Ok(ck) = serde_json::from_str::<ChKline>(first_line.trim()) {
                                    if ck.individual_trade_count.is_none()
                                        && ck.ofi.is_none()
                                        && ck.trade_intensity.is_none()
                                    {
                                        log::warn!(
                                            "[CH poll] {} @{}: bars missing microstructure \
                                             — check rangebar-py feature toggles",
                                            symbol, threshold_dbps
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("[CH poll] {} @{}: query error: {}", symbol, threshold_dbps, e);
                }
            }
        }
    })
}
