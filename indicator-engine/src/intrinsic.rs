use crate::hir::{CallSiteId, IntrinsicId};
use crate::types::{Qualifier, Type, Value, ValueType};
use chrono::{Datelike, TimeZone, Utc};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::mem::size_of;

pub type QualifiedName = &'static str;

#[derive(Debug, Clone, PartialEq)]
pub enum IntrinsicState {
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
    AnchoredVwap {
        anchor: i64,
        price_volume: f64,
        volume: f64,
    },
}

impl IntrinsicState {
    pub(crate) fn heap_bytes(&self) -> usize {
        let state_bytes = size_of::<Self>();
        match self {
            Self::Ema { .. }
            | Self::Rma { .. }
            | Self::Cum { .. }
            | Self::TrueRange { .. }
            | Self::AnchoredVwap { .. } => state_bytes,
            Self::Sma { values }
            | Self::Window { values, .. }
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
    /// Argument index of the rolling window length, when the intrinsic is a
    /// windowed calculation. Used by static warmup analysis.
    pub window: Option<usize>,
}

impl IntrinsicSignature {
    pub(crate) fn windowed(mut self, argument: usize) -> Self {
        self.window = Some(argument);
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
        }
    }
}

impl std::error::Error for IntrinsicError {}

pub struct IntrinsicEvalContext<'a> {
    pub bar_index: usize,
    pub bar_time: i64,
    pub max_heap_bytes: usize,
    pub max_window_length: usize,
    pub max_collection_elements: usize,
    pub states: &'a mut BTreeMap<CallSiteId, IntrinsicState>,
}

pub type IntrinsicEvaluator = for<'a> fn(
    &mut IntrinsicEvalContext<'a>,
    CallSiteId,
    &[Value],
) -> Result<Value, IntrinsicError>;
pub type StateFactory = fn() -> IntrinsicState;

