use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        hub::{TradeBuffer, WsAdapter, WsSession, WsTransport},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::de_string_to_number,
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{WS_DOMAIN, exchange_from_market_type, raw_qty_unit_from_market_type};
use crate::adapter::hub::AdapterError;
use fastwebsockets::Frame;
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde_json::Value;
use sonic_rs::{Deserialize, JsonValueTrait, to_object_iter_unchecked};
use std::{collections::HashMap, sync::Arc};

const BYBIT_PING_PAYLOAD: &[u8] = br#"{"op":"ping"}"#;

#[derive(Deserialize)]
struct SonicDepth {
    #[serde(rename = "u")]
    pub update_id: u64,
    #[serde(rename = "b")]
    pub bids: Vec<DeOrder>,
    #[serde(rename = "a")]
    pub asks: Vec<DeOrder>,
}

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "T")]
    pub time: u64,
    #[serde(rename = "p", deserialize_with = "de_string_to_number")]
    pub price: f32,
    #[serde(rename = "v", deserialize_with = "de_string_to_number")]
    pub qty: f32,
    #[serde(rename = "S")]
    pub is_sell: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SonicKline {
    #[serde(rename = "start")]
    pub time: u64,
    #[serde(rename = "open", deserialize_with = "de_string_to_number")]
    pub open: f32,
    #[serde(rename = "high", deserialize_with = "de_string_to_number")]
    pub high: f32,
    #[serde(rename = "low", deserialize_with = "de_string_to_number")]
    pub low: f32,
    #[serde(rename = "close", deserialize_with = "de_string_to_number")]
    pub close: f32,
    #[serde(rename = "volume", deserialize_with = "de_string_to_number")]
    pub volume: f32,
    #[serde(rename = "interval")]
    pub interval: String,
}

enum StreamData {
    Trade(Ticker, Vec<SonicTrade>),
    Depth(SonicDepth, String, u64),
    Kline(Ticker, Vec<SonicKline>),
}

#[derive(Debug)]
enum StreamName {
    Depth(Ticker),
    Trade(Ticker),
    Kline(Ticker),
    Unknown,
}

impl StreamName {
    fn from_topic(topic: &str, is_ticker: Option<Ticker>, market_type: MarketKind) -> Self {
        let parts: Vec<&str> = topic.split('.').collect();

        if let Some(ticker_str) = parts.last() {
            let exchange = exchange_from_market_type(market_type);
            let ticker = is_ticker.unwrap_or_else(|| Ticker::new(ticker_str, exchange));

            match parts.first() {
                Some(&"publicTrade") => StreamName::Trade(ticker),
                Some(&"orderbook") => StreamName::Depth(ticker),
                Some(&"kline") => StreamName::Kline(ticker),
                _ => StreamName::Unknown,
            }
        } else {
            StreamName::Unknown
        }
    }
}

#[derive(Debug)]
enum StreamWrapper {
    Trade,
    Depth,
    Kline,
}

