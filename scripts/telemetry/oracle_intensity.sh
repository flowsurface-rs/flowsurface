#!/usr/bin/env bash
# Oracle verification: verify live ODB bars have correct microstructure + intensity bins.
#
# Usage:
#   ./scripts/telemetry/oracle_intensity.sh [log_file]
#
# Reads [oracle-micro], [oracle-rebuild-tail], [oracle-hist] log lines.
# Phase 2 queries ClickHouse directly (independent ground truth, not from the app).
#
# Requires: SSH access to bigblack (for CH direct query)

set -euo pipefail

LOG="${1:-$HOME/Library/Application Support/flowsurface/flowsurface-current.log}"

if [[ ! -f "$LOG" ]]; then
    echo "ERROR: Log file not found: $LOG"
    exit 1
fi

# Helper: extract value for key=value from a log line (no grep -P needed)
extract() {
    local key="$1" line="$2"
    echo "$line" | sed -n "s/.*${key}=\([^ ]*\).*/\1/p"
}

echo "=== Oracle Intensity Verification ==="
echo "Log: $(basename "$LOG")"
echo ""

# ─── Phase 1: Core assertion — stored_has_micro ──────────────────────────────
echo "--- Phase 1: Core assertion (stored_has_micro) ---"

ORACLE_LINES=$(grep '\[oracle-micro\]' "$LOG" 2>/dev/null || true)
if [[ -z "$ORACLE_LINES" ]]; then
    ORACLE_COUNT=0
else
    ORACLE_COUNT=$(echo "$ORACLE_LINES" | wc -l | tr -d ' ')
fi
echo "Found $ORACLE_COUNT oracle-micro entries"

FAIL_COUNT=0
PASS_COUNT=0
PROVISIONAL_COUNT=0

if [[ "$ORACLE_COUNT" -gt 0 ]]; then
    echo ""
    printf "%-19s | %-11s | %-11s | %-9s | %-13s | %-8s\n" \
        "bar_ts" "ch_ti" "stored_ti" "has_micro" "had_provisional" "verdict"
    printf "%-19s-+-%-11s-+-%-11s-+-%-9s-+-%-13s-+-%-8s\n" \
        "-------------------" "-----------" "-----------" "---------" "-------------" "--------"

    while IFS= read -r line; do
        bar_ts=$(extract "bar_ts" "$line")
        ch_ti=$(extract "ch_ti" "$line")
        stored_has_micro=$(extract "stored_has_micro" "$line")
        stored_ti=$(extract "stored_ti" "$line")
        had_prov=$(extract "had_provisional" "$line")

        if [[ "$had_prov" == "true" ]]; then
            ((PROVISIONAL_COUNT++)) || true
        fi

        if [[ "$stored_has_micro" == "true" ]]; then
            verdict="PASS"
            ((PASS_COUNT++)) || true
        else
            verdict="**FAIL**"
            ((FAIL_COUNT++)) || true
        fi

        printf "%-19s | %-11s | %-11s | %-9s | %-13s | %s\n" \
            "$bar_ts" "$ch_ti" "$stored_ti" "$stored_has_micro" "$had_prov" "$verdict"
    done <<< "$ORACLE_LINES"

    echo ""
    echo "Core assertion: $PASS_COUNT PASS, $FAIL_COUNT FAIL, $PROVISIONAL_COUNT had provisional bars"
    if [[ "$FAIL_COUNT" -gt 0 ]]; then
        echo "!! CRITICAL: $FAIL_COUNT bars stored WITHOUT microstructure — intensity coloring bug NOT fixed !!"
    fi
fi

# ─── Phase 2: Rebuild-tail assertion (bin != 0) ──────────────────────────────
echo ""
echo "--- Phase 2: Rebuild-tail assertion (newest bar bin != 0) ---"

REBUILD_TAIL=$(grep '\[oracle-rebuild-tail\]' "$LOG" 2>/dev/null || true)
REBUILD_COUNT=$(echo "$REBUILD_TAIL" | grep -c 'idx=' 2>/dev/null || echo "0")
echo "Found $REBUILD_COUNT rebuild-tail entries"

if [[ "$REBUILD_COUNT" -gt 0 ]]; then
    echo ""
    while IFS= read -r line; do
        idx=$(extract "idx" "$line")
        ti=$(extract "ti" "$line")
        bin_val=$(echo "$line" | sed -n 's/.*bin=\([0-9]*\)\/\([0-9]*\).*/\1/p')
        k_val=$(echo "$line" | sed -n 's/.*bin=\([0-9]*\)\/\([0-9]*\).*/\2/p')
        has_micro=$(extract "has_micro" "$line")
        bin_nonzero=$(extract "bin_nonzero" "$line")

        if [[ "$has_micro" == "true" && "$bin_nonzero" == "true" ]]; then
            echo "  idx=$idx ti=$ti bin=$bin_val/$k_val has_micro=$has_micro → PASS (thermal color active)"
        elif [[ "$has_micro" == "true" && "$bin_nonzero" == "false" ]]; then
            echo "  idx=$idx ti=$ti bin=$bin_val/$k_val has_micro=$has_micro → **FAIL** (bin=0 sentinel despite micro!)"
        else
            echo "  idx=$idx ti=$ti bin=$bin_val/$k_val has_micro=$has_micro → SKIP (no micro, expected blue)"
        fi
    done <<< "$REBUILD_TAIL"
