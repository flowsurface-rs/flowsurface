use super::super::ast::{AstArgument, AstCall, AstExpression};
use super::super::lexer::{Token, TokenKind};
use super::{MAX_PARSER_NESTING_DEPTH, Parser};
use crate::diagnostics::Span;
use crate::hir::{BinaryOperator, UnaryOperator};

pub(super) struct ParsedExpression {
    pub(super) expression: AstExpression,
    pub(super) depth: usize,
}

impl ParsedExpression {
    pub(super) fn leaf(expression: AstExpression) -> Self {
        Self {
            expression,
            depth: 0,
        }
    }
}

pub(super) struct ParsedArgument {
    pub(super) name: Option<String>,
    pub(super) expression: ParsedExpression,
}

pub(super) enum ExpressionTask {
    Parse {
        minimum_precedence: u8,
        depth: usize,
    },
    ParsePrefix {
        depth: usize,
    },
    AfterPrefix {
        minimum_precedence: u8,
        depth: usize,
    },
    Continue {
        expression: ParsedExpression,
        minimum_precedence: u8,
        depth: usize,
    },
    FinishUnary {
        operator: UnaryOperator,
        start: usize,
    },
    FinishBinary {
        left: ParsedExpression,
        operator: BinaryOperator,
        minimum_precedence: u8,
        depth: usize,
    },
    FinishIndex {
        expression: ParsedExpression,
        minimum_precedence: u8,
        depth: usize,
    },
    FinishParenthesized {
        start: usize,
        values: Vec<ParsedExpression>,
        depth: usize,
    },
    FinishArray {
        start: usize,
        values: Vec<ParsedExpression>,
        depth: usize,
    },
    ParseCallArguments {
        start: usize,
        path: Vec<String>,
        arguments: Vec<ParsedArgument>,
        depth: usize,
    },
    FinishCallArgument {
        start: usize,
        path: Vec<String>,
        arguments: Vec<ParsedArgument>,
        named: Option<String>,
        depth: usize,
    },
    ParseRecordFields {
        start: usize,
        fields: Vec<(String, ParsedExpression)>,
        depth: usize,
    },
    FinishRecordField {
        start: usize,
        fields: Vec<(String, ParsedExpression)>,
        name: String,
        depth: usize,
    },
    FinishSwitchOperand {
        start: usize,
        depth: usize,
    },
    ParseSwitchBranches {
        start: usize,
        operand: ParsedExpression,
        branches: Vec<(ParsedExpression, ParsedExpression)>,
        fallback: Option<Box<ParsedExpression>>,
        depth: usize,
    },
    FinishSwitchCondition {
        start: usize,
        operand: ParsedExpression,
        branches: Vec<(ParsedExpression, ParsedExpression)>,
        fallback: Option<Box<ParsedExpression>>,
        depth: usize,
    },
    FinishSwitchResult {
        start: usize,
        operand: ParsedExpression,
        branches: Vec<(ParsedExpression, ParsedExpression)>,
        fallback: Option<Box<ParsedExpression>>,
        condition: Option<ParsedExpression>,
        depth: usize,
    },
}

impl Parser {
    pub(super) fn parse_expression(&mut self, minimum_precedence: u8) -> Option<AstExpression> {
        self.parse_expression_with_depth(minimum_precedence)
            .map(|parsed| parsed.expression)
    }

    pub(super) fn parsed_expression(
        &mut self,
        expression: AstExpression,
        child_depth: usize,
    ) -> Option<ParsedExpression> {
        let depth = child_depth.saturating_add(1);
        if depth > MAX_PARSER_NESTING_DEPTH {
            self.report_nesting_limit();
            return None;
        }
        Some(ParsedExpression { expression, depth })
    }

    pub(super) fn parsed_expression_from_children(
        &mut self,
        expression: AstExpression,
        child_depths: impl IntoIterator<Item = usize>,
    ) -> Option<ParsedExpression> {
        self.parsed_expression(expression, child_depths.into_iter().max().unwrap_or(0))
    }

