use crate::{
    Event, Kline, Price, PushFrequency, Ticker, TickerInfo, Timeframe, Trade, Volume,
    adapter::{
        MarketKind, StreamKind, StreamTicksize,
        hub::{TradeBuffer, WsAdapter, WsSession, WsTransport},
    },
    depth::{DeOrder, DepthPayload, DepthUpdate, LocalDepthCache},
    serde_util::{self, de_string_to_number},
    unit::qty::{QtyNormalization, SizeUnit, volume_size_unit},
};

use super::{WS_DOMAIN, raw_qty_unit_from_market_type, timeframe_to_okx_bar};
use crate::adapter::hub::AdapterError;
use fastwebsockets::Frame;
use futures::{SinkExt, Stream, channel::mpsc};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

const OKX_PING_PAYLOAD: &[u8] = b"ping";

#[derive(Deserialize, Debug)]
struct SonicTrade {
    #[serde(rename = "ts", deserialize_with = "de_string_to_number")]
    pub time: u64,
    #[serde(rename = "px", deserialize_with = "de_string_to_number")]
    pub price: f32,
    #[serde(rename = "sz", deserialize_with = "de_string_to_number")]
    pub qty: f32,
    #[serde(rename = "side")]
    pub is_sell: String,
}

struct SonicDepth {
    pub update_id: u64,
    pub bids: Vec<DeOrder>,
    pub asks: Vec<DeOrder>,
}

enum StreamData {
    Trade(String, Vec<SonicTrade>),
    Depth(SonicDepth, String, u64),
}

