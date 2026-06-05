use crate::{
    Event, Kline, Price, PushFrequency, TickMultiplier, Ticker, TickerInfo, Timeframe, Trade,
    Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        hub::{TradeBuffer, WsAdapter, WsSession, WsTransport},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{HyperliquidHandle, WS_DOMAIN, raw_qty_unit_from_market_type};
use crate::adapter::hub::AdapterError;
use fastwebsockets::Frame;
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const SIG_FIG_LIMIT: i32 = 5;
const ALLOWED_MANTISSA: [i32; 3] = [1, 2, 5];
const HYPERLIQUID_PING_PAYLOAD: &[u8] = br#"{"method":"ping"}"#;

#[derive(Clone, Copy, Debug)]
struct DepthFeedConfig {
    pub n_sig_figs: Option<i32>,
    pub mantissa: Option<i32>,
}

impl DepthFeedConfig {
    fn new(n_sig_figs: Option<i32>, mantissa: Option<i32>) -> Self {
        Self {
            n_sig_figs,
            mantissa,
        }
    }

    fn full_precision() -> Self {
        Self {
            n_sig_figs: None,
            mantissa: None,
        }
    }
}

fn snap_multiplier_to_125(multiplier: u16) -> (i32, i32) {
    const SQRT2: f32 = std::f32::consts::SQRT_2;
    const SQRT10: f32 = 3.162_277_7;
    const SQRT50: f32 = 7.071_068;

    let m = (multiplier as f32).max(1.0);
    let mut kf = m.log10().floor();
    let rem = m / 10_f32.powf(kf);

    let (mantissa, bump) = if rem < SQRT2 {
        (1, false)
    } else if rem < SQRT10 {
        (2, false)
    } else if rem < SQRT50 {
        (5, false)
    } else {
        (1, true)
    };

    if bump {
        kf += 1.0;
    }

    (kf as i32, mantissa)
}

fn config_from_multiplier(price: f32, multiplier: u16) -> DepthFeedConfig {
    if price <= 0.0 {
        return DepthFeedConfig::full_precision();
    }
    if multiplier <= 1 {
        return DepthFeedConfig::full_precision();
    }

    let int_digits = if price >= 1.0 {
        (price.abs().log10().floor() as i32 + 1).max(1)
    } else {
        0
    };

    let (k, m125) = snap_multiplier_to_125(multiplier);
    let n = if int_digits > SIG_FIG_LIMIT {
        (int_digits - k).clamp(2, SIG_FIG_LIMIT)
    } else {
        (SIG_FIG_LIMIT - k).clamp(2, SIG_FIG_LIMIT)
    };

    let mantissa = if n == SIG_FIG_LIMIT && (m125 == 2 || m125 == 5) {
        Some(m125)
    } else {
        None
    };

    DepthFeedConfig::new(Some(n), mantissa)
}

#[derive(Debug, Deserialize)]
struct HyperliquidDepth {
    levels: [Vec<HyperliquidLevel>; 2],
    time: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidLevel {
    #[serde(deserialize_with = "de_string_to_number")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    sz: f32,
}

#[derive(Debug, Deserialize)]
struct HyperliquidTrade {
    coin: String,
    side: String,
    #[serde(deserialize_with = "de_string_to_number")]
    px: f32,
    #[serde(deserialize_with = "de_string_to_number")]
    sz: f32,
    time: u64,
}

#[derive(Debug, Deserialize)]
struct HyperliquidKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "i")]
    interval: String,
    #[serde(rename = "o", deserialize_with = "de_string_to_number")]
    open: f32,
    #[serde(rename = "h", deserialize_with = "de_string_to_number")]
    high: f32,
    #[serde(rename = "l", deserialize_with = "de_string_to_number")]
    low: f32,
    #[serde(rename = "c", deserialize_with = "de_string_to_number")]
    close: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_number")]
    volume: f32,
}

enum StreamData {
    Trade(Vec<HyperliquidTrade>),
    Depth(HyperliquidDepth),
    Kline(HyperliquidKline),
}

async fn connect_websocket(
    domain: &str,
    path: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    WsTransport::establish(domain, &url, proxy_cfg).await
}

fn parse_websocket_message(payload: &[u8]) -> Result<StreamData, AdapterError> {
    let json: Value =
        serde_json::from_slice(payload).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let channel = json
        .get("channel")
        .and_then(|c| c.as_str())
        .ok_or_else(|| AdapterError::ParseError("Missing channel".to_string()))?;

    match channel {
        "trades" => {
            let trades: Vec<HyperliquidTrade> = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Trade(trades))
        }
        "l2Book" => {
            let depth: HyperliquidDepth = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Depth(depth))
        }
        "candle" => {
            let kline: HyperliquidKline = serde_json::from_value(json["data"].clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?;
            Ok(StreamData::Kline(kline))
        }
        _ => Err(AdapterError::ParseError(format!(
            "Unknown channel: {}",
            channel
        ))),
    }
}

