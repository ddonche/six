//! Runtime-level standard-library operations.
//!
//! These earn a place in Rust either because Six cannot express them from
//! within itself (I/O, conversions, `size`) or because the operation is a
//! fundamental Group mutation (`insert`, `remove`). Higher-order helpers
//! (`map`, `filter`, `fold`, `find`) live in the Six prelude instead, so the
//! language demonstrably bootstraps them from its own machinery.

use std::io::{self, Write};

use crate::error::{Result, SixError};
use crate::format;
use crate::interp::Interpreter;
use crate::value::{GroupRef, Value};

pub fn dispatch(interp: &mut Interpreter, name: &str, args: Vec<Value>, line: usize) -> Result<Value> {
    match name {
        "print" => builtin_print(interp, args, line),
        "input" => builtin_input(interp, args, line),
        "size" => builtin_size(args, line),
        "has?" => builtin_has(args, line),
        "split" => builtin_split(args, line),
        "number" => builtin_number(args, line),
        "text" => builtin_text(args, line),
        "insert" => builtin_insert(args, line),
        "remove" => builtin_remove(args, line),
        _ => Err(SixError::at(line, format!("unknown builtin '{}'", name))),
    }
}

fn arity(name: &str, args: &[Value], min: usize, max: usize, line: usize) -> Result<()> {
    if args.len() < min || args.len() > max {
        let want = if min == max { format!("{}", min) } else { format!("{} to {}", min, max) };
        return Err(SixError::at(
            line,
            format!("'{}' expects {} argument(s) but got {}", name, want, args.len()),
        ));
    }
    Ok(())
}

fn builtin_print(interp: &mut Interpreter, args: Vec<Value>, line: usize) -> Result<Value> {
    arity("print", &args, 1, 1, line)?;
    let s = format::display(&args[0]);
    interp.output(&s);
    interp.output("\n");
    Ok(Value::Empty)
}

fn builtin_input(interp: &mut Interpreter, args: Vec<Value>, line: usize) -> Result<Value> {
    arity("input", &args, 0, 1, line)?;
    if let Some(prompt) = args.get(0) {
        let s = format::display(prompt);
        interp.output(&s);
    }
    let mut buf = String::new();
    match io::stdin().read_line(&mut buf) {
        Ok(0) => Ok(Value::Text(String::new())), // EOF
        Ok(_) => {
            while buf.ends_with('\n') || buf.ends_with('\r') {
                buf.pop();
            }
            Ok(Value::Text(buf))
        }
        Err(e) => Err(SixError::at(line, format!("could not read input: {}", e))),
    }
}

fn builtin_size(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("size", &args, 1, 1, line)?;
    let n = match &args[0] {
        Value::Group(g) => g.items.borrow().len(),
        Value::Text(s) => s.chars().count(),
        Value::Module(scope) => scope.borrow().vars.len(),
        Value::Empty => 0,
        other => {
            return Err(SixError::at(line, format!("size is not supported for a {}", other.type_name())));
        }
    };
    Ok(Value::Number(n as f64))
}

fn builtin_has(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("has?", &args, 2, 2, line)?;
    let key = match &args[1] {
        Value::Text(k) => k.clone(),
        other => return Err(SixError::at(line, format!("has? needs a text key, not a {}", other.type_name()))),
    };
    // A module answers existence over its top-level bindings.
    if let Value::Module(scope) = &args[0] {
        return Ok(Value::Bool(scope.borrow().vars.contains_key(&key)));
    }
    let g = as_group(&args[0], line, "has?")?;
    let found = g.items.borrow().iter().any(|item| {
        if let Value::Group(pair) = item {
            let pair = pair.items.borrow();
            pair.len() == 2 && matches!(&pair[0], Value::Text(k) if k == &key)
        } else {
            false
        }
    });
    Ok(Value::Bool(found))
}

