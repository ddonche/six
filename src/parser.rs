//! The Six parser: `Vec<Token>` to a `Vec<Stmt>` program.
//!
//! Notable, Six-specific parsing rules:
//!
//! * Whitespace separates function-application arguments, so `add 2 3` is a
//!   call while `i + 1` is arithmetic (operators break application).
//! * `>>` plays three roles, disambiguated by position: a statement-initial
//!   `>>` starts a function declaration, a same-line `>>` after an expression is
//!   transformation flow, and inside a conditional it separates an arm's
//!   condition from its body. Flow and dot-flow are desugared into `Call`s here.
//! * A `.` is a block terminator when it stands at statement position, and
//!   dot-flow when it hugs the preceding operand (no space before it).
//! * Group literals: when `[` is followed by a newline each subsequent line is
//!   one member (a full expression); otherwise members are whitespace-separated
//!   atoms. This resolves the `[name person age person]` ambiguity (spec §10).

use crate::ast::*;
use crate::error::{Result, SixError};
use crate::token::{Tok, Token};

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>> {
    let mut parser = Parser { tokens, pos: 0 };
    parser.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    // --- cursor helpers -----------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_tok(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_at(&self, offset: usize) -> &Tok {
        self.tokens
            .get(self.pos + offset)
            .map(|t| &t.tok)
            .unwrap_or(&Tok::Eof)
    }

    fn line(&self) -> usize {
        self.tokens[self.pos].line
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn check(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.check(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> Result<Token> {
        if self.check(tok) {
            Ok(self.advance())
        } else {
            Err(SixError::at(
                self.line(),
                format!("expected {} but found {}", what, describe(self.peek())),
            ))
        }
    }

    fn skip_newlines(&mut self) {
        while self.check(&Tok::Newline) {
            self.advance();
        }
    }

    // --- program ------------------------------------------------------------

    fn parse_program(&mut self) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Tok::Eof) {
                break;
            }
            if self.check(&Tok::Dot) {
                return Err(SixError::at(self.line(), "stray block terminator '.'"));
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    // --- statements ---------------------------------------------------------

    fn parse_stmt(&mut self) -> Result<Stmt> {
        if self.check(&Tok::FatArrow) {
            return self.parse_func_decl();
        }

        let line = self.line();
        let expr = self.parse_expr()?;

        match self.peek() {
            Tok::Colon | Tok::ColonColon => {
                let deep = self.check(&Tok::ColonColon);
                self.advance();
                let name = match expr {
                    Expr::Var { name, .. } => name,
                    _ => {
                        return Err(SixError::at(line, "the left side of a binding must be a name"));
                    }
                };
                self.skip_newlines();
                let value = self.parse_expr()?;
                let immutable = is_immutable_name(&name);
                Ok(Stmt::Bind { name, value, deep, immutable, line })
            }
            Tok::Assign => {
                self.advance();
                if !is_lvalue(&expr) {
                    return Err(SixError::at(line, "the left side of '=' must be a name or an index"));
                }
                self.skip_newlines();
                let value = self.parse_expr()?;
                Ok(Stmt::Assign { target: expr, value, line })
            }
            _ => Ok(Stmt::Expr(expr)),
        }
    }

    fn parse_func_decl(&mut self) -> Result<Stmt> {
        let line = self.line();
        self.expect(&Tok::FatArrow, "'>>' to start a function declaration")?;

        let name = match self.advance().tok {
            Tok::Ident(n) => n,
            other => {
                return Err(SixError::at(line, format!("expected a function name, found {}", describe(&other))));
            }
        };

        let mut params = Vec::new();
        if self.eat(&Tok::Colon) {
            self.expect(&Tok::LBracket, "'[' to start the parameter list")?;
            while !self.check(&Tok::RBracket) {
                self.skip_newlines();
                match self.advance().tok {
                    Tok::Ident(p) => params.push(p),
                    Tok::RBracket => break,
                    other => {
                        return Err(SixError::at(line, format!("expected a parameter name, found {}", describe(&other))));
                    }
                }
                self.skip_newlines();
            }
            self.expect(&Tok::RBracket, "']' to close the parameter list")?;
        }

        let arrow = self.expect(&Tok::FatArrow, "'>>' before the function body")?;

        let body = if self.same_line_body(arrow.line) {
            // Single-expression function: `>> add: [x y] >> x + y` (no terminator).
            vec![Stmt::Expr(self.parse_expr()?)]
        } else {
            self.parse_block()?
        };

        let def = std::rc::Rc::new(FuncDef { name: name.clone(), params, body, line });
        Ok(Stmt::Func { def, immutable: is_immutable_name(&name), line })
    }

    /// Parse statements until the matching `.` terminator, which is consumed.
    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Tok::Dot) {
                self.advance();
                break;
            }
            if self.check(&Tok::Eof) {
                return Err(SixError::at(self.line(), "unterminated block: expected '.'"));
            }
            stmts.push(self.parse_stmt()?);
        }
        Ok(stmts)
    }

    /// True when the token after an arrow sits on the same line and can begin a
    /// consequent expression (i.e. the body is written inline).
    fn same_line_body(&self, arrow_line: usize) -> bool {
        let t = self.peek_tok();
        t.line == arrow_line && !matches!(t.tok, Tok::Newline | Tok::Dot | Tok::Eof)
    }

    // --- expressions --------------------------------------------------------

    /// Full expression, including `>>` flow chains.
    fn parse_expr(&mut self) -> Result<Expr> {
        let mut left = self.parse_binary()?;
        // Flow only continues when `>>` immediately follows on the same line.
        while self.check(&Tok::FatArrow) {
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let stage = self.parse_application()?;
            left = apply_flow(left, stage, line);
        }
        Ok(left)
    }

    // Binary-operator precedence ladder (lowest to highest).
    fn parse_binary(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.check(&Tok::Or) {
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_and()?;
            left = Expr::Binary { op: BinOp::Or, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_equality()?;
        while self.check(&Tok::And) {
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_equality()?;
            left = Expr::Binary { op: BinOp::And, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                _ => break,
            };
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_comparison()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_additive()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_multiplicative()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let right = self.parse_unary()?;
            left = Expr::Binary { op, left: Box::new(left), right: Box::new(right), line };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let (op, line) = match self.peek() {
            Tok::Not => (UnOp::Not, self.line()),
            Tok::Minus => (UnOp::Neg, self.line()),
            _ => return self.parse_application(),
        };
        self.advance();
        let expr = self.parse_unary()?;
        Ok(Expr::Unary { op, expr: Box::new(expr), line })
    }

    /// Function application: a callee followed by whitespace-separated argument
    /// atoms. Dot-flow (`recv.name args`) is handled here so the trailing
    /// arguments attach to the resulting call, not to its result.
    fn parse_application(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        // Immediate postfix indexing on the primary (e.g. `arr[0]`).
        expr = self.parse_index_suffix(expr)?;

        loop {
            if self.is_dot_flow() {
                let line = self.line();
                self.advance(); // '.'
                let name = self.expect_ident("a name after '.'")?;
                let mut args = vec![Arg::Normal(expr)];
                while self.can_start_arg() {
                    args.push(self.parse_arg()?);
                }
                expr = Expr::Call { callee: Box::new(Expr::Var { name, line }), args, line };
            } else if self.can_start_arg() {
                let line = self.line();
                let mut args = Vec::new();
                while self.can_start_arg() {
                    args.push(self.parse_arg()?);
                }
                expr = Expr::Call { callee: Box::new(expr), args, line };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// A single argument atom: a primary with postfix index/dot-flow, but no
    /// further application (so `f x.size y` reads as `f(x.size, y)`).
    fn parse_arg(&mut self) -> Result<Arg> {
        // Splat: `<group>` opens a Group into separate arguments. The body is
        // parsed as a tight application so the closing `>` is never mistaken for
        // a greater-than operator (comparisons are written with spaces).
        if self.check(&Tok::Lt) && !self.peek_tok_at(1).space_before {
            self.advance(); // '<'
            let expr = self.parse_application()?;
            self.expect(&Tok::Gt, "'>' to close a splat argument")?;
            return Ok(Arg::Splat(expr));
        }

        let mut expr = self.parse_primary()?;
        loop {
            if self.is_index_suffix() {
                expr = self.parse_one_index(expr)?;
            } else if self.is_dot_flow() {
                let line = self.line();
                self.advance();
                let name = self.expect_ident("a name after '.'")?;
                expr = Expr::Call {
                    callee: Box::new(Expr::Var { name, line }),
                    args: vec![Arg::Normal(expr)],
                    line,
                };
            } else {
                break;
            }
        }
        Ok(Arg::Normal(expr))
    }

    fn parse_index_suffix(&mut self, mut expr: Expr) -> Result<Expr> {
        while self.is_index_suffix() {
            expr = self.parse_one_index(expr)?;
        }
        Ok(expr)
    }

    fn parse_one_index(&mut self, base: Expr) -> Result<Expr> {
        let line = self.line();
        self.advance(); // '['
        self.skip_newlines();
        let index = self.parse_expr()?;
        self.skip_newlines();
        self.expect(&Tok::RBracket, "']' to close an index")?;
        Ok(Expr::Index { base: Box::new(base), index: Box::new(index), line })
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Number(n) => {
                self.advance();
                Ok(Expr::Number(n))
            }
            Tok::Text(s) => {
                self.advance();
                Ok(Expr::Text(s))
            }
            Tok::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Tok::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Tok::Nil | Tok::Empty => {
                self.advance();
                Ok(Expr::Empty)
            }
            Tok::Dollar => {
                self.advance();
                Ok(Expr::Last)
            }
            Tok::Ident(name) => {
                self.advance();
                Ok(Expr::Var { name, line })
            }
            Tok::LParen => {
                self.advance();
                self.skip_newlines();
                let expr = self.parse_expr()?;
                self.skip_newlines();
                self.expect(&Tok::RParen, "')'")?;
                Ok(expr)
            }
            Tok::LBracket => self.parse_group(),
            Tok::If | Tok::QIf | Tok::QAny => self.parse_conditional(),
            other => Err(SixError::at(line, format!("unexpected {}", describe(&other)))),
        }
    }

    fn parse_group(&mut self) -> Result<Expr> {
        let line = self.line();
        self.expect(&Tok::LBracket, "'['")?;
        let mut members = Vec::new();

        // Multi-line mode: `[` immediately followed by a newline. Each line is
        // one member (a full expression). Otherwise inline mode: members are
        // whitespace-separated atoms.
        let multiline = self.check(&Tok::Newline);

        loop {
            self.skip_newlines();
            if self.check(&Tok::RBracket) {
                break;
            }
            if self.check(&Tok::Eof) {
                return Err(SixError::at(line, "unterminated Group literal: expected ']'"));
            }
            if multiline {
                members.push(self.parse_expr()?);
            } else {
                members.push(self.parse_group_atom()?);
            }
        }
        self.expect(&Tok::RBracket, "']' to close a Group")?;
        Ok(Expr::Group { members, line })
    }

    /// An inline Group member: a single atom (primary + postfix), never a bare
    /// application. Compound calls must be parenthesised inline.
    fn parse_group_atom(&mut self) -> Result<Expr> {
        // Reuse the argument atom logic but return the bare Expr.
        match self.parse_arg()? {
            Arg::Normal(e) => Ok(e),
            Arg::Splat(_) => Err(SixError::at(self.line(), "splat is not allowed as a Group member")),
        }
    }

    /// Parse `if` / `if any` / `?` / `?*` conditionals. Consumes the closing `.`.
    fn parse_conditional(&mut self) -> Result<Expr> {
        let line = self.line();
        let any = match self.peek() {
            Tok::If => {
                self.advance();
                self.eat(&Tok::Any)
            }
            Tok::QIf => {
                self.advance();
                false
            }
            Tok::QAny => {
                self.advance();
                true
            }
            _ => unreachable!(),
        };

        let mut arms = Vec::new();
        let mut else_body = None;

        loop {
            self.skip_newlines();
            if self.check(&Tok::Dot) {
                self.advance();
                break;
            }
            if self.check(&Tok::Eof) {
                return Err(SixError::at(line, "unterminated conditional: expected '.'"));
            }

            // else / ?? arm.
            if self.check(&Tok::Else) || self.check(&Tok::QElse) {
                self.advance();
                let arrow = self.expect(&Tok::FatArrow, "'>>' after 'else'")?;
                else_body = Some(self.parse_arm_body(arrow.line)?);
                continue;
            }

            // condition >> body
            let cond = self.parse_binary()?;
            let arrow = self.expect(&Tok::FatArrow, "'>>' after a condition")?;
            let body = self.parse_arm_body(arrow.line)?;
            arms.push(Arm { cond, body });
        }

        Ok(Expr::If { any, arms, else_body, line })
    }

    /// Parse an arm's consequent: an inline single statement when it shares the
    /// arrow's line, otherwise a sequence of statements up to the next arm
    /// header or the conditional's terminator.
    fn parse_arm_body(&mut self, arrow_line: usize) -> Result<Vec<Stmt>> {
        if self.same_line_body(arrow_line) {
            return Ok(vec![self.parse_stmt()?]);
        }
        let mut body = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Tok::Dot) || self.check(&Tok::Eof) {
                break;
            }
            if self.line_is_arm_header() {
                break;
            }
            body.push(self.parse_stmt()?);
        }
        Ok(body)
    }

    /// Heuristic: does the current logical line begin a new conditional arm?
    /// An arm header carries a top-level `>>` (depth 0) before its newline and
    /// does not itself begin a nested conditional or declaration.
    fn line_is_arm_header(&self) -> bool {
        match self.peek() {
            Tok::Else | Tok::QElse => return true,
            // A nested `if`/flow-declaration statement is a body statement.
            Tok::If | Tok::QIf | Tok::QAny | Tok::FatArrow => return false,
            _ => {}
        }
        let mut depth = 0i32;
        let mut i = self.pos;
        while let Some(t) = self.tokens.get(i) {
            match t.tok {
                Tok::Newline | Tok::Eof => return false,
                Tok::LParen | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBracket => depth -= 1,
                Tok::FatArrow if depth == 0 => return true,
                _ => {}
            }
            i += 1;
        }
        false
    }

    // --- small predicates ---------------------------------------------------

    fn peek_tok_at(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.pos + offset)
            .unwrap_or_else(|| self.tokens.last().unwrap())
    }

    /// An index suffix is a `[` that hugs the preceding operand (no space).
    fn is_index_suffix(&self) -> bool {
        self.check(&Tok::LBracket) && !self.peek_tok().space_before
    }

    /// Dot-flow is a `.` that hugs the preceding operand and is followed by a
    /// name (distinguishing it from a standalone `.` block terminator).
    fn is_dot_flow(&self) -> bool {
        self.check(&Tok::Dot)
            && !self.peek_tok().space_before
            && matches!(self.peek_at(1), Tok::Ident(_))
    }

    fn expect_ident(&mut self, what: &str) -> Result<String> {
        match self.advance().tok {
            Tok::Ident(n) => Ok(n),
            other => Err(SixError::at(self.line(), format!("expected {}, found {}", what, describe(&other)))),
        }
    }

    /// Can the current token begin an application argument? Arguments never
    /// start with an infix operator, and must share the previous token's line.
    fn can_start_arg(&self) -> bool {
        let t = self.peek_tok();
        match &t.tok {
            Tok::Number(_)
            | Tok::Text(_)
            | Tok::True
            | Tok::False
            | Tok::Nil
            | Tok::Empty
            | Tok::Dollar
            | Tok::Ident(_)
            | Tok::LParen
            | Tok::If
            | Tok::QIf
            | Tok::QAny => true,
            // A Group literal argument requires a space before `[`; without a
            // space it is an index suffix on the previous atom.
            Tok::LBracket => t.space_before,
            // A splat `<...>` — `<` with no space before the inner expression.
            Tok::Lt => !self.peek_tok_at(1).space_before,
            _ => false,
        }
    }
}

/// Desugar `value >> stage` into a call whose first argument is `value`.
fn apply_flow(value: Expr, stage: Expr, line: usize) -> Expr {
    match stage {
        Expr::Call { callee, mut args, line: cline } => {
            args.insert(0, Arg::Normal(value));
            Expr::Call { callee, args, line: cline }
        }
        other => Expr::Call { callee: Box::new(other), args: vec![Arg::Normal(value)], line },
    }
}

fn is_lvalue(expr: &Expr) -> bool {
    matches!(expr, Expr::Var { .. } | Expr::Index { .. })
}

/// A name is immutable when its first alphabetic character is uppercase.
pub fn is_immutable_name(name: &str) -> bool {
    name.chars()
        .find(|c| c.is_alphabetic())
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Number(n) => format!("number {}", n),
        Tok::Text(_) => "text".to_string(),
        Tok::Ident(n) => format!("name '{}'", n),
        Tok::True => "'true'".to_string(),
        Tok::False => "'false'".to_string(),
        Tok::Nil => "'nil'".to_string(),
        Tok::Empty => "'..'".to_string(),
        Tok::Dollar => "'$'".to_string(),
        Tok::Colon => "':'".to_string(),
        Tok::ColonColon => "'::'".to_string(),
        Tok::Assign => "'='".to_string(),
        Tok::LBracket => "'['".to_string(),
        Tok::RBracket => "']'".to_string(),
        Tok::LParen => "'('".to_string(),
        Tok::RParen => "')'".to_string(),
        Tok::Lt => "'<'".to_string(),
        Tok::Gt => "'>'".to_string(),
        Tok::Le => "'<='".to_string(),
        Tok::Ge => "'>='".to_string(),
        Tok::EqEq => "'=='".to_string(),
        Tok::NotEq => "'!='".to_string(),
        Tok::Plus => "'+'".to_string(),
        Tok::Minus => "'-'".to_string(),
        Tok::Star => "'*'".to_string(),
        Tok::Slash => "'/'".to_string(),
        Tok::Percent => "'%'".to_string(),
        Tok::And => "'and'".to_string(),
        Tok::Or => "'or'".to_string(),
        Tok::Not => "'not'".to_string(),
        Tok::FatArrow => "'>>'".to_string(),
        Tok::Dot => "'.'".to_string(),
        Tok::If => "'if'".to_string(),
        Tok::Any => "'any'".to_string(),
        Tok::Else => "'else'".to_string(),
        Tok::QIf => "'?'".to_string(),
        Tok::QElse => "'??'".to_string(),
        Tok::QAny => "'?*'".to_string(),
        Tok::Newline => "end of line".to_string(),
        Tok::Eof => "end of input".to_string(),
    }
}
