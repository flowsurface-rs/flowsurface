use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, UnixMs, Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        connect::{WsAdapter, WsSession, WsTransport, emit_connected},
        hub::{TradeBuffer, channel},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{
    MEXC_FUTURES_WS_DOMAIN, MEXC_FUTURES_WS_PATH, MexcHandle, contract_size_for_market,
    convert_to_mexc_timeframe, exchange_from_market_type, raw_qty_unit_from_market_type,
};
use crate::adapter::hub::AdapterError;
use fastwebsockets::Frame;
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde_json::json;
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use std::{collections::HashMap, sync::Arc};

const PING_PAYLOAD: &[u8] = br#"{"method":"ping"}"#;

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "p")]
    pub price: f32,
    #[serde(rename = "v")]
    pub qty: f32,
    #[serde(rename = "T")]
    pub direction: u8,
    #[serde(rename = "t")]
    pub time: u64,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct FuturesDepthItem {
    #[serde()]
    pub price: f32,
    #[serde()]
    pub qty: f32,
    #[serde()]
    pub order_count: f32,
}

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "asks")]
    pub asks: Vec<FuturesDepthItem>,
    #[serde(rename = "bids")]
    pub bids: Vec<FuturesDepthItem>,
    #[serde(rename = "version")]
    pub version: u64,
}

#[derive(Deserialize, Debug, Clone)]
struct SonicKline {
    #[serde(rename = "t")]
    time: u64,
    #[serde(rename = "o")]
    open: f32,
    #[serde(rename = "h")]
    high: f32,
    #[serde(rename = "l")]
    low: f32,
    #[serde(rename = "c")]
    close: f32,
    #[serde(rename = "q")]
    quote_volume: f32,
    #[serde(rename = "a")]
    _amount: f32,
    #[serde(rename = "interval")]
    interval: String,
    #[serde(rename = "symbol")]
    symbol: String,
}

#[allow(dead_code)]
enum StreamData {
    Trade(Ticker, Vec<SonicTrade>, u64),
    Depth(SonicDepth, u64),
    Kline(Ticker, Vec<SonicKline>),
    Pong(u64),
    Subscription(String),
}

#[derive(Debug)]
enum StreamName {
    Depth,
    Trade,
    Kline,
    Subscription(String),
    Error,
    Pong,
    Unknown,
}

impl StreamName {
    fn from_topic(topic: &str) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if parts.first() == Some(&"pong") {
            return StreamName::Pong;
        }

        match parts.get(1) {
            Some(&"sub") => {
                StreamName::Subscription(parts.get(2).map(|s| s.to_string()).unwrap_or_default())
            }
            Some(&"deal") => StreamName::Trade,
            Some(&"depth") => StreamName::Depth,
            Some(&"kline") => StreamName::Kline,
            Some(&"error") => StreamName::Error,
            _ => StreamName::Unknown,
        }
    }
}