async fn connect_and_subscribe(
    streams: &Value,
    topic: &str,
    proxy_cfg: Option<&crate::proxy::Proxy>,
) -> Result<WsTransport, String> {
    let url = format!("wss://{WS_DOMAIN}/ws/v5/{topic}");

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

fn feed_de(slice: &[u8]) -> Result<StreamData, AdapterError> {
    let v: Value =
        serde_json::from_slice(slice).map_err(|e| AdapterError::ParseError(e.to_string()))?;

    let mut channel = String::new();
    let mut inst_id = String::new();
    if let Some(arg) = v.get("arg")
        && let Some(ch) = arg.get("channel").and_then(|c| c.as_str())
    {
        channel = ch.to_string();

        if let Some(symbol) = arg.get("instId").and_then(|c| c.as_str()) {
            inst_id = symbol.to_string();
        }
    }

    if let Some(action) = v.get("action").and_then(|a| a.as_str())
        && let Some(data_arr) = v.get("data")
        && let Some(first) = data_arr.get(0)
    {
        let bids: Vec<DeOrder> = if let Some(b) = first.get("bids") {
            serde_json::from_value(b.clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?
        } else {
            Vec::new()
        };
        let asks: Vec<DeOrder> = if let Some(a) = first.get("asks") {
            serde_json::from_value(a.clone())
                .map_err(|e| AdapterError::ParseError(e.to_string()))?
        } else {
            Vec::new()
        };

        let seq_id = first.get("seqId").and_then(|s| s.as_u64()).unwrap_or(0);

        let time = first
            .get("ts")
            .and_then(serde_util::value_as_u64)
            .unwrap_or(0);

        let depth = SonicDepth {
            update_id: seq_id,
            bids,
            asks,
        };

        match channel.as_str() {
            "books" => {
                let dtype = if action == "update" {
                    "delta"
                } else {
                    "snapshot"
                };
                return Ok(StreamData::Depth(depth, dtype.to_string(), time));
            }
            _ => {
                return Err(AdapterError::ParseError(
                    "Depth message for non-depth subscription".to_string(),
                ));
            }
        }
    }

    if let Some(data_arr) = v.get("data") {
        let trades: Vec<SonicTrade> = serde_json::from_value(data_arr.clone())
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        if matches!(channel.as_str(), "trades" | "trade") {
            if inst_id.is_empty() {
                return Err(AdapterError::ParseError(
                    "Missing instId for trade data".to_string(),
                ));
            }

            return Ok(StreamData::Trade(inst_id, trades));
        }
    }

    Err(AdapterError::ParseError("Unknown data".to_string()))
}

struct TradeAdapter {
    symbol_to_ticker: FxHashMap<String, Ticker>,
    buffer: TradeBuffer,
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for TradeAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        connect_and_subscribe(&self.subscribe_message, "public", self.proxy_cfg.as_ref()).await
    }

    async fn on_connected(&mut self, output: &mut mpsc::Sender<Event>) {
        self.buffer.flush(output).await;
    }

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Trade(inst_id, de_trade_vec)) = feed_de(payload) {
            if let Some(ticker) = self.symbol_to_ticker.get(&inst_id)
                && let Some((ticker_info, qty_norm)) = self.buffer.ticker_info(ticker)
            {
                let ticker_info = *ticker_info;
                let qty_norm = *qty_norm;

                for de_trade in &de_trade_vec {
                    let price =
                        Price::from_f32(de_trade.price).round_to_min_tick(ticker_info.min_ticksize);
                    let qty = qty_norm.normalize_qty(de_trade.qty, de_trade.price);

                    let trade = Trade {
                        time: de_trade.time.into(),
                        is_sell: de_trade.is_sell == "sell" || de_trade.is_sell == "SELL",
                        price,
                        qty,
                    };
                    self.buffer.push(*ticker, trade);
                }
            } else {
                log::error!("Ticker info not found for symbol: {}", inst_id);
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
    streams: Vec<TickerInfo>,
    market_type: MarketKind,
    proxy_cfg: Option<crate::proxy::Proxy>,
) -> impl Stream<Item = Event> {
    let stream_scope: Arc<[StreamKind]> = Arc::from(
        streams
            .iter()
            .map(|ticker_info| StreamKind::Trades {
                ticker_info: *ticker_info,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    let args = streams
        .iter()
        .map(|ticker_info| {
            let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();
            serde_json::json!({
                "channel": "trades",
                "instId": symbol_str,
            })
        })
        .collect::<Vec<_>>();

    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": args,
    });

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let ticker_info_map = streams
        .iter()
        .map(|ticker_info| {
            (
                ticker_info.ticker,
                (
                    *ticker_info,
                    QtyNormalization::with_raw_qty_unit(
                        size_in_quote_ccy,
                        *ticker_info,
                        raw_qty_unit_from_market_type(market_type),
                    ),
                ),
            )
        })
        .collect::<FxHashMap<Ticker, (TickerInfo, QtyNormalization)>>();

    let symbol_to_ticker = streams
        .iter()
        .map(|ticker_info| {
            let (symbol_str, _) = ticker_info.ticker.to_full_symbol_and_type();
            (symbol_str, ticker_info.ticker)
        })
        .collect::<FxHashMap<String, Ticker>>();

    let adapter = TradeAdapter {
        symbol_to_ticker,
        buffer: TradeBuffer::new(ticker_info_map),
        subscribe_message,
        proxy_cfg,
    };
    let session = WsSession::with_text_ping(OKX_PING_PAYLOAD, Some(b"pong"), stream_scope);

    session.run(adapter)
}

struct DepthAdapter {
    stream: StreamKind,
    ticker_info: TickerInfo,
    qty_norm: QtyNormalization,
    orderbook: LocalDepthCache,
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
}

impl WsAdapter for DepthAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        connect_and_subscribe(&self.subscribe_message, "public", self.proxy_cfg.as_ref()).await
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(StreamData::Depth(de_depth, data_type, time)) = feed_de(payload) {
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

    let stream_scope = Arc::from(vec![stream].into_boxed_slice());

    let ticker = ticker_info.ticker;

    let (symbol_str, market_type) = ticker.to_full_symbol_and_type();

    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": [
            { "channel": "books",  "instId": symbol_str },
        ],
    });

    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;
    let qty_norm = QtyNormalization::with_raw_qty_unit(
        size_in_quote_ccy,
        ticker_info,
        raw_qty_unit_from_market_type(market_type),
    );

    let adapter = DepthAdapter {
        stream,
        ticker_info,
        qty_norm,
        orderbook: LocalDepthCache::default(),
        subscribe_message,
        proxy_cfg,
    };

    WsSession::with_text_ping(OKX_PING_PAYLOAD, Some(b"pong"), stream_scope).run(adapter)
}

