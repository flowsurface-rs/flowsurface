use crate::hir::{BinaryOperator, UnaryOperator};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::mem::size_of;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Tuple(Vec<Value>),
    Array(Arc<ArrayValue>),
    Record(BTreeMap<String, Value>),
    Na,
}

#[derive(Debug)]
pub struct ArrayValue {
    values: Vec<Value>,
    collection_elements: usize,
    heap_bytes: usize,
    max_string_capacity: usize,
    nesting_depth: usize,
}

impl ArrayValue {
    pub fn new(values: Vec<Value>) -> Self {
        let nesting_depth = values
            .iter()
            .map(Value::nesting_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let (nested_elements, nested_bytes, max_string_capacity) = values.iter().fold(
            (0usize, 0usize, 0usize),
            |(elements, bytes, max_string_capacity), value| {
                let (value_elements, value_bytes) = value.resource_usage();
                (
                    elements.saturating_add(value_elements),
                    bytes.saturating_add(value_bytes),
                    max_string_capacity.max(value.max_string_capacity()),
                )
            },
        );
        Self {
            collection_elements: values.len().saturating_add(nested_elements),
            heap_bytes: Self::container_heap_bytes(values.capacity()).saturating_add(nested_bytes),
            max_string_capacity,
            nesting_depth,
            values,
        }
    }

    pub fn as_slice(&self) -> &[Value] {
        &self.values
    }

    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(crate) fn push(&mut self, value: Value) {
        self.nesting_depth = self
            .nesting_depth
            .max(value.nesting_depth().saturating_add(1));
        let (value_elements, value_bytes) = value.resource_usage();
        let value_max_string_capacity = value.max_string_capacity();
        let previous_capacity = self.values.capacity();
        self.values.push(value);
        let capacity_growth = self
            .values
            .capacity()
            .saturating_sub(previous_capacity)
            .saturating_mul(size_of::<Value>());
        self.collection_elements = self
            .collection_elements
            .saturating_add(1)
            .saturating_add(value_elements);
        self.heap_bytes = self
            .heap_bytes
            .saturating_add(value_bytes)
            .saturating_add(capacity_growth);
        self.max_string_capacity = self.max_string_capacity.max(value_max_string_capacity);
    }

    pub(crate) fn projected_push_usage(&self, value: &Value) -> (usize, usize) {
        let (value_elements, value_bytes) = value.resource_usage();
        (
            self.collection_elements
                .saturating_add(1)
                .saturating_add(value_elements),
            self.heap_bytes
                .saturating_add(size_of::<Value>())
                .saturating_add(value_bytes),
        )
    }

    pub(crate) fn resource_usage(&self) -> (usize, usize) {
        (self.collection_elements, self.heap_bytes)
    }

    fn container_heap_bytes(capacity: usize) -> usize {
        size_of::<Vec<Value>>().saturating_add(capacity.saturating_mul(size_of::<Value>()))
    }
}

impl Clone for ArrayValue {
    fn clone(&self) -> Self {
        let values = self.values.clone();
        let old_container_bytes = Self::container_heap_bytes(self.values.capacity());
        let nested_bytes = self.heap_bytes.saturating_sub(old_container_bytes);
        Self {
            collection_elements: self.collection_elements,
            heap_bytes: Self::container_heap_bytes(values.capacity()).saturating_add(nested_bytes),
            max_string_capacity: self.max_string_capacity,
            nesting_depth: self.nesting_depth,
            values,
        }
    }
}

impl PartialEq for ArrayValue {
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl Value {
    pub(crate) fn nesting_depth(&self) -> usize {
        match self {
            Self::Array(values) => values.nesting_depth,
            Self::Tuple(values) => values
                .iter()
                .map(Self::nesting_depth)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            Self::Record(values) => values
                .values()
                .map(Self::nesting_depth)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
            _ => 0,
        }
    }

    pub(crate) fn comparison_work(&self) -> usize {
        match self {
            Self::String(value) => value.len(),
            Self::Tuple(values) => values.iter().fold(values.len(), |work, value| {
                work.saturating_add(value.comparison_work())
            }),
            Self::Array(values) => values.as_slice().iter().fold(values.len(), |work, value| {
                work.saturating_add(value.comparison_work())
            }),
            Self::Record(values) => values.iter().fold(values.len(), |work, (key, value)| {
                work.saturating_add(key.len())
                    .saturating_add(value.comparison_work())
            }),
            _ => 0,
        }
    }
    pub(crate) fn copy_work(&self) -> usize {
        match self {
            Self::String(value) => value.len(),
            Self::Tuple(values) => values.iter().fold(values.len(), |work, value| {
                work.saturating_add(value.copy_work())
            }),
            Self::Record(values) => values.iter().fold(values.len(), |work, (key, value)| {
                work.saturating_add(key.len())
                    .saturating_add(value.copy_work())
            }),
            _ => 0,
        }
    }

    pub(crate) fn scan_work(&self) -> usize {
        match self {
            Self::Tuple(values) => values.iter().fold(1, |work: usize, value| {
                work.saturating_add(value.scan_work())
            }),
            Self::Record(values) => values.values().fold(1, |work: usize, value| {
                work.saturating_add(value.scan_work())
            }),
            _ => 1,
        }
    }
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Int(_) => "integer",
            Self::Float(_) => "number",
            Self::Bool(_) => "boolean",
            Self::String(_) => "string",
            Self::Color(_) => "color",
            Self::Tuple(_) => "tuple",
            Self::Array(_) => "array",
            Self::Record(_) => "record",
            Self::Na => "na",
        }
    }