fn builtin_split(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("split", &args, 2, 2, line)?;
    let text = match &args[0] {
        Value::Text(s) => s,
        other => return Err(SixError::at(line, format!("split needs text, not a {}", other.type_name()))),
    };
    let delim = match &args[1] {
        Value::Text(d) => d,
        other => return Err(SixError::at(line, format!("split needs a text delimiter, not a {}", other.type_name()))),
    };
    let pieces: Vec<Value> = if delim.is_empty() {
        text.chars().map(|c| Value::Text(c.to_string())).collect()
    } else {
        text.split(delim.as_str()).map(|p| Value::Text(p.to_string())).collect()
    };
    Ok(Value::new_group(pieces))
}

fn builtin_number(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("number", &args, 1, 1, line)?;
    match &args[0] {
        Value::Number(n) => Ok(Value::Number(*n)),
        Value::Empty => Ok(Value::Empty),
        Value::Text(s) => {
            let cleaned: String = s.chars().filter(|c| *c != ',').collect();
            let cleaned = cleaned.trim();
            match cleaned.parse::<f64>() {
                Ok(n) => Ok(Value::Number(n)),
                Err(_) => Err(SixError::at(line, format!("cannot convert \"{}\" to a number", s))),
            }
        }
        other => Err(SixError::at(line, format!("cannot convert a {} to a number", other.type_name()))),
    }
}

fn builtin_text(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("text", &args, 1, 1, line)?;
    match &args[0] {
        Value::Text(s) => Ok(Value::Text(s.clone())),
        Value::Number(n) => Ok(Value::Text(format::number(*n))),
        Value::Bool(b) => Ok(Value::Text(b.to_string())),
        Value::Empty => Ok(Value::Text(String::new())),
        other => Err(SixError::at(line, format!("cannot convert a {} to text", other.type_name()))),
    }
}

fn builtin_insert(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("insert", &args, 2, 3, line)?;
    let g = as_group(&args[0], line, "insert")?;
    if g.immutable.get() {
        return Err(SixError::at(line, "cannot insert into an immutable Group"));
    }
    let value = args[1].clone();
    let mut items = g.items.borrow_mut();
    match args.get(2) {
        None => items.push(value),
        Some(idx) => {
            let len = items.len();
            let pos = match idx {
                Value::Last => len, // insert after the current last
                Value::Number(n) => {
                    if n.fract() != 0.0 || *n < 0.0 {
                        return Err(SixError::at(line, "insert position must be a whole, non-negative number"));
                    }
                    let p = *n as usize;
                    if p > len {
                        return Err(SixError::at(line, format!("insert position {} is out of range for a Group of size {}", p, len)));
                    }
                    p
                }
                other => return Err(SixError::at(line, format!("insert position must be a number, not a {}", other.type_name()))),
            };
            items.insert(pos, value);
        }
    }
    Ok(Value::Empty)
}

fn builtin_remove(args: Vec<Value>, line: usize) -> Result<Value> {
    arity("remove", &args, 2, 2, line)?;
    let g = as_group(&args[0], line, "remove")?;
    if g.immutable.get() {
        return Err(SixError::at(line, "cannot remove from an immutable Group"));
    }
    let mut items = g.items.borrow_mut();
    let len = items.len();
    let pos = match &args[1] {
        Value::Last => {
            if len == 0 {
                return Err(SixError::at(line, "cannot remove from an empty Group"));
            }
            len - 1
        }
        Value::Number(n) => {
            if n.fract() != 0.0 || *n < 0.0 {
                return Err(SixError::at(line, "remove position must be a whole, non-negative number"));
            }
            let p = *n as usize;
            if p >= len {
                return Err(SixError::at(line, format!("remove position {} is out of range for a Group of size {}", p, len)));
            }
            p
        }
        other => return Err(SixError::at(line, format!("remove position must be a number, not a {}", other.type_name()))),
    };
    items.remove(pos);
    Ok(Value::Empty)
}

fn as_group<'a>(v: &'a Value, line: usize, op: &str) -> Result<&'a GroupRef> {
    match v {
        Value::Group(g) => Ok(g),
        other => Err(SixError::at(line, format!("{} needs a Group, not a {}", op, other.type_name()))),
    }
}

// Keep the io::Write import used even if print paths change.
#[allow(dead_code)]
fn _touch(_w: &dyn Write) {}
