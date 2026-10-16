//! Static history analysis over the typed HIR.
//!
//! Warmup bars and anchored history starts are derived from the compiled
//! expressions so hosts can plan fetches and gate availability per indicator
//! instead of falling back to the retained-history cap.

use crate::PlanningError;
use crate::hir::{ExprId, ExpressionKind, HirProgram, LocalSlot, StatementId, StatementKind};
use crate::intrinsic::{IntrinsicRegistry, WarmupSpec};
use crate::runtime::charge_work;
use crate::types::{IndicatorConfig, IndicatorDescriptor, InputValue, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_ANALYSIS_WORK: usize = 250_000;
const MAX_ANALYSIS_DEPTH: usize = 64;

pub(super) fn warmup_bars(
    program: &HirProgram,
    registry: &IntrinsicRegistry,
    descriptor: &IndicatorDescriptor,
    config: &IndicatorConfig,
    max_window_length: usize,
) -> Result<usize, PlanningError> {
    let mut evaluator = Evaluator::new(program, registry, descriptor, config, max_window_length);
    match program.blocks.get(program.entry_point.0) {
        Some(block) => evaluator.warmup_block(&block.statements),
        None => Ok(0),
    }
}

/// Resolve the earliest anchored boundary within the analysis budget.
pub(super) fn anchor_start(
    program: &HirProgram,
    registry: &IntrinsicRegistry,
    descriptor: &IndicatorDescriptor,
    config: &IndicatorConfig,
    reference_time: i64,
) -> Result<Option<i64>, PlanningError> {
    let mut evaluator = Evaluator::new(program, registry, descriptor, config, usize::MAX);
    evaluator.budget.charge(program.expressions.len())?;
    let mut earliest: Option<i64> = None;
    for arguments in anchored_arguments(program, registry) {
        evaluator.budget.charge(1)?;
        if let Some(anchor) = arguments.get(2).copied()
            && let Some(Value::String(name)) = evaluator.static_value(anchor)?
            && let Some(start) = crate::intrinsic::anchored_start(reference_time, &name)
        {
            earliest = Some(earliest.map_or(start, |current| current.min(start)));
        }
    }
    Ok(earliest)
}

pub(super) fn requires_anchored_history(
    program: &HirProgram,
    registry: &IntrinsicRegistry,
) -> bool {
    anchored_arguments(program, registry).next().is_some()
}

fn anchored_arguments<'a>(
    program: &'a HirProgram,
    registry: &IntrinsicRegistry,
) -> impl Iterator<Item = &'a [ExprId]> {
    let avwap = registry.find("ta.avwap").map(|(id, _)| id);
    program
        .expressions
        .iter()
        .filter_map(move |expression| match &expression.kind {
            ExpressionKind::IntrinsicCall {
                intrinsic,
                arguments,
                ..
            } if Some(*intrinsic) == avwap => Some(arguments.as_slice()),
            _ => None,
        })
}

#[derive(Default)]
struct AnalysisBudget {
    work: usize,
    depth: usize,
}

impl AnalysisBudget {
    fn charge(&mut self, work: usize) -> Result<(), PlanningError> {
        if charge_work(&mut self.work, MAX_ANALYSIS_WORK, work) {
            Ok(())
        } else {
            Err(PlanningError::HistoryAnalysisWorkLimitExceeded)
        }
    }

    fn copy_value(&mut self, value: &Value) -> Result<Value, PlanningError> {
        self.charge(value.copy_work())?;
        Ok(value.clone())
    }
}

struct Evaluator<'a> {
    program: &'a HirProgram,
    registry: &'a IntrinsicRegistry,
    descriptor: &'a IndicatorDescriptor,
    config: &'a IndicatorConfig,
    max_window_length: usize,
    budget: AnalysisBudget,
    visiting: HashSet<ExprId>,
    warmup_cache: HashMap<ExprId, usize>,
    static_value_cache: HashMap<ExprId, Option<Value>>,
    parameter_warmups: Vec<BTreeMap<LocalSlot, usize>>,
    parameter_values: Vec<BTreeMap<LocalSlot, Value>>,
}

