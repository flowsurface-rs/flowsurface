use super::super::ast::{AstArgument, AstCall, constant_value};
use crate::DataRequirements;
use crate::diagnostics::{Diagnostic, Span};
use crate::hir::{
    CallSiteId, ConstantId, ExprId, ExpressionKind, FunctionId, HirProgram, LocalSlot, SeriesSlot,
    StateSlot, StatementKind, TypedExpression, TypedStatement,
};
use crate::intrinsic::IntrinsicRegistry;
use crate::scene::IndicatorPresentation;
use crate::types::{
    IndicatorDescriptor, InputDescriptor, Qualifier, SeriesInputDescriptor, Type, Value, ValueType,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(super) struct IrBuilder {
    pub(super) program: HirProgram,
    pub(super) history_expressions: Vec<(ExprId, Span)>,
    pub(super) next_series: usize,
    pub(super) next_call_site: usize,
    pub(super) state_call_sites: Vec<CallSiteId>,
    pub(super) state_functions: Vec<Option<FunctionId>>,
}

pub(super) struct ScopeState {
    pub(super) values: BTreeMap<String, (LocalSlot, Type)>,
    pub(super) states: BTreeMap<String, (StateSlot, Type)>,
    pub(super) local_definitions: Vec<Option<ExprId>>,
    pub(super) mutable_values: Vec<bool>,
    pub(super) next_local: usize,
    pub(super) next_state: usize,
    pub(super) loop_depth: usize,
    pub(super) statement_scope_depth: usize,
}

pub(super) struct FunctionState {
    pub(super) by_name: BTreeMap<String, FunctionId>,
    pub(super) signatures: Vec<(Vec<String>, usize)>,
    pub(super) current: Option<FunctionId>,
    pub(super) active: Vec<FunctionId>,
    pub(super) call_edges: Vec<Vec<FunctionId>>,
}

pub(super) struct DeclarationState {
    pub(super) descriptor: Option<IndicatorDescriptor>,
    pub(super) inputs: Vec<InputDescriptor>,
    pub(super) input_spans: BTreeMap<String, Span>,
    pub(super) series_inputs: Vec<SeriesInputDescriptor>,
    pub(super) requirements: DataRequirements,
    pub(super) max_bars_back_explicit: bool,
}

#[derive(Debug, Default)]
pub(super) struct PresentationSpans {
    pub(super) indicator: Option<Span>,
    pub(super) display: Option<Span>,
    pub(super) format: Option<Span>,
    pub(super) plots: BTreeMap<String, Span>,
    pub(super) styles: BTreeMap<String, Span>,
    pub(super) marks: Vec<Span>,
    pub(super) segments: BTreeMap<String, Span>,
    pub(super) region_style_slots: BTreeMap<String, Span>,
    pub(super) fills: Vec<Span>,
    pub(super) readouts: Vec<Span>,
    pub(super) columns: Vec<Span>,
}

pub(super) struct PresentationState {
    pub(super) data: IndicatorPresentation,
    pub(super) spans: PresentationSpans,
}

pub(super) struct LoweringContext {
    pub(super) path: PathBuf,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) registry: IntrinsicRegistry,
    pub(super) ir: IrBuilder,
    pub(super) scope: ScopeState,
    pub(super) function: FunctionState,
    pub(super) declarations: DeclarationState,
    pub(super) presentation: PresentationState,
}

