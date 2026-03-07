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

---

## Data Aggregation

### TickAggr (Vec-based, oldest-first)

Used for **Tick** and **ODB** basis. Bars stored in a `Vec<TickAccumulation>` ordered oldest-first.

```rust
pub struct TickAccumulation {
    pub tick_count: usize,
    pub kline: Kline,
    pub footprint: KlineTrades,
    pub microstructure: Option<RangeBarMicrostructure>,  // Fork-specific
}

pub struct RangeBarMicrostructure {
    pub trade_count: u32,
    pub ofi: f32,
    pub trade_intensity: f32,
}
```

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
