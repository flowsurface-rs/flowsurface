//! Static history analysis over the typed HIR.
//!
//! Warmup bars and anchored history starts are derived from the compiled
//! expressions so hosts can plan fetches and gate availability per indicator
//! instead of falling back to the retained-history cap.

use crate::hir::{
    BinaryOperator, ExprId, ExpressionKind, HirProgram, LocalSlot, StatementId, StatementKind,
    UnaryOperator,
};
use crate::intrinsic::IntrinsicRegistry;
use crate::types::{IndicatorConfig, IndicatorDescriptor, Value};
use std::collections::{BTreeMap, HashSet};

pub(super) fn warmup_bars(
    program: &HirProgram,
    registry: &IntrinsicRegistry,
    descriptor: &IndicatorDescriptor,
    config: &IndicatorConfig,
    max_window_length: usize,
) -> Option<usize> {
    let mut evaluator = Evaluator::new(program, registry, descriptor, config, max_window_length);
    let warmup = program
        .blocks
        .get(program.entry_point.0)
        .map(|block| evaluator.warmup_block(&block.statements))
        .unwrap_or(0);
    Some(warmup)
}

/// Earliest timestamp the indicator must start evaluating from, driven by
/// anchored intrinsics such as `ta.avwap`. Resolves anchor names from the
/// configuration and returns the earliest UTC boundary across all sites.
pub(super) fn anchor_start(
    program: &HirProgram,
    registry: &IntrinsicRegistry,
    descriptor: &IndicatorDescriptor,
    config: &IndicatorConfig,
    reference_time: i64,
) -> Option<i64> {
    let (avwap, _) = registry.find("ta.avwap")?;
    let mut evaluator = Evaluator::new(program, registry, descriptor, config, usize::MAX);
    let mut earliest: Option<i64> = None;
    for expression in &program.expressions {
        if let ExpressionKind::IntrinsicCall {
            intrinsic,
            arguments,
            ..
        } = &expression.kind
            && *intrinsic == avwap
            && let Some(anchor) = arguments.get(2).copied()
            && let Some(Value::String(name)) = evaluator.static_value(anchor)
            && let Some(start) = crate::intrinsic::anchored_start(reference_time, &name)
        {
            earliest = Some(earliest.map_or(start, |current| current.min(start)));
        }
    }
    earliest
}

