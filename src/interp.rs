//! The Six tree-walking interpreter.
//!
//! Two design points matter most:
//!
//! * **Guaranteed tail calls (spec §25.1).** A call in tail position does not
//!   recurse in Rust; instead `eval` returns a [`Flow::Tail`] signal and
//!   [`Interpreter::call_function`] loops (a trampoline). This gives ordinary
//!   recursion bounded-stack execution, so `countdown 1,000,000` runs fine.
//! * **Reference vs value semantics** falls out of [`Value`]: cloning a Group
//!   shares its `Rc`, cloning anything else copies it.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::*;
use crate::error::{Result, SixError};
use crate::value::*;

/// The result of evaluating something in (possibly) tail position.
enum Flow {
    Value(Value),
    /// A pending tail call the trampoline should perform without growing stack.
    Tail(Value, Vec<Value>),
}

pub struct Interpreter {
    /// The base scope holding the runtime builtins. Every program (the main
    /// file and every imported module) runs in a child of this scope, so a
    /// module's own top-level bindings stay separate from the builtins.
    builtins: Env,
    pub global: Env,
    out: Box<dyn Write>,
    /// In REPL mode, re-binding a name at the top level replaces it instead of
    /// erroring, so a function can be redefined interactively.
    repl: bool,
    /// Directory that `@name` imports resolve against (the importing file's
    /// directory). Saved and restored around each module evaluation.
    base_dir: PathBuf,
    /// Module cache keyed by canonical path: a resolved file is evaluated once
    /// per execution and shared by every importer.
    modules: HashMap<PathBuf, Value>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self::build(Box::new(io::stdout()))
    }

    /// Build an interpreter that writes program output to an in-memory buffer,
    /// used by the test suite.
    pub fn with_writer(out: Box<dyn Write>) -> Self {
        Self::build(out)
    }

    fn build(out: Box<dyn Write>) -> Self {
        let builtins = Scope::new_global();
        {
            let store = builtins.borrow().store();
            let mut items = store.items.borrow_mut();
            for name in BUILTINS {
                items.push(Value::new_group(vec![Value::Text(name.to_string()), Value::Builtin(name)]));
            }
        }
        let global = Scope::child(&builtins);
        Interpreter {
            builtins,
            global,
            out,
            repl: false,
            base_dir: PathBuf::from("."),
            modules: HashMap::new(),
        }
    }

    /// Enable REPL semantics (top-level redefinition).
    pub fn set_repl(&mut self, repl: bool) {
        self.repl = repl;
    }

    /// Set the directory that top-level `@` imports resolve against.
    pub fn set_base_dir(&mut self, dir: PathBuf) {
        self.base_dir = dir;
    }

    /// Run a whole program. Returns the value of its final statement.
    pub fn run(&mut self, program: &[Stmt]) -> Result<Value> {
        let env = self.global.clone();
        let mut last = Value::Empty;
        for stmt in program {
            last = self.exec_stmt(stmt, &env, false)?.into_value(self)?;
        }
        Ok(last)
    }

    // --- statements ---------------------------------------------------------

    fn exec_body(&mut self, body: &[Stmt], env: &Env, tail: bool) -> Result<Flow> {
        if body.is_empty() {
            return Ok(Flow::Value(Value::Empty));
        }
        let last = body.len() - 1;
        for stmt in &body[..last] {
            self.exec_stmt(stmt, env, false)?.into_value(self)?;
        }
        self.exec_stmt(&body[last], env, tail)
    }

    fn exec_stmt(&mut self, stmt: &Stmt, env: &Env, tail: bool) -> Result<Flow> {
        match stmt {
            Stmt::Bind { name, value, deep, immutable, line } => {
                let mut v = self.eval(value, env, false)?.into_value(self)?;
                if *deep {
                    v = v.deep_copy();
                }
                if *immutable {
                    if let Value::Group(g) = &v {
                        g.immutable.set(true);
                    }
                }
                self.bind_new(env, name, v, *line)?;
                Ok(Flow::Value(Value::Empty))
            }
            Stmt::Assign { target, value, line } => {
                let v = self.eval(value, env, false)?.into_value(self)?;
                self.assign(target, v, env, *line)?;
                Ok(Flow::Value(Value::Empty))
            }
            Stmt::Func { def, immutable: _, line } => {
                let closure = Value::Func(Rc::new(Closure { def: def.clone(), env: env.clone() }));
                self.bind_new(env, &def.name, closure, *line)?;
                Ok(Flow::Value(Value::Empty))
            }
            Stmt::Import { name, line } => {
                let module = self.import_module(name, *line)?;
                self.bind_new(env, name, module, *line)?;
                Ok(Flow::Value(Value::Empty))
            }
            Stmt::Expr(e) => self.eval(e, env, tail),
        }
    }

    /// Resolve, load (once), and return the program Group for `name.six`.
    ///
    /// The value returned is an ordinary [`Value::Group`]: the module's
    /// top-level scope stores its bindings in a `GroupRef`, and that same
    /// `GroupRef` is what we hand out. After this returns, nothing in the
    /// evaluator can tell the Group came from another file — `@` is special,
    /// the value it produces is not.
    fn import_module(&mut self, name: &str, line: usize) -> Result<Value> {
        let path = self.base_dir.join(format!("{}.six", name));
        let canonical = std::fs::canonicalize(&path).map_err(|_| {
            SixError::at(line, format!("cannot import '{}': no file {} found", name, path.display()))
        })?;

        // Module identity: a resolved file is evaluated once per execution.
        if let Some(m) = self.modules.get(&canonical) {
            return Ok(m.clone());
        }

        // The module's top-level scope store IS its program Group. Cache it
        // before evaluating so cyclic imports resolve to the in-progress module.
        let mod_scope = Scope::child(&self.builtins);
        let module_val = Value::Group(mod_scope.borrow().store());
        self.modules.insert(canonical.clone(), module_val.clone());

        let source = std::fs::read_to_string(&canonical)
            .map_err(|e| SixError::at(line, format!("cannot read {}: {}", canonical.display(), e)))?;
        let program = crate::parse(&source).map_err(|e| SixError::new(format!("in {}: {}", name, e)))?;

        let mod_dir = canonical.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let prev_dir = std::mem::replace(&mut self.base_dir, mod_dir);
        let result = self.run_program_in(&program, &mod_scope);
        self.base_dir = prev_dir;
        result.map_err(|e| SixError::new(format!("in {}: {}", name, e)))?;

        Ok(module_val)
    }

    fn run_program_in(&mut self, program: &[Stmt], env: &Env) -> Result<()> {
        for stmt in program {
            self.exec_stmt(stmt, env, false)?.into_value(self)?;
        }
        Ok(())
    }

    /// Create a new binding in this scope's store (an ordinary keyed Group).
    /// Redeclaration is an error, except that REPL mode replaces at top level.
    fn bind_new(&self, env: &Env, name: &str, value: Value, line: usize) -> Result<()> {
        let repl_global = self.repl && Rc::ptr_eq(env, &self.global);
        let store = env.borrow().store();
        if let Some(idx) = store.find_key(name) {
            if repl_global {
                set_pair_value(&store, idx, value);
                return Ok(());
            }
            return Err(SixError::at(line, format!("'{}' is already defined in this scope", name)));
        }
        store.items.borrow_mut().push(Value::new_group(vec![Value::Text(name.to_string()), value]));
        Ok(())
    }

    // --- expressions --------------------------------------------------------

    fn eval(&mut self, expr: &Expr, env: &Env, tail: bool) -> Result<Flow> {
        match expr {
            Expr::Number(n) => Ok(Flow::Value(Value::Number(*n))),
            Expr::Text(s) => Ok(Flow::Value(Value::Text(s.clone()))),
            Expr::Bool(b) => Ok(Flow::Value(Value::Bool(*b))),
            Expr::Empty => Ok(Flow::Value(Value::Empty)),
            Expr::Last => Ok(Flow::Value(Value::Last)),
            Expr::Var { name, line } => Ok(Flow::Value(self.lookup(env, name, *line)?)),
            Expr::Group { members, .. } => {
                let mut items = Vec::with_capacity(members.len());
                for m in members {
                    items.push(self.eval(m, env, false)?.into_value(self)?);
                }
                Ok(Flow::Value(Value::new_group(items)))
            }
            Expr::Index { base, index, line } => {
                let base_v = self.eval(base, env, false)?.into_value(self)?;
                let idx_v = self.eval(index, env, false)?.into_value(self)?;
                Ok(Flow::Value(self.index_get(&base_v, &idx_v, *line)?))
            }
            Expr::Unary { op, expr, line } => {
                let v = self.eval(expr, env, false)?.into_value(self)?;
                Ok(Flow::Value(self.eval_unary(*op, v, *line)?))
            }
            Expr::Postfix { op, expr, arg, line } => {
                let v = self.eval(expr, env, false)?.into_value(self)?;
                let arg_v = match arg {
                    Some(a) => Some(self.eval(a, env, false)?.into_value(self)?),
                    None => None,
                };
                Ok(Flow::Value(self.eval_postfix(*op, v, arg_v, *line)?))
            }
            Expr::Binary { op, left, right, line } => {
                Ok(Flow::Value(self.eval_binary(*op, left, right, env, *line)?))
            }
            Expr::Call { callee, args, line } => self.eval_call(callee, args, env, tail, *line),
            Expr::If { any, arms, else_body, line } => {
                self.eval_if(*any, arms, else_body, env, tail, *line)
            }
        }
    }

    fn lookup(&self, env: &Env, name: &str, line: usize) -> Result<Value> {
        let mut cur = Some(env.clone());
        while let Some(scope) = cur {
            let store = scope.borrow().store();
            if let Some(idx) = store.find_key(name) {
                return Ok(pair_value(&store, idx));
            }
            cur = scope.borrow().parent.clone();
        }
        Err(SixError::at(line, format!("undefined name '{}'", name)))
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Arg], env: &Env, tail: bool, line: usize) -> Result<Flow> {
        let func = self.eval(callee, env, false)?.into_value(self)?;
        let mut argv = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                Arg::Normal(e) => argv.push(self.eval(e, env, false)?.into_value(self)?),
                Arg::Splat(e) => {
                    let v = self.eval(e, env, false)?.into_value(self)?;
                    match v {
                        Value::Group(g) => {
                            for item in g.items.borrow().iter() {
                                argv.push(item.clone());
                            }
                        }
                        other => {
                            return Err(SixError::at(line, format!("cannot splat a {} — only a Group opens with < >", other.type_name())));
                        }
                    }
                }
            }
        }

        // A user function in tail position becomes a trampolined tail call.
        if tail && matches!(func, Value::Func(_)) {
            return Ok(Flow::Tail(func, argv));
        }
        Ok(Flow::Value(self.call_function(func, argv, line)?))
    }

    /// Invoke a callable, trampolining tail calls so recursion stays bounded.
    pub fn call_function(&mut self, mut func: Value, mut args: Vec<Value>, line: usize) -> Result<Value> {
        loop {
            match func {
                Value::Builtin(name) => return self.call_builtin(name, args, line),
                Value::Func(closure) => {
                    let def = &closure.def;
                    if args.len() != def.params.len() {
                        return Err(SixError::at(
                            def.line,
                            format!(
                                "function '{}' expects {} argument(s) but got {}",
                                def.name,
                                def.params.len(),
                                args.len()
                            ),
                        ));
                    }
                    let call_env = Scope::child(&closure.env);
                    {
                        let store = call_env.borrow().store();
                        let mut items = store.items.borrow_mut();
                        for (param, value) in def.params.iter().zip(args.into_iter()) {
                            items.push(Value::new_group(vec![Value::Text(param.clone()), value]));
                        }
                    }
                    match self.exec_body(&def.body, &call_env, true)? {
                        Flow::Value(v) => return Ok(v),
                        Flow::Tail(next_func, next_args) => {
                            func = next_func;
                            args = next_args;
                            continue;
                        }
                    }
                }
                other => {
                    return Err(SixError::at(line, format!("cannot call a {} — it is not a function", other.type_name())));
                }
            }
        }
    }

    fn eval_if(
        &mut self,
        any: bool,
        arms: &[Arm],
        else_body: &Option<Vec<Stmt>>,
        env: &Env,
        tail: bool,
        line: usize,
    ) -> Result<Flow> {
        if any {
            // `if any`: run every matching branch; else runs only if none did.
            let mut matched = false;
            let mut last = Value::Empty;
            for arm in arms {
                let c = self.eval(&arm.cond, env, false)?.into_value(self)?;
                if self.as_bool(&c, line)? {
                    matched = true;
                    let branch_env = Scope::child(env);
                    last = self.exec_body(&arm.body, &branch_env, false)?.into_value(self)?;
                }
            }
            if !matched {
                if let Some(body) = else_body {
                    let branch_env = Scope::child(env);
                    last = self.exec_body(body, &branch_env, false)?.into_value(self)?;
                }
            }
            return Ok(Flow::Value(last));
        }

        // Plain `if`: first true branch wins and may carry a tail call.
        for arm in arms {
            let c = self.eval(&arm.cond, env, false)?.into_value(self)?;
            if self.as_bool(&c, line)? {
                let branch_env = Scope::child(env);
                return self.exec_body(&arm.body, &branch_env, tail);
            }
        }
        if let Some(body) = else_body {
            let branch_env = Scope::child(env);
            return self.exec_body(body, &branch_env, tail);
        }
        Ok(Flow::Value(Value::Empty))
    }

    fn as_bool(&self, v: &Value, line: usize) -> Result<bool> {
        match v {
            Value::Bool(b) => Ok(*b),
            other => Err(SixError::at(
                line,
                format!("a condition must be a boolean, but this is a {} (Six has no truthiness)", other.type_name()),
            )),
        }
    }

    // --- operators ----------------------------------------------------------

    fn eval_unary(&self, op: UnOp, v: Value, line: usize) -> Result<Value> {
        match op {
            UnOp::Neg => match v {
                Value::Number(n) => Ok(Value::Number(-n)),
                other => Err(SixError::at(line, format!("cannot negate a {}", other.type_name()))),
            },
            UnOp::Not => match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                other => Err(SixError::at(line, format!("'not' needs a boolean, not a {}", other.type_name()))),
            },
        }
    }

    fn eval_postfix(&self, op: PostOp, v: Value, arg: Option<Value>, line: usize) -> Result<Value> {
        let n = match v {
            Value::Number(n) => n,
            other => return Err(SixError::at(line, format!("this operator needs a number, not a {}", other.type_name()))),
        };
        let result = match op {
            PostOp::Round => match arg {
                None => n.round(),
                Some(Value::Number(places)) => {
                    if places.fract() != 0.0 || places < 0.0 {
                        return Err(SixError::at(line, "'~' decimal count must be a whole, non-negative number"));
                    }
                    let factor = 10f64.powi(places as i32);
                    (n * factor).round() / factor
                }
                Some(other) => return Err(SixError::at(line, format!("'~' decimal count must be a number, not a {}", other.type_name()))),
            },
            PostOp::Ceil => n.ceil(),
            PostOp::Floor => n.floor(),
            PostOp::Square => n * n,
            PostOp::Sqrt => {
                if n < 0.0 {
                    return Err(SixError::at(line, "cannot take the square root of a negative number"));
                }
                n.sqrt()
            }
            PostOp::Power => match arg {
                Some(Value::Number(y)) => n.powf(y),
                _ => return Err(SixError::at(line, "'**' power needs a number exponent")),
            },
        };
        Ok(Value::Number(result))
    }

    fn eval_binary(&mut self, op: BinOp, left: &Expr, right: &Expr, env: &Env, line: usize) -> Result<Value> {
        // Logical operators short-circuit but still require boolean operands.
        if matches!(op, BinOp::And | BinOp::Or) {
            let l = self.eval(left, env, false)?.into_value(self)?;
            let lb = self.as_bool(&l, line)?;
            match op {
                BinOp::And if !lb => return Ok(Value::Bool(false)),
                BinOp::Or if lb => return Ok(Value::Bool(true)),
                _ => {}
            }
            let r = self.eval(right, env, false)?.into_value(self)?;
            let rb = self.as_bool(&r, line)?;
            return Ok(Value::Bool(rb));
        }

        let l = self.eval(left, env, false)?.into_value(self)?;
        let r = self.eval(right, env, false)?.into_value(self)?;

        match op {
            BinOp::Add => self.eval_add(l, r, line),
            BinOp::Sub => arith(l, r, line, "-", |a, b| a - b),
            BinOp::Mul => arith(l, r, line, "*", |a, b| a * b),
            BinOp::Div => {
                if let Value::Number(b) = r {
                    if b == 0.0 {
                        return Err(SixError::at(line, "division by zero"));
                    }
                }
                arith(l, r, line, "/", |a, b| a / b)
            }
            BinOp::Mod => {
                if let Value::Number(b) = r {
                    if b == 0.0 {
                        return Err(SixError::at(line, "division by zero (modulo)"));
                    }
                }
                arith(l, r, line, "%", |a, b| a % b)
            }
            BinOp::Eq => Ok(Value::Bool(values_equal(&l, &r))),
            BinOp::Ne => Ok(Value::Bool(!values_equal(&l, &r))),
            BinOp::Lt => compare(l, r, line, |o| o.is_lt()),
            BinOp::Gt => compare(l, r, line, |o| o.is_gt()),
            BinOp::Le => compare(l, r, line, |o| o.is_le()),
            BinOp::Ge => compare(l, r, line, |o| o.is_ge()),
            BinOp::And | BinOp::Or => unreachable!(),
        }
    }

    fn eval_add(&self, l: Value, r: Value, line: usize) -> Result<Value> {
        match (&l, &r) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::Text(a), Value::Text(b)) => Ok(Value::Text(format!("{}{}", a, b))),
            (Value::Text(_), other) | (other, Value::Text(_)) => Err(SixError::at(
                line,
                format!(
                    "'+' will not mix text with {} — convert explicitly, e.g. text {}",
                    other.type_name(),
                    "x"
                ),
            )),
            (a, b) => Err(SixError::at(
                line,
                format!("'+' cannot combine {} and {}", a.type_name(), b.type_name()),
            )),
        }
    }

    // --- indexing -----------------------------------------------------------

    fn index_get(&self, base: &Value, index: &Value, line: usize) -> Result<Value> {
        match base {
            Value::Group(g) => {
                let items = g.items.borrow();
                match index {
                    Value::Number(_) | Value::Last => {
                        let i = resolve_pos(index, items.len(), line, "Group")?;
                        Ok(items[i].clone())
                    }
                    Value::Text(key) => match g.find_key(key) {
                        Some(idx) => Ok(pair_value(g, idx)),
                        None => Err(SixError::at(line, format!("no key \"{}\" in Group", key))),
                    },
                    other => Err(SixError::at(line, format!("cannot index a Group with a {}", other.type_name()))),
                }
            }
            Value::Text(s) => {
                let chars: Vec<char> = s.chars().collect();
                match index {
                    Value::Number(_) | Value::Last => {
                        let i = resolve_pos(index, chars.len(), line, "text")?;
                        Ok(Value::Text(chars[i].to_string()))
                    }
                    Value::Text(_) => Err(SixError::at(line, "text does not support keyed lookup")),
                    other => Err(SixError::at(line, format!("cannot index text with a {}", other.type_name()))),
                }
            }
            other => Err(SixError::at(line, format!("cannot index a {}", other.type_name()))),
        }
    }

    // --- assignment ---------------------------------------------------------

    fn assign(&mut self, target: &Expr, value: Value, env: &Env, line: usize) -> Result<()> {
        match target {
            Expr::Var { name, line } => self.rebind(env, name, value, *line),
            Expr::Index { base, index, line } => {
                let idx = self.eval(index, env, false)?.into_value(self)?;
                self.assign_index(base, idx, value, env, *line)
            }
            _ => Err(SixError::at(line, "invalid assignment target")),
        }
    }

    /// Rebind an existing name (`=`). This is the *binding* operation, so it
    /// enforces the name-case immutability rule (uppercase names cannot be
    /// reassigned) — distinct from ordinary keyed Group writes, which do not.
    fn rebind(&self, env: &Env, name: &str, value: Value, line: usize) -> Result<()> {
        let mut cur = Some(env.clone());
        while let Some(scope) = cur {
            let is_builtins = Rc::ptr_eq(&scope, &self.builtins);
            let store = scope.borrow().store();
            if let Some(idx) = store.find_key(name) {
                if is_builtins {
                    return Err(SixError::at(line, format!("cannot reassign builtin '{}'", name)));
                }
                if crate::parser::is_immutable_name(name) {
                    return Err(SixError::at(line, format!("cannot reassign immutable name '{}'", name)));
                }
                set_pair_value(&store, idx, value);
                return Ok(());
            }
            cur = scope.borrow().parent.clone();
        }
        Err(SixError::at(line, format!("cannot assign to undefined name '{}'", name)))
    }

    fn assign_index(&mut self, base: &Expr, idx: Value, value: Value, env: &Env, line: usize) -> Result<()> {
        let base_v = self.eval(base, env, false)?.into_value(self)?;
        match base_v {
            Value::Group(g) => {
                if g.immutable.get() {
                    return Err(SixError::at(line, "cannot mutate an immutable Group"));
                }
                self.group_set(&g, &idx, value, line)
            }
            Value::Text(_) => {
                // Text is value-semantic: rebuild the string and write it back
                // through the base lvalue (supported one level deep).
                let new_text = self.text_set(&base_v, &idx, value, line)?;
                self.assign(base, new_text, env, line)
            }
            other => Err(SixError::at(line, format!("cannot index-assign into a {}", other.type_name()))),
        }
    }

    fn group_set(&self, g: &GroupRef, idx: &Value, value: Value, line: usize) -> Result<()> {
        let mut items = g.items.borrow_mut();
        match idx {
            Value::Number(_) | Value::Last => {
                let i = resolve_pos(idx, items.len(), line, "Group")?;
                items[i] = value;
                Ok(())
            }
            Value::Text(key) => {
                // Keyed Group write. This is *not* a binding operation, so it
                // performs no name-case immutability check — `group["MAX"] = v`
                // is legal even when the Group is another program's environment.
                if let Some(pos) = items.iter().position(|it| pair_key_matches(it, key)) {
                    if let Value::Group(pair) = &items[pos] {
                        pair.items.borrow_mut()[1] = value;
                    }
                } else {
                    // Missing keyed write creates the key (spec §13).
                    items.push(Value::new_group(vec![Value::Text(key.clone()), value]));
                }
                Ok(())
            }
            other => Err(SixError::at(line, format!("cannot index-assign a Group with a {}", other.type_name()))),
        }
    }

    fn text_set(&self, base: &Value, idx: &Value, value: Value, line: usize) -> Result<Value> {
        let s = match base {
            Value::Text(s) => s,
            _ => unreachable!(),
        };
        let mut chars: Vec<char> = s.chars().collect();
        let i = resolve_pos(idx, chars.len(), line, "text")?;
        let repl = match value {
            Value::Text(t) => t,
            other => return Err(SixError::at(line, format!("can only assign text into text, not a {}", other.type_name()))),
        };
        if repl.chars().count() != 1 {
            return Err(SixError::at(line, "text position assignment expects a single character"));
        }
        chars[i] = repl.chars().next().unwrap();
        Ok(Value::Text(chars.into_iter().collect()))
    }

    // --- builtins & I/O -----------------------------------------------------

    pub fn output(&mut self, s: &str) {
        let _ = self.out.write_all(s.as_bytes());
        let _ = self.out.flush();
    }

    fn call_builtin(&mut self, name: &str, args: Vec<Value>, line: usize) -> Result<Value> {
        crate::builtins::dispatch(self, name, args, line)
    }
}