#[derive(Clone)]
pub struct IntrinsicDefinition {
    pub name: QualifiedName,
    pub signature: IntrinsicSignature,
    pub effects: IntrinsicEffects,
    pub state_factory: Option<StateFactory>,
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
        registry.register(
            "math.abs",
            math_abs(),
            IntrinsicEffects::PURE,
            None,
            eval_abs,
        );
        registry.register(
            "math.max",
            math_min_max(true),
            IntrinsicEffects::PURE,
            None,
            eval_max,
        );
        registry.register(
            "math.min",
            math_min_max(true),
            IntrinsicEffects::PURE,
            None,
            eval_min,
        );
        registry.register(
            "ta.sma",
            ta_average(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Sma {
                values: VecDeque::new(),
            }),
            eval_sma,
        );
        registry.register(
            "ta.ema",
            ta_average(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Ema { previous: None }),
            eval_ema,
        );
        registry.register(
            "ta.rma",
            ta_average(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Rma { previous: None }),
            eval_rma,
        );
        registry.register(
            "ta.vwma",
            ta_vwma(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Vwma {
                values: VecDeque::new(),
            }),
            eval_vwma,
        );
        registry.register(
            "ta.highest",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Window {
                maximum: true,
                values: VecDeque::new(),
            }),
            eval_highest,
        );
        registry.register(
            "ta.lowest",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Window {
                maximum: false,
                values: VecDeque::new(),
            }),
            eval_lowest,
        );
        registry.register(
            "ta.cum",
            ta_cum(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Cum { total: 0.0 }),
            eval_cum,
        );
        registry.register(
            "ta.stdev",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Stdev {
                values: VecDeque::new(),
            }),
            eval_stdev,
        );
        registry.register(
            "ta.dev",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::Dev {
                values: VecDeque::new(),
            }),
            eval_dev,
        );
        registry.register(
            "ta.highestbars",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::ExtremeBars {
                maximum: true,
                values: VecDeque::new(),
            }),
            eval_highest_bars,
        );
        registry.register(
            "ta.lowestbars",
            ta_window(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::ExtremeBars {
                maximum: false,
                values: VecDeque::new(),
            }),
            eval_lowest_bars,
        );
        registry.register(
            "ta.tr",
            ta_tr_signature(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::TrueRange {
                previous_close: None,
            }),
            eval_true_range,
        );
        registry.register(
            "ta.avwap",
            ta_avwap_signature(),
            IntrinsicEffects::STATEFUL,
            Some(|| IntrinsicState::AnchoredVwap {
                anchor: 0,
                price_volume: 0.0,
                volume: 0.0,
            }),
            eval_anchored_vwap,
        );
        registry.register("nz", nz_signature(), IntrinsicEffects::PURE, None, eval_nz);
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
            None,
            eval_na,
        );
        registry.register(
            "coalesce",
            coalesce_signature(),
            IntrinsicEffects::PURE,
            None,
            eval_coalesce,
        );
        registry.register(
            "select",
            select_signature(),
            IntrinsicEffects::PURE,
            None,
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
            None,
            eval_tostring,
        );
        registry.register(
            "color.rgb",
            color_rgb_signature(),
            IntrinsicEffects::PURE,
            None,
            eval_color_rgb,
        );
        registry.register(
            "array.new",
            signature(Vec::new(), Type::simple(ValueType::Array), true),
            IntrinsicEffects::PURE,
            None,
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
            None,
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
            None,
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
            None,
            eval_array_size,
        );
        registry.register(
            "record.new",
            signature(Vec::new(), Type::simple(ValueType::Record), false),
            IntrinsicEffects::PURE,
            None,
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
            None,
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
            None,
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
        state_factory: Option<StateFactory>,
        evaluator: IntrinsicEvaluator,
    ) -> IntrinsicId {
        let id = IntrinsicId(self.definitions.len());
        self.ids.insert(name, id);
        self.definitions.push(IntrinsicDefinition {
            name,
            signature,
            effects,
            state_factory,
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
        window: None,
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
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
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
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
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
    check_collection_limits(
        context,
        collection_elements(args),
        collection_heap_bytes(args),
    )?;
    Ok(Value::Array(args.to_vec()))
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
    let projected = collection_elements(values)
        .saturating_add(1)
        .saturating_add(value.resource_usage().0);
    let projected_heap = collection_heap_bytes(values).saturating_add(value.resource_usage().1);
    check_collection_limits(context, projected, projected_heap)?;
    let mut values = values.clone();
    values.push(value);
    Ok(Value::Array(values))
}

fn eval_array_get(
    _context: &mut IntrinsicEvalContext<'_>,
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
        .map(|value| key.len().saturating_add(value.resource_usage().1))
        .unwrap_or(0);
    let projected = record_elements(values)
        .saturating_sub(previous)
        .saturating_add(1)
        .saturating_add(value.resource_usage().0);
    let projected_heap = record_heap_bytes(values)
        .saturating_sub(previous_heap)
        .saturating_add(key.len())
        .saturating_add(value.resource_usage().1);
    check_collection_limits(context, projected, projected_heap)?;
    let mut values = values.clone();
    values.insert(key.clone(), value);
    Ok(Value::Record(values))
}

fn eval_record_get(
    _context: &mut IntrinsicEvalContext<'_>,
    _site: CallSiteId,
    args: &[Value],
) -> Result<Value, IntrinsicError> {
    let Some(Value::Record(values)) = args.first() else {
        return Err(IntrinsicError::InvalidArguments(
            "record.get expects a record",
        ));
    };
    let Some(Value::String(key)) = args.get(1) else {
        return Err(IntrinsicError::InvalidArguments(
            "record.get expects a string key",
        ));
    };
    Ok(values.get(key).cloned().unwrap_or(Value::Na))
}

fn collection_elements(values: &[Value]) -> usize {
    values.iter().fold(0usize, |total, value| {
        total
            .saturating_add(1)
            .saturating_add(value.resource_usage().0)
    })
}

fn collection_heap_bytes(values: &[Value]) -> usize {
    values.iter().fold(0usize, |total, value| {
        total.saturating_add(value.resource_usage().1)
    })
}

fn record_elements(values: &BTreeMap<String, Value>) -> usize {
    values.values().fold(values.len(), |total, value| {
        total.saturating_add(value.resource_usage().0)
    })
}

fn record_heap_bytes(values: &BTreeMap<String, Value>) -> usize {
    values.iter().fold(0usize, |total, (key, value)| {
        total
            .saturating_add(key.len())
            .saturating_add(value.resource_usage().1)
    })
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
