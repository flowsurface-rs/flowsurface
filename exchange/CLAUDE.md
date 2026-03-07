# Exchange Crate

**Parent**: [/CLAUDE.md](/CLAUDE.md)

Exchange adapters, WebSocket/REST stream handling, and type definitions. Crate name: `flowsurface-exchange`.

---

## Quick Reference

| Adapter         | File             | Protocol                           | Markets                    |
| --------------- | ---------------- | ---------------------------------- | -------------------------- |
| **ClickHouse**  | `clickhouse.rs`  | HTTP poll + SSE (opendeviationbar) | ODB range bars from cache  |
| **Binance**     | `binance.rs`     | REST + WebSocket                   | Spot, Linear/Inverse Perps |
| **Bybit**       | `bybit.rs`       | REST + WebSocket                   | Perpetuals                 |
| **Hyperliquid** | `hyperliquid.rs` | REST + WebSocket                   | DEX Perpetuals             |
| **OKX**         | `okex.rs`        | REST + WebSocket                   | Multi-product              |

---

## StreamKind (Current)

```rust
pub enum StreamKind {
    Kline { ticker_info, timeframe },        // Time-based candles
    OdbKline { ticker_info, threshold_dbps }, // Fork: ODB from ClickHouse
    Depth { ticker_info, depth_aggr, push_freq },
    Trades { ticker_info },
}
```

**Stream routing** in `adapter.rs`:

```
StreamKind::Kline      → binance/bybit/okex/hyperliquid WebSocket
StreamKind::OdbKline   → clickhouse::connect_kline_stream() (HTTP poll, 5s)
StreamKind::Depth      → exchange WebSocket (orderbook)
StreamKind::Trades     → exchange WebSocket (@aggTrade)
```

---

## ClickHouse Adapter (Fork-Specific)

**File**: `src/adapter/clickhouse.rs`

Reads precomputed ODB bars from opendeviationbar-py's ClickHouse cache via HTTP + SSE.

### Connection

| Setting | Default    | Override env var      |
| ------- | ---------- | --------------------- |
| Host    | `bigblack` | `FLOWSURFACE_CH_HOST` |
| Port    | `8123`     | `FLOWSURFACE_CH_PORT` |
| Timeout | 30 seconds | —                     |

In practice, `FLOWSURFACE_CH_HOST=localhost` and `FLOWSURFACE_CH_PORT=18123` via `.mise.toml`, with SSH tunnel forwarding to bigblack.

### Ouroboros Mode

SQL queries filter by `ouroboros_mode` — configured via `FLOWSURFACE_OUROBOROS_MODE` env var (default: `day`). Day-mode creates UTC-midnight-bounded sessions. Set to `month` for legacy data.

Implemented as `OUROBOROS_MODE: LazyLock<String>` static — read once at first access.

### Data Flow (HTTP)

```
fetch_klines() / fetch_klines_with_microstructure()
  → build_odb_sql()          Build SELECT with DESC ORDER + LIMIT
  → query()                  HTTP POST to ClickHouse
  → parse ChKline (NDJSON)   serde_json per-line
  → klines.reverse()         DESC → ASC (oldest first)
  → Vec<Kline>               + Optional Vec<ChMicrostructure>
```

### SQL Query

```sql
SELECT close_time_ms, open_time_ms, open, high, low, close, buy_volume, sell_volume,
       individual_trade_count, ofi, trade_intensity
FROM opendeviationbar_cache.open_deviation_bars
WHERE symbol = '{symbol}' AND threshold_decimal_bps = {threshold}
  AND ouroboros_mode = '{mode}'
ORDER BY close_time_ms DESC
LIMIT {limit}
FORMAT JSONEachRow
```

**Adaptive limit**: Scaled inversely with threshold. BPR25 (250 dbps) → 20K bars, floor 13K for all thresholds.

### Streaming (ClickHouse Polling)

`connect_kline_stream()` polls ClickHouse every 5 seconds for new bars with `close_time_ms > last_ts`. Uses ASC ordering for incremental updates.

### SSE Stream (Live Bars)

`connect_sse_stream()` receives live bar events from opendeviationbar-py's SSE sidecar. Controlled by `FLOWSURFACE_SSE_ENABLED`, `FLOWSURFACE_SSE_HOST`, `FLOWSURFACE_SSE_PORT`.

**Orphan bar filter**: Bars with `is_orphan == Some(true)` (incomplete UTC-midnight-boundary bars) are skipped with an INFO log. Defense-in-depth — the `is_orphan` column was removed from the backfill pipeline in opendeviationbar-py v12.56.1.

### Key Types

| Type               | Purpose                                          |
| ------------------ | ------------------------------------------------ |
| `ChKline`          | Serde struct for ClickHouse JSON row             |
| `ChMicrostructure` | Sidecar: `trade_count`, `ofi`, `trade_intensity` |

### Microstructure Fields

Three fields from opendeviationbar-py's microstructure features are surfaced as indicators:

| Field                    | Type          | Used By        |
| ------------------------ | ------------- | -------------- |
| `individual_trade_count` | `Option<u32>` | TradeCount     |
| `ofi`                    | `Option<f64>` | OFI            |
| `trade_intensity`        | `Option<f64>` | TradeIntensity |

### ODB Sidecar HTTP Endpoints (Gap-Fill)

Two HTTP endpoints on the same `SSE_HOST:SSE_PORT` sidecar, used for trade continuity gap-fill:

| Endpoint                                           | Purpose                                                | Response                                                      |
| -------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------- |
| `GET /ariadne/{symbol}/{threshold}`                | Last processed `agg_trade_id` (5-source cascade)       | `{"last_agg_trade_id": N, "source": "...", "degraded": bool}` |
| `GET /trades/gap-fill?symbol=&from_agg_id=&limit=` | Binance-compatible gap-fill trades (Parquet fast path) | `[{"a":..,"p":"..","q":"..","T":..,"m":..}, ...]`             |

**Rate limit**: Gap-fill has 1s cooldown (429 → retry after 1.1s). Pagination: max 1000/batch, 100 batches.

**Key types**: `GapFillTrade` (private, Binance-compatible deser struct in `clickhouse.rs`).

---

## Core Types

### Kline

```rust
pub struct Kline {
    pub time: u64,           // close timestamp (milliseconds UTC)
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: (f32, f32),  // (buy_volume, sell_volume)
}
```

Shared across ALL exchanges and chart types (Time, Tick, ODB).

### TickerInfo

```rust
pub struct TickerInfo {
    pub ticker: Ticker,       // Exchange + symbol + market type
    pub min_ticksize: Power10,
    pub min_qty: Power10,
    pub contract_size: Option<f64>,
}
```

---

## Adding a New Exchange

1. Create `src/adapter/{exchange}.rs`
2. Implement WebSocket connection + message parsing
3. Add `Exchange` variant in `src/lib.rs`
4. Add stream routing in `src/adapter.rs`
5. Handle in UI: `src/modal/pane/stream.rs` (exchange selector)

---

## Related

- [/CLAUDE.md](/CLAUDE.md) — Project hub
- [/data/CLAUDE.md](/data/CLAUDE.md) — Data aggregation, indicators