async fn connect_and_subscribe(
    streams: &Value,
    market_type: MarketKind,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, String> {
    let url = format!(
        "wss://{}/v5/public/{}",
        WS_DOMAIN,
        match market_type {
            MarketKind::Spot => "spot",
            MarketKind::LinearPerps => "linear",
            MarketKind::InversePerps => "inverse",
        }
    );

    let mut websocket = WsTransport::establish(WS_DOMAIN, &url, proxy_cfg)
        .await
        .map_err(|err| format!("Failed to connect: {err}"))?;

    websocket
        .write_frame(Frame::text(fastwebsockets::Payload::Borrowed(
            streams.to_string().as_bytes(),
        )))
        .await
        .map_err(|e| format!("Failed subscribing: {e}"))?;

    Ok(websocket)
}

#[allow(unused_assignments)]
fn feed_de(
    slice: &[u8],
    ticker: Option<Ticker>,
    market_type: MarketKind,
) -> Result<StreamData, AdapterError> {
    let mut stream_type: Option<StreamWrapper> = None;
    let mut depth_wrap: Option<SonicDepth> = None;

    let mut data_type = String::new();
    let mut topic_ticker: Option<Ticker> = ticker;

    let iter: sonic_rs::ObjectJsonIter = unsafe { to_object_iter_unchecked(slice) };

    for elem in iter {
        let (k, v) = elem.map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if k == "topic" {
            if let Some(val) = v.as_str() {
                let mut is_ticker = None;

                if let Some(t) = ticker {
                    is_ticker = Some(t);
                }

                match StreamName::from_topic(val, is_ticker, market_type) {
                    StreamName::Depth(t) => {
                        stream_type = Some(StreamWrapper::Depth);
                        topic_ticker = Some(t);
                    }
                    StreamName::Trade(t) => {
                        stream_type = Some(StreamWrapper::Trade);
                        topic_ticker = Some(t);
                    }
                    StreamName::Kline(t) => {
                        stream_type = Some(StreamWrapper::Kline);
                        topic_ticker = Some(t);
                    }
                    _ => {
                        log::error!("Unknown stream name");
                    }
                }
            }
        } else if k == "type" {
            if let Some(value) = v.as_str() {
                value.clone_into(&mut data_type);
            } else {
                return Err(AdapterError::ParseError(
                    "Bybit frame `type` field is not a string".to_string(),
                ));
            }
        } else if k == "data" {
            match stream_type {
                Some(StreamWrapper::Trade) => {
                    let trade_wrap: Vec<SonicTrade> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    if let Some(t) = topic_ticker {
                        return Ok(StreamData::Trade(t, trade_wrap));
                    } else {
                        return Err(AdapterError::ParseError(
                            "Missing ticker for trade data".to_string(),
                        ));
                    }
                }
                Some(StreamWrapper::Depth) => {
                    if depth_wrap.is_none() {
                        depth_wrap = Some(SonicDepth {
                            update_id: 0,
                            bids: Vec::new(),
                            asks: Vec::new(),
                        });
                    }
                    depth_wrap = Some(
                        sonic_rs::from_str(&v.as_raw_faststr())
                            .map_err(|e| AdapterError::ParseError(e.to_string()))?,
                    );
                }
                Some(StreamWrapper::Kline) => {
                    let kline_wrap: Vec<SonicKline> = sonic_rs::from_str(&v.as_raw_faststr())
                        .map_err(|e| AdapterError::ParseError(e.to_string()))?;

                    if let Some(t) = topic_ticker {
                        return Ok(StreamData::Kline(t, kline_wrap));
                    } else {
                        return Err(AdapterError::ParseError(
                            "Missing ticker for kline data".to_string(),
                        ));
                    }
                }
                _ => {
                    log::error!("Unknown stream type");
                }
            }
        } else if k == "cts"
            && let Some(dw) = depth_wrap
        {
            let time: u64 = v
                .as_u64()
                .ok_or_else(|| AdapterError::ParseError("Failed to parse u64".to_string()))?;

            return Ok(StreamData::Depth(dw, data_type.to_string(), time));
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

struct TradeAdapter {
    market_type: MarketKind,
    buffer: TradeBuffer,
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let market_type = self.market_type;
        connect_and_subscribe(
            &self.subscribe_message,
            market_type,
            self.proxy_cfg.as_ref(),
        )
        .await
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Trade(ticker, de_trade_vec)) =
            feed_de(payload, None, self.market_type)
        {
            if let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(&ticker) {
                let ticker_info = *ticker_info;
                let qty_norm = *qty_norm;

                for de_trade in &de_trade_vec {
                    let price =
                        Price::from_f32(de_trade.price).round_to_min_tick(ticker_info.min_ticksize);
                    let qty = qty_norm.normalize_qty(de_trade.qty, de_trade.price);

                    self.buffer.push(
                        ticker,
                        Trade {
                            time: de_trade.time.into(),
                            is_sell: de_trade.is_sell == "Sell",
                            price,
                            qty,
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
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        tickers
            .iter()
            .map(|ticker_info| StreamKind::Trades {
                ticker_info: *ticker_info,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let stream = tickers
        .iter()
        .map(|ticker_info| {
            format!(
                "publicTrade.{}",
                ticker_info.ticker.to_full_symbol_and_type().0
            )
        })
        .collect::<Vec<_>>();
    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": stream
    });

    let ticker_info_map: FxHashMap<Ticker, (TickerInfo, QtyNormalization)> = tickers
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

    let adapter = TradeAdapter {
        market_type,
        buffer: TradeBuffer::new(ticker_info_map),
        subscribe_message,
        proxy_cfg,
    };

    WsSession::with_text_ping(BYBIT_PING_PAYLOAD, stream_scope).run(adapter)
}

struct DepthAdapter {
    stream: StreamKind,
    ticker_info: TickerInfo,
    market_type: MarketKind,
    qty_norm: QtyNormalization,
    orderbook: LocalDepthCache,
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let market_type = self.market_type;
        connect_and_subscribe(
            &self.subscribe_message,
            market_type,
            self.proxy_cfg.as_ref(),
        )
        .await
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(data) = feed_de(payload, Some(self.ticker_info.ticker), self.market_type) {
            match data {
                StreamData::Depth(de_depth, data_type, time) => {
                    let depth = DepthPayload {
                        last_update_id: de_depth.update_id,
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

                    if (data_type == "snapshot") || (depth.last_update_id == 1) {
                        self.orderbook.update_with_qty_norm(
                            DepthUpdate::Snapshot(depth),
                            self.ticker_info.min_ticksize,
                            Some(self.qty_norm),
                        );
                    } else if data_type == "delta" {
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
                }
                _ => {
                    log::warn!("Unknown data received");
                }
            }
        }

        Ok(())
    }

    async fn on_disconnected(&mut self, _reason: &str, _output: &mut mpsc::Sender<Event>) {}
}

pub fn connect_depth_stream(
    ticker_info: TickerInfo,
    depth_aggr: StreamTicksize,
    push_freq: PushFrequency,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream = StreamKind::Depth {
        ticker_info,
        depth_aggr,
        push_freq,
    };
    let stream_scope: Arc<[StreamKind]> = Arc::from(vec![stream].into_boxed_slice());

    let (symbol_str, market_type) = ticker_info.ticker.to_full_symbol_and_type();

    let qty_norm = QtyNormalization::with_raw_qty_unit(
        volume_size_unit() == SizeUnit::Quote,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let depth_level = if let PushFrequency::Custom(tf) = push_freq {
        match market_type {
            MarketKind::Spot => match tf {
                Timeframe::MS200 => "200",
                Timeframe::MS300 => "1000",
                _ => "200",
            },
            MarketKind::LinearPerps | MarketKind::InversePerps => match tf {
                Timeframe::MS100 => "200",
                Timeframe::MS300 => "1000",
                _ => "200",
            },
        }
    } else {
        "200"
    };

    let ws_stream = format!("orderbook.{depth_level}.{symbol_str}");
    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": [ws_stream]
    });

    let adapter = DepthAdapter {
        stream,
        ticker_info,
        market_type,
        qty_norm,
        orderbook: LocalDepthCache::default(),
        subscribe_message,
        proxy_cfg,
    };

    WsSession::with_text_ping(BYBIT_PING_PAYLOAD, stream_scope).run(adapter)
}

fn string_to_timeframe(interval: &str) -> Option<Timeframe> {
    Timeframe::KLINE
        .iter()
        .find(|&tf| {
            tf.to_minutes().to_string() == interval || {
                if tf == &Timeframe::D1 {
                    interval == "D"
                } else {
                    false
                }
            }
        })
        .copied()
}

struct KlineAdapter {
    market_type: MarketKind,
    ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)>,
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        let market_type = self.market_type;
        connect_and_subscribe(
            &self.subscribe_message,
            market_type,
            self.proxy_cfg.as_ref(),
        )
        .await
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
                if let Some(timeframe) = string_to_timeframe(&de_kline.interval) {
                    if let Some((ticker_info, qty_norm)) = self.ticker_info_map.get(&ticker) {
                        let ticker_info = *ticker_info;
                        let volume = qty_norm.normalize_qty(de_kline.volume, de_kline.close);

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
                    } else {
                        log::error!("Ticker info not found for ticker: {}", ticker);
                    }
                } else {
                    log::error!("Failed to find timeframe: {}", &de_kline.interval);
                }
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

    let stream_str = streams
        .iter()
        .map(|(ticker_info, timeframe)| {
            let ticker = ticker_info.ticker;
            let timeframe_str = if Timeframe::D1 == *timeframe {
                "D".to_string()
            } else {
                timeframe.to_minutes().to_string()
            };
            format!(
                "kline.{timeframe_str}.{}",
                ticker.to_full_symbol_and_type().0
            )
        })
        .collect::<Vec<String>>();
    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": stream_str
    });

    let ticker_info_map: HashMap<Ticker, (TickerInfo, QtyNormalization)> = streams
        .iter()
        .map(|(ticker_info, _)| {
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

    let adapter = KlineAdapter {
        market_type,
        ticker_info_map,
        subscribe_message,
        proxy_cfg,
    };

    WsSession::with_text_ping(BYBIT_PING_PAYLOAD, stream_scope).run(adapter)
}
