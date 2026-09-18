//! The Six lexer: source text to a flat `Vec<Token>`.
//!
//! Six is whitespace-sensitive in several places, so the lexer records, for
//! every token, its line/column and whether whitespace preceded it. Blank lines
//! and comments collapse away; a single `Newline` token marks each logical line
//! break so the parser can treat newlines as statement separators.

use crate::error::{Result, SixError};
use crate::token::{Tok, Token};

pub fn lex(source: &str) -> Result<Vec<Token>> {
    Lexer::new(source).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    /// True when whitespace / newline / start-of-input precedes the next token.
    pending_space: bool,
    tokens: Vec<Token>,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            pending_space: true,
            tokens: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if let Some(c) = c {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        c
    }

    fn push(&mut self, tok: Tok, line: usize, col: usize) {
        let space_before = self.pending_space;
        self.tokens.push(Token::new(tok, line, col, space_before));
        self.pending_space = false;
    }

    fn run(mut self) -> Result<Vec<Token>> {
        loop {
            // Skip spaces, tabs, carriage returns and comments (but not newlines).
            loop {
                match self.peek() {
                    Some(' ') | Some('\t') | Some('\r') => {
                        self.advance();
                        self.pending_space = true;
                    }
                    Some('#') => {
                        if self.peek_at(1) == Some('#') {
                            // Block comment: `##` ... `##`.
                            self.advance();
                            self.advance();
                            loop {
                                match self.peek() {
                                    None => break,
                                    Some('#') if self.peek_at(1) == Some('#') => {
                                        self.advance();
                                        self.advance();
                                        break;
                                    }
                                    _ => {
                                        self.advance();
                                    }
                                }
                            }
                        } else {
                            // Line comment to end of line.
                            while let Some(c) = self.peek() {
                                if c == '\n' {
                                    break;
                                }
                                self.advance();
                            }
                        }
                        self.pending_space = true;
                    }
                    _ => break,
                }
            }

            let line = self.line;
            let col = self.col;
            let c = match self.peek() {
                Some(c) => c,
                None => {
                    self.push(Tok::Eof, line, col);
                    break;
                }
            };

            match c {
                '\n' => {
                    self.advance();
                    // Collapse consecutive newlines: don't emit a Newline if the
                    // previous token is already a Newline (or nothing yet).
                    let emit = matches!(self.tokens.last(), Some(t) if t.tok != Tok::Newline);
                    if emit {
                        self.tokens.push(Token::new(Tok::Newline, line, col, true));
                    }
                    self.pending_space = true;
                }
                '"' => self.lex_text(line, col)?,
                '0'..='9' => self.lex_number(line, col)?,
                // A lone `_` (not starting an identifier) is the floor operator.
                '_' if !matches!(self.peek_at(1), Some(n) if is_ident_continue(n)) => {
                    self.lex_symbol(line, col)?
                }
                c if is_ident_start(c) => self.lex_ident(line, col),
                _ => self.lex_symbol(line, col)?,
            }
        }
        Ok(self.tokens)
    }

    fn lex_text(&mut self, line: usize, col: usize) -> Result<()> {
        self.advance(); // opening quote
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err(SixError::at(line, "unterminated text literal")),
                Some('"') => break,
                Some('\\') => {
                    // Minimal escape support keeps text a plain value type.
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some(other) => {
                            s.push('\\');
                            s.push(other);
                        }
                        None => return Err(SixError::at(line, "unterminated text literal")),
                    }
                }
                Some(other) => s.push(other),
            }
        }
        self.push(Tok::Text(s), line, col);
        Ok(())
    }

    fn lex_number(&mut self, line: usize, col: usize) -> Result<()> {
        let mut raw = String::new();
        // Integer part: digits and thousands separators.
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == ',' {
                raw.push(c);
                self.advance();
            } else {
                break;
            }
        }
        validate_thousands(&raw, line)?;

        let mut digits: String = raw.chars().filter(|c| *c != ',').collect();

        // Optional fractional part (only when a digit follows the dot, so that
        // `5.text` stays `5` `.text` and a trailing `.` is a terminator).
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(d) if d.is_ascii_digit()) {
            self.advance(); // dot
            digits.push('.');
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    self.advance();
                } else if c == ',' {
                    return Err(SixError::at(line, "malformed numeric separator: commas are not allowed in a fractional part"));
                } else {
                    break;
                }
            }
        }

        match digits.parse::<f64>() {
            Ok(n) => {
                self.push(Tok::Number(n), line, col);
                Ok(())
            }
            Err(_) => Err(SixError::at(line, format!("malformed number literal: {}", raw))),
        }
    }

    fn lex_ident(&mut self, line: usize, col: usize) {
        let mut name = String::new();
        // First char.
        name.push(self.advance().unwrap());
        loop {
            match self.peek() {
                Some(c) if is_ident_continue(c) => {
                    name.push(c);
                    self.advance();
                }
                // A hyphen continues the identifier only when directly followed
                // by another identifier char, so `build-first` is one name while
                // `n - 1` is subtraction.
                Some('-') if matches!(self.peek_at(1), Some(n) if is_ident_start(n)) => {
                    name.push('-');
                    self.advance();
                }
                // A trailing `?` marks a predicate name (`even?`, `has?`).
                Some('?') => {
                    name.push('?');
                    self.advance();
                    break;
                }
                _ => break,
            }
        }

        let tok = match name.as_str() {
            "true" => Tok::True,
            "false" => Tok::False,
            "nil" => Tok::Nil,
            "if" => Tok::If,
            "any" => Tok::Any,
            "else" => Tok::Else,
            "and" => Tok::And,
            "or" => Tok::Or,
            "not" => Tok::Not,
            _ => Tok::Ident(name),
        };
        self.push(tok, line, col);
    }

    fn lex_symbol(&mut self, line: usize, col: usize) -> Result<()> {
        let c = self.advance().unwrap();
        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            '$' => Tok::Dollar,
            '@' => Tok::At,
            '~' => Tok::Tilde,
            '^' => Tok::Caret,
            '_' => Tok::Underscore,
            '+' => {
                if self.peek() == Some('+') {
                    self.advance();
                    Tok::PlusPlus
                } else {
                    Tok::Plus
                }
            }
            '-' => {
                if self.peek() == Some('-') {
                    self.advance();
                    Tok::MinusMinus
                } else {
                    Tok::Minus
                }
            }
            '*' => {
                if self.peek() == Some('*') {
                    self.advance();
                    Tok::StarStar
                } else {
                    Tok::Star
                }
            }
            '/' => {
                if self.peek() == Some('/') {
                    self.advance();
                    Tok::SlashSlash
                } else {
                    Tok::Slash
                }
            }
            '%' => Tok::Percent,
            ':' => {
                if self.peek() == Some(':') {
                    self.advance();
                    Tok::ColonColon
                } else {
                    Tok::Colon
                }
            }
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Tok::EqEq
                } else {
                    Tok::Assign
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Tok::NotEq
                } else {
                    Tok::Not
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Tok::Le
                } else {
                    Tok::Lt
                }
            }
            '>' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Tok::FatArrow
                } else if self.peek() == Some('=') {
                    self.advance();
                    Tok::Ge
                } else {
                    Tok::Gt
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Tok::And
                } else {
                    return Err(SixError::at(line, "unexpected character '&' (did you mean '&&'?)"));
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    Tok::Or
                } else {
                    return Err(SixError::at(line, "unexpected character '|' (did you mean '||'?)"));
                }
            }
            '.' => {
                if self.peek() == Some('.') {
                    self.advance();
                    Tok::Empty
                } else {
                    Tok::Dot
                }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.advance();
                    Tok::QElse
                } else if self.peek() == Some('*') {
                    self.advance();
                    Tok::QAny
                } else {
                    Tok::QIf
                }
            }
            other => {
                return Err(SixError::at(line, format!("unexpected character '{}'", other)));
            }
        };
        self.push(tok, line, col);
        Ok(())
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// Validate that an integer literal uses proper three-digit thousands grouping
/// (spec §3). Only runs when the literal actually contains a comma.
fn validate_thousands(raw: &str, line: usize) -> Result<()> {
    if !raw.contains(',') {
        return Ok(());
    }
    let parts: Vec<&str> = raw.split(',').collect();
    let bad = |line| Err(SixError::at(line, format!("malformed numeric separator: {}", raw)));
    if parts.len() < 2 {
        return bad(line);
    }
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return bad(line);
        }
        if i == 0 {
            if part.len() < 1 || part.len() > 3 {
                return bad(line);
            }
        } else if part.len() != 3 {
            return bad(line);
        }
    }
    Ok(())
}
