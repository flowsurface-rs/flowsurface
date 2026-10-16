use crate::hir::{CallSiteId, IntrinsicId};
use crate::types::{ArrayValue, Qualifier, Type, Value, ValueType};
use chrono::{Datelike, TimeZone, Utc};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::mem::size_of;
use std::sync::Arc;

pub type QualifiedName = &'static str;

#[derive(Debug, Clone, PartialEq)]
pub enum IntrinsicState {
    PreviousWhen {
        value: Option<Value>,
    },
    Ema {
        previous: Option<f64>,
    },
    Rma {
        previous: Option<f64>,
    },
    Sma {
        values: VecDeque<Option<f64>>,
    },
    Vwma {
        values: VecDeque<Option<(f64, f64)>>,
    },
    Window {
        maximum: bool,
        values: VecDeque<Option<f64>>,
    },
    Pivot {
        maximum: bool,
        values: VecDeque<Option<f64>>,
    },
    Cum {
        total: f64,
    },
    Stdev {
        values: VecDeque<Option<f64>>,
    },
    Dev {
        values: VecDeque<Option<f64>>,
    },
    ExtremeBars {
        maximum: bool,
        values: VecDeque<Option<(f64, usize)>>,
    },
    TrueRange {
        previous_close: Option<f64>,
    },
    Rsi {
        previous_source: Option<f64>,
        initial_gain_sum: f64,
        initial_loss_sum: f64,
        initial_count: usize,
        average_gain: Option<f64>,
        average_loss: Option<f64>,
    },
    AnchoredVwap {
        anchor: i64,
        price_volume: f64,
        volume: f64,
    },
}