struct TradeAdapter {
    symbol_to_ticker: FxHashMap<String, Ticker>,
    buffer: TradeBuffer,
    subscription_coins: Vec<String>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(WS_DOMAIN, "/ws", self.proxy_cfg.as_ref())
            .await
            .map_err(|e| format!("Failed to connect to websocket: {e}"))?;

        for symbol_str in &self.subscription_coins {
            let trades_subscribe_msg = json!({
                "method": "subscribe",
                "subscription": {
                    "type": "trades",
                    "coin": symbol_str
                }
            });

            websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    trades_subscribe_msg.to_string().as_bytes(),
                )))
                .await
                .map_err(|e| format!("Failed subscribing: {e}"))?;
        }

        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {
        self.buffer.last_flush = tokio::time::Instant::now();
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Trade(trades)) = parse_websocket_message(payload) {
            for hl_trade in trades {
                if let Some(ticker) = self.symbol_to_ticker.get(&hl_trade.coin)
                    && let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(ticker)
                {
                    let ticker_info = *ticker_info;
                    let qty_norm = *qty_norm;
                    let price =
                        Price::from_f32(hl_trade.px).round_to_min_tick(ticker_info.min_ticksize);
                    self.buffer.push(
                        *ticker,
                        Trade {
                            time: hl_trade.time.into(),
                            is_sell: hl_trade.side == "A",
                            price,
                            qty: qty_norm.normalize_qty(hl_trade.sz, hl_trade.px),
                        },
                    );
                } else {
                    log::error!(
                        "Ticker info not found for Hyperliquid coin: {}",
                        hl_trade.coin
                    );
                }
            }
        }

        self.buffer.flush_if_ready(output).await;
        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }
}

pub fn connect_trade_stream(
    tickers: Vec<TickerInfo>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        tickers
            .iter()
            .map(|ticker_info| StreamKind::Trades {
                ticker_info: *ticker_info,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let ticker_info_map = tickers
        .iter()
        .map(|ticker_info| {
            (
                ticker_info.ticker,
                (
                    *ticker_info,
                    QtyNormalization::with_raw_qty_unit(
                        volume_size_unit() == SizeUnit::Quote,
                        *ticker_info,
                        raw_qty_unit_from_market_type(market_type),
                    ),
                ),
            )
        })
        .collect();

    let symbol_to_ticker = tickers
        .iter()
        .map(|ticker_info| {
            let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();
            (symbol_str, ticker_info.ticker)
        })
        .collect();

    let subscription_coins = tickers
        .iter()
        .map(|ticker_info| ticker_info.ticker.to_full_symbol_and_type().0)
        .collect();

    let adapter = TradeAdapter {
        symbol_to_ticker,
        buffer: TradeBuffer::new(ticker_info_map),
        subscription_coins,
        proxy_cfg,
    };

    WsSession::with_text_ping(HYPERLIQUID_PING_PAYLOAD, None, stream_scope).run(adapter)
}

struct DepthAdapter {
    handle: HyperliquidHandle,
    stream: StreamKind,
    ticker_info: TickerInfo,
    symbol_str: String,
    qty_norm: QtyNormalization,
    user_multiplier: u16,
    local_depth_cache: LocalDepthCache,
    pending_snapshot_emit_ms: Option<u64>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let snapshot = self
            .handle
            .fetch_depth_snapshot(self.ticker_info.ticker)
            .await
            .map_err(|e| format!("Failed to fetch depth snapshot: {e}"))?;

        let Some(best_bid_price) = snapshot.bids.first().map(|o| o.price) else {
            return Err("Depth snapshot missing bids".to_string());
        };

        let depth_cfg = config_from_multiplier(best_bid_price, self.user_multiplier);
        let snapshot_time_ms = snapshot.time.as_u64();

        self.local_depth_cache.update_with_qty_norm(
            DepthUpdate::Snapshot(snapshot),
            self.ticker_info.min_ticksize,
            Some(self.qty_norm),
        );
        self.pending_snapshot_emit_ms = Some(snapshot_time_ms);

        let mut websocket = connect_websocket(WS_DOMAIN, "/ws", self.proxy_cfg.as_ref())
            .await
            .map_err(|e| format!("Failed to connect to websocket: {e}"))?;

        let mut depth_subscription = json!({
            "method": "subscribe",
            "subscription": {
                "type": "l2Book",
                "coin": self.symbol_str,
            }
        });

        if let Some(n) = depth_cfg.n_sig_figs {
            depth_subscription["subscription"]["nSigFigs"] = json!(n);
        }
        if let (Some(m), Some(5)) = (depth_cfg.mantissa, depth_cfg.n_sig_figs)
            && m != 1
            && ALLOWED_MANTISSA.contains(&m)
        {
            depth_subscription["subscription"]["mantissa"] = json!(m);
        }

        websocket
            .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                depth_subscription.to_string().as_bytes(),
            )))
            .await
            .map_err(|e| format!("Failed subscribing: {e}"))?;

        Ok(websocket)
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        if let Some(snapshot_time_ms) = self.pending_snapshot_emit_ms.take() {
            let _ = output
                .send(Event::DepthReceived(
                    self.stream,
                    snapshot_time_ms.into(),
                    self.local_depth_cache.depth.clone(),
                ))
                .await;
        }
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Depth(depth)) = parse_websocket_message(payload) {
            let bids = depth.levels[0]
                .iter()
                .map(|level| DeOrder {
                    price: level.px,
                    qty: level.sz,
                })
                .collect();
            let asks = depth.levels[1]
                .iter()
                .map(|level| DeOrder {
                    price: level.px,
                    qty: level.sz,
                })
                .collect();

            let depth_payload = DepthPayload {
                last_update_id: depth.time,
                time: depth.time.into(),
                bids,
                asks,
            };
            self.local_depth_cache.update_with_qty_norm(
                DepthUpdate::Snapshot(depth_payload),
                self.ticker_info.min_ticksize,
                Some(self.qty_norm),
            );

            let _ = output
                .send(Event::DepthReceived(
                    self.stream,
                    depth.time.into(),
                    self.local_depth_cache.depth.clone(),
                ))
                .await;
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_depth_stream(
    handle: HyperliquidHandle,
    ticker_info: TickerInfo,
    depth_aggr: StreamTicksize,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let tick_multiplier = match depth_aggr {
        StreamTicksize::ServerSide(multiplier) => Some(multiplier),
        StreamTicksize::Client => None,
    };
    let stream = StreamKind::Depth {
        ticker_info,
        depth_aggr,
        push_freq,
    };

    let stream_scope: Arc<[StreamKind]> = Arc::from(vec![stream].into_boxed_slice());

    let ticker = ticker_info.ticker;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(ticker_info.market_type()),
    );
    let user_multiplier = tick_multiplier.unwrap_or(TickMultiplier(1)).0;

    let (symbol_str, _) = ticker.to_full_symbol_and_type();

    let adapter = DepthAdapter {
        handle,
        stream,
        ticker_info,
        symbol_str,
        qty_norm,
        user_multiplier,
        local_depth_cache: LocalDepthCache::default(),
        pending_snapshot_emit_ms: None,
        proxy_cfg,
    };

    WsSession::with_text_ping(HYPERLIQUID_PING_PAYLOAD, None, stream_scope).run(adapter)
}

