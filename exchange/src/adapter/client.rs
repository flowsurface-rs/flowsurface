use super::{
    AdapterError, Event, Exchange, MarketKind, StreamConfig, Venue,
    hub::{binance, bybit, hyperliquid, mexc, okex},
};
use crate::{Kline, OpenInterest, Ticker, TickerInfo, TickerStats, Timeframe, Trade, UnixMs};

use futures::{StreamExt, stream, stream::BoxStream};
use std::{collections::HashMap, collections::HashSet, path::PathBuf};

// Keep topics per websocket conservative across venues
// allow up to 100 tickers per websocket stream
pub const MAX_TRADE_TICKERS_PER_STREAM: usize = 100;
pub const MAX_KLINE_STREAMS_PER_STREAM: usize = 100;

#[derive(Debug, Clone, Default)]
pub struct AdapterNetworkConfig {
    pub proxy_cfg: Option<super::proxy::Proxy>,
}

#[derive(Clone, Default)]
pub struct AdapterHandles {
    binance: Option<binance::BinanceHandle>,
    bybit: Option<bybit::BybitHandle>,
    hyperliquid: Option<hyperliquid::HyperliquidHandle>,
    okex: Option<okex::OkexHandle>,
    mexc: Option<mexc::MexcHandle>,
}

impl AdapterHandles {
    pub fn spawn_all(config: AdapterNetworkConfig) -> Result<Self, AdapterError> {
        Self::spawn_selected(config, Venue::ALL)
    }

    pub fn spawn_selected(
        config: AdapterNetworkConfig,
        venues: impl IntoIterator<Item = Venue>,
    ) -> Result<Self, AdapterError> {
        let mut out = Self::default();
        let mut seen = HashSet::new();

        for venue in venues {
            if !seen.insert(venue) {
                continue;
            }

            out.spawn_venue(venue, config.clone())?;
        }

        Ok(out)
    }

