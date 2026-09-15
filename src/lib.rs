//! Six — a microscopic general-purpose programming language.
//!
//! This crate is the reference implementation of Six v0.1: lexer, parser,
//! tree-walking interpreter with guaranteed tail-call optimisation, and a
//! deliberately small standard library. See `docs/six_spec.txt` for the
//! authoritative language specification.

pub mod ast;
pub mod builtins;
pub mod error;
pub mod format;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod prelude;
pub mod token;
pub mod value;

pub use error::{Result, SixError};
pub use interp::Interpreter;
pub use value::Value;

/// Parse `source` into a program (list of statements).
pub fn parse(source: &str) -> Result<Vec<ast::Stmt>> {
    let tokens = lexer::lex(source)?;
    parser::parse(tokens)
}

/// Run Six `source` on a fresh interpreter (prelude loaded), returning the value
/// of the program's final statement.
pub fn run(source: &str) -> Result<Value> {
    let program = parse(source)?;
    let mut interp = Interpreter::new();
    interp.load_prelude()?;
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
    interp.load_prelude()?;
    let value = interp.run(&program)?;
    let text = String::from_utf8_lossy(&buffer.borrow()).into_owned();
    Ok((value, text))
}
