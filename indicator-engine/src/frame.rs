use crate::scene::Color;
use crate::types::Value;
use std::sync::Arc;

/// The two bar bases understood by the live indicator runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimeOrTick {
    #[default]
    Time,
    Tick,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TradeAggregate {
    pub buy_qty: f64,
    pub sell_qty: f64,
    pub buy_count: u64,
    pub sell_count: u64,
}

impl TradeAggregate {
    pub fn trade_count(self) -> u64 {
        self.buy_count.saturating_add(self.sell_count)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartFrame {
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub directional_volume_available: bool,
    pub buy_qty: f64,
    pub sell_qty: f64,
    pub trade: Option<TradeAggregate>,
}

impl ChartFrame {
    pub fn to_value(&self) -> Value {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("open".to_owned(), optional_float(self.open));
        fields.insert("high".to_owned(), optional_float(self.high));
        fields.insert("low".to_owned(), optional_float(self.low));
        fields.insert("close".to_owned(), optional_float(self.close));
        fields.insert("volume".to_owned(), optional_float(self.volume));
        fields.insert(
            "directional_volume_available".to_owned(),
            Value::Bool(self.directional_volume_available),
        );
        fields.insert(
            "buy_qty".to_owned(),
            directional_float(self.directional_volume_available, self.buy_qty),
        );
        fields.insert(
            "sell_qty".to_owned(),
            directional_float(self.directional_volume_available, self.sell_qty),
        );
        fields.insert("trade".to_owned(), trade_value(self.trade));
        Value::Record(fields)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameField {
    ChartTime,
    ChartOpen,
    ChartHigh,
    ChartLow,
    ChartClose,
    ChartVolume,
    ChartDirectionalVolumeAvailable,
    ChartBuyQty,
    ChartSellQty,
    ChartTrade,
    ChartTradeAvailable,
    ChartTradeBuyQty,
    ChartTradeSellQty,
    ChartTradeBuyCount,
    ChartTradeSellCount,
    ChartTradeCount,
    OpenInterest,
    Chart,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorFrame {
    pub index: usize,
    pub time: i64,
    pub basis: TimeOrTick,
    pub chart: ChartFrame,
    pub open_interest: Option<f64>,
}

impl IndicatorFrame {
    pub(crate) fn primary_value(&self, field: FrameField) -> Value {
        match field {
            FrameField::ChartTime => Value::Int(self.time),
            FrameField::ChartOpen => optional_float(self.chart.open),
            FrameField::ChartHigh => optional_float(self.chart.high),
            FrameField::ChartLow => optional_float(self.chart.low),
            FrameField::ChartClose => optional_float(self.chart.close),
            FrameField::ChartVolume => optional_float(self.chart.volume),
            FrameField::ChartDirectionalVolumeAvailable => {
                Value::Bool(self.chart.directional_volume_available)
            }
            FrameField::ChartBuyQty => {
                directional_float(self.chart.directional_volume_available, self.chart.buy_qty)
            }
            FrameField::ChartSellQty => {
                directional_float(self.chart.directional_volume_available, self.chart.sell_qty)
            }
            FrameField::ChartTrade => trade_value(self.chart.trade),
            FrameField::ChartTradeAvailable => Value::Bool(self.chart.trade.is_some()),
            FrameField::ChartTradeBuyQty => trade_float(self.chart.trade, |trade| trade.buy_qty),
            FrameField::ChartTradeSellQty => trade_float(self.chart.trade, |trade| trade.sell_qty),
            FrameField::ChartTradeBuyCount => trade_int(self.chart.trade, |trade| trade.buy_count),
            FrameField::ChartTradeSellCount => {
                trade_int(self.chart.trade, |trade| trade.sell_count)
            }
            FrameField::ChartTradeCount => trade_int(self.chart.trade, TradeAggregate::trade_count),
            FrameField::OpenInterest => optional_float(self.open_interest),
            FrameField::Chart => self.chart.to_value(),
        }
    }

    pub fn field(&self, field: FrameField) -> Value {
        self.primary_value(field)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotOutput {
    pub id: String,
    pub value: Value,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerOutput {
    pub id: String,
    pub value: Option<f64>,
    pub color: Option<Color>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SegmentOutput {
    pub id: String,
    pub start_index: usize,
    pub start_value: Option<f64>,
    pub end_index: usize,
    pub end_value: Option<f64>,
    pub color: Option<Color>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HorizontalLevelOutput {
    pub id: String,
    pub value: Option<f64>,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OhlcOutput {
    pub id: String,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeOutput {
    pub id: String,
    pub upper: Option<f64>,
    pub lower: Option<f64>,
    pub color: Option<Color>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameOutput {
    pub index: usize,
    pub time: i64,
    pub plots: Arc<[PlotOutput]>,
    pub markers: Arc<[MarkerOutput]>,
    pub segments: Arc<[SegmentOutput]>,
    pub levels: Arc<[HorizontalLevelOutput]>,
    pub ohlc: Arc<[OhlcOutput]>,
    pub ranges: Arc<[RangeOutput]>,
}

fn optional_float(value: Option<f64>) -> Value {
    value.map_or(Value::Na, Value::Float)
}

fn directional_float(available: bool, value: f64) -> Value {
    if available {
        Value::Float(value)
    } else {
        Value::Na
    }
}

fn trade_float(trade: Option<TradeAggregate>, value: impl FnOnce(TradeAggregate) -> f64) -> Value {
    trade.map_or(Value::Na, |trade| Value::Float(value(trade)))
}

fn trade_int(trade: Option<TradeAggregate>, value: impl FnOnce(TradeAggregate) -> u64) -> Value {
    trade.map_or(Value::Na, |trade| Value::Int(value(trade) as i64))
}

fn trade_value(trade: Option<TradeAggregate>) -> Value {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert("available".to_owned(), Value::Bool(trade.is_some()));
    fields.insert(
        "buy_qty".to_owned(),
        trade_float(trade, |trade| trade.buy_qty),
    );
    fields.insert(
        "sell_qty".to_owned(),
        trade_float(trade, |trade| trade.sell_qty),
    );
    fields.insert(
        "buy_count".to_owned(),
        trade_int(trade, |trade| trade.buy_count),
    );
    fields.insert(
        "sell_count".to_owned(),
        trade_int(trade, |trade| trade.sell_count),
    );
    fields.insert(
        "count".to_owned(),
        trade_int(trade, TradeAggregate::trade_count),
    );
    Value::Record(fields)
}
