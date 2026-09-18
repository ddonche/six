//! An interactive Read-Eval-Print Loop for Six.
//!
//! State (names and functions) persists across inputs. The REPL reads a
//! complete construct before evaluating: a declaration or conditional spanning
//! several lines is gathered until its `.` terminator, and unbalanced
//! parentheses or a trailing operator likewise ask for another line. A blank
//! line force-submits whatever has been typed, so you are never stuck.

use std::io::{self, Write};

use crate::format;
use crate::interp::Interpreter;
use crate::lexer;
use crate::token::Tok;
use crate::value::Value;

const PROMPT: &str = "six> ";
const CONT: &str = "...  ";

pub fn run() -> i32 {
    let mut interp = Interpreter::new();

    println!("Six v{} — interactive REPL", env!("CARGO_PKG_VERSION"));
    println!("Type an expression, or a declaration ending in '.'. 'exit' or Ctrl-D to quit.");

    let stdin = io::stdin();
    let mut buffer = String::new();

    loop {
        // Prompt (continuation prompt while gathering a multi-line construct).
        print!("{}", if buffer.is_empty() { PROMPT } else { CONT });
        let _ = io::stdout().flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                // Ctrl-D / EOF.
                println!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("six: input error: {}", e);
                break;
            }
        }

        let trimmed = line.trim();

        // Top-level meta-commands (only when not mid-construct). These are REPL
        // tools, not Six: `clear` removes bindings so a name can be defined
        // afresh without pretending redeclaration is legal.
        if buffer.is_empty() {
            if trimmed.is_empty() {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            match parts.next().unwrap() {
                "exit" | "quit" => break,
                "help" => {
                    print_help();
                    continue;
                }
                "clear_all" => {
                    interp.clear_all();
                    println!("cleared the whole session");
                    continue;
                }
                "clear" => {
                    let names: Vec<&str> = parts.collect();
                    if names.is_empty() {
                        println!("usage: clear <name> [name ...]   (or clear_all to reset everything)");
                    } else {
                        for n in names {
                            if interp.clear_binding(n) {
                                println!("cleared '{}'", n);
                            } else {
                                println!("nothing named '{}'", n);
                            }
                        }
                    }
                    continue;
                }
                _ => {}
            }
        }

        let blank = trimmed.is_empty();
        buffer.push_str(&line);

        // A blank line force-submits; otherwise wait until the input is complete.
        if !blank && !input_is_complete(&buffer) {
            continue;
        }

        let source = std::mem::take(&mut buffer);
        if source.trim().is_empty() {
            continue;
        }
        evaluate(&mut interp, &source);
    }

    0
}

fn evaluate(interp: &mut Interpreter, source: &str) {
    let program = match crate::parse(source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };
    match interp.run(&program) {
        Ok(value) => {
            // Echo a useful result; suppress the empty produced by statements
            // and effect-only calls so the session stays quiet.
            if !matches!(value, Value::Empty) {
                println!("=> {}", format::repr(&value));
            }
        }
        Err(e) => eprintln!("{}", e),
    }
}

/// Decide whether the accumulated buffer forms a complete construct.
///
/// Incomplete when: parentheses/brackets are open, a block (`:decl`/`if`/`?`)
/// is still awaiting its `.`, or the last token expects a right-hand side.
fn input_is_complete(buffer: &str) -> bool {
    let tokens = match lexer::lex(buffer) {
        Ok(t) => t,
        // A lex error mid-typing (e.g. an unterminated text literal) means keep
        // gathering; a blank line will force submission if it is truly wrong.
        Err(_) => return false,
    };

    let mut depth: i32 = 0;
    let mut pending_blocks: i32 = 0;
    let mut prev: Option<&Tok> = None;
    let mut last_significant: Option<&Tok> = None;

    for t in &tokens {
        match &t.tok {
            Tok::LParen | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBracket => depth -= 1,
            Tok::If | Tok::QIf | Tok::QAny => pending_blocks += 1,
            // A statement-initial `:` opens a function declaration.
            Tok::Colon if matches!(prev, None | Some(Tok::Newline)) => pending_blocks += 1,
            // A standalone `.` (space before it) terminates a block; dot-flow
            // hugs its operand and has no space before it.
            Tok::Dot if t.space_before => pending_blocks -= 1,
            _ => {}
        }
        if t.tok != Tok::Newline && t.tok != Tok::Eof {
            last_significant = Some(&t.tok);
        }
        prev = Some(&t.tok);
    }

    if depth > 0 || pending_blocks > 0 {
        return false;
    }

    // A trailing operator (or binding/flow arrow) expects more input, so the
    // buffer is complete only when the last token is *not* one of these. A
    // buffer with no significant tokens at all (a comment or blank line) is
    // complete and simply evaluates to nothing.
    !matches!(
        last_significant,
        Some(Tok::Plus) | Some(Tok::Minus) | Some(Tok::Star) | Some(Tok::Slash)
            | Some(Tok::Percent) | Some(Tok::Lt) | Some(Tok::Gt) | Some(Tok::Le) | Some(Tok::Ge)
            | Some(Tok::EqEq) | Some(Tok::NotEq) | Some(Tok::And) | Some(Tok::Or) | Some(Tok::Not)
            | Some(Tok::FatArrow) | Some(Tok::Colon) | Some(Tok::ColonColon) | Some(Tok::Assign)
    )
}

fn print_help() {
    println!("Six REPL commands:");
    println!("  help                show this help");
    println!("  clear <name> ...    remove one or more bindings (so they can be redefined)");
    println!("  clear_all           reset the whole session (keeps builtins)");
    println!("  exit / quit         leave the REPL (also Ctrl-D)");
    println!();
    println!("Redeclaring a name with ':' is an error, exactly as in a .six file;");
    println!("use '=' to change a value, or 'clear' to redefine a function.");
    println!();
    println!("Enter Six directly. A blank line force-submits a partial entry.");
    println!("Examples:");
    println!("  2 + 3 * 4");
    println!("  x : [1 2 3]");
    println!("  print(x[$])");
    println!("  :square(n)");
    println!("      n * n");
    println!("  .");
    println!("  square(9)");
}
