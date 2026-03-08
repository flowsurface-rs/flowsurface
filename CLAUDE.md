# CLAUDE.md - Project Hub

**flowsurface**: Native desktop charting app for crypto markets. Rust + iced 0.14 + WGPU. This fork adds **ODB (Open Deviation Bar) visualization** from precomputed [opendeviationbar-py](https://github.com/terrylica/opendeviationbar-py) cache via ClickHouse.

**Upstream**: [flowsurface-rs/flowsurface](https://github.com/flowsurface-rs/flowsurface) | **Fork**: [terrylica/flowsurface](https://github.com/terrylica/flowsurface)

---

## Quick Reference

| Task                 | Command / Entry Point         | Details                                 |
| -------------------- | ----------------------------- | --------------------------------------- |
| Build & run          | `mise run run`                | Preflight + cargo run                   |
| Launch .app bundle   | `mise run run:app`            | Preflight + open Flowsurface.app        |
| ClickHouse preflight | `mise run preflight`          | Tunnel + connectivity + data validation |
| SSH tunnel           | `mise run tunnel:start`       | localhost:18123 → bigblack:8123         |
| Release build        | `mise run build:release`      | Optimized binary                        |
| .app bundle          | `mise run release:app-bundle` | Build + update .app + register icon     |
| Lint                 | `mise run lint`               | fmt:check + clippy                      |
| Sync upstream        | `mise run upstream:diff`      | Show new upstream commits               |

---

## CLAUDE.md Network (Hub-and-Spoke)

| Directory    | CLAUDE.md                                | Scope                                            |
| ------------ | ---------------------------------------- | ------------------------------------------------ |
| `/`          | This file                                | Hub — architecture, env vars, patterns, errors   |
| `/exchange/` | [exchange/CLAUDE.md](exchange/CLAUDE.md) | Exchange adapters, ClickHouse, SSE, stream types |
| `/data/`     | [data/CLAUDE.md](data/CLAUDE.md)         | Chart types, indicators, aggregation, layout     |

---

## Architecture

```
flowsurface/                 Main crate — GUI, chart rendering, event handling
├── exchange/                Exchange adapters, WebSocket/REST/HTTP streams
│   └── adapter/
│       ├── clickhouse.rs    ODB adapter (HTTP + SSE, reads opendeviationbar-py cache)
│       ├── binance.rs       Binance Spot + Perpetuals
│       ├── bybit.rs         Bybit Perpetuals
│       ├── hyperliquid.rs   Hyperliquid DEX
│       └── okex.rs          OKX Multi-product
├── data/                    Data aggregation, indicators, layout models
│   ├── chart.rs             Basis enum (Time, Tick, Odb)
│   ├── chart/indicator.rs   KlineIndicator enum (6 types)
│   ├── aggr/ticks.rs        TickAggr, RangeBarMicrostructure
│   └── session.rs           Trading session boundaries (NY/London/Tokyo)
└── src/                     GUI application
    ├── chart/kline.rs       Chart rendering (candles, ODB bars, footprint)
    ├── chart/indicator/     Indicator renderers (volume, delta, OFI, etc.)
    ├── chart/session.rs     Session line rendering
    ├── connector/           Stream connection + data fetching
    │   ├── stream.rs        ResolvedStream, stream matching
    │   └── fetcher.rs       FetchedData, RequestHandler, batch fetching
    ├── screen/dashboard/    Pane grid UI + pane state
    ├── modal/               Settings & configuration modals
    └── widget/              BTC widget overlay
```

---

## Environment Variables

All set in `.mise.toml`. The app reads them at runtime via `std::env::var()`.

| Variable                     | Default     | Purpose                             |
| ---------------------------- | ----------- | ----------------------------------- |
| `FLOWSURFACE_CH_HOST`        | `bigblack`  | ClickHouse HTTP host                |
| `FLOWSURFACE_CH_PORT`        | `8123`      | ClickHouse HTTP port                |
| `FLOWSURFACE_SSE_ENABLED`    | `false`     | Enable SSE live bar stream          |
| `FLOWSURFACE_SSE_HOST`       | `localhost` | SSE sidecar host                    |
| `FLOWSURFACE_SSE_PORT`       | `8081`      | SSE sidecar port                    |
| `FLOWSURFACE_OUROBOROS_MODE` | `day`       | ODB session mode (`day` or `month`) |
| `FLOWSURFACE_ALWAYS_ON_TOP`  | _(unset)_   | Pin window above all others if set  |
| `FLOWSURFACE_TG_BOT_TOKEN`   | _(unset)_   | Telegram bot token for alerts       |
| `FLOWSURFACE_TG_CHAT_ID`     | _(unset)_   | Telegram chat ID for alerts         |

---

## ODB Integration (Fork-Specific)

ODB panes use **triple-stream architecture**:

```
Stream 1: OdbKline — ClickHouse (completed bars, 5s poll)
  → fetch_klines() → ChKline → Kline → TickAggr
  → update_latest_kline() → replace_or_append_kline()

Stream 2: Trades — Binance @aggTrade WebSocket (live trades)
  → TradesReceived → insert_trades_buffer()
  → TickAggr::insert_trades() → is_full_range_bar(threshold_dbps)
  → Forming bar oscillates until threshold breach → bar completes

Stream 3: Depth — Binance depth WebSocket (orderbook)
  → DepthReceived → heatmap / footprint data

Reconciliation: ClickHouse bar replaces locally-built bar (authoritative)

Stream 4: Gap-fill — ODB sidecar Ariadne + /trades/gap-fill
  → After initial CH klines load, query Ariadne for last_agg_trade_id
  → Fetch missing trades via /trades/gap-fill (Parquet fast path)
  → Dedup fence: WS trades with id <= fence are skipped
  → CH bars buffered during gap-fill, flushed after completion
```

**CRITICAL**: ODB panes must subscribe to ALL THREE streams (`OdbKline`, `Trades`, `Depth`) in `resolve_content()` at `src/screen/dashboard/pane.rs`. Missing `Trades` causes "Waiting for trades..." forever because `matches_stream()` silently drops unmatched events.

**Key types** (see [data/CLAUDE.md](data/CLAUDE.md) and [exchange/CLAUDE.md](exchange/CLAUDE.md) for details):

| Type                     | Location                             | Purpose                                    |
| ------------------------ | ------------------------------------ | ------------------------------------------ |
| `Basis::Odb(u32)`        | `data/src/chart.rs`                  | Chart basis (threshold in dbps)            |
| `KlineChartKind::Odb`    | `data/src/chart/kline.rs`            | Chart type variant                         |
| `RangeBarMicrostructure` | `data/src/aggr/ticks.rs`             | Sidecar: trade_count, ofi, trade_intensity |
| `ChKline`                | `exchange/src/adapter/clickhouse.rs` | ClickHouse row deserialization             |
| `ODB_THRESHOLDS`         | `data/src/chart.rs`                  | `[250, 500, 750, 1000]` dbps               |
| `ContentKind::OdbChart`  | `data/src/layout/pane.rs`            | Pane serialization variant                 |

**Threshold display**: `BPR{dbps/10}` — BPR25 = 250 dbps = 0.25%, BPR50 = 500 dbps, etc.

---

## ClickHouse Infrastructure

All range bar data served from **bigblack** via SSH tunnel. No local ClickHouse.

| Setting    | Value                   | Source       |
| ---------- | ----------------------- | ------------ |
| Host       | `localhost`             | `.mise.toml` |
| Port       | `18123`                 | `.mise.toml` |
| SSH tunnel | `18123 → bigblack:8123` | `infra.toml` |

**Preflight** (`mise run preflight`): Runs before `mise run run` and `mise run run:app`:

1. Establishes SSH tunnel (idempotent)
2. Verifies ClickHouse responds (3 retries)
3. Verifies `opendeviationbar_cache.open_deviation_bars` table exists
4. Verifies BTCUSDT data present for all thresholds

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
| `release:app-bundle`  | Build + update .app + sign + register icon   |
| `sign:app`            | Ad-hoc codesign the .app bundle              |

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

## Release Model

**Native desktop app** — no crates.io, no version tags, no changelog.

| Task                           | What It Does                                     |
| ------------------------------ | ------------------------------------------------ |
| `mise run build:release`       | Optimized binary at `target/release/flowsurface` |
| `mise run release:app-bundle`  | Build + update `.app` + SSH launcher + icon      |
| `mise run release:macos-arm64` | aarch64-only release binary                      |
| `mise run release:macos`       | Universal binary (x86_64 + aarch64 via lipo)     |

**Code signing**: Ad-hoc via `codesign --deep --force --sign -` (built into `run:app` and `release:app-bundle`).

---

## Common Patterns

### Adding a New Indicator

1. Add variant to `KlineIndicator` enum in `data/src/chart/indicator.rs`
2. Add to `FOR_SPOT` and/or `FOR_PERPS` arrays
3. Add `Display` impl
4. Create indicator file in `src/chart/indicator/kline/`
5. Implement `KlineIndicatorImpl` trait
6. Register in factory `src/chart/indicator/kline.rs`

### Extending ODB Support

When modifying ODB rendering or behavior, check **all** match arms for `Basis::Odb(_)`, `KlineChartKind::Odb`, and `ContentKind::OdbChart` across:

- `src/screen/dashboard/pane.rs` — pane streams (must include `OdbKline` + `Depth` + `Trades`)
- `src/screen/dashboard.rs` — event dispatch, pane switching
- `src/chart/kline.rs` — rendering, trade insertion
- `src/chart/heatmap.rs` — depth heatmap
- `src/modal/pane/stream.rs` — settings UI
- `src/modal/pane/settings.rs` — chart config
- `data/src/layout/pane.rs` — serialization

### Upstream Merge Checklist

After merging upstream, check for:

1. New `StreamKind` variants — add match arms in fork-specific code
2. Changes to `window::Settings` — preserve `level:` field in `main.rs`
3. Changes to `FetchedData` — preserve fork's `microstructure` field in `connector/fetcher.rs`
4. New `ContentKind` variants — add to pane setup in `dashboard/pane.rs`
5. Changes to `FetchRange` — preserve fork's `OdbCatchup` variant in `connector/fetcher.rs`
6. Changes to `Message` in `dashboard.rs` — preserve `TriggerOdbGapFill` variant
7. Changes to `Trade` struct — preserve `agg_trade_id` field in `exchange/src/lib.rs`

---

## Common Errors

| Error                       | Cause                               | Fix                                                                  |
| --------------------------- | ----------------------------------- | -------------------------------------------------------------------- |
| "Waiting for trades..."     | ODB pane missing `Trades` stream    | Add `trades_stream()` to pane's stream vec in `pane.rs`              |
| "Fetching Klines..." loop   | ClickHouse unreachable              | `mise run preflight`                                                 |
| "No chart found for stream" | Widget/pane stream mismatch         | Check `matches_stream()` in `connector/stream.rs`                    |
| Tiny dot candlesticks       | Wrong cell_width/limit              | Check adaptive scaling in `kline.rs`                                 |
| Crosshair panic             | NaN in indicator data               | Add NaN guard before rendering                                       |
| "ClickHouse HTTP 404"       | Wrong table/schema                  | Verify `opendeviationbar_cache.open_deviation_bars`                  |
| "no microstructure data"    | `FetchedData::Klines` missing field | Ensure `microstructure: Some(micro)` in ODB fetch path               |
| "Fetching trades..." stuck  | ODB sidecar unreachable (Ariadne)   | Verify sidecar at `http://{SSE_HOST}:{SSE_PORT}/ariadne/BTCUSDT/250` |
| Gap-fill silently skipped   | Ariadne returned `None` or error    | Check sidecar logs; gap-fill is best-effort                          |

---

## Terminology

| Term           | Definition                                                               |
| -------------- | ------------------------------------------------------------------------ |
| **dbps**       | Decimal basis points. 1 dbps = 0.001%. 250 dbps = 0.25%.                 |
| **BPR**        | Basis Points Range. Display label: BPR25 = 250 dbps threshold.           |
| **ODB**        | Open Deviation Bar. Range bar that closes on % deviation from open.      |
| **OFI**        | Order Flow Imbalance. `(buy_vol - sell_vol) / total_vol`. Range: [-1,1]. |
| **TickAggr**   | Vec-based aggregation (oldest-first). Used for Tick and ODB basis.       |
| **TimeSeries** | Time-based aggregation. Used for Time basis (1m, 5m, 1h, etc.).          |
| **SSE**        | Server-Sent Events. Live bar stream from opendeviationbar-py sidecar.    |
