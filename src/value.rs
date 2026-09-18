//! Runtime values and the scope chain.
//!
//! The value/reference split is the heart of Six's semantics (spec §8):
//! numbers, text, booleans, functions and empties are value-semantic (a plain
//! `clone` copies them), while a Group is reference-semantic — cloning a
//! `Value::Group` clones only the `Rc`, so both names share one Group. `::`
//! performs the deep copy that breaks that sharing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::FuncDef;

/// The shared, mutable backing store of a Group, with an immutability latch set
/// when the Group is first bound to an uppercase (immutable) name.
#[derive(Debug)]
pub struct GroupData {
    pub items: RefCell<Vec<Value>>,
    pub immutable: Cell<bool>,
}

pub type GroupRef = Rc<GroupData>;

/// A closure: the static definition plus the environment it was defined in.
#[derive(Debug)]
pub struct Closure {
    pub def: Rc<FuncDef>,
    pub env: Env,
}

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    /// Generic or typed empty. Six distinguishes "empty" (a value that exists
    /// but holds nothing) from "nonexistent" (an unbound name), which is an
    /// error to read.
    Empty,
    /// The `$` sentinel — the final valid index, resolved by whatever indexes it.
    Last,
    Group(GroupRef),
    Func(Rc<Closure>),
    Builtin(&'static str),
    /// An imported module. It wraps the module file's *live* top-level scope —
    /// keyed access (`hero["health"]`) reads and writes that scope directly, so
    /// the program Group and the module environment are one binding store.
    Module(Env),
}

impl Value {
    pub fn new_group(items: Vec<Value>) -> Value {
        Value::Group(Rc::new(GroupData {
            items: RefCell::new(items),
            immutable: Cell::new(false),
        }))
    }

    /// The short type name used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Bool(_) => "boolean",
            Value::Empty => "empty",
            Value::Last => "$",
            Value::Group(_) => "Group",
            Value::Func(_) => "function",
            Value::Builtin(_) => "function",
            Value::Module(_) => "module",
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// Recursively deep-copy Groups (used by `::`). Simple values copy directly.
    pub fn deep_copy(&self) -> Value {
        match self {
            Value::Group(g) => {
                let items = g.items.borrow().iter().map(|v| v.deep_copy()).collect();
                Value::new_group(items)
            }
            // Deep-copying a module snapshots its bindings into an ordinary
            // Group of key/value pairs (import itself never snapshots).
            Value::Module(scope) => {
                let mut pairs: Vec<(String, Value)> =
                    scope.borrow().vars.iter().map(|(k, b)| (k.clone(), b.value.deep_copy())).collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                let items = pairs
                    .into_iter()
                    .map(|(k, v)| Value::new_group(vec![Value::Text(k), v]))
                    .collect();
                Value::new_group(items)
            }
            other => other.clone(),
        }
    }
}

// --- environment ------------------------------------------------------------

pub type Env = Rc<RefCell<Scope>>;

#[derive(Debug)]
pub struct Binding {
    pub value: Value,
    pub immutable: bool,
}

#[derive(Debug, Default)]
pub struct Scope {
    pub vars: HashMap<String, Binding>,
    pub parent: Option<Env>,
}

impl Scope {
    pub fn new_global() -> Env {
        Rc::new(RefCell::new(Scope { vars: HashMap::new(), parent: None }))
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(RefCell::new(Scope { vars: HashMap::new(), parent: Some(parent.clone()) }))
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Number({})", n),
            Value::Text(s) => write!(f, "Text({:?})", s),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Empty => write!(f, "Empty"),
            Value::Last => write!(f, "Last"),
            Value::Group(_) => write!(f, "Group(..)"),
            Value::Func(_) => write!(f, "Func(..)"),
            Value::Builtin(n) => write!(f, "Builtin({})", n),
            Value::Module(_) => write!(f, "Module(..)"),
        }
    }
}