impl IntrinsicState {
    pub(crate) fn copy_work(&self) -> usize {
        match self {
            Self::Sma { values }
            | Self::Window { values, .. }
            | Self::Pivot { values, .. }
            | Self::Stdev { values }
            | Self::Dev { values } => values.len().saturating_add(1),
            Self::Vwma { values } => values.len().saturating_add(1),
            Self::ExtremeBars { values, .. } => values.len().saturating_add(1),
            Self::PreviousWhen { value } => {
                1usize.saturating_add(value.as_ref().map_or(0, Value::copy_work))
            }
            _ => 1,
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        let state_bytes = size_of::<Self>();
        match self {
            Self::Rsi { .. }
            | Self::Ema { .. }
            | Self::Rma { .. }
            | Self::Cum { .. }
            | Self::TrueRange { .. }
            | Self::AnchoredVwap { .. } => state_bytes,
            Self::PreviousWhen { value } => state_bytes.saturating_add(
                value
                    .as_ref()
                    .map(|value| value.resource_usage().1)
                    .unwrap_or(0),
            ),
            Self::Sma { values }
            | Self::Window { values, .. }
            | Self::Pivot { values, .. }
            | Self::Stdev { values }
            | Self::Dev { values } => state_bytes
                .saturating_add(values.capacity().saturating_mul(size_of::<Option<f64>>())),
            Self::Vwma { values } => state_bytes.saturating_add(
                values
                    .capacity()
                    .saturating_mul(size_of::<Option<(f64, f64)>>()),
            ),
            Self::ExtremeBars { values, .. } => state_bytes.saturating_add(
                values
                    .capacity()
                    .saturating_mul(size_of::<Option<(f64, usize)>>()),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntrinsicSignature {
    pub arguments: Vec<ArgumentSignature>,
    pub result: Type,
    pub variadic: bool,
    pub min_arguments: usize,
    pub(crate) warmup: Option<WarmupSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WarmupSpec {
    Window {
        argument: usize,
        preceding: usize,
    },
    Centered {
        left_argument: usize,
        right_argument: usize,
    },
}

impl IntrinsicSignature {
    pub(crate) fn at_least(mut self, arguments: usize) -> Self {
        self.min_arguments = arguments;
        self
    }

    pub(crate) fn windowed(mut self, argument: usize) -> Self {
        self.warmup = Some(WarmupSpec::Window {
            argument,
            preceding: 0,
        });
        self
    }

    pub(crate) fn windowed_with_history(mut self, argument: usize, preceding: usize) -> Self {
        self.warmup = Some(WarmupSpec::Window {
            argument,
            preceding,
        });
        self
    }

    pub(crate) fn centered(mut self, left: usize, right: usize) -> Self {
        self.warmup = Some(WarmupSpec::Centered {
            left_argument: left,
            right_argument: right,
        });
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentSignature {
    pub name: &'static str,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntrinsicEffects {
    pub stateful: bool,
}

impl IntrinsicEffects {
    pub const PURE: Self = Self { stateful: false };

    pub const STATEFUL: Self = Self { stateful: true };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrinsicError {
    InvalidArguments(&'static str),
    InvalidState(&'static str),
    LimitExceeded(&'static str),
    ExecutionLimitExceeded,
}

impl Display for IntrinsicError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArguments(message) | Self::InvalidState(message) => {
                formatter.write_str(message)
            }
            Self::LimitExceeded(resource) => {
                write!(formatter, "{resource} exceeds the runtime limit")
            }
            Self::ExecutionLimitExceeded => formatter.write_str("execution work limit exceeded"),
        }
    }
}

impl std::error::Error for IntrinsicError {}

pub struct IntrinsicEvalContext<'a> {
    pub bar_index: usize,
    pub bar_time: i64,
    pub max_heap_bytes: usize,
    pub max_window_length: usize,
    pub fixed_window: bool,
    pub max_collection_elements: usize,
    pub states: &'a mut BTreeMap<CallSiteId, IntrinsicState>,
    pub executed_work: &'a mut usize,
    pub max_work: usize,
}

impl IntrinsicEvalContext<'_> {
    pub(crate) fn charge(&mut self, work: usize) -> Result<(), IntrinsicError> {
        charge_intrinsic_work(self.executed_work, self.max_work, work)
    }
}

fn charge_intrinsic_work(
    used: &mut usize,
    limit: usize,
    work: usize,
) -> Result<(), IntrinsicError> {
    if crate::runtime::charge_work(used, limit, work) {
        Ok(())
    } else {
        Err(IntrinsicError::ExecutionLimitExceeded)
    }
}

pub type IntrinsicEvaluator = for<'a> fn(
    &mut IntrinsicEvalContext<'a>,
    CallSiteId,
    &[Value],
) -> Result<Value, IntrinsicError>;

#[derive(Clone)]
pub struct IntrinsicDefinition {
    pub name: QualifiedName,
    pub signature: IntrinsicSignature,
    pub effects: IntrinsicEffects,
    pub evaluator: IntrinsicEvaluator,
}

impl std::fmt::Debug for IntrinsicDefinition {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntrinsicDefinition")
            .field("name", &self.name)
            .field("signature", &self.signature)
            .field("effects", &self.effects)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct IntrinsicRegistry {
    definitions: Vec<IntrinsicDefinition>,
    ids: BTreeMap<&'static str, IntrinsicId>,
}

impl IntrinsicRegistry {
    pub fn standard() -> Self {
        let mut registry = Self {
            definitions: Vec::new(),
            ids: BTreeMap::new(),
        };
        registry.register("math.abs", math_abs(), IntrinsicEffects::PURE, eval_abs);
        registry.register(
            "math.max",
            math_min_max(true),
            IntrinsicEffects::PURE,
            eval_max,
        );
        registry.register(
            "math.min",
            math_min_max(true),
            IntrinsicEffects::PURE,
            eval_min,
        );
        registry.register("ta.sma", ta_average(), IntrinsicEffects::STATEFUL, eval_sma);
        registry.register("ta.ema", ta_average(), IntrinsicEffects::STATEFUL, eval_ema);
        registry.register("ta.rma", ta_average(), IntrinsicEffects::STATEFUL, eval_rma);
        registry.register(
            "ta.rsi",
            ta_rsi_signature(),
            IntrinsicEffects::STATEFUL,
            eval_rsi,
        );
        registry.register(
            "ta.previous_when",
            ta_previous_when_signature(),
            IntrinsicEffects::STATEFUL,
            eval_previous_when,
        );
        registry.register("ta.vwma", ta_vwma(), IntrinsicEffects::STATEFUL, eval_vwma);
        registry.register(
            "ta.highest",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            eval_highest,
        );
        registry.register(
            "ta.lowest",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            eval_lowest,
        );
        registry.register(
            "ta.pivothigh",
            ta_pivot(),
            IntrinsicEffects::STATEFUL,
            eval_pivothigh,
        );
        registry.register(
            "ta.pivotlow",
            ta_pivot(),
            IntrinsicEffects::STATEFUL,
            eval_pivotlow,
        );
        registry.register("ta.cum", ta_cum(), IntrinsicEffects::STATEFUL, eval_cum);
        registry.register(
            "ta.stdev",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            eval_stdev,
        );
        registry.register("ta.dev", ta_window(), IntrinsicEffects::STATEFUL, eval_dev);
        registry.register(
            "ta.highestbars",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            eval_highest_bars,
        );
        registry.register(
            "ta.lowestbars",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            eval_lowest_bars,
        );
        registry.register(
            "ta.tr",
            ta_tr_signature(),
            IntrinsicEffects::STATEFUL,
            eval_true_range,
        );
        registry.register(
            "ta.avwap",
            ta_avwap_signature(),
            IntrinsicEffects::STATEFUL,
            eval_anchored_vwap,
        );
        registry.register("nz", nz_signature(), IntrinsicEffects::PURE, eval_nz);
        registry.register(
            "na",
            signature(
                vec![ArgumentSignature {
                    name: "value",
                    ty: Type::series(ValueType::Any),
                }],
                Type::series(ValueType::Bool),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_na,
        );
        registry.register(
            "coalesce",
            coalesce_signature(),
            IntrinsicEffects::PURE,
            eval_coalesce,
        );
        registry.register(
            "select",
            select_signature(),
            IntrinsicEffects::PURE,
            eval_select,
        );
        registry.register(
            "str.tostring",
            signature(
                vec![ArgumentSignature {
                    name: "value",
                    ty: Type::series(ValueType::Any),
                }],
                Type::series(ValueType::String),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_tostring,
        );
        registry.register(
            "color.rgb",
            color_rgb_signature(),
            IntrinsicEffects::PURE,
            eval_color_rgb,
        );
        registry.register(
            "array.new",
            signature(Vec::new(), Type::simple(ValueType::Array), true),
            IntrinsicEffects::PURE,
            eval_array_new,
        );
        registry.register(
            "array.push",
            signature(
                vec![
                    ArgumentSignature {
                        name: "array",
                        ty: Type::series(ValueType::Array),
                    },
                    ArgumentSignature {
                        name: "value",
                        ty: Type::series(ValueType::Any),
                    },
                ],
                Type::series(ValueType::Array),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_array_push,
        );
        registry.register(
            "array.get",
            signature(
                vec![
                    ArgumentSignature {
                        name: "array",
                        ty: Type::series(ValueType::Array),
                    },
                    ArgumentSignature {
                        name: "index",
                        ty: Type::series(ValueType::Int),
                    },
                ],
                Type::series(ValueType::Any),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_array_get,
        );
        registry.register(
            "array.size",
            signature(
                vec![ArgumentSignature {
                    name: "array",
                    ty: Type::series(ValueType::Array),
                }],
                Type::series(ValueType::Int),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_array_size,
        );
        registry.register(
            "record.new",
            signature(Vec::new(), Type::simple(ValueType::Record), false),
            IntrinsicEffects::PURE,
            eval_record_new,
        );
        registry.register(
            "record.set",
            signature(
                vec![
                    ArgumentSignature {
                        name: "record",
                        ty: Type::series(ValueType::Record),
                    },
                    ArgumentSignature {
                        name: "key",
                        ty: Type::series(ValueType::String),
                    },
                    ArgumentSignature {
                        name: "value",
                        ty: Type::series(ValueType::Any),
                    },
                ],
                Type::series(ValueType::Record),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_record_set,
        );
        registry.register(
            "record.get",
            signature(
                vec![
                    ArgumentSignature {
                        name: "record",
                        ty: Type::series(ValueType::Record),
                    },
                    ArgumentSignature {
                        name: "key",
                        ty: Type::series(ValueType::String),
                    },
                ],
                Type::series(ValueType::Any),
                false,
            ),
            IntrinsicEffects::PURE,
            eval_record_get,
        );
        registry
    }

    pub fn get(&self, id: IntrinsicId) -> Option<&IntrinsicDefinition> {
        self.definitions.get(id.0)
    }

    pub fn find(&self, name: &str) -> Option<(IntrinsicId, &IntrinsicDefinition)> {
        let id = *self.ids.get(name)?;
        Some((id, &self.definitions[id.0]))
    }

    pub fn register(
        &mut self,
        name: &'static str,
        signature: IntrinsicSignature,
        effects: IntrinsicEffects,
        evaluator: IntrinsicEvaluator,
    ) -> IntrinsicId {
        let id = IntrinsicId(self.definitions.len());
        self.ids.insert(name, id);
        self.definitions.push(IntrinsicDefinition {
            name,
            signature,
            effects,
            evaluator,
        });
        id
    }
}

fn signature(
    arguments: Vec<ArgumentSignature>,
    result: Type,
    variadic: bool,
) -> IntrinsicSignature {
    IntrinsicSignature {
        arguments,
        result,
        variadic,
        min_arguments: 0,
        warmup: None,
    }
}

fn number(name: &'static str, qualifier: Qualifier) -> ArgumentSignature {
    ArgumentSignature {
        name,
        ty: Type {
            value: ValueType::Float,
            qualifier,
        },
    }
}

fn math_abs() -> IntrinsicSignature {
    signature(
        vec![number("value", Qualifier::Series)],
        Type::series(ValueType::Float),
        false,
    )
}

fn math_min_max(variadic: bool) -> IntrinsicSignature {
    signature(
        vec![
            number("left", Qualifier::Series),
            number("right", Qualifier::Series),
        ],
        Type::series(ValueType::Float),
        variadic,
    )
    .at_least(2)
}

fn ta_average() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            ArgumentSignature {
                name: "length",
                ty: Type::simple(ValueType::Int),
            },
        ],
        Type::series(ValueType::Float),
        false,
    )
    .windowed(1)
}

fn ta_rsi_signature() -> IntrinsicSignature {
    ta_average().windowed_with_history(1, 1)
}

fn ta_previous_when_signature() -> IntrinsicSignature {
    signature(
        vec![
            ArgumentSignature {
                name: "condition",
                ty: Type::series(ValueType::Bool),
            },
            number("source", Qualifier::Series),
        ],
        Type::series(ValueType::Any),
        false,
    )
}

fn nz_signature() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            number("replacement", Qualifier::Simple),
        ],
        Type::series(ValueType::Float),
        false,
    )
}

fn ta_vwma() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            number("volume", Qualifier::Series),
            ArgumentSignature {
                name: "length",
                ty: Type::simple(ValueType::Int),
            },
        ],
        Type::series(ValueType::Float),
        false,
    )
    .windowed(2)
}

fn ta_window() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            ArgumentSignature {
                name: "length",
                ty: Type::simple(ValueType::Int),
            },
        ],
        Type::series(ValueType::Float),
        false,
    )
    .windowed(1)
}

fn ta_pivot() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            ArgumentSignature {
                name: "left",
                ty: Type::simple(ValueType::Int),
            },
            ArgumentSignature {
                name: "right",
                ty: Type::simple(ValueType::Int),
            },
        ],
        Type::series(ValueType::Float),
        false,
    )
    .centered(1, 2)
}

