//! Integration tests for ODB gap-fill via ClickHouse + sidecar.
//!
//! All tests require:
//!   1. SSH tunnel to bigblack (`mise run tunnel:start`)
//!   2. ClickHouse with `opendeviationbar_cache.open_deviation_bars` table
//!   3. ODB sidecar running on SSE_HOST:SSE_PORT
//!
//! Run with: `cargo nextest run --workspace --run-ignored=only`

use flowsurface_exchange::adapter::clickhouse;

/// Verify that `fetch_catchup` returns trades for BTCUSDT@250 with monotonic IDs.
#[tokio::test]
#[ignore]
async fn catchup_returns_trades_for_btcusdt_250() {
    let result = clickhouse::fetch_catchup("BTCUSDT", 250)
        .await
        .expect("fetch_catchup should succeed with tunnel active");

    assert!(!result.trades.is_empty(), "catchup should return trades");

    // Verify monotonically ascending agg_trade_ids
    let ids: Vec<u64> = result
        .trades
        .iter()
        .filter_map(|t| t.agg_trade_id)
        .collect();
    for window in ids.windows(2) {
        assert!(
            window[0] < window[1],
            "trades must be ascending: {} >= {}",
            window[0],
            window[1]
        );
    }

    // through_agg_id should match the last trade
    if let Some(through) = result.through_agg_id {
        let last_id = ids.last().copied().unwrap();
        assert_eq!(
            through, last_id,
            "through_agg_id should match last trade ID"
        );
    }

    // Validation should produce no critical warnings (count mismatch, misordering)
    let critical_warnings: Vec<_> = result
        .warnings
        .iter()
        .filter(|w| w.contains("mismatch") || w.contains("misordered"))
        .collect();
    assert!(
        critical_warnings.is_empty(),
        "no critical validation warnings expected: {critical_warnings:?}"
    );

    println!(
        "catchup OK: {} trades, through={:?}, partial={}, warnings={:?}, uuid={}",
        result.trades.len(),
        result.through_agg_id,
        result.partial,
        result.warnings,
        result.request_uuid,
    );
}

/// Verify catchup's through_agg_id >= last CH bar's last_agg_trade_id.
#[tokio::test]
#[ignore]
async fn catchup_through_id_gte_last_ch_bar() {
    // Query CH for the last bar's last_agg_trade_id
    let sql = "SELECT last_agg_trade_id FROM opendeviationbar_cache.open_deviation_bars \
               WHERE symbol = 'BTCUSDT' AND threshold_decimal_bps = 250 \
               AND ouroboros_mode = 'day' \
               ORDER BY close_time_ms DESC LIMIT 1 FORMAT JSONEachRow";

    let ch_response = clickhouse::query(sql)
        .await
        .expect("CH query should succeed");

    #[derive(serde::Deserialize)]
    struct Row {
        last_agg_trade_id: u64,
    }
    let row: Row = serde_json::from_str(ch_response.trim()).expect("should parse CH row");

    let result = clickhouse::fetch_catchup("BTCUSDT", 250)
        .await
        .expect("fetch_catchup should succeed");

    if let Some(through) = result.through_agg_id {
        assert!(
            through >= row.last_agg_trade_id,
            "catchup through_agg_id ({through}) should be >= last CH bar's last_agg_trade_id ({})",
            row.last_agg_trade_id
        );
    }
}

/// Verify recent CH bars have continuous agg_trade_id ranges (no large gaps).
#[tokio::test]
#[ignore]
async fn ch_bars_have_continuous_agg_trade_ids() {
    let sql = "SELECT first_agg_trade_id, last_agg_trade_id, close_time_ms \
               FROM opendeviationbar_cache.open_deviation_bars \
               WHERE symbol = 'BTCUSDT' AND threshold_decimal_bps = 250 \
               AND ouroboros_mode = 'day' \
               ORDER BY close_time_ms DESC LIMIT 50 FORMAT JSONEachRow";

    let ch_response = clickhouse::query(sql)
        .await
        .expect("CH query should succeed");

    #[derive(serde::Deserialize)]
    struct Row {
        first_agg_trade_id: u64,
        last_agg_trade_id: u64,
        close_time_ms: u64,
    }

    let mut rows: Vec<Row> = ch_response
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("should parse CH row"))
        .collect();

    // Reverse to ascending order
    rows.reverse();

    assert!(rows.len() >= 2, "need at least 2 bars for continuity check");

    let mut large_gaps = 0;
    for window in rows.windows(2) {
        let prev_last = window[0].last_agg_trade_id;
        let curr_first = window[1].first_agg_trade_id;

        // No overlaps
        assert!(
            curr_first > prev_last,
            "bars should not overlap: prev.last={prev_last} >= curr.first={curr_first} \
             (prev_close_ms={}, curr_close_ms={})",
            window[0].close_time_ms,
            window[1].close_time_ms
        );

        // Warn on large gaps (>1000 trades between bars)
        let gap = curr_first - prev_last;
        if gap > 1000 {
            large_gaps += 1;
            eprintln!(
                "WARNING: large gap of {gap} between bars at close_ms={} and {}",
                window[0].close_time_ms, window[1].close_time_ms
            );
        }
    }

    assert!(
        large_gaps <= 2,
        "too many large gaps ({large_gaps}) in recent 50 bars"
    );
}