struct Evaluator<'a> {
    program: &'a HirProgram,
    registry: &'a IntrinsicRegistry,
    descriptor: &'a IndicatorDescriptor,
    config: &'a IndicatorConfig,
    max_window_length: usize,
    visiting: HashSet<ExprId>,
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
            visiting: HashSet::new(),
            parameter_warmups: Vec::new(),
            parameter_values: Vec::new(),
        }
    }

    fn warmup_block(&mut self, statements: &[StatementId]) -> usize {
        statements
            .iter()
            .map(|statement| self.warmup_statement(*statement))
            .max()
            .unwrap_or(0)
    }

    fn warmup_statement(&mut self, statement: StatementId) -> usize {
        let Some(statement) = self.program.statements.get(statement.0).cloned() else {
            return 0;
        };
        match statement.kind {
            StatementKind::Binding { expression, .. }
            | StatementKind::LocalUpdate { expression, .. }
            | StatementKind::StateDeclaration {
                initial: expression,
                ..
            }
            | StatementKind::StateUpdate { expression, .. }
            | StatementKind::Expression(expression)
            | StatementKind::Return(Some(expression))
            | StatementKind::Destructure { expression, .. } => self.warmup(expression),
            StatementKind::Plot {
                expression, color, ..
            }
            | StatementKind::HorizontalLevel {
                expression, color, ..
            } => self.warmup_all([Some(expression), color]),
            StatementKind::Marker {
                expression,
                visible,
                color,
                ..
            } => self.warmup_all([Some(expression), visible, color]),
            StatementKind::Segment {
                start_value,
                end_value,
                start_index,
                end_index,
                visible,
                color,
                ..
            } => self.warmup_all([
                Some(start_value),
                Some(end_value),
                Some(start_index),
                Some(end_index),
                visible,
                color,
            ]),
            StatementKind::OhlcPlot {
                open,
                high,
                low,
                close,
                color,
                ..
            } => self.warmup_all([Some(open), Some(high), Some(low), Some(close), color]),
            StatementKind::RangePlot {
                upper,
                lower,
                color,
                ..
            } => self.warmup_all([Some(upper), Some(lower), color]),
            StatementKind::If {
                condition,
                then_branch,
                else_branch,
            } => self
                .warmup(condition)
                .max(self.warmup_block(&then_branch))
                .max(self.warmup_block(&else_branch)),
            StatementKind::While { condition, body } => {
                self.warmup(condition).max(self.warmup_block(&body))
            }
            StatementKind::ForRange {
                start, end, body, ..
            } => self
                .warmup(start)
                .max(self.warmup(end))
                .max(self.warmup_block(&body)),
            StatementKind::ForEach {
                collection, body, ..
            } => self.warmup(collection).max(self.warmup_block(&body)),
            StatementKind::Break | StatementKind::Continue | StatementKind::Return(None) => 0,
        }
    }

    fn warmup_all<const N: usize>(&mut self, expressions: [Option<ExprId>; N]) -> usize {
        expressions
            .into_iter()
            .flatten()
            .map(|expression| self.warmup(expression))
            .max()
            .unwrap_or(0)
    }

    fn warmup(&mut self, expr: ExprId) -> usize {
        let Some(expression) = self.program.expressions.get(expr.0).cloned() else {
            return 0;
        };
        match expression.kind {
            ExpressionKind::Constant(_)
            | ExpressionKind::LoadBarIndex
            | ExpressionKind::LoadBarState(_)
            | ExpressionKind::LoadSeries(_)
            | ExpressionKind::LoadInput(_)
            | ExpressionKind::LoadState(_)
            | ExpressionKind::LoadFrame(_) => 0,
            ExpressionKind::RecordField { source, .. } => self.warmup(source),
            ExpressionKind::LoadLocal(slot) => {
                if let Some(warmup) = self.parameter_warmup(slot) {
                    return warmup;
                }
                let Some(Some(definition)) = self.program.local_definitions.get(slot.0) else {
                    return 0;
                };
                if !self.visiting.insert(expr) {
                    return 0;
                }
                let warmup = self.warmup(*definition);
                self.visiting.remove(&expr);
                warmup
            }
            ExpressionKind::History { source, offset } => {
                let offset = match self.static_value(offset).and_then(|value| value.as_float()) {
                    Some(value) if value >= 0.0 => value as usize,
                    _ => self.descriptor.max_bars_back,
                };
                offset.saturating_add(self.warmup(source))
            }
            ExpressionKind::Tuple(expressions) | ExpressionKind::Array(expressions) => expressions
                .iter()
                .map(|id| self.warmup(*id))
                .max()
                .unwrap_or(0),
            ExpressionKind::Record(fields) => fields
                .iter()
                .map(|(_, id)| self.warmup(*id))
                .max()
                .unwrap_or(0),
            ExpressionKind::Switch {
                operand,
                branches,
                fallback,
            } => {
                let mut warmup = self.warmup(operand);
                for (condition, result) in branches {
                    warmup = warmup.max(self.warmup(condition)).max(self.warmup(result));
                }
                if let Some(fallback) = fallback {
                    warmup = warmup.max(self.warmup(fallback));
                }
                warmup
            }
            ExpressionKind::CollectionIndex { source, index } => {
                self.warmup(source).max(self.warmup(index))
            }
            ExpressionKind::Unary { operand, .. } => self.warmup(operand),
            ExpressionKind::Binary { left, right, .. } => self.warmup(left).max(self.warmup(right)),
            ExpressionKind::Conditional {
                condition,
                when_true,
                when_false,
            } => self
                .warmup(condition)
                .max(self.warmup(when_true))
                .max(self.warmup(when_false)),
            ExpressionKind::Coalesce { source, fallback } => {
                self.warmup(source).max(self.warmup(fallback))
            }
            ExpressionKind::IntrinsicCall {
                intrinsic,
                arguments,
                ..
            } => {
                let Some(definition) = self.registry.get(intrinsic) else {
                    return arguments
                        .iter()
                        .map(|id| self.warmup(*id))
                        .max()
                        .unwrap_or(0);
                };
                let mut warmup = arguments
                    .iter()
                    .map(|id| self.warmup(*id))
                    .max()
                    .unwrap_or(0);
                if let Some(window) = definition.signature.window
                    && let Some(length) = arguments.get(window).copied()
                {
                    let length = self
                        .static_value(length)
                        .and_then(|value| value.as_float())
                        .filter(|value| *value >= 1.0)
                        .map(|value| value as usize)
                        .unwrap_or(1)
                        .min(self.max_window_length);
                    warmup = warmup.saturating_add(length);
                }
                warmup
            }
            ExpressionKind::FunctionCall {
                function,
                arguments,
                argument_slots,
                ..
            } => self.warmup_function(function, &arguments, &argument_slots),
        }
    }

    fn warmup_function(
        &mut self,
        function_id: crate::hir::FunctionId,
        arguments: &[ExprId],
        argument_slots: &[usize],
    ) -> usize {
        let argument_warmups = arguments
            .iter()
            .map(|argument| self.warmup(*argument))
            .collect::<Vec<_>>();
        let argument_values = arguments
            .iter()
            .map(|argument| self.static_value(*argument))
            .collect::<Vec<_>>();
        let Some(function) = self.program.functions.get(function_id.0).cloned() else {
            return argument_warmups.into_iter().max().unwrap_or(0);
        };
        let mut parameter_warmups = BTreeMap::new();
        let mut parameter_values = BTreeMap::new();
        for (argument_index, parameter_index) in argument_slots.iter().copied().enumerate() {
            let Some(parameter) = function.parameters.get(parameter_index).copied() else {
                continue;
            };
            parameter_warmups.insert(
                parameter,
                argument_warmups.get(argument_index).copied().unwrap_or(0),
            );
            if let Some(Some(value)) = argument_values.get(argument_index) {
                parameter_values.insert(parameter, value.clone());
            }
        }
        self.parameter_warmups.push(parameter_warmups);
        self.parameter_values.push(parameter_values);
        for (parameter_index, default) in function.parameter_defaults.iter().enumerate() {
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
            let warmup = default.map_or(0, |default| self.warmup(default));
            let value = default.and_then(|default| self.static_value(default));
            if let Some(values) = self.parameter_warmups.last_mut() {
                values.insert(parameter, warmup);
            }
            if let Some(value) = value
                && let Some(values) = self.parameter_values.last_mut()
            {
                values.insert(parameter, value);
            }
        }
        let warmup = self.warmup_block(&function.body);
        self.parameter_warmups.pop();
        self.parameter_values.pop();
        argument_warmups.into_iter().max().unwrap_or(0).max(warmup)
    }

    fn parameter_warmup(&self, slot: LocalSlot) -> Option<usize> {
        self.parameter_warmups
            .iter()
            .rev()
            .find_map(|values| values.get(&slot).copied())
    }

    fn parameter_value(&self, slot: LocalSlot) -> Option<Value> {
        self.parameter_values
            .iter()
            .rev()
            .find_map(|values| values.get(&slot).cloned())
    }

    fn input_value(&self, slot: crate::hir::LocalSlot) -> Option<Value> {
        let (_, name) = self
            .program
            .inputs
            .iter()
            .find(|(slot_id, _)| *slot_id == slot)?;
        let descriptor = self
            .descriptor
            .inputs
            .iter()
            .find(|input| &input.id == name)?;
        Some(self.config.value(descriptor).into_value())
    }

    fn static_value(&mut self, expr: ExprId) -> Option<Value> {
        let expression = self.program.expressions.get(expr.0)?.clone();
        match expression.kind {
            ExpressionKind::Constant(constant) => self.program.constants.get(constant.0).cloned(),
            ExpressionKind::LoadInput(slot) => self.input_value(slot),
            ExpressionKind::LoadLocal(slot) => {
                if let Some(value) = self.parameter_value(slot) {
                    return Some(value);
                }
                if let Some(Some(definition)) = self.program.local_definitions.get(slot.0) {
                    if !self.visiting.insert(expr) {
                        return None;
                    }
                    let value = self.static_value(*definition);
                    self.visiting.remove(&expr);
                    value
                } else {
                    self.input_value(slot)
                }
            }
            ExpressionKind::Unary { operator, operand } => {
                let value = self.static_value(operand)?;
                match operator {
                    UnaryOperator::Plus => Some(value),
                    UnaryOperator::Minus => value.as_float().map(|value| Value::Float(-value)),
                    UnaryOperator::Not => match value {
                        Value::Bool(value) => Some(Value::Bool(!value)),
                        _ => None,
                    },
                }
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => {
                let left = self.static_value(left)?;
                let right = self.static_value(right)?;
                static_binary(operator, left, right)
            }
            ExpressionKind::Conditional {
                condition,
                when_true,
                when_false,
            } => match self.static_value(condition)? {
                Value::Bool(true) => self.static_value(when_true),
                Value::Bool(false) => self.static_value(when_false),
                _ => None,
            },
            _ => None,
        }
    }
}

