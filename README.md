# Flowsurface

An experimental open-source desktop charting application. Supports Binance, Bybit, Hyperliquid and OKX

<div align="center">
  <img width="2330" height="1440" alt="overview-layout-1" src="https://github.com/user-attachments/assets/7875117e-2475-4549-ac8c-6d350dacdb75" />
</div>

### Key Features

-   Multiple chart/panel types:
    -   **Heatmap (Historical DOM):** Uses live trades and L2 orderbook to create a time-series heatmap chart. Supports customizable price grouping and selectable time intervals. Includes a configurable fixed or visible range volume profile.
    -   **Candlestick:** Traditional kline chart supporting both time-based and custom tick-based intervals.
    -   **Footprint:** Price-grouped and interval-aggregated views for trades on top of candlestick chart; supports different clustering methods. Includes configurable imbalance and naked-POC studies.
    -   **Time & Sales:** Scrollable list of live trades.
    -   **DOM (Depth of Market) / Ladder:** A real-time, interactive panel that displays current L2 orderbook alongside recent trade volumes on grouped price levels.
-   Real-time sound effects driven by trade streams
-   Pane linking and grouping for quickly switching tickers across multiple panes
-   Customizable and persistent layouts, themes, panel and chart settings

<div align="center">
  <img width="268" height="287" alt="expanded-ticker-card" src="https://github.com/user-attachments/assets/ab8776b1-7e81-4a2d-a9e7-42d3b238cf1a" />
  <img width="199" height="405" alt="layout-manager" src="https://github.com/user-attachments/assets/63b5cf07-c4bf-4199-90b4-f7530c60de63" />
</div>

##### Market data is received directly from exchanges' public REST APIs and WebSockets.

#### Historical Trades on Footprint Charts

-   By default, they capture and plot live trades in real time via WebSocket.
-   For Binance tickers, you can optionally backfill the visible time range by enabling trade fetching in the settings:
    -   [data.binance.vision](https://data.binance.vision/): Fast daily bulk downloads (no intraday).
    -   REST API (e.g., `/fapi/v1/aggTrades`): Slower, paginated intraday fetching (subject to rate limits).
    -   The Binance connector can use either or both methods to retrieve historical data as needed.
-   Trade fetching for Bybit/Hyperliquid is not supported, as both lack a suitable REST API. OKX is WIP.

---

## Installation

### Using Prebuilt Binaries

Prebuilt binaries for Windows, macOS, and Linux are available on the [Releases page](https://github.com/akenshaw/flowsurface/releases).

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
git clone https://github.com/akenshaw/flowsurface

cd flowsurface

# Build and run
cargo build --release
cargo run --release
```

<a href="https://github.com/iced-rs/iced">
  <img src="https://gist.githubusercontent.com/hecrj/ad7ecd38f6e47ff3688a38c79fd108f0/raw/74384875ecbad02ae2a926425e9bcafd0695bade/color.svg" width="130px">
</a>

---
