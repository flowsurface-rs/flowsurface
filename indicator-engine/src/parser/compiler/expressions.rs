use super::super::ast::{AstCall, AstExpression, constant_value, named_color};
use super::context::{
    LoweringContext, is_boolean_type, is_numeric_type, is_orderable_type, merge_value_types,
};
use crate::diagnostics::Span;
use crate::frame::FrameField;
use crate::hir::*;
use crate::types::*;
use std::collections::BTreeSet;

impl LoweringContext {
    pub(super) fn compile_expression(&mut self, expression: &AstExpression) -> ExprId {
        match expression {
            AstExpression::Number(value, _) => {
                let value = if value.fract() == 0.0 {
                    Value::Int(*value as i64)
                } else {
                    Value::Float(*value)
                };
                let ty = match value {
                    Value::Int(_) => Type::simple(ValueType::Int),
                    _ => Type::simple(ValueType::Float),
                };
                self.push_constant(value, ty)
            }
            AstExpression::String(value, _) => self.push_constant(
                Value::String(value.clone()),
                Type::simple(ValueType::String),
            ),
            AstExpression::Boolean(value, _) => {
                self.push_constant(Value::Bool(*value), Type::simple(ValueType::Bool))
            }
            AstExpression::Na(_) => self.push_constant(Value::Na, Type::series(ValueType::Any)),
            AstExpression::Identifier(path, span) => self.compile_identifier(path, *span),
            AstExpression::Unary {
                operator,
                expression,
                span,
            } => {
                let operand = self.compile_expression(expression);
                let operand_type = self.ir.program.expressions[operand.0].ty;
                let result = match operator {
                    UnaryOperator::Not if operand_type.value == ValueType::Bool => Type {
                        value: ValueType::Bool,
                        qualifier: operand_type.qualifier,
                    },
                    UnaryOperator::Plus | UnaryOperator::Minus
                        if matches!(operand_type.value, ValueType::Int | ValueType::Float) =>
                    {
                        operand_type
                    }
                    _ => {
                        self.error(*span, "invalid unary operand");
                        Type::simple(ValueType::Any)
                    }
                };
                self.push_expression(
                    ExpressionKind::Unary {
                        operator: *operator,
                        operand,
                    },
                    result,
                )
            }
            AstExpression::Binary {
                left,
                operator,
                right,
                span,
            } => {
                let left = self.compile_expression(left);
                let right = self.compile_expression(right);
                let left_type = self.ir.program.expressions[left.0].ty;
                let right_type = self.ir.program.expressions[right.0].ty;
                let qualifier = if left_type.qualifier == Qualifier::Series
                    || right_type.qualifier == Qualifier::Series
                {
                    Qualifier::Series
                } else {
                    Qualifier::Simple
                };
                let value = match operator {
                    BinaryOperator::Equal | BinaryOperator::NotEqual => ValueType::Bool,
                    BinaryOperator::And | BinaryOperator::Or => {
                        if !is_boolean_type(left_type.value) || !is_boolean_type(right_type.value) {
                            self.error(*span, "logical operands must be boolean");
                        }
                        ValueType::Bool
                    }
                    BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual => {
                        if !is_orderable_type(left_type.value)
                            || !is_orderable_type(right_type.value)
                        {
                            self.error(*span, "ordering operands must be numeric or time");
                        }
                        ValueType::Bool
                    }
                    _ if is_numeric_type(left_type.value) && is_numeric_type(right_type.value) => {
                        if left_type.value == ValueType::Float
                            || right_type.value == ValueType::Float
                            || left_type.value == ValueType::Any
                            || right_type.value == ValueType::Any
                        {
                            ValueType::Float
                        } else {
                            ValueType::Int
                        }
                    }
                    _ => {
                        self.error(*span, "invalid binary operands");
                        ValueType::Any
                    }
                };
                self.push_expression(
                    ExpressionKind::Binary {
                        operator: *operator,
                        left,
                        right,
                    },
                    Type { value, qualifier },
                )
            }
            AstExpression::Call(call) => self.compile_call(call),
            AstExpression::Tuple(expressions, _) => {
                let expressions = expressions
                    .iter()
                    .map(|expression| self.compile_expression(expression))
                    .collect::<Vec<_>>();
                let qualifier = if expressions.iter().any(|expression| {
                    self.ir.program.expressions[expression.0].ty.qualifier == Qualifier::Series
                }) {
                    Qualifier::Series
                } else {
                    Qualifier::Simple
                };
                self.push_expression(
                    ExpressionKind::Tuple(expressions),
                    Type {
                        value: ValueType::Tuple,
                        qualifier,
                    },
                )
            }
            AstExpression::Array(expressions, _) => {
                let expressions = expressions
                    .iter()
                    .map(|expression| self.compile_expression(expression))
                    .collect::<Vec<_>>();
                let qualifier = if expressions.iter().any(|expression| {
                    self.ir.program.expressions[expression.0].ty.qualifier == Qualifier::Series
                }) {
                    Qualifier::Series
                } else {
                    Qualifier::Simple
                };
                self.push_expression(
                    ExpressionKind::Array(expressions),
                    Type {
                        value: ValueType::Array,
                        qualifier,
                    },
                )
            }
            AstExpression::Record(fields, _) => {
                let fields = fields
                    .iter()
                    .map(|(name, expression)| (name.clone(), self.compile_expression(expression)))
                    .collect::<Vec<_>>();
                let qualifier = if fields.iter().any(|(_, expression)| {
                    self.ir.program.expressions[expression.0].ty.qualifier == Qualifier::Series
                }) {
                    Qualifier::Series
                } else {
                    Qualifier::Simple
                };
                self.push_expression(
                    ExpressionKind::Record(fields),
                    Type {
                        value: ValueType::Record,
                        qualifier,
                    },
                )
            }
            AstExpression::Switch {
                operand,
                branches,
                fallback,
                span,
            } => {
                let operand = self.compile_expression(operand);
                let branches = branches
                    .iter()
                    .map(|(condition, result)| {
                        (
                            self.compile_expression(condition),
                            self.compile_expression(result),
                        )
                    })
                    .collect::<Vec<_>>();
                let fallback = fallback
                    .as_deref()
                    .map(|expression| self.compile_expression(expression));
                let mut result_type = fallback
                    .map(|expression| self.ir.program.expressions[expression.0].ty.value)
                    .unwrap_or(ValueType::Any);
                let mut qualifier = self.ir.program.expressions[operand.0].ty.qualifier;
                for (condition, result) in &branches {
                    if self.ir.program.expressions[condition.0].ty.value
                        != self.ir.program.expressions[operand.0].ty.value
                        && self.ir.program.expressions[condition.0].ty.value != ValueType::Any
                        && self.ir.program.expressions[operand.0].ty.value != ValueType::Any
                    {
                        self.error(*span, "switch branch condition has the wrong type");
                    }
                    result_type = self.merge_result_type(
                        result_type,
                        self.ir.program.expressions[result.0].ty.value,
                        *span,
                        "switch result branches",
                    );
                    qualifier = if qualifier == Qualifier::Series
                        || self.ir.program.expressions[condition.0].ty.qualifier
                            == Qualifier::Series
                        || self.ir.program.expressions[result.0].ty.qualifier == Qualifier::Series
                    {
                        Qualifier::Series
                    } else {
                        Qualifier::Simple
                    };
                }
                self.push_expression(
                    ExpressionKind::Switch {
                        operand,
                        branches,
                        fallback,
                    },
                    Type {
                        value: result_type,
                        qualifier,
                    },
                )
            }
            AstExpression::Index {
                expression,
                index,
                span,
            } => {
                let source = self.compile_expression(expression);
                let offset = self.compile_expression(index);
                let source_type = self.ir.program.expressions[source.0].ty;
                let offset_type = self.ir.program.expressions[offset.0].ty;
                if matches!(source_type.value, ValueType::Tuple | ValueType::Array) {
                    if offset_type.value != ValueType::Int {
                        self.error(*span, "collection indexing requires an integer index");
                    }
                    return self.push_expression(
                        ExpressionKind::CollectionIndex {
                            source,
                            index: offset,
                        },
                        Type::series(self.collection_element_type(source)),
                    );
                }
                if source_type.qualifier != Qualifier::Series || offset_type.value != ValueType::Int
                {
                    self.error(
                        *span,
                        "history indexing requires a series and integer offset",
                    );
                }
                match constant_value(index) {
                    Some(Value::Int(offset)) if offset >= 0 => {
                        if let Some(descriptor) = self.declarations.descriptor.as_mut() {
                            descriptor.max_bars_back = descriptor
                                .max_bars_back
                                .max(offset.saturating_add(1) as usize);
                        }
                    }
                    Some(Value::Int(_)) => self.error(*span, "history offset cannot be negative"),
                    _ if !self.declarations.max_bars_back_explicit => self.error(
                        *span,
                        "dynamic history offsets require max_bars_back in indicator declaration",
                    ),
                    _ => {}
                }
                let needs_history = !matches!(constant_value(index), Some(Value::Int(0)));
                let history =
                    self.push_expression(ExpressionKind::History { source, offset }, source_type);
                if source_type.qualifier == Qualifier::Series
                    && offset_type.value == ValueType::Int
                    && needs_history
                {
                    self.ir.history_expressions.push((history, *span));
                }
                history
            }
            AstExpression::Field {
                expression,
                field,
                span,
            } => {
                let source = self.compile_expression(expression);
                let source_type = self.ir.program.expressions[source.0].ty;
                let Some(field_kind) = self.record_field_for_expression(source, field) else {
                    self.error(*span, format!("unknown record field {field}"));
                    return self.push_constant(Value::Na, Type::series(ValueType::Any));
                };
                if !matches!(source_type.value, ValueType::Record | ValueType::Any) {
                    self.error(*span, "field access requires a record");
                }
                self.push_expression(
                    ExpressionKind::RecordField {
                        source,
                        field: field.clone(),
                    },
                    Type::series(record_field_type(field_kind)),
                )
            }
        }
    }