fn static_binary(operator: BinaryOperator, left: Value, right: Value) -> Option<Value> {
    use BinaryOperator::*;
    match operator {
        And => boolean_pair(left, right, |left, right| left && right),
        Or => boolean_pair(left, right, |left, right| left || right),
        Equal | NotEqual => {
            let equal = match left.equals(&right) {
                Some(equal) => equal,
                None => return Some(Value::Na),
            };
            Some(Value::Bool(if matches!(operator, Equal) {
                equal
            } else {
                !equal
            }))
        }
        Less | LessEqual | Greater | GreaterEqual => {
            let (Some(left), Some(right)) = (left.as_float(), right.as_float()) else {
                return None;
            };
            Some(Value::Bool(match operator {
                Less => left < right,
                LessEqual => left <= right,
                Greater => left > right,
                GreaterEqual => left >= right,
                _ => unreachable!(),
            }))
        }
        Add | Subtract | Multiply | Divide | Remainder => {
            let (Some(left), Some(right)) = (left.as_float(), right.as_float()) else {
                return None;
            };
            let value = match operator {
                Add => left + right,
                Subtract => left - right,
                Multiply => left * right,
                Divide if right != 0.0 => left / right,
                Remainder if right != 0.0 => left % right,
                _ => return None,
            };
            Some(Value::Float(value))
        }
    }
}