fi

# ─── Phase 3: CH direct query (independent ground truth) ─────────────────────
echo ""
echo "--- Phase 3: ClickHouse direct query (independent ground truth) ---"

BAR_TIMESTAMPS=$(grep '\[oracle-micro\]' "$LOG" 2>/dev/null | sed -n 's/.*bar_ts=\([0-9]*\).*/\1/p' | sort -u || true)

if [[ -z "$BAR_TIMESTAMPS" ]]; then
    echo "No live bars to verify against CH yet."
else
    TS_LIST=$(echo "$BAR_TIMESTAMPS" | tr '\n' ',' | sed 's/,$//')

    CH_RESULT=$(ssh bigblack "curl -s 'http://localhost:8123/' -d \"
        SELECT
            close_time_ms,
            individual_trade_count,
            ofi,
            trade_intensity
        FROM opendeviationbar_cache.open_deviation_bars
        WHERE close_time_ms IN ($TS_LIST)
          AND symbol = 'BTCUSDT'
          AND threshold_dbps = 250
        ORDER BY close_time_ms
        FORMAT TabSeparatedWithNames
    \"" 2>/dev/null || echo "CH_QUERY_FAILED")

    if [[ "$CH_RESULT" == "CH_QUERY_FAILED" ]]; then
        echo "WARNING: ClickHouse query failed (SSH down?)"
    else
        echo "$CH_RESULT"
        echo ""
        # Verify app's ch_ti matches direct CH query (tests adapter correctness)
        echo "Cross-reference (adapter fidelity):"
        echo "$CH_RESULT" | tail -n +2 | while IFS=$'\t' read -r ch_ts ch_tc ch_ofi ch_ti; do
            ORACLE_LINE=$(grep "\[oracle-micro\] bar_ts=$ch_ts " "$LOG" 2>/dev/null | tail -1 || true)
            if [[ -n "$ORACLE_LINE" ]]; then
                log_ch_ti=$(extract "ch_ti" "$ORACLE_LINE")
                if awk "BEGIN{exit !($ch_ti == $log_ch_ti)}" 2>/dev/null; then
                    echo "  bar_ts=$ch_ts: CH_direct=$ch_ti == log_ch=$log_ch_ti → adapter OK"
                else
                    echo "  bar_ts=$ch_ts: CH_direct=$ch_ti != log_ch=$log_ch_ti → ADAPTER MISMATCH"
                fi
            fi
        done
    fi
fi

# ─── Phase 4: Historical baseline ────────────────────────────────────────────
echo ""
echo "--- Phase 4: Historical baseline (last 20 bars from load) ---"
HIST_COUNT=$(grep -c '\[oracle-hist\]' "$LOG" 2>/dev/null || echo "0")
echo "Found $HIST_COUNT historical oracle entries"

if [[ "$HIST_COUNT" -gt 0 ]]; then
    echo "Last 5:"
    grep '\[oracle-hist\]' "$LOG" | tail -5
fi

# ─── Phase 5: Bin distribution ────────────────────────────────────────────────
echo ""
echo "--- Phase 5: Bin distribution (from [oracle-bin] DEBUG lines) ---"
BIN_COUNT=$(grep -c '\[oracle-bin\]' "$LOG" 2>/dev/null || echo "0")
echo "Found $BIN_COUNT bin assignments"

if [[ "$BIN_COUNT" -gt 0 ]]; then
    echo ""
    echo "Bin frequency (should show spread, not all one value):"
    grep '\[oracle-bin\]' "$LOG" | sed -n 's/.*bin=\([0-9]*\)\/.*/\1/p' | sort -n | uniq -c | sort -rn
    echo ""
    ZERO_BINS=$(grep '\[oracle-bin\]' "$LOG" | sed -n 's/.*bin=\([0-9]*\)\/.*/\1/p' | grep -c '^0$' 2>/dev/null || true)
    ZERO_BINS=${ZERO_BINS:-0}
    ZERO_BINS=$(echo "$ZERO_BINS" | tr -d '[:space:]')
    if [[ "$ZERO_BINS" -gt 0 ]]; then
        echo "WARNING: $ZERO_BINS bars have bin=0 (sentinel) — check microstructure pipeline"
    else
        echo "No bin=0 sentinels — all bars have real intensity data."
    fi
fi

# ─── Phase 6: oracle-FAIL assertion check ─────────────────────────────────────
echo ""
FAIL_LINES=$(grep '\[oracle-FAIL\]' "$LOG" 2>/dev/null || true)
if [[ -n "$FAIL_LINES" ]]; then
    echo "!! ASSERTION FAILURES DETECTED !!"
    echo "$FAIL_LINES"
else
    echo "No oracle assertion failures."
fi

echo ""
echo "=== Done ==="