    pub(super) fn compile_identifier(&mut self, path: &[String], span: Span) -> ExprId {
        let name = path.join(".");
        if let Some(value) = named_color(&name) {
            return self.push_constant(Value::Color(value), Type::simple(ValueType::Color));
        }
        if name == "bar_index" {
            return self
                .push_expression(ExpressionKind::LoadBarIndex, Type::series(ValueType::Int));
        }
        if let Some((slot, ty)) = self.scope.values.get(&name).copied() {
            let series_slot = (ty.qualifier == Qualifier::Series)
                .then(|| self.scope.local_definitions.get(slot.0).copied().flatten())
                .flatten()
                .filter(|_| {
                    !self
                        .scope
                        .mutable_values
                        .get(slot.0)
                        .copied()
                        .unwrap_or(false)
                })
                .and_then(|definition| self.ir.program.expressions[definition.0].series_slot);
            return self.push_expression_with_series_slot(
                ExpressionKind::LoadLocal(slot),
                ty,
                series_slot,
            );
        }
        if let Some((slot, ty)) = self.scope.states.get(&name).copied() {
            return self.push_expression(ExpressionKind::LoadState(slot), ty);
        }
        if path.len() == 2
            && let Some(field_kind) = record_field(&path[1])
        {
            let source = if let Some((slot, ty)) = self.scope.values.get(&path[0]).copied() {
                if !matches!(ty.value, ValueType::Record | ValueType::Any) {
                    self.error(span, "field access requires a record");
                }
                Some((
                    self.push_expression(ExpressionKind::LoadLocal(slot), ty),
                    ty,
                ))
            } else if let Some((slot, ty)) = self.scope.states.get(&path[0]).copied() {
                if !matches!(ty.value, ValueType::Record | ValueType::Any) {
                    self.error(span, "field access requires a record");
                }
                Some((
                    self.push_expression(ExpressionKind::LoadState(slot), ty),
                    ty,
                ))
            } else {
                None
            };
            if let Some((source, _)) = source {
                return self.push_expression(
                    ExpressionKind::RecordField {
                        source,
                        field: path[1].clone(),
                    },
                    Type::series(record_field_type(field_kind)),
                );
            }
        }
        let frame = match name.as_str() {
            "chart" => Some((FrameField::Chart, ValueType::Record)),
            "chart.time" => Some((FrameField::ChartTime, ValueType::Time)),
            "chart.open" => Some((FrameField::ChartOpen, ValueType::Float)),
            "chart.high" => Some((FrameField::ChartHigh, ValueType::Float)),
            "chart.low" => Some((FrameField::ChartLow, ValueType::Float)),
            "chart.close" => Some((FrameField::ChartClose, ValueType::Float)),
            "chart.hl2" => Some((FrameField::ChartHl2, ValueType::Float)),
            "chart.hlc3" => Some((FrameField::ChartHlc3, ValueType::Float)),
            "chart.ohlc4" => Some((FrameField::ChartOhlc4, ValueType::Float)),
            "chart.hlcc4" => Some((FrameField::ChartHlcc4, ValueType::Float)),
            "chart.volume" => Some((FrameField::ChartVolume, ValueType::Float)),
            "chart.directional_volume_available" => {
                Some((FrameField::ChartDirectionalVolumeAvailable, ValueType::Bool))
            }
            "chart.buy_qty" => Some((FrameField::ChartBuyQty, ValueType::Float)),
            "chart.sell_qty" => Some((FrameField::ChartSellQty, ValueType::Float)),
            "chart.trade" => Some((FrameField::ChartTrade, ValueType::Record)),
            "chart.trade.available" => Some((FrameField::ChartTradeAvailable, ValueType::Bool)),
            "chart.trade.buy_qty" => Some((FrameField::ChartTradeBuyQty, ValueType::Float)),
            "chart.trade.sell_qty" => Some((FrameField::ChartTradeSellQty, ValueType::Float)),
            "chart.trade.buy_count" => Some((FrameField::ChartTradeBuyCount, ValueType::Int)),
            "chart.trade.sell_count" => Some((FrameField::ChartTradeSellCount, ValueType::Int)),
            "chart.trade.count" => Some((FrameField::ChartTradeCount, ValueType::Int)),
            "instrument.min_tick" => Some((FrameField::InstrumentMinTick, ValueType::Float)),
            "open_interest" => Some((FrameField::OpenInterest, ValueType::Float)),
            _ => None,
        };
        if let Some((field, value_type)) = frame {
            match field {
                FrameField::Chart
                | FrameField::ChartTime
                | FrameField::ChartOpen
                | FrameField::ChartHigh
                | FrameField::ChartLow
                | FrameField::ChartClose
                | FrameField::ChartHl2
                | FrameField::ChartHlc3
                | FrameField::ChartOhlc4
                | FrameField::ChartHlcc4
                | FrameField::ChartVolume => {
                    self.declarations.requirements.ohlc = true;
                }
                FrameField::InstrumentMinTick => {}
                FrameField::ChartTrade
                | FrameField::ChartTradeAvailable
                | FrameField::ChartTradeBuyQty
                | FrameField::ChartTradeSellQty
                | FrameField::ChartTradeBuyCount
                | FrameField::ChartTradeSellCount
                | FrameField::ChartTradeCount => {
                    self.declarations.requirements.trades = true;
                }
                FrameField::ChartDirectionalVolumeAvailable
                | FrameField::ChartBuyQty
                | FrameField::ChartSellQty => {
                    self.declarations.requirements.directional_volume = true;
                }
                FrameField::OpenInterest => self.declarations.requirements.open_interest = true,
            }
            let value_type = if matches!(field, FrameField::InstrumentMinTick) {
                Type::simple(value_type)
            } else {
                Type::series(value_type)
            };
            return self.push_expression(ExpressionKind::LoadFrame(field), value_type);
        }
        let bar_state = match name.as_str() {
            "barstate.isfirst" => Some(BarStateField::IsFirst),
            "barstate.islast" => Some(BarStateField::IsLast),
            "barstate.isconfirmed" => Some(BarStateField::IsConfirmed),
            "barstate.isrealtime" => Some(BarStateField::IsRealtime),
            _ => None,
        };
        if let Some(field) = bar_state {
            return self.push_expression(
                ExpressionKind::LoadBarState(field),
                Type::series(ValueType::Bool),
            );
        }
        if path.len() == 2 {
            let source = self
                .scope
                .values
                .get(&path[0])
                .copied()
                .map(|(slot, ty)| {
                    (
                        self.push_expression(ExpressionKind::LoadLocal(slot), ty),
                        ty,
                    )
                })
                .or_else(|| {
                    self.scope.states.get(&path[0]).copied().map(|(slot, ty)| {
                        (
                            self.push_expression(ExpressionKind::LoadState(slot), ty),
                            ty,
                        )
                    })
                });
            if let Some((source, source_type)) = source {
                let namespace = match source_type.value {
                    ValueType::Record => Some("record"),
                    _ => None,
                };
                if let Some(namespace) = namespace
                    && let Some((intrinsic, result)) = self
                        .registry
                        .find(&format!("{namespace}.get"))
                        .map(|(intrinsic, definition)| (intrinsic, definition.signature.result))
                {
                    let key = self.push_constant(
                        Value::String(path[1].clone()),
                        Type::simple(ValueType::String),
                    );
                    let call_site = CallSiteId(self.ir.next_call_site);
                    self.ir.next_call_site += 1;
                    return self.push_expression(
                        ExpressionKind::IntrinsicCall {
                            intrinsic,
                            call_site,
                            arguments: vec![source, key],
                        },
                        result,
                    );
                }
            }
        }
        self.error(span, format!("unknown name {name}"));
        self.push_constant(Value::Na, Type::series(ValueType::Any))
    }
    pub(super) fn record_field_for_expression(
        &self,
        source: ExprId,
        name: &str,
    ) -> Option<RecordField> {
        if self.is_trade_record_expression(source) {
            return trade_field(name);
        }
        record_field(name)
    }

