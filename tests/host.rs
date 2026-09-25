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

// --- file domain (Addendum A) -----------------------------------------------

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A self-cleaning temp directory for filesystem tests.
struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new() -> Self {
        let id = FILE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("six_file_{}_{}_{}", std::process::id(), nanos, id));
        std::fs::create_dir_all(&path).unwrap();
        Dir { path }
    }
    /// A path inside the sandbox, as a Six text literal.
    fn lit(&self, name: &str) -> String {
        format!("\"{}\"", self.path.join(name).to_string_lossy().replace('\\', "\\\\"))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn file_text_write_read_and_replace() {
    let d = Dir::new();
    let f = d.lit("book.txt");
    assert_eq!(out(&format!("out([\"file\" {f} \"text\"] \"abc\")\nout(output in([\"file\" {f} \"text\"]))")), "abc");
    // Replacement establishes complete contents.
    assert_eq!(out(&format!("out([\"file\" {f} \"text\"] \"xyz\")\nout(output in([\"file\" {f} \"text\"]))")), "xyz");
}

#[test]
fn file_missing_read_is_empty() {
    let d = Dir::new();
    let f = d.lit("nope.txt");
    assert_eq!(out(&format!("out(output text(in([\"file\" {f} \"text\"]) == ..))")), "true");
    assert_eq!(out(&format!("out(output text(in([\"file\" {f} \"binary\"]) == ..))")), "true");
    assert_eq!(out(&format!("out(output text(in([\"file\" {f} \"metadata\"]) == ..))")), "true");
    assert_eq!(out(&format!("out(output text(in([\"file\" {f} \"directory\"]) == ..))")), "true");
}

#[test]
fn file_delete_and_delete_absent() {
    let d = Dir::new();
    let f = d.lit("gone.txt");
    let prog = format!(
        "out([\"file\" {f} \"text\"] \"data\")\nout([\"file\" {f}] ..)\nout(output text(in([\"file\" {f} \"text\"]) == ..))"
    );
    assert_eq!(out(&prog), "true");
    // Deleting an already-absent entry succeeds (desired state already holds).
    let g = d.lit("never.txt");
    assert_eq!(out(&format!("out([\"file\" {g}] ..)")), "");
}

#[test]
fn file_binary_round_trip_and_validation() {
    let d = Dir::new();
    let f = d.lit("data.bin");
    let prog = format!(
        "out([\"file\" {f} \"binary\"] [0 1 255])\nd : in([\"file\" {f} \"binary\"])\nout(output text(d[0]) + \",\" + text(d[1]) + \",\" + text(d[2]) + \",\" + text(size(d)))"
    );
    assert_eq!(out(&prog), "0,1,255,3");
    // Empty binary file round-trips to [].
    let e = d.lit("empty.bin");
    assert_eq!(out(&format!("out([\"file\" {e} \"binary\"] [])\nout(output text(size(in([\"file\" {e} \"binary\"]))))")), "0");
    // Invalid byte values are errors.
    assert!(err(&format!("out([\"file\" {f} \"binary\"] [256])")).contains("0..255"));
    assert!(err(&format!("out([\"file\" {f} \"binary\"] [(0 - 1)])")).contains("0..255"));
    assert!(err(&format!("out([\"file\" {f} \"binary\"] [1.5])")).contains("integer"));
    assert!(err(&format!("out([\"file\" {f} \"binary\"] [\"x\"])")).contains("number"));
}

#[test]
fn file_constructed_append() {
    let d = Dir::new();
    let f = d.lit("log.txt");
    let prog = format!(
        "out([\"file\" {f} \"text\"] \"one\")\nc : in([\"file\" {f} \"text\"])\nc = c + \"-two\"\nout([\"file\" {f} \"text\"] c)\nout(output in([\"file\" {f} \"text\"]))"
    );
    assert_eq!(out(&prog), "one-two");
}

#[test]
fn file_directory_list_create_and_nonempty_delete() {
    let d = Dir::new();
    let sub = d.lit("sub");
    let a = d.lit("sub/a.txt");
    let b = d.lit("sub/b.txt");
    let dir = d.lit("sub");
    let setup = format!("out([\"file\" {sub} \"directory\"] [])\nout([\"file\" {a} \"text\"] \"A\")\nout([\"file\" {b} \"text\"] \"B\")\n");
    // Listing returns both names in some order.
    let list = format!(
        "{setup}:has-name(g name i)\n    if\n        i >= size(g) >> false\n        g[i] == name >> true\n        else >> has-name(g name (i + 1))\n    .\n.\ne : in([\"file\" {dir} \"directory\"])\nout(output text(size(e)) + text(has-name(e \"a.txt\" 0)) + text(has-name(e \"b.txt\" 0)))"
    );
    assert_eq!(out(&list), "2truetrue");
    // A non-empty directory cannot be removed.
    assert!(err(&format!("out([\"file\" {dir}] ..)")).contains("cannot remove"));
}

#[test]
fn file_metadata() {
    let d = Dir::new();
    let f = d.lit("m.txt");
    let prog = format!(
        "out([\"file\" {f} \"text\"] \"hello\")\nm : in([\"file\" {f} \"metadata\"])\nout(output m[\"kind\"] + \",\" + text(m[\"size\"]) + \",\" + text(m[\"readonly\"]))"
    );
    assert_eq!(out(&prog), "file,5,false");
}

#[test]
fn file_rename_and_collision() {
    let d = Dir::new();
    let src = d.lit("draft.txt");
    let dst = d.lit("final.txt");
    let prog = format!(
        "out([\"file\" {src} \"text\"] \"content\")\nout([\"file\" {src} \"path\"] {dst})\nout(output text(in([\"file\" {src} \"text\"]) == ..) + in([\"file\" {dst} \"text\"]))"
    );
    // source gone (true), destination has the content
    assert_eq!(out(&prog), "truecontent");
    // Renaming onto an existing target fails.
    let src2 = d.lit("a.txt");
    let dst2 = d.lit("b.txt");
    let coll = format!("out([\"file\" {src2} \"text\"] \"a\")\nout([\"file\" {dst2} \"text\"] \"b\")\nout([\"file\" {src2} \"path\"] {dst2})");
    assert!(err(&coll).contains("already exists"));
}

#[test]
fn file_open_and_bare_in_are_rejected() {
    let d = Dir::new();
    let f = d.lit("x.txt");
    assert!(err(&format!("open([\"file\" {f} \"text\"])")).contains("one-shot"));
    assert!(err(&format!("in([\"file\" {f}])")).contains("not valid"));
}
