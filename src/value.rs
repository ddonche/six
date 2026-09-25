//! Runtime values and the scope chain.
//!
//! The value/reference split is the heart of Six's semantics (spec §8):
//! numbers, text, booleans, functions and empties are value-semantic (a plain
//! `clone` copies them), while a Group is reference-semantic — cloning a
//! `Value::Group` clones only the `Rc`, so both names share one Group. `::`
//! performs the deep copy that breaks that sharing.
//!
//! A scope's binding store is itself a `GroupRef` — an ordinary keyed Group of
//! `["name" value]` pairs. That is what makes `@` imports work with no new
//! value kind: a module is exposed by handing out its program scope's store as
//! an ordinary `Value::Group`. The specialness of `@` lives entirely in the
//! loader and in the binding *rules* the scope layer applies (name-case
//! immutability, redeclaration) — never in the value it produces.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ast::FuncDef;

/// The shared, mutable backing store of a Group, with an immutability latch set
/// when the Group is first bound to an uppercase (immutable) name, and an
/// optional hidden host association bound to this Group's identity (see
/// [`crate::host::Assoc`]). The association lives here, on the `Rc`-shared
/// identity, never in `items` — so aliasing shares it and `::` deep-copy (which
/// builds a fresh `GroupData`) strips it.
#[derive(Debug)]
pub struct GroupData {
    pub items: RefCell<Vec<Value>>,
    pub immutable: Cell<bool>,
    pub assoc: RefCell<Option<Box<crate::host::Assoc>>>,
}

pub type GroupRef = Rc<GroupData>;

impl GroupData {
    /// Index of the first direct member that is a two-member `["key" _]` pair
    /// matching `key`. This one scan underlies both keyed Group access and
    /// scope name resolution — a scope store is just such a Group.
    pub fn find_key(&self, key: &str) -> Option<usize> {
        self.items.borrow().iter().position(|item| pair_key_matches(item, key))
    }
}

/// True when `item` is a two-member Group whose first member is text == `key`.
pub fn pair_key_matches(item: &Value, key: &str) -> bool {
    if let Value::Group(pair) = item {
        let p = pair.items.borrow();
        p.len() == 2 && matches!(&p[0], Value::Text(k) if k == key)
    } else {
        false
    }
}

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
}

impl Value {
    pub fn new_group(items: Vec<Value>) -> Value {
        Value::Group(new_group_ref(items))
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
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// Recursively deep-copy Groups (used by `::`). Simple values copy directly.
    /// An imported module is an ordinary Group, so it deep-copies like one.
    pub fn deep_copy(&self) -> Value {
        match self {
            Value::Group(g) => {
                let items = g.items.borrow().iter().map(|v| v.deep_copy()).collect();
                Value::new_group(items)
            }
            other => other.clone(),
        }
    }
}

/// Build a fresh Group backing store (no host association).
pub fn new_group_ref(items: Vec<Value>) -> GroupRef {
    Rc::new(GroupData {
        items: RefCell::new(items),
        immutable: Cell::new(false),
        assoc: RefCell::new(None),
    })
}

/// Build a fresh Group carrying a hidden host association (a live relationship).
pub fn new_relationship(items: Vec<Value>, assoc: crate::host::Assoc) -> GroupRef {
    Rc::new(GroupData {
        items: RefCell::new(items),
        immutable: Cell::new(false),
        assoc: RefCell::new(Some(Box::new(assoc))),
    })
}

// --- environment ------------------------------------------------------------

pub type Env = Rc<RefCell<Scope>>;

/// A lexical scope. Its `store` is an ordinary keyed Group of `["name" value]`
/// pairs — the very same representation a program exposes when imported.
#[derive(Debug)]
pub struct Scope {
    pub store: GroupRef,
    pub parent: Option<Env>,
}

impl Scope {
    pub fn new_global() -> Env {
        Rc::new(RefCell::new(Scope { store: new_group_ref(Vec::new()), parent: None }))
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(RefCell::new(Scope { store: new_group_ref(Vec::new()), parent: Some(parent.clone()) }))
    }

    /// The scope's binding store as a Group value (this is what `@` hands out).
    pub fn store(&self) -> GroupRef {
        self.store.clone()
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
        }
    }
}
