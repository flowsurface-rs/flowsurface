use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::mem::size_of;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Tuple(Vec<Value>),
    Array(Vec<Value>),
    Record(BTreeMap<String, Value>),
    Na,
}

impl Value {
    pub(crate) fn resource_usage(&self) -> (usize, usize) {
        match self {
            Self::String(value) => (0, size_of::<String>().saturating_add(value.capacity())),
            Self::Tuple(values) | Self::Array(values) => values.iter().fold(
                (
                    values.len(),
                    size_of::<Vec<Value>>()
                        .saturating_add(values.capacity().saturating_mul(size_of::<Value>())),
                ),
                |(elements, bytes), value| {
                    let (nested_elements, nested_bytes) = value.resource_usage();
                    (
                        elements.saturating_add(nested_elements),
                        bytes.saturating_add(nested_bytes),
                    )
                },
            ),
            Self::Record(values) => values.iter().fold(
                (
                    values.len(),
                    size_of::<BTreeMap<String, Value>>()
                        .saturating_add(values.len().saturating_mul(size_of::<(String, Value)>())),
                ),
                |(elements, bytes), (key, value)| {
                    let (nested_elements, nested_bytes) = value.resource_usage();
                    (
                        elements.saturating_add(nested_elements),
                        bytes
                            .saturating_add(size_of::<String>())
                            .saturating_add(key.capacity())
                            .saturating_add(nested_bytes),
                    )
                },
            ),
            _ => (0, 0),
        }
    }

    pub(crate) fn has_string_over(&self, limit: usize) -> bool {
        match self {
            Self::String(value) => value.capacity() > limit,
            Self::Tuple(values) | Self::Array(values) => {
                values.iter().any(|value| value.has_string_over(limit))
            }
            Self::Record(values) => values
                .iter()
                .any(|(key, value)| key.capacity() > limit || value.has_string_over(limit)),
            _ => false,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(value) if value.is_finite() => Some(*value),
            Self::Int(value) => Some(*value as f64),
            Self::Na => None,
            _ => None,
        }
    }

    pub(crate) fn normalize_nonfinite(self) -> Self {
        match self {
            Self::Float(value) if !value.is_finite() => Self::Na,
            value => value,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            Self::Float(value) if value.is_finite() => Some(*value as i64),
            _ => None,
        }
    }

    pub fn is_truthy(&self) -> bool {
        matches!(self, Self::Bool(true))
    }

    pub fn is_na(&self) -> bool {
        matches!(self, Self::Na)
    }

