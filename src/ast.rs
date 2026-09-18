//! The Six abstract syntax tree.
//!
//! The tree is deliberately small — it mirrors the six fundamental concepts:
//! values, names, operations, functions, conditionals, and Groups. Syntactic
//! sugar (dot-flow, `>>` flow, the `?`/`??`/`?*` shorthands) is desugared by the
//! parser, so the interpreter never sees it.

use std::rc::Rc;

/// A top-level or block-level statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// `name : value`  or  `name :: value` (deep copy).
    Bind {
        name: String,
        value: Expr,
        deep: bool,
        immutable: bool,
        line: usize,
    },
    /// `target = value` — rebind a name or write through an index/key.
    Assign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    /// A function declaration binds a function value to `name`.
    Func {
        def: Rc<FuncDef>,
        immutable: bool,
        line: usize,
    },
    /// `@name` — import `name.six` and bind its program Group under `name`.
    Import { name: String, line: usize },
    /// A bare expression evaluated for its value and/or effect.
    Expr(Expr),
}

/// The static description of a function (shared by every closure over it).
#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

/// An expression.
#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    /// Generic empty (`..` / `nil`).
    Empty,
    /// `$` — the final valid index, resolved against whatever it indexes.
    Last,
    Var { name: String, line: usize },
    /// A Group literal: `[a b c]`.
    Group { members: Vec<Expr>, line: usize },
    /// Structural access: `base[index]`.
    Index { base: Box<Expr>, index: Box<Expr>, line: usize },
    /// Function application. `callee` is applied to `args`; the flowing value in
    /// `>>` / dot-flow is desugared to `args[0]`.
    Call { callee: Box<Expr>, args: Vec<Arg>, line: usize },
    Unary { op: UnOp, expr: Box<Expr>, line: usize },
    /// A numeric postfix operator: `x~`, `x~2`, `x^`, `x_`, `x**`, `x**y`, `x//`.
    Postfix { op: PostOp, expr: Box<Expr>, arg: Option<Box<Expr>>, line: usize },
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr>, line: usize },
    /// A conditional. `any == true` means "run every matching branch".
    If { any: bool, arms: Vec<Arm>, else_body: Option<Vec<Stmt>>, line: usize },
}

impl Expr {
    pub fn line(&self) -> usize {
        match self {
            Expr::Var { line, .. }
            | Expr::Group { line, .. }
            | Expr::Index { line, .. }
            | Expr::Call { line, .. }
            | Expr::Unary { line, .. }
            | Expr::Postfix { line, .. }
            | Expr::Binary { line, .. }
            | Expr::If { line, .. } => *line,
            _ => 0,
        }
    }
}

/// A call argument. `<group>` splat-opens a Group into separate arguments.
#[derive(Debug, Clone)]
pub enum Arg {
    Normal(Expr),
    Splat(Expr),
}

/// One arm of a conditional: `condition >> body`.
#[derive(Debug, Clone)]
pub struct Arm {
    pub cond: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

/// Numeric postfix operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PostOp {
    Round,  // `~` (round to integer) or `~n` (round to n decimals)
    Ceil,   // `^`
    Floor,  // `_`
    Square, // `**` with no operand
    Power,  // `**y`
    Sqrt,   // `//`
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}
