use super::super::ast::{AstExpression, AstStatement};
use super::super::lexer::{Token, TokenKind};
use super::{MAX_PARSER_NESTING_DEPTH, Parser};
use crate::diagnostics::Span;

pub(super) enum StatementTask {
    ParseStatement {
        depth: usize,
    },
    ParseBody {
        depth: usize,
    },
    ParseBlock {
        depth: usize,
        statements: Vec<AstStatement>,
    },
    AppendStatement,
    BodyFromStatement,
    FinishIfThen {
        start: usize,
        condition: AstExpression,
        body_depth: usize,
    },
    FinishIfElse {
        start: usize,
        condition: AstExpression,
        then_branch: Vec<AstStatement>,
    },
    FinishBody {
        pending: PendingBody,
    },
}

pub(super) enum PendingBody {
    While {
        start: usize,
        condition: AstExpression,
    },
    ForRange {
        start: usize,
        name: String,
        initial: AstExpression,
        end: AstExpression,
    },
    ForEach {
        start: usize,
        name: String,
        collection: AstExpression,
    },
}

pub(super) enum StatementResult {
    Statement(Option<AstStatement>),
    Body(Option<Vec<AstStatement>>),
}

impl Parser {
    pub(super) fn parse_statement(&mut self) -> Option<AstStatement> {
        match self.run_statement_tasks(StatementTask::ParseStatement { depth: 0 })? {
            StatementResult::Statement(statement) => statement,
            StatementResult::Body(_) => None,
        }
    }

    pub(super) fn looks_like_destructure(&self) -> bool {
        let mut offset = 1;
        let mut names = 0usize;
        loop {
            if !matches!(self.peek_kind(offset), Some(TokenKind::Identifier(_))) {
                return false;
            }
            names += 1;
            offset += 1;
            match self.peek_kind(offset) {
                Some(TokenKind::Comma) => offset += 1,
                Some(TokenKind::RightParen) => {
                    return names > 1
                        && matches!(
                            self.peek_kind(offset + 1),
                            Some(TokenKind::Equal | TokenKind::ColonEqual)
                        );
                }
                _ => return false,
            }
        }
    }

    pub(super) fn parse_destructure_statement(&mut self) -> Option<AstStatement> {
        let start = self.current().span.start;
        self.index += 1;
        let mut names = Vec::new();
        loop {
            let TokenKind::Identifier(name) = self.current().kind.clone() else {
                self.error(self.current().span, "expected a destructured binding name");
                return None;
            };
            self.index += 1;
            names.push(name);
            if !self.consume_kind(TokenKind::Comma) {
                break;
            }
        }
        if !self.consume_kind(TokenKind::RightParen) {
            self.error(
                self.current().span,
                "expected a closing parenthesis in destructuring",
            );
            return None;
        }
        let update = if self.consume_kind(TokenKind::ColonEqual) {
            true
        } else if self.consume_kind(TokenKind::Equal) {
            false
        } else {
            self.error(self.current().span, "destructuring requires '=' or ':='");
            return None;
        };
        let expression = self.parse_expression(0)?;
        Some(AstStatement::Destructure {
            names,
            expression,
            update,
            span: Span::new(start, self.previous_span().end),
        })
    }