    pub(super) fn is_trade_record_expression(&self, expression: ExprId) -> bool {
        matches!(
            self.ir.program
            .expressions
                .get(expression.0)
                .map(|expression| &expression.kind),
            Some(ExpressionKind::RecordField { field, .. }) if field == "trade"
        )
    }

    pub(super) fn collection_element_type(&self, source: ExprId) -> ValueType {
        let kind = &self.ir.program.expressions[source.0].kind;
        let elements = match kind {
            ExpressionKind::Tuple(elements) | ExpressionKind::Array(elements) => elements,
            _ => return ValueType::Any,
        };
        let mut value_type = None;
        for expression in elements {
            let current = self.ir.program.expressions[expression.0].ty.value;
            value_type =
                Some(value_type.map_or(current, |previous| merge_value_types(previous, current)));
        }
        value_type.unwrap_or(ValueType::Any)
    }

    pub(super) fn is_numeric_function_expression(&self, expression: ExprId) -> bool {
        let Some(expression) = self.ir.program.expressions.get(expression.0) else {
            return false;
        };
        if matches!(expression.ty.value, ValueType::Int | ValueType::Float) {
            return true;
        }
        matches!(
            expression.kind,
            ExpressionKind::Constant(constant)
                if matches!(self.ir.program.constants.get(constant.0), Some(Value::Na))
        )
    }