fn feed_de(
    slice: &[u8],
    ticker: Option<Ticker>,
    market_type: MarketKind,
) -> Result<StreamData, AdapterError> {
    let mut stream_type: Option<StreamName> = None;
    let mut ts: Option<u64> = None;
    let mut data_faststr: Option<sonic_rs::FastStr> = None;

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    let mut topic_ticker: Option<Ticker> = ticker;

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "channel" {
            if let Some(val) = v.as_str() {
                stream_type = Some(StreamName::from_topic(val));
            }
        } else if k == "data" {
            data_faststr = Some(v.as_raw_faststr().clone());
        } else if k == "ts" {
            ts = Some(
                v.as_u64()
                    .ok_or_else(|| AdapterError::ParseError("ts not found".to_string()))?,
            );
        } else if k == "symbol" {
            let ticker_str = v
                .as_str()
                .ok_or_else(|| AdapterError::ParseError("symbol does not exist".to_string()))?;
            if topic_ticker.is_none() {
                topic_ticker = Some(Ticker::new(
                    ticker_str,
                    exchange_from_market_type(market_type),
                ));
            }
        }
    }

    if let Some(data) = data_faststr {
        match stream_type {
            Some(StreamName::Kline) => {
                let mut kline_data: SonicKline = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                kline_data.time *= 1000;

                let ticker =
                    Ticker::new(&kline_data.symbol, exchange_from_market_type(market_type));
                return Ok(StreamData::Kline(ticker, vec![kline_data]));
            }
            Some(StreamName::Trade) => {
                let deals_data: Vec<SonicTrade> = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                let trade_ticker = topic_ticker.ok_or_else(|| {
                    AdapterError::ParseError("Missing ticker for trade data".to_string())
                })?;
                return Ok(StreamData::Trade(
                    trade_ticker,
                    deals_data,
                    ts.unwrap_or_default(),
                ));
            }
            Some(StreamName::Depth) => {
                let depth = sonic_rs::from_str(&data)
                    .map_err(|e| AdapterError::ParseError(e.to_string()))?;
                return Ok(StreamData::Depth(depth, ts.unwrap_or_default()));
            }
            Some(StreamName::Pong) => {
                return Ok(StreamData::Pong(ts.unwrap_or_default()));
            }
            Some(StreamName::Subscription(name)) => {
                return Ok(StreamData::Subscription(name));
            }
            Some(StreamName::Error) => {
                log::error!("Error: {data}");
            }
            _ => {
                log::error!("Unknown stream type");
            }
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    match interval {
        "Min1" => Some(Timeframe::M1),
        "Min5" => Some(Timeframe::M5),
        "Min15" => Some(Timeframe::M15),
        "Min30" => Some(Timeframe::M30),
        "Min60" => Some(Timeframe::H1),
        "Hour4" => Some(Timeframe::H4),
        "Day1" => Some(Timeframe::D1),
        _ => None,
    }
}

async fn connect_websocket(
    domain: &str,
    path: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, AdapterError> {
    let url = format!("wss://{}{}", domain, path);
    WsTransport::establish(domain, &url, proxy_cfg).await
}

struct TradeAdapter {
    market_type: MarketKind,
    tickers: Vec<TickerInfo>,
    buffer: TradeBuffer,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|_| "Failed to connect to websocket".to_string())?;

        for ticker_info in &self.tickers {
            let symbol = ticker_info.ticker.to_full_symbol_and_type().0;
            let deal_subscription = json!({
                "method": "sub.deal",
                "param": {
                    "symbol": symbol,
                }
            });

            if websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    deal_subscription.to_string().as_bytes(),
                )))
                .await
                .is_err()
            {
                log::error!("Failed to subscribe to trade stream for {}", symbol);
            }
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
        if let Ok(StreamData::Trade(ticker, mut de_trades, _)) =
            feed_de(payload, None, self.market_type)
        {
            if let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(&ticker) {
                let ticker_info = *ticker_info;
                let qty_norm = *qty_norm;

                de_trades.sort_unstable_by_key(|t| t.time);
                for trade in &de_trades {
                    let price =
                        Price::from_f32(trade.price).round_to_min_tick(ticker_info.min_ticksize);
                    self.buffer.push(
                        ticker,
                        Trade {
                            time: trade.time.into(),
                            is_sell: trade.direction == 2,
                            price,
                            qty: qty_norm.normalize_qty(trade.qty, trade.price),
                        },
                    );
                }
            } else {
                log::error!("Ticker info not found for ticker: {}", ticker);
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
    channel(100, move |mut output| async move {
        let stream_scope: Arc<[StreamKind]> = Arc::from(
            tickers
                .iter()
                .map(|ticker_info| StreamKind::Trades {
                    ticker_info: *ticker_info,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );

        if tickers.is_empty() {
            let _ = output
                .send(Event::Disconnected(
                    stream_scope,
                    "Empty MEXC trade stream payload".to_string(),
                ))
                .await;
            return;
        }

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
            .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

        let mut adapter = TradeAdapter {
            market_type,
            tickers,
            buffer: TradeBuffer::new(ticker_info_map),
            proxy_cfg,
        };

        WsSession::with_text_ping(PING_PAYLOAD, Some(b"pong"), stream_scope)
            .run(&mut adapter, &mut output)
            .await;
    })
}

struct DepthAdapter {
    handle: MexcHandle,
    ticker_info: TickerInfo,
    market_type: MarketKind,
    symbol_str: String,
    stream: StreamKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
    qty_norm: QtyNormalization,
    orderbook: LocalDepthCache,
    snapshot_ready: bool,
    snapshot_time: UnixMs,
    stream_scope: Arc<[StreamKind]>,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|_| "Failed to connect to websocket".to_string())?;

        let depth_subscription = json!({
            "method": "sub.depth",
            "param": {
                "symbol": self.symbol_str,
            }
        });

        websocket
            .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                depth_subscription.to_string().as_bytes(),
            )))
            .await
            .map_err(|_| "Failed to send depth subscription frame".to_string())?;

        Ok(websocket)
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        self.snapshot_ready = false;
        self.snapshot_time = UnixMs::ZERO;
        emit_connected(output, &self.stream_scope).await;
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        let ticker = self.ticker_info.ticker;

        match feed_de(payload, Some(ticker), self.market_type) {
            Ok(data) => match data {
                StreamData::Pong(_) => {}
                StreamData::Subscription(stream_name) => {
                    if stream_name == "depth" {
                        match self.handle.fetch_depth_snapshot(ticker).await {
                            Ok(snapshot) => {
                                self.snapshot_time = snapshot.time;
                                self.snapshot_ready = true;
                                self.orderbook.update_with_qty_norm(
                                    DepthUpdate::Snapshot(snapshot),
                                    self.ticker_info.min_ticksize,
                                    Some(self.qty_norm),
                                );
                            }
                            Err(e) => {
                                return Err(format!("Failed to fetch depth snapshot: {e}"));
                            }
                        }
                    }
                }
                StreamData::Depth(de_depth, time) => {
                    if !self.snapshot_ready || time < self.snapshot_time.as_u64() {
                        return Ok(());
                    }

                    let depth = DepthPayload {
                        last_update_id: de_depth.version,
                        time: time.into(),
                        bids: de_depth
                            .bids
                            .iter()
                            .map(|x| DeOrder {
                                price: x.price,
                                qty: x.qty,
                            })
                            .collect(),
                        asks: de_depth
                            .asks
                            .iter()
                            .map(|x| DeOrder {
                                price: x.price,
                                qty: x.qty,
                            })
                            .collect(),
                    };

                    self.orderbook.update_with_qty_norm(
                        DepthUpdate::Diff(depth),
                        self.ticker_info.min_ticksize,
                        Some(self.qty_norm),
                    );

                    let _ = output
                        .send(Event::DepthReceived(
                            self.stream,
                            time.into(),
                            self.orderbook.depth.clone(),
                        ))
                        .await;
                }
                StreamData::Trade(_, _, _) | StreamData::Kline(_, _) => {}
            },
            Err(e) => {
                log::error!("Failed to parse MEXC depth message: {}", e);
            }
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_depth_stream(
    handle: MexcHandle,
    ticker_info: TickerInfo,
    depth_aggr: StreamTicksize,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        let stream = StreamKind::Depth {
            ticker_info,
            depth_aggr,
            push_freq,
        };
        let stream_scope: Arc<[StreamKind]> = Arc::from(vec![stream].into_boxed_slice());
        let ticker = ticker_info.ticker;
        let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

        let qty_norm = QtyNormalization::with_raw_qty_unit(
            volume_size_unit() == SizeUnit::Quote,
            ticker_info,
            raw_qty_unit_from_market_type(market_type),
        );

        let mut adapter = DepthAdapter {
            handle,
            ticker_info,
            market_type,
            symbol_str,
            stream,
            proxy_cfg,
            qty_norm,
            orderbook: LocalDepthCache::default(),
            snapshot_ready: false,
            snapshot_time: UnixMs::ZERO,
            stream_scope: stream_scope.clone(),
        };

        WsSession::with_text_ping(PING_PAYLOAD, Some(b"pong"), stream_scope)
            .run(&mut adapter, &mut output)
            .await;
    })
}

