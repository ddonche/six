//! The Six parser: `Vec<Token>` to a `Vec<Stmt>` program.
//!
//! The canonical Six surface syntax (see `examples/tiny_inventory.six`):
//!
//! * **Calls are parenthesised**: `find(items name i)`, `out(output "")`. Arguments
//!   are separated by whitespace or newlines inside the parentheses; there is no
//!   bare `f x y` application. A bare name is the function value itself.
//! * **Declarations start with `:`**: `:name(params)` then a body terminated by
//!   `.`. A statement-initial `:` is a declaration; an infix `:` is a binding.
//! * **`>>`** is transformation flow (and the conditional arm separator). The
//!   flowing value becomes the first argument of the next call stage.
//! * **Postfix math**: `x~` `x~2` `x^` `x_` `x**` `x**y` `x//`, and the mutating
//!   `x++` / `x--`.
//! * Inside parentheses, newlines are insignificant, so an expression may
//!   continue onto the next line (including a leading `+`).

use crate::ast::*;
use crate::error::{Result, SixError};
use crate::token::{Tok, Token};

pub fn parse(tokens: Vec<Token>) -> Result<Vec<Stmt>> {
    let mut parser = Parser { tokens, pos: 0, paren_depth: 0 };
    parser.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Depth of enclosing `(` / `[`; while > 0, newlines are insignificant.
    paren_depth: usize,
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
        self.tokens.get(self.pos + offset).map(|t| &t.tok).unwrap_or(&Tok::Eof)
    }

    fn peek_tok_at(&self, offset: usize) -> &Token {
        self.tokens.get(self.pos + offset).unwrap_or_else(|| self.tokens.last().unwrap())
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

    /// Skip newlines only when inside parentheses/brackets, where they are
    /// insignificant. This is what lets an expression continue across lines.
    fn skip_insignificant_newlines(&mut self) {
        if self.paren_depth > 0 {
            self.skip_newlines();
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
        // `@name` imports a module.
        if self.check(&Tok::At) {
            let line = self.line();
            self.advance();
            let name = self.expect_ident("a module name after '@'")?;
            return Ok(Stmt::Import { name, line });
        }
        // A statement-initial `:` introduces a function declaration.
        if self.check(&Tok::Colon) {
            return self.parse_func_decl();
        }

        let line = self.line();
        let expr = self.parse_expr()?;

        match self.peek() {
            // `x++` / `x--` desugar to an increment/decrement assignment.
            Tok::PlusPlus | Tok::MinusMinus => {
                let inc = self.check(&Tok::PlusPlus);
                self.advance();
                if !is_lvalue(&expr) {
                    return Err(SixError::at(line, "'++' and '--' need a name or an index"));
                }
                let op = if inc { BinOp::Add } else { BinOp::Sub };
                let value = Expr::Binary {
                    op,
                    left: Box::new(expr.clone()),
                    right: Box::new(Expr::Number(1.0)),
                    line,
                };
                Ok(Stmt::Assign { target: expr, value, line })
            }
            Tok::Colon | Tok::ColonColon => {
                let deep = self.check(&Tok::ColonColon);
                self.advance();
                let name = match expr {
                    Expr::Var { name, .. } => name,
                    _ => return Err(SixError::at(line, "the left side of a binding must be a name")),
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

    /// `:name(params)` NEWLINE body `.`
    fn parse_func_decl(&mut self) -> Result<Stmt> {
        let line = self.line();
        self.expect(&Tok::Colon, "':' to start a function declaration")?;
        let name = self.expect_ident("a function name")?;

        self.paren_depth += 1;
        self.expect(&Tok::LParen, "'(' to start the parameter list")?;
        let mut params = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Tok::RParen) {
                break;
            }
            params.push(self.expect_ident("a parameter name")?);
        }
        self.expect(&Tok::RParen, "')' to close the parameter list")?;
        self.paren_depth -= 1;

        let body = self.parse_block()?;
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

    // --- expressions --------------------------------------------------------

    /// Full expression, including `>>` flow chains.
    fn parse_expr(&mut self) -> Result<Expr> {
        let mut left = self.parse_binary()?;
        // Flow continues only when `>>` immediately follows on the same line.
        while self.check(&Tok::FatArrow) {
            let line = self.line();
            self.advance();
            self.skip_newlines();
            let stage = self.parse_postfix()?;
            left = apply_flow(left, stage, line);
        }
        Ok(left)
    }

    fn parse_binary(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_insignificant_newlines();
            if !self.check(&Tok::Or) {
                break;
            }
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
        loop {
            self.skip_insignificant_newlines();
            if !self.check(&Tok::And) {
                break;
            }
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
            self.skip_insignificant_newlines();
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
            self.skip_insignificant_newlines();
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
            self.skip_insignificant_newlines();
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
            self.skip_insignificant_newlines();
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
            _ => return self.parse_postfix(),
        };
        self.advance();
        let expr = self.parse_unary()?;
        Ok(Expr::Unary { op, expr: Box::new(expr), line })
    }

    /// A primary followed by postfix suffixes: calls `(...)`, indexes `[...]`,
    /// dot-flow `.name`, and numeric postfix operators. Each suffix must hug the
    /// operand (no space before it).
    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            let t = self.peek_tok();
            if t.space_before {
                break;
            }
            match &t.tok {
                Tok::LParen => {
                    let line = self.line();
                    let args = self.parse_call_args()?;
                    expr = Expr::Call { callee: Box::new(expr), args, line };
                }
                Tok::LBracket => {
                    let line = self.line();
                    self.paren_depth += 1;
                    self.advance();
                    self.skip_newlines();
                    let index = self.parse_expr()?;
                    self.skip_newlines();
                    self.expect(&Tok::RBracket, "']' to close an index")?;
                    self.paren_depth -= 1;
                    expr = Expr::Index { base: Box::new(expr), index: Box::new(index), line };
                }
                Tok::Dot if matches!(self.peek_at(1), Tok::Ident(_)) => {
                    let line = self.line();
                    self.advance(); // '.'
                    let name = self.expect_ident("a name after '.'")?;
                    // Dot-flow: `recv.name` == `name(recv)`, optionally with a
                    // parenthesised argument list that receives `recv` first.
                    let mut args = vec![Arg::Normal(expr)];
                    if self.check(&Tok::LParen) && !self.peek_tok().space_before {
                        for a in self.parse_call_args()? {
                            args.push(a);
                        }
                    }
                    expr = Expr::Call { callee: Box::new(Expr::Var { name, line }), args, line };
                }
                Tok::Tilde => {
                    let line = self.line();
                    self.advance();
                    // Optional adjacent decimal count.
                    let arg = if !self.peek_tok().space_before {
                        if let Tok::Number(_) = self.peek() {
                            Some(Box::new(self.parse_primary()?))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    expr = Expr::Postfix { op: PostOp::Round, expr: Box::new(expr), arg, line };
                }
                Tok::Caret => {
                    let line = self.line();
                    self.advance();
                    expr = Expr::Postfix { op: PostOp::Ceil, expr: Box::new(expr), arg: None, line };
                }
                Tok::Underscore => {
                    let line = self.line();
                    self.advance();
                    expr = Expr::Postfix { op: PostOp::Floor, expr: Box::new(expr), arg: None, line };
                }
                Tok::SlashSlash => {
                    let line = self.line();
                    self.advance();
                    expr = Expr::Postfix { op: PostOp::Sqrt, expr: Box::new(expr), arg: None, line };
                }
                Tok::StarStar => {
                    let line = self.line();
                    self.advance();
                    // `x**y` is power; a bare `x**` is square.
                    let arg = if !self.peek_tok().space_before && self.can_start_primary() {
                        Some(Box::new(self.parse_primary()?))
                    } else {
                        None
                    };
                    let op = if arg.is_some() { PostOp::Power } else { PostOp::Square };
                    expr = Expr::Postfix { op, expr: Box::new(expr), arg, line };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// Parse a parenthesised, whitespace-separated argument list.
    fn parse_call_args(&mut self) -> Result<Vec<Arg>> {
        self.paren_depth += 1;
        self.expect(&Tok::LParen, "'('")?;
        let mut args = Vec::new();
        loop {
            self.skip_newlines();
            if self.check(&Tok::RParen) {
                break;
            }
            // Splat: `<group>` opens a Group into separate arguments.
            if self.check(&Tok::Lt) && !self.peek_tok_at(1).space_before {
                self.advance();
                let expr = self.parse_postfix()?;
                self.expect(&Tok::Gt, "'>' to close a splat argument")?;
                args.push(Arg::Splat(expr));
            } else {
                args.push(Arg::Normal(self.parse_expr()?));
            }
        }
        self.expect(&Tok::RParen, "')' to close a call")?;
        self.paren_depth -= 1;
        Ok(args)
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
                // Typed empty: `number ..` and `text ..` (the only bare
                // application forms Six keeps).
                if self.check(&Tok::Empty) {
                    if name == "number" {
                        self.advance();
                        return Ok(Expr::Empty);
                    }
                    if name == "text" {
                        self.advance();
                        return Ok(Expr::Text(String::new()));
                    }
                }
                Ok(Expr::Var { name, line })
            }
            Tok::LParen => {
                self.paren_depth += 1;
                self.advance();
                self.skip_newlines();
                let expr = self.parse_expr()?;
                self.skip_newlines();
                self.expect(&Tok::RParen, "')'")?;
                self.paren_depth -= 1;
                Ok(expr)
            }
            Tok::LBracket => self.parse_group(),
            Tok::If | Tok::QIf | Tok::QAny => self.parse_conditional(),
            other => Err(SixError::at(line, format!("unexpected {}", describe(&other)))),
        }
    }

    fn parse_group(&mut self) -> Result<Expr> {
        let line = self.line();
        self.paren_depth += 1;
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
        self.paren_depth -= 1;
        Ok(Expr::Group { members, line })
    }

    /// An inline Group member: a single atom (primary + postfix suffixes),
    /// never a bare application. Compound calls are already parenthesised.
    fn parse_group_atom(&mut self) -> Result<Expr> {
        self.parse_postfix()
    }

    /// Parse `if` / `if any` / `?` / `?*`. Consumes the closing `.`.
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

            if self.check(&Tok::Else) || self.check(&Tok::QElse) {
                self.advance();
                let arrow = self.expect(&Tok::FatArrow, "'>>' after 'else'")?;
                else_body = Some(self.parse_arm_body(arrow.line)?);
                continue;
            }

            let cond = self.parse_binary()?;
            let arrow = self.expect(&Tok::FatArrow, "'>>' after a condition")?;
            let body = self.parse_arm_body(arrow.line)?;
            arms.push(Arm { cond, body });
        }

        Ok(Expr::If { any, arms, else_body, line })
    }

    /// An arm's consequent: an inline single statement when it shares the
    /// arrow's line, otherwise statements up to the next arm header or the
    /// conditional's terminator.
    fn parse_arm_body(&mut self, arrow_line: usize) -> Result<Vec<Stmt>> {
        let t = self.peek_tok();
        if t.line == arrow_line && !matches!(t.tok, Tok::Newline | Tok::Dot | Tok::Eof) {
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
    /// does not itself begin a nested conditional.
    fn line_is_arm_header(&self) -> bool {
        match self.peek() {
            Tok::Else | Tok::QElse => return true,
            Tok::If | Tok::QIf | Tok::QAny => return false,
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

    fn can_start_primary(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Number(_)
                | Tok::Text(_)
                | Tok::True
                | Tok::False
                | Tok::Nil
                | Tok::Empty
                | Tok::Dollar
                | Tok::Ident(_)
                | Tok::LParen
                | Tok::LBracket
        )
    }

    fn expect_ident(&mut self, what: &str) -> Result<String> {
        match self.advance().tok {
            Tok::Ident(n) => Ok(n),
            other => Err(SixError::at(self.line(), format!("expected {}, found {}", what, describe(&other)))),
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
    name.chars().find(|c| c.is_alphabetic()).map(|c| c.is_uppercase()).unwrap_or(false)
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
        Tok::At => "'@'".to_string(),
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
        Tok::Tilde => "'~'".to_string(),
        Tok::Caret => "'^'".to_string(),
        Tok::Underscore => "'_'".to_string(),
        Tok::StarStar => "'**'".to_string(),
        Tok::SlashSlash => "'//'".to_string(),
        Tok::PlusPlus => "'++'".to_string(),
        Tok::MinusMinus => "'--'".to_string(),
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