    pub(super) fn compile_call(&mut self, call: &AstCall) -> ExprId {
        let name = call.path.join(".");
        if unsupported_call(&name) {
            self.error(
                call.span,
                format!("{name} is not available in the live chart runtime"),
            );
            return self.push_constant(Value::Na, Type::series(ValueType::Any));
        }
        if name.starts_with("input.") {
            if !self.is_top_level_declaration() {
                self.error(call.span, "input declarations must be top-level");
            } else {
                self.error(call.span, "inputs must be assigned to a named binding");
            }
            return self.push_constant(Value::Na, Type::simple(ValueType::Any));
        }
        if let Some(function) = self.function.by_name.get(&name).copied() {
            if let Some(caller) = self.function.current
                && let Some(edges) = self.function.call_edges.get_mut(caller.0)
                && let Err(index) = edges.binary_search(&function)
            {
                edges.insert(index, function);
            }
            if self.function.active.contains(&function) {
                self.error(
                    call.span,
                    "recursive functions are not supported in language version 1",
                );
                return self.push_constant(Value::Na, Type::series(ValueType::Any));
            }
            let (expected, required, parameter_names, signature_ready) = self
                .ir
                .program
                .functions
                .get(function.0)
                .map(|function| {
                    (
                        function.parameters.len(),
                        function
                            .parameter_defaults
                            .iter()
                            .filter(|default| default.is_none())
                            .count(),
                        function.parameter_names.clone(),
                        !function.body.is_empty() || !function.parameter_names.is_empty(),
                    )
                })
                .unwrap_or((0, 0, Vec::new(), false));
            let (parameter_names, required) = if parameter_names.is_empty() {
                self.function
                    .signatures
                    .get(function.0)
                    .cloned()
                    .unwrap_or((parameter_names, required))
            } else {
                (parameter_names, required)
            };
            let expected = parameter_names.len().max(expected);
            let mut positional_index = 0usize;
            let mut compiled_arguments = Vec::new();
            for argument in &call.arguments {
                let index = if let Some(argument_name) = argument.name.as_deref() {
                    let Some(index) = parameter_names
                        .iter()
                        .position(|name| name == argument_name)
                    else {
                        self.error(
                            argument.expression.span(),
                            format!("unknown named argument {argument_name} to function {name}"),
                        );
                        continue;
                    };
                    index
                } else {
                    let index = positional_index;
                    positional_index += 1;
                    index
                };
                let expression = self.compile_expression(&argument.expression);
                if !self.is_numeric_function_expression(expression) {
                    self.error(
                        argument.expression.span(),
                        format!(
                            "argument {} to function {name} must be numeric or na",
                            index + 1
                        ),
                    );
                }
                compiled_arguments.push((index, expression));
            }
            let mut seen_arguments = BTreeSet::new();
            for (index, _) in &compiled_arguments {
                if !seen_arguments.insert(*index) {
                    self.error(call.span, "function arguments cannot be supplied twice");
                }
            }
            if signature_ready
                && (positional_index > expected || compiled_arguments.len() > expected)
            {
                self.error(
                    call.span,
                    format!(
                        "function {name} expects at most {expected} arguments, got {}",
                        call.arguments.len()
                    ),
                );
            }
            let provided = compiled_arguments
                .iter()
                .map(|(index, _)| *index)
                .collect::<std::collections::BTreeSet<_>>();
            if signature_ready && provided.len() < required {
                self.error(
                    call.span,
                    format!(
                        "function {name} requires {required} arguments, got {}",
                        provided.len()
                    ),
                );
            }
            compiled_arguments.sort_by_key(|(index, _)| *index);
            let argument_slots = compiled_arguments.iter().map(|(index, _)| *index).collect();
            let arguments = compiled_arguments
                .into_iter()
                .map(|(_, expression)| expression)
                .collect();
            let call_site = CallSiteId(self.ir.next_call_site);
            self.ir.next_call_site += 1;
            let result = self
                .ir
                .program
                .functions
                .get(function.0)
                .map(|function| function.return_type)
                .unwrap_or(Type::series(ValueType::Any));
            return self.push_expression(
                ExpressionKind::FunctionCall {
                    function,
                    call_site,
                    arguments,
                    argument_slots,
                },
                result,
            );
        }
        let Some((intrinsic, definition)) = self.registry.find(&name) else {
            self.error(call.span, format!("unknown intrinsic {name}"));
            return self.push_constant(Value::Na, Type::series(ValueType::Any));
        };
        let signature = definition.signature.clone();
        if !signature.variadic && call.arguments.len() != signature.arguments.len() {
            self.error(
                call.span,
                format!(
                    "{name} expects {} arguments, got {}",
                    signature.arguments.len(),
                    call.arguments.len()
                ),
            );
        } else if signature.variadic && call.arguments.len() < signature.min_arguments {
            self.error(
                call.span,
                format!(
                    "{name} expects at least {} arguments, got {}",
                    signature.min_arguments,
                    call.arguments.len()
                ),
            );
        }
        let mut arguments = Vec::new();
        let mut branch_value_type = None;
        let mut branch_fallback_type = None;
        let mut positional_index = 0usize;
        let mut compiled_arguments = Vec::new();
        for argument in &call.arguments {
            let index = match argument.name.as_deref() {
                Some(argument_name) => match signature
                    .arguments
                    .iter()
                    .position(|expected| expected.name == argument_name)
                {
                    Some(index) => index,
                    None => {
                        self.error(
                            argument.expression.span(),
                            format!("unknown named argument {argument_name} to {name}"),
                        );
                        let index = positional_index;
                        positional_index += 1;
                        index
                    }
                },
                None => {
                    let index = positional_index;
                    positional_index += 1;
                    index
                }
            };
            let expression = self.compile_expression(&argument.expression);
            let actual = self.ir.program.expressions[expression.0].ty;
            if index == 1 {
                branch_value_type = Some(actual.value);
            }
            if index == 2 {
                branch_fallback_type = Some(actual.value);
            }
            if let Some(expected) = signature.arguments.get(index)
                && !(expected.ty.accepts(actual)
                    || expected.ty.value == ValueType::Float && actual.value == ValueType::Int)
            {
                self.error(
                    argument.expression.span(),
                    format!("argument {} to {name} has the wrong type", index + 1),
                );
            }
            compiled_arguments.push((index, expression));
        }
        let mut seen_arguments = BTreeSet::new();
        for (index, _) in &compiled_arguments {
            if !seen_arguments.insert(*index) {
                self.error(
                    call.span,
                    format!("arguments to {name} cannot be supplied twice"),
                );
            }
        }
        compiled_arguments.sort_by_key(|(index, _)| *index);
        for (index, expression) in compiled_arguments {
            while arguments.len() < index {
                arguments.push(self.push_constant(Value::Na, Type::series(ValueType::Any)));
            }
            arguments.push(expression);
        }
        if name == "select" && arguments.len() >= 3 {
            let value_type = self.merge_result_type(
                branch_value_type.unwrap_or(ValueType::Any),
                branch_fallback_type.unwrap_or(ValueType::Any),
                call.span,
                "select result branches",
            );
            return self.push_expression(
                ExpressionKind::Conditional {
                    condition: arguments[0],
                    when_true: arguments[1],
                    when_false: arguments[2],
                },
                Type::series(value_type),
            );
        }
        if name == "coalesce" && arguments.len() >= 2 {
            let value_type = self.merge_result_type(
                self.ir.program.expressions[arguments[0].0].ty.value,
                self.ir.program.expressions[arguments[1].0].ty.value,
                call.span,
                "coalesce result branches",
            );
            return self.push_expression(
                ExpressionKind::Coalesce {
                    source: arguments[0],
                    fallback: arguments[1],
                },
                Type::series(value_type),
            );
        }
        let call_site = CallSiteId(self.ir.next_call_site);
        self.ir.next_call_site += 1;
        let result = if matches!(name.as_str(), "select" | "coalesce") {
            let value_type = self.merge_result_type(
                branch_value_type.unwrap_or(ValueType::Any),
                branch_fallback_type.unwrap_or(ValueType::Any),
                call.span,
                "result branches",
            );
            Type::series(value_type)
        } else {
            signature.result
        };
        self.push_expression(
            ExpressionKind::IntrinsicCall {
                intrinsic,
                call_site,
                arguments,
            },
            result,
        )
    }
}

