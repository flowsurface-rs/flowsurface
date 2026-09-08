use crate::DataRequirements;
use crate::diagnostics::{Diagnostic, Span};
use crate::frame::FrameField;
use crate::hir::{
    BarStateField, BinaryOperator, CallSiteId, ConstantId, ExprId, ExpressionKind, FunctionId,
    HirFunction, HirProgram, LocalSlot, SeriesSlot, StateSlot, StatementKind, TypedExpression,
    TypedStatement, UnaryOperator,
};
use crate::intrinsic::IntrinsicRegistry;
use crate::scene::{
    IndicatorPresentation, OutputPrecision, PaneTarget, PlotDescriptor, PlotScale, PlotStyle,
    PlotValueType, PresentationAxis, PresentationColumn, PresentationColumnColor,
    PresentationDataKind, PresentationFill, PresentationGeometry, PresentationInputSource,
    PresentationLabelMode, PresentationLayer, PresentationMark, PresentationMarkKind,
    PresentationPrecision, PresentationReadout, PresentationScale, PresentationSegment,
    PresentationStyle, PresentationViewKind, ReadoutBehavior,
};
use crate::types::{
    IndicatorDescriptor, IndicatorId, InputDescriptor, InputOption, InputValue, Qualifier, Type,
    Value, ValueType,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Identifier(String),
    Number(f64),
    String(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Equal,
    ColonEqual,
    Colon,
    EqualEqual,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Arrow,
    AndAnd,
    OrOr,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Dot,
    Newline,
    Semicolon,
    End,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
enum AstExpression {
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
    fn span(&self) -> Span {
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
struct AstCall {
    path: Vec<String>,
    arguments: Vec<AstArgument>,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
struct AstArgument {
    name: Option<String>,
    expression: AstExpression,
}

#[derive(Debug, Clone, PartialEq)]
enum AstStatement {
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
struct AstEnum {
    name: String,
    variants: Vec<String>,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
struct AstProgram {
    indicator: Option<AstCall>,
    functions: Vec<AstFunction>,
    enums: Vec<AstEnum>,
    statements: Vec<AstStatement>,
}

#[derive(Debug, Clone, PartialEq)]
struct AstFunction {
    name: String,
    parameters: Vec<AstParameter>,
    body: Vec<AstStatement>,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
struct AstParameter {
    name: String,
    default: Option<AstExpression>,
}

pub(crate) fn compile_with_registry(
    source: &str,
    path: &Path,
    registry: IntrinsicRegistry,
) -> Result<crate::CompiledIndicator, Vec<Diagnostic>> {
    let mut lexer = Lexer::new(source, path.to_path_buf());
    let tokens = lexer.lex();
    let mut diagnostics = lexer.diagnostics;
    let mut parser = Parser::new(tokens, path.to_path_buf());
    let ast = parser.parse_program();
    diagnostics.extend(parser.diagnostics);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut compiler = Compiler::new(path.to_path_buf(), registry);
    let (descriptor, program, requirements) = compiler.compile_program(ast);
    diagnostics.extend(compiler.diagnostics);
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::Severity::Error)
    {
        Err(diagnostics)
    } else {
        Ok(crate::CompiledIndicator {
            descriptor,
            requirements,
            program,
            registry: compiler.registry,
        })
    }
}

struct Lexer {
    source: Vec<char>,
    path: PathBuf,
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

impl Lexer {
    fn new(source: &str, path: PathBuf) -> Self {
        Self {
            source: source.chars().collect(),
            path,
            index: 0,
            diagnostics: Vec::new(),
        }
    }

    fn lex(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while self.index < self.source.len() {
            let start = self.index;
            match self.source[self.index] {
                ' ' | '\t' | '\r' => self.index += 1,
                '\n' => {
                    self.index += 1;
                    tokens.push(Token {
                        kind: TokenKind::Newline,
                        span: Span::new(start, self.index),
                    });
                }
                ';' => {
                    self.index += 1;
                    tokens.push(Token {
                        kind: TokenKind::Semicolon,
                        span: Span::new(start, self.index),
                    });
                }
                '/' if self.peek(1) == Some('/') => {
                    self.index += 2;
                    while self.index < self.source.len() && self.source[self.index] != '\n' {
                        self.index += 1;
                    }
                }
                '"' => tokens.push(self.string_token()),
                character if character.is_ascii_digit() => tokens.push(self.number_token()),
                character if character.is_ascii_alphabetic() || character == '_' => {
                    tokens.push(self.identifier_token())
                }
                '+' => tokens.push(self.single(TokenKind::Plus)),
                '-' => tokens.push(self.single(TokenKind::Minus)),
                '*' => tokens.push(self.single(TokenKind::Star)),
                '/' => tokens.push(self.single(TokenKind::Slash)),
                '%' => tokens.push(self.single(TokenKind::Percent)),
                '(' => tokens.push(self.single(TokenKind::LeftParen)),
                ')' => tokens.push(self.single(TokenKind::RightParen)),
                '[' => tokens.push(self.single(TokenKind::LeftBracket)),
                ']' => tokens.push(self.single(TokenKind::RightBracket)),
                '{' => tokens.push(self.single(TokenKind::LeftBrace)),
                '}' => tokens.push(self.single(TokenKind::RightBrace)),
                ',' => tokens.push(self.single(TokenKind::Comma)),
                '.' => tokens.push(self.single(TokenKind::Dot)),
                '!' if self.peek(1) == Some('=') => tokens.push(self.double(TokenKind::BangEqual)),
                '!' => tokens.push(self.single(TokenKind::Bang)),
                '=' if self.peek(1) == Some('=') => tokens.push(self.double(TokenKind::EqualEqual)),
                '=' if self.peek(1) == Some('>') => tokens.push(self.double(TokenKind::Arrow)),
                '=' => tokens.push(self.single(TokenKind::Equal)),
                ':' if self.peek(1) == Some('=') => tokens.push(self.double(TokenKind::ColonEqual)),
                ':' => tokens.push(self.single(TokenKind::Colon)),
                '<' if self.peek(1) == Some('=') => tokens.push(self.double(TokenKind::LessEqual)),
                '<' => tokens.push(self.single(TokenKind::Less)),
                '>' if self.peek(1) == Some('=') => {
                    tokens.push(self.double(TokenKind::GreaterEqual))
                }
                '>' => tokens.push(self.single(TokenKind::Greater)),
                '&' if self.peek(1) == Some('&') => tokens.push(self.double(TokenKind::AndAnd)),
                '|' if self.peek(1) == Some('|') => tokens.push(self.double(TokenKind::OrOr)),
                character => {
                    self.index += 1;
                    self.diagnostics.push(Diagnostic::error(
                        self.path.clone(),
                        Span::new(start, self.index),
                        format!("unexpected character {character}"),
                    ));
                }
            }
        }
        tokens.push(Token {
            kind: TokenKind::End,
            span: Span::new(self.index, self.index),
        });
        tokens
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.source.get(self.index + offset).copied()
    }

    fn single(&mut self, kind: TokenKind) -> Token {
        let start = self.index;
        self.index += 1;
        Token {
            kind,
            span: Span::new(start, self.index),
        }
    }

    fn double(&mut self, kind: TokenKind) -> Token {
        let start = self.index;
        self.index += 2;
        Token {
            kind,
            span: Span::new(start, self.index),
        }
    }

    fn number_token(&mut self) -> Token {
        let start = self.index;
        while self
            .peek(0)
            .is_some_and(|character| character.is_ascii_digit() || character == '.')
        {
            self.index += 1;
        }
        let text: String = self.source[start..self.index].iter().collect();
        match text.parse::<f64>() {
            Ok(value) => Token {
                kind: TokenKind::Number(value),
                span: Span::new(start, self.index),
            },
            Err(_) => {
                self.diagnostics.push(Diagnostic::error(
                    self.path.clone(),
                    Span::new(start, self.index),
                    "invalid number",
                ));
                Token {
                    kind: TokenKind::Number(0.0),
                    span: Span::new(start, self.index),
                }
            }
        }
    }

    fn identifier_token(&mut self) -> Token {
        let start = self.index;
        while self
            .peek(0)
            .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            self.index += 1;
        }
        let text: String = self.source[start..self.index].iter().collect();
        Token {
            kind: TokenKind::Identifier(text),
            span: Span::new(start, self.index),
        }
    }

    fn string_token(&mut self) -> Token {
        let start = self.index;
        self.index += 1;
        let mut value = String::new();
        let mut terminated = false;
        while self.index < self.source.len() {
            match self.source[self.index] {
                '"' => {
                    self.index += 1;
                    terminated = true;
                    break;
                }
                '\\' => {
                    self.index += 1;
                    if let Some(character) = self.source.get(self.index).copied() {
                        value.push(match character {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            other => other,
                        });
                        self.index += 1;
                    }
                }
                character => {
                    value.push(character);
                    self.index += 1;
                }
            }
        }
        if !terminated {
            self.diagnostics.push(Diagnostic::error(
                self.path.clone(),
                Span::new(start, self.index),
                "unterminated string",
            ));
        }
        Token {
            kind: TokenKind::String(value),
            span: Span::new(start, self.index),
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    path: PathBuf,
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

impl Parser {
    fn new(tokens: Vec<Token>, path: PathBuf) -> Self {
        Self {
            tokens,
            path,
            index: 0,
            diagnostics: Vec::new(),
        }
    }

    fn parse_program(&mut self) -> AstProgram {
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

    fn parse_language_directive(&mut self) {
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

    fn parse_enum(&mut self) -> Option<AstEnum> {
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

    fn parse_function(&mut self) -> Option<AstFunction> {
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

    fn parse_statement(&mut self) -> Option<AstStatement> {
        self.skip_boundaries();
        if self.consume_keyword("if") {
            return self.parse_if_statement();
        }
        if self.consume_keyword("while") {
            return self.parse_while_statement();
        }
        if self.consume_keyword("for") {
            return self.parse_for_statement();
        }
        if self.consume_keyword("break") {
            return Some(AstStatement::Break(self.previous_span()));
        }
        if self.consume_keyword("continue") {
            return Some(AstStatement::Continue(self.previous_span()));
        }
        if self.consume_keyword("return") {
            let span = self.previous_span();
            if matches!(
                self.current().kind,
                TokenKind::Newline | TokenKind::Semicolon | TokenKind::RightBrace | TokenKind::End
            ) {
                return Some(AstStatement::Return {
                    expression: None,
                    span,
                });
            }
            let expression = self.parse_expression(0)?;
            return Some(AstStatement::Return {
                span: Span::new(span.start, expression.span().end),
                expression: Some(expression),
            });
        }
        if self.check(&TokenKind::LeftParen) && self.looks_like_destructure() {
            return self.parse_destructure_statement();
        }
        let intrabar = self.consume_keyword("varip");
        let state = intrabar || self.consume_keyword("state") || self.consume_keyword("var");
        let Token {
            kind: TokenKind::Identifier(name),
            span,
        } = self.current().clone()
        else {
            if !self.at_end() {
                self.error(self.current().span, "expected a binding or call");
            }
            return None;
        };
        if self.peek_kind(1) == Some(&TokenKind::Equal)
            || self.peek_kind(1) == Some(&TokenKind::ColonEqual)
        {
            self.index += 2;
            let update = self.tokens[self.index - 1].kind == TokenKind::ColonEqual;
            let expression = self.parse_expression(0)?;
            return Some(AstStatement::Binding {
                name,
                expression,
                state,
                intrabar,
                update,
                span: Span::new(span.start, self.previous_span().end),
            });
        }
        if state {
            self.error(span, "state declarations require a name and expression");
            return None;
        }
        let call = self.parse_call()?;
        Some(AstStatement::Call { call })
    }

    fn looks_like_destructure(&self) -> bool {
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

    fn parse_destructure_statement(&mut self) -> Option<AstStatement> {
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

    fn parse_if_statement(&mut self) -> Option<AstStatement> {
        let start = self.tokens[self.index.saturating_sub(1)].span.start;
        let condition = self.parse_expression(0)?;
        let then_branch = self.parse_block_or_statement()?;
        self.skip_boundaries();
        let else_branch = if self.consume_keyword("else") {
            self.parse_block_or_statement()?
        } else {
            Vec::new()
        };
        let end = self.previous_span().end;
        Some(AstStatement::If {
            condition,
            then_branch,
            else_branch,
            span: Span::new(start, end),
        })
    }

    fn parse_while_statement(&mut self) -> Option<AstStatement> {
        let start = self.tokens[self.index.saturating_sub(1)].span.start;
        let condition = self.parse_expression(0)?;
        let body = self.parse_block_or_statement()?;
        let end = self.previous_span().end;
        Some(AstStatement::While {
            condition,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_for_statement(&mut self) -> Option<AstStatement> {
        let start = self.tokens[self.index.saturating_sub(1)].span.start;
        let TokenKind::Identifier(name) = self.current().kind.clone() else {
            self.error(self.current().span, "expected a loop variable");
            return None;
        };
        self.index += 1;
        if self.consume_keyword("in") {
            let collection = self.parse_expression(0)?;
            let body = self.parse_block_or_statement()?;
            return Some(AstStatement::ForEach {
                name,
                collection,
                span: Span::new(start, self.previous_span().end),
                body,
            });
        }
        if !self.consume_kind(TokenKind::Equal) {
            self.error(
                self.current().span,
                "loops require '=' for a range or 'in' for a collection",
            );
            return None;
        }
        let initial = self.parse_expression(0)?;
        if !self.consume_keyword("to") {
            self.error(
                self.current().span,
                "range loops require 'to' and an end expression",
            );
            return None;
        }
        let end = self.parse_expression(0)?;
        let body = self.parse_block_or_statement()?;
        Some(AstStatement::ForRange {
            name,
            start: initial,
            end,
            span: Span::new(start, self.previous_span().end),
            body,
        })
    }

    fn parse_block_or_statement(&mut self) -> Option<Vec<AstStatement>> {
        self.skip_boundaries();
        if self.consume_kind(TokenKind::LeftBrace) {
            let mut statements = Vec::new();
            self.skip_boundaries();
            while !self.check(&TokenKind::RightBrace) && !self.at_end() {
                if let Some(statement) = self.parse_statement() {
                    statements.push(statement);
                } else {
                    self.synchronize_statement();
                }
                self.skip_boundaries();
            }
            if !self.consume_kind(TokenKind::RightBrace) {
                self.error(self.current().span, "expected a closing brace");
            }
            Some(statements)
        } else {
            self.parse_statement().map(|statement| vec![statement])
        }
    }

    fn parse_call(&mut self) -> Option<AstCall> {
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

    fn parse_expression(&mut self, minimum_precedence: u8) -> Option<AstExpression> {
        let mut expression = self.parse_prefix()?;
        loop {
            if self.consume_kind(TokenKind::LeftBracket) {
                let index = self.parse_expression(0)?;
                let end = self.current().span;
                if !self.consume_kind(TokenKind::RightBracket) {
                    self.error(
                        self.current().span,
                        "expected a closing bracket after history index",
                    );
                }
                expression = AstExpression::Index {
                    span: Span::new(expression.span().start, end.end),
                    expression: Box::new(expression),
                    index: Box::new(index),
                };
                continue;
            }
            if self.consume_kind(TokenKind::Dot) {
                let Token {
                    kind: TokenKind::Identifier(field),
                    span,
                } = self.current().clone()
                else {
                    self.error(self.current().span, "expected a field name after a dot");
                    break;
                };
                self.index += 1;
                expression = AstExpression::Field {
                    span: Span::new(expression.span().start, span.end),
                    expression: Box::new(expression),
                    field,
                };
                continue;
            }
            let Some((operator, precedence)) = self.binary_operator() else {
                break;
            };
            if precedence < minimum_precedence {
                break;
            }
            self.index += 1;
            let right = self.parse_expression(precedence + 1)?;
            expression = AstExpression::Binary {
                span: Span::new(expression.span().start, right.span().end),
                left: Box::new(expression),
                operator,
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_prefix(&mut self) -> Option<AstExpression> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Number(value) => {
                self.index += 1;
                Some(AstExpression::Number(value, token.span))
            }
            TokenKind::String(value) => {
                self.index += 1;
                Some(AstExpression::String(value, token.span))
            }
            TokenKind::Identifier(name) => {
                self.index += 1;
                if name == "switch" && !self.check(&TokenKind::Dot) {
                    return self.parse_switch_expression(token.span.start);
                }
                let mut path = vec![name];
                while self.consume_kind(TokenKind::Dot) {
                    match self.current().kind.clone() {
                        TokenKind::Identifier(segment) => {
                            path.push(segment);
                            self.index += 1;
                        }
                        _ => {
                            self.error(self.current().span, "expected a name after a dot");
                            break;
                        }
                    }
                }
                if self.check(&TokenKind::LeftParen) {
                    let start = token.span.start;
                    self.index += 1;
                    let arguments = self.parse_call_arguments()?;
                    Some(AstExpression::Call(AstCall {
                        path,
                        arguments,
                        span: Span::new(start, self.previous_span().end),
                    }))
                } else {
                    let end = self.previous_span().end;
                    let value = match path.as_slice() {
                        [name] if name == "true" => AstExpression::Boolean(true, token.span),
                        [name] if name == "false" => AstExpression::Boolean(false, token.span),
                        [name] if name == "na" => AstExpression::Na(token.span),
                        _ => AstExpression::Identifier(path, Span::new(token.span.start, end)),
                    };
                    Some(value)
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
                let expression = self.parse_expression(7)?;
                Some(AstExpression::Unary {
                    span: Span::new(token.span.start, expression.span().end),
                    operator,
                    expression: Box::new(expression),
                })
            }
            TokenKind::LeftParen => {
                self.index += 1;
                self.skip_inline_boundaries();
                if self.consume_kind(TokenKind::RightParen) {
                    self.error(token.span, "empty parentheses are not an expression");
                    return None;
                }
                let first = self.parse_expression(0)?;
                let mut expressions = vec![first];
                self.skip_inline_boundaries();
                while self.consume_kind(TokenKind::Comma) {
                    self.skip_inline_boundaries();
                    expressions.push(self.parse_expression(0)?);
                    self.skip_inline_boundaries();
                }
                if !self.consume_kind(TokenKind::RightParen) {
                    self.error(self.current().span, "expected a closing parenthesis");
                }
                if expressions.len() == 1 {
                    Some(expressions.remove(0))
                } else {
                    Some(AstExpression::Tuple(
                        expressions,
                        Span::new(token.span.start, self.previous_span().end),
                    ))
                }
            }
            TokenKind::LeftBracket => {
                self.index += 1;
                let mut expressions = Vec::new();
                self.skip_inline_boundaries();
                while !self.check(&TokenKind::RightBracket) && !self.at_end() {
                    expressions.push(self.parse_expression(0)?);
                    self.skip_inline_boundaries();
                    if !self.consume_kind(TokenKind::Comma) {
                        break;
                    }
                    self.skip_inline_boundaries();
                }
                if !self.consume_kind(TokenKind::RightBracket) {
                    self.error(
                        self.current().span,
                        "expected a closing bracket after array literal",
                    );
                }
                Some(AstExpression::Array(
                    expressions,
                    Span::new(token.span.start, self.previous_span().end),
                ))
            }
            TokenKind::LeftBrace => {
                self.index += 1;
                self.parse_record_literal(token.span.start)
            }
            _ => {
                self.error(token.span, "expected an expression");
                None
            }
        }
    }

    fn parse_record_literal(&mut self, start: usize) -> Option<AstExpression> {
        let mut fields = Vec::new();
        self.skip_boundaries();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let TokenKind::Identifier(name) = self.current().kind.clone() else {
                self.error(self.current().span, "record fields require a name");
                return None;
            };
            self.index += 1;
            if !self.consume_kind(TokenKind::Colon) {
                self.error(self.current().span, "record fields require ':'");
                return None;
            }
            let expression = self.parse_expression(0)?;
            fields.push((name, expression));
            self.skip_inline_boundaries();
            if !self.consume_kind(TokenKind::Comma) {
                self.skip_boundaries();
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
                self.error(
                    self.current().span,
                    "expected a comma between record fields",
                );
                return None;
            }
            self.skip_boundaries();
        }
        if !self.consume_kind(TokenKind::RightBrace) {
            self.error(
                self.current().span,
                "expected a closing brace after record fields",
            );
            return None;
        }
        Some(AstExpression::Record(
            fields,
            Span::new(start, self.previous_span().end),
        ))
    }

    fn parse_switch_expression(&mut self, start: usize) -> Option<AstExpression> {
        let operand = self.parse_expression(0)?;
        self.skip_boundaries();
        if !self.consume_kind(TokenKind::LeftBrace) {
            self.error(self.current().span, "switch requires a branch block");
            return None;
        }
        let mut branches = Vec::new();
        let mut fallback = None;
        self.skip_boundaries();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let is_default = self.is_named("default");
            let condition = if is_default {
                self.index += 1;
                None
            } else {
                Some(self.parse_expression(0)?)
            };
            if !self.consume_kind(TokenKind::Arrow) {
                self.error(self.current().span, "switch branches require '=>' ");
                return None;
            }
            let result = self.parse_expression(0)?;
            if let Some(condition) = condition {
                branches.push((condition, result));
            } else {
                fallback = Some(Box::new(result));
            }
            self.skip_inline_boundaries();
            if !self.consume_kind(TokenKind::Comma) {
                self.skip_boundaries();
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
                self.error(
                    self.current().span,
                    "expected a comma between switch branches",
                );
                return None;
            }
            self.skip_boundaries();
        }
        if !self.consume_kind(TokenKind::RightBrace) {
            self.error(
                self.current().span,
                "expected a closing brace after switch branches",
            );
            return None;
        }
        Some(AstExpression::Switch {
            operand: Box::new(operand),
            branches,
            fallback,
            span: Span::new(start, self.previous_span().end),
        })
    }

    fn parse_call_arguments(&mut self) -> Option<Vec<AstArgument>> {
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

    fn parse_path(&mut self) -> Option<Vec<String>> {
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

    fn binary_operator(&self) -> Option<(BinaryOperator, u8)> {
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

    fn synchronize_statement(&mut self) {
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

    fn skip_boundaries(&mut self) {
        while matches!(
            self.current().kind,
            TokenKind::Newline | TokenKind::Semicolon
        ) {
            self.index += 1;
        }
    }

    fn skip_inline_boundaries(&mut self) {
        while matches!(self.current().kind, TokenKind::Newline) {
            self.index += 1;
        }
    }

    fn is_named(&self, name: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Identifier(value) if value == name)
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        if self.is_named(keyword) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn consume_kind(&mut self, kind: TokenKind) -> bool {
        if self.current().kind == kind {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        &self.current().kind == kind
    }

    fn peek_kind(&self, offset: usize) -> Option<&TokenKind> {
        self.tokens
            .get(self.index + offset)
            .map(|token| &token.kind)
    }

    fn previous_span(&self) -> Span {
        self.tokens
            .get(self.index.saturating_sub(1))
            .map_or(self.current().span, |token| token.span)
    }

    fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len() - 1)]
    }

    fn at_end(&self) -> bool {
        matches!(self.current().kind, TokenKind::End)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(self.path.clone(), span, message));
    }
}

struct Compiler {
    path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    registry: IntrinsicRegistry,
    program: HirProgram,
    descriptor: Option<IndicatorDescriptor>,
    presentation: IndicatorPresentation,
    values: BTreeMap<String, (LocalSlot, Type)>,
    states: BTreeMap<String, (StateSlot, Type)>,
    inputs: BTreeMap<String, InputDescriptor>,
    input_spans: BTreeMap<String, Span>,
    local_definitions: Vec<Option<ExprId>>,
    mutable_values: BTreeSet<usize>,
    next_local: usize,
    next_state: usize,
    state_call_sites: Vec<CallSiteId>,
    state_functions: Vec<Option<FunctionId>>,
    next_series: usize,
    next_call_site: usize,
    requirements: DataRequirements,
    max_bars_back_explicit: bool,
    loop_depth: usize,
    functions: BTreeMap<String, FunctionId>,
    function_signatures: BTreeMap<FunctionId, (Vec<String>, usize)>,
    current_function: Option<FunctionId>,
    active_functions: Vec<FunctionId>,
    call_edges: BTreeMap<FunctionId, BTreeSet<FunctionId>>,
    statement_scope_depth: usize,
    presentation_spans: PresentationSpans,
}

/// Source locations for compiler-owned presentation declarations.  The
/// typed scene intentionally stays host-neutral and does not carry parser
/// spans, so validation keeps this small side table until compilation is
/// complete.  Diagnostics can consequently point at the declaration that
/// introduced an invalid output, style, or reference.
#[derive(Debug, Default)]
struct PresentationSpans {
    indicator: Option<Span>,
    plots: BTreeMap<String, Span>,
    styles: BTreeMap<String, Span>,
    marks: Vec<Span>,
    segments: BTreeMap<String, Span>,
    fills: Vec<Span>,
    readouts: Vec<Span>,
    columns: Vec<Span>,
}

impl Compiler {
    fn new(path: PathBuf, registry: IntrinsicRegistry) -> Self {
        Self {
            path,
            diagnostics: Vec::new(),
            registry,
            program: HirProgram::empty(),
            descriptor: None,
            presentation: IndicatorPresentation::default(),
            values: BTreeMap::new(),
            states: BTreeMap::new(),
            inputs: BTreeMap::new(),
            input_spans: BTreeMap::new(),
            local_definitions: Vec::new(),
            mutable_values: BTreeSet::new(),
            next_local: 0,
            next_state: 0,
            state_call_sites: Vec::new(),
            state_functions: Vec::new(),
            next_series: 0,
            next_call_site: 0,
            requirements: DataRequirements::default(),
            max_bars_back_explicit: false,
            loop_depth: 0,
            functions: BTreeMap::new(),
            function_signatures: BTreeMap::new(),
            current_function: None,
            active_functions: Vec::new(),
            call_edges: BTreeMap::new(),
            statement_scope_depth: 0,
            presentation_spans: PresentationSpans::default(),
        }
    }

    fn compile_program(
        &mut self,
        ast: AstProgram,
    ) -> (IndicatorDescriptor, HirProgram, DataRequirements) {
        let descriptor = self.compile_descriptor(ast.indicator.as_ref());
        self.presentation = descriptor.presentation.clone();
        self.descriptor = Some(descriptor);
        for item in ast.enums {
            self.error(
                item.span,
                "enum declarations are not available; use strings or integers",
            );
        }
        for function in &ast.functions {
            if self.functions.contains_key(&function.name) {
                self.error(
                    function.span,
                    format!("function {} is already declared", function.name),
                );
                continue;
            }
            let id = FunctionId(self.program.functions.len());
            self.functions.insert(function.name.clone(), id);
            self.function_signatures.insert(
                id,
                (
                    function
                        .parameters
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                    function
                        .parameters
                        .iter()
                        .filter(|parameter| parameter.default.is_none())
                        .count(),
                ),
            );
            self.program.functions.push(HirFunction {
                id,
                name: function.name.clone(),
                parameters: Vec::new(),
                parameter_names: Vec::new(),
                parameter_defaults: Vec::new(),
                local_slots: Vec::new(),
                body: Vec::new(),
                return_type: Type::series(ValueType::Float),
            });
        }
        for statement in ast.statements {
            if let Some(statement) = self.compile_statement(statement) {
                self.program.blocks[0].statements.push(statement);
            }
        }
        for function in ast.functions {
            self.compile_function(function);
        }
        self.reject_recursive_functions();
        let mut descriptor = self.descriptor.take().expect("descriptor is initialized");
        if descriptor.max_bars_back > 50_000 {
            self.error(
                Span::new(0, 0),
                "history requirements cannot exceed the host maximum of 50000 bars",
            );
            descriptor.max_bars_back = 50_000;
        }
        descriptor.inputs = self.inputs.values().cloned().collect();
        self.validate_inputs(&descriptor);
        self.ensure_default_plot_styles();
        let indicator_overlay =
            matches!(self.presentation.view, PresentationViewKind::PrimaryOverlay);
        self.ensure_default_presentation_layers(indicator_overlay);
        descriptor.plots = self.program.plots.clone();
        descriptor.presentation = self.presentation.clone();
        self.validate_presentation(&descriptor);
        self.program.locals = self.next_local;
        self.program.states = self.next_state;
        self.program.state_call_sites = self.state_call_sites.clone();
        self.program.state_functions = self.state_functions.clone();
        self.program.series = self.next_series;
        self.program.call_sites = self.next_call_site;
        self.program.local_definitions = self.local_definitions.clone();
        (descriptor, self.program.clone(), self.requirements)
    }

    fn ensure_default_plot_styles(&mut self) {
        let existing = self
            .presentation
            .styles
            .iter()
            .map(|style| style.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut generated = Vec::new();
        for plot in &mut self.program.plots {
            if !matches!(plot.value_type, PlotValueType::Number) {
                continue;
            }
            // An omitted slot receives the deterministic plot-ID style. An
            // explicitly named slot is a source contract and must already
            // be declared with `style`; silently inventing it would hide a
            // typo and defeat missing-style diagnostics.
            let generated_slot = plot.color_slot.is_none();
            let slot = plot
                .color_slot
                .get_or_insert_with(|| plot.id.clone())
                .clone();
            if generated_slot
                && !existing.contains(slot.as_str())
                && !generated
                    .iter()
                    .any(|style: &PresentationStyle| style.id == slot)
            {
                generated.push(PresentationStyle {
                    id: slot,
                    label: plot.label.clone(),
                    role: "neutral".to_owned(),
                    shade: "base".to_owned(),
                });
            }
        }
        // Table columns intentionally default to the neutral `main` slot so a
        // small table indicator does not need boilerplate style metadata. An
        // explicitly named column slot still has to be declared by the source
        // and is validated below.
        if self.presentation.columns.iter().any(|column| {
            matches!(
                &column.color,
                PresentationColumnColor::Static { slot } if slot == "main"
            )
        }) && !existing.contains("main")
            && !generated
                .iter()
                .any(|style: &PresentationStyle| style.id == "main")
        {
            generated.push(PresentationStyle {
                id: "main".to_owned(),
                label: "Main".to_owned(),
                role: "neutral".to_owned(),
                shade: "base".to_owned(),
            });
        }
        self.presentation.styles.extend(generated);
    }

    /// Materialize the scene implied by numeric plots once, while the source
    /// is being compiled.  The chart host can then map these typed layers
    /// directly without repeating inference or retaining a second set of
    /// presentation defaults.
    fn ensure_default_presentation_layers(&mut self, indicator_overlay: bool) {
        if matches!(self.presentation.view, PresentationViewKind::Table) {
            return;
        }
        let existing = self
            .presentation
            .layers
            .iter()
            .map(|layer| layer.channel.as_str())
            .collect::<BTreeSet<_>>();
        let generated = self
            .program
            .plots
            .iter()
            .filter(|plot| {
                plot.layer
                    && !matches!(plot.value_type, PlotValueType::Boolean)
                    && !existing.contains(plot.id.as_str())
            })
            .map(|plot| PresentationLayer {
                channel: plot.id.clone(),
                name: plot
                    .layer_title
                    .clone()
                    .unwrap_or_else(|| plot.label.clone()),
                data_kind: Some(
                    plot.presentation_data_kind
                        .unwrap_or_else(|| default_data_kind_for_plot(plot)),
                ),
                mark: Some(
                    plot.presentation_mark
                        .unwrap_or_else(|| default_mark_for_plot(plot)),
                ),
                axis: Some(
                    plot.presentation_axis
                        .unwrap_or_else(|| default_axis_for_plot(indicator_overlay, plot)),
                ),
            })
            .collect::<Vec<_>>();
        self.presentation.layers.extend(generated);
    }

    fn validate_inputs(&mut self, descriptor: &IndicatorDescriptor) {
        for input in &descriptor.inputs {
            let span = self
                .input_spans
                .get(&input.id)
                .copied()
                .unwrap_or_else(|| Span::new(0, 0));
            if let (Some(min), Some(max)) = (input.min, input.max)
                && min > max
            {
                self.error(
                    span,
                    format!("input '{}' min must not exceed max", input.id),
                );
            }
            if let Some(step) = input.step
                && (!step.is_finite() || step <= 0.0)
            {
                self.error(span, format!("input '{}' step must be positive", input.id));
            }
            let default = match &input.default {
                InputValue::Int(value) => Some(*value as f64),
                InputValue::Float(value) => Some(*value),
                _ => None,
            };
            if let Some(default) = default {
                if input.min.is_some_and(|min| default < min) {
                    self.error(
                        span,
                        format!("input '{}' default is below its minimum", input.id),
                    );
                }
                if input.max.is_some_and(|max| default > max) {
                    self.error(
                        span,
                        format!("input '{}' default is above its maximum", input.id),
                    );
                }
            }
            if !input.options.is_empty()
                && let InputValue::String(default) = &input.default
                && !input.options.iter().any(|option| option.id == *default)
            {
                self.error(
                    span,
                    format!("input '{}' default is not one of its options", input.id),
                );
            }
        }
    }

    fn validate_presentation(&mut self, descriptor: &IndicatorDescriptor) {
        let output_types = descriptor
            .plots
            .iter()
            .map(|plot| (plot.id.as_str(), plot.value_type))
            .collect::<BTreeMap<_, _>>();
        let expected_pane = if matches!(
            descriptor.presentation.view,
            PresentationViewKind::PrimaryOverlay
        ) {
            PaneTarget::Main
        } else {
            PaneTarget::Indicator
        };
        let has_numeric_output = descriptor
            .plots
            .iter()
            .any(|plot| plot.value_type != PlotValueType::Boolean);
        if matches!(descriptor.presentation.view, PresentationViewKind::Plot)
            && has_numeric_output
            && descriptor.presentation.value_precision.is_none()
        {
            self.error(
                self.indicator_span(),
                "numeric plot indicators must declare value_precision for panel axes and crosshair snapping",
            );
        }
        for plot in &descriptor.plots {
            if plot.pane != expected_pane {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' uses pane {:?}, but this indicator's default pane is {:?}",
                        plot.id, plot.pane, expected_pane
                    ),
                );
            }
        }
        let style_ids = descriptor
            .presentation
            .styles
            .iter()
            .map(|style| style.id.as_str())
            .collect::<BTreeSet<_>>();
        for (index, plot) in descriptor.plots.iter().enumerate() {
            if plot.id.trim().is_empty() {
                self.error(self.plot_span(&plot.id), "plot ID cannot be empty");
            }
            if plot.label.trim().is_empty() {
                self.error(self.plot_span(&plot.id), "plot title cannot be empty");
            }
            if descriptor.plots[..index]
                .iter()
                .any(|previous| previous.id == plot.id)
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!("duplicate plot ID '{}'; output IDs must be unique", plot.id),
                );
            }
            for slot in [
                plot.color_slot.as_deref(),
                plot.true_color_slot.as_deref(),
                plot.false_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.plot_span(&plot.id),
                        format!("plot '{}' references unknown style slot '{slot}'", plot.id),
                    );
                }
            }
            if let Some(style_by) = &plot.style_by
                && output_types.get(style_by.as_str()) != Some(&PlotValueType::Boolean)
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' style_by must reference a boolean output",
                        plot.id
                    ),
                );
            }
            let data_kind = plot
                .presentation_data_kind
                .unwrap_or_else(|| default_data_kind_for_plot(plot));
            if !plot_value_supports_data_kind(plot.value_type, data_kind) {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' value type is incompatible with presentation data kind",
                        plot.id
                    ),
                );
            }
            let mark = plot
                .presentation_mark
                .unwrap_or_else(|| default_mark_for_plot(plot));
            if !presentation_mark_compatible(data_kind, mark) {
                self.error(
                    self.plot_span(&plot.id),
                    format!("plot '{}' mark is incompatible with its data kind", plot.id),
                );
            }
            if plot.value_type != PlotValueType::Number && plot.style_by.is_some() {
                self.error(
                    self.plot_span(&plot.id),
                    format!("plot '{}' style_by requires a numeric output", plot.id),
                );
            }
            if plot.true_color_slot.is_some() != plot.false_color_slot.is_some() {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' must provide both true_color_slot and false_color_slot",
                        plot.id
                    ),
                );
            }
            if plot.style_by.is_some()
                && (plot.true_color_slot.is_none() || plot.false_color_slot.is_none())
            {
                self.error(
                    self.plot_span(&plot.id),
                    format!(
                        "plot '{}' style_by requires both true_color_slot and false_color_slot",
                        plot.id
                    ),
                );
            }
        }
        for (index, style) in descriptor.presentation.styles.iter().enumerate() {
            if style.id.trim().is_empty() {
                self.error(self.style_span(&style.id), "style ID cannot be empty");
            }
            if !valid_presentation_role(&style.role) {
                self.error(
                    self.style_span(&style.id),
                    format!("unknown style role '{}'", style.role),
                );
            }
            if !valid_presentation_shade(&style.shade) {
                self.error(
                    self.style_span(&style.id),
                    format!("unknown style shade '{}'", style.shade),
                );
            }
            if descriptor.presentation.styles[..index]
                .iter()
                .any(|previous| previous.id == style.id)
            {
                self.error(
                    self.style_span(&style.id),
                    format!("duplicate style ID '{}'", style.id),
                );
            }
        }
        for (layer_index, layer) in descriptor.presentation.layers.iter().enumerate() {
            if layer.channel.trim().is_empty() {
                self.error(
                    self.plot_span(&layer.channel),
                    "layer output ID cannot be empty",
                );
            }
            if layer.name.trim().is_empty() {
                self.error(
                    self.plot_span(&layer.channel),
                    "layer title cannot be empty",
                );
            }
            if descriptor.presentation.layers[..layer_index]
                .iter()
                .any(|previous| previous.channel == layer.channel)
            {
                self.error(
                    self.plot_span(&layer.channel),
                    format!(
                        "duplicate presentation layer for output '{}'",
                        layer.channel
                    ),
                );
            }
            if !output_types.contains_key(layer.channel.as_str()) {
                self.error(
                    self.plot_span(&layer.channel),
                    format!("layer references unknown output '{}'", layer.channel),
                );
                continue;
            }
            if let Some(plot) = descriptor
                .plots
                .iter()
                .find(|plot| plot.id == layer.channel)
            {
                if plot.value_type == PlotValueType::Boolean {
                    self.error(
                        self.plot_span(&layer.channel),
                        format!(
                            "layer '{}' cannot reference a boolean output; use a table column or output-only plot",
                            layer.channel
                        ),
                    );
                    continue;
                }
                let data_kind = layer.data_kind.unwrap_or_else(|| {
                    plot.presentation_data_kind
                        .unwrap_or_else(|| default_data_kind_for_plot(plot))
                });
                let mark = layer.mark.unwrap_or_else(|| {
                    plot.presentation_mark
                        .unwrap_or_else(|| default_mark_for_plot(plot))
                });
                if !plot_value_supports_data_kind(plot.value_type, data_kind)
                    || !presentation_mark_compatible(data_kind, mark)
                {
                    self.error(
                        self.plot_span(&layer.channel),
                        format!("layer '{}' is incompatible with its output", layer.channel),
                    );
                }
            }
        }
        let mut mark_keys = BTreeSet::new();
        for (mark_index, mark) in descriptor.presentation.marks.iter().enumerate() {
            if !mark_keys.insert((mark.source_channel.clone(), mark.kind)) {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "duplicate mark for output '{}' and kind {:?}",
                        mark.source_channel, mark.kind
                    ),
                );
            }
            let Some(source_plot) = descriptor
                .plots
                .iter()
                .find(|plot| plot.id == mark.source_channel)
            else {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' references unknown output '{}'",
                        mark.label, mark.source_channel
                    ),
                );
                continue;
            };
            if source_plot.value_type != PlotValueType::Number {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' source must reference a numeric output",
                        mark.label
                    ),
                );
            } else if !numeric_mark_compatible(mark.kind) {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' uses a rendering incompatible with a numeric output",
                        mark.label
                    ),
                );
            }
            if mark.label.trim().is_empty() {
                self.error(self.mark_span(mark_index), "mark title cannot be empty");
            }
            for slot in [
                Some(mark.color_slot.as_str()),
                mark.true_color_slot.as_deref(),
                mark.false_color_slot.as_deref(),
                mark.highlight_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' references unknown style slot '{slot}'",
                            mark.label
                        ),
                    );
                }
            }
            if mark.color_slot.trim().is_empty() {
                self.error(
                    self.mark_span(mark_index),
                    format!("mark '{}' requires a color_slot", mark.label),
                );
            }
            if mark.true_color_slot.is_some() != mark.false_color_slot.is_some() {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' must provide both true_color_slot and false_color_slot",
                        mark.label
                    ),
                );
            }
            if mark.highlight_by.is_some() != mark.highlight_color_slot.is_some() {
                self.error(
                    self.mark_span(mark_index),
                    format!(
                        "mark '{}' highlight_by requires highlight_color_slot",
                        mark.label
                    ),
                );
            }
            if let Some(highlight_by) = &mark.highlight_by {
                if output_types.get(highlight_by.as_str()) != Some(&PlotValueType::Boolean) {
                    self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' highlight_by must reference a boolean output",
                            mark.label
                        ),
                    );
                }
                if !matches!(
                    mark.kind,
                    PresentationMarkKind::RegularBar
                        | PresentationMarkKind::Histogram
                        | PresentationMarkKind::HistogramSignedOverlay
                ) {
                    self.error(
                        self.mark_span(mark_index),
                        format!("mark '{}' highlight_by requires a bar mark", mark.label),
                    );
                }
            }

            if let Some(fallback_mark) = mark.fallback_mark {
                if fallback_mark == mark.kind {
                    self.error(
                        self.mark_span(mark_index),
                        format!("mark '{}' cannot fall back to itself", mark.label),
                    );
                }

                let fallback_candidates = descriptor
                    .presentation
                    .marks
                    .iter()
                    .filter(|candidate| candidate.kind == fallback_mark)
                    .collect::<Vec<_>>();
                match fallback_candidates.as_slice() {
                    [] => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' references unknown fallback mark {:?}",
                            mark.label, fallback_mark
                        ),
                    ),
                    [fallback] if fallback.required_input.is_some() => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' fallback mark {:?} cannot require input",
                            mark.label, fallback_mark
                        ),
                    ),
                    [_] => {}
                    _ => self.error(
                        self.mark_span(mark_index),
                        format!(
                            "mark '{}' fallback mark {:?} is ambiguous",
                            mark.label, fallback_mark
                        ),
                    ),
                }
            }
        }
        for (index, segment) in descriptor.presentation.segments.iter().enumerate() {
            if segment.id.trim().is_empty() {
                self.error(self.segment_span(&segment.id), "segment ID cannot be empty");
            }
            if segment.label.trim().is_empty() {
                self.error(
                    self.segment_span(&segment.id),
                    "segment title cannot be empty",
                );
            }
            if !segment.line_width.is_finite() || segment.line_width <= 0.0 {
                self.error(
                    self.segment_span(&segment.id),
                    "segment width must be positive",
                );
            }
            if descriptor.presentation.segments[..index]
                .iter()
                .any(|previous| previous.id == segment.id)
            {
                self.error(
                    self.segment_span(&segment.id),
                    format!("duplicate segment ID '{}'", segment.id),
                );
            }
            if !style_ids.contains(segment.color_slot.as_str()) {
                self.error(
                    self.segment_span(&segment.id),
                    format!(
                        "segment '{}' references unknown style slot '{}'",
                        segment.id, segment.color_slot
                    ),
                );
            }
        }
        let mut fill_keys = BTreeSet::new();
        for (fill_index, fill) in descriptor.presentation.fills.iter().enumerate() {
            if fill.upper == fill.lower {
                self.error(
                    self.fill_span(fill_index),
                    format!(
                        "fill upper and lower outputs must differ; '{}' cannot reference itself",
                        fill.upper
                    ),
                );
            }
            if !fill_keys.insert((
                fill.upper.clone(),
                fill.lower.clone(),
                fill.geometry,
                fill.upper_offset_bars,
                fill.lower_offset_bars,
            )) {
                self.error(
                    self.fill_span(fill_index),
                    format!(
                        "duplicate fill for outputs '{}' and '{}'",
                        fill.upper, fill.lower
                    ),
                );
            }
            for channel in [&fill.upper, &fill.lower] {
                if output_types.get(channel.as_str()) != Some(&PlotValueType::Number) {
                    self.error(
                        self.fill_span(fill_index),
                        format!("fill references unknown or non-numeric output '{channel}'"),
                    );
                }
            }
            if !fill.opacity.is_finite() || !(0.0..=1.0).contains(&fill.opacity) {
                self.error(
                    self.fill_span(fill_index),
                    "fill opacity must be between 0 and 1",
                );
            }
            for slot in [
                Some(fill.color_slot.as_str()),
                fill.true_color_slot.as_deref(),
                fill.false_color_slot.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !style_ids.contains(slot) {
                    self.error(
                        self.fill_span(fill_index),
                        format!("fill references unknown style slot '{slot}'"),
                    );
                }
            }
            if fill.true_color_slot.is_some() != fill.false_color_slot.is_some() {
                self.error(
                    self.fill_span(fill_index),
                    "fill must provide both true_color_slot and false_color_slot",
                );
            }
            if let Some(style_by) = &fill.style_by
                && output_types.get(style_by.as_str()) != Some(&PlotValueType::Boolean)
            {
                self.error(
                    self.fill_span(fill_index),
                    "fill style_by must reference a boolean output",
                );
            }
            if fill.style_by.is_some()
                && (fill.true_color_slot.is_none() || fill.false_color_slot.is_none())
            {
                self.error(
                    self.fill_span(fill_index),
                    "fill style_by requires both true_color_slot and false_color_slot",
                );
            }
        }
        let mut readout_keys = BTreeSet::new();
        for (readout_index, readout) in descriptor.presentation.readouts.iter().enumerate() {
            if !readout_keys.insert((readout.channel.clone(), readout.mark)) {
                self.error(
                    self.readout_span(readout_index),
                    format!("duplicate readout for output '{}'", readout.channel),
                );
            }
            if readout.label.trim().is_empty() {
                self.error(
                    self.readout_span(readout_index),
                    "readout title cannot be empty",
                );
            }
            if output_types.get(readout.channel.as_str()) != Some(&PlotValueType::Number) {
                self.error(
                    self.readout_span(readout_index),
                    format!(
                        "readout references unknown or non-numeric output '{}'",
                        readout.channel
                    ),
                );
            }
            if !style_ids.contains(readout.color_slot.as_str()) {
                self.error(
                    self.readout_span(readout_index),
                    format!(
                        "readout references unknown style slot '{}'",
                        readout.color_slot
                    ),
                );
            }
            if let Some(mark) = readout.mark
                && !numeric_mark_compatible(mark)
            {
                self.error(
                    self.readout_span(readout_index),
                    "readout mark is incompatible with a numeric output",
                );
            }
        }
        let mut column_channels = BTreeSet::new();
        for (column_index, column) in descriptor.presentation.columns.iter().enumerate() {
            if !column_channels.insert(column.channel.clone()) {
                self.error(
                    self.column_span(column_index),
                    format!("duplicate table column for output '{}'", column.channel),
                );
            }
            if column.label.trim().is_empty() {
                self.error(
                    self.column_span(column_index),
                    "table column title cannot be empty",
                );
            }
            let Some(value_type) = output_types.get(column.channel.as_str()).copied() else {
                self.error(
                    self.column_span(column_index),
                    format!("column references unknown output '{}'", column.channel),
                );
                continue;
            };
            let format = column.format.to_ascii_lowercase();
            let compatible = match format.as_str() {
                "boolean" => value_type == PlotValueType::Boolean,
                "quantity" | "integer" | "percent" => value_type == PlotValueType::Number,
                _ => {
                    self.error(
                        self.column_span(column_index),
                        format!("unknown table column format '{}'", column.format),
                    );
                    false
                }
            };
            if !compatible {
                self.error(
                    self.column_span(column_index),
                    format!("table column '{}' has an incompatible output", column.label),
                );
            }
            let validate_column_style_slot = |slot: &str, compiler: &mut Self| {
                if !style_ids.contains(slot) {
                    compiler.error(
                        compiler.column_span(column_index),
                        format!("column references unknown style slot '{slot}'"),
                    );
                }
            };
            match &column.color {
                PresentationColumnColor::Static { slot } => {
                    validate_column_style_slot(slot, self);
                }
                PresentationColumnColor::Signed {
                    positive_slot,
                    negative_slot,
                } => {
                    validate_column_style_slot(positive_slot, self);
                    validate_column_style_slot(negative_slot, self);
                    if value_type == PlotValueType::Boolean {
                        self.error(
                            self.column_span(column_index),
                            "signed table column colors require a numeric output",
                        );
                    }
                }
            }
        }
        if !descriptor.presentation.columns.is_empty()
            && !matches!(descriptor.presentation.view, PresentationViewKind::Table)
        {
            self.error(
                self.column_span(0),
                "table column declarations require indicator(view = \"table\")",
            );
        }
        if matches!(descriptor.presentation.view, PresentationViewKind::Table)
            && descriptor.presentation.columns.is_empty()
        {
            self.error(
                descriptor
                    .plots
                    .first()
                    .map_or_else(|| self.indicator_span(), |plot| self.plot_span(&plot.id)),
                "indicator(view = \"table\") requires at least one column declaration",
            );
        }
        if matches!(descriptor.presentation.view, PresentationViewKind::Plot)
            && descriptor.presentation.layers.is_empty()
            && descriptor
                .plots
                .iter()
                .any(|plot| plot.value_type != PlotValueType::Boolean)
        {
            self.error(
                descriptor
                    .plots
                    .first()
                    .map_or_else(|| self.indicator_span(), |plot| self.plot_span(&plot.id)),
                "indicator presentation could not derive any output layers",
            );
        }
        if matches!(
            descriptor.presentation.view,
            PresentationViewKind::PrimaryOverlay
        ) && descriptor.presentation.layers.is_empty()
        {
            self.error(
                self.indicator_span(),
                "primary_overlay view requires at least one numeric output layer",
            );
        }
        if let (Some(data_kind), Some(mark)) = (
            descriptor.presentation.data_kind,
            descriptor.presentation.mark,
        ) && !presentation_mark_compatible(data_kind, mark)
        {
            let span = descriptor
                .plots
                .iter()
                .find(|plot| plot.value_type != PlotValueType::Boolean)
                .map_or_else(|| self.indicator_span(), |plot| self.plot_span(&plot.id));
            self.error(span, "indicator mark is incompatible with its data kind");
        }
    }

    fn plot_span(&self, id: &str) -> Span {
        self.presentation_spans
            .plots
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn indicator_span(&self) -> Span {
        self.presentation_spans
            .indicator
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn style_span(&self, id: &str) -> Span {
        self.presentation_spans
            .styles
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn mark_span(&self, index: usize) -> Span {
        self.presentation_spans
            .marks
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn segment_span(&self, id: &str) -> Span {
        self.presentation_spans
            .segments
            .get(id)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn fill_span(&self, index: usize) -> Span {
        self.presentation_spans
            .fills
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn readout_span(&self, index: usize) -> Span {
        self.presentation_spans
            .readouts
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn column_span(&self, index: usize) -> Span {
        self.presentation_spans
            .columns
            .get(index)
            .copied()
            .unwrap_or_else(|| Span::new(0, 0))
    }

    fn compile_function(&mut self, function: AstFunction) {
        let Some(function_id) = self.functions.get(&function.name).copied() else {
            return;
        };
        let saved_values = self.values.clone();
        let saved_states = self.states.clone();
        let saved_function = self.current_function;
        let saved_loop_depth = self.loop_depth;
        self.current_function = Some(function_id);
        self.active_functions.push(function_id);
        self.loop_depth = 0;
        let local_start = self.next_local;
        let mut parameter_slots = Vec::new();
        let mut parameter_names = Vec::new();
        let mut parameter_defaults = Vec::new();
        let parameter_specs = function.parameters;
        for parameter in &parameter_specs {
            let slot = LocalSlot(self.next_local);
            self.next_local += 1;
            self.local_definitions.push(None);
            parameter_slots.push(slot);
            self.values.insert(
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
        let local_slots = (local_start..self.next_local).map(LocalSlot).collect();
        if let Some(compiled) = self.program.functions.get_mut(function_id.0) {
            compiled.parameters = parameter_slots;
            compiled.parameter_names = parameter_names;
            compiled.parameter_defaults = parameter_defaults;
            compiled.local_slots = local_slots;
            compiled.body = body;
            compiled.return_type = Type::series(ValueType::Float);
        }
        self.values = saved_values;
        self.states = saved_states;
        self.current_function = saved_function;
        self.active_functions.pop();
        self.loop_depth = saved_loop_depth;
    }

    fn reject_recursive_functions(&mut self) {
        let function_ids = self
            .program
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        for function in function_ids {
            let mut path = Vec::new();
            let mut visited = BTreeSet::new();
            if self.function_reaches(function, function, &mut path, &mut visited) {
                self.error(
                    Span::new(0, 0),
                    format!(
                        "recursive function cycle involving {} is not supported in language version 1",
                        self.program
                            .functions
                            .get(function.0)
                            .map(|function| function.name.as_str())
                            .unwrap_or("function")
                    ),
                );
            }
        }
    }

    fn function_reaches(
        &self,
        start: FunctionId,
        current: FunctionId,
        path: &mut Vec<FunctionId>,
        visited: &mut BTreeSet<FunctionId>,
    ) -> bool {
        if !visited.insert(current) {
            return false;
        }
        path.push(current);
        let reaches = self.call_edges.get(&current).is_some_and(|edges| {
            edges.iter().any(|next| {
                (path.len() > 1 && *next == start)
                    || self.function_reaches(start, *next, path, visited)
            })
        });
        path.pop();
        reaches
    }

    fn compile_descriptor(&mut self, call: Option<&AstCall>) -> IndicatorDescriptor {
        self.presentation_spans.indicator = call.map(|call| call.span);
        let mut title = self
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("indicator")
            .to_owned();
        let mut id = format!("local.{title}");
        let mut optional_trades = false;
        let mut max_bars_back = 5_000;
        if let Some(call) = call {
            self.validate_named_arguments(
                call,
                &[
                    "title",
                    "id",
                    "optional_trades",
                    "max_bars_back",
                    "view",
                    "panel_title",
                    "layer_name",
                    "data_kind",
                    "mark",
                    "axis",
                    "value_precision",
                    "axis_label_mode",
                    "header_label_mode",
                    "preferred_scale",
                    "max_decimals",
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
            if let Some(value) = self.static_named_value(call, "max_bars_back") {
                if let Value::Int(value) = value
                    && value > 0
                {
                    max_bars_back = value as usize;
                    self.max_bars_back_explicit = true;
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
        let mut presentation = IndicatorPresentation::default();
        if let Some(view) = call.and_then(|call| self.static_named_string(call, "view")) {
            presentation.view = match view.to_ascii_lowercase().as_str() {
                "plot" => PresentationViewKind::Plot,
                "table" => PresentationViewKind::Table,
                "primary_overlay" => PresentationViewKind::PrimaryOverlay,
                value => {
                    if let Some(call) = call {
                        self.error(
                            call.span,
                            format!(
                                "unknown indicator view {value}; use plot, table, or primary_overlay"
                            ),
                        );
                    }
                    presentation.view
                }
            };
        }
        if let Some(call) = call {
            presentation.panel_title = self.static_named_string(call, "panel_title");
            presentation.layer_name = self.static_named_string(call, "layer_name");
            presentation.data_kind = self.parse_presentation_data_kind(call, "data_kind");
            presentation.mark = self.parse_presentation_mark(call, "mark");
            presentation.axis = self.parse_presentation_axis(call, "axis");
            presentation.value_precision =
                self.parse_presentation_precision(call, "value_precision");
            presentation.axis_label_mode =
                self.parse_presentation_label_mode(call, "axis_label_mode");
            presentation.header_label_mode =
                self.parse_presentation_label_mode(call, "header_label_mode");
            presentation.preferred_scale = self.parse_presentation_scale(call, "preferred_scale");
            presentation.max_decimals =
                self.static_named_value(call, "max_decimals")
                    .and_then(|value| match value {
                        Value::Int(value) if (0..=18).contains(&value) => Some(value as u8),
                        _ => {
                            self.error(call.span, "max_decimals must be an integer from 0 to 18");
                            None
                        }
                    });
        }
        IndicatorDescriptor {
            id,
            title,
            optional_trades,
            max_bars_back,
            inputs: Vec::new(),
            plots: Vec::new(),
            presentation,
        }
    }

    fn compile_statement(&mut self, statement: AstStatement) -> Option<crate::hir::StatementId> {
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
                    let state_slot = self.states.get(&name).map(|(slot, _)| *slot);
                    let local_slot = self.values.get(&name).map(|(slot, _)| *slot);
                    if let Some(slot) = local_slot {
                        self.mutable_values.insert(slot.0);
                    }
                    let expression = self.compile_expression(&expression);
                    let expression_type = self.program.expressions[expression.0].ty;
                    if let Some(slot) = state_slot {
                        if let Some(entry) = self.states.get_mut(&name) {
                            entry.1 = expression_type;
                        }
                        Some(self.push_statement(TypedStatement {
                            kind: StatementKind::StateUpdate {
                                target: slot,
                                expression,
                            },
                        }))
                    } else if let Some(slot) = local_slot {
                        if let Some(entry) = self.values.get_mut(&name) {
                            entry.1 = expression_type;
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
                    if self.states.contains_key(&name) {
                        self.error(span, format!("state {name} is already declared"));
                        return None;
                    }
                    let expression = self.compile_expression(&expression);
                    let ty = self.program.expressions[expression.0].ty;
                    let slot = StateSlot(self.next_state);
                    self.next_state += 1;
                    let call_site = CallSiteId(self.next_call_site);
                    self.next_call_site += 1;
                    self.state_call_sites.push(call_site);
                    self.state_functions.push(self.current_function);
                    self.states.insert(name, (slot, ty));
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::StateDeclaration {
                            target: slot,
                            initial: expression,
                            call_site,
                        },
                    }))
                } else {
                    if self.values.contains_key(&name) || self.states.contains_key(&name) {
                        self.error(span, format!("{name} is already declared"));
                        return None;
                    }
                    if let AstExpression::Call(call) = &expression
                        && call.path.first().is_some_and(|segment| segment == "input")
                    {
                        return self.compile_input(&name, call, span);
                    }
                    let expression_id = self.compile_expression(&expression);
                    let ty = self.program.expressions[expression_id.0].ty;
                    let slot = LocalSlot(self.next_local);
                    self.next_local += 1;
                    self.local_definitions.push(Some(expression_id));
                    self.values.insert(name, (slot, ty));
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
                        .map(|name| self.values.get(name).map(|(slot, _)| *slot))
                        .collect::<Option<Vec<_>>>();
                    let Some(targets) = targets else {
                        self.error(span, "destructuring update references an unknown local");
                        return None;
                    };
                    let expression = self.compile_expression(&expression);
                    if !matches!(
                        self.program.expressions[expression.0].ty.value,
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
                        self.values.contains_key(name) || self.states.contains_key(name)
                    }) {
                        self.error(span, "destructuring binding shadows an existing value");
                        return None;
                    }
                    let expression = self.compile_expression(&expression);
                    let expression_type = self.program.expressions[expression.0].ty;
                    if !matches!(
                        expression_type.value,
                        ValueType::Tuple | ValueType::Array | ValueType::Any
                    ) {
                        self.error(span, "destructuring requires a tuple value");
                    }
                    let targets = names
                        .into_iter()
                        .map(|name| {
                            let slot = LocalSlot(self.next_local);
                            self.next_local += 1;
                            self.local_definitions.push(None);
                            self.values
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
                "level" => self.compile_level(&call),
                "plotcandle" | "plotohlc" => self.compile_ohlc(&call),
                "broken_cloud" | "plotrange" => self.compile_range(&call),
                "emit" => self.compile_emit(&call),
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
                if self.program.expressions[condition.0].ty.value != ValueType::Bool {
                    self.error(span, "if condition must be boolean");
                }
                let saved_values = self.values.clone();
                let saved_states = self.states.clone();
                let then_branch = self.compile_scoped_statements(then_branch);
                self.values = saved_values.clone();
                self.states = saved_states.clone();
                let else_branch = self.compile_scoped_statements(else_branch);
                self.values = saved_values;
                self.states = saved_states;
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
                if self.program.expressions[condition.0].ty.value != ValueType::Bool {
                    self.error(span, "while condition must be boolean");
                }
                self.loop_depth += 1;
                let saved_values = self.values.clone();
                let saved_states = self.states.clone();
                let body = self.compile_scoped_statements(body);
                self.values = saved_values;
                self.states = saved_states;
                self.loop_depth = self.loop_depth.saturating_sub(1);
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
                if self.values.contains_key(&name) || self.states.contains_key(&name) {
                    self.error(span, format!("{name} is already declared"));
                    return None;
                }
                let saved_values = self.values.clone();
                let saved_states = self.states.clone();
                let start = self.compile_expression(&start);
                let end = self.compile_expression(&end);
                if self.program.expressions[start.0].ty.value != ValueType::Int
                    || self.program.expressions[end.0].ty.value != ValueType::Int
                {
                    self.error(span, "range loop bounds must be integers");
                }
                let target = LocalSlot(self.next_local);
                self.next_local += 1;
                self.local_definitions.push(None);
                self.values
                    .insert(name, (target, Type::series(ValueType::Int)));
                self.loop_depth += 1;
                let body = self.compile_scoped_statements(body);
                self.values = saved_values;
                self.states = saved_states;
                self.loop_depth = self.loop_depth.saturating_sub(1);
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
                if self.values.contains_key(&name) || self.states.contains_key(&name) {
                    self.error(span, format!("{name} is already declared"));
                    return None;
                }
                let saved_values = self.values.clone();
                let saved_states = self.states.clone();
                let collection = self.compile_expression(&collection);
                let collection_type = self.program.expressions[collection.0].ty.value;
                if !matches!(
                    collection_type,
                    ValueType::Array | ValueType::Tuple | ValueType::Any
                ) {
                    self.error(span, "collection loops require an array or tuple");
                }
                let target = LocalSlot(self.next_local);
                self.next_local += 1;
                self.local_definitions.push(None);
                self.values.insert(
                    name,
                    (
                        target,
                        Type::series(self.collection_element_type(collection)),
                    ),
                );
                self.loop_depth += 1;
                let body = self.compile_scoped_statements(body);
                self.values = saved_values;
                self.states = saved_states;
                self.loop_depth = self.loop_depth.saturating_sub(1);
                Some(self.push_statement(TypedStatement {
                    kind: StatementKind::ForEach {
                        target,
                        collection,
                        body,
                    },
                }))
            }
            AstStatement::Break(span) => {
                if self.loop_depth == 0 {
                    self.error(span, "break is only valid inside a loop");
                    None
                } else {
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Break,
                    }))
                }
            }
            AstStatement::Continue(span) => {
                if self.loop_depth == 0 {
                    self.error(span, "continue is only valid inside a loop");
                    None
                } else {
                    Some(self.push_statement(TypedStatement {
                        kind: StatementKind::Continue,
                    }))
                }
            }
            AstStatement::Return { expression, span } => {
                if self.current_function.is_none() {
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

    fn compile_scoped_statements(
        &mut self,
        statements: Vec<AstStatement>,
    ) -> Vec<crate::hir::StatementId> {
        self.statement_scope_depth = self.statement_scope_depth.saturating_add(1);
        let compiled = statements
            .into_iter()
            .filter_map(|statement| self.compile_statement(statement))
            .collect();
        self.statement_scope_depth = self.statement_scope_depth.saturating_sub(1);
        compiled
    }

    fn compile_input(
        &mut self,
        name: &str,
        call: &AstCall,
        span: Span,
    ) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(span, "input declarations must be top-level");
            return None;
        }
        self.validate_input_arguments(call);
        let Some(kind) = call.path.get(1).map(String::as_str) else {
            self.error(span, "input requires a type, for example input.int");
            return None;
        };
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
        self.inputs.insert(
            name.to_owned(),
            InputDescriptor {
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
                options,
            },
        );
        self.input_spans.insert(name.to_owned(), span);
        let slot = LocalSlot(self.next_local);
        self.next_local += 1;
        self.local_definitions.push(None);
        self.values
            .insert(name.to_owned(), (slot, Type::simple(value_type)));
        self.program.inputs.push((slot, name.to_owned()));
        let expression =
            self.push_expression(ExpressionKind::LoadInput(slot), Type::simple(value_type));
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Binding {
                target: slot,
                expression,
            },
        }))
    }

    fn input_options(&mut self, call: &AstCall) -> Option<Vec<InputOption>> {
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

    fn presentation_string(
        &mut self,
        call: &AstCall,
        name: &str,
        positional: Option<usize>,
    ) -> Option<String> {
        if let Some(value) = self.static_named_string(call, name) {
            return Some(value);
        }
        let index = positional?;
        let argument = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index);
        match argument.and_then(|argument| constant_value(&argument.expression)) {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(
                    argument.map_or(call.span, |argument| argument.expression.span()),
                    format!("{name} must be a string literal"),
                );
                None
            }
            None => {
                self.error(
                    argument.map_or(call.span, |argument| argument.expression.span()),
                    format!("{name} must be a compile-time string"),
                );
                None
            }
        }
    }

    fn required_presentation_string(
        &mut self,
        call: &AstCall,
        names: &[&str],
        positional: Option<usize>,
        description: &str,
    ) -> Option<String> {
        for name in names {
            if call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some(*name))
            {
                return self.presentation_string(call, name, None);
            }
        }
        let Some(index) = positional else {
            self.error(call.span, format!("{description} is required"));
            return None;
        };
        let has_positional = call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(index)
            .is_some();
        if !has_positional {
            self.error(call.span, format!("{description} is required"));
            return None;
        }
        self.presentation_string(call, names[0], Some(index))
    }

    fn required_presentation_mark(
        &mut self,
        call: &AstCall,
        name: &str,
        description: &str,
    ) -> Option<PresentationMarkKind> {
        if !call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some(name))
        {
            self.error(call.span, format!("{description} is required"));
            return None;
        }
        self.parse_presentation_mark(call, name)
    }

    fn validate_named_arguments(&mut self, call: &AstCall, allowed: &[&str]) {
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

    fn validate_input_arguments(&mut self, call: &AstCall) {
        self.validate_positional_arguments(call, 2, "input");
        self.validate_named_arguments(
            call,
            &[
                "defval", "title", "min", "max", "step", "options", "group", "inline", "tooltip",
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

    fn validate_positional_arguments(&mut self, call: &AstCall, maximum: usize, name: &str) {
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

    fn presentation_shade(&mut self, call: &AstCall) -> Option<String> {
        match self.static_named_value(call, "shade") {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(call.span, "shade must be a string literal");
                None
            }
            None => None,
        }
    }

    fn presentation_float(&mut self, call: &AstCall, name: &str, default: f32) -> f32 {
        match self.static_named_value(call, name) {
            Some(Value::Int(value)) => value as f32,
            Some(Value::Float(value)) => value as f32,
            Some(_) => {
                self.error(call.span, format!("{name} must be numeric"));
                default
            }
            None => default,
        }
    }

    fn presentation_int(&mut self, call: &AstCall, name: &str, default: i32) -> i32 {
        match self.static_named_value(call, name) {
            Some(Value::Int(value)) => match i32::try_from(value) {
                Ok(value) => value,
                Err(_) => {
                    self.error(call.span, format!("{name} must fit in a 32-bit integer"));
                    default
                }
            },
            Some(_) => {
                self.error(call.span, format!("{name} must be an integer"));
                default
            }
            None => default,
        }
    }

    fn presentation_color_bindings(
        &mut self,
        call: &AstCall,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let fallback = self.static_named_string(call, "color_slot");
        let (true_slot, false_slot) = self.presentation_conditional_color_bindings(call);
        (fallback, true_slot, false_slot)
    }

    fn presentation_conditional_color_bindings(
        &mut self,
        call: &AstCall,
    ) -> (Option<String>, Option<String>) {
        let true_slot = self.static_named_string(call, "true_color_slot");
        let false_slot = self.static_named_string(call, "false_color_slot");
        (true_slot, false_slot)
    }

    fn compile_style_declaration(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "style declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 2, "style");
        self.validate_named_arguments(call, &["id", "title", "role", "shade"]);
        let id = self.presentation_string(call, "id", Some(0))?;
        let label = if call
            .arguments
            .iter()
            .filter(|argument| argument.name.is_none())
            .nth(1)
            .is_some()
            || call
                .arguments
                .iter()
                .any(|argument| argument.name.as_deref() == Some("title"))
        {
            self.presentation_string(call, "title", Some(1))
                .unwrap_or_else(|| id.clone())
        } else {
            id.clone()
        };
        let role = self
            .presentation_string(call, "role", None)
            .unwrap_or_else(|| "neutral".to_owned());
        let shade = self
            .presentation_shade(call)
            .unwrap_or_else(|| "base".to_owned());
        self.presentation_spans.styles.insert(id.clone(), call.span);
        self.presentation.styles.push(PresentationStyle {
            id,
            label,
            role,
            shade,
        });
        None
    }

    fn compile_mark_declaration(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "mark declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "mark");
        self.validate_named_arguments(
            call,
            &[
                "title",
                "source",
                "kind",
                "color_slot",
                "true_color_slot",
                "false_color_slot",
                "highlight_by",
                "highlight_color_slot",
                "requires",
                "fallback",
            ],
        );
        let label = self.required_presentation_string(call, &["title"], Some(0), "mark title")?;
        let source_channel =
            self.required_presentation_string(call, &["source"], None, "mark source")?;
        let kind = self.required_presentation_mark(call, "kind", "mark kind")?;
        let required_input = self.parse_presentation_input(call, "requires");
        let fallback_mark = self.parse_presentation_mark(call, "fallback");
        let (color_slot, true_color_slot, false_color_slot) =
            self.presentation_color_bindings(call);
        let highlight_by = self.static_named_string(call, "highlight_by");
        let highlight_color_slot = self.static_named_string(call, "highlight_color_slot");
        if color_slot.is_none() {
            self.error(call.span, "mark requires at least one color_slot");
        }
        self.presentation_spans.marks.push(call.span);
        self.presentation.marks.push(PresentationMark {
            label,
            kind,
            source_channel,
            required_input,
            fallback_mark,
            color_slot: color_slot.unwrap_or_default(),
            true_color_slot,
            false_color_slot,
            highlight_by,
            highlight_color_slot,
        });
        None
    }

    fn compile_fill_declaration(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "fill declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 2, "fill");
        self.validate_named_arguments(
            call,
            &[
                "upper",
                "lower",
                "color_slot",
                "geometry",
                "style_by",
                "opacity",
                "upper_offset",
                "lower_offset",
                "true_color_slot",
                "false_color_slot",
            ],
        );
        let upper =
            self.required_presentation_string(call, &["upper"], Some(0), "fill upper output")?;
        let lower =
            self.required_presentation_string(call, &["lower"], Some(1), "fill lower output")?;
        let color_slot =
            self.required_presentation_string(call, &["color_slot"], None, "fill color_slot")?;
        let geometry = self
            .presentation_string(call, "geometry", None)
            .unwrap_or_else(|| "panel_band".to_owned());
        let geometry = match geometry.to_ascii_lowercase().as_str() {
            "panel_band" => PresentationGeometry::PanelBand,
            "series_band" => PresentationGeometry::SeriesBand,
            value => {
                self.error(call.span, format!("unknown fill geometry {value}"));
                PresentationGeometry::PanelBand
            }
        };
        let style_by = self.static_named_string(call, "style_by");
        let (true_color_slot, false_color_slot) =
            self.presentation_conditional_color_bindings(call);
        let opacity = self.presentation_float(call, "opacity", 0.2);
        let upper_offset_bars = self.presentation_int(call, "upper_offset", 0);
        let lower_offset_bars = self.presentation_int(call, "lower_offset", 0);
        self.presentation_spans.fills.push(call.span);
        self.presentation.fills.push(PresentationFill {
            upper,
            lower,
            upper_offset_bars,
            lower_offset_bars,
            style_by,
            geometry,
            color_slot,
            true_color_slot,
            false_color_slot,
            opacity,
        });
        None
    }

    fn compile_readout_declaration(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "readout declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "readout");
        self.validate_named_arguments(call, &["title", "source", "color_slot", "mark"]);
        let label =
            self.required_presentation_string(call, &["title"], Some(0), "readout title")?;
        let channel =
            self.required_presentation_string(call, &["source"], None, "readout source")?;
        let color_slot =
            self.required_presentation_string(call, &["color_slot"], None, "readout color_slot")?;
        let mark = self.parse_presentation_mark(call, "mark");
        self.presentation_spans.readouts.push(call.span);
        self.presentation.readouts.push(PresentationReadout {
            label,
            channel,
            color_slot,
            mark,
        });
        None
    }

    fn compile_column_declaration(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "column declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "column");
        self.validate_named_arguments(
            call,
            &[
                "title",
                "source",
                "format",
                "color_slot",
                "color_mode",
                "positive_color_slot",
                "negative_color_slot",
            ],
        );
        let label = self.required_presentation_string(call, &["title"], Some(0), "column title")?;
        let channel =
            self.required_presentation_string(call, &["source"], None, "column source")?;
        let format = self
            .presentation_string(call, "format", None)
            .unwrap_or_else(|| "quantity".to_owned());
        let color_mode = self
            .presentation_string(call, "color_mode", None)
            .unwrap_or_else(|| "static".to_owned());
        let has_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("color_slot"));
        let has_positive_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("positive_color_slot"));
        let has_negative_color_slot = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("negative_color_slot"));
        let color = match color_mode.to_ascii_lowercase().as_str() {
            "static" => {
                if has_positive_color_slot || has_negative_color_slot {
                    self.error(
                        call.span,
                        "static table columns use color_slot and cannot provide signed color slots",
                    );
                }
                PresentationColumnColor::Static {
                    slot: self
                        .presentation_string(call, "color_slot", None)
                        .unwrap_or_else(|| "main".to_owned()),
                }
            }
            "signed" => {
                if has_color_slot {
                    self.error(
                        call.span,
                        "signed table columns cannot provide color_slot; use positive_color_slot and negative_color_slot",
                    );
                }
                if !has_positive_color_slot {
                    self.error(
                        call.span,
                        "signed table columns require positive_color_slot",
                    );
                }
                if !has_negative_color_slot {
                    self.error(
                        call.span,
                        "signed table columns require negative_color_slot",
                    );
                }
                PresentationColumnColor::Signed {
                    positive_slot: self
                        .presentation_string(call, "positive_color_slot", None)
                        .unwrap_or_default(),
                    negative_slot: self
                        .presentation_string(call, "negative_color_slot", None)
                        .unwrap_or_default(),
                }
            }
            _ => {
                self.error(
                    call.span,
                    format!(
                        "unknown table column color_mode '{}'; use static or signed",
                        color_mode
                    ),
                );
                PresentationColumnColor::Static {
                    slot: self
                        .presentation_string(call, "color_slot", None)
                        .unwrap_or_else(|| "main".to_owned()),
                }
            }
        };
        self.presentation_spans.columns.push(call.span);
        self.presentation.columns.push(PresentationColumn {
            label,
            channel,
            format,
            color,
        });
        None
    }

    fn compile_plot(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 1, "plot");
        let Some(argument) = argument_at(call, 0, "value") else {
            self.error(call.span, "plot expects a series expression");
            return None;
        };
        let expression = self.compile_expression(&argument.expression);
        let ty = self.program.expressions[expression.0].ty;
        if !matches!(
            ty.value,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(argument.expression.span(), "plot expects a series value");
            return None;
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| match &argument.expression {
                AstExpression::Identifier(path, _) => path.last().cloned(),
                _ => None,
            })
            .unwrap_or_else(|| format!("plot_{}", self.program.plots.len()));
        let plot = crate::hir::PlotId(self.program.plots.len());
        let label = self
            .static_named_string(call, "title")
            .unwrap_or_else(|| "Plot".to_owned());
        let value_type = if matches!(ty.value, ValueType::Bool) {
            PlotValueType::Boolean
        } else {
            PlotValueType::Number
        };
        let descriptor = self.plot_descriptor(call, id, label, value_type);
        self.presentation_spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Plot {
                plot,
                expression,
                color,
            },
        }))
    }

    fn compile_emit(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let Some(id_argument) = argument_at(call, 0, "id") else {
            self.error(
                call.span,
                "emit expects a constant output ID as its first argument",
            );
            return None;
        };
        let Some(Value::String(id)) = constant_value(&id_argument.expression) else {
            self.error(
                id_argument.expression.span(),
                "emit expects a constant output ID as its first argument",
            );
            return None;
        };
        let Some(argument) = argument_at(call, 1, "value") else {
            self.error(call.span, "emit expects a value as its second argument");
            return None;
        };
        let expression = self.compile_expression(&argument.expression);
        let ty = self.program.expressions[expression.0].ty;
        if !matches!(
            ty.value,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(
                argument.expression.span(),
                "emit currently supports numeric and boolean values",
            );
            return None;
        }
        let plot = crate::hir::PlotId(self.program.plots.len());
        let value_type = if matches!(ty.value, ValueType::Bool) {
            PlotValueType::Boolean
        } else {
            PlotValueType::Number
        };
        let descriptor = self.plot_descriptor(call, id, "Output".to_owned(), value_type);
        self.presentation_spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Plot {
                plot,
                expression,
                color,
            },
        }))
    }

    fn compile_marker(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "marker declarations must be top-level");
            return None;
        }
        let value_argument = argument_at(call, 0, "value");
        let Some(value_argument) = value_argument else {
            self.error(call.span, "marker expects a value expression");
            return None;
        };
        let expression = self.compile_expression(&value_argument.expression);
        let value_type = self.program.expressions[expression.0].ty.value;
        if !matches!(
            value_type,
            ValueType::Int | ValueType::Float | ValueType::Bool | ValueType::Any
        ) {
            self.error(
                value_argument.expression.span(),
                "marker expects a numeric or boolean value",
            );
            return None;
        }
        let visible = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("visible"))
            .map(|argument| self.compile_expression(&argument.expression));
        if let Some(visible) = visible {
            let visible_type = self.program.expressions[visible.0].ty.value;
            if !matches!(visible_type, ValueType::Bool | ValueType::Any) {
                self.error(call.span, "marker visibility must be boolean");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("marker_{}", self.marker_count()));
        let color = self.compile_color_argument(call);
        let text = self.static_named_string(call, "text");
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Marker {
                id,
                expression,
                visible,
                color,
                text,
            },
        }))
    }

    fn compile_segment(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "segment declarations must be top-level");
            return None;
        }
        self.validate_positional_arguments(call, 4, "segment");
        self.validate_named_arguments(
            call,
            &[
                "start_value",
                "end_value",
                "start_index",
                "end_index",
                "id",
                "title",
                "label",
                "color_slot",
                "width",
                "visible",
                "color",
                "text",
            ],
        );

        let Some(start_value_argument) = argument_at(call, 0, "start_value") else {
            self.error(call.span, "segment expects a start value expression");
            return None;
        };
        let Some(end_value_argument) = argument_at(call, 1, "end_value") else {
            self.error(call.span, "segment expects an end value expression");
            return None;
        };
        let Some(start_index_argument) = argument_at(call, 2, "start_index") else {
            self.error(call.span, "segment expects a start index expression");
            return None;
        };
        let Some(end_index_argument) = argument_at(call, 3, "end_index") else {
            self.error(call.span, "segment expects an end index expression");
            return None;
        };

        let start_value = self.compile_expression(&start_value_argument.expression);
        let end_value = self.compile_expression(&end_value_argument.expression);
        let start_index = self.compile_expression(&start_index_argument.expression);
        let end_index = self.compile_expression(&end_index_argument.expression);

        for (argument, expression, label) in [
            (start_value_argument, start_value, "start value"),
            (end_value_argument, end_value, "end value"),
        ] {
            if !matches!(
                self.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("segment {label} must be numeric"),
                );
            }
        }
        for (argument, expression, label) in [
            (start_index_argument, start_index, "start index"),
            (end_index_argument, end_index, "end index"),
        ] {
            if !matches!(
                self.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Any
            ) {
                self.error(
                    argument.expression.span(),
                    format!("segment {label} must be an integer"),
                );
            }
        }

        let visible = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("visible"))
            .map(|argument| self.compile_expression(&argument.expression));
        if let Some(visible) = visible
            && !matches!(
                self.program.expressions[visible.0].ty.value,
                ValueType::Bool | ValueType::Any
            )
        {
            self.error(call.span, "segment visibility must be boolean");
        }

        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("segment_{}", self.segment_count()));
        let label = self
            .static_named_string(call, "title")
            .or_else(|| self.static_named_string(call, "label"))
            .unwrap_or_else(|| id.clone());
        let color_slot = self
            .static_named_string(call, "color_slot")
            .unwrap_or_else(|| "main".to_owned());
        let line_width = self.presentation_float(call, "width", 1.0);
        self.presentation_spans
            .segments
            .insert(id.clone(), call.span);
        self.presentation.segments.push(PresentationSegment {
            id: id.clone(),
            label,
            color_slot,
            line_width,
        });
        let color = self.compile_color_argument(call);
        let text = self.static_named_string(call, "text");
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::Segment {
                id,
                start_value,
                end_value,
                start_index,
                end_index,
                visible,
                color,
                text,
            },
        }))
    }

    fn compile_level(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "level declarations must be top-level");
            return None;
        }
        let value_argument = argument_at(call, 0, "value");
        let Some(value_argument) = value_argument else {
            self.error(call.span, "level expects a value expression");
            return None;
        };
        let expression = self.compile_expression(&value_argument.expression);
        let value_type = self.program.expressions[expression.0].ty.value;
        if !matches!(
            value_type,
            ValueType::Int | ValueType::Float | ValueType::Any
        ) {
            self.error(
                value_argument.expression.span(),
                "level expects a numeric value",
            );
            return None;
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("level_{}", self.level_count()));
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::HorizontalLevel {
                id,
                expression,
                color,
            },
        }))
    }

    fn compile_ohlc(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let argument = |name: &str, index: usize| argument_at(call, index, name);
        let Some(open) = argument("open", 0) else {
            self.error(
                call.span,
                "plotohlc expects open, high, low, and close expressions",
            );
            return None;
        };
        let Some(high) = argument("high", 1) else {
            self.error(
                call.span,
                "plotohlc expects high, low, and close expressions",
            );
            return None;
        };
        let Some(low) = argument("low", 2) else {
            self.error(call.span, "plotohlc expects low and close expressions");
            return None;
        };
        let Some(close) = argument("close", 3) else {
            self.error(call.span, "plotohlc expects a close expression");
            return None;
        };
        let values = [open, high, low, close]
            .into_iter()
            .map(|argument| self.compile_expression(&argument.expression))
            .collect::<Vec<_>>();
        for (argument, expression) in [open, high, low, close].into_iter().zip(values.iter()) {
            if !matches!(
                self.program.expressions[expression.0].ty.value,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(argument.expression.span(), "OHLC values must be numeric");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("plot_{}", self.program.plots.len()));
        let plot = crate::hir::PlotId(self.program.plots.len());
        let mut descriptor = self.plot_descriptor(call, id, "OHLC".to_owned(), PlotValueType::Ohlc);
        descriptor.style = if call.path.join(".") == "plotohlc" {
            PlotStyle::Bars
        } else {
            PlotStyle::Candles
        };
        self.presentation_spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::OhlcPlot {
                plot,
                open: values[0],
                high: values[1],
                low: values[2],
                close: values[3],
                color,
            },
        }))
    }

    fn compile_range(&mut self, call: &AstCall) -> Option<crate::hir::StatementId> {
        if !self.is_top_level_declaration() {
            self.error(call.span, "plot declarations must be top-level");
            return None;
        }
        let upper_argument = argument_at(call, 0, "upper").or_else(|| argument_at(call, 0, "high"));
        let lower_argument = argument_at(call, 1, "lower").or_else(|| argument_at(call, 1, "low"));
        let (Some(upper_argument), Some(lower_argument)) = (upper_argument, lower_argument) else {
            self.error(call.span, "range plot expects upper and lower expressions");
            return None;
        };
        let upper = self.compile_expression(&upper_argument.expression);
        let lower = self.compile_expression(&lower_argument.expression);
        for (argument, expression) in [(upper_argument, upper), (lower_argument, lower)] {
            let value_type = self.program.expressions[expression.0].ty.value;
            if !matches!(
                value_type,
                ValueType::Int | ValueType::Float | ValueType::Any
            ) {
                self.error(argument.expression.span(), "range values must be numeric");
            }
        }
        let id = self
            .static_named_string(call, "id")
            .or_else(|| self.static_named_string(call, "title"))
            .unwrap_or_else(|| format!("plot_{}", self.program.plots.len()));
        let plot = crate::hir::PlotId(self.program.plots.len());
        let mut descriptor =
            self.plot_descriptor(call, id, "Range".to_owned(), PlotValueType::Range);
        descriptor.style = match call.path.join(".").as_str() {
            "broken_cloud" => PlotStyle::BrokenCloud,
            _ => PlotStyle::Cloud,
        };
        self.presentation_spans
            .plots
            .insert(descriptor.id.clone(), call.span);
        self.program.plots.push(descriptor);
        let color = self.compile_color_argument(call);
        Some(self.push_statement(TypedStatement {
            kind: StatementKind::RangePlot {
                plot,
                upper,
                lower,
                color,
            },
        }))
    }

    fn compile_color_argument(&mut self, call: &AstCall) -> Option<ExprId> {
        let argument = call
            .arguments
            .iter()
            .find(|argument| argument.name.as_deref() == Some("color"))?;
        let expression = self.compile_expression(&argument.expression);
        let value_type = self.program.expressions[expression.0].ty.value;
        if !matches!(value_type, ValueType::Color | ValueType::Any) {
            self.error(
                argument.expression.span(),
                "plot color must be a color value",
            );
        }
        Some(expression)
    }

    fn is_top_level_declaration(&self) -> bool {
        self.current_function.is_none() && self.statement_scope_depth == 0
    }

    fn marker_count(&self) -> usize {
        self.program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::Marker { .. }))
            .count()
    }

    fn segment_count(&self) -> usize {
        self.program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::Segment { .. }))
            .count()
    }

    fn level_count(&self) -> usize {
        self.program
            .statements
            .iter()
            .filter(|statement| matches!(statement.kind, StatementKind::HorizontalLevel { .. }))
            .count()
    }

    fn compile_expression(&mut self, expression: &AstExpression) -> ExprId {
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
                let operand_type = self.program.expressions[operand.0].ty;
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
                let left_type = self.program.expressions[left.0].ty;
                let right_type = self.program.expressions[right.0].ty;
                let qualifier = if left_type.qualifier == Qualifier::Series
                    || right_type.qualifier == Qualifier::Series
                {
                    Qualifier::Series
                } else {
                    Qualifier::Simple
                };
                let value = match operator {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::And
                    | BinaryOperator::Or => ValueType::Bool,
                    _ if matches!(
                        left_type.value,
                        ValueType::Int | ValueType::Float | ValueType::Any
                    ) && matches!(
                        right_type.value,
                        ValueType::Int | ValueType::Float | ValueType::Any
                    ) =>
                    {
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
                    self.program.expressions[expression.0].ty.qualifier == Qualifier::Series
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
                    self.program.expressions[expression.0].ty.qualifier == Qualifier::Series
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
                    self.program.expressions[expression.0].ty.qualifier == Qualifier::Series
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
                    .map(|expression| self.program.expressions[expression.0].ty.value)
                    .unwrap_or(ValueType::Any);
                let mut qualifier = self.program.expressions[operand.0].ty.qualifier;
                for (condition, result) in &branches {
                    if self.program.expressions[condition.0].ty.value
                        != self.program.expressions[operand.0].ty.value
                        && self.program.expressions[condition.0].ty.value != ValueType::Any
                        && self.program.expressions[operand.0].ty.value != ValueType::Any
                    {
                        self.error(*span, "switch branch condition has the wrong type");
                    }
                    result_type =
                        merge_value_types(result_type, self.program.expressions[result.0].ty.value);
                    qualifier = if qualifier == Qualifier::Series
                        || self.program.expressions[condition.0].ty.qualifier == Qualifier::Series
                        || self.program.expressions[result.0].ty.qualifier == Qualifier::Series
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
                let source_type = self.program.expressions[source.0].ty;
                let offset_type = self.program.expressions[offset.0].ty;
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
                        if let Some(descriptor) = self.descriptor.as_mut() {
                            descriptor.max_bars_back = descriptor
                                .max_bars_back
                                .max(offset.saturating_add(1) as usize);
                        }
                    }
                    Some(Value::Int(_)) => self.error(*span, "history offset cannot be negative"),
                    _ if !self.max_bars_back_explicit => self.error(
                        *span,
                        "dynamic history offsets require max_bars_back in indicator declaration",
                    ),
                    _ => {}
                }
                self.push_expression(ExpressionKind::History { source, offset }, source_type)
            }
            AstExpression::Field {
                expression,
                field,
                span,
            } => {
                let source = self.compile_expression(expression);
                let source_type = self.program.expressions[source.0].ty;
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

    fn plot_descriptor(
        &mut self,
        call: &AstCall,
        id: String,
        label: String,
        value_type: PlotValueType,
    ) -> PlotDescriptor {
        self.validate_named_arguments(
            call,
            &[
                "value",
                "id",
                "title",
                "pane",
                "scale",
                "style",
                "display",
                "precision",
                "width",
                "offset",
                "color_slot",
                "style_by",
                "true_color_slot",
                "false_color_slot",
                "layer_title",
                "overlay",
                "data_kind",
                "mark",
                "axis",
                "layer",
                "color",
            ],
        );
        let default_pane = if self.descriptor.as_ref().is_some_and(|descriptor| {
            matches!(
                descriptor.presentation.view,
                PresentationViewKind::PrimaryOverlay
            )
        }) {
            PaneTarget::Main
        } else {
            PaneTarget::Indicator
        };
        let pane = match self.static_named_string(call, "pane").as_deref() {
            Some("main") => PaneTarget::Main,
            Some("indicator") => PaneTarget::Indicator,
            None => default_pane,
            Some(value) if value.strip_prefix("named:").is_some() => value
                .strip_prefix("named:")
                .and_then(|value| value.parse::<u32>().ok())
                .map(PaneTarget::Named)
                .unwrap_or_else(|| {
                    self.error(
                        call.span,
                        "plot pane must be main, indicator, or named:<id>",
                    );
                    default_pane
                }),
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot pane {value}; use main, indicator, or named:<id>"),
                );
                default_pane
            }
        };
        let scale = match self.static_named_string(call, "scale").as_deref() {
            Some("independent") => PlotScale::Independent,
            Some("price") | None => PlotScale::Price,
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot scale {value}; use price or independent"),
                );
                PlotScale::Price
            }
        };
        let style = match self.static_named_string(call, "style").as_deref() {
            Some("line") => PlotStyle::Line,
            Some("step") => PlotStyle::Step,
            Some("histogram") => PlotStyle::Histogram,
            Some("area") => PlotStyle::Area,
            Some("candles") => PlotStyle::Candles,
            Some("bars") => PlotStyle::Bars,
            Some("cloud") => PlotStyle::Cloud,
            Some("broken_cloud") => PlotStyle::BrokenCloud,
            Some(value) => {
                self.error(call.span, format!("unknown plot style {value}"));
                PlotStyle::Line
            }
            None => PlotStyle::Line,
        };
        let explicit_display = call
            .arguments
            .iter()
            .any(|argument| argument.name.as_deref() == Some("display"));
        let readout = match self.static_named_string(call, "display").as_deref() {
            Some("none") => ReadoutBehavior::Hidden,
            Some("value") | None => ReadoutBehavior::Value,
            Some(value) => {
                self.error(
                    call.span,
                    format!("unknown plot display mode {value}; use value or none"),
                );
                ReadoutBehavior::Value
            }
        };
        // Boolean series are signals for other declarations (for example
        // `style_by` or a table `column`), not visual output by themselves.
        // Boolean series are hidden unless the source explicitly displays them.
        let readout = if value_type == PlotValueType::Boolean && !explicit_display {
            ReadoutBehavior::Hidden
        } else {
            readout
        };
        let output_precision = match self.static_named_value(call, "precision") {
            Some(Value::String(value)) => {
                let output_precision = if value.eq_ignore_ascii_case("instrument") {
                    Some(OutputPrecision::Instrument)
                } else if let Some(decimals) = value.strip_prefix("decimals:") {
                    decimals
                        .parse::<u8>()
                        .ok()
                        .filter(|decimals| *decimals <= 18)
                        .map(OutputPrecision::Decimals)
                } else if let Some(step) = value.strip_prefix("step:") {
                    step.parse::<f32>()
                        .ok()
                        .filter(|step| step.is_finite() && *step > 0.0)
                        .map(OutputPrecision::Step)
                } else {
                    None
                };
                if output_precision.is_none() {
                    self.error(
                        call.span,
                        "plot precision must be instrument, decimals:<n>, or step:<value>",
                    );
                }
                output_precision
            }
            Some(_) => {
                self.error(
                    call.span,
                    "plot precision must be instrument, decimals:<n>, or step:<value>",
                );
                None
            }
            None => None,
        };
        let width = match self.static_named_value(call, "width") {
            Some(Value::Int(value)) if (0..=64).contains(&value) && value > 0 => value as f32,
            Some(Value::Float(value))
                if value.is_finite() && (0.0..=64.0).contains(&value) && value > 0.0 =>
            {
                value as f32
            }
            Some(_) => {
                self.error(call.span, "plot width must be a positive number up to 64");
                1.0
            }
            None => 1.0,
        };
        let offset_bars = match self.static_named_value(call, "offset") {
            Some(Value::Int(value)) => match i32::try_from(value) {
                Ok(value) => value,
                Err(_) => {
                    self.error(call.span, "plot offset must fit in a 32-bit integer");
                    0
                }
            },
            Some(_) => {
                self.error(call.span, "plot offset must be an integer");
                0
            }
            None => 0,
        };
        PlotDescriptor {
            id,
            label,
            value_type,
            pane,
            scale,
            width,
            output_precision,
            style,
            readout,
            offset_bars,
            color_slot: self.static_named_string(call, "color_slot"),
            style_by: self.static_named_string(call, "style_by"),
            true_color_slot: self.static_named_string(call, "true_color_slot"),
            false_color_slot: self.static_named_string(call, "false_color_slot"),
            layer_title: self.static_named_string(call, "layer_title"),
            overlay: match self.static_named_value(call, "overlay") {
                Some(Value::Bool(value)) => value,
                Some(_) => {
                    self.error(call.span, "plot overlay must be a boolean literal");
                    true
                }
                None => true,
            },
            presentation_data_kind: self.parse_presentation_data_kind(call, "data_kind"),
            presentation_mark: self.parse_presentation_mark(call, "mark"),
            presentation_axis: self.parse_presentation_axis(call, "axis"),
            layer: match self.static_named_value(call, "layer") {
                Some(Value::Bool(value)) => value,
                Some(_) => {
                    self.error(call.span, "plot layer must be a boolean literal");
                    true
                }
                None => true,
            },
        }
    }

    fn static_argument(
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

    fn static_input_argument(
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

    fn static_input_string(
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

    fn static_named_number(&mut self, call: &AstCall, names: &[&str]) -> Option<f64> {
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

    fn parse_presentation_data_kind(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationDataKind> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "scalar" => PresentationDataKind::Scalar,
            "histogram" => PresentationDataKind::Histogram,
            "ohlc" => PresentationDataKind::Ohlc,
            "range" => PresentationDataKind::Range,
            value => {
                self.error(call.span, format!("unknown presentation data kind {value}"));
                PresentationDataKind::Scalar
            }
        })
    }

    fn parse_presentation_mark(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationMarkKind> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "candle" => PresentationMarkKind::Candle,
            "regular_bar" => PresentationMarkKind::RegularBar,
            "histogram" => PresentationMarkKind::Histogram,
            "histogram_signed_overlay" => PresentationMarkKind::HistogramSignedOverlay,
            "line" => PresentationMarkKind::Line,
            "step" => PresentationMarkKind::Step,
            "area" => PresentationMarkKind::Area,
            "point" => PresentationMarkKind::Point,
            "cross" => PresentationMarkKind::Cross,
            "segment" => PresentationMarkKind::Segment,
            "horizontal_level" => PresentationMarkKind::HorizontalLevel,
            value => {
                self.error(call.span, format!("unknown presentation mark {value}"));
                PresentationMarkKind::Line
            }
        })
    }

    fn parse_presentation_input(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationInputSource> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "klines" => PresentationInputSource::Klines,
            "directional_volume" => PresentationInputSource::DirectionalVolume,
            "trades" => PresentationInputSource::Trades,
            "open_interest" => PresentationInputSource::OpenInterest,
            value => {
                self.error(
                    call.span,
                    format!(
                        "unknown presentation input {value}; use klines, directional_volume, trades, or open_interest"
                    ),
                );
                return None;
            }
        })
    }

    fn parse_presentation_axis(&mut self, call: &AstCall, name: &str) -> Option<PresentationAxis> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "primary" => PresentationAxis::Primary,
            "secondary" => PresentationAxis::Secondary,
            value => {
                self.error(call.span, format!("unknown presentation axis {value}"));
                PresentationAxis::Secondary
            }
        })
    }

    fn parse_presentation_precision(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationPrecision> {
        let value = self.static_named_value(call, name)?;
        match value {
            Value::String(value) => match value.to_ascii_lowercase().as_str() {
                "base_ticker_min_tick" => Some(PresentationPrecision::BaseTickerMinTick),
                "base_ticker_min_qty" => Some(PresentationPrecision::BaseTickerMinQty),
                value => value
                    .strip_prefix("fixed_step:")
                    .and_then(|step| step.parse::<f32>().ok())
                    .filter(|step| step.is_finite() && *step > 0.0)
                    .map(PresentationPrecision::FixedStep)
                    .or_else(|| {
                        self.error(call.span, format!("unknown presentation precision {value}"));
                        None
                    }),
            },
            _ => {
                self.error(call.span, format!("{name} must be a precision string"));
                None
            }
        }
    }

    fn parse_presentation_label_mode(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationLabelMode> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "compact" => PresentationLabelMode::Compact,
            "commas" => PresentationLabelMode::Commas,
            "abbreviated" => PresentationLabelMode::Abbreviated,
            value => {
                self.error(
                    call.span,
                    format!("unknown presentation label mode {value}"),
                );
                PresentationLabelMode::Compact
            }
        })
    }

    fn parse_presentation_scale(
        &mut self,
        call: &AstCall,
        name: &str,
    ) -> Option<PresentationScale> {
        let value = self.static_named_string(call, name)?;
        Some(match value.to_ascii_lowercase().as_str() {
            "absolute" => PresentationScale::Absolute,
            "fit_visible" => PresentationScale::FitVisible,
            "fit_visible_include_zero" => PresentationScale::FitVisibleIncludeZero,
            "logarithmic" => PresentationScale::Logarithmic,
            "percent_from_base" => PresentationScale::PercentFromBase,
            value => {
                self.error(call.span, format!("unknown presentation scale {value}"));
                PresentationScale::FitVisible
            }
        })
    }

    fn static_named_value(&mut self, call: &AstCall, name: &str) -> Option<Value> {
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

    fn static_named_string(&mut self, call: &AstCall, name: &str) -> Option<String> {
        match self.static_named_value(call, name) {
            Some(Value::String(value)) => Some(value),
            Some(_) => {
                self.error(call.span, format!("{name} must be a string literal"));
                None
            }
            None => None,
        }
    }

    fn compile_identifier(&mut self, path: &[String], span: Span) -> ExprId {
        let name = path.join(".");
        let color = match name.as_str() {
            "color.red" => Some(0xffff_0000),
            "color.green" => Some(0xff00_ff00),
            "color.blue" => Some(0xff00_00ff),
            "color.white" => Some(0xffff_ffff),
            "color.black" => Some(0xff00_0000),
            _ => None,
        };
        if let Some(value) = color {
            return self.push_constant(Value::Color(value), Type::simple(ValueType::Color));
        }
        if name == "bar_index" {
            return self
                .push_expression(ExpressionKind::LoadBarIndex, Type::series(ValueType::Int));
        }
        if let Some((slot, ty)) = self.values.get(&name).copied() {
            let series_slot = (ty.qualifier == Qualifier::Series)
                .then(|| self.local_definitions.get(slot.0).copied().flatten())
                .flatten()
                .filter(|_| !self.mutable_values.contains(&slot.0))
                .and_then(|definition| self.program.expressions[definition.0].series_slot);
            return self.push_expression_with_series_slot(
                ExpressionKind::LoadLocal(slot),
                ty,
                series_slot,
            );
        }
        if let Some((slot, ty)) = self.states.get(&name).copied() {
            return self.push_expression(ExpressionKind::LoadState(slot), ty);
        }
        if path.len() == 2
            && let Some(field_kind) = record_field(&path[1])
        {
            let source = if let Some((slot, ty)) = self.values.get(&path[0]).copied() {
                if !matches!(ty.value, ValueType::Record | ValueType::Any) {
                    self.error(span, "field access requires a record");
                }
                Some((
                    self.push_expression(ExpressionKind::LoadLocal(slot), ty),
                    ty,
                ))
            } else if let Some((slot, ty)) = self.states.get(&path[0]).copied() {
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
                | FrameField::ChartVolume => {
                    self.requirements.ohlc = true;
                }
                FrameField::ChartTrade
                | FrameField::ChartTradeAvailable
                | FrameField::ChartTradeBuyQty
                | FrameField::ChartTradeSellQty
                | FrameField::ChartTradeBuyCount
                | FrameField::ChartTradeSellCount
                | FrameField::ChartTradeCount => {
                    self.requirements.trades = true;
                }
                FrameField::ChartDirectionalVolumeAvailable
                | FrameField::ChartBuyQty
                | FrameField::ChartSellQty => {
                    self.requirements.directional_volume = true;
                }
                FrameField::OpenInterest => self.requirements.open_interest = true,
            }
            return self
                .push_expression(ExpressionKind::LoadFrame(field), Type::series(value_type));
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
        if let Some((slot, ty)) = self.values.get(&name).copied() {
            return self.push_expression(ExpressionKind::LoadLocal(slot), ty);
        }
        if let Some((slot, ty)) = self.states.get(&name).copied() {
            return self.push_expression(ExpressionKind::LoadState(slot), ty);
        }
        if path.len() == 2 {
            let source = self
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
                    self.states.get(&path[0]).copied().map(|(slot, ty)| {
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
                    let call_site = CallSiteId(self.next_call_site);
                    self.next_call_site += 1;
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

    fn record_field_for_expression(&self, source: ExprId, name: &str) -> Option<RecordField> {
        if self.is_trade_record_expression(source) {
            return trade_field(name);
        }
        record_field(name)
    }

    fn is_trade_record_expression(&self, expression: ExprId) -> bool {
        matches!(
            self.program
            .expressions
                .get(expression.0)
                .map(|expression| &expression.kind),
            Some(ExpressionKind::RecordField { field, .. }) if field == "trade"
        )
    }

    fn collection_element_type(&self, source: ExprId) -> ValueType {
        let kind = &self.program.expressions[source.0].kind;
        let elements = match kind {
            ExpressionKind::Tuple(elements) | ExpressionKind::Array(elements) => elements,
            _ => return ValueType::Any,
        };
        let mut value_type = None;
        for expression in elements {
            let current = self.program.expressions[expression.0].ty.value;
            value_type =
                Some(value_type.map_or(current, |previous| merge_value_types(previous, current)));
        }
        value_type.unwrap_or(ValueType::Any)
    }

    fn is_numeric_function_expression(&self, expression: ExprId) -> bool {
        let Some(expression) = self.program.expressions.get(expression.0) else {
            return false;
        };
        if matches!(expression.ty.value, ValueType::Int | ValueType::Float) {
            return true;
        }
        matches!(
            expression.kind,
            ExpressionKind::Constant(constant)
                if matches!(self.program.constants.get(constant.0), Some(Value::Na))
        )
    }

    fn compile_call(&mut self, call: &AstCall) -> ExprId {
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
        if let Some(function) = self.functions.get(&name).copied() {
            if let Some(caller) = self.current_function {
                self.call_edges.entry(caller).or_default().insert(function);
            }
            if self.active_functions.contains(&function) {
                self.error(
                    call.span,
                    "recursive functions are not supported in language version 1",
                );
                return self.push_constant(Value::Na, Type::series(ValueType::Any));
            }
            let (expected, required, parameter_names, signature_ready) = self
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
                self.function_signatures
                    .get(&function)
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
            let call_site = CallSiteId(self.next_call_site);
            self.next_call_site += 1;
            let result = self
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
            let actual = self.program.expressions[expression.0].ty;
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
            return self.push_expression(
                ExpressionKind::Conditional {
                    condition: arguments[0],
                    when_true: arguments[1],
                    when_false: arguments[2],
                },
                Type::series(merge_value_types(
                    branch_value_type.unwrap_or(ValueType::Any),
                    branch_fallback_type.unwrap_or(ValueType::Any),
                )),
            );
        }
        if name == "coalesce" && arguments.len() >= 2 {
            let value_type = merge_value_types(
                self.program.expressions[arguments[0].0].ty.value,
                self.program.expressions[arguments[1].0].ty.value,
            );
            return self.push_expression(
                ExpressionKind::Coalesce {
                    source: arguments[0],
                    fallback: arguments[1],
                },
                Type::series(value_type),
            );
        }
        let call_site = CallSiteId(self.next_call_site);
        self.next_call_site += 1;
        let result = if matches!(name.as_str(), "select" | "coalesce") {
            let value_type = merge_value_types(
                branch_value_type.unwrap_or(ValueType::Any),
                branch_fallback_type.unwrap_or(ValueType::Any),
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

    fn push_constant(&mut self, value: Value, ty: Type) -> ExprId {
        let constant = ConstantId(self.program.constants.len());
        self.program.constants.push(value);
        self.push_expression(ExpressionKind::Constant(constant), ty)
    }

    fn push_statement(&mut self, statement: TypedStatement) -> crate::hir::StatementId {
        let id = crate::hir::StatementId(self.program.statements.len());
        self.program.statements.push(statement);
        id
    }

    fn push_expression(&mut self, kind: ExpressionKind, ty: Type) -> ExprId {
        let series_slot = (ty.qualifier == Qualifier::Series).then(|| {
            let slot = SeriesSlot(self.next_series);
            self.next_series += 1;
            slot
        });
        self.push_expression_with_series_slot(kind, ty, series_slot)
    }

    fn push_expression_with_series_slot(
        &mut self,
        kind: ExpressionKind,
        ty: Type,
        series_slot: Option<SeriesSlot>,
    ) -> ExprId {
        let id = ExprId(self.program.expressions.len());
        self.program.expressions.push(TypedExpression {
            kind,
            ty,
            series_slot,
        });
        id
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(self.path.clone(), span, message));
    }
}

fn default_data_kind_for_plot(plot: &PlotDescriptor) -> PresentationDataKind {
    match plot.value_type {
        PlotValueType::Ohlc => PresentationDataKind::Ohlc,
        PlotValueType::Range => PresentationDataKind::Range,
        PlotValueType::Boolean => PresentationDataKind::Scalar,
        PlotValueType::Number => match plot.style {
            PlotStyle::Histogram => PresentationDataKind::Histogram,
            PlotStyle::Candles | PlotStyle::Bars => PresentationDataKind::Ohlc,
            PlotStyle::Cloud | PlotStyle::BrokenCloud => PresentationDataKind::Range,
            PlotStyle::Line | PlotStyle::Step | PlotStyle::Area => PresentationDataKind::Scalar,
        },
    }
}

fn default_mark_for_plot(plot: &PlotDescriptor) -> PresentationMarkKind {
    match plot.value_type {
        PlotValueType::Ohlc => PresentationMarkKind::Candle,
        PlotValueType::Range => PresentationMarkKind::Line,
        PlotValueType::Boolean => PresentationMarkKind::Line,
        PlotValueType::Number => match plot.style {
            PlotStyle::Histogram => PresentationMarkKind::Histogram,
            PlotStyle::Candles => PresentationMarkKind::Candle,
            PlotStyle::Bars => PresentationMarkKind::RegularBar,
            PlotStyle::Step => PresentationMarkKind::Step,
            PlotStyle::Area => PresentationMarkKind::Area,
            PlotStyle::Cloud | PlotStyle::BrokenCloud | PlotStyle::Line => {
                PresentationMarkKind::Line
            }
        },
    }
}

fn default_axis_for_plot(indicator_overlay: bool, plot: &PlotDescriptor) -> PresentationAxis {
    if (plot.overlay || indicator_overlay)
        && plot.pane == PaneTarget::Main
        && matches!(plot.scale, PlotScale::Price)
    {
        PresentationAxis::Primary
    } else {
        PresentationAxis::Secondary
    }
}

fn plot_value_supports_data_kind(
    value_type: PlotValueType,
    data_kind: PresentationDataKind,
) -> bool {
    match value_type {
        PlotValueType::Number => {
            matches!(
                data_kind,
                PresentationDataKind::Scalar | PresentationDataKind::Histogram
            )
        }
        PlotValueType::Boolean => matches!(data_kind, PresentationDataKind::Scalar),
        PlotValueType::Ohlc => matches!(data_kind, PresentationDataKind::Ohlc),
        PlotValueType::Range => matches!(data_kind, PresentationDataKind::Range),
    }
}

fn presentation_mark_compatible(
    data_kind: PresentationDataKind,
    mark: PresentationMarkKind,
) -> bool {
    match data_kind {
        PresentationDataKind::Ohlc => {
            matches!(
                mark,
                PresentationMarkKind::Candle | PresentationMarkKind::Line
            )
        }
        PresentationDataKind::Scalar => matches!(
            mark,
            PresentationMarkKind::Line
                | PresentationMarkKind::Step
                | PresentationMarkKind::Area
                | PresentationMarkKind::Point
                | PresentationMarkKind::Cross
                | PresentationMarkKind::Segment
                | PresentationMarkKind::HorizontalLevel
                | PresentationMarkKind::RegularBar
        ),
        PresentationDataKind::Range => matches!(mark, PresentationMarkKind::Line),
        PresentationDataKind::Histogram => matches!(
            mark,
            PresentationMarkKind::Line
                | PresentationMarkKind::Step
                | PresentationMarkKind::Area
                | PresentationMarkKind::Point
                | PresentationMarkKind::Cross
                | PresentationMarkKind::Segment
                | PresentationMarkKind::HorizontalLevel
                | PresentationMarkKind::RegularBar
                | PresentationMarkKind::Histogram
                | PresentationMarkKind::HistogramSignedOverlay
        ),
    }
}

fn numeric_mark_compatible(mark: PresentationMarkKind) -> bool {
    !matches!(mark, PresentationMarkKind::Candle)
}

fn valid_presentation_role(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "neutral" | "secondary" | "primary" | "success" | "danger" | "warning"
    )
}

fn valid_presentation_shade(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "faint" | "soft" | "base" | "strong" | "vivid"
    )
}

fn argument_at<'a>(
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

fn merge_value_types(left: ValueType, right: ValueType) -> ValueType {
    if left == right {
        return left;
    }
    if matches!(left, ValueType::Any) {
        return right;
    }
    if matches!(right, ValueType::Any) {
        return left;
    }
    if matches!(
        (left, right),
        (ValueType::Int, ValueType::Float) | (ValueType::Float, ValueType::Int)
    ) {
        ValueType::Float
    } else {
        ValueType::Any
    }
}

fn unsupported_call(name: &str) -> bool {
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

fn constant_value(expression: &AstExpression) -> Option<Value> {
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

#[derive(Clone, Copy)]
enum RecordField {
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

fn record_field(name: &str) -> Option<RecordField> {
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

fn trade_field(name: &str) -> Option<RecordField> {
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

fn record_field_type(field: RecordField) -> ValueType {
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