impl<'a> Evaluator<'a> {
    fn new(
        program: &'a HirProgram,
        registry: &'a IntrinsicRegistry,
        descriptor: &'a IndicatorDescriptor,
        config: &'a IndicatorConfig,
        max_window_length: usize,
    ) -> Self {
        Self {
            program,
            registry,
            descriptor,
            config,
            max_window_length,
            budget: AnalysisBudget::default(),
            visiting: HashSet::new(),
            warmup_cache: HashMap::new(),
            static_value_cache: HashMap::new(),
            parameter_warmups: Vec::new(),
            parameter_values: Vec::new(),
        }
    }

    fn visit<T>(
        &mut self,
        analyze: impl FnOnce(&mut Self) -> Result<T, PlanningError>,
    ) -> Result<T, PlanningError> {
        self.budget.charge(1)?;
        if self.budget.depth >= MAX_ANALYSIS_DEPTH {
            return Err(PlanningError::HistoryAnalysisDepthLimitExceeded);
        }
        self.budget.depth += 1;
        let result = analyze(self);
        self.budget.depth -= 1;
        result
    }

    fn warmup_block(&mut self, statements: &[StatementId]) -> Result<usize, PlanningError> {
        let mut warmup = 0;
        for statement in statements {
            warmup = warmup.max(self.warmup_statement(*statement)?);
        }
        Ok(warmup)
    }

    fn warmup_statement(&mut self, statement: StatementId) -> Result<usize, PlanningError> {
        self.visit(|evaluator| evaluator.warmup_statement_inner(statement))
    }

    fn warmup_statement_inner(&mut self, statement: StatementId) -> Result<usize, PlanningError> {
        let Some(statement) = self.program.statements.get(statement.0) else {
            return Ok(0);
        };
        match &statement.kind {
            StatementKind::Binding { expression, .. }
            | StatementKind::LocalUpdate { expression, .. }
            | StatementKind::StateDeclaration {
                initial: expression,
                ..
            }
            | StatementKind::StateUpdate { expression, .. }
            | StatementKind::Expression(expression)
            | StatementKind::Return(Some(expression))
            | StatementKind::Destructure { expression, .. } => self.warmup(*expression),
            StatementKind::Plot {
                expression, color, ..
            }
            | StatementKind::HorizontalLevel {
                expression, color, ..
            } => self.warmup_all([Some(*expression), *color]),
            StatementKind::Marker {
                expression,
                visible,
                color,
                ..
            } => self.warmup_all([Some(*expression), *visible, *color]),
            StatementKind::Segment {
                start_value,
                end_value,
                start_index,
                end_index,
                visible,
                color,
                ..
            } => self.warmup_all([
                Some(*start_value),
                Some(*end_value),
                Some(*start_index),
                Some(*end_index),
                *visible,
                *color,
            ]),
            StatementKind::OhlcPlot {
                open,
                high,
                low,
                close,
                color,
                ..
            } => self.warmup_all([Some(*open), Some(*high), Some(*low), Some(*close), *color]),
            StatementKind::RangePlot {
                upper,
                lower,
                color,
                ..
            } => self.warmup_all([Some(*upper), Some(*lower), *color]),
            StatementKind::Region {
                key,
                upper,
                lower,
                left_index,
                right_index,
                visible,
                border_color,
                fill_color,
                style_by,
                extend_right,
                ..
            } => self.warmup_all([
                Some(*key),
                Some(*upper),
                Some(*lower),
                Some(*left_index),
                Some(*right_index),
                *visible,
                *border_color,
                *fill_color,
                *style_by,
                *extend_right,
            ]),
            StatementKind::RegionRemove { key, .. } => self.warmup(*key),
            StatementKind::If {
                condition,
                then_branch,
                else_branch,
            } => Ok(self
                .warmup(*condition)?
                .max(self.warmup_block(then_branch)?)
                .max(self.warmup_block(else_branch)?)),
            StatementKind::While { condition, body } => {
                Ok(self.warmup(*condition)?.max(self.warmup_block(body)?))
            }
            StatementKind::ForRange {
                start, end, body, ..
            } => Ok(self
                .warmup(*start)?
                .max(self.warmup(*end)?)
                .max(self.warmup_block(body)?)),
            StatementKind::ForEach {
                collection, body, ..
            } => Ok(self.warmup(*collection)?.max(self.warmup_block(body)?)),
            StatementKind::Break | StatementKind::Continue | StatementKind::Return(None) => Ok(0),
        }
    }

