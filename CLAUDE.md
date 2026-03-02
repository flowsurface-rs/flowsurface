# CLAUDE.md - Project Memory

**flowsurface**: Native desktop charting app for crypto markets. Rust + iced 0.14 + WGPU. This fork adds **range bar visualization** from precomputed [opendeviationbar-py](https://github.com/terrylica/opendeviationbar-py) cache via ClickHouse.

**Upstream**: [flowsurface-rs/flowsurface](https://github.com/flowsurface-rs/flowsurface) | **Fork**: [terrylica/flowsurface](https://github.com/terrylica/flowsurface)

---

## Quick Reference

| Task                 | Command / Entry Point                    | Details                                 |
| -------------------- | ---------------------------------------- | --------------------------------------- |
| Build & run          | `mise run run`                           | Preflight + cargo run                   |
| Launch .app bundle   | `mise run run:app`                       | Preflight + open Flowsurface.app        |
| ClickHouse preflight | `mise run preflight`                     | Tunnel + connectivity + data validation |
| SSH tunnel           | `mise run tunnel:start`                  | localhost:18123 → bigblack:8123         |
| Release build        | `mise run build:release`                 | Optimized binary                        |
| .app bundle          | `mise run release:app-bundle`            | Build + update .app + register icon     |
| Lint                 | `mise run lint`                          | fmt:check + clippy                      |
| Sync upstream        | `mise run upstream:diff`                 | Show new upstream commits               |
| Exchange adapter     | [exchange/CLAUDE.md](exchange/CLAUDE.md) | ClickHouse, Binance, Bybit, etc.        |
| Data layer           | [data/CLAUDE.md](data/CLAUDE.md)         | Chart types, indicators, aggregation    |

---

## Architecture

```
flowsurface/                 Main crate — GUI, chart rendering, event handling
├── exchange/                Exchange adapters, WebSocket/REST streams
│   └── adapter/
│       ├── clickhouse.rs    Range bar adapter (HTTP, reads opendeviationbar-py cache)
│       ├── binance.rs       Binance Spot + Perpetuals
│       ├── bybit.rs         Bybit Perpetuals
│       ├── hyperliquid.rs   Hyperliquid DEX
│       └── okex.rs          OKX Multi-product
├── data/                    Data aggregation, indicators, layout models
│   ├── chart.rs             Basis enum (Time, Tick, RangeBar)
│   ├── chart/indicator.rs   KlineIndicator enum (6 types)
│   └── aggr/ticks.rs        TickAggr, RangeBarMicrostructure
└── src/                     GUI application
    ├── chart/kline.rs       Chart rendering (candles, range bars, footprint)
    ├── chart/indicator/     Indicator renderers (volume, delta, OFI, etc.)
    ├── screen/dashboard/    Pane grid UI
    └── modal/               Settings & configuration modals
```

---

## Range Bar Integration (Fork-Specific)

Range bar panes use **dual-stream architecture**: historical/completed bars from ClickHouse + live trades from Binance WebSocket.

```
Stream 1: ClickHouse (completed bars, 5s poll)
  → fetch_klines() → ChKline → Kline → TickAggr
  → update_latest_kline() → replace_or_append_kline()

Stream 2: Binance @aggTrade WebSocket (live trades)
  → DepthAndTrades → insert_trades_buffer()
  → TickAggr::insert_trades() → is_full_range_bar(threshold_dbps)
  → Forming bar oscillates until threshold breach → bar completes

Reconciliation: ClickHouse bar replaces locally-built bar (authoritative)
```

**Three-layer data pipeline** (opendeviationbar-py owns all layers, flowsurface polls ClickHouse):

| Layer         | Source                  | Latency | Status  | Tracking                                                                             |
| ------------- | ----------------------- | ------- | ------- | ------------------------------------------------------------------------------------ |
| L1 Historical | Binance Vision archives | Batch   | Working | —                                                                                    |
| L2 Recent     | REST API backfill       | Minutes | Planned | [opendeviationbar-py#92](https://github.com/terrylica/opendeviationbar-py/issues/92) |
| L3 Live       | WebSocket `@aggTrade`   | ~5s     | Planned | [opendeviationbar-py#91](https://github.com/terrylica/opendeviationbar-py/issues/91) |

flowsurface polls `connect_kline_stream()` every 5s. All layers merge into `opendeviationbar_cache.open_deviation_bars`. See also [opendeviationbar-py#93](https://github.com/terrylica/opendeviationbar-py/issues/93) for crash recovery.

**Key types**:

| Type                       | Location                             | Purpose                                    |
| -------------------------- | ------------------------------------ | ------------------------------------------ |
| `Basis::RangeBar(u32)`     | `data/src/chart.rs`                  | Chart basis (threshold in dbps)            |
| `KlineChartKind::RangeBar` | `data/src/chart/kline.rs`            | Chart type variant                         |
| `RangeBarMicrostructure`   | `data/src/aggr/ticks.rs`             | Sidecar: trade_count, ofi, trade_intensity |
| `ChKline`                  | `exchange/src/adapter/clickhouse.rs` | ClickHouse row deserialization             |
| `RANGE_BAR_THRESHOLDS`     | `data/src/chart.rs`                  | `[250, 500, 750, 1000]` dbps               |

**Threshold display**: `BPR{dbps/10}` — BPR25 = 250 dbps = 0.25%, BPR50 = 500 dbps, etc.

**Adaptive rendering**: `cell_width` and SQL `LIMIT` scale with threshold so all BPR levels show similar visual density.

---

## ClickHouse Infrastructure

**Architecture**: All range bar data served from **bigblack** via SSH tunnel. No local ClickHouse.

| Setting               | Value                   | Source       |
| --------------------- | ----------------------- | ------------ |
| `FLOWSURFACE_CH_HOST` | `localhost`             | `.mise.toml` |
| `FLOWSURFACE_CH_PORT` | `18123`                 | `.mise.toml` |
| SSH tunnel            | `18123 → bigblack:8123` | `infra.toml` |

**Preflight** (`mise run preflight`): Runs before every `mise run run` and `mise run run:app`:

1. Establishes SSH tunnel (idempotent)
2. Verifies ClickHouse responds (3 retries)
3. Verifies `opendeviationbar_cache.open_deviation_bars` table exists
4. Verifies BTCUSDT data present for all thresholds

**Data source**: [opendeviationbar-py](https://github.com/terrylica/opendeviationbar-py) populates the cache on bigblack. See opendeviationbar-py's `populate_cache_resumable()`.

---

## Mise Tasks

### Dev (`.mise/tasks/dev.toml`)

| Task            | Description                   | Depends On  |
| --------------- | ----------------------------- | ----------- |
| `build`         | Debug binary                  | —           |
| `build:release` | Optimized release binary      | —           |
| `run`           | Build + run with ClickHouse   | `preflight` |
| `run:app`       | Launch Flowsurface.app bundle | `preflight` |
| `check`         | Type-check (no codegen)       | —           |
| `clippy`        | Lint with `-D warnings`       | —           |
| `fmt`           | Format all Rust code          | —           |
| `lint`          | `fmt:check` + `clippy`        | —           |

### Release (`.mise/tasks/release.toml`)

| Task                  | Description                                  |
| --------------------- | -------------------------------------------- |
| `release:macos`       | Universal binary (x86_64 + aarch64 via lipo) |
| `release:macos-arm64` | aarch64-only release                         |
| `release:app-bundle`  | Build + update .app + SSH launcher + icon    |
| `icon:generate`       | Generate 1024x1024 app icon PNG              |
| `icon:convert`        | PNG → macOS .icns                            |

### Infrastructure (`.mise/tasks/infra.toml`)

| Task            | Description                                   |
| --------------- | --------------------------------------------- |
| `tunnel:start`  | SSH tunnel to bigblack (idempotent)           |
| `tunnel:stop`   | Kill SSH tunnel                               |
| `tunnel:status` | Verify tunnel + ClickHouse connectivity       |
| `preflight`     | Full validation (tunnel + CH + schema + data) |

### Upstream (`.mise/tasks/upstream.toml`)

| Task              | Description               |
| ----------------- | ------------------------- |
| `upstream:fetch`  | Fetch upstream changes    |
| `upstream:diff`   | Show new upstream commits |
| `upstream:merge`  | Merge upstream/main       |
| `upstream:rebase` | Rebase onto upstream/main |

---

## Common Patterns

### Adding a New Indicator

1. Add variant to `KlineIndicator` enum in `data/src/chart/indicator.rs`
2. Add to `FOR_SPOT` and/or `FOR_PERPS` arrays
3. Add `Display` impl
4. Create indicator file in `src/chart/indicator/kline/`
5. Implement `KlineIndicatorImpl` trait
6. Register in factory `src/chart/indicator/kline.rs`

### Adding a New Exchange Adapter

1. Create adapter file in `exchange/src/adapter/`
2. Add `Exchange` variant in `exchange/src/lib.rs`
3. Implement stream routing in `exchange/src/adapter.rs`
4. Add market type handling

### Extending Range Bar Support

When modifying range bar rendering, check **all** match arms for `Basis::RangeBar(_)`, `KlineChartKind::RangeBar`, and `ContentKind::RangeBarChart` across:

- `src/chart/kline.rs` (rendering)
- `src/screen/dashboard/pane.rs` (pane state)
- `src/screen/dashboard.rs` (pane switching)
- `src/modal/pane/stream.rs` (settings UI)
- `src/modal/pane/settings.rs` (chart config)
- `data/src/layout/pane.rs` (serialization)

---

## Common Errors

| Error                     | Cause                  | Fix                                                 |
| ------------------------- | ---------------------- | --------------------------------------------------- |
| "Fetching Klines..." loop | ClickHouse unreachable | `mise run preflight`                                |
| Tiny dot candlesticks     | Wrong cell_width/limit | Check adaptive scaling in `kline.rs`                |
| Crosshair panic           | NaN in indicator data  | Add NaN guard before rendering                      |
| "ClickHouse HTTP 404"     | Wrong table/schema     | Verify `opendeviationbar_cache.open_deviation_bars` |
| Missing BPR threshold     | No data for that dbps  | Check `mise run tunnel:status`                      |

---

## CLAUDE.md Network (Hub-and-Spoke)

| Directory    | CLAUDE.md                                | Purpose                                |
| ------------ | ---------------------------------------- | -------------------------------------- |
| `/`          | This file                                | Hub, quick reference                   |
| `/exchange/` | [exchange/CLAUDE.md](exchange/CLAUDE.md) | Exchange adapters, ClickHouse, streams |
| `/data/`     | [data/CLAUDE.md](data/CLAUDE.md)         | Chart types, indicators, aggregation   |

---

## Terminology

| Term           | Definition                                                                |
| -------------- | ------------------------------------------------------------------------- |
| **dbps**       | Decimal basis points. 1 dbps = 0.001%. 250 dbps = 0.25%.                  |
| **BPR**        | Basis Points Range. Display label: BPR25 = 250 dbps threshold.            |
| **OFI**        | Order Flow Imbalance. `(buy_vol - sell_vol) / total_vol`. Range: [-1, 1]. |
| **TickAggr**   | Vec-based aggregation (oldest-first). Used for Tick and RangeBar basis.   |
| **TimeSeries** | Time-based aggregation. Used for Time basis (1m, 5m, 1h, etc.).           |
