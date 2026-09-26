//! How Six renders values as text — for the `text` conversion and error
//! messages.
//!
//! `display` is the top-level rendering (text appears raw); `repr` is used for
//! values nested inside a Group (text appears quoted) so the structure stays
//! legible.

use crate::value::Value;

/// Format a number without a trailing `.0` for integral values.
pub fn number(n: f64) -> String {
    if n == 0.0 {
        // Avoid rendering "-0".
        return "0".to_string();
    }
    format!("{}", n)
}

/// Top-level rendering (used by `print` and `text`).
pub fn display(v: &Value) -> String {
    match v {
        Value::Number(n) => number(*n),
        Value::Text(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Empty => "nil".to_string(),
        Value::Last => "$".to_string(),
        Value::Group(g) => {
            let items = g.items.borrow();
            let inner: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", inner.join(" "))
        }
        Value::Func(_) | Value::Builtin(_) => "<function>".to_string(),
    }
}

/// Nested rendering (text quoted).
pub fn repr(v: &Value) -> String {
    match v {
        Value::Text(s) => format!("\"{}\"", s),
        _ => display(v),
    }
}
