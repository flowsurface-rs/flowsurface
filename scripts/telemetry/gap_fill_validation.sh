#!/usr/bin/env bash
# Gap-fill validation: verify gap detection → recovery pipeline from production logs.
# Usage: bash scripts/telemetry/gap_fill_validation.sh [logfile]
# If no logfile is given, reads from the most recent app log.
set -euo pipefail

LOG="${1:-/tmp/flowsurface.log}"

if [[ ! -f "$LOG" ]]; then
    echo "ERROR: Log file not found: $LOG"
    echo "Usage: $0 [logfile]"
    echo "  Tip: Run app with 'cargo run 2>&1 | tee /tmp/flowsurface.log'"
    exit 1
fi

echo "=== Gap-Fill Validation Pipeline ==="
echo "Log: $LOG"
echo ""

PASS=0
WARN=0

# Phase 1: Gap detection events
echo "--- Phase 1: Gap Detection ---"
GAP_COUNT=$(grep -c '\[telemetry\] agg_trade_id GAP' "$LOG" 2>/dev/null || echo 0)
echo "  Gap events: $GAP_COUNT"
if [[ "$GAP_COUNT" -gt 0 ]]; then
    grep '\[telemetry\] agg_trade_id GAP' "$LOG" | tail -5
fi
echo ""

# Phase 2: Recovery triggers
echo "--- Phase 2: Recovery Triggers ---"
TRIGGER_COUNT=$(grep -c '\[gap-recovery\] triggering' "$LOG" 2>/dev/null || echo 0)
echo "  Trigger events: $TRIGGER_COUNT"
if [[ "$TRIGGER_COUNT" -gt 0 ]]; then
    grep '\[gap-recovery\] triggering' "$LOG" | tail -5
    PASS=$((PASS + 1))
else
    if [[ "$GAP_COUNT" -gt 0 ]]; then
        echo "  WARNING: Gaps detected but no recovery triggered"
        WARN=$((WARN + 1))
    fi
fi
echo ""

# Phase 3: Catchup completions
echo "--- Phase 3: Catchup Completions ---"
CATCHUP_COUNT=$(grep -c '\[catchup\].*trades' "$LOG" 2>/dev/null || echo 0)
echo "  Catchup events: $CATCHUP_COUNT"
if [[ "$CATCHUP_COUNT" -gt 0 ]]; then
    grep '\[catchup\].*trades' "$LOG" | tail -5
    PASS=$((PASS + 1))
fi
echo ""

# Phase 4: Fence applications
echo "--- Phase 4: Fence Applications ---"
FENCE_COUNT=$(grep -c '\[gap-fill\] finalize\|gap-fill\] complete' "$LOG" 2>/dev/null || echo 0)
echo "  Fence events: $FENCE_COUNT"
if [[ "$FENCE_COUNT" -gt 0 ]]; then
    grep '\[gap-fill\] finalize\|\[gap-fill\] complete' "$LOG" | tail -5
    PASS=$((PASS + 1))
fi
echo ""

# Phase 5: Bar-boundary replays
echo "--- Phase 5: Bar-Boundary Replays ---"
REPLAY_COUNT=$(grep -c '\[SSE\] reset ODB.*replayed' "$LOG" 2>/dev/null || echo 0)
echo "  Replay events: $REPLAY_COUNT"
if [[ "$REPLAY_COUNT" -gt 0 ]]; then
    grep '\[SSE\] reset ODB.*replayed' "$LOG" | tail -5
fi
echo ""

# Phase 6: Validation warnings
echo "--- Phase 6: Catchup Validation ---"
VALIDATION_WARN=$(grep -c '\[catchup-validation\]' "$LOG" 2>/dev/null || echo 0)
echo "  Validation warnings: $VALIDATION_WARN"
if [[ "$VALIDATION_WARN" -gt 0 ]]; then
    grep '\[catchup-validation\]' "$LOG" | tail -5
    WARN=$((WARN + 1))
fi
echo ""

# Phase 7: CH ground truth cross-check (optional, requires SSH)
echo "--- Phase 7: CH Ground Truth ---"
if ssh -o ConnectTimeout=3 bigblack 'echo ok' >/dev/null 2>&1; then
    echo "  Querying last 5 bars from ClickHouse..."
    ssh bigblack 'curl -s http://localhost:8123/ -d "
        SELECT close_time_ms, first_agg_trade_id, last_agg_trade_id,
               last_agg_trade_id - first_agg_trade_id + 1 AS trade_span
        FROM opendeviationbar_cache.open_deviation_bars
        WHERE symbol = '\''BTCUSDT'\'' AND threshold_decimal_bps = 250
          AND ouroboros_mode = '\''day'\''
        ORDER BY close_time_ms DESC LIMIT 5
        FORMAT PrettyCompact
    "' 2>/dev/null || echo "  CH query failed"
else
    echo "  Skipped (SSH to bigblack unavailable)"
fi
echo ""

# Summary
echo "=== Summary ==="
echo "  Gaps detected: $GAP_COUNT"
echo "  Recoveries triggered: $TRIGGER_COUNT"
echo "  Catchup completions: $CATCHUP_COUNT"
echo "  Fences applied: $FENCE_COUNT"
echo "  Replays: $REPLAY_COUNT"
echo "  Validation warnings: $VALIDATION_WARN"
echo ""

if [[ "$WARN" -gt 0 ]]; then
    echo "RESULT: $WARN warning(s) — review above"
    exit 1
elif [[ "$GAP_COUNT" -eq 0 ]]; then
    echo "RESULT: No gaps observed (clean run)"
else
    echo "RESULT: $PASS/3 pipeline phases confirmed"
fi