    pub(super) fn run_statement_tasks(&mut self, root: StatementTask) -> Option<StatementResult> {
        let mut tasks = vec![root];
        let mut results = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                StatementTask::ParseStatement { depth } => {
                    self.skip_boundaries();
                    if self.consume_keyword("if") {
                        let start = self.previous_span().start;
                        let Some(condition) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        let body_depth = depth.saturating_add(1);
                        tasks.push(StatementTask::FinishIfThen {
                            start,
                            condition,
                            body_depth,
                        });
                        tasks.push(StatementTask::ParseBody { depth: body_depth });
                        continue;
                    }
                    if self.consume_keyword("while") {
                        let start = self.previous_span().start;
                        let Some(condition) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        let body_depth = depth.saturating_add(1);
                        tasks.push(StatementTask::FinishBody {
                            pending: PendingBody::While { start, condition },
                        });
                        tasks.push(StatementTask::ParseBody { depth: body_depth });
                        continue;
                    }
                    if self.consume_keyword("for") {
                        let start = self.previous_span().start;
                        let TokenKind::Identifier(name) = self.current().kind.clone() else {
                            self.error(self.current().span, "expected a loop variable");
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        self.index += 1;
                        let body_depth = depth.saturating_add(1);
                        if self.consume_keyword("in") {
                            let Some(collection) = self.parse_expression(0) else {
                                results.push(StatementResult::Statement(None));
                                continue;
                            };
                            tasks.push(StatementTask::FinishBody {
                                pending: PendingBody::ForEach {
                                    start,
                                    name,
                                    collection,
                                },
                            });
                            tasks.push(StatementTask::ParseBody { depth: body_depth });
                            continue;
                        }
                        if !self.consume_kind(TokenKind::Equal) {
                            self.error(
                                self.current().span,
                                "loops require '=' for a range or 'in' for a collection",
                            );
                            results.push(StatementResult::Statement(None));
                            continue;
                        }
                        let Some(initial) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        if !self.consume_keyword("to") {
                            self.error(
                                self.current().span,
                                "range loops require 'to' and an end expression",
                            );
                            results.push(StatementResult::Statement(None));
                            continue;
                        }
                        let Some(end) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        tasks.push(StatementTask::FinishBody {
                            pending: PendingBody::ForRange {
                                start,
                                name,
                                initial,
                                end,
                            },
                        });
                        tasks.push(StatementTask::ParseBody { depth: body_depth });
                        continue;
                    }
                    if self.consume_keyword("break") {
                        results.push(StatementResult::Statement(Some(AstStatement::Break(
                            self.previous_span(),
                        ))));
                        continue;
                    }
                    if self.consume_keyword("continue") {
                        results.push(StatementResult::Statement(Some(AstStatement::Continue(
                            self.previous_span(),
                        ))));
                        continue;
                    }
                    if self.consume_keyword("return") {
                        let span = self.previous_span();
                        if matches!(
                            self.current().kind,
                            TokenKind::Newline
                                | TokenKind::Semicolon
                                | TokenKind::RightBrace
                                | TokenKind::End
                        ) {
                            results.push(StatementResult::Statement(Some(AstStatement::Return {
                                expression: None,
                                span,
                            })));
                            continue;
                        }
                        let Some(expression) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        results.push(StatementResult::Statement(Some(AstStatement::Return {
                            span: Span::new(span.start, expression.span().end),
                            expression: Some(expression),
                        })));
                        continue;
                    }
                    if self.check(&TokenKind::LeftParen) && self.looks_like_destructure() {
                        results.push(StatementResult::Statement(
                            self.parse_destructure_statement(),
                        ));
                        continue;
                    }
                    let intrabar = self.consume_keyword("varip");
                    let state =
                        intrabar || self.consume_keyword("state") || self.consume_keyword("var");
                    let Token {
                        kind: TokenKind::Identifier(name),
                        span,
                    } = self.current().clone()
                    else {
                        if !self.at_end() {
                            self.error(self.current().span, "expected a binding or call");
                        }
                        results.push(StatementResult::Statement(None));
                        continue;
                    };
                    if self.peek_kind(1) == Some(&TokenKind::Equal)
                        || self.peek_kind(1) == Some(&TokenKind::ColonEqual)
                    {
                        self.index += 2;
                        let update = self.tokens[self.index - 1].kind == TokenKind::ColonEqual;
                        let Some(expression) = self.parse_expression(0) else {
                            results.push(StatementResult::Statement(None));
                            continue;
                        };
                        results.push(StatementResult::Statement(Some(AstStatement::Binding {
                            name,
                            expression,
                            state,
                            intrabar,
                            update,
                            span: Span::new(span.start, self.previous_span().end),
                        })));
                        continue;
                    }
                    if state {
                        self.error(span, "state declarations require a name and expression");
                        results.push(StatementResult::Statement(None));
                        continue;
                    }
                    let Some(call) = self.parse_call() else {
                        results.push(StatementResult::Statement(None));
                        continue;
                    };
                    results.push(StatementResult::Statement(Some(AstStatement::Call {
                        call,
                    })));
                }
                StatementTask::ParseBody { depth } => {
                    if depth > MAX_PARSER_NESTING_DEPTH {
                        self.report_nesting_limit();
                        return None;
                    }
                    self.skip_boundaries();
                    if self.consume_kind(TokenKind::LeftBrace) {
                        self.skip_boundaries();
                        tasks.push(StatementTask::ParseBlock {
                            depth,
                            statements: Vec::new(),
                        });
                    } else {
                        tasks.push(StatementTask::BodyFromStatement);
                        tasks.push(StatementTask::ParseStatement { depth });
                    }
                }
                StatementTask::ParseBlock { depth, statements } => {
                    self.skip_boundaries();
                    if self.consume_kind(TokenKind::RightBrace) {
                        results.push(StatementResult::Body(Some(statements)));
                    } else if self.at_end() {
                        self.error(self.current().span, "expected a closing brace");
                        results.push(StatementResult::Body(Some(statements)));
                    } else {
                        tasks.push(StatementTask::ParseBlock { depth, statements });
                        tasks.push(StatementTask::AppendStatement);
                        tasks.push(StatementTask::ParseStatement { depth });
                    }
                }
                StatementTask::AppendStatement => {
                    let Some(StatementResult::Statement(statement)) = results.pop() else {
                        return None;
                    };
                    let Some(StatementTask::ParseBlock {
                        depth,
                        mut statements,
                    }) = tasks.pop()
                    else {
                        return None;
                    };
                    if let Some(statement) = statement {
                        statements.push(statement);
                    } else {
                        self.synchronize_statement();
                    }
                    tasks.push(StatementTask::ParseBlock { depth, statements });
                }
                StatementTask::BodyFromStatement => {
                    let Some(StatementResult::Statement(statement)) = results.pop() else {
                        return None;
                    };
                    results.push(StatementResult::Body(
                        statement.map(|statement| vec![statement]),
                    ));
                }
                StatementTask::FinishIfThen {
                    start,
                    condition,
                    body_depth,
                } => {
                    let Some(StatementResult::Body(then_branch)) = results.pop() else {
                        return None;
                    };
                    let Some(then_branch) = then_branch else {
                        results.push(StatementResult::Statement(None));
                        continue;
                    };
                    self.skip_boundaries();
                    if self.consume_keyword("else") {
                        tasks.push(StatementTask::FinishIfElse {
                            start,
                            condition,
                            then_branch,
                        });
                        tasks.push(StatementTask::ParseBody { depth: body_depth });
                    } else {
                        results.push(StatementResult::Statement(Some(AstStatement::If {
                            condition,
                            then_branch,
                            else_branch: Vec::new(),
                            span: Span::new(start, self.previous_span().end),
                        })));
                    }
                }
                StatementTask::FinishIfElse {
                    start,
                    condition,
                    then_branch,
                } => {
                    let Some(StatementResult::Body(else_branch)) = results.pop() else {
                        return None;
                    };
                    let Some(else_branch) = else_branch else {
                        results.push(StatementResult::Statement(None));
                        continue;
                    };
                    results.push(StatementResult::Statement(Some(AstStatement::If {
                        condition,
                        then_branch,
                        else_branch,
                        span: Span::new(start, self.previous_span().end),
                    })));
                }
                StatementTask::FinishBody { pending } => {
                    let Some(StatementResult::Body(body)) = results.pop() else {
                        return None;
                    };
                    let Some(body) = body else {
                        results.push(StatementResult::Statement(None));
                        continue;
                    };
                    let end = self.previous_span().end;
                    let statement = match pending {
                        PendingBody::While { start, condition } => AstStatement::While {
                            condition,
                            body,
                            span: Span::new(start, end),
                        },
                        PendingBody::ForRange {
                            start,
                            name,
                            initial,
                            end: range_end,
                        } => AstStatement::ForRange {
                            name,
                            start: initial,
                            end: range_end,
                            body,
                            span: Span::new(start, end),
                        },
                        PendingBody::ForEach {
                            start,
                            name,
                            collection,
                        } => AstStatement::ForEach {
                            name,
                            collection,
                            body,
                            span: Span::new(start, end),
                        },
                    };
                    results.push(StatementResult::Statement(Some(statement)));
                }
            }
        }
        results.pop()
    }

    pub(super) fn parse_block_or_statement(&mut self) -> Option<Vec<AstStatement>> {
        match self.run_statement_tasks(StatementTask::ParseBody { depth: 0 })? {
            StatementResult::Body(body) => body,
            StatementResult::Statement(_) => None,
        }
    }
}
