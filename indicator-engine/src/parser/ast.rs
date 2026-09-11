use crate::CompileLimits;
use crate::diagnostics::{Diagnostic, Span};
use crate::hir::{BinaryOperator, UnaryOperator};
use crate::types::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum AstExpression {
    Number(f64, Span),
    String(String, Span),
    Boolean(bool, Span),
    Na(Span),
    Identifier(Vec<String>, Span),
    Unary {
        operator: UnaryOperator,
        expression: Box<AstExpression>,
        span: Span,
    },
    Binary {
        left: Box<AstExpression>,
        operator: BinaryOperator,
        right: Box<AstExpression>,
        span: Span,
    },
    Call(AstCall),
    Index {
        expression: Box<AstExpression>,
        index: Box<AstExpression>,
        span: Span,
    },
    Field {
        expression: Box<AstExpression>,
        field: String,
        span: Span,
    },
    Tuple(Vec<AstExpression>, Span),
    Array(Vec<AstExpression>, Span),
    Record(Vec<(String, AstExpression)>, Span),
    Switch {
        operand: Box<AstExpression>,
        branches: Vec<(AstExpression, AstExpression)>,
        fallback: Option<Box<AstExpression>>,
        span: Span,
    },
}

impl AstExpression {
    pub(super) fn span(&self) -> Span {
        match self {
            Self::Number(_, span)
            | Self::String(_, span)
            | Self::Boolean(_, span)
            | Self::Na(span)
            | Self::Identifier(_, span) => *span,
            Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Index { span, .. }
            | Self::Field { span, .. } => *span,
            Self::Tuple(_, span) | Self::Array(_, span) | Self::Record(_, span) => *span,
            Self::Switch { span, .. } => *span,
            Self::Call(call) => call.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstCall {
    pub(super) path: Vec<String>,
    pub(super) arguments: Vec<AstArgument>,
    pub(super) span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstArgument {
    pub(super) name: Option<String>,
    pub(super) expression: AstExpression,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum AstStatement {
    Binding {
        name: String,
        expression: AstExpression,
        state: bool,
        intrabar: bool,
        update: bool,
        span: Span,
    },
    Call {
        call: AstCall,
    },
    If {
        condition: AstExpression,
        then_branch: Vec<AstStatement>,
        else_branch: Vec<AstStatement>,
        span: Span,
    },
    While {
        condition: AstExpression,
        body: Vec<AstStatement>,
        span: Span,
    },
    ForRange {
        name: String,
        start: AstExpression,
        end: AstExpression,
        body: Vec<AstStatement>,
        span: Span,
    },
    ForEach {
        name: String,
        collection: AstExpression,
        body: Vec<AstStatement>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    Return {
        expression: Option<AstExpression>,
        span: Span,
    },
    Destructure {
        names: Vec<String>,
        expression: AstExpression,
        update: bool,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstEnum {
    pub(super) name: String,
    pub(super) variants: Vec<String>,
    pub(super) span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstProgram {
    pub(super) indicator: Option<AstCall>,
    pub(super) functions: Vec<AstFunction>,
    pub(super) enums: Vec<AstEnum>,
    pub(super) statements: Vec<AstStatement>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstFunction {
    pub(super) name: String,
    pub(super) parameters: Vec<AstParameter>,
    pub(super) body: Vec<AstStatement>,
    pub(super) span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AstParameter {
    pub(super) name: String,
    pub(super) default: Option<AstExpression>,
}

fn is_output_declaration(path: &[String]) -> bool {
    matches!(
        path.join(".").as_str(),
        "plot"
            | "emit"
            | "marker"
            | "segment"
            | "level"
            | "horizontal_level"
            | "plotcandle"
            | "plotohlc"
            | "plotrange"
            | "broken_cloud"
    )
}

#[derive(Default)]
struct AstCounts {
    expressions: usize,
    statements: usize,
    outputs: usize,
}

fn count_expression_tree(expression: &AstExpression, counts: &mut AstCounts) {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        counts.expressions = counts.expressions.saturating_add(1);
        match expression {
            AstExpression::Unary { expression, .. } | AstExpression::Field { expression, .. } => {
                pending.push(expression)
            }
            AstExpression::Binary { left, right, .. } => {
                pending.push(left);
                pending.push(right);
            }
            AstExpression::Call(call) => {
                for argument in &call.arguments {
                    pending.push(&argument.expression);
                }
            }
            AstExpression::Index {
                expression, index, ..
            } => {
                pending.push(expression);
                pending.push(index);
            }
            AstExpression::Tuple(values, _) | AstExpression::Array(values, _) => {
                pending.extend(values.iter());
            }
            AstExpression::Record(values, _) => {
                pending.extend(values.iter().map(|(_, value)| value));
            }
            AstExpression::Switch {
                operand,
                branches,
                fallback,
                ..
            } => {
                pending.push(operand);
                pending.extend(
                    branches
                        .iter()
                        .flat_map(|(condition, result)| [condition, result]),
                );
                if let Some(fallback) = fallback.as_deref() {
                    pending.push(fallback);
                }
            }
            AstExpression::Number(_, _)
            | AstExpression::String(_, _)
            | AstExpression::Boolean(_, _)
            | AstExpression::Na(_)
            | AstExpression::Identifier(_, _) => {}
        }
    }
}

fn count_statement_tree(statements: &[AstStatement], counts: &mut AstCounts) {
    let mut pending = statements.iter().collect::<Vec<_>>();
    while let Some(statement) = pending.pop() {
        counts.statements = counts.statements.saturating_add(1);
        match statement {
            AstStatement::Binding { expression, .. }
            | AstStatement::Destructure { expression, .. } => {
                count_expression_tree(expression, counts);
            }
            AstStatement::Call { call } => {
                if is_output_declaration(&call.path) {
                    counts.outputs = counts.outputs.saturating_add(1);
                }
                counts.expressions = counts.expressions.saturating_add(1);
                for argument in &call.arguments {
                    count_expression_tree(&argument.expression, counts);
                }
            }
            AstStatement::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                count_expression_tree(condition, counts);
                pending.extend(then_branch.iter());
                pending.extend(else_branch.iter());
            }
            AstStatement::While {
                condition, body, ..
            } => {
                count_expression_tree(condition, counts);
                pending.extend(body.iter());
            }
            AstStatement::ForRange {
                start, end, body, ..
            } => {
                count_expression_tree(start, counts);
                count_expression_tree(end, counts);
                pending.extend(body.iter());
            }
            AstStatement::ForEach {
                collection, body, ..
            } => {
                count_expression_tree(collection, counts);
                pending.extend(body.iter());
            }
            AstStatement::Return { expression, .. } => {
                if let Some(expression) = expression {
                    count_expression_tree(expression, counts);
                }
            }
            AstStatement::Break(_) | AstStatement::Continue(_) => {}
        }
    }
}

fn count_ast(ast: &AstProgram) -> AstCounts {
    let mut counts = AstCounts::default();
    if let Some(indicator) = &ast.indicator {
        counts.expressions = counts.expressions.saturating_add(1);
        for argument in &indicator.arguments {
            count_expression_tree(&argument.expression, &mut counts);
        }
    }
    for function in &ast.functions {
        for parameter in &function.parameters {
            if let Some(default) = &parameter.default {
                count_expression_tree(default, &mut counts);
            }
        }
        count_statement_tree(&function.body, &mut counts);
    }
    count_statement_tree(&ast.statements, &mut counts);
    counts
}

pub(super) fn validate_ast_limits(
    ast: &AstProgram,
    path: &Path,
    limits: CompileLimits,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let span = ast
        .indicator
        .as_ref()
        .map_or(Span::new(0, 0), |indicator| indicator.span);
    let counts = count_ast(ast);
    for (actual, limit, label) in [
        (counts.expressions, limits.max_expressions, "expressions"),
        (counts.statements, limits.max_statements, "statements"),
        (ast.functions.len(), limits.max_functions, "functions"),
        (
            counts.outputs,
            limits.max_output_channels,
            "output channels",
        ),
    ] {
        if actual > limit {
            diagnostics.push(Diagnostic::error(
                path,
                span,
                format!("source exceeds compile limit of {limit} {label}"),
            ));
        }
    }
}

pub(super) fn named_color(name: &str) -> Option<u32> {
    match name {
        "color.red" => Some(0xffff_0000),
        "color.green" => Some(0xff00_ff00),
        "color.blue" => Some(0xff00_00ff),
        "color.white" => Some(0xffff_ffff),
        "color.black" => Some(0xff00_0000),
        _ => None,
    }
}

pub(super) fn constant_value(expression: &AstExpression) -> Option<Value> {
    match expression {
        AstExpression::Number(value, _) => {
            if value.fract() == 0.0 {
                Some(Value::Int(*value as i64))
            } else {
                Some(Value::Float(*value))
            }
        }
        AstExpression::String(value, _) => Some(Value::String(value.clone())),
        AstExpression::Boolean(value, _) => Some(Value::Bool(*value)),
        AstExpression::Na(_) => Some(Value::Na),
        AstExpression::Identifier(path, _) => named_color(&path.join(".")).map(Value::Color),
        AstExpression::Call(call) if call.path.join(".") == "color.rgb" => {
            if call
                .arguments
                .iter()
                .any(|argument| argument.name.is_some())
                || !(3..=4).contains(&call.arguments.len())
            {
                return None;
            }
            let mut components = [0_u32; 4];
            for (index, argument) in call.arguments.iter().enumerate() {
                let Value::Int(value) = constant_value(&argument.expression)? else {
                    return None;
                };
                if !(0..=255).contains(&value) {
                    return None;
                }
                components[index] = value as u32;
            }
            if call.arguments.len() == 3 {
                components[3] = 255;
            }
            Some(Value::Color(
                (components[3] << 24)
                    | (components[0] << 16)
                    | (components[1] << 8)
                    | components[2],
            ))
        }
        AstExpression::Unary {
            operator,
            expression,
            ..
        } => match (operator, constant_value(expression)?) {
            (UnaryOperator::Plus, value @ (Value::Int(_) | Value::Float(_))) => Some(value),
            (UnaryOperator::Minus, Value::Int(value)) => Some(Value::Int(-value)),
            (UnaryOperator::Minus, Value::Float(value)) => Some(Value::Float(-value)),
            (UnaryOperator::Not, Value::Bool(value)) => Some(Value::Bool(!value)),
            _ => None,
        },
        _ => None,
    }
}
