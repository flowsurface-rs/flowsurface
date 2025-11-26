# Flowsurface

An experimental open-source desktop charting application. Supports Binance, Bybit, Hyperliquid and OKX

<div align="center">
 <img width="2330" height="1440" alt="flowsurface_v0-8-6" src="https://github.com/user-attachments/assets/baddc444-e079-48e5-82b2-4f97094eba07" />
</div>

### Key Features

-   Multiple chart/panel types:
    -   **Heatmap (Historical DOM):** Uses live trades and L2 orderbook to create a time-series heatmap chart. Supports customizable price grouping, different time aggregations, fixed or visible range volume profiles.
    -   **Candlestick:** Traditional kline chart supporting both time-based and custom tick-based intervals.
    -   **Footprint:** Price grouped and interval aggregated views for trades on top of a candlestick chart. Supports different clustering methods, configurable imbalance and naked-POC studies.
    -   **Time & Sales:** Scrollable list of live trades.
    -   **DOM (Depth of Market) / Ladder:** Displays current L2 orderbook alongside recent trade volumes on grouped price levels.
    -   **Comparison:** Line graph for comparing multiple data sources, normalized by kline `close` prices on a percentage scale
-   Real-time sound effects driven by trade streams
-   Pane linking/grouping for quickly switching tickers across multiple panes
-   Persistent layouts and customizable themes with editable color palettes

##### Market data is received directly from exchanges' public REST APIs and WebSockets
#
#### Historical Trades on Footprint Charts:

-   By default, it captures and plots live trades in real time via WebSocket.
-   For Binance tickers, you can optionally backfill the visible time range by enabling trade fetching in the settings:
    -   [data.binance.vision](https://data.binance.vision/): Fast daily bulk downloads (no intraday).
    -   REST API (e.g., `/fapi/v1/aggTrades`): Slower, paginated intraday fetching (subject to rate limits).
    -   The Binance connector can use either or both methods to retrieve historical data as needed.
-   Trade fetching for Bybit/Hyperliquid is not supported, as both lack a suitable REST API. OKX is WIP.

## Installation

### Using Prebuilt Binaries

Prebuilt binaries for Windows, macOS, and Linux are available on the [Releases page](https://github.com/flowsurface-rs/flowsurface/releases)

### Build from Source

#### Requirements

-   [Rust toolchain](https://www.rust-lang.org/tools/install)
-   [Git version control system](https://git-scm.com/)
-   System dependencies:
    -   **Linux**:
        -   Debian/Ubuntu: `sudo apt install build-essential pkg-config libasound2-dev`
        -   Arch: `sudo pacman -S base-devel alsa-lib`
        -   Fedora: `sudo dnf install gcc make alsa-lib-devel`
    -   **macOS**: Install Xcode Command Line Tools: `xcode-select --install`
    -   **Windows**: No additional dependencies required

#### Build and Run

```bash
# Clone the repository
git clone https://github.com/flowsurface-rs/flowsurface

cd flowsurface

# Build and run
cargo build --release
cargo run --release
```

<a href="https://iced.rs/">
  <img src="https://gist.githubusercontent.com/hecrj/ad7ecd38f6e47ff3688a38c79fd108f0/raw/74384875ecbad02ae2a926425e9bcafd0695bade/color.svg" width="130px">
</a>

### Credits
- [Kraken Desktop](https://www.kraken.com/desktop) (formerly [Cryptowatch](https://blog.kraken.com/product/cryptowatch-to-sunset-kraken-pro-to-integrate-cryptowatch-features)), the main inspiration that sparked this project
- [Halloy](https://github.com/squidowl/halloy), an excellent open-source reference for the foundational code design and the project architecture
- And of course, [iced](https://github.com/iced-rs/iced), the GUI library that makes all of this possible
