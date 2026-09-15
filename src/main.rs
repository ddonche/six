//! The `six` command-line runner.
//!
//! Usage:
//!   six <file.six>     run a program
//!   six run <file.six> run a program
//!   six --version      print the version
//!   six --help         print usage

use std::process::ExitCode;

use six::Interpreter;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rest = &args[1..];

    match rest.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some("--version") | Some("-V") => {
            println!("six {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("run") => match rest.get(1) {
            Some(path) => run_file(path),
            None => {
                eprintln!("six: 'run' needs a file path");
                ExitCode::FAILURE
            }
        },
        Some(path) => run_file(path),
    }
}

fn run_file(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("six: cannot read {}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };

    let program = match six::parse(&source) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("six: {}", e);
            return ExitCode::FAILURE;
        }
    };

    let mut interp = Interpreter::new();
    if let Err(e) = interp.load_prelude() {
        eprintln!("six: internal prelude error: {}", e);
        return ExitCode::FAILURE;
    }
    match interp.run(&program) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("six: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!("Six v{} — a microscopic general-purpose language", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Usage:");
    println!("  six <file.six>       run a program");
    println!("  six run <file.six>   run a program");
    println!("  six --version        print the version");
    println!("  six --help           print this help");
}
