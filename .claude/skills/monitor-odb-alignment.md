# /monitor-odb-alignment

Monitor ODB (Open Deviation Bar) alignment between SSE, ClickHouse, and local RBP.

## What it does

1. Reads the app log at `~/Library/Application Support/flowsurface/flowsurface-current.log`
2. Queries ClickHouse (via bigblack) for the latest authoritative bars
3. Compares timestamps and OHLCV values to detect:
   - Duplicate bars (local + SSE with different timestamps)
   - Price deltas > $1 between SSE and local RBP
   - SKIPPED(sse) actions (expected when SSE gating works)
   - Missing SSE bars (SSE connected but no bars delivered)
4. Reports findings and suggests fixes

## Usage

```
/monitor-odb-alignment
```

## Steps

### Step 1: Check app is running

```bash
pgrep -f "flowsurface.bin" || echo "NOT RUNNING"
```

### Step 2: Parse current log

```bash
LOG="$HOME/Library/Application Support/flowsurface/flowsurface-current.log"

# SSE status
grep -c "\[SSE\] connected" "$LOG"
grep "\[SSE\].*bar" "$LOG" | tail -5

# RBP bar completions
grep "BAR COMPLETED" "$LOG" | tail -5

# SKIPPED actions (SSE gating working correctly)
grep "SKIPPED(sse)" "$LOG" | tail -5

# Check for APPEND actions (potential duplicates)
grep "action=APPEND" "$LOG" | tail -5

# Forming bar status
grep "RBP.*forming" "$LOG" | tail -3
```

### Step 3: Query ClickHouse for latest bars

```bash
ssh bigblack 'curl -s http://localhost:8123/ -d "
  SELECT close_time_ms, open, high, low, close, buy_volume, sell_volume
  FROM opendeviationbar_cache.open_deviation_bars
  WHERE symbol = '\''BTCUSDT'\'' AND threshold_decimal_bps = 250
  ORDER BY close_time_ms DESC
  LIMIT 3
  FORMAT JSONEachRow
"'
```

### Step 4: Compare SSE bar timestamps with CH

- SSE bars should match CH bars exactly (same source — sidecar writes to both)
- Local RBP bars should be SKIPPED when SSE is active
- If local RBP bars are APPENDED, that's a regression

### Step 5: Diagnose and fix

- If SSE not connected: check tunnel (`lsof -ti:18081`), sidecar health
- If bars APPENDED instead of SKIPPED: `sse_enabled()` returning false — check env vars
- If price delta > $1: cold-start misalignment still occurring
- If no bars at all: BTC hasn't moved 0.25% — wait longer or use BPR50 (0.5%) for faster bars