    fn warmup_all<const N: usize>(
        &mut self,
        expressions: [Option<ExprId>; N],
    ) -> Result<usize, PlanningError> {
        self.warmup_expressions(expressions.into_iter().flatten())
    }

    fn warmup_expressions(
        &mut self,
        expressions: impl IntoIterator<Item = ExprId>,
    ) -> Result<usize, PlanningError> {
        let mut warmup = 0;
        for expression in expressions {
            warmup = warmup.max(self.warmup(expression)?);
        }
        Ok(warmup)
    }

    fn warmup(&mut self, expr: ExprId) -> Result<usize, PlanningError> {
        self.visit(|evaluator| evaluator.warmup_inner(expr))
    }

    fn warmup_inner(&mut self, expr: ExprId) -> Result<usize, PlanningError> {
        let root_context = self.parameter_warmups.is_empty() && self.parameter_values.is_empty();
        if root_context && let Some(warmup) = self.warmup_cache.get(&expr) {
            return Ok(*warmup);
        }
        let Some(expression) = self.program.expressions.get(expr.0) else {
            return Ok(0);
        };
        let warmup = match &expression.kind {
            ExpressionKind::Constant(_)
            | ExpressionKind::LoadBarIndex
            | ExpressionKind::LoadBarState(_)
            | ExpressionKind::LoadSeries(_)
            | ExpressionKind::LoadInput(_)
            | ExpressionKind::LoadSeriesInput(_)
            | ExpressionKind::LoadState(_)
            | ExpressionKind::LoadFrame(_) => 0,
            ExpressionKind::RecordField { source, .. } => self.warmup(*source)?,
            ExpressionKind::LoadLocal(slot) => {
                if let Some(warmup) = self.parameter_warmup(*slot)? {
                    return Ok(warmup);
                }
                let Some(Some(definition)) = self.program.local_definitions.get(slot.0) else {
                    return Ok(0);
                };
                if !self.visiting.insert(expr) {
                    return Ok(0);
                }
                let warmup = self.warmup(*definition)?;
                self.visiting.remove(&expr);
                warmup
            }
            ExpressionKind::History { source, offset } => {
                let offset = match self
                    .static_value(*offset)?
                    .and_then(|value| value.as_float())
                {
                    Some(value) if value >= 0.0 => value as usize,
                    _ => self.descriptor.max_bars_back,
                };
                offset.saturating_add(self.warmup(*source)?)
            }
            ExpressionKind::Tuple(expressions) | ExpressionKind::Array(expressions) => {
                self.warmup_expressions(expressions.iter().copied())?
            }
            ExpressionKind::Record(fields) => {
                self.warmup_expressions(fields.iter().map(|(_, id)| *id))?
            }
            ExpressionKind::Switch {
                operand,
                branches,
                fallback,
            } => {
                let mut warmup = self.warmup(*operand)?;
                for (condition, result) in branches {
                    warmup = warmup
                        .max(self.warmup(*condition)?)
                        .max(self.warmup(*result)?);
                }
                if let Some(fallback) = fallback {
                    warmup = warmup.max(self.warmup(*fallback)?);
                }
                warmup
            }
            ExpressionKind::CollectionIndex { source, index } => {
                self.warmup(*source)?.max(self.warmup(*index)?)
            }
            ExpressionKind::Unary { operand, .. } => self.warmup(*operand)?,
            ExpressionKind::Binary { left, right, .. } => {
                self.warmup(*left)?.max(self.warmup(*right)?)
            }
            ExpressionKind::Conditional {
                condition,
                when_true,
                when_false,
            } => self
                .warmup(*condition)?
                .max(self.warmup(*when_true)?)
                .max(self.warmup(*when_false)?),
            ExpressionKind::Coalesce { source, fallback } => {
                self.warmup(*source)?.max(self.warmup(*fallback)?)
            }
            ExpressionKind::IntrinsicCall {
                intrinsic,
                arguments,
                ..
            } => {
                let mut warmup = self.warmup_expressions(arguments.iter().copied())?;
                if let Some(spec) = self
                    .registry
                    .get(*intrinsic)
                    .and_then(|definition| definition.signature.warmup.as_ref())
                {
                    let additional = match spec {
                        WarmupSpec::Window {
                            argument,
                            preceding,
                        } => self
                            .window_length(arguments, *argument)?
                            .min(self.max_window_length)
                            .saturating_add(*preceding),
                        WarmupSpec::Centered {
                            left_argument,
                            right_argument,
                        } => self
                            .window_length(arguments, *left_argument)?
                            .saturating_add(self.window_length(arguments, *right_argument)?)
                            .saturating_add(1)
                            .min(self.max_window_length),
                    };
                    warmup = warmup.saturating_add(additional);
                }
                warmup
            }
            ExpressionKind::FunctionCall {
                function,
                arguments,
                argument_slots,
                ..
            } => self.warmup_function(*function, arguments, argument_slots)?,
        };
        if root_context {
            self.warmup_cache.insert(expr, warmup);
        }
        Ok(warmup)
    }

