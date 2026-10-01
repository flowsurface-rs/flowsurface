use super::super::ast::{AstCall, AstExpression};
use super::context::{LoweringContext, argument_at};
use crate::diagnostics::{Diagnostic, Span};
use crate::frame::{FrameField, TimeOrTick};
use crate::hir::{ExpressionKind, LocalSlot, SourceInputSlot, StatementKind, TypedStatement};
use crate::scene::*;
use crate::types::*;
use std::collections::BTreeSet;

fn is_valid_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

impl LoweringContext {
    pub(super) fn compile_descriptor(&mut self, call: Option<&AstCall>) -> IndicatorDescriptor {
        self.presentation.spans.indicator = call.map(|call| call.span);
        let mut title = self
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("indicator")
            .to_owned();
        let mut id = format!("local.{title}");
        let mut author = None;
        let mut description = None;
        let mut source = None;
        let mut sourceable = false;
        let mut optional_trades = false;
        let mut bases = None;
        let mut max_bars_back = 5_000;
        if let Some(call) = call {
            self.validate_named_arguments(
                call,
                &[
                    "title",
                    "id",
                    "author",
                    "author_url",
                    "description",
                    "source",
                    "sourceable",
                    "optional_trades",
                    "bases",
                    "max_bars_back",
                ],
            );
            if let Some(value) = self.static_argument(call, 0, "indicator title") {
                if let Value::String(value) = value {
                    title = value;
                } else {
                    self.error(call.span, "indicator title must be a string literal");
                }
            }
            if let Some(value) = self.static_named_value(call, "title") {
                if let Value::String(value) = value {
                    title = value;
                } else {
                    self.error(call.span, "indicator title must be a string literal");
                }
            }
            if let Some(value) = self.static_named_value(call, "id") {
                if let Value::String(value) = value {
                    id = value;
                } else {
                    self.error(call.span, "indicator ID must be a string literal");
                }
            }
            let has_author = call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("author"));
            let has_author_url = call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("author_url"));
            if has_author || has_author_url {
                if !has_author {
                    self.error(call.span, "indicator author_url requires author");
                }
                if !has_author_url {
                    self.error(call.span, "indicator author requires author_url");
                }
                if let (Some(name), Some(url)) = (
                    self.static_named_string(call, "author"),
                    self.static_named_string(call, "author_url"),
                ) {
                    if name.trim().is_empty() {
                        self.error(call.span, "indicator author must not be empty");
                    }
                    if !is_valid_http_url(&url) {
                        self.error(call.span, "indicator author_url must be a valid URL");
                    }
                    author = Some(IndicatorAuthor { name, url });
                }
            }
            if let Some(value) = self.static_named_value(call, "description") {
                if let Value::String(value) = value {
                    description = Some(value);
                } else {
                    self.error(call.span, "indicator description must be a string literal");
                }
            }
            if let Some(value) = self.static_named_value(call, "source") {
                if let Value::String(value) = value {
                    if !is_valid_http_url(&value) {
                        self.error(call.span, "indicator source must be a valid URL");
                    }
                    source = Some(value);
                } else {
                    self.error(call.span, "indicator source must be a string literal");
                }
            }
            if let Some(value) = self.static_named_value(call, "sourceable") {
                if let Value::Bool(value) = value {
                    sourceable = value;
                } else {
                    self.error(call.span, "indicator sourceable must be a boolean literal");
                }
            }
            if let Some(value) = self.static_named_value(call, "optional_trades") {
                if let Value::Bool(value) = value {
                    optional_trades = value;
                } else {
                    self.error(
                        call.span,
                        "indicator optional_trades must be a boolean literal",
                    );
                }
            }
            bases = self.indicator_bases(call);
            if let Some(value) = self.static_named_value(call, "max_bars_back") {
                if let Value::Int(value) = value
                    && value > 0
                {
                    max_bars_back = value as usize;
                    self.declarations.max_bars_back_explicit = true;
                } else {
                    self.error(call.span, "max_bars_back must be a positive integer");
                }
            }
        }
        if max_bars_back > 50_000 {
            self.diagnostics.push(Diagnostic::error(
                self.path.clone(),
                call.map_or(Span::new(0, 0), |call| call.span),
                "max_bars_back cannot exceed the host maximum of 50000",
            ));
            max_bars_back = 50_000;
        }
        let id = match IndicatorId::new(id) {
            Ok(id) => id,
            Err(message) => {
                self.diagnostics.push(Diagnostic::error(
                    self.path.clone(),
                    call.map_or(Span::new(0, 0), |call| call.span),
                    message,
                ));
                IndicatorId::new("local.invalid").expect("static ID is valid")
            }
        };
        IndicatorDescriptor {
            id,
            title,
            author,
            description,
            source,
            sourceable,
            optional_trades,
            bases,
            max_bars_back,
            inputs: Vec::new(),
            series_inputs: Vec::new(),
            plots: Vec::new(),
            presentation: IndicatorPresentation::default(),
        }
    }

    fn indicator_bases(&mut self, call: &AstCall) -> Option<Vec<TimeOrTick>> {
        let argument = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("bases"))?;
        let AstExpression::Array(elements, _) = &argument.expression else {
            self.error(
                call.span,
                "indicator bases must be a non-empty array of string literals",
            );
            return None;
        };
        if elements.is_empty() {
            self.error(call.span, "indicator bases must not be empty");
            return None;
        }
        let mut bases = Vec::new();
        for element in elements {
            let basis = match element {
                AstExpression::String(value, _) if value == "time" => TimeOrTick::Time,
                AstExpression::String(value, _) if value == "tick" => TimeOrTick::Tick,
                _ => {
                    self.error(
                        element.span(),
                        "indicator bases entries must be \"time\" or \"tick\"",
                    );
                    return None;
                }
            };
            if !bases.contains(&basis) {
                bases.push(basis);
            }
        }
        Some(bases)
    }

    pub(super) fn compile_input(
        &mut self,
        name: &str,
        call: &AstCall,
        span: Span,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(span, "input declarations must be top-level");
            return None;
        }
        let Some(kind) = call.path.get(1).map(String::as_str) else {
            self.error(span, "input requires a type, for example input.int");
            return None;
        };
        if kind == "source" {
            return self.compile_source_input(name, call, span);
        }
        self.validate_input_arguments(call);
        let default_value = self.static_input_argument(call, 0, "input default");
        let (value_type, default) = match kind {
            "int" => (ValueType::Int, default_value),
            "float" => (ValueType::Float, default_value),
            "bool" => (ValueType::Bool, default_value),
            "string" => (ValueType::String, default_value),
            "enum" => {
                self.error(call.span, "enum inputs are not available; use input.string");
                return None;
            }
            "color" => (ValueType::Color, default_value),
            "time" => (ValueType::Time, default_value),
            _ => {
                self.error(call.span, format!("unknown input type {kind}"));
                return None;
            }
        };
        let default = match (value_type, default) {
            (ValueType::Int, Some(Value::Int(value))) => InputValue::Int(value),
            (ValueType::Float, Some(Value::Float(value))) => InputValue::Float(value),
            (ValueType::Bool, Some(Value::Bool(value))) => InputValue::Bool(value),
            (ValueType::String, Some(Value::String(value))) => InputValue::String(value),
            (ValueType::Color, Some(Value::Color(value))) => InputValue::Color(value),
            (ValueType::Time, Some(Value::Int(value))) => InputValue::Time(value),
            (ValueType::Time, Some(Value::Float(value))) => InputValue::Time(value as i64),
            (ValueType::Int, Some(Value::Float(value))) => InputValue::Int(value as i64),
            (ValueType::Float, Some(Value::Int(value))) => InputValue::Float(value as f64),
            (ValueType::Bool, None) => InputValue::Bool(false),
            (ValueType::String, None) => InputValue::String(String::new()),
            (ValueType::Color, None) => InputValue::Color(0xffff_ffff),
            (ValueType::Time, None) => InputValue::Time(0),
            (ValueType::Int, None) => InputValue::Int(0),
            (ValueType::Float, None) => InputValue::Float(0.0),
            (ValueType::Tuple | ValueType::Array | ValueType::Record, None) => {
                self.error(call.span, "input type requires a literal default");
                return None;
            }
            (ValueType::Any | ValueType::Void, None) => {
                self.error(call.span, "input type cannot be any or void");
                return None;
            }
            (_, Some(_)) => {
                self.error(call.span, "input default has the wrong type");
                return None;
            }
        };
        let label = self
            .static_input_string(call, 1, "input title")
            .unwrap_or_else(|| name.to_owned());
        let min = self.static_named_number(call, &["min"]);
        let max = self.static_named_number(call, &["max"]);
        let step = self.static_named_number(call, &["step"]);
        let options = self.input_options(call)?;
        let group = self.static_named_string(call, "group");
        let inline = self.static_named_string(call, "inline");
        let tooltip = self.static_named_string(call, "tooltip");
        let visible_when = self.static_named_string(call, "visible_when");
        let required_input = self.parse_presentation_input(call, "requires");
        self.declarations.inputs.push(InputDescriptor {
            id: name.to_owned(),
            label,
            value_type,
            default,
            min,
            max,
            step,
            group,
            inline,
            tooltip,
            visible_when,
            required_input,
            options,
        });
        self.declarations.input_spans.insert(name.to_owned(), span);
        let slot = LocalSlot(self.scope.next_local);
        self.scope.next_local += 1;
        self.scope.local_definitions.push(None);
        self.scope.mutable_values.push(false);
        self.scope
            .values
            .insert(name.to_owned(), (slot, Type::simple(value_type)));
        self.ir.program.inputs.push((slot, name.to_owned()));
        let expression =
            self.push_expression(ExpressionKind::LoadInput(slot), Type::simple(value_type));
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Binding {
                target: slot,
                expression,
            },
        }))
    }

    pub(super) fn compile_source_input(
        &mut self,
        name: &str,
        call: &AstCall,
        _span: Span,
    ) -> Option<crate::hir::StatementId> {
        self.validate_positional_arguments(call, 2, "input.source");
        self.validate_named_arguments(call, &["defval", "title", "allow_indicator"]);
        let positional_count = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .count();
        if positional_count > 0
            && call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("defval"))
        {
            self.error(call.span, "input.source default cannot be supplied twice");
        }
        if positional_count > 1
            && call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("title"))
        {
            self.error(call.span, "input.source title cannot be supplied twice");
        }
        let Some(default_argument) = argument_at(call, 0, "defval") else {
            self.error(call.span, "input.source requires a numeric chart source");
            return None;
        };
        let default_field = match &default_argument.expression {
            AstExpression::Identifier(path, _) => match path.join(".").as_str() {
                "chart.open" => Some(FrameField::ChartOpen),
                "chart.high" => Some(FrameField::ChartHigh),
                "chart.low" => Some(FrameField::ChartLow),
                "chart.close" => Some(FrameField::ChartClose),
                "chart.hl2" => Some(FrameField::ChartHl2),
                "chart.hlc3" => Some(FrameField::ChartHlc3),
                "chart.ohlc4" => Some(FrameField::ChartOhlc4),
                "chart.hlcc4" => Some(FrameField::ChartHlcc4),
                "chart.volume" => Some(FrameField::ChartVolume),
                "open_interest" => Some(FrameField::OpenInterest),
                _ => None,
            },
            _ => None,
        };
        let Some(default_field) = default_field else {
            self.error(
                default_argument.expression.span(),
                "input.source default must be chart.open, chart.high, chart.low, chart.close, chart.hl2, chart.hlc3, chart.ohlc4, chart.hlcc4, chart.volume, or open_interest",
            );
            return None;
        };
        match default_field {
            FrameField::ChartOpen
            | FrameField::ChartHigh
            | FrameField::ChartLow
            | FrameField::ChartClose
            | FrameField::ChartHl2
            | FrameField::ChartHlc3
            | FrameField::ChartOhlc4
            | FrameField::ChartHlcc4
            | FrameField::ChartVolume => self.declarations.requirements.ohlc = true,
            FrameField::OpenInterest => self.declarations.requirements.open_interest = true,
            _ => {}
        }
        let label = self
            .static_input_string(call, 1, "input.source title")
            .unwrap_or_else(|| name.to_owned());
        let allow_indicator = match self.static_named_value(call, "allow_indicator") {
            Some(Value::Bool(value)) => value,
            Some(_) => {
                self.error(
                    call.span,
                    "input.source allow_indicator must be a boolean literal",
                );
                true
            }
            None => true,
        };
        self.declarations.series_inputs.push(SeriesInputDescriptor {
            id: name.to_owned(),
            label,
            default_field,
            allow_indicator,
        });
        let slot = SourceInputSlot(self.ir.program.source_inputs.len());
        self.ir.program.source_inputs.push((slot, name.to_owned()));
        let local = LocalSlot(self.scope.next_local);
        self.scope.next_local += 1;
        let expression = self.push_expression(
            ExpressionKind::LoadSeriesInput(slot),
            Type::series(ValueType::Float),
        );
        self.scope.local_definitions.push(Some(expression));
        self.scope.mutable_values.push(false);
        self.scope
            .values
            .insert(name.to_owned(), (local, Type::series(ValueType::Float)));
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Binding {
                target: local,
                expression,
            },
        }))
    }

    pub(super) fn input_options(&mut self, call: &AstCall) -> Option<Vec<InputOption>> {
        let Some(argument) = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("options"))
        else {
            return Some(Vec::new());
        };
        let AstExpression::Array(elements, _) = &argument.expression else {
            self.error(call.span, "input options must be an array of strings");
            return None;
        };
        let mut options = Vec::with_capacity(elements.len());
        let mut option_ids = BTreeSet::new();
        for element in elements {
            let AstExpression::String(value, _) = element else {
                self.error(call.span, "input options must be string literals");
                return None;
            };
            if !option_ids.insert(value.clone()) {
                self.error(call.span, format!("input option '{value}' is duplicated"));
            }
            options.push(InputOption {
                id: value.clone(),
                label: value.clone(),
            });
        }
        Some(options)
    }

    pub(super) fn validate_input_arguments(&mut self, call: &AstCall) {
        self.validate_positional_arguments(call, 2, "input");
        self.validate_named_arguments(
            call,
            &[
                "defval",
                "title",
                "min",
                "max",
                "step",
                "options",
                "group",
                "inline",
                "tooltip",
                "visible_when",
                "requires",
            ],
        );
        let positional_count = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .count();
        if positional_count > 0
            && call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("defval"))
        {
            self.error(call.span, "input default cannot be supplied twice");
        }
        if positional_count > 1
            && call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("title"))
        {
            self.error(call.span, "input title cannot be supplied twice");
        }
    }
}