pub(super) fn unsupported_call(name: &str) -> bool {
    [
        "box.",
        "line.",
        "label.",
        "polyline.",
        "table.",
        "map.",
        "request.",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}
#[derive(Clone, Copy)]
pub(super) enum RecordField {
    Open,
    High,
    Low,
    Close,
    Volume,
    DirectionalVolumeAvailable,
    BuyQty,
    SellQty,
    TradeCount,
    Trade,
    TradeAvailable,
    TradeBuyQty,
    TradeSellQty,
    TradeBuyCount,
    TradeSellCount,
}

pub(super) fn record_field(name: &str) -> Option<RecordField> {
    match name {
        "open" => Some(RecordField::Open),
        "high" => Some(RecordField::High),
        "low" => Some(RecordField::Low),
        "close" => Some(RecordField::Close),
        "volume" => Some(RecordField::Volume),
        "directional_volume_available" => Some(RecordField::DirectionalVolumeAvailable),
        "buy_qty" => Some(RecordField::BuyQty),
        "sell_qty" => Some(RecordField::SellQty),
        "trade" => Some(RecordField::Trade),
        _ => None,
    }
}

pub(super) fn trade_field(name: &str) -> Option<RecordField> {
    match name {
        "available" => Some(RecordField::TradeAvailable),
        "buy_qty" => Some(RecordField::TradeBuyQty),
        "sell_qty" => Some(RecordField::TradeSellQty),
        "buy_count" => Some(RecordField::TradeBuyCount),
        "sell_count" => Some(RecordField::TradeSellCount),
        "count" => Some(RecordField::TradeCount),
        _ => None,
    }
}
pub(super) fn record_field_type(field: RecordField) -> ValueType {
    match field {
        RecordField::Open
        | RecordField::High
        | RecordField::Low
        | RecordField::Close
        | RecordField::Volume
        | RecordField::BuyQty
        | RecordField::SellQty
        | RecordField::TradeBuyQty
        | RecordField::TradeSellQty => ValueType::Float,
        RecordField::DirectionalVolumeAvailable | RecordField::TradeAvailable => ValueType::Bool,
        RecordField::TradeCount | RecordField::TradeBuyCount | RecordField::TradeSellCount => {
            ValueType::Int
        }
        RecordField::Trade => ValueType::Record,
    }
}