    fn window_length(
        &mut self,
        arguments: &[ExprId],
        argument: usize,
    ) -> Result<usize, PlanningError> {
        let value = match arguments.get(argument) {
            Some(id) => self.static_value(*id)?,
            None => None,
        };
        Ok(value
            .and_then(|value| value.as_float())
            .filter(|value| *value >= 1.0)
            .map(|value| value as usize)
            .unwrap_or(1))
    }

    fn warmup_function(
        &mut self,
        function_id: crate::hir::FunctionId,
        arguments: &[ExprId],
        argument_slots: &[usize],
    ) -> Result<usize, PlanningError> {
        let argument_warmups = arguments
            .iter()
            .map(|argument| self.warmup(*argument))
            .collect::<Result<Vec<_>, _>>()?;
        let argument_values = arguments
            .iter()
            .map(|argument| self.static_value(*argument))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(function) = self.program.functions.get(function_id.0) else {
            return Ok(argument_warmups.into_iter().max().unwrap_or(0));
        };
        let mut parameter_warmups = BTreeMap::new();
        let mut parameter_values = BTreeMap::new();
        for (argument_index, parameter_index) in argument_slots.iter().copied().enumerate() {
            self.budget.charge(1)?;
            let Some(parameter) = function.parameters.get(parameter_index).copied() else {
                continue;
            };
            parameter_warmups.insert(
                parameter,
                argument_warmups.get(argument_index).copied().unwrap_or(0),
            );
            if let Some(Some(value)) = argument_values.get(argument_index) {
                parameter_values.insert(parameter, self.budget.copy_value(value)?);
            }
        }
        self.parameter_warmups.push(parameter_warmups);
        self.parameter_values.push(parameter_values);
        let result = (|| {
            for (parameter_index, default) in function.parameter_defaults.iter().enumerate() {
                self.budget.charge(1)?;
                let Some(parameter) = function.parameters.get(parameter_index).copied() else {
                    continue;
                };
                if self
                    .parameter_warmups
                    .last()
                    .is_some_and(|values| values.contains_key(&parameter))
                {
                    continue;
                }
                let (warmup, value) = match default {
                    Some(default) => (self.warmup(*default)?, self.static_value(*default)?),
                    None => (0, None),
                };
                if let Some(values) = self.parameter_warmups.last_mut() {
                    values.insert(parameter, warmup);
                }
                if let Some(value) = value
                    && let Some(values) = self.parameter_values.last_mut()
                {
                    values.insert(parameter, value);
                }
            }
            self.warmup_block(&function.body)
        })();
        self.parameter_warmups.pop();
        self.parameter_values.pop();
        result.map(|warmup| argument_warmups.into_iter().max().unwrap_or(0).max(warmup))
    }

    fn parameter_warmup(&mut self, slot: LocalSlot) -> Result<Option<usize>, PlanningError> {
        self.budget.charge(self.parameter_warmups.len())?;
        Ok(self
            .parameter_warmups
            .iter()
            .rev()
            .find_map(|values| values.get(&slot).copied()))
    }

    fn parameter_value(&mut self, slot: LocalSlot) -> Result<Option<Value>, PlanningError> {
        self.budget.charge(self.parameter_values.len())?;
        self.parameter_values
            .iter()
            .rev()
            .find_map(|values| values.get(&slot))
            .map(|value| self.budget.copy_value(value))
            .transpose()
    }