impl Flow {
    /// Force a Flow to a concrete Value, performing a pending tail call if any.
    fn into_value(self, interp: &mut Interpreter) -> Result<Value> {
        match self {
            Flow::Value(v) => Ok(v),
            Flow::Tail(func, args) => interp.call_function(func, args, 0),
        }
    }
}

// --- free helpers -----------------------------------------------------------

/// The names the runtime provides directly.
///
/// The doctrine: a *builtin* provides a capability Six cannot create for itself
/// (I/O, the size of a value, the fundamental Group mutations, keyed
/// existence). A *conversion* moves between Six's value categories. Everything
/// derivable from those — `map`, `filter`, `fold`, `find`, `split`, … — is
/// ordinary Six, written by the programmer, never primitive.
pub const BUILTINS: &[&str] = &[
    // The six capability builtins.
    "print", "input", "size", "insert", "remove", "has",
    // Conversions between value categories (associated with the value types,
    // not capabilities — the shared callable machinery is an implementation
    // detail of the language model).
    "number", "text",
];

/// The value of the `["key" value]` pair at `idx` in a store/Group (clone of
/// its second member). Scope stores are ordinary keyed Groups, so this serves
/// both name resolution and keyed Group reads.
fn pair_value(store: &GroupRef, idx: usize) -> Value {
    match &store.items.borrow()[idx] {
        Value::Group(pair) => pair.items.borrow()[1].clone(),
        _ => Value::Empty,
    }
}