fn ta_cum() -> IntrinsicSignature {
    signature(
        vec![number("source", Qualifier::Series)],
        Type::series(ValueType::Float),
        false,
    )
}

fn ta_tr_signature() -> IntrinsicSignature {
    signature(
        vec![
            number("high", Qualifier::Series),
            number("low", Qualifier::Series),
            number("close", Qualifier::Series),
        ],
        Type::series(ValueType::Float),
        false,
    )
}

fn ta_avwap_signature() -> IntrinsicSignature {
    signature(
        vec![
            number("source", Qualifier::Series),
            number("volume", Qualifier::Series),
            ArgumentSignature {
                name: "anchor",
                ty: Type::simple(ValueType::String),
            },
        ],
        Type::series(ValueType::Float),
        false,
    )
}

fn select_signature() -> IntrinsicSignature {
    signature(
        vec![
            ArgumentSignature {
                name: "condition",
                ty: Type::series(ValueType::Bool),
            },
            ArgumentSignature {
                name: "when_true",
                ty: Type::series(ValueType::Any),
            },
            ArgumentSignature {
                name: "when_false",
                ty: Type::series(ValueType::Any),
            },
        ],
        Type::series(ValueType::Any),
        false,
    )
}

fn coalesce_signature() -> IntrinsicSignature {
    signature(
        vec![
            ArgumentSignature {
                name: "source",
                ty: Type::series(ValueType::Any),
            },
            ArgumentSignature {
                name: "fallback",
                ty: Type::series(ValueType::Any),
            },
        ],
        Type::series(ValueType::Any),
        false,
    )
}