struct KlineAdapter {
    market_type: MarketKind,
    streams: Vec<(TickerInfo, Timeframe)>,
    ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)>,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let mut websocket = connect_websocket(
            MEXC_FUTURES_WS_DOMAIN,
            MEXC_FUTURES_WS_PATH,
            self.proxy_cfg.as_ref(),
        )
        .await
        .map_err(|e| format!("Failed to connect: {e}"))?;

        let mut subscribed_any = false;

        for (ticker_info, timeframe) in &self.streams {
            let ticker = ticker_info.ticker;
            let symbol = ticker.to_full_symbol_and_type().0;

            let Some(interval) = convert_to_mexc_timeframe(*timeframe, self.market_type) else {
                log::error!(
                    "Unsupported MEXC kline timeframe requested: {} ({})",
                    timeframe,
                    ticker
                );
                continue;
            };

            let subscribe_msg = json!({
                "method": "sub.kline",
                "param": {
                    "symbol": symbol.to_uppercase(),
                    "interval": interval,
                },
                "gzip": false,
            });

            if websocket
                .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
                    subscribe_msg.to_string().as_bytes(),
                )))
                .await
                .is_ok()
            {
                subscribed_any = true;
            }
        }

        if !subscribed_any {
            return Err("No supported MEXC kline timeframes requested".to_string());
        }

        Ok(websocket)
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Kline(ticker, de_kline_vec)) =
            feed_de(payload, None, self.market_type)
        {
            for de_kline in &de_kline_vec {
                let Some(timeframe) = string_to_timeframe(&de_kline.interval) else {
                    log::error!(
                        "Failed to find timeframe: {}, {:?}",
                        &de_kline.interval,
                        self.streams
                    );
                    continue;
                };

                let Some((ticker_info, qty_norm)) = self.ticker_info_map.get(&ticker) else {
                    log::error!("Ticker info not found for ticker: {}", ticker);
                    continue;
                };

                let ticker_info = *ticker_info;
                let volume = qty_norm.normalize_qty(de_kline.quote_volume, de_kline.close);

                let kline = Kline::new(
                    de_kline.time,
                    de_kline.open,
                    de_kline.high,
                    de_kline.low,
                    de_kline.close,
                    Volume::TotalOnly(volume),
                    ticker_info.min_ticksize,
                );

                let _ = output
                    .send(Event::KlineReceived(
                        StreamKind::Kline {
                            ticker_info,
                            timeframe,
                        },
                        kline,
                    ))
                    .await;
            }
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
    channel(100, move |mut output| async move {
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

        if streams.is_empty() {
            let _ = output
                .send(Event::Disconnected(
                    stream_scope,
                    "Empty MEXC kline stream payload".to_string(),
                ))
                .await;
            return;
        }

        if market_type == MarketKind::Spot {
            let _ = output
                .send(Event::Disconnected(
                    stream_scope,
                    "MEXC spot kline websocket stream is not supported".to_string(),
                ))
                .await;
            return;
        }

        let ticker_info_map = streams
            .iter()
            .map(|(ticker_info, _)| {
                contract_size_for_market(*ticker_info, market_type, "connect_kline_stream").map(
                    |_| {
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
                    },
                )
            })
            .collect::<Result<HashMap<_, _>, _>>();

        let ticker_info_map = match ticker_info_map {
            Ok(map) => map,
            Err(err) => {
                let _ = output
                    .send(Event::Disconnected(stream_scope, err.to_string()))
                    .await;
                return;
            }
        };

        let mut adapter = KlineAdapter {
            market_type,
            streams,
            ticker_info_map,
            proxy_cfg,
        };

        WsSession::with_text_ping(PING_PAYLOAD, None, stream_scope)
            .run(&mut adapter, &mut output)
            .await;
    })
}