    pub(crate) fn unary(self, operator: UnaryOperator) -> Option<Self> {
        Some(match (operator, self) {
            (_, Self::Na) => Self::Na,
            (UnaryOperator::Plus, value @ (Self::Int(_) | Self::Float(_))) => {
                value.normalize_nonfinite()
            }
            (UnaryOperator::Minus, Self::Int(value)) => {
                value.checked_neg().map_or(Self::Na, Self::Int)
            }
            (UnaryOperator::Minus, Self::Float(value)) => Self::Float(-value).normalize_nonfinite(),
            (UnaryOperator::Not, Self::Bool(value)) => Self::Bool(!value),
            _ => return None,
        })
    }

    pub(crate) fn numeric_compare(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => Some(left.cmp(right)),
            (Self::Float(left), Self::Float(right)) if left.is_finite() && right.is_finite() => {
                left.partial_cmp(right)
            }
            (Self::Int(integer), Self::Float(float)) => compare_integer_float(*integer, *float),
            (Self::Float(float), Self::Int(integer)) => {
                compare_integer_float(*integer, *float).map(Ordering::reverse)
            }
            _ => None,
        }
    }

    pub(crate) fn binary(self, operator: BinaryOperator, other: Self) -> Self {
        use BinaryOperator::*;
        match operator {
            And | Or => match (self, other) {
                (Self::Bool(left), Self::Bool(right)) => Self::Bool(if operator == And {
                    left && right
                } else {
                    left || right
                }),
                _ => Self::Na,
            },
            Equal | NotEqual => self.equals(&other).map_or(Self::Na, |equal| {
                Self::Bool(if operator == Equal { equal } else { !equal })
            }),
            Less | LessEqual | Greater | GreaterEqual => {
                self.numeric_compare(&other).map_or(Self::Na, |order| {
                    Self::Bool(match operator {
                        Less => order.is_lt(),
                        LessEqual => order.is_le(),
                        Greater => order.is_gt(),
                        GreaterEqual => order.is_ge(),
                        _ => unreachable!(),
                    })
                })
            }
            Add | Subtract | Multiply | Divide | Remainder => {
                if let (Self::Int(left), Self::Int(right)) = (&self, &other)
                    && operator != Divide
                {
                    return match operator {
                        Add => left.checked_add(*right),
                        Subtract => left.checked_sub(*right),
                        Multiply => left.checked_mul(*right),
                        Remainder => left.checked_rem(*right),
                        _ => unreachable!(),
                    }
                    .map_or(Self::Na, Self::Int);
                }
                let (Some(left), Some(right)) = (self.as_float(), other.as_float()) else {
                    return Self::Na;
                };
                let value = match operator {
                    Add => left + right,
                    Subtract => left - right,
                    Multiply => left * right,
                    Divide if right != 0.0 => left / right,
                    Remainder if right != 0.0 => left % right,
                    Divide | Remainder => return Self::Na,
                    _ => unreachable!(),
                };
                Self::Float(value).normalize_nonfinite()
            }
        }
    }

    pub(crate) fn resource_usage(&self) -> (usize, usize) {
        match self {
            Self::String(value) => (0, size_of::<String>().saturating_add(value.capacity())),
            Self::Tuple(values) => collection_resource_usage(values, values.capacity()),
            Self::Array(values) => values.resource_usage(),
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
        self.max_string_capacity() > limit
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
            (Self::Int(_) | Self::Float(_), Self::Int(_) | Self::Float(_)) => {
                return self.numeric_compare(other).map(Ordering::is_eq);
            }
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Color(left), Self::Color(right)) => left == right,
            (Self::Tuple(left), Self::Tuple(right)) => left == right,
            (Self::Array(left), Self::Array(right)) => left == right,
            (Self::Record(left), Self::Record(right)) => left == right,
            _ => false,
        })
    }

    fn max_string_capacity(&self) -> usize {
        match self {
            Self::String(value) => value.capacity(),
            Self::Tuple(values) => values
                .iter()
                .map(Self::max_string_capacity)
                .max()
                .unwrap_or(0),
            Self::Array(values) => values.max_string_capacity,
            Self::Record(values) => values.iter().fold(0usize, |maximum, (key, value)| {
                maximum.max(key.capacity()).max(value.max_string_capacity())
            }),
            _ => 0,
        }
    }
}

fn compare_integer_float(integer: i64, float: f64) -> Option<Ordering> {
    if !float.is_finite() {
        return None;
    }
    if float < i64::MIN as f64 {
        return Some(Ordering::Greater);
    }
    if float >= -(i64::MIN as f64) {
        return Some(Ordering::Less);
    }
    let order = integer.cmp(&(float as i64));
    if order.is_eq() {
        0.0f64.partial_cmp(&float.fract())
    } else {
        Some(order)
    }
}

fn collection_resource_usage(values: &[Value], capacity: usize) -> (usize, usize) {
    values.iter().fold(
        (
            values.len(),
            size_of::<Vec<Value>>().saturating_add(capacity.saturating_mul(size_of::<Value>())),
        ),
        |(elements, bytes), value| {
            let (nested_elements, nested_bytes) = value.resource_usage();
            (
                elements.saturating_add(nested_elements),
                bytes.saturating_add(nested_bytes),
            )
        },
    )
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndicatorAuthor {
    pub name: String,
    pub url: String,
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
    pub visible_when: Option<String>,
    pub required_input: Option<crate::scene::PresentationInputSource>,
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
    pub author: Option<IndicatorAuthor>,
    pub description: Option<String>,
    pub source: Option<String>,
    pub sourceable: bool,
    pub optional_trades: bool,
    pub bases: Option<Vec<crate::frame::TimeOrTick>>,
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
