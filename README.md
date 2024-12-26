<div align="center">
  <img height="247" width="400" alt="iced-trade" src="https://github.com/user-attachments/assets/79bd0f07-d97c-4186-921f-2e726dcb2c00">
  <img height="247" width="400" alt="iced-trade" src="https://github.com/user-attachments/assets/c862ba41-71f9-411d-bfe4-97f716c36b56">
</div>

### Some of the features:

- Customizable and savable grid layouts, selectable themes
- Supports most of spot(USDT) & linear perp pairs from Binance & Bybit
- Orderbook total bid/ask levels: 1000 for Binance Perp/Spot; 500 for Bybit Perps, 200 for Bybit Spot
- Binance perp/spot & Bybit perp streams @100ms; Bybit spot pairs streams @200ms
- Tick size multipliers for price grouping on footprint and heatmap charts

<div align="center">
  <img height="200" width="300" alt="iced-trade" src="https://github.com/user-attachments/assets/89894672-4ad6-41a2-ab7f-84c5acdb76a9">
  <img height="235" width="200" alt="iced-trade" src="https://github.com/user-attachments/assets/a93ff39f-e80a-4f87-a99b-d4582f4bb818">
</div>

##### User receives market data directly from exchange APIs.

- As historical data, it can fetch OHLCV, open interest and partially, trades:

#### Historical trades on footprint chart:

Optionally, you can enable trade fetching from settings menu, experimental because of unreliability:

- Binance connector supports downloading historical trades from [data.binance.vision](https://data.binance.vision), fast and easy way to get trades, but they dont support intraday data.
Intraday trades fetched by pagination using Binance's public REST APIs: `/fapi/v1/aggTrades` & `api/v3/aggTrades`, it might be slow because of rate-limits

- Bybit itself doesnt have a similar purpose public API

Flowsurface tries to leverage this via Binance tickers, to visualize historical public trades while being independent of a 'middleman' database between exchange and the user.
So, when a chart instance signal the exchange connector after a data integrity check, about missing trades in the visible range; it tries via fetching, downloading and/or loading from cache, whichever suitable, to ensure this integrity

## Build from source

The releases might not be up-to-date with newest features.<sup>or bugs :)</sup>

- For that you could
  clone the repository into a directory of your choice and build with cargo.

Requirements:

- [Rust toolchain](https://www.rust-lang.org/tools/install)
- [Git version control system](https://git-scm.com/)

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