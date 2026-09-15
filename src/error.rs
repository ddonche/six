//! Error types shared across the Six implementation.
//!
//! Six prefers an explicit error to silently inventing meaning (spec §47), so
//! every failure — in the lexer, the parser, or the interpreter — surfaces as a
//! [`SixError`] carrying a human-readable message and (where known) a source
//! line.

use std::fmt;

/// A single error produced anywhere in the pipeline.
#[derive(Debug, Clone)]
pub struct SixError {
    pub message: String,
    pub line: Option<usize>,
}

impl SixError {
    pub fn new(message: impl Into<String>) -> Self {
        SixError { message: message.into(), line: None }
    }

    pub fn at(line: usize, message: impl Into<String>) -> Self {
        SixError { message: message.into(), line: Some(line) }
    }
}

impl fmt::Display for SixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "error (line {}): {}", line, self.message),
            None => write!(f, "error: {}", self.message),
        }
    }
}

impl std::error::Error for SixError {}

/// Convenient result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, SixError>;