fn color_rgb_signature() -> IntrinsicSignature {
    signature(
        vec![
            ArgumentSignature {
                name: "red",
                ty: Type::series(ValueType::Int),
            },
            ArgumentSignature {
                name: "green",
                ty: Type::series(ValueType::Int),
            },
            ArgumentSignature {
                name: "blue",
                ty: Type::series(ValueType::Int),
            },
            ArgumentSignature {
                name: "alpha",
                ty: Type::series(ValueType::Int),
            },
        ],
        Type::series(ValueType::Color),
        true,
    )
    .at_least(3)
}

fn eval_abs(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(value) = args.first().and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    Ok(float_value(value.abs()))
}

fn eval_max(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    fold_min_max(args, f64::max)
}

fn eval_min(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    fold_min_max(args, f64::min)
}

fn fold_min_max(args: &[Value], operation: fn(f64, f64) -> f64) -> Result<Value, IntrinsicError> {
    if args.iter().any(Value::is_na) {
        return Ok(Value::Na);
    }
    let mut values = args.iter().filter_map(Value::as_float);
    let Some(first) = values.next() else {
        return Ok(Value::Na);
    };
    Ok(float_value(values.fold(first, operation)))
}

fn eval_nz(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    context.charge(
        args.iter()
            .fold(0usize, |work, value| work.saturating_add(value.copy_work())),
    )?;
    let source = args
        .first()
        .ok_or(IntrinsicError::InvalidArguments("nz expects a source"))?;
    if source.is_na() {
        Ok(args.get(1).cloned().unwrap_or(Value::Float(0.0)))
    } else {
        Ok(source.clone())
    }
}

fn eval_na(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    Ok(Value::Bool(args.first().is_some_and(Value::is_na)))
}

fn eval_coalesce(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let source = args.first().ok_or(IntrinsicError::InvalidArguments(
        "coalesce expects a source",
    ))?;
    Ok(if source.is_na() {
        args.get(1).cloned().unwrap_or(Value::Na)
    } else {
        source.clone()
    })
}

fn eval_select(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let condition = args.first().ok_or(IntrinsicError::InvalidArguments(
        "select expects a condition",
    ))?;
    Ok(if condition.is_truthy() {
        args.get(1).cloned().unwrap_or(Value::Na)
    } else {
        args.get(2).cloned().unwrap_or(Value::Na)
    })
}

fn eval_tostring(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    context.charge(
        args.iter()
            .fold(0usize, |work, value| work.saturating_add(value.copy_work())),
    )?;
    let value = args.first().cloned().unwrap_or(Value::Na);
    let text = match value {
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::String(value) => value,
        Value::Color(value) => format!("#{:08x}", value),
        Value::Tuple(values) => format!("({} values)", values.len()),
        Value::Array(values) => format!("[{} values]", values.len()),
        Value::Record(values) => format!("{{{} fields}}", values.len()),
        Value::Na => "na".to_owned(),
    };
    Ok(Value::String(text))
}

fn eval_color_rgb(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let component = |index: usize, name: &'static str| {
        args.get(index)
            .and_then(Value::as_int)
            .filter(|value| (0..=255).contains(value))
            .ok_or(IntrinsicError::InvalidArguments(name))
    };
    let red = component(0, "color.rgb red must be between 0 and 255")? as u32;
    let green = component(1, "color.rgb green must be between 0 and 255")? as u32;
    let blue = component(2, "color.rgb blue must be between 0 and 255")? as u32;
    let alpha = args
        .get(3)
        .filter(|value| !value.is_na())
        .map(|_| component(3, "color.rgb alpha must be between 0 and 255"))
        .transpose()?
        .unwrap_or(255) as u32;
    Ok(Value::Color(
        (alpha << 24) | (red << 16) | (green << 8) | blue,
    ))
}