impl LoweringContext {
    pub(super) fn new(path: PathBuf, registry: IntrinsicRegistry) -> Self {
        Self {
            path,
            diagnostics: Vec::new(),
            registry,
            ir: IrBuilder {
                program: HirProgram::empty(),
                history_expressions: Vec::new(),
                next_series: 0,
                next_call_site: 0,
                state_call_sites: Vec::new(),
                state_functions: Vec::new(),
            },
            scope: ScopeState {
                values: BTreeMap::new(),
                states: BTreeMap::new(),
                local_definitions: Vec::new(),
                mutable_values: Vec::new(),
                next_local: 0,
                next_state: 0,
                loop_depth: 0,
                statement_scope_depth: 0,
            },
            function: FunctionState {
                by_name: BTreeMap::new(),
                signatures: Vec::new(),
                current: None,
                active: Vec::new(),
                call_edges: Vec::new(),
            },
            declarations: DeclarationState {
                descriptor: None,
                inputs: Vec::new(),
                input_spans: BTreeMap::new(),
                series_inputs: Vec::new(),
                requirements: DataRequirements::default(),
                max_bars_back_explicit: false,
            },
            presentation: PresentationState {
                data: IndicatorPresentation::default(),
                spans: PresentationSpans::default(),
            },
        }
    }

    pub(super) fn merge_binding_type(
        &mut self,
        name: &str,
        left: Type,
        right: Type,
        span: Span,
    ) -> Type {
        match merge_types(left, right) {
            Some(merged) => merged,
            None => {
                self.error(span, format!("binding {name} has incompatible types"));
                left
            }
        }
    }

    pub(super) fn merge_result_type(
        &mut self,
        left: ValueType,
        right: ValueType,
        span: Span,
        context: &str,
    ) -> ValueType {
        match merge_value_types_checked(left, right) {
            Some(value) => value,
            None => {
                self.error(span, format!("{context} have incompatible types"));
                ValueType::Any
            }
        }
    }

    pub(super) fn merge_visible_bindings<Slot: Copy>(
        &mut self,
        base: BTreeMap<String, (Slot, Type)>,
        left: &BTreeMap<String, (Slot, Type)>,
        right: &BTreeMap<String, (Slot, Type)>,
        span: Span,
    ) -> BTreeMap<String, (Slot, Type)> {
        base.into_iter()
            .map(|(name, (slot, base_type))| {
                let left_type = left.get(&name).map_or(base_type, |(_, ty)| *ty);
                let right_type = right.get(&name).map_or(base_type, |(_, ty)| *ty);
                let merged = self.merge_binding_type(&name, base_type, left_type, span);
                let merged = self.merge_binding_type(&name, merged, right_type, span);
                (name, (slot, merged))
            })
            .collect()
    }

