# Data Crate

**Parent**: [/CLAUDE.md](/CLAUDE.md)

Data aggregation, chart models, indicator types, layout persistence, and configuration. Crate name: `flowsurface-data`.

---

## Quick Reference

| Module           | File                     | Purpose                                    |
| ---------------- | ------------------------ | ------------------------------------------ |
| Chart basis      | `src/chart.rs`           | `Basis` enum (Time, Tick, Odb)             |
| Chart types      | `src/chart/kline.rs`     | `KlineChartKind` (Candles, Odb, Footprint) |
| Indicators       | `src/chart/indicator.rs` | `KlineIndicator` enum (6 types)            |
| Tick aggregation | `src/aggr/ticks.rs`      | `TickAggr`, `RangeBarMicrostructure`       |
| Time aggregation | `src/aggr/time.rs`       | `TimeSeries`                               |
| Sessions         | `src/session.rs`         | NY/London/Tokyo boundaries + DST via jiff  |
| Pane layout      | `src/layout/pane.rs`     | `ContentKind` (serialization model)        |
| Timezone labels  | `src/config/timezone.rs` | `format_range_bar_label()` (x-axis)        |

---

## Basis System

The `Basis` enum determines how data is aggregated and rendered on the x-axis:

```rust
pub enum Basis {
    Time(Timeframe),    // Fixed intervals: 1m, 5m, 1h, 4h, 1d, 1w
    Tick(u16),          // N trades per bar
    #[serde(alias = "RangeBar")]
    Odb(u32),           // Threshold in dbps (e.g., 250 = 0.25%)
}

pub const ODB_THRESHOLDS: [u32; 4] = [250, 500, 750, 1000];
```

| Basis  | Storage      | X-Axis      | Data Source              |
| ------ | ------------ | ----------- | ------------------------ |
| `Time` | `TimeSeries` | Continuous  | Exchange WebSocket       |
| `Tick` | `TickAggr`   | Index-based | Exchange WebSocket       |
| `Odb`  | `TickAggr`   | Index-based | ClickHouse HTTP (cached) |

**Key difference**: Time-based charts have uniform spacing. Tick and ODB charts have non-uniform spacing (index-based, newest rightmost).

---

## Chart Types

```rust
pub enum KlineChartKind {
    Candles,                                  // Traditional OHLC candlesticks
    Odb,                                      // Open deviation bars (fork-specific)
    Footprint { clusters, scaling, studies },  // Price-clustered trade visualization
}
```

Matched in rendering, scaling, settings, and serialization code. When adding behavior, check all match sites.

---

## Indicator System

### KlineIndicator Enum

```rust
pub enum KlineIndicator {
    Volume,          // Buy/sell volume stacked bars
    OpenInterest,    // Perpetuals only (futures open contracts)
    Delta,           // Buy vol - Sell vol (signed bars)
    TradeCount,      // Trade count histogram (ODB only)
    OFI,             // Order Flow Imbalance line (ODB only)
    TradeIntensity,  // Trades/sec heatmap (ODB only)
}
```

The last three (TradeCount, OFI, TradeIntensity) only have data for ODB charts — they come from ClickHouse microstructure fields. See [exchange/CLAUDE.md](../exchange/CLAUDE.md) for field mapping.

### Indicator Storage

Both `TradeIntensityHeatmapIndicator` and `OFICumulativeEmaIndicator` use `Vec<T>` (not `BTreeMap`) for O(1) incremental updates. Index = forward storage index matching `TickSeries::datapoints` order. Gap sentinels are used for missing data.

### TradeIntensityHeatmap Algorithm

Colors each ODB candle body by percentile rank of trade intensity within a rolling lookback window.

```
log_val  = log10(intensity).max(0.0)
k_actual = round(cbrt(window.len())).max(5)   ← cube-root rule, fully adaptive
rank     = count(window ≤ log_val) / window.len()   ← binary search, no look-ahead
bin      = ceil(rank × k_actual).clamp(1, k_actual)
t        = (bin − 1) / (k_actual − 1)              ← normalised to [0, 1]
color    = thermal_color(t)                         ← 300° HSV blue→magenta
push log_val to ring-buffer; evict oldest if len > lookback
```

**K thresholds** (`adaptive_k(n)` in `data/src/chart/kline.rs`):

| Lookback | Bars needed to saturate | K achieved |
| -------- | ----------------------- | ---------- |
| 2000     | 2000                    | 13         |
| 7000     | 6332                    | 19 (max)   |