    pub(super) fn parse_expression_with_depth(
        &mut self,
        minimum_precedence: u8,
    ) -> Option<ParsedExpression> {
        let mut tasks = vec![ExpressionTask::Parse {
            minimum_precedence,
            depth: 0,
        }];
        let mut values = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                ExpressionTask::Parse {
                    minimum_precedence,
                    depth,
                } => {
                    if depth > MAX_PARSER_NESTING_DEPTH {
                        self.report_nesting_limit();
                        return None;
                    }
                    tasks.push(ExpressionTask::AfterPrefix {
                        minimum_precedence,
                        depth,
                    });
                    tasks.push(ExpressionTask::ParsePrefix { depth });
                }
                ExpressionTask::ParsePrefix { depth } => {
                    let token = self.current().clone();
                    match token.kind {
                        TokenKind::Number(value) => {
                            self.index += 1;
                            values.push(ParsedExpression::leaf(AstExpression::Number(
                                value, token.span,
                            )));
                        }
                        TokenKind::String(value) => {
                            self.index += 1;
                            values.push(ParsedExpression::leaf(AstExpression::String(
                                value, token.span,
                            )));
                        }
                        TokenKind::Identifier(name) => {
                            self.index += 1;
                            if name == "switch" && !self.check(&TokenKind::Dot) {
                                tasks.push(ExpressionTask::FinishSwitchOperand {
                                    start: token.span.start,
                                    depth,
                                });
                                tasks.push(ExpressionTask::Parse {
                                    minimum_precedence: 0,
                                    depth: depth.saturating_add(1),
                                });
                                continue;
                            }
                            let mut path = vec![name];
                            while self.consume_kind(TokenKind::Dot) {
                                match self.current().kind.clone() {
                                    TokenKind::Identifier(segment) => {
                                        path.push(segment);
                                        self.index += 1;
                                    }
                                    _ => {
                                        self.error(
                                            self.current().span,
                                            "expected a name after a dot",
                                        );
                                        break;
                                    }
                                }
                            }
                            if self.check(&TokenKind::LeftParen) {
                                let start = token.span.start;
                                self.index += 1;
                                self.skip_inline_boundaries();
                                if self.consume_kind(TokenKind::RightParen) {
                                    let call = AstExpression::Call(AstCall {
                                        path,
                                        arguments: Vec::new(),
                                        span: Span::new(start, self.previous_span().end),
                                    });
                                    values.push(self.parsed_expression(call, 0)?);
                                } else {
                                    tasks.push(ExpressionTask::ParseCallArguments {
                                        start,
                                        path,
                                        arguments: Vec::new(),
                                        depth,
                                    });
                                }
                            } else {
                                let end = self.previous_span().end;
                                let expression = match path.as_slice() {
                                    [name] if name == "true" => {
                                        AstExpression::Boolean(true, token.span)
                                    }
                                    [name] if name == "false" => {
                                        AstExpression::Boolean(false, token.span)
                                    }
                                    [name] if name == "na" => AstExpression::Na(token.span),
                                    _ => AstExpression::Identifier(
                                        path,
                                        Span::new(token.span.start, end),
                                    ),
                                };
                                values.push(ParsedExpression::leaf(expression));
                            }
                        }
                        TokenKind::Minus | TokenKind::Plus | TokenKind::Bang => {
                            self.index += 1;
                            let operator = match token.kind {
                                TokenKind::Minus => UnaryOperator::Minus,
                                TokenKind::Plus => UnaryOperator::Plus,
                                TokenKind::Bang => UnaryOperator::Not,
                                _ => unreachable!(),
                            };
                            tasks.push(ExpressionTask::FinishUnary {
                                operator,
                                start: token.span.start,
                            });
                            tasks.push(ExpressionTask::Parse {
                                minimum_precedence: 7,
                                depth: depth.saturating_add(1),
                            });
                        }
                        TokenKind::LeftParen => {
                            self.index += 1;
                            self.skip_inline_boundaries();
                            if self.consume_kind(TokenKind::RightParen) {
                                self.error(token.span, "empty parentheses are not an expression");
                                return None;
                            }
                            tasks.push(ExpressionTask::FinishParenthesized {
                                start: token.span.start,
                                values: Vec::new(),
                                depth,
                            });
                            tasks.push(ExpressionTask::Parse {
                                minimum_precedence: 0,
                                depth: depth.saturating_add(1),
                            });
                        }
                        TokenKind::LeftBracket => {
                            self.index += 1;
                            self.skip_inline_boundaries();
                            if self.consume_kind(TokenKind::RightBracket) {
                                let array = AstExpression::Array(
                                    Vec::new(),
                                    Span::new(token.span.start, self.previous_span().end),
                                );
                                values.push(self.parsed_expression(array, 0)?);
                            } else {
                                tasks.push(ExpressionTask::FinishArray {
                                    start: token.span.start,
                                    values: Vec::new(),
                                    depth,
                                });
                                tasks.push(ExpressionTask::Parse {
                                    minimum_precedence: 0,
                                    depth: depth.saturating_add(1),
                                });
                            }
                        }
                        TokenKind::LeftBrace => {
                            self.index += 1;
                            self.skip_boundaries();
                            if self.consume_kind(TokenKind::RightBrace) {
                                let record = AstExpression::Record(
                                    Vec::new(),
                                    Span::new(token.span.start, self.previous_span().end),
                                );
                                values.push(self.parsed_expression(record, 0)?);
                            } else {
                                tasks.push(ExpressionTask::ParseRecordFields {
                                    start: token.span.start,
                                    fields: Vec::new(),
                                    depth,
                                });
                            }
                        }
                        _ => {
                            self.error(token.span, "expected an expression");
                            return None;
                        }
                    }
                }
                ExpressionTask::AfterPrefix {
                    minimum_precedence,
                    depth,
                } => {
                    let expression = values.pop()?;
                    tasks.push(ExpressionTask::Continue {
                        expression,
                        minimum_precedence,
                        depth,
                    });
                }
                ExpressionTask::Continue {
                    expression,
                    minimum_precedence,
                    depth,
                } => {
                    if expression.depth > MAX_PARSER_NESTING_DEPTH {
                        self.report_nesting_limit();
                        return None;
                    }
                    if self.consume_kind(TokenKind::LeftBracket) {
                        tasks.push(ExpressionTask::FinishIndex {
                            expression,
                            minimum_precedence,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    } else if self.consume_kind(TokenKind::Dot) {
                        let Token {
                            kind: TokenKind::Identifier(field),
                            span,
                        } = self.current().clone()
                        else {
                            self.error(self.current().span, "expected a field name after a dot");
                            values.push(expression);
                            continue;
                        };
                        self.index += 1;
                        let base_depth = expression.depth;
                        let field = AstExpression::Field {
                            span: Span::new(expression.expression.span().start, span.end),
                            expression: Box::new(expression.expression),
                            field,
                        };
                        let expression = self.parsed_expression(field, base_depth)?;
                        tasks.push(ExpressionTask::Continue {
                            expression,
                            minimum_precedence,
                            depth: depth.saturating_add(1),
                        });
                    } else if let Some((operator, precedence)) = self.binary_operator() {
                        if precedence < minimum_precedence {
                            values.push(expression);
                        } else {
                            self.index += 1;
                            tasks.push(ExpressionTask::FinishBinary {
                                left: expression,
                                operator,
                                minimum_precedence,
                                depth,
                            });
                            tasks.push(ExpressionTask::Parse {
                                minimum_precedence: precedence + 1,
                                depth: depth.saturating_add(1),
                            });
                        }
                    } else {
                        values.push(expression);
                    }
                }
                ExpressionTask::FinishUnary { operator, start } => {
                    let expression = values.pop()?;
                    let end = expression.expression.span().end;
                    let child_depth = expression.depth;
                    let unary = AstExpression::Unary {
                        operator,
                        expression: Box::new(expression.expression),
                        span: Span::new(start, end),
                    };
                    values.push(self.parsed_expression(unary, child_depth)?);
                }
                ExpressionTask::FinishBinary {
                    left,
                    operator,
                    minimum_precedence,
                    depth,
                } => {
                    let right = values.pop()?;
                    let left_depth = left.depth;
                    let right_depth = right.depth;
                    let expression = AstExpression::Binary {
                        span: Span::new(left.expression.span().start, right.expression.span().end),
                        left: Box::new(left.expression),
                        operator,
                        right: Box::new(right.expression),
                    };
                    let expression = self
                        .parsed_expression_from_children(expression, [left_depth, right_depth])?;
                    tasks.push(ExpressionTask::Continue {
                        expression,
                        minimum_precedence,
                        depth: depth.saturating_add(1),
                    });
                }
                ExpressionTask::FinishIndex {
                    expression,
                    minimum_precedence,
                    depth,
                } => {
                    let index = values.pop()?;
                    let end = self.current().span;
                    if !self.consume_kind(TokenKind::RightBracket) {
                        self.error(
                            self.current().span,
                            "expected a closing bracket after history index",
                        );
                    }
                    let expression_depth = expression.depth;
                    let index_depth = index.depth;
                    let expression = AstExpression::Index {
                        span: Span::new(expression.expression.span().start, end.end),
                        expression: Box::new(expression.expression),
                        index: Box::new(index.expression),
                    };
                    let expression = self.parsed_expression_from_children(
                        expression,
                        [expression_depth, index_depth],
                    )?;
                    tasks.push(ExpressionTask::Continue {
                        expression,
                        minimum_precedence,
                        depth: depth.saturating_add(1),
                    });
                }
                ExpressionTask::FinishParenthesized {
                    start,
                    values: mut expressions,
                    depth,
                } => {
                    expressions.push(values.pop()?);
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::Comma) {
                        self.skip_inline_boundaries();
                        tasks.push(ExpressionTask::FinishParenthesized {
                            start,
                            values: expressions,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    } else {
                        if !self.consume_kind(TokenKind::RightParen) {
                            self.error(self.current().span, "expected a closing parenthesis");
                        }
                        if expressions.len() == 1 {
                            values.push(expressions.pop().expect("parenthesized value exists"));
                        } else {
                            let tuple_depth = expressions
                                .iter()
                                .map(|expression| expression.depth)
                                .max()
                                .unwrap_or(0);
                            let tuple = AstExpression::Tuple(
                                expressions
                                    .into_iter()
                                    .map(|expression| expression.expression)
                                    .collect(),
                                Span::new(start, self.previous_span().end),
                            );
                            values.push(self.parsed_expression(tuple, tuple_depth)?);
                        }
                    }
                }
                ExpressionTask::FinishArray {
                    start,
                    values: mut expressions,
                    depth,
                } => {
                    expressions.push(values.pop()?);
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::Comma) {
                        self.skip_inline_boundaries();
                        if self.consume_kind(TokenKind::RightBracket) {
                            let array_depth = expressions
                                .iter()
                                .map(|expression| expression.depth)
                                .max()
                                .unwrap_or(0);
                            let array = AstExpression::Array(
                                expressions
                                    .into_iter()
                                    .map(|expression| expression.expression)
                                    .collect(),
                                Span::new(start, self.previous_span().end),
                            );
                            values.push(self.parsed_expression(array, array_depth)?);
                        } else {
                            tasks.push(ExpressionTask::FinishArray {
                                start,
                                values: expressions,
                                depth,
                            });
                            tasks.push(ExpressionTask::Parse {
                                minimum_precedence: 0,
                                depth: depth.saturating_add(1),
                            });
                        }
                    } else {
                        if !self.consume_kind(TokenKind::RightBracket) {
                            self.error(
                                self.current().span,
                                "expected a closing bracket after array literal",
                            );
                        }
                        let array_depth = expressions
                            .iter()
                            .map(|expression| expression.depth)
                            .max()
                            .unwrap_or(0);
                        let array = AstExpression::Array(
                            expressions
                                .into_iter()
                                .map(|expression| expression.expression)
                                .collect(),
                            Span::new(start, self.previous_span().end),
                        );
                        values.push(self.parsed_expression(array, array_depth)?);
                    }
                }
                ExpressionTask::ParseCallArguments {
                    start,
                    path,
                    arguments,
                    depth,
                } => {
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::RightParen) {
                        let call = AstExpression::Call(AstCall {
                            path,
                            arguments: Vec::new(),
                            span: Span::new(start, self.previous_span().end),
                        });
                        values.push(self.parsed_expression(call, 0)?);
                    } else {
                        let named = match (self.current().kind.clone(), self.peek_kind(1)) {
                            (TokenKind::Identifier(name), Some(TokenKind::Equal)) => {
                                self.index += 2;
                                Some(name)
                            }
                            _ => None,
                        };
                        tasks.push(ExpressionTask::FinishCallArgument {
                            start,
                            path,
                            arguments,
                            named,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    }
                }
                ExpressionTask::FinishCallArgument {
                    start,
                    path,
                    mut arguments,
                    named,
                    depth,
                } => {
                    let expression = values.pop()?;
                    arguments.push(ParsedArgument {
                        name: named,
                        expression,
                    });
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::Comma) {
                        self.skip_inline_boundaries();
                        tasks.push(ExpressionTask::ParseCallArguments {
                            start,
                            path,
                            arguments,
                            depth,
                        });
                    } else if self.consume_kind(TokenKind::RightParen) {
                        let call_depth = arguments
                            .iter()
                            .map(|argument| argument.expression.depth)
                            .max()
                            .unwrap_or(0);
                        let call = AstExpression::Call(AstCall {
                            path,
                            arguments: arguments
                                .into_iter()
                                .map(|argument| AstArgument {
                                    name: argument.name,
                                    expression: argument.expression.expression,
                                })
                                .collect(),
                            span: Span::new(start, self.previous_span().end),
                        });
                        values.push(self.parsed_expression(call, call_depth)?);
                    } else {
                        self.error(
                            self.current().span,
                            "expected a closing parenthesis after arguments",
                        );
                        return None;
                    }
                }
                ExpressionTask::ParseRecordFields {
                    start,
                    fields,
                    depth,
                } => {
                    self.skip_boundaries();
                    if self.consume_kind(TokenKind::RightBrace) {
                        let field_depth = fields
                            .iter()
                            .map(|(_, field)| field.depth)
                            .max()
                            .unwrap_or(0);
                        let record = AstExpression::Record(
                            fields
                                .into_iter()
                                .map(|(name, field)| (name, field.expression))
                                .collect(),
                            Span::new(start, self.previous_span().end),
                        );
                        values.push(self.parsed_expression(record, field_depth)?);
                    } else {
                        let TokenKind::Identifier(name) = self.current().kind.clone() else {
                            self.error(self.current().span, "record fields require a name");
                            return None;
                        };
                        self.index += 1;
                        if !self.consume_kind(TokenKind::Colon) {
                            self.error(self.current().span, "record fields require ':'");
                            return None;
                        }
                        tasks.push(ExpressionTask::FinishRecordField {
                            start,
                            fields,
                            name,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    }
                }
                ExpressionTask::FinishRecordField {
                    start,
                    mut fields,
                    name,
                    depth,
                } => {
                    fields.push((name, values.pop()?));
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::Comma) {
                        self.skip_boundaries();
                        tasks.push(ExpressionTask::ParseRecordFields {
                            start,
                            fields,
                            depth,
                        });
                    } else {
                        self.skip_boundaries();
                        if !self.consume_kind(TokenKind::RightBrace) {
                            self.error(
                                self.current().span,
                                "expected a comma between record fields",
                            );
                            return None;
                        }
                        let field_depth = fields
                            .iter()
                            .map(|(_, field)| field.depth)
                            .max()
                            .unwrap_or(0);
                        let record = AstExpression::Record(
                            fields
                                .into_iter()
                                .map(|(name, field)| (name, field.expression))
                                .collect(),
                            Span::new(start, self.previous_span().end),
                        );
                        values.push(self.parsed_expression(record, field_depth)?);
                    }
                }
                ExpressionTask::FinishSwitchOperand { start, depth } => {
                    let operand = values.pop()?;
                    self.skip_boundaries();
                    if !self.consume_kind(TokenKind::LeftBrace) {
                        self.error(self.current().span, "switch requires a branch block");
                        return None;
                    }
                    self.skip_boundaries();
                    tasks.push(ExpressionTask::ParseSwitchBranches {
                        start,
                        operand,
                        branches: Vec::new(),
                        fallback: None,
                        depth,
                    });
                }
                ExpressionTask::ParseSwitchBranches {
                    start,
                    operand,
                    branches,
                    fallback,
                    depth,
                } => {
                    self.skip_boundaries();
                    if self.consume_kind(TokenKind::RightBrace) {
                        let switch_depth =
                            std::iter::once(operand.depth)
                                .chain(branches.iter().flat_map(|(condition, result)| {
                                    [condition.depth, result.depth]
                                }))
                                .chain(fallback.iter().map(|result| result.depth))
                                .max()
                                .unwrap_or(0);
                        let switch = AstExpression::Switch {
                            operand: Box::new(operand.expression),
                            branches: branches
                                .into_iter()
                                .map(|(condition, result)| {
                                    (condition.expression, result.expression)
                                })
                                .collect(),
                            fallback: fallback.map(|result| Box::new(result.expression)),
                            span: Span::new(start, self.previous_span().end),
                        };
                        values.push(self.parsed_expression(switch, switch_depth)?);
                    } else if self.is_named("default") {
                        self.index += 1;
                        if !self.consume_kind(TokenKind::Arrow) {
                            self.error(self.current().span, "switch branches require '=>' ");
                            return None;
                        }
                        tasks.push(ExpressionTask::FinishSwitchResult {
                            start,
                            operand,
                            branches,
                            fallback,
                            condition: None,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    } else {
                        tasks.push(ExpressionTask::FinishSwitchCondition {
                            start,
                            operand,
                            branches,
                            fallback,
                            depth,
                        });
                        tasks.push(ExpressionTask::Parse {
                            minimum_precedence: 0,
                            depth: depth.saturating_add(1),
                        });
                    }
                }
                ExpressionTask::FinishSwitchCondition {
                    start,
                    operand,
                    branches,
                    fallback,
                    depth,
                } => {
                    let condition = values.pop()?;
                    if !self.consume_kind(TokenKind::Arrow) {
                        self.error(self.current().span, "switch branches require '=>' ");
                        return None;
                    }
                    tasks.push(ExpressionTask::FinishSwitchResult {
                        start,
                        operand,
                        branches,
                        fallback,
                        condition: Some(condition),
                        depth,
                    });
                    tasks.push(ExpressionTask::Parse {
                        minimum_precedence: 0,
                        depth: depth.saturating_add(1),
                    });
                }
                ExpressionTask::FinishSwitchResult {
                    start,
                    operand,
                    mut branches,
                    mut fallback,
                    condition,
                    depth,
                } => {
                    let result = values.pop()?;
                    if let Some(condition) = condition {
                        branches.push((condition, result));
                    } else {
                        fallback = Some(Box::new(result));
                    }
                    self.skip_inline_boundaries();
                    if self.consume_kind(TokenKind::Comma) {
                        self.skip_boundaries();
                        tasks.push(ExpressionTask::ParseSwitchBranches {
                            start,
                            operand,
                            branches,
                            fallback,
                            depth,
                        });
                    } else {
                        self.skip_boundaries();
                        if !self.consume_kind(TokenKind::RightBrace) {
                            self.error(
                                self.current().span,
                                "expected a comma between switch branches",
                            );
                            return None;
                        }
                        let switch_depth =
                            std::iter::once(operand.depth)
                                .chain(branches.iter().flat_map(|(condition, result)| {
                                    [condition.depth, result.depth]
                                }))
                                .chain(fallback.iter().map(|result| result.depth))
                                .max()
                                .unwrap_or(0);
                        let switch = AstExpression::Switch {
                            operand: Box::new(operand.expression),
                            branches: branches
                                .into_iter()
                                .map(|(condition, result)| {
                                    (condition.expression, result.expression)
                                })
                                .collect(),
                            fallback: fallback.map(|result| Box::new(result.expression)),
                            span: Span::new(start, self.previous_span().end),
                        };
                        values.push(self.parsed_expression(switch, switch_depth)?);
                    }
                }
            }
        }
        values.pop()
    }
}