    pub(super) fn static_argument(
        &mut self,
        call: &AstCall,
        index: usize,
        description: &str,
    ) -> Option<Value> {
        let argument = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index)?;
        let Some(value) = constant_value(&argument.expression) else {
            self.error(
                argument.expression.span(),
                format!("{description} must be a compile-time constant"),
            );
            return None;
        };
        Some(value)
    }

    pub(super) fn static_input_argument(
        &mut self,
        call: &AstCall,
        index: usize,
        description: &str,
    ) -> Option<Value> {
        let argument = argument_at(call, index, "defval")?;
        let Some(value) = constant_value(&argument.expression) else {
            self.error(
                argument.expression.span(),
                format!("{description} must be a compile-time constant"),
            );
            return None;
        };
        Some(value)
    }

    pub(super) fn static_input_string(
        &mut self,
        call: &AstCall,
        index: usize,
        description: &str,
    ) -> Option<String> {
        let argument = argument_at(call, index, "title")?;
        match constant_value(&argument.expression) {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(
                    argument.expression.span(),
                    format!("{description} must be a string literal"),
                );
                None
            }
            None => {
                self.error(
                    argument.expression.span(),
                    format!("{description} must be a compile-time constant"),
                );
                None
            }
        }
    }

    pub(super) fn static_named_number(&mut self, call: &AstCall, names: &[&str]) -> Option<f64> {
        for name in names {
            if call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some(name))
            {
                return match self.static_named_value(call, name) {
                    Some(Value::Int(value)) => Some(value as f64),
                    Some(Value::Float(value)) => Some(value),
                    Some(_) => {
                        self.error(call.span, format!("{name} must be numeric"));
                        None
                    }
                    None => None,
                };
            }
        }
        None
    }

    pub(super) fn static_named_value(&mut self, call: &AstCall, name: &str) -> Option<Value> {
        let argument = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some(name))?;
        let Some(value) = constant_value(&argument.expression) else {
            self.error(
                argument.expression.span(),
                format!("{name} must be a compile-time constant"),
            );
            return None;
        };
        Some(value)
    }

    pub(super) fn static_named_string(&mut self, call: &AstCall, name: &str) -> Option<String> {
        match self.static_named_value(call, name) {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(call.span, format!("{name} must be a string literal"));
                None
            }
            None => None,
        }
    }

    pub(super) fn validate_named_arguments(&mut self, call: &AstCall, allowed: &[&str]) {
        let mut seen = BTreeSet::new();
        for argument in &call.arguments {
            let Some(name) = argument.name.as_deref() else {
                continue;
            };
            if !allowed.contains(&name) {
                self.error(
                    argument.expression.span(),
                    format!("unknown argument '{name}'"),
                );
            } else if !seen.insert(name.to_owned()) {
                self.error(
                    argument.expression.span(),
                    format!("duplicate argument '{name}'"),
                );
            }
        }
    }

    pub(super) fn validate_positional_arguments(
        &mut self,
        call: &AstCall,
        maximum: usize,
        name: &str,
    ) {
        let positional = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .collect::<Vec<_>>();
        if let Some(argument) = positional.get(maximum) {
            self.error(
                argument.expression.span(),
                format!(
                    "{name} accepts at most {maximum} positional argument{}",
                    if maximum == 1 { "" } else { "s" }
                ),
            );
        }
    }

    pub(super) fn resolve_fixed_expressions(&mut self) {
        let inputs = self
            .ir
            .program
            .inputs
            .iter()
            .map(|(slot, _)| *slot)
            .collect::<BTreeSet<_>>();
        // Operands precede their expressions. Mutable locals and unknown values
        // cannot establish a fixed rolling-window length.
        for index in 0..self.ir.program.expressions.len() {
            let fixed = |expression: ExprId| self.ir.program.expressions[expression.0].is_fixed;
            let is_fixed = match &self.ir.program.expressions[index].kind {
                ExpressionKind::Constant(_) | ExpressionKind::LoadInput(_) => true,
                ExpressionKind::LoadLocal(slot) if !self.scope.mutable_values[slot.0] => {
                    inputs.contains(slot) || self.scope.local_definitions[slot.0].is_some_and(fixed)
                }
                ExpressionKind::Unary { operand, .. } => fixed(*operand),
                ExpressionKind::Binary { left, right, .. } => fixed(*left) && fixed(*right),
                ExpressionKind::Conditional {
                    condition,
                    when_true,
                    when_false,
                } => fixed(*condition) && fixed(*when_true) && fixed(*when_false),
                ExpressionKind::Coalesce { source, fallback } => fixed(*source) && fixed(*fallback),
                ExpressionKind::Switch {
                    operand,
                    branches,
                    fallback,
                } => {
                    fixed(*operand)
                        && branches
                            .iter()
                            .all(|(condition, result)| fixed(*condition) && fixed(*result))
                        && fallback.is_none_or(fixed)
                }
                _ => false,
            };
            self.ir.program.expressions[index].is_fixed = is_fixed;
        }
    }

    pub(super) fn retained_series(&self) -> Vec<bool> {
        let mut retained = vec![false; self.ir.next_series];
        for (history, _) in &self.ir.history_expressions {
            let Some(ExpressionKind::History { source, .. }) = self
                .ir
                .program
                .expressions
                .get(history.0)
                .map(|expression| &expression.kind)
            else {
                continue;
            };
            if let Some(slot) = self
                .ir
                .program
                .expressions
                .get(source.0)
                .and_then(|expression| expression.series_slot)
                && let Some(retained) = retained.get_mut(slot.0)
            {
                *retained = true;
            }
        }
        retained
    }

    pub(super) fn is_top_level_declaration(&self) -> bool {
        self.function.current.is_none() && self.scope.statement_scope_depth == 0
    }

    pub(super) fn marker_count(&self) -> usize {
        self.ir
            .program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::Marker { .. }))
            .count()
    }

    pub(super) fn segment_count(&self) -> usize {
        self.ir
            .program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::Segment { .. }))
            .count()
    }

    pub(super) fn region_count(&self) -> usize {
        self.ir
            .program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::Region { .. }))
            .count()
    }

    pub(super) fn level_count(&self) -> usize {
        self.ir
            .program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::HorizontalLevel { .. }))
            .count()
    }

    pub(super) fn push_constant(&mut self, value: Value, ty: Type) -> ExprId {
        let constant = ConstantId(self.ir.program.constants.len());
        self.ir.program.constants.push(value);
        self.push_expression(ExpressionKind::Constant(constant), ty)
    }

    pub(super) fn push_statement(&mut self, statement: TypedStatement) -> crate::hir::StatementId {
        let id = crate::hir::StatementId(self.ir.program.statements.len());
        self.ir.program.statements.push(statement);
        id
    }

    pub(super) fn push_expression(&mut self, kind: ExpressionKind, ty: Type) -> ExprId {
        let series_slot = (ty.qualifier == Qualifier::Series).then(|| {
            let slot = SeriesSlot(self.ir.next_series);
            self.ir.next_series += 1;
            slot
        });
        self.push_expression_with_series_slot(kind, ty, series_slot)
    }

    pub(super) fn push_expression_with_series_slot(
        &mut self,
        kind: ExpressionKind,
        ty: Type,
        series_slot: Option<SeriesSlot>,
    ) -> ExprId {
        let id = ExprId(self.ir.program.expressions.len());
        self.ir.program.expressions.push(TypedExpression {
            kind,
            ty,
            series_slot,
            is_fixed: false,
        });
        id
    }

    pub(super) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(self.path.clone(), span, message));
    }
}

