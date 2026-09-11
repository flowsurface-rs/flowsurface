use super::super::ast::{AstExpression, AstFunction, AstStatement};
use super::context::LoweringContext;
use crate::hir::*;
use crate::types::*;

impl LoweringContext {
    pub(super) fn compile_function(&mut self, function: AstFunction, function_id: FunctionId) {
        let saved_values = self.scope.values.clone();
        let saved_states = self.scope.states.clone();
        let saved_function = self.function.current;
        let saved_loop_depth = self.scope.loop_depth;
        self.function.current = Some(function_id);
        self.function.active.push(function_id);
        self.scope.loop_depth = 0;
        let local_start = self.scope.next_local;
        let mut parameter_slots = Vec::new();
        let mut parameter_names = Vec::new();
        let mut parameter_defaults = Vec::new();
        let parameter_specs = function.parameters;
        for parameter in &parameter_specs {
            let slot = LocalSlot(self.scope.next_local);
            self.scope.next_local += 1;
            self.scope.local_definitions.push(None);
            parameter_slots.push(slot);
            self.scope.values.insert(
                parameter.name.clone(),
                (slot, Type::series(ValueType::Float)),
            );
            parameter_names.push(parameter.name.clone());
        }
        for parameter in &parameter_specs {
            let default = parameter
                .default
                .as_ref()
                .map(|default| self.compile_expression(default));
            if let Some(default) = default
                && !self.is_numeric_function_expression(default)
            {
                self.error(
                    parameter
                        .default
                        .as_ref()
                        .map_or(function.span, |default| default.span()),
                    format!(
                        "default for function parameter '{}' must be numeric or na",
                        parameter.name
                    ),
                );
            }
            parameter_defaults.push(default);
        }
        let mut body = Vec::new();
        for statement in function.body {
            if let Some(statement) = self.compile_statement(statement) {
                body.push(statement);
            }
        }
        let local_slots = (local_start..self.scope.next_local)
            .map(LocalSlot)
            .collect();
        if let Some(compiled) = self.ir.program.functions.get_mut(function_id.0) {
            compiled.parameters = parameter_slots;
            compiled.parameter_names = parameter_names;
            compiled.parameter_defaults = parameter_defaults;
            compiled.local_slots = local_slots;
            compiled.body = body;
            compiled.return_type = Type::series(ValueType::Float);
        }
        self.scope.values = saved_values;
        self.scope.states = saved_states;
        self.function.current = saved_function;
        self.function.active.pop();
        self.scope.loop_depth = saved_loop_depth;
    }
    pub(super) fn compile_statement(
        &mut self,
        statement: AstStatement,
    ) -> Option<crate::hir::StatementId> {
        match statement {
            AstStatement::Binding {
                name,
                expression,
                state,
                intrabar,
                update,
                span,
            } => {
                if intrabar {
                    self.error(
                        span,
                        "intrabar state (varip) is not available in the live chart runtime",
                    );
                }
                if state && update {
                    self.error(span, "a state declaration cannot use an update assignment");
                    return None;
                }
                if !state && update {
                    let state_slot = self.scope.states.get(&name).map(|(slot, _)| *slot);
                    let local_slot = self.scope.values.get(&name).map(|(slot, _)| *slot);
                    let current_type = if state_slot.is_some() {
                        self.scope.states.get(&name).map(|(_, ty)| *ty)
                    } else {
                        self.scope.values.get(&name).map(|(_, ty)| *ty)
                    };
                    if let Some(slot) = local_slot {
                        self.scope.mutable_values.insert(slot.0);
                    }
                    let expression = self.compile_expression(&expression);
                    let expression_type = self.ir.program.expressions[expression.0].ty;
                    let merged_type = current_type.map(|current_type| {
                        self.merge_binding_type(&name, current_type, expression_type, span)
                    });
                    if let Some(slot) = state_slot {
                        if let Some(entry) = self.scope.states.get_mut(&name)
                            && let Some(merged_type) = merged_type
                        {
                            entry.1 = merged_type;
                        }
                        Some(self.push_statement(TypedStatement {
                            kind: StatementKind::StateUpdate {
                                target: slot,
                                expression,
                            },
                        }))
                    } else if let Some(slot) = local_slot {
                        if let Some(entry) = self.scope.values.get_mut(&name)
                            && let Some(merged_type) = merged_type
                        {
                            entry.1 = merged_type;
                        }
                        Some(self.push_statement(TypedStatement {
                            kind: StatementKind::LocalUpdate {
                                target: slot,
                                expression,
                            },
                        }))
                    } else {
                        self.error(span, format!("unknown variable {name}"));
                        None
                    }
                } else if state {
                    if self.scope.states.contains_key(&name) {
                        self.error(span, format!("state {name} is already declared"));
                        return None;
                    }
                    let expression = self.compile_expression(&expression);
                    let ty = self.ir.program.expressions[expression.0].ty;
                    let slot = StateSlot(self.scope.next_state);
                    self.scope.next_state += 1;
                    let call_site = CallSiteId(self.ir.next_call_site);
                    self.ir.next_call_site += 1;
                    self.ir.state_call_sites.push(call_site);
                    self.ir.state_functions.push(self.function.current);
                    self.scope.states.insert(name, (slot, ty));
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::StateDeclaration {
                            target: slot,
                            initial: expression,
                            call_site,
                        },
                    }))
                } else {
                    if self.scope.values.contains_key(&name)
                        || self.scope.states.contains_key(&name)
                    {
                        self.error(span, format!("{name} is already declared"));
                        return None;
                    }
                    if let AstExpression::Call(call) = &expression
                        && call.path.first().is_some_and(|segment| segment == "input")
                    {
                        return self.compile_input(&name, call, span);
                    }
                    let expression_id = self.compile_expression(&expression);
                    let ty = self.ir.program.expressions[expression_id.0].ty;
                    let slot = LocalSlot(self.scope.next_local);
                    self.scope.next_local += 1;
                    self.scope.local_definitions.push(Some(expression_id));
                    self.scope.values.insert(name, (slot, ty));
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Binding {
                            target: slot,
                            expression: expression_id,
                        },
                    }))
                }
            }
            AstStatement::Destructure {
                names,
                expression,
                update,
                span,
            } => {
                if update {
                    let targets = names
                        .iter()
                        .map(|name| self.scope.values.get(name).map(|(slot, _)| *slot))
                        .collect::<Option<Vec<_>>>();
                    let Some(targets) = targets else {
                        self.error(span, "destructuring update references an unknown local");
                        return None;
                    };
                    for target in &targets {
                        self.scope.mutable_values.insert(target.0);
                    }
                    let expression = self.compile_expression(&expression);
                    if !matches!(
                        self.ir.program.expressions[expression.0].ty.value,
                        ValueType::Tuple | ValueType::Array | ValueType::Any
                    ) {
                        self.error(span, "destructuring requires a tuple value");
                    }
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Destructure {
                            targets,
                            expression,
                        },
                    }))
                } else {
                    if names.iter().any(|name| {
                        self.scope.values.contains_key(name) || self.scope.states.contains_key(name)
                    }) {
                        self.error(span, "destructuring binding shadows an existing value");
                        return None;
                    }
                    let expression = self.compile_expression(&expression);
                    let expression_type = self.ir.program.expressions[expression.0].ty;
                    if !matches!(
                        expression_type.value,
                        ValueType::Tuple | ValueType::Array | ValueType::Any
                    ) {
                        self.error(span, "destructuring requires a tuple value");
                    }
                    let targets = names
                        .into_iter()
                        .map(|name| {
                            let slot = LocalSlot(self.scope.next_local);
                            self.scope.next_local += 1;
                            self.scope.local_definitions.push(None);
                            self.scope
                                .values
                                .insert(name, (slot, Type::series(ValueType::Any)));
                            slot
                        })
                        .collect();
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Destructure {
                            targets,
                            expression,
                        },
                    }))
                }
            }
            AstStatement::Call { call } => match call.path.join(".").as_str() {
                "plot" => self.compile_plot(&call),
                "marker" => self.compile_marker(&call),
                "segment" => self.compile_segment(&call),
                "region" => self.compile_region(&call),
                "region_remove" => self.compile_region_remove(&call),
                "level" => self.compile_level(&call),
                "plotcandle" | "plotohlc" => self.compile_ohlc(&call),
                "broken_cloud" | "plotrange" => self.compile_range(&call),
                "emit" => self.compile_emit(&call),
                "display" => self.compile_display_declaration(&call),
                "format" => self.compile_format_declaration(&call),
                "style" => self.compile_style_declaration(&call),
                "mark" => self.compile_mark_declaration(&call),
                "fill" => self.compile_fill_declaration(&call),
                "readout" => self.compile_readout_declaration(&call),
                "column" => self.compile_column_declaration(&call),
                "language" | "indicator" => None,
                _ => {
                    let expression = self.compile_call(&call);
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Expression(expression),
                    }))
                }
            },
            AstStatement::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                let condition = self.compile_expression(&condition);
                if self.ir.program.expressions[condition.0].ty.value != ValueType::Bool {
                    self.error(span, "if condition must be boolean");
                }
                let saved_values = self.scope.values.clone();
                let saved_states = self.scope.states.clone();
                let then_branch = self.compile_scoped_statements(then_branch);
                let then_values = self.scope.values.clone();
                let then_states = self.scope.states.clone();
                self.scope.values = saved_values.clone();
                self.scope.states = saved_states.clone();
                let else_branch = self.compile_scoped_statements(else_branch);
                let else_values = self.scope.values.clone();
                let else_states = self.scope.states.clone();
                self.scope.values =
                    self.merge_visible_bindings(saved_values, &then_values, &else_values, span);
                self.scope.states =
                    self.merge_visible_bindings(saved_states, &then_states, &else_states, span);
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::If {
                        condition,
                        then_branch,
                        else_branch,
                    },
                }))
            }
            AstStatement::While {
                condition,
                body,
                span,
            } => {
                let condition = self.compile_expression(&condition);
                if self.ir.program.expressions[condition.0].ty.value != ValueType::Bool {
                    self.error(span, "while condition must be boolean");
                }
                self.scope.loop_depth += 1;
                let saved_values = self.scope.values.clone();
                let saved_states = self.scope.states.clone();
                let body = self.compile_scoped_statements(body);
                let body_values = self.scope.values.clone();
                let body_states = self.scope.states.clone();
                self.scope.values = self.merge_visible_bindings(
                    saved_values.clone(),
                    &body_values,
                    &saved_values,
                    span,
                );
                self.scope.states = self.merge_visible_bindings(
                    saved_states.clone(),
                    &body_states,
                    &saved_states,
                    span,
                );
                self.scope.loop_depth = self.scope.loop_depth.saturating_sub(1);
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::While { condition, body },
                }))
            }
            AstStatement::ForRange {
                name,
                start,
                end,
                body,
                span,
            } => {
                if self.scope.values.contains_key(&name) || self.scope.states.contains_key(&name) {
                    self.error(span, format!("{name} is already declared"));
                    return None;
                }
                let saved_values = self.scope.values.clone();
                let saved_states = self.scope.states.clone();
                let start = self.compile_expression(&start);
                let end = self.compile_expression(&end);
                if self.ir.program.expressions[start.0].ty.value != ValueType::Int
                    || self.ir.program.expressions[end.0].ty.value != ValueType::Int
                {
                    self.error(span, "range loop bounds must be integers");
                }
                let target = LocalSlot(self.scope.next_local);
                self.scope.next_local += 1;
                self.scope.local_definitions.push(None);
                self.scope
                    .values
                    .insert(name, (target, Type::series(ValueType::Int)));
                self.scope.mutable_values.insert(target.0);
                self.scope.loop_depth += 1;
                let body = self.compile_scoped_statements(body);
                let body_values = self.scope.values.clone();
                let body_states = self.scope.states.clone();
                self.scope.values = self.merge_visible_bindings(
                    saved_values.clone(),
                    &body_values,
                    &saved_values,
                    span,
                );
                self.scope.states = self.merge_visible_bindings(
                    saved_states.clone(),
                    &body_states,
                    &saved_states,
                    span,
                );
                self.scope.loop_depth = self.scope.loop_depth.saturating_sub(1);
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::ForRange {
                        target,
                        start,
                        end,
                        body,
                    },
                }))
            }
            AstStatement::ForEach {
                name,
                collection,
                body,
                span,
            } => {
                if self.scope.values.contains_key(&name) || self.scope.states.contains_key(&name) {
                    self.error(span, format!("{name} is already declared"));
                    return None;
                }
                let saved_values = self.scope.values.clone();
                let saved_states = self.scope.states.clone();
                let collection = self.compile_expression(&collection);
                let collection_type = self.ir.program.expressions[collection.0].ty.value;
                if !matches!(
                    collection_type,
                    ValueType::Array | ValueType::Tuple | ValueType::Any
                ) {
                    self.error(span, "collection loops require an array or tuple");
                }
                let target = LocalSlot(self.scope.next_local);
                self.scope.next_local += 1;
                self.scope.local_definitions.push(None);
                self.scope.values.insert(
                    name,
                    (
                        target,
                        Type::series(self.collection_element_type(collection)),
                    ),
                );
                self.scope.mutable_values.insert(target.0);
                self.scope.loop_depth += 1;
                let body = self.compile_scoped_statements(body);
                let body_values = self.scope.values.clone();
                let body_states = self.scope.states.clone();
                self.scope.values = self.merge_visible_bindings(
                    saved_values.clone(),
                    &body_values,
                    &saved_values,
                    span,
                );
                self.scope.states = self.merge_visible_bindings(
                    saved_states.clone(),
                    &body_states,
                    &saved_states,
                    span,
                );
                self.scope.loop_depth = self.scope.loop_depth.saturating_sub(1);
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::ForEach {
                        target,
                        collection,
                        body,
                    },
                }))
            }
            AstStatement::Break(span) => {
                if self.scope.loop_depth == 0 {
                    self.error(span, "break is only valid inside a loop");
                    None
                } else {
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Break,
                    }))
                }
            }
            AstStatement::Continue(span) => {
                if self.scope.loop_depth == 0 {
                    self.error(span, "continue is only valid inside a loop");
                    None
                } else {
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Continue,
                    }))
                }
            }
            AstStatement::Return { expression, span } => {
                if self.function.current.is_none() {
                    self.error(span, "return is only valid inside a function");
                    return None;
                }
                let expression = expression.map(|expression| self.compile_expression(&expression));
                if let Some(expression) = expression
                    && !self.is_numeric_function_expression(expression)
                {
                    self.error(span, "function return expressions must be numeric or na");
                }
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::Return(expression),
                }))
            }
        }
    }

    pub(super) fn compile_scoped_statements(
        &mut self,
        statements: Vec<AstStatement>,
    ) -> Vec<crate::hir::StatementId> {
        self.scope.statement_scope_depth = self.scope.statement_scope_depth.saturating_add(1);
        let compiled = statements
            .into_iter()
            .filter_map(|statement| self.compile_statement(statement))
            .collect();
        self.scope.statement_scope_depth = self.scope.statement_scope_depth.saturating_sub(1);
        compiled
    }
}
