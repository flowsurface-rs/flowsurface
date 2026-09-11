use crate::scene::Color;
use crate::types::Value;
use serde::{Deserialize, Serialize};
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

/// Stable metadata shared by all bars for one instrument evaluation.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InstrumentContext {
    pub min_tick: Option<f64>,
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
    ChartHl2,
    ChartHlc3,
    ChartOhlc4,
    ChartHlcc4,
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
    InstrumentMinTick,
    OpenInterest,
    Chart,
}

/// Standard chart-derived numeric sources exposed by `input.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSeriesSource {
    Open,
    High,
    Low,
    Close,
    Hl2,
    Hlc3,
    Ohlc4,
    Hlcc4,
}

impl BaseSeriesSource {
    pub const ALL: [Self; 8] = [
        Self::Open,
        Self::High,
        Self::Low,
        Self::Close,
        Self::Hl2,
        Self::Hlc3,
        Self::Ohlc4,
        Self::Hlcc4,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::High => "High",
            Self::Low => "Low",
            Self::Close => "Close",
            Self::Hl2 => "(H + L)/2",
            Self::Hlc3 => "(H + L + C)/3",
            Self::Ohlc4 => "(O + H + L + C)/4",
            Self::Hlcc4 => "(H + L + C + C)/4",
        }
    }

    pub fn value(self, frame: &IndicatorFrame) -> Option<f64> {
        let finite = |value: Option<f64>| value.filter(|value| value.is_finite());
        let average = |values: &[Option<f64>]| {
            let sum = values
                .iter()
                .try_fold(0.0, |sum, value| Some(sum + finite(*value)?))?;
            Some(sum / values.len() as f64).filter(|value| value.is_finite())
        };

        match self {
            Self::Open => finite(frame.chart.open),
            Self::High => finite(frame.chart.high),
            Self::Low => finite(frame.chart.low),
            Self::Close => finite(frame.chart.close),
            Self::Hl2 => average(&[frame.chart.high, frame.chart.low]),
            Self::Hlc3 => average(&[frame.chart.high, frame.chart.low, frame.chart.close]),
            Self::Ohlc4 => average(&[
                frame.chart.open,
                frame.chart.high,
                frame.chart.low,
                frame.chart.close,
            ]),
            Self::Hlcc4 => average(&[
                frame.chart.high,
                frame.chart.low,
                frame.chart.close,
                frame.chart.close,
            ]),
        }
    }
}

impl FrameField {
    pub const fn source_input_label(self) -> Option<&'static str> {
        match self {
            Self::ChartOpen => Some(BaseSeriesSource::Open.label()),
            Self::ChartHigh => Some(BaseSeriesSource::High.label()),
            Self::ChartLow => Some(BaseSeriesSource::Low.label()),
            Self::ChartClose => Some(BaseSeriesSource::Close.label()),
            Self::ChartHl2 => Some(BaseSeriesSource::Hl2.label()),
            Self::ChartHlc3 => Some(BaseSeriesSource::Hlc3.label()),
            Self::ChartOhlc4 => Some(BaseSeriesSource::Ohlc4.label()),
            Self::ChartHlcc4 => Some(BaseSeriesSource::Hlcc4.label()),
            Self::ChartVolume => Some("Volume"),
            Self::OpenInterest => Some("Open interest"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SeriesInputValue {
    Default,
    Base(BaseSeriesSource),
    Value(Option<f64>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorFrame {
    pub index: usize,
    pub time: i64,
    pub basis: TimeOrTick,
    pub instrument: InstrumentContext,
    pub chart: ChartFrame,
    pub open_interest: Option<f64>,
    pub series_inputs: Arc<[SeriesInputValue]>,
}

impl IndicatorFrame {
    pub(crate) fn primary_value(&self, field: FrameField) -> Value {
        match field {
            FrameField::ChartTime => Value::Int(self.time),
            FrameField::ChartOpen => optional_float(self.chart.open),
            FrameField::ChartHigh => optional_float(self.chart.high),
            FrameField::ChartLow => optional_float(self.chart.low),
            FrameField::ChartClose => optional_float(self.chart.close),
            FrameField::ChartHl2 => BaseSeriesSource::Hl2
                .value(self)
                .map_or(Value::Na, Value::Float),
            FrameField::ChartHlc3 => BaseSeriesSource::Hlc3
                .value(self)
                .map_or(Value::Na, Value::Float),
            FrameField::ChartOhlc4 => BaseSeriesSource::Ohlc4
                .value(self)
                .map_or(Value::Na, Value::Float),
            FrameField::ChartHlcc4 => BaseSeriesSource::Hlcc4
                .value(self)
                .map_or(Value::Na, Value::Float),
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
            FrameField::InstrumentMinTick => optional_float(self.instrument.min_tick),
            FrameField::OpenInterest => optional_float(self.open_interest),
            FrameField::Chart => self.chart.to_value(),
        }
    }

    pub fn field(&self, field: FrameField) -> Value {
        self.primary_value(field)
    }

    pub fn series_input(&self, slot: usize) -> SeriesInputValue {
        self.series_inputs
            .get(slot)
            .cloned()
            .unwrap_or(SeriesInputValue::Default)
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

#[derive(Debug, Clone, PartialEq)]
pub struct RegionOutput {
    pub id: String,
    pub key: i64,
    pub left_index: usize,
    pub right_index: usize,
    pub upper: f64,
    pub lower: f64,
    pub border_color: Option<Color>,
    pub fill_color: Option<Color>,
    pub border_color_slot: Option<String>,
    pub fill_color_slot: Option<String>,
    pub extend_right: bool,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegionUpdate {
    Upsert(RegionOutput),
    Remove { id: String, key: i64 },
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
    pub regions: Arc<[RegionOutput]>,
    pub region_updates: Arc<[RegionUpdate]>,
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