pub(super) fn argument_at<'a>(
    call: &'a AstCall,
    positional_index: usize,
    name: &str,
) -> Option<&'a AstArgument> {
    call.arguments
        .iter()
        .find(|argument| argument.name.as_deref() == Some(name))
        .or_else(|| {
            call.arguments
                .iter()
                .filter(|argument| argument.name.is_none())
                .nth(positional_index)
        })
}

pub(super) fn merge_value_types(left: ValueType, right: ValueType) -> ValueType {
    merge_value_types_checked(left, right).unwrap_or(ValueType::Any)
}

pub(super) fn merge_value_types_checked(left: ValueType, right: ValueType) -> Option<ValueType> {
    if left == right {
        return Some(left);
    }
    if matches!(left, ValueType::Any) {
        return Some(right);
    }
    if matches!(right, ValueType::Any) {
        return Some(left);
    }
    if matches!(
        (left, right),
        (ValueType::Int, ValueType::Float) | (ValueType::Float, ValueType::Int)
    ) {
        Some(ValueType::Float)
    } else {
        None
    }
}

pub(super) fn merge_types(left: Type, right: Type) -> Option<Type> {
    Some(Type {
        value: merge_value_types_checked(left.value, right.value)?,
        qualifier: if left.qualifier == Qualifier::Series || right.qualifier == Qualifier::Series {
            Qualifier::Series
        } else {
            Qualifier::Simple
        },
    })
}

pub(super) fn is_boolean_type(value: ValueType) -> bool {
    matches!(value, ValueType::Bool | ValueType::Any)
}

pub(super) fn is_numeric_type(value: ValueType) -> bool {
    matches!(value, ValueType::Int | ValueType::Float | ValueType::Any)
}

pub(super) fn is_orderable_type(value: ValueType) -> bool {
    matches!(
        value,
        ValueType::Int | ValueType::Float | ValueType::Time | ValueType::Any
    )
}
