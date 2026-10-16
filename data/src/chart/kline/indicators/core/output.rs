use crate::chart::kline::IndicatorInstanceId;
use indicator_engine::{OutputPrecision, ReadoutBehavior};

/// The value domain supported by a declared indicator output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputValueType {
    Number,
    Boolean,
    Ohlc,
    Range,
}

/// A stable, definition-local output channel declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputChannelSpec {
    pub id: String,
    pub label: String,
    pub value_type: OutputValueType,
    pub precision: OutputPrecision,
    pub line_width: f32,
    pub readout: ReadoutBehavior,
    pub style_slot: Option<String>,
    pub offset_bars: i32,
    pub style_by: Option<String>,
    pub independent_scale: bool,
}

/// A value emitted by an indicator for one chart bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputValue {
    Number(f64),
    Boolean(bool),
}

impl OutputValue {
    pub const fn value_type(self) -> OutputValueType {
        match self {
            Self::Number(_) => OutputValueType::Number,
            Self::Boolean(_) => OutputValueType::Boolean,
        }
    }

    pub fn as_f64(self) -> f64 {
        match self {
            Self::Number(value) => value,
            Self::Boolean(value) => f64::from(value),
        }
    }

    pub fn as_number(self) -> Option<f64> {
        match self {
            Self::Number(value) => value.is_finite().then_some(value),
            Self::Boolean(_) => None,
        }
    }

    pub fn as_boolean(self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(value),
            Self::Number(_) => None,
        }
    }
}

/// A reference to one named output of one indicator instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IndicatorOutputRef {
    pub instance_id: IndicatorInstanceId,
    pub channel_id: String,
}

impl IndicatorOutputRef {
    pub fn new(instance_id: IndicatorInstanceId, channel_id: impl Into<String>) -> Self {
        Self {
            instance_id,
            channel_id: channel_id.into(),
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.channel_id.trim().is_empty()
    }
}