    pub fn equals(&self, other: &Self) -> Option<bool> {
        if self.is_na() || other.is_na() {
            return None;
        }
        Some(match (self, other) {
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Float(left), Self::Float(right)) => left == right,
            (Self::Int(left), Self::Float(right)) | (Self::Float(right), Self::Int(left)) => {
                (*left as f64) == *right
            }
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Color(left), Self::Color(right)) => left == right,
            (Self::Tuple(left), Self::Tuple(right)) | (Self::Array(left), Self::Array(right)) => {
                left == right
            }
            (Self::Record(left), Self::Record(right)) => left == right,
            _ => false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    Int,
    Float,
    Bool,
    String,
    Color,
    Time,
    Tuple,
    Array,
    Record,
    Any,
    Void,
}

pub type InputType = ValueType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Qualifier {
    Simple,
    Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Type {
    pub value: ValueType,
    pub qualifier: Qualifier,
}

impl Type {
    pub const fn simple(value: ValueType) -> Self {
        Self {
            value,
            qualifier: Qualifier::Simple,
        }
    }

    pub const fn series(value: ValueType) -> Self {
        Self {
            value,
            qualifier: Qualifier::Series,
        }
    }

    pub fn accepts(self, actual: Self) -> bool {
        let value_matches = self.value == ValueType::Any
            || actual.value == ValueType::Any
            || self.value == actual.value;
        let qualifier_matches = self.value == ValueType::Any
            || actual.value == ValueType::Any
            || self.qualifier == Qualifier::Series
            || self.qualifier == actual.qualifier;
        value_matches && qualifier_matches
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IndicatorId(String);

impl IndicatorId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() {
            return Err("indicator IDs must be a non-empty namespace.name".to_owned());
        }
        if !value.is_ascii() || !value.contains('.') {
            return Err(
                "indicator IDs must use lowercase namespace.name segments separated by '.'"
                    .to_owned(),
            );
        }
        if value.split('.').any(|segment| {
            segment.is_empty()
                || segment.starts_with('-')
                || segment.ends_with('-')
                || segment.bytes().any(|character| {
                    !(character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || character == b'-')
                })
        }) {
            return Err(
                "indicator IDs must use lowercase letters, digits, and internal '-' characters"
                    .to_owned(),
            );
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn short_name(&self) -> &str {
        self.0
            .rsplit_once('.')
            .map_or(self.0.as_str(), |(_, short_name)| short_name)
    }

    pub fn source_file_name(&self) -> String {
        format!("{}.ind", self.short_name())
    }
}

impl Display for IndicatorId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Time(i64),
}

impl InputValue {
    pub fn into_value(self) -> Value {
        match self {
            Self::Int(value) => Value::Int(value),
            Self::Float(value) => Value::Float(value),
            Self::Bool(value) => Value::Bool(value),
            Self::String(value) => Value::String(value),
            Self::Color(value) => Value::Color(value),
            Self::Time(value) => Value::Int(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputDescriptor {
    pub id: String,
    pub label: String,
    pub value_type: ValueType,
    pub default: InputValue,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub group: Option<String>,
    pub inline: Option<String>,
    pub tooltip: Option<String>,
    pub options: Vec<InputOption>,
}

impl InputDescriptor {
    pub fn accepts(&self, value: &InputValue) -> bool {
        let type_matches = matches!(
            (self.value_type, value),
            (ValueType::Int, InputValue::Int(_))
                | (ValueType::Float, InputValue::Float(_))
                | (ValueType::Float, InputValue::Int(_))
                | (ValueType::Bool, InputValue::Bool(_))
                | (ValueType::String, InputValue::String(_))
                | (ValueType::Color, InputValue::Color(_))
                | (ValueType::Time, InputValue::Time(_))
        );
        if !type_matches {
            return false;
        }

        if let InputValue::Float(value) = value
            && !value.is_finite()
        {
            return false;
        }
        if let InputValue::String(value) = value
            && !self.options.is_empty()
            && !self.options.iter().any(|option| option.id == *value)
        {
            return false;
        }

        let Some(number) = (match value {
            InputValue::Int(value) => Some(*value as f64),
            InputValue::Float(value) => Some(*value),
            _ => None,
        }) else {
            return true;
        };
        self.min.is_none_or(|minimum| number >= minimum)
            && self.max.is_none_or(|maximum| number <= maximum)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesInputDescriptor {
    pub id: String,
    pub label: String,
    pub default_field: crate::frame::FrameField,
    pub allow_indicator: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorDescriptor {
    pub id: IndicatorId,
    pub title: String,
    pub sourceable: bool,
    pub optional_trades: bool,
    pub max_bars_back: usize,
    pub inputs: Vec<InputDescriptor>,
    pub series_inputs: Vec<SeriesInputDescriptor>,
    pub plots: Vec<crate::scene::PlotDescriptor>,
    pub presentation: crate::scene::IndicatorPresentation,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndicatorConfig {
    pub values: BTreeMap<String, InputValue>,
}

impl IndicatorConfig {
    pub fn with(mut self, id: impl Into<String>, value: InputValue) -> Self {
        self.values.insert(id.into(), value);
        self
    }

    pub fn value<'a>(&'a self, descriptor: &'a InputDescriptor) -> InputValue {
        self.values
            .get(&descriptor.id)
            .cloned()
            .unwrap_or_else(|| descriptor.default.clone())
    }
}
