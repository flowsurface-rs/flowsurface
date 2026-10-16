mod expressions;
mod statements;

use super::ast::constant_value;
use super::ast::{AstArgument, AstCall, AstEnum, AstFunction, AstParameter, AstProgram};
use super::lexer::{Token, TokenKind};
use crate::diagnostics::{Diagnostic, Span};
use crate::hir::BinaryOperator;
use crate::types::Value;
use std::path::PathBuf;

pub(super) const MAX_PARSER_NESTING_DEPTH: usize = 256;

pub(super) struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) path: PathBuf,
    pub(super) index: usize,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) nesting_limit_reported: bool,
}

pub(super) struct ParseOutput {
    pub(super) ast: AstProgram,
    pub(super) diagnostics: Vec<Diagnostic>,
}

pub(super) fn parse(tokens: Vec<Token>, path: PathBuf) -> ParseOutput {
    let mut parser = Parser::new(tokens, path);
    let ast = parser.parse_program();
    ParseOutput {
        ast,
        diagnostics: parser.diagnostics,
    }
}

impl Parser {
    pub(super) fn new(tokens: Vec<Token>, path: PathBuf) -> Self {
        Self {
            tokens,
            path,
            index: 0,
            diagnostics: Vec::new(),
            nesting_limit_reported: false,
        }
    }

