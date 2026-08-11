# Flowsurface

[![Crates.io](https://img.shields.io/crates/v/flowsurface)](https://crates.io/crates/flowsurface)
[![Lint](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml)
[![Format](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?&logo=discord&logoColor=white)](https://discord.gg/RN2XAF7ZuR)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/flowsurface-rs/flowsurface/blob/main/LICENSE)
[![Made with iced](https://iced.rs/badge.svg)](https://github.com/iced-rs/iced)

An open-source native desktop charting application for crypto markets. Supports Binance, Bybit, Hyperliquid, OKX, and MEXC.

<div align="center">
  <img
    src="https://github.com/user-attachments/assets/baddc444-e079-48e5-82b2-4f97094eba07"
    alt="Flowsurface screenshot"
    style="max-width: 100%; height: auto;"
  />
</div>

### Key Features

- Multiple chart/panel types:
    - **Heatmap (Historical DOM):** Uses live trades and L2 orderbook to create a time-series heatmap chart. Supports customizable price grouping, different time aggregations, fixed or visible range volume profile.
    - **Candlestick:** Traditional kline chart supporting both time-based and custom tick-based intervals.
    - **Footprint:** Price grouped and interval aggregated views for trades on top of a candlestick chart. Supports different clustering methods, configurable imbalance and naked-POC studies.
    - **Time & Sales:** Scrollable list of live trades.
    - **DOM (Depth of Market) / Ladder:** Displays current L2 orderbook alongside recent trade volumes on grouped price levels.
    - **Comparison:** Line graph for comparing multiple data sources, normalized by kline `close` prices on a percentage scale
- Real-time sound effects driven by trade streams
- Multi window/monitor support
- Pane linking for quickly switching tickers across multiple panes
- Persistent layouts and customizable themes with editable color palettes

#

#### Historical Trades on Footprint Charts:

By default, Flowsurface captures and plots live trades in real time via WebSocket.

To backfill the visible time range on footprint charts, enable **trade fetching**
in Settings → Network. Three modes are available:

- **Exchange**: Fetches directly from Binance's public REST API and bulk data
  mirrors ([data.binance.vision](https://data.binance.vision/)). Fast for
  daily archives; paginated and rate-limited for intraday ranges. **Binance
  only** (spot, linear, inverse).

- **[Server](https://github.com/flowsurface-rs/server)**: Fetches from any HTTP server that exposes a
  `GET /trades.arrow` endpoint returning an
  [Arrow IPC](https://arrow.apache.org/) stream with:
    - Response: `Content-Type: application/vnd.apache.arrow.stream`
    - Schema (columns matched by name, order not significant):

        `ts (int64)`, `price (float64)`, `qty (float64)`, `is_sell (bool)`

    - Query parameters:
      `venue`, `market`, `symbol`, `from` (inclusive), `to` (inclusive), `limit`
    - Optional bearer-token auth: (`Authorization: Bearer <token>`)

    Self-signed TLS certificates are
    accepted. Works with **all exchanges** and has no exchange rate limits.
    Bring your own data source.

    A reference implementation is available at
    [flowsurface-server](https://github.com/flowsurface-rs/server), a
    self-contained collector that writes trades to DuckDB and serves
    them over this protocol.

- **Off**: Live trades only; no historical backfill.

##### Historical klines, open interest, and ticker metadata/stats are always fetched from exchange REST APIs.

##### Live trades, orderbook and kline updates are streamed from exchange WebSocket feeds.

## Installation

### Method 1: Prebuilt Binaries

Standalone executables are available for Windows, macOS, and Linux on the [Releases page](https://github.com/flowsurface-rs/flowsurface/releases).

<details><summary><strong>Having trouble running the file? (Permission/Security warnings)</strong></summary>
 
Since these binaries are currently unsigned they might get flagged.

- **Windows**: If you see a "Windows protected your PC" pop-up, click **More info** -> **Run anyway**.
- **macOS**: If you see "Developer cannot be verified", control-click (right-click) the app and select **Open**, or go to _System Settings > Privacy & Security_ to allow it.</details>

### Method 2: Build from Source

#### Requirements

- [Rust toolchain](https://www.rust-lang.org/tools/install)
- [Git version control system](https://git-scm.com/)
- System dependencies:
    - **Linux**:
        - Debian/Ubuntu: `sudo apt install build-essential pkg-config libasound2-dev`
        - Arch: `sudo pacman -S base-devel alsa-lib`
        - Fedora: `sudo dnf install gcc make alsa-lib-devel`
    - **macOS**: Install Xcode Command Line Tools: `xcode-select --install`
    - **Windows**: No additional dependencies required

#### Option A: `cargo install`

```bash
# Install latest globally
cargo install --git https://github.com/flowsurface-rs/flowsurface flowsurface

# Run
flowsurface
```

#### Option B: Cloning the repo

```bash
# Clone the repository
git clone https://github.com/flowsurface-rs/flowsurface

cd flowsurface

# Build and run
cargo build --release
cargo run --release
```

#### macOS DMG packaging

To build a drag-to-Applications DMG:

```bash
# Install packaging tools once
cargo install cargo-bundle
brew install create-dmg

# Apple Silicon
bash scripts/package-dmg.sh aarch64

# Intel
bash scripts/package-dmg.sh x86_64
```

The DMG is written to `target/macos-dist/`. Install it by opening the DMG,
dragging `Flowsurface.app` to `Applications`, and launching it from `/Applications`.

For public release builds, provide Apple Developer credentials/signing identities before
running the packaging script:

```bash
export SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export DMG_SIGNING_IDENTITY="Developer ID Installer: Your Name (TEAMID)"
export NOTARY_PROFILE="flowsurface-notary"

xcrun notarytool store-credentials "$NOTARY_PROFILE" \
  --apple-id "you@example.com" \
  --team-id "TEAMID" \
  --password "xxxx-xxxx-xxxx-xxxx"

bash scripts/package-dmg.sh aarch64
```

Without these variables, the script produces a local testing DMG with ad-hoc signing and no notarization.

## Credits and thanks to

- [Kraken Desktop](https://www.kraken.com/desktop) (formerly [Cryptowatch](https://blog.kraken.com/product/cryptowatch-to-sunset-kraken-pro-to-integrate-cryptowatch-features)), the main inspiration that sparked this project
- [Halloy](https://github.com/squidowl/halloy), an excellent open-source reference for the foundational code design and the project architecture
- And of course, [iced](https://github.com/iced-rs/iced), the GUI library that makes all of this possible

## Community

Discord: https://discord.gg/RN2XAF7ZuR

## License

Flowsurface is released under the [GPLv3](./LICENSE) license. Contributions to the project are shared under the same license.