struct KlineAdapter {
    subscribe_message: serde_json::Value,
    proxy_cfg: Option<crate::proxy::Proxy>,
    lookup: Arc<HashMap<(String, String), (TickerInfo, Timeframe)>>,
    size_in_quote_ccy: bool,
    market_type: MarketKind,
}

impl WsAdapter for KlineAdapter {
    async fn connect(&mut self) -> Result<WsTransport, String> {
        connect_and_subscribe(&self.subscribe_message, "business", self.proxy_cfg.as_ref()).await
    }

    async fn on_connected(&mut self, _output: &mut mpsc::Sender<Event>) {}

    async fn on_text(
        &mut self,
        payload: &[u8],
        output: &mut mpsc::Sender<Event>,
    ) -> Result<(), String> {
        if let Ok(v) = serde_json::from_slice::<Value>(payload) {
            let channel = v["arg"]["channel"].as_str().unwrap_or("");
            if !channel.starts_with("candle") {
                return Ok(());
            }

            let inst = match v["arg"]["instId"].as_str() {
                Some(s) => s,
                None => return Ok(()),
            };
            let (ticker_info, timeframe) =
                match self.lookup.get(&(channel.to_string(), inst.to_string())) {
                    Some(t) => *t,
                    None => return Ok(()),
                };
            let qty_norm = QtyNormalization::with_raw_qty_unit(
                self.size_in_quote_ccy,
                ticker_info,
                raw_qty_unit_from_market_type(self.market_type),
            );

            if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
                for row in data {
                    let time = row.get(0).and_then(serde_util::value_as_u64);
                    let open = row.get(1).and_then(serde_util::value_as_f32);
                    let high = row.get(2).and_then(serde_util::value_as_f32);
                    let low = row.get(3).and_then(serde_util::value_as_f32);
                    let close = row.get(4).and_then(serde_util::value_as_f32);
                    let volume = row.get(5).and_then(serde_util::value_as_f32);

                    let (ts, open, high, low, close) = match (time, open, high, low, close) {
                        (Some(ts), Some(open), Some(high), Some(low), Some(close)) => {
                            (ts, open, high, low, close)
                        }
                        _ => continue,
                    };

                    let volume_in_display = if let Some(vq) = volume {
                        qty_norm.normalize_qty(vq, close)
                    } else {
                        qty_norm.normalize_qty(0.0, close)
                    };

                    let kline = Kline::new(
                        ts,
                        open,
                        high,
                        low,
                        close,
                        Volume::TotalOnly(volume_in_display),
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

    let mut args = Vec::with_capacity(streams.len());
    let mut lookup = HashMap::new();
    for (ticker_info, timeframe) in &streams {
        let ticker = ticker_info.ticker;

        if let Some(bar) = timeframe_to_okx_bar(*timeframe) {
            let (symbol, _mt) = ticker.to_full_symbol_and_type();
            let channel = format!("candle{bar}");
            args.push(serde_json::json!({
                "channel": channel,
                "instId": symbol,
            }));
            lookup.insert((channel, symbol), (*ticker_info, *timeframe));
        }
    }

    let subscribe_message = serde_json::json!({
        "op": "subscribe",
        "args": args,
    });

    let lookup = Arc::new(lookup);
    let size_in_quote_ccy = volume_size_unit() == SizeUnit::Quote;

    let adapter = KlineAdapter {
        subscribe_message,
        proxy_cfg,
        lookup,
        size_in_quote_ccy,
        market_type,
    };

    WsSession::with_text_ping(OKX_PING_PAYLOAD, Some(b"pong"), stream_scope).run(adapter)
}
