//! Six — a microscopic general-purpose programming language.
//!
//! This crate is the reference implementation of Six v0.1: lexer, parser, and
//! a tree-walking interpreter with guaranteed tail-call optimisation. Six ships
//! no standard library — the core is only the language, six builtins, and two
//! conversions; libraries live outside Six as ordinary `.six` modules used via
//! `@`. See `docs/six_spec.txt` for the authoritative language specification.

pub mod ast;
pub mod builtins;
pub mod error;
pub mod format;
pub mod host;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod repl;
pub mod token;
pub mod value;

/// The authoritative language specification, embedded in the binary.
pub const SPEC: &str = include_str!("../docs/six_spec.txt");

pub use error::{Result, SixError};
pub use interp::Interpreter;
pub use value::Value;

/// Parse `source` into a program (list of statements).
pub fn parse(source: &str) -> Result<Vec<ast::Stmt>> {
    let tokens = lexer::lex(source)?;
    parser::parse(tokens)
}

/// Run Six `source` on a fresh interpreter, returning the value of the
/// program's final statement.
pub fn run(source: &str) -> Result<Value> {
    let program = parse(source)?;
    let mut interp = Interpreter::new();
    interp.run(&program)
}

/// Run Six `source`, capturing everything the program prints as a string.
/// Handy for tests and embedding.
pub fn run_capture(source: &str) -> Result<(Value, String)> {
    use std::cell::RefCell;
    use std::rc::Rc;

    // A writer that appends into a shared buffer.
    #[derive(Clone)]
    struct SharedBuf(Rc<RefCell<Vec<u8>>>);
    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let program = parse(source)?;
    let buffer = Rc::new(RefCell::new(Vec::new()));
    let writer = SharedBuf(buffer.clone());
    let mut interp = Interpreter::with_writer(Box::new(writer));
    let value = interp.run(&program)?;
    let text = String::from_utf8_lossy(&buffer.borrow()).into_owned();
    Ok((value, text))
}

/// Run the Six program at `path`, resolving `@` imports relative to its
/// directory, and capture everything it prints.
pub fn run_file_capture(path: &std::path::Path) -> Result<(Value, String)> {
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone)]
    struct SharedBuf(Rc<RefCell<Vec<u8>>>);
    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let source = std::fs::read_to_string(path)
        .map_err(|e| SixError::new(format!("cannot read {}: {}", path.display(), e)))?;
    let program = parse(&source)?;
    let buffer = Rc::new(RefCell::new(Vec::new()));
    let mut interp = Interpreter::with_writer(Box::new(SharedBuf(buffer.clone())));
    if let Some(dir) = path.parent() {
        interp.set_base_dir(dir.to_path_buf());
    }
    let value = interp.run(&program)?;
    let text = String::from_utf8_lossy(&buffer.borrow()).into_owned();
    Ok((value, text))
}