    fn input_value(&mut self, slot: LocalSlot) -> Result<Option<Value>, PlanningError> {
        self.budget.charge(self.program.inputs.len())?;
        let Some((_, name)) = self
            .program
            .inputs
            .iter()
            .find(|(slot_id, _)| *slot_id == slot)
        else {
            return Ok(None);
        };
        self.budget.charge(self.descriptor.inputs.len())?;
        let Some(descriptor) = self
            .descriptor
            .inputs
            .iter()
            .find(|input| &input.id == name)
        else {
            return Ok(None);
        };
        let value = self
            .config
            .values
            .get(&descriptor.id)
            .unwrap_or(&descriptor.default);
        if let InputValue::String(text) = value {
            self.budget.charge(text.len())?;
        }
        Ok(Some(value.clone().into_value()))
    }

    fn static_value(&mut self, expr: ExprId) -> Result<Option<Value>, PlanningError> {
        self.visit(|evaluator| evaluator.static_value_inner(expr))
    }

    fn static_value_inner(&mut self, expr: ExprId) -> Result<Option<Value>, PlanningError> {
        let root_context = self.parameter_warmups.is_empty() && self.parameter_values.is_empty();
        if root_context && let Some(value) = self.static_value_cache.get(&expr) {
            return value
                .as_ref()
                .map(|value| self.budget.copy_value(value))
                .transpose();
        }
        let Some(expression) = self.program.expressions.get(expr.0) else {
            return Ok(None);
        };
        let value = match &expression.kind {
            ExpressionKind::Constant(constant) => self
                .program
                .constants
                .get(constant.0)
                .map(|value| self.budget.copy_value(value))
                .transpose()?,
            ExpressionKind::LoadInput(slot) => self.input_value(*slot)?,
            ExpressionKind::LoadLocal(slot) => {
                if let Some(value) = self.parameter_value(*slot)? {
                    return Ok(Some(value));
                }
                if let Some(Some(definition)) = self.program.local_definitions.get(slot.0) {
                    if !self.visiting.insert(expr) {
                        return Ok(None);
                    }
                    let value = self.static_value(*definition)?;
                    self.visiting.remove(&expr);
                    value
                } else {
                    self.input_value(*slot)?
                }
            }
            ExpressionKind::Unary { operator, operand } => self
                .static_value(*operand)?
                .and_then(|value| value.unary(*operator)),
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => match self.static_value(*left)? {
                Some(left) => self
                    .static_value(*right)?
                    .map(|right| left.binary(*operator, right)),
                None => None,
            },
            ExpressionKind::Conditional {
                condition,
                when_true,
                when_false,
            } => match self.static_value(*condition)? {
                Some(Value::Bool(true)) => self.static_value(*when_true)?,
                Some(Value::Bool(false)) => self.static_value(*when_false)?,
                _ => None,
            },
            _ => None,
        };
        if root_context {
            let cached = value
                .as_ref()
                .map(|value| self.budget.copy_value(value))
                .transpose()?;
            self.static_value_cache.insert(expr, cached);
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::{IndicatorConfig, InputValue, compile};
    use std::path::Path;

    #[test]
    fn warmup_tracks_offsets_lengths_and_function_intrinsics() {
        let source = r#"
            language(1)
            indicator("Warmup", id = "community.warmup")
            format(precision = "base_ticker_min_tick")
            length = input.int(2, "Length", min = 1, max = 200)
            average = ta.sma(chart.close, length)
            shifted = chart.close[3]
            plot(average, id = "average")
            plot(shifted, id = "shifted")
        "#;
        let compiled =
            compile(source, Path::new("warmup.ind")).expect("warmup source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(3));
        let config = IndicatorConfig::default().with("length", InputValue::Int(5));
        assert_eq!(compiled.warmup_bars(&config), Some(5));

        let capped = r#"
            language(1)
            indicator("Capped warmup", id = "community.capped-warmup", max_bars_back = 3)
            format(precision = "base_ticker_min_tick")
            length = input.int(2, "Length", min = 1)
            plot(ta.sma(chart.close, length), id = "average")
        "#;
        let compiled = compile(capped, Path::new("capped-warmup.ind"))
            .expect("capped warmup source should compile");
        let config = IndicatorConfig::default().with("length", InputValue::Int(100_000));
        assert_eq!(compiled.warmup_bars(&config), Some(5_000));

        let wide = r#"
            language(1)
            indicator("Wide warmup", id = "community.wide-warmup", max_bars_back = 50000)
            format(precision = "base_ticker_min_tick")
            length = input.int(2, "Length", min = 1)
            plot(ta.sma(chart.close, length), id = "average")
        "#;
        let compiled =
            compile(wide, Path::new("wide-warmup.ind")).expect("wide warmup should compile");
        assert_eq!(compiled.warmup_bars(&config), Some(5_000));
        assert_eq!(
            compiled.warmup_bars_with_limits(
                &config,
                crate::RuntimeLimits {
                    max_window_length: 7,
                    ..crate::RuntimeLimits::default()
                }
            ),
            Some(7)
        );

        let chained = r#"
            language(1)
            indicator("Chained warmup", id = "community.chained-warmup")
            format(precision = "base_ticker_min_tick")
            first = ta.sma(chart.close, 5000)
            shared = first + first
            plot(ta.sma(first, 5000), id = "average")
            plot(shared, id = "shared")
        "#;
        let compiled = compile(chained, Path::new("chained-warmup.ind"))
            .expect("chained warmup should compile");
        assert_eq!(
            compiled.warmup_bars(&IndicatorConfig::default()),
            Some(10_000)
        );

        let source = r#"
            language(1)
            indicator("Function warmup", id = "community.function-warmup")
            format(precision = "base_ticker_min_tick")
            fn average(value) {
                return ta.sma(value, 7)
            }
            plot(average(chart.close))
        "#;
        let compiled = compile(source, Path::new("function-warmup.ind"))
            .expect("function source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(7));

        let source = r#"
            language(1)
            indicator("Parameterized warmup", id = "community.parameterized-warmup")
            format(precision = "base_ticker_min_tick")
            fn average(value) {
                return ta.sma(value, 7)
            }
            short = average(chart.close)
            long = average(ta.sma(chart.close, 3))
            plot(short, id = "short")
            plot(long, id = "long")
        "#;
        let compiled = compile(source, Path::new("parameterized-warmup.ind"))
            .expect("parameterized function source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(10));
    }

    #[test]
    fn dynamic_history_uses_the_declared_history_bound() {
        let source = r#"
            language(1)
            indicator("Dynamic history", id = "community.dynamic-history", max_bars_back = 7)
            format(precision = "base_ticker_min_tick")
            offset = bar_index % 5
            plot(chart.close[offset], id = "value")
        "#;
        let compiled = compile(source, Path::new("dynamic-history.ind"))
            .expect("dynamic history source should compile");

        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(7));
    }

    #[test]
    fn anchor_start_resolves_day_and_week_boundaries_from_config() {
        let source = r#"
            language(1)
            indicator("VWAP", id = "community.vwap")
            format(precision = "base_ticker_min_tick")
            anchor = input.string("Day", "Anchor", options = ["Day", "Week", "Month"])
            value = ta.avwap(chart.close, chart.volume, anchor)
            plot(value, id = "value")
        "#;
        let compiled = compile(source, Path::new("vwap.ind")).expect("vwap source should compile");
        // 2024-01-03 12:00 UTC is a Wednesday.
        let reference = 1_704_283_200_000;
        assert_eq!(
            compiled.anchor_start(&IndicatorConfig::default(), reference),
            Some(1_704_240_000_000)
        );
        let week = IndicatorConfig::default().with("anchor", InputValue::String("Week".to_owned()));
        assert_eq!(
            compiled.anchor_start(&week, reference),
            Some(1_704_067_200_000)
        );
        let plain = r#"
            language(1)
            indicator("Plain", id = "community.plain")
            format(precision = "base_ticker_min_tick")
            plot(chart.close)
        "#;
        let compiled = compile(plain, Path::new("plain.ind")).expect("plain source should compile");
        assert_eq!(
            compiled.anchor_start(&IndicatorConfig::default(), reference),
            None
        );
    }
}