    pub fn configured_venues(&self) -> impl Iterator<Item = Venue> + '_ {
        Venue::ALL
            .into_iter()
            .filter(|venue| self.has_venue(*venue))
    }

    pub fn has_venue(&self, venue: Venue) -> bool {
        match venue {
            Venue::Binance => self.binance.is_some(),
            Venue::Bybit => self.bybit.is_some(),
            Venue::Hyperliquid => self.hyperliquid.is_some(),
            Venue::Okex => self.okex.is_some(),
            Venue::Mexc => self.mexc.is_some(),
        }
    }

    fn spawn_venue(
        &mut self,
        venue: Venue,
        config: AdapterNetworkConfig,
    ) -> Result<(), AdapterError> {
        match venue {
            Venue::Binance => {
                self.binance = Some(binance::spawn_binance_with_network(config)?);
            }
            Venue::Bybit => {
                self.bybit = Some(bybit::spawn_bybit_with_network(config)?);
            }
            Venue::Hyperliquid => {
                self.hyperliquid = Some(hyperliquid::spawn_hyperliquid_with_network(config)?);
            }
            Venue::Okex => {
                self.okex = Some(okex::spawn_okex_with_network(config)?);
            }
            Venue::Mexc => {
                self.mexc = Some(mexc::spawn_mexc_with_network(config)?);
            }
        }

        Ok(())
    }

    fn missing_venue_stream(exchange: Exchange) -> BoxStream<'static, Event> {
        let reason = format!(
            "No adapter handle configured for venue {}",
            exchange.venue()
        );
        stream::once(async move { Event::Disconnected(exchange, reason) }).boxed()
    }

    fn missing_venue_error(venue: Venue) -> AdapterError {
        AdapterError::InvalidRequest(format!("No adapter handle configured for venue {venue}"))
    }

    pub fn kline_stream(
        &self,
        config: &StreamConfig<Vec<(TickerInfo, Timeframe)>>,
    ) -> BoxStream<'static, Event> {
        let streams = config.id.clone();
        let market_kind = config.exchange.market_type();

        match config.exchange.venue() {
            Venue::Binance => self.binance.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_kline_stream(streams, market_kind).boxed(),
            ),
            Venue::Bybit => self.bybit.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_kline_stream(streams, market_kind).boxed(),
            ),
            Venue::Hyperliquid => self.hyperliquid.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_kline_stream(streams, market_kind).boxed(),
            ),
            Venue::Okex => self.okex.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_kline_stream(streams, market_kind).boxed(),
            ),
            Venue::Mexc => self.mexc.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_kline_stream(streams, market_kind).boxed(),
            ),
        }
    }

    pub fn trade_stream(
        &self,
        config: &StreamConfig<Vec<TickerInfo>>,
    ) -> BoxStream<'static, Event> {
        let streams = config.id.clone();
        let market_kind = config.exchange.market_type();

        match config.exchange.venue() {
            Venue::Binance => self.binance.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_trade_stream(streams, market_kind).boxed(),
            ),
            Venue::Bybit => self.bybit.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_trade_stream(streams, market_kind).boxed(),
            ),
            Venue::Hyperliquid => self.hyperliquid.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_trade_stream(streams, market_kind).boxed(),
            ),
            Venue::Okex => self.okex.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_trade_stream(streams, market_kind).boxed(),
            ),
            Venue::Mexc => self.mexc.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_trade_stream(streams, market_kind).boxed(),
            ),
        }
    }

    pub fn depth_stream(&self, config: &StreamConfig<TickerInfo>) -> BoxStream<'static, Event> {
        let ticker = config.id;
        let push_freq = config.push_freq;

        match config.exchange.venue() {
            Venue::Binance => self.binance.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_depth_stream(ticker, push_freq).boxed(),
            ),
            Venue::Bybit => self.bybit.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_depth_stream(ticker, push_freq).boxed(),
            ),
            Venue::Hyperliquid => self.hyperliquid.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| {
                    handle
                        .connect_depth_stream(ticker, config.tick_mltp, push_freq)
                        .boxed()
                },
            ),
            Venue::Okex => self.okex.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_depth_stream(ticker, push_freq).boxed(),
            ),
            Venue::Mexc => self.mexc.clone().map_or_else(
                || Self::missing_venue_stream(config.exchange),
                |handle| handle.connect_depth_stream(ticker, push_freq).boxed(),
            ),
        }
    }

    /// Returns a map of tickers to their [`TickerInfo`].
    /// If metadata for a ticker can't be fetched/parsed expectedly, it will still be included in the map as `None`.
    ///
    /// `Binance`, `Bybit`, and `Hyperliquid` are fetched per market, while
    /// `Okex` and `Mexc` handle market branching internally due to combined perps endpoints.
    pub async fn fetch_ticker_metadata(
        &self,
        venue: Venue,
        markets: &[MarketKind],
    ) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError> {
        match venue {
            Venue::Binance => {
                let Some(handle) = self.binance.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(
                        handle
                            .fetch_ticker_metadata(binance::BinanceMarketScope::metadata(*market))
                            .await?,
                    );
                }
                Ok(out)
            }
            Venue::Bybit => {
                let Some(handle) = self.bybit.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(handle.fetch_ticker_metadata(*market).await?);
                }
                Ok(out)
            }
            Venue::Hyperliquid => {
                let Some(handle) = self.hyperliquid.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(handle.fetch_ticker_metadata(*market).await?);
                }
                Ok(out)
            }
            Venue::Okex => {
                let Some(handle) = self.okex.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_ticker_metadata(markets.to_vec()).await
            }
            Venue::Mexc => {
                let Some(handle) = self.mexc.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle
                    .fetch_ticker_metadata(mexc::MexcMarketScope::metadata(markets))
                    .await
            }
        }
    }

    /// Returns a map of tickers to their [`TickerStats`].
    ///
    /// `Binance`, `Bybit`, and `Hyperliquid` are fetched per market, while
    /// `Okex` and `Mexc` handle market branching internally due to combined perps endpoints.
    pub async fn fetch_ticker_stats(
        &self,
        venue: Venue,
        markets: &[MarketKind],
        contract_sizes: Option<HashMap<Ticker, f32>>,
    ) -> Result<HashMap<Ticker, TickerStats>, AdapterError> {
        match venue {
            Venue::Binance => {
                let Some(handle) = self.binance.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(
                        handle
                            .fetch_ticker_stats(binance::BinanceMarketScope::stats(
                                *market,
                                contract_sizes.clone(),
                            ))
                            .await?,
                    );
                }
                Ok(out)
            }
            Venue::Bybit => {
                let Some(handle) = self.bybit.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(handle.fetch_ticker_stats(*market).await?);
                }
                Ok(out)
            }
            Venue::Hyperliquid => {
                let Some(handle) = self.hyperliquid.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };

                let mut out = HashMap::default();
                for market in markets {
                    out.extend(handle.fetch_ticker_stats(*market).await?);
                }
                Ok(out)
            }
            Venue::Okex => {
                let Some(handle) = self.okex.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_ticker_stats(markets.to_vec()).await
            }
            Venue::Mexc => {
                let Some(handle) = self.mexc.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle
                    .fetch_ticker_stats(mexc::MexcMarketScope::stats(markets, contract_sizes))
                    .await
            }
        }
    }

    pub async fn fetch_klines(
        &self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(UnixMs, UnixMs)>,
    ) -> Result<Vec<Kline>, AdapterError> {
        let venue = ticker_info.ticker.exchange.venue();

        match venue {
            Venue::Binance => {
                let Some(handle) = self.binance.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_klines(ticker_info, timeframe, range).await
            }
            Venue::Bybit => {
                let Some(handle) = self.bybit.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_klines(ticker_info, timeframe, range).await
            }
            Venue::Hyperliquid => {
                let Some(handle) = self.hyperliquid.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_klines(ticker_info, timeframe, range).await
            }
            Venue::Okex => {
                let Some(handle) = self.okex.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_klines(ticker_info, timeframe, range).await
            }
            Venue::Mexc => {
                let Some(handle) = self.mexc.as_ref() else {
                    return Err(Self::missing_venue_error(venue));
                };
                handle.fetch_klines(ticker_info, timeframe, range).await
            }
        }
    }

    pub async fn fetch_open_interest(
        &self,
        ticker_info: TickerInfo,
        timeframe: Timeframe,
        range: Option<(UnixMs, UnixMs)>,
    ) -> Result<Vec<OpenInterest>, AdapterError> {
        let exchange = ticker_info.ticker.exchange;

        match exchange {
            Exchange::BinanceLinear | Exchange::BinanceInverse => {
                let Some(handle) = self.binance.as_ref() else {
                    return Err(Self::missing_venue_error(exchange.venue()));
                };
                handle
                    .fetch_open_interest(ticker_info, timeframe, range)
                    .await
            }
            Exchange::BybitLinear | Exchange::BybitInverse => {
                let Some(handle) = self.bybit.as_ref() else {
                    return Err(Self::missing_venue_error(exchange.venue()));
                };
                handle
                    .fetch_open_interest(ticker_info, timeframe, range)
                    .await
            }
            Exchange::OkexLinear | Exchange::OkexInverse => {
                let Some(handle) = self.okex.as_ref() else {
                    return Err(Self::missing_venue_error(exchange.venue()));
                };
                handle
                    .fetch_open_interest(ticker_info, timeframe, range)
                    .await
            }
            _ => Err(AdapterError::InvalidRequest(format!(
                "Open interest data not available for {exchange}"
            ))),
        }
    }

    pub async fn fetch_trades(
        &self,
        ticker_info: TickerInfo,
        from_time: UnixMs,
        data_path: Option<PathBuf>,
    ) -> Result<Vec<Trade>, AdapterError> {
        let exchange = ticker_info.ticker.exchange;

        match exchange.venue() {
            Venue::Binance => {
                let Some(handle) = self.binance.as_ref() else {
                    return Err(Self::missing_venue_error(exchange.venue()));
                };
                handle.fetch_trades(ticker_info, from_time, data_path).await
            }
            Venue::Hyperliquid => {
                let Some(handle) = self.hyperliquid.as_ref() else {
                    return Err(Self::missing_venue_error(exchange.venue()));
                };
                handle.fetch_trades(ticker_info, from_time, data_path).await
            }
            _ => Err(AdapterError::InvalidRequest(format!(
                "Trade fetch not available for {exchange}"
            ))),
        }
    }
}

impl std::hash::Hash for AdapterHandles {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::any::TypeId::of::<Self>().hash(state);
    }
}
