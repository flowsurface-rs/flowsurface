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
    src="assets/screenshot.png"
    alt="Flowsurface screenshot"
    style="max-width: 100%; height: auto;"
  />
</div>

### Key Features

- Multiple chart/pane types:
    - **Time & Sales:** Scrollable list of live trades.
    - **DOM (Depth of Market) / Ladder:** Displays current L2 orderbook alongside recent trade volumes on grouped price levels.
    - **Heatmap (Historical DOM):** Uses live trades and orderbook to display a time-series heatmap chart. Supports customizable price grouping, different time aggregations, fixed or visible range volume profile.
    - **Candlestick:** Includes footprints, profiles, indicators with time-based or custom tick-based intervals.
        - **Order flow:** Footprints with bid/ask, volume, delta, and table views; configurable price grouping, imbalance highlighting, POC and naked-POC levels, and large-trade markers.
        - **Profiles:** Visible-range and recurring volume profiles, plus daily, weekly, or monthly TPO profiles with value areas, initial balance, single prints, and interactive splitting and merging.
        - **Scriptable indicators:** Use bundled indicators, install more from the catalog, or write your own with the `.ind` DSL. See [Indicators](#indicators) to get started.
- Real-time sound effects driven by trade streams
- [\*](#platform-notes)Multi window/monitor support on macOS and Linux.
- Pane linking for quickly switching tickers across multiple panes.
- Persistent layouts and customizable themes with editable color palettes.

#### Historical Trades on Footprint Charts:

By default, Flowsurface captures and plots live trades in real time via WebSocket.

To backfill the visible time range on footprint charts, enable **trade fetching**
in Settings → Network, apply the settings, then restart Flowsurface. Three modes are available:

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

        Use Unix milliseconds for `ts`, `from`, and `to`. Quantities are in base
        units for spot/linear markets and quote notional for inverse markets.
        Return the oldest matching trades first, in ascending timestamp order.

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

## Indicators

Candlestick/footprint charts include [bundled indicators](data/indicators/README.md).
Add more from the optional [catalog](https://github.com/flowsurface-rs/indicators/tree/main/sources) or write your own `.ind` files;
installing indicators requires an app restart.

Open the chart pane's **Indicators** sidebar and click **+** to browse the catalog.
Click **Install**, then restart the app and add the indicator through the same sidebar.
Downloads are verified and compiled before installation.

For manual installation, go to **Settings → Open data folder** and copy `.ind`
files into the `indicators/` subfolder, then restart. The data directory is `$FLOWSURFACE_DATA_PATH`
when set, otherwise your platform's data directory under `flowsurface/`.
See [installation details](https://github.com/flowsurface-rs/indicators#install-and-run) for path overrides
and loading behavior.

From an application source checkout, you can load a separate catalog checkout:

```sh
FLOWSURFACE_INDICATORS_PATH="/path/to/indicators/sources" cargo run
```

To create an indicator, see the [language reference and examples](indicator-engine/LANGUAGE.md)
and the [CLI workflow](indicator-engine/README.md#cli) for scaffolding, checking,
and testing sources.

## Installation

### Method 1: Prebuilt Binaries

Downloads are available on the [Releases page](https://github.com/flowsurface-rs/flowsurface/releases).

- **macOS**: Open the universal DMG and drag Flowsurface into Applications. Supports Apple Silicon and Intel Macs running macOS 11 or newer.
- **Windows**: Run the x64 MSIX to install for your current user, then launch Flowsurface from the Start menu. No administrator approval is required.
- **Linux**: Extract the archive and run `./install.sh` without sudo, then launch Flowsurface from the application menu. Run `./install.sh` from a newer archive to upgrade, or `./uninstall.sh` to remove the app. Settings and credentials are preserved. For portable use, run `./bin/flowsurface` directly.

<details><summary><strong>Having trouble running the file? (Permission/Security warnings)</strong></summary>
 
Windows executables and MSIX installers are code-signed; macOS apps have no Apple Developer ID signature or notarization. Security warnings may still appear.

- **Windows**: If App Installer cannot verify the publisher or signing certificate, [report the installation error](https://github.com/flowsurface-rs/flowsurface/issues).
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

### Platform Notes

<details open>
<summary><strong>Windows</strong></summary>

> ⚠️ Multi-window support is disabled on Windows due to a [redraw issue](https://github.com/rust-windowing/winit/issues/3648) affecting iced’s current winit backend.

</details>

## Community

Discord: https://discord.gg/RN2XAF7ZuR

## Acknowledgments

- Free code signing for Windows releases is provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/)
- [Kraken Desktop](https://www.kraken.com/desktop) (formerly [Cryptowatch](https://blog.kraken.com/product/cryptowatch-to-sunset-kraken-pro-to-integrate-cryptowatch-features)), the main inspiration that sparked this project
- [Halloy](https://github.com/squidowl/halloy), an excellent open-source IRC client, the main reference for the foundational code design
- And of course, [iced](https://github.com/iced-rs/iced), the GUI library that makes this possible

## License

Flowsurface is released under [GPL-3.0-or-later](LICENSE).
Some components use separate licenses:

| Component                                                                                        | License                                                                      |
| ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Application, data crate, and [bundled indicators](data/indicators/README.md)                     | [GPL-3.0-or-later](LICENSE)                                                  |
| Indicator engine and CLI                                                                         | [Apache-2.0](indicator-engine/LICENSE)                                       |
| [Optional indicator catalog](https://github.com/flowsurface-rs/indicators) and its documentation | [Apache-2.0](https://github.com/flowsurface-rs/indicators/blob/main/LICENSE) |
| Exchange adapters (`flowsurface-exchange`)                                                       | [MIT](exchange/LICENSE)                                                      |

Contributions follow the license of the component being changed; submit catalog indicators to the [indicators repository](https://github.com/flowsurface-rs/indicators/blob/main/CONTRIBUTING.md).