**Color palette**: 300° HSV hue sweep — blue (240°) → cyan → green → yellow → orange → red → magenta (300°). At K=19: ≈16.7°/bin = 8× the ~2° just-noticeable-difference threshold. All 19 bins are perceptually distinct.

**Oracle**: `log_oracle_spectrum()` fires on every `rebuild_from_source()` and writes to `/tmp/flowsurface-oracle.log`. Shows colour table, all-K histogram, and filtered histogram for `k_actual==K_current` only. Use to verify bin distribution and colour distinctness.

> ⚠️ **CRITICAL — bar count dependency**: `k_actual` is computed from the current rolling window size, NOT the `lookback` setting directly. To reach K=19, the TickAggr must have ≥ 6332 bars loaded. This requires the ClickHouse initial fetch to use the **full-reload path** (`FetchRange::Kline(0, u64::MAX)`) which applies the adaptive limit (13K-20K bars). If `missing_data_task()` accidentally uses `FetchRange::Kline(0, now_ms)`, it hits `LIMIT 2000` and K can never exceed 13. See [exchange/CLAUDE.md](../exchange/CLAUDE.md) for the `u64::MAX` sentinel pattern.

---

## Data Aggregation

### TickAggr (Vec-based, oldest-first)

Used for **Tick** and **ODB** basis. Bars stored in a `Vec<TickAccumulation>` ordered oldest-first.

```rust
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,                                    // kline.time = close_time_ms
    pub footprint: KlineTrades,
    pub microstructure: Option<OdbMicrostructure>,       // Fork-specific
    pub agg_trade_id_range: Option<(u64, u64)>,          // Fork-specific (ODB only)
    pub open_time_ms: Option<u64>,                       // Fork-specific (ODB only)
}

pub struct OdbMicrostructure {
    pub trade_count: u32,
    pub ofi: f32,
    pub trade_intensity: f32,
}
```

> ⚠️ **`open_time_ms` is NOT the same as `kline.time`** for ODB bars:
>
> - `kline.time` = `close_time_ms` (the trigger trade that closed bar N)
> - `open_time_ms` = `open_time_ms` from ClickHouse (the first trade of bar N)
>
> These differ by the gap between the closing trigger trade and the first new-session trade (typically 100–500ms, but can span a UTC day boundary). `open_time_ms` is used exclusively by `draw_crosshair_tooltip()` in `src/chart/kline.rs` to display correct open time. **Do not use `prev_bar.kline.time` as a substitute for `this_bar.open_time_ms`.**
>
> **Threading chain**: `ChKline.open_time_ms` → `fetch_klines_with_microstructure()` return tuple (4th element) → `FetchedData::Klines.open_time_ms_list` → `insert_odb_klines()` → `from_klines_with_microstructure()` / `prepend_klines_with_microstructure()`. For streaming bars: `KlineReceived` 6th field → `update_latest_kline(bar_open_time_ms)` → attached to `last_dp.open_time_ms` after `replace_or_append_kline`.

**Bar completion dispatch**: `TickAggr.range_bar_threshold_dbps: Option<u32>`:

- `None` → tick-count based: `is_full(tick_count)` (Tick basis)
- `Some(dbps)` → price-range based: `is_full_range_bar(dbps)` uses integer `Price.units` math (ODB basis)

**ClickHouse reconciliation**: `replace_or_append_kline()` replaces the locally-built forming bar with the authoritative ClickHouse completed bar when timestamps match.

---

## Pane Serialization

**File**: `src/layout/pane.rs`

```rust
pub enum ContentKind {
    Starter,
    HeatmapChart,
    FootprintChart,
    CandlestickChart,
    OdbChart,          // Fork-specific
    ComparisonChart,
    TimeAndSales,
    Ladder,
}
```

Pane state persisted to `~/Library/Application Support/flowsurface/saved-state.json`.

---

## Session Lines

**File**: `src/session.rs`

Renders NY/London/Tokyo trading session boundaries as dotted lines + colored strips. Automatic DST handling via jiff timezone library. Works on both Time-based and ODB chart bases via binary search coordinate mapping.

---

## Related

- [/CLAUDE.md](/CLAUDE.md) — Project hub
- [/exchange/CLAUDE.md](../exchange/CLAUDE.md) — Exchange adapters, ClickHouse