    pub(super) fn parse_program(&mut self) -> AstProgram {
        self.skip_boundaries();
        self.parse_language_directive();
        let indicator = if self.is_named("indicator") {
            let value = self.parse_call();
            self.skip_boundaries();
            value
        } else {
            None
        };
        let mut functions = Vec::new();
        let mut enums = Vec::new();
        let mut statements = Vec::new();
        while !self.at_end() {
            if self.consume_keyword("enum") {
                if let Some(item) = self.parse_enum() {
                    enums.push(item);
                } else {
                    self.synchronize_statement();
                }
                self.skip_boundaries();
                continue;
            }
            if self.consume_keyword("fn") {
                if let Some(function) = self.parse_function() {
                    functions.push(function);
                } else {
                    self.synchronize_statement();
                }
                self.skip_boundaries();
                continue;
            }
            if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            } else {
                self.synchronize_statement();
            }
            self.skip_boundaries();
        }
        AstProgram {
            indicator,
            functions,
            enums,
            statements,
        }
    }

    pub(super) fn parse_language_directive(&mut self) {
        if !self.is_named("language") {
            self.error(self.current().span, "source must begin with language(1)");
            return;
        }
        let Some(call) = self.parse_call() else {
            self.error(self.current().span, "source must begin with language(1)");
            return;
        };
        self.skip_boundaries();

        let valid_name = call.path.len() == 1 && call.path[0] == "language";
        let valid_arguments = call.arguments.len() == 1
            && call.arguments[0].name.is_none()
            && matches!(
                constant_value(&call.arguments[0].expression),
                Some(Value::Int(1))
            );
        if !valid_name || !valid_arguments {
            self.error(call.span, "source must begin with language(1)");
        }
    }

    pub(super) fn parse_enum(&mut self) -> Option<AstEnum> {
        let start = self.previous_span().start;
        let TokenKind::Identifier(name) = self.current().kind.clone() else {
            self.error(self.current().span, "expected an enum name");
            return None;
        };
        self.index += 1;
        if !self.consume_kind(TokenKind::LeftBrace) {
            self.error(
                self.current().span,
                "expected an opening brace after enum name",
            );
            return None;
        }
        let mut variants = Vec::new();
        self.skip_boundaries();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let TokenKind::Identifier(variant) = self.current().kind.clone() else {
                self.error(self.current().span, "expected an enum variant name");
                return None;
            };
            self.index += 1;
            variants.push(variant);
            self.skip_boundaries();
            if !self.consume_kind(TokenKind::Comma) {
                self.skip_boundaries();
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
                if matches!(self.current().kind, TokenKind::Identifier(_)) {
                    continue;
                }
                self.error(
                    self.current().span,
                    "expected a comma between enum variants",
                );
                return None;
            }
            self.skip_boundaries();
        }
        if !self.consume_kind(TokenKind::RightBrace) {
            self.error(
                self.current().span,
                "expected a closing brace after enum variants",
            );
            return None;
        }
        Some(AstEnum {
            name,
            variants,
            span: Span::new(start, self.previous_span().end),
        })
    }

    pub(super) fn parse_function(&mut self) -> Option<AstFunction> {
        let start = self.previous_span().start;
        let TokenKind::Identifier(name) = self.current().kind.clone() else {
            self.error(self.current().span, "expected a function name");
            return None;
        };
        self.index += 1;
        if !self.consume_kind(TokenKind::LeftParen) {
            self.error(
                self.current().span,
                "expected an opening parenthesis after function name",
            );
            return None;
        }
        let mut parameters = Vec::new();
        self.skip_inline_boundaries();
        while !self.check(&TokenKind::RightParen) && !self.at_end() {
            let TokenKind::Identifier(parameter) = self.current().kind.clone() else {
                self.error(self.current().span, "expected a function parameter name");
                return None;
            };
            self.index += 1;
            self.skip_inline_boundaries();
            let default = if self.consume_kind(TokenKind::Equal) {
                Some(self.parse_expression(0)?)
            } else {
                None
            };
            parameters.push(AstParameter {
                name: parameter,
                default,
            });
            self.skip_inline_boundaries();
            if !self.consume_kind(TokenKind::Comma) {
                break;
            }
            self.skip_inline_boundaries();
        }
        if !self.consume_kind(TokenKind::RightParen) {
            self.error(
                self.current().span,
                "expected a closing parenthesis after parameters",
            );
            return None;
        }
        let body = self.parse_block_or_statement()?;
        Some(AstFunction {
            name,
            parameters,
            body,
            span: Span::new(start, self.previous_span().end),
        })
    }

    pub(super) fn parse_call(&mut self) -> Option<AstCall> {
        let start = self.current().span.start;
        let path = self.parse_path()?;
        if !self.consume_kind(TokenKind::LeftParen) {
            self.error(
                self.current().span,
                "expected an opening parenthesis after call name",
            );
            return None;
        }
        let arguments = self.parse_call_arguments()?;
        Some(AstCall {
            path,
            arguments,
            span: Span::new(start, self.previous_span().end),
        })
    }

    pub(super) fn parse_call_arguments(&mut self) -> Option<Vec<AstArgument>> {
        let mut arguments = Vec::new();
        self.skip_inline_boundaries();
        while !self.check(&TokenKind::RightParen) && !self.at_end() {
            let named = match (self.current().kind.clone(), self.peek_kind(1)) {
                (TokenKind::Identifier(name), Some(TokenKind::Equal)) => {
                    self.index += 2;
                    Some(name)
                }
                _ => None,
            };
            let expression = self.parse_expression(0)?;
            arguments.push(AstArgument {
                name: named,
                expression,
            });
            self.skip_inline_boundaries();
            if !self.consume_kind(TokenKind::Comma) {
                break;
            }
            self.skip_inline_boundaries();
        }
        if !self.consume_kind(TokenKind::RightParen) {
            self.error(
                self.current().span,
                "expected a closing parenthesis after arguments",
            );
            return None;
        }
        Some(arguments)
    }

    pub(super) fn parse_path(&mut self) -> Option<Vec<String>> {
        let TokenKind::Identifier(first) = self.current().kind.clone() else {
            self.error(self.current().span, "expected a name");
            return None;
        };
        self.index += 1;
        let mut path = vec![first];
        while self.consume_kind(TokenKind::Dot) {
            let TokenKind::Identifier(segment) = self.current().kind.clone() else {
                self.error(self.current().span, "expected a name after a dot");
                return None;
            };
            path.push(segment);
            self.index += 1;
        }
        Some(path)
    }

    pub(super) fn binary_operator(&self) -> Option<(BinaryOperator, u8)> {
        Some(match self.current().kind {
            TokenKind::OrOr => (BinaryOperator::Or, 1),
            TokenKind::AndAnd => (BinaryOperator::And, 2),
            TokenKind::EqualEqual => (BinaryOperator::Equal, 3),
            TokenKind::BangEqual => (BinaryOperator::NotEqual, 3),
            TokenKind::Less => (BinaryOperator::Less, 4),
            TokenKind::LessEqual => (BinaryOperator::LessEqual, 4),
            TokenKind::Greater => (BinaryOperator::Greater, 4),
            TokenKind::GreaterEqual => (BinaryOperator::GreaterEqual, 4),
            TokenKind::Plus => (BinaryOperator::Add, 5),
            TokenKind::Minus => (BinaryOperator::Subtract, 5),
            TokenKind::Star => (BinaryOperator::Multiply, 6),
            TokenKind::Slash => (BinaryOperator::Divide, 6),
            TokenKind::Percent => (BinaryOperator::Remainder, 6),
            _ => return None,
        })
    }

    pub(super) fn synchronize_statement(&mut self) {
        while !self.at_end()
            && !matches!(
                self.current().kind,
                TokenKind::Newline | TokenKind::Semicolon
            )
        {
            self.index += 1;
        }
        self.skip_boundaries();
    }

    pub(super) fn skip_boundaries(&mut self) {
        while matches!(
            self.current().kind,
            TokenKind::Newline | TokenKind::Semicolon
        ) {
            self.index += 1;
        }
    }

    pub(super) fn skip_inline_boundaries(&mut self) {
        while matches!(self.current().kind, TokenKind::Newline) {
            self.index += 1;
        }
    }

    pub(super) fn is_named(&self, name: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Identifier(value) if value == name)
    }

    pub(super) fn consume_keyword(&mut self, keyword: &str) -> bool {
        if self.is_named(keyword) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn consume_kind(&mut self, kind: TokenKind) -> bool {
        if self.current().kind == kind {
            self.index += 1;
            true
        } else {
            false
        }
    }

    pub(super) fn check(&self, kind: &TokenKind) -> bool {
        &self.current().kind == kind
    }

    pub(super) fn peek_kind(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens
            .get(self.index + offset)
            .map(|token| &token.kind)
    }

    pub(super) fn previous_span(&self) -> Span {
        self.tokens
            .get(self.index.saturating_sub(1))
            .map_or(self.current().span, |token| token.span)
    }

    pub(super) fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len() - 1)]
    }

    pub(super) fn at_end(&self) -> bool {
        matches!(self.current().kind, TokenKind::End)
    }

    pub(super) fn report_nesting_limit(&mut self) {
        if !self.nesting_limit_reported {
            self.error(
                self.current().span,
                format!("source exceeds parser nesting limit of {MAX_PARSER_NESTING_DEPTH}"),
            );
            self.nesting_limit_reported = true;
        }
        self.index = self.tokens.len().saturating_sub(1);
    }

    pub(super) fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(self.path.clone(), span, message));
    }
}