fn eval_array_new(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    context.charge(args.iter().fold(args.len(), |work, value| {
        work.saturating_add(value.copy_work())
    }))?;
    let array = ArrayValue::new(args.to_vec());
    let (elements, heap_bytes) = array.resource_usage();
    check_collection_limits(context, elements, heap_bytes)?;
    Ok(Value::Array(Arc::new(array)))
}

fn eval_array_push(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Array(values)) = args.first() else {
        return Err(IntrinsicError::InvalidArguments(
            "array.push expects an array",
        ));
    };
    let value = args.get(1).cloned().unwrap_or(Value::Na);
    let (projected, projected_heap) = values.projected_push_usage(&value);
    check_collection_limits(context, projected, projected_heap)?;
    context.charge(values.as_slice().iter().fold(values.len(), |work, value| {
        work.saturating_add(value.copy_work())
    }))?;
    let mut values = Arc::clone(values);
    Arc::make_mut(&mut values).push(value);
    Ok(Value::Array(values))
}

fn eval_array_get(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Array(values)) = args.first() else {
        return Err(IntrinsicError::InvalidArguments(
            "array.get expects an array",
        ));
    };
    let Some(index) = args
        .get(1)
        .and_then(Value::as_int)
        .and_then(|index| usize::try_from(index).ok())
    else {
        return Ok(Value::Na);
    };
    context.charge(values.get(index).map_or(0, Value::copy_work))?;
    Ok(values.get(index).cloned().unwrap_or(Value::Na))
}

fn eval_array_size(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Array(values)) = args.first() else {
        return Err(IntrinsicError::InvalidArguments(
            "array.size expects an array",
        ));
    };
    Ok(Value::Int(values.len() as i64))
}

fn eval_record_new(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    _args: &[Value],
) -> Result<Value, IntrinsicError> {
    Ok(Value::Record(BTreeMap::new()))
}

fn eval_record_set(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Record(values)) = args.first() else {
        return Err(IntrinsicError::InvalidArguments(
            "record.set expects a record",
        ));
    };
    let Some(Value::String(key)) = args.get(1) else {
        return Err(IntrinsicError::InvalidArguments(
            "record.set expects a string key",
        ));
    };
    let value = args.get(2).cloned().unwrap_or(Value::Na);
    let previous = values
        .get(key)
        .map(|value| 1usize.saturating_add(value.resource_usage().0))
        .unwrap_or(0);
    let previous_heap = values
        .get(key)
        .map(|value| {
            size_of::<String>()
                .saturating_add(key.capacity())
                .saturating_add(value.resource_usage().1)
        })
        .unwrap_or(0);
    let projected = record_elements(values)
        .saturating_sub(previous)
        .saturating_add(1)
        .saturating_add(value.resource_usage().0);
    let projected_heap = record_heap_bytes(values)
        .saturating_sub(previous_heap)
        .saturating_add(size_of::<String>())
        .saturating_add(key.capacity())
        .saturating_add(value.resource_usage().1);
    check_collection_limits(context, projected, projected_heap)?;
    context.charge(values.iter().fold(values.len(), |work, (key, value)| {
        work.saturating_add(key.len())
            .saturating_add(value.copy_work())
    }))?;
    let mut values = values.clone();
    values.insert(key.clone(), value);
    Ok(Value::Record(values))
}

fn eval_record_get(
    context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Record(values)) = args.first() else {
        return Ok(Value::Na);
    };
    let Some(Value::String(key)) = args.get(1) else {
        return Err(IntrinsicError::InvalidArguments(
            "record.get expects a string key",
        ));
    };
    context.charge(values.get(key).map_or(0, Value::copy_work))?;
    Ok(values.get(key).cloned().unwrap_or(Value::Na))
}

fn record_elements(values: &BTreeMap<String, Value>) -> usize {
    values.values().fold(values.len(), |total, value| {
        total.saturating_add(value.resource_usage().0)
    })
}

fn record_heap_bytes(values: &BTreeMap<String, Value>) -> usize {
    values.iter().fold(
        size_of::<BTreeMap<String, Value>>()
            .saturating_add(values.len().saturating_mul(size_of::<(String, Value)>())),
        |total, (key, value)| {
            total
                .saturating_add(size_of::<String>())
                .saturating_add(key.capacity())
                .saturating_add(value.resource_usage().1)
        },
    )
}

fn check_collection_limits(
    context: &IntrinsicEvalContext<'_>,
    elements: usize,
    heap_bytes: usize,
) -> Result<(), IntrinsicError> {
    if elements > context.max_collection_elements {
        return Err(IntrinsicError::LimitExceeded("collection elements"));
    }
    if heap_bytes > context.max_heap_bytes {
        return Err(IntrinsicError::LimitExceeded("heap bytes"));
    }
    Ok(())
}