fn boolean_pair(left: Value, right: Value, operation: fn(bool, bool) -> bool) -> Option<Value> {
    let (Value::Bool(left), Value::Bool(right)) = (left, right) else {
        return None;
    };
    Some(Value::Bool(operation(left, right)))
}

#[cfg(test)]
mod tests {
    use crate::{IndicatorConfig, InputValue, compile};
    use std::path::Path;

    #[test]
    fn warmup_tracks_offsets_lengths_and_function_intrinsics() {
        let source = r#"
            language(1)
            indicator("Warmup", id = "community.warmup", value_precision = "base_ticker_min_tick")
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
            indicator("Capped warmup", id = "community.capped-warmup", max_bars_back = 3,
                value_precision = "base_ticker_min_tick")
            length = input.int(2, "Length", min = 1)
            plot(ta.sma(chart.close, length), id = "average")
        "#;
        let compiled = compile(capped, Path::new("capped-warmup.ind"))
            .expect("capped warmup source should compile");
        let config = IndicatorConfig::default().with("length", InputValue::Int(100_000));
        assert_eq!(compiled.warmup_bars(&config), Some(5_000));

        let wide = r#"
            language(1)
            indicator("Wide warmup", id = "community.wide-warmup", max_bars_back = 50000,
                value_precision = "base_ticker_min_tick")
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
            indicator("Chained warmup", id = "community.chained-warmup",
                value_precision = "base_ticker_min_tick")
            first = ta.sma(chart.close, 5000)
            plot(ta.sma(first, 5000), id = "average")
        "#;
        let compiled = compile(chained, Path::new("chained-warmup.ind"))
            .expect("chained warmup should compile");
        assert_eq!(
            compiled.warmup_bars(&IndicatorConfig::default()),
            Some(10_000)
        );

        let source = r#"
            language(1)
            indicator("Function warmup", id = "community.function-warmup", value_precision = "base_ticker_min_tick")
            fn average(value) {
                return ta.sma(value, 7)
            }
            plot(average(chart.close))
        "#;
        let compiled = compile(source, Path::new("function-warmup.ind"))
            .expect("function source should compile");
        assert_eq!(compiled.warmup_bars(&IndicatorConfig::default()), Some(7));
    }

    #[test]
    fn dynamic_history_uses_the_declared_history_bound() {
        let source = r#"
            language(1)
            indicator("Dynamic history", id = "community.dynamic-history",
                max_bars_back = 7, value_precision = "base_ticker_min_tick")
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
            indicator("VWAP", id = "community.vwap", value_precision = "base_ticker_min_tick")
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
            indicator("Plain", id = "community.plain", value_precision = "base_ticker_min_tick")
            plot(chart.close)
        "#;
        let compiled = compile(plain, Path::new("plain.ind")).expect("plain source should compile");
        assert_eq!(
            compiled.anchor_start(&IndicatorConfig::default(), reference),
            None
        );
    }
}
