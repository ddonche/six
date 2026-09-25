//! Host I/O foundation tests: the runtime channels (input/output/error), the
//! four primitives (open/in/out/close), and the Runtime Association Law.

use six::{run, run_capture, run_capture_io};

fn out(src: &str) -> String {
    run_capture(src).expect("program should run").1
}

fn err(src: &str) -> String {
    match run(src) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.message,
    }
}

// --- runtime channels -------------------------------------------------------

#[test]
fn output_channel_writes_without_newline() {
    // out adds no newline (§E.7): two writes concatenate.
    assert_eq!(out("out(output \"a\")\nout(output \"b\")"), "ab");
}

#[test]
fn error_channel_is_separate_from_output() {
    let (_v, o, e) = run_capture_io("out(output \"normal\")\nout(error \"diag\")").unwrap();
    assert_eq!(o, "normal");
    assert_eq!(e, "diag");
}

#[test]
fn output_channel_accepts_only_text() {
    assert!(err("out(output 5)").contains("only text"));
    assert!(err("out(output [1 2])").contains("only text"));
}

#[test]
fn input_at_eof_is_empty() {
    // The test harness supplies no stdin, so in(input) sees EOF -> `..`.
    assert_eq!(out("x : in(input)\nout(output text(x == ..))"), "true");
}

// --- capability dispatch (§E.3, §44) ----------------------------------------

#[test]
fn channel_capability_mismatches_error() {
    assert!(err("in(output)").contains("output channel"));
    assert!(err("in(error)").contains("error channel"));
    assert!(err("out(input \"x\")").contains("input channel"));
    assert!(err("open(input)").contains("subordinate"));
    assert!(err("open(output)").contains("subordinate"));
}

#[test]
fn primitives_require_a_group_target() {
    assert!(err("in(5)").contains("descriptor or relationship"));
    assert!(err("out(\"hi\" \"x\")").contains("descriptor or relationship"));
    assert!(err("close(42)").contains("descriptor or relationship"));
}

// --- the Runtime Association Law (§4, §21) ----------------------------------

#[test]
fn association_follows_alias() {
    // Closing through an alias closes the relationship seen through the original.
    assert!(err("o : output\nclose(o)\nout(output \"x\")").contains("closed"));
}

#[test]
fn deep_copy_strips_association() {
    // `::` copies visible contents but not the runtime association.
    assert!(err("c :: output\nout(c \"x\")").contains("not a host relationship"));
    assert!(err("c :: input\nin(c)").contains("not a host relationship"));
}

#[test]
fn repeated_close_is_an_error() {
    assert!(err("close(output)\nclose(output)").contains("already closed"));
}

#[test]
fn use_after_close_is_an_error() {
    assert!(err("close(output)\nout(output \"x\")").contains("closed"));
    assert!(err("close(input)\nin(input)").contains("closed"));
}

#[test]
fn close_requires_a_relationship() {
    // A plain descriptor (or deep copy) has no relationship to close.
    assert!(err("close([\"file\" \"x.txt\"])").contains("not a host relationship"));
}

#[test]
fn channels_are_ordinary_rebindable_names() {
    // Rebinding the name does not disturb the original relationship (aliased).
    let src = "keep : output\noutput = []\nout(keep \"through the alias\")";
    assert_eq!(out(src), "through the alias");
}