fn eval_sma(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let source = args.first().and_then(Value::as_float);
    let length = window_length(context, args, 1)?;
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Sma {
            values: VecDeque::new(),
        });
    let IntrinsicState::Sma { values } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.sma call site has incompatible state",
        ));
    };
    push_window(values, source, length);
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        Ok(Value::Na)
    } else {
        Ok(float_value(
            values.iter().flatten().sum::<f64>() / values.len() as f64,
        ))
    }
}

fn eval_ema(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let Some(source) = args.first().and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Ema { previous: None });
    let previous = match &mut *state {
        IntrinsicState::Ema { previous } => previous,
        _ => {
            return Err(IntrinsicError::InvalidState(
                "ta.ema call site has incompatible state",
            ));
        }
    };
    let next = match *previous {
        Some(previous) => {
            let alpha = 2.0 / (length as f64 + 1.0);
            alpha * source + (1.0 - alpha) * previous
        }
        None => source,
    };
    if !next.is_finite() {
        return Ok(Value::Na);
    }
    *previous = Some(next);
    Ok(float_value(next))
}

fn eval_rma(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let Some(source) = args.first().and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Rma { previous: None });
    let previous = match &mut *state {
        IntrinsicState::Rma { previous } => previous,
        _ => {
            return Err(IntrinsicError::InvalidState(
                "ta.rma call site has incompatible state",
            ));
        }
    };
    let next = match *previous {
        Some(previous) => (previous * (length as f64 - 1.0) + source) / length as f64,
        None => source,
    };
    if !next.is_finite() {
        return Ok(Value::Na);
    }
    *previous = Some(next);
    Ok(float_value(next))
}

fn eval_rsi(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let source = args.first().and_then(Value::as_float);
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Rsi {
            previous_source: None,
            initial_gain_sum: 0.0,
            initial_loss_sum: 0.0,
            initial_count: 0,
            average_gain: None,
            average_loss: None,
        });
    let IntrinsicState::Rsi {
        previous_source,
        initial_gain_sum,
        initial_loss_sum,
        initial_count,
        average_gain,
        average_loss,
    } = state
    else {
        return Err(IntrinsicError::InvalidState(
            "ta.rsi call site has incompatible state",
        ));
    };

    let Some(source) = source else {
        *previous_source = None;
        return Ok(Value::Na);
    };
    let Some(previous) = *previous_source else {
        *previous_source = Some(source);
        return Ok(Value::Na);
    };
    *previous_source = Some(source);

    let delta = source - previous;
    if !delta.is_finite() {
        return Ok(Value::Na);
    }
    let gain = delta.max(0.0);
    let loss = (-delta).max(0.0);
    let (next_gain, next_loss) = match (*average_gain, *average_loss) {
        (Some(previous_gain), Some(previous_loss)) => (
            (previous_gain * (length as f64 - 1.0) + gain) / length as f64,
            (previous_loss * (length as f64 - 1.0) + loss) / length as f64,
        ),
        (None, None) => {
            let next_gain_sum = *initial_gain_sum + gain;
            let next_loss_sum = *initial_loss_sum + loss;
            if !next_gain_sum.is_finite() || !next_loss_sum.is_finite() {
                return Ok(Value::Na);
            }
            *initial_gain_sum = next_gain_sum;
            *initial_loss_sum = next_loss_sum;
            *initial_count = initial_count.saturating_add(1);
            if *initial_count < length {
                return Ok(Value::Na);
            }
            (next_gain_sum / length as f64, next_loss_sum / length as f64)
        }
        _ => {
            return Err(IntrinsicError::InvalidState(
                "ta.rsi call site has incomplete average state",
            ));
        }
    };
    if !next_gain.is_finite() || !next_loss.is_finite() {
        return Ok(Value::Na);
    }
    *average_gain = Some(next_gain);
    *average_loss = Some(next_loss);

    let value = if next_gain == 0.0 {
        if next_loss == 0.0 { 50.0 } else { 0.0 }
    } else if next_loss == 0.0 {
        100.0
    } else {
        let relative_strength = next_gain / next_loss;
        100.0 - 100.0 / (1.0 + relative_strength)
    };
    Ok(float_value(value))
}

fn eval_previous_when(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let condition = args.first().is_some_and(Value::is_truthy);
    let source = args.get(1).cloned().unwrap_or(Value::Na);
    if !matches!(source, Value::Int(_) | Value::Float(_) | Value::Na) {
        return Err(IntrinsicError::InvalidArguments(
            "ta.previous_when source must be numeric",
        ));
    }
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::PreviousWhen { value: None });
    let IntrinsicState::PreviousWhen { value } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.previous_when call site has incompatible state",
        ));
    };
    let previous = value.clone().unwrap_or(Value::Na);
    if condition {
        *value = Some(source);
    }
    Ok(previous)
}

fn eval_vwma(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 2)?;
    let source = args.first().and_then(Value::as_float);
    let volume = args.get(1).and_then(Value::as_float);
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Vwma {
            values: VecDeque::new(),
        });
    let IntrinsicState::Vwma { values } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.vwma call site has incompatible state",
        ));
    };
    push_window(values, source.zip(volume), length);
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        return Ok(Value::Na);
    }
    let (weighted, total) = values
        .iter()
        .flatten()
        .fold((0.0, 0.0), |(weighted, total), &(source, volume)| {
            (weighted + source * volume, total + volume)
        });
    Ok(if total == 0.0 {
        Value::Na
    } else {
        float_value(weighted / total)
    })
}