/// Replace the value of the pair at `idx`.
fn set_pair_value(store: &GroupRef, idx: usize, value: Value) {
    if let Value::Group(pair) = &store.items.borrow()[idx] {
        pair.items.borrow_mut()[1] = value;
    }
}

fn arith(l: Value, r: Value, line: usize, op: &str, f: impl Fn(f64, f64) -> f64) -> Result<Value> {
    match (l, r) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(f(a, b))),
        (a, b) => Err(SixError::at(
            line,
            format!("'{}' needs two numbers, not {} and {}", op, a.type_name(), b.type_name()),
        )),
    }
}

fn compare(l: Value, r: Value, line: usize, pick: impl Fn(std::cmp::Ordering) -> bool) -> Result<Value> {
    use std::cmp::Ordering;
    let ord = match (&l, &r) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::Text(a), Value::Text(b)) => a.cmp(b),
        (a, b) => {
            return Err(SixError::at(
                line,
                format!("cannot order {} and {}", a.type_name(), b.type_name()),
            ));
        }
    };
    Ok(Value::Bool(pick(ord)))
}

/// Structural equality for simple values; Groups compare by identity (`Rc`).
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Empty, Value::Empty) => true,
        (Value::Group(x), Value::Group(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Resolve a positional (or `$`) index against a length, erroring on empties
/// and out-of-range access.
fn resolve_pos(index: &Value, len: usize, line: usize, what: &str) -> Result<usize> {
    match index {
        Value::Last => {
            if len == 0 {
                Err(SixError::at(line, format!("'$' has no final position in an empty {}", what)))
            } else {
                Ok(len - 1)
            }
        }
        Value::Number(n) => {
            if n.fract() != 0.0 {
                return Err(SixError::at(line, format!("index must be a whole number, got {}", n)));
            }
            if *n < 0.0 {
                return Err(SixError::at(line, "negative indexes are not supported"));
            }
            let i = *n as usize;
            if i >= len {
                Err(SixError::at(line, format!("index {} is out of range for a {} of size {}", i, what, len)))
            } else {
                Ok(i)
            }
        }
        other => Err(SixError::at(line, format!("cannot use a {} as an index", other.type_name()))),
    }
}