struct KlineAdapter {
    market_type: MarketKind,
    size_in_quote_ccy: bool,
    stream_lookup: FxHashMap<(String, String), (TickerInfo, Timeframe)>,
    subscriptions: Vec<(String, String)>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(WS_DOMAIN, "/ws", self.proxy_cfg.as_ref())
            .await
            .map_err(|e| format!("Failed to connect to websocket: {e}"))?;

        for (symbol_str, interval) in &self.subscriptions {
            let subscribe_msg = json!({
                "method": "subscribe",
                "subscription": {
                    "type": "candle",
                    "coin": symbol_str,
                    "interval": interval
                }
            });

            websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    subscribe_msg.to_string().as_bytes(),
                )))
                .await
                .map_err(|e| format!("Failed subscribing: {e}"))?;
        }

        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Kline(hl_kline)) = parse_websocket_message(payload)
            && let Some((ticker_info, timeframe)) = self
                .stream_lookup
                .get(&(hl_kline.symbol.clone(), hl_kline.interval.clone()))
        {
            let qty_norm = QtyNormalization::with_raw_qty_unit(
                self.size_in_quote_ccy,
                *ticker_info,
                raw_qty_unit_from_market_type(self.market_type),
            );
            let volume = qty_norm.normalize_qty(hl_kline.volume, hl_kline.close);

            let kline = Kline::new(
                hl_kline.time,
                hl_kline.open,
                hl_kline.high,
                hl_kline.low,
                hl_kline.close,
                Volume::TotalOnly(volume),
                ticker_info.min_ticksize,
            );

            let stream_kind = StreamKind::Kline {
                ticker_info: *ticker_info,
                timeframe: *timeframe,
            };
            let _ = output.send(Event::KlineReceived(stream_kind, kline)).await;
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_kline_stream(
    streams: Vec<(TickerInfo, Timeframe)>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        streams
            .iter()
            .map(|(ticker_info, timeframe)| StreamKind::Kline {
                ticker_info: *ticker_info,
                timeframe: *timeframe,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let stream_lookup = streams
        .iter()
        .map(|(ticker_info, timeframe)| {
            (
                (
                    ticker_info.ticker.to_full_symbol_and_type().0,
                    timeframe.to_string(),
                ),
                (*ticker_info, *timeframe),
            )
        })
        .collect();

    let subscriptions = streams
        .iter()
        .map(|(ticker_info, timeframe)| {
            (
                ticker_info.ticker.to_full_symbol_and_type().0,
                timeframe.to_string(),
            )
        })
        .collect();

    let adapter = KlineAdapter {
        market_type,
        size_in_quote_ccy: volume_size_unit() == SizeUnit::Quote,
        stream_lookup,
        subscriptions,
        proxy_cfg,
    };

    WsSession::with_text_ping(HYPERLIQUID_PING_PAYLOAD, None, stream_scope).run(adapter)
}