fn eval_highest(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_window(context, site, args, true)
}

fn eval_lowest(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_window(context, site, args, false)
}

fn eval_pivothigh(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_pivot(context, site, args, true)
}

fn eval_pivotlow(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_pivot(context, site, args, false)
}

fn eval_pivot(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
    maximum: bool,
) -> Result<Value, IntrinsicError> {
    let left = pivot_leg(context, args, 1)?;
    let right = pivot_leg(context, args, 2)?;
    let length = left
        .checked_add(right)
        .and_then(|length| length.checked_add(1))
        .ok_or(IntrinsicError::LimitExceeded("pivot window"))?;
    if length > context.max_window_length {
        return Err(IntrinsicError::LimitExceeded("pivot window"));
    }
    let source = args.first().and_then(Value::as_float);
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Pivot {
            maximum,
            values: VecDeque::new(),
        });
    let IntrinsicState::Pivot {
        maximum: state_maximum,
        values,
    } = state
    else {
        return Err(IntrinsicError::InvalidState(
            "pivot call site has incompatible state",
        ));
    };
    if *state_maximum != maximum {
        return Err(IntrinsicError::InvalidState(
            "pivot mode changed at a call site",
        ));
    }
    let retained_length = if context.fixed_window {
        length
    } else {
        context.max_window_length
    };
    push_window(values, source, retained_length);
    if values.len() < length {
        return Ok(Value::Na);
    }
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        length.saturating_mul(3),
    )?;
    let candidate_index = values.len() - 1 - right;
    let Some(candidate) = values[candidate_index] else {
        return Ok(Value::Na);
    };
    let window_start = values.len() - length;
    let window = values.iter().skip(window_start).take(length);
    if window.clone().any(Option::is_none) {
        return Ok(Value::Na);
    }
    let extreme = if maximum {
        window.flatten().copied().fold(f64::NEG_INFINITY, f64::max)
    } else {
        window.flatten().copied().fold(f64::INFINITY, f64::min)
    };
    Ok(if candidate == extreme {
        float_value(candidate)
    } else {
        Value::Na
    })
}

fn pivot_leg(
    context: &IntrinsicEvalContext<'_>,
    args: &[Value],
    index: usize,
) -> Result<usize, IntrinsicError> {
    let value = match args.get(index) {
        Some(Value::Int(value)) => *value,
        _ => {
            return Err(IntrinsicError::InvalidArguments(
                "pivot legs must be integers",
            ));
        }
    };
    if value <= 0 {
        return Err(IntrinsicError::InvalidArguments(
            "pivot legs must be positive",
        ));
    }
    let value = usize::try_from(value)
        .map_err(|_| IntrinsicError::InvalidArguments("pivot leg is too large"))?;
    if value > context.max_window_length {
        return Err(IntrinsicError::LimitExceeded("pivot window"));
    }
    Ok(value)
}

fn eval_window(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
    maximum: bool,
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let source = args.first().and_then(Value::as_float);
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Window {
            maximum,
            values: VecDeque::new(),
        });
    let IntrinsicState::Window {
        maximum: state_maximum,
        values,
    } = state
    else {
        return Err(IntrinsicError::InvalidState(
            "window call site has incompatible state",
        ));
    };
    if *state_maximum != maximum {
        return Err(IntrinsicError::InvalidState(
            "window mode changed at a call site",
        ));
    }
    push_window(values, source, length);
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        Ok(Value::Na)
    } else {
        let value = if maximum {
            values
                .iter()
                .flatten()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max)
        } else {
            values
                .iter()
                .flatten()
                .copied()
                .fold(f64::INFINITY, f64::min)
        };
        Ok(float_value(value))
    }
}

fn eval_cum(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(source) = args.first().and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Cum { total: 0.0 });
    let IntrinsicState::Cum { total } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.cum call site has incompatible state",
        ));
    };
    let next = *total + source;
    if !next.is_finite() {
        return Ok(Value::Na);
    }
    *total = next;
    Ok(float_value(next))
}

fn window_length(
    context: &IntrinsicEvalContext<'_>,
    args: &[Value],
    index: usize,
) -> Result<usize, IntrinsicError> {
    let length =
        args.get(index)
            .and_then(Value::as_int)
            .ok_or(IntrinsicError::InvalidArguments(
                "window length must be an integer",
            ))?;
    if length <= 0 {
        return Err(IntrinsicError::InvalidArguments(
            "window length must be positive",
        ));
    }
    let length = usize::try_from(length)
        .map_err(|_| IntrinsicError::InvalidArguments("window length is too large"))?;
    if length > context.max_window_length {
        return Err(IntrinsicError::LimitExceeded("window length"));
    }
    Ok(length)
}

fn push_window<T>(values: &mut VecDeque<Option<T>>, value: Option<T>, length: usize) {
    while values.len() >= length {
        values.pop_front();
    }
    values.push_back(value);
}

fn eval_stdev(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Stdev {
            values: VecDeque::new(),
        });
    let IntrinsicState::Stdev { values } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.stdev call site has incompatible state",
        ));
    };
    push_window(values, args.first().and_then(Value::as_float), length);
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        return Ok(Value::Na);
    }
    let mean = values.iter().flatten().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .flatten()
        .map(|value| {
            let difference = *value - mean;
            difference * difference
        })
        .sum::<f64>()
        / values.len() as f64;
    Ok(float_value(variance.max(0.0).sqrt()))
}

