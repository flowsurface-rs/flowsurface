use crate::CompileLimits;
use crate::diagnostics::{Diagnostic, Span};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum TokenKind {
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
pub(super) struct Token {
    pub(super) kind: TokenKind,
    pub(super) span: Span,
}

pub(super) struct Lexer {
    source: Vec<char>,
    path: PathBuf,
    index: usize,
    diagnostics: Vec<Diagnostic>,
    max_tokens: usize,
    token_limit_reported: bool,
}

impl Lexer {
    fn new(source: &str, path: PathBuf, limits: CompileLimits) -> Self {
        Self {
            source: source.chars().collect(),
            path,
            index: 0,
            diagnostics: Vec::new(),
            max_tokens: limits.max_tokens,
            token_limit_reported: false,
        }
    }

    fn push_token(&mut self, tokens: &mut Vec<Token>, token: Token) {
        if matches!(token.kind, TokenKind::End) || tokens.len() < self.max_tokens {
            tokens.push(token);
        } else if !self.token_limit_reported {
            self.token_limit_reported = true;
            self.diagnostics.push(Diagnostic::error(
                self.path.clone(),
                token.span,
                format!("source exceeds compile limit of {} tokens", self.max_tokens),
            ));
            self.index = self.source.len();
        }
    }

    fn push_single(&mut self, tokens: &mut Vec<Token>, kind: TokenKind) {
        let token = self.single(kind);
        self.push_token(tokens, token);
    }

    fn push_double(&mut self, tokens: &mut Vec<Token>, kind: TokenKind) {
        let token = self.double(kind);
        self.push_token(tokens, token);
    }

    fn push_string(&mut self, tokens: &mut Vec<Token>) {
        let token = self.string_token();
        self.push_token(tokens, token);
    }

    fn push_number(&mut self, tokens: &mut Vec<Token>) {
        let token = self.number_token();
        self.push_token(tokens, token);
    }

    fn push_identifier(&mut self, tokens: &mut Vec<Token>) {
        let token = self.identifier_token();
        self.push_token(tokens, token);
    }

    fn lex(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while self.index < self.source.len() {
            let start = self.index;
            match self.source[self.index] {
                ' ' | '\t' | '\r' => self.index += 1,
                '\n' => {
                    self.index += 1;
                    self.push_token(
                        &mut tokens,
                        Token {
                            kind: TokenKind::Newline,
                            span: Span::new(start, self.index),
                        },
                    );
                }
                ';' => {
                    self.index += 1;
                    self.push_token(
                        &mut tokens,
                        Token {
                            kind: TokenKind::Semicolon,
                            span: Span::new(start, self.index),
                        },
                    );
                }
                '/' if self.peek(1) == Some('/') => {
                    self.index += 2;
                    while self.index < self.source.len() && self.source[self.index] != '\n' {
                        self.index += 1;
                    }
                }
                '"' => self.push_string(&mut tokens),
                character if character.is_ascii_digit() => self.push_number(&mut tokens),
                character if character.is_ascii_alphabetic() || character == '_' => {
                    self.push_identifier(&mut tokens)
                }
                '+' => self.push_single(&mut tokens, TokenKind::Plus),
                '-' => self.push_single(&mut tokens, TokenKind::Minus),
                '*' => self.push_single(&mut tokens, TokenKind::Star),
                '/' => self.push_single(&mut tokens, TokenKind::Slash),
                '%' => self.push_single(&mut tokens, TokenKind::Percent),
                '(' => self.push_single(&mut tokens, TokenKind::LeftParen),
                ')' => self.push_single(&mut tokens, TokenKind::RightParen),
                '[' => self.push_single(&mut tokens, TokenKind::LeftBracket),
                ']' => self.push_single(&mut tokens, TokenKind::RightBracket),
                '{' => self.push_single(&mut tokens, TokenKind::LeftBrace),
                '}' => self.push_single(&mut tokens, TokenKind::RightBrace),
                ',' => self.push_single(&mut tokens, TokenKind::Comma),
                '.' => self.push_single(&mut tokens, TokenKind::Dot),
                '!' if self.peek(1) == Some('=') => {
                    self.push_double(&mut tokens, TokenKind::BangEqual)
                }
                '!' => self.push_single(&mut tokens, TokenKind::Bang),
                '=' if self.peek(1) == Some('=') => {
                    self.push_double(&mut tokens, TokenKind::EqualEqual)
                }
                '=' if self.peek(1) == Some('>') => self.push_double(&mut tokens, TokenKind::Arrow),
                '=' => self.push_single(&mut tokens, TokenKind::Equal),
                ':' if self.peek(1) == Some('=') => {
                    self.push_double(&mut tokens, TokenKind::ColonEqual)
                }
                ':' => self.push_single(&mut tokens, TokenKind::Colon),
                '<' if self.peek(1) == Some('=') => {
                    self.push_double(&mut tokens, TokenKind::LessEqual)
                }
                '<' => self.push_single(&mut tokens, TokenKind::Less),
                '>' if self.peek(1) == Some('=') => {
                    self.push_double(&mut tokens, TokenKind::GreaterEqual)
                }
                '>' => self.push_single(&mut tokens, TokenKind::Greater),
                '&' if self.peek(1) == Some('&') => {
                    self.push_double(&mut tokens, TokenKind::AndAnd)
                }
                '|' if self.peek(1) == Some('|') => self.push_double(&mut tokens, TokenKind::OrOr),
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
        self.push_token(
            &mut tokens,
            Token {
                kind: TokenKind::End,
                span: Span::new(self.index, self.index),
            },
        );
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

pub(super) struct LexOutput {
    pub(super) tokens: Vec<Token>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

pub(super) fn lex(source: &str, path: &Path, limits: CompileLimits) -> LexOutput {
    let mut lexer = Lexer::new(source, path.to_path_buf(), limits);
    let tokens = lexer.lex();
    LexOutput {
        tokens,
        diagnostics: lexer.diagnostics,
    }
}
