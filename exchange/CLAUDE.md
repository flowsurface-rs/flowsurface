# Exchange Crate

**Parent**: [/CLAUDE.md](/CLAUDE.md)

Exchange adapters, WebSocket/REST stream handling, and type definitions. Crate name: `flowsurface-exchange`.

---

## Quick Reference

| Adapter         | File             | Protocol                   | Markets                    |
| --------------- | ---------------- | -------------------------- | -------------------------- |
| **ClickHouse**  | `clickhouse.rs`  | HTTP (opendeviationbar-py) | Range bars from cache      |
| **Binance**     | `binance.rs`     | REST + WebSocket           | Spot, Linear/Inverse Perps |
| **Bybit**       | `bybit.rs`       | REST + WebSocket           | Perpetuals                 |
| **Hyperliquid** | `hyperliquid.rs` | REST + WebSocket           | DEX Perpetuals             |
| **OKX**         | `okex.rs`        | REST + WebSocket           | Multi-product              |

---

## ClickHouse Adapter (Fork-Specific)

**File**: `src/adapter/clickhouse.rs` (~570 LOC)

Reads precomputed range bars from opendeviationbar-py's ClickHouse cache via HTTP.

### Connection

| Setting | Default    | Override env var      |
| ------- | ---------- | --------------------- |
| Host    | `bigblack` | `FLOWSURFACE_CH_HOST` |
| Port    | `8123`     | `FLOWSURFACE_CH_PORT` |
| Timeout | 30 seconds | —                     |

In practice, `FLOWSURFACE_CH_HOST=localhost` and `FLOWSURFACE_CH_PORT=18123` via `.mise.toml`, with SSH tunnel forwarding to bigblack.

### Data Flow

```
fetch_klines() / fetch_klines_with_microstructure()
  → build_range_bar_sql()     Build SELECT with DESC ORDER + LIMIT
  → query()                   HTTP POST to ClickHouse
  → parse ChKline (NDJSON)    serde_json per-line
  → klines.reverse()          DESC → ASC (oldest first)
  → Vec<Kline>                + Optional Vec<ChMicrostructure>
```

### SQL Query

```sql
SELECT close_time_ms, open_time_ms, open, high, low, close, buy_volume, sell_volume,
       individual_trade_count, ofi, trade_intensity
FROM opendeviationbar_cache.open_deviation_bars
WHERE symbol = '{symbol}' AND threshold_decimal_bps = {threshold}
ORDER BY close_time_ms DESC
LIMIT {limit}
FORMAT JSONEachRow
```

**Adaptive limit**: Scaled inversely with threshold. BPR25 (250 dbps) → 500 bars, BPR50 → 250, BPR100 → 125. Keeps similar time window across thresholds.

### Streaming (Polling)

`connect_kline_stream()` polls ClickHouse every 5 seconds for new bars with `close_time_ms > last_ts`. Uses ASC ordering for incremental updates. The 5s interval is designed for near-real-time range bar updates once opendeviationbar-py's streaming sidecar (terrylica/opendeviationbar-py#91) writes live bars to ClickHouse.

### Key Types

| Type               | Purpose                                          |
| ------------------ | ------------------------------------------------ |
| `ChKline`          | Serde struct for ClickHouse JSON row             |
| `ChMicrostructure` | Sidecar: `trade_count`, `ofi`, `trade_intensity` |

### Microstructure Fields

Three fields from opendeviationbar-py's 10 microstructure features are surfaced as indicators:

| Field                    | Type          | Used By        |
| ------------------------ | ------------- | -------------- |
| `individual_trade_count` | `Option<u32>` | TradeCount     |
| `ofi`                    | `Option<f64>` | OFI            |
| `trade_intensity`        | `Option<f64>` | TradeIntensity |

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

Shared across ALL exchanges and chart types (Time, Tick, RangeBar).

### StreamKind

```rust
pub enum StreamKind {
    Kline { ticker_info, timeframe },
    TickKline { ticker_info, tick_count },
    RangeBarKline { ticker_info, threshold_dbps },  // Fork-specific
    Trade { ticker_info },
    DepthAndTrades { ticker_info },
    // ...
}
```

### TickerInfo

```rust
pub struct TickerInfo {
    pub ticker: Ticker,
    pub exchange: Exchange,
    pub market_type: MarketKind,  // Spot, LinearPerps, InversePerps
    pub min_ticksize: f32,
    // ...
}
```

### MarketKind

```rust
pub enum MarketKind { Spot, LinearPerps, InversePerps }
```

---

## Stream Routing

All exchange streams are routed through `adapter.rs`:

```
StreamKind::Kline         → binance/bybit/okex/hyperliquid WebSocket
StreamKind::TickKline     → binance/bybit WebSocket (trade aggregation)
StreamKind::RangeBarKline → clickhouse::connect_kline_stream() (HTTP polling)
```

The ClickHouse adapter is unique — it uses HTTP polling (60s interval) instead of WebSocket, because range bars are precomputed batch data, not real-time.

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
- `src/adapter/clickhouse.rs` — ClickHouse adapter source