fn eval_dev(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::Dev {
            values: VecDeque::new(),
        });
    let IntrinsicState::Dev { values } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.dev call site has incompatible state",
        ));
    };
    push_window(values, args.first().and_then(Value::as_float), length);
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        return Ok(Value::Na);
    }
    let mean = values.iter().flatten().sum::<f64>() / values.len() as f64;
    let deviation = values
        .iter()
        .flatten()
        .map(|value| (*value - mean).abs())
        .sum::<f64>()
        / values.len() as f64;
    Ok(float_value(deviation))
}

fn eval_highest_bars(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_extreme_bars(context, site, args, true)
}

fn eval_lowest_bars(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    eval_extreme_bars(context, site, args, false)
}

fn eval_extreme_bars(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
    maximum: bool,
) -> Result<Value, IntrinsicError> {
    let length = window_length(context, args, 1)?;
    let source = args.first().and_then(Value::as_float);
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::ExtremeBars {
            maximum,
            values: VecDeque::new(),
        });
    let IntrinsicState::ExtremeBars {
        maximum: state_maximum,
        values,
    } = state
    else {
        return Err(IntrinsicError::InvalidState(
            "extreme bars call site has incompatible state",
        ));
    };
    if *state_maximum != maximum {
        return Err(IntrinsicError::InvalidState(
            "extreme bar mode changed at a call site",
        ));
    }
    push_window(
        values,
        source.map(|source| (source, context.bar_index)),
        length,
    );
    charge_intrinsic_work(
        context.executed_work,
        context.max_work,
        values.len().saturating_mul(3),
    )?;
    if values.len() < length || values.iter().any(|value| value.is_none()) {
        return Ok(Value::Na);
    }
    let mut best = values
        .iter()
        .flatten()
        .next()
        .copied()
        .expect("window is non-empty");
    for &candidate in values.iter().flatten().skip(1) {
        let better = if maximum {
            candidate.0 > best.0
        } else {
            candidate.0 < best.0
        };
        if better {
            best = candidate;
        }
    }
    Ok(float_value((context.bar_index - best.1) as f64))
}

fn eval_true_range(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(high) = args.first().and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let Some(low) = args.get(1).and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let Some(close) = args.get(2).and_then(Value::as_float) else {
        return Ok(Value::Na);
    };
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::TrueRange {
            previous_close: None,
        });
    let IntrinsicState::TrueRange { previous_close } = state else {
        return Err(IntrinsicError::InvalidState(
            "ta.tr call site has incompatible state",
        ));
    };
    let range = match *previous_close {
        Some(previous_close) => (high - low)
            .max((high - previous_close).abs())
            .max((low - previous_close).abs()),
        None => high - low,
    };
    *previous_close = Some(close);
    Ok(float_value(range))
}

fn eval_anchored_vwap(
    context: &mut IntrinsicEvalContext<'_>,
    site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::String(anchor_name)) = args.get(2) else {
        return Err(IntrinsicError::InvalidArguments(
            "ta.avwap anchor must be a string",
        ));
    };
    let Some(anchor) = anchored_start(context.bar_time, anchor_name) else {
        return Ok(Value::Na);
    };
    let state = context
        .states
        .entry(site)
        .or_insert_with(|| IntrinsicState::AnchoredVwap {
            anchor: 0,
            price_volume: 0.0,
            volume: 0.0,
        });
    let IntrinsicState::AnchoredVwap {
        anchor: current_anchor,
        price_volume,
        volume,
    } = state
    else {
        return Err(IntrinsicError::InvalidState(
            "ta.avwap call site has incompatible state",
        ));
    };
    if *current_anchor != anchor {
        *current_anchor = anchor;
        *price_volume = 0.0;
        *volume = 0.0;
    }
    if let (Some(source), Some(volume_value)) = (
        args.first().and_then(Value::as_float),
        args.get(1).and_then(Value::as_float),
    ) {
        let next_price_volume = *price_volume + source * volume_value;
        let next_volume = *volume + volume_value;
        if !next_price_volume.is_finite() || !next_volume.is_finite() {
            return Ok(Value::Na);
        }
        *price_volume = next_price_volume;
        *volume = next_volume;
    }
    if *volume > f64::EPSILON {
        Ok(float_value(*price_volume / *volume))
    } else {
        Ok(Value::Na)
    }
}

fn float_value(value: f64) -> Value {
    if value.is_finite() {
        Value::Float(value)
    } else {
        Value::Na
    }
}

pub(crate) fn anchored_start(time: i64, anchor: &str) -> Option<i64> {
    let datetime = Utc.timestamp_millis_opt(time).single()?;
    let date = datetime.date_naive();
    let date = match anchor.to_ascii_lowercase().as_str() {
        "day" => date,
        "week" => date.checked_sub_days(chrono::Days::new(u64::from(
            date.weekday().num_days_from_monday(),
        )))?,
        "month" => date.with_day(1)?,
        _ => return None,
    };
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&midnight).timestamp_millis())
}
