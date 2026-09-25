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

// --- network domain (Addendum B) --------------------------------------------

/// A unique loopback port per test (pid-based base avoids TIME_WAIT across runs).
fn next_port() -> u16 {
    static C: AtomicUsize = AtomicUsize::new(0);
    let base = 20000 + (std::process::id() as usize % 20000);
    (base + C.fetch_add(1, Ordering::SeqCst)) as u16
}

#[test]
fn tcp_text_loopback_both_directions() {
    let p = next_port();
    let src = format!(
        "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n\
         c : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])\n\
         s : open(l)\n\
         out(c \"ping\")\n\
         out(output in(s))\n\
         out(s \"pong\")\n\
         out(output in(c))\n\
         close(c)\nclose(s)\nclose(l)"
    );
    assert_eq!(out(&src), "pingpong");
}

#[test]
fn tcp_eof_is_empty() {
    let p = next_port();
    // Server accepts then closes; the client sees EOF (`..`), not "".
    let src = format!(
        "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n\
         c : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])\n\
         s : open(l)\n\
         close(s)\n\
         out(output text(in(c) == ..))\n\
         close(c)\nclose(l)"
    );
    assert_eq!(out(&src), "true");
}

#[test]
fn tcp_binary_round_trip() {
    let p = next_port();
    let src = format!(
        "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"binary\"])\n\
         c : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"binary\"])\n\
         s : open(l)\n\
         out(c [104 105])\n\
         d : in(s)\n\
         out(output text(d[0]) + \",\" + text(d[1]))\n\
         close(c)\nclose(s)\nclose(l)"
    );
    assert_eq!(out(&src), "104,105");
}

#[test]
fn tcp_listener_capabilities() {
    let p = next_port();
    // A listener supports open/close but not in/out.
    let mk = |op: &str| {
        format!(
            "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n{op}"
        )
    };
    assert!(err(&mk("in(l)")).contains("listener does not support in"));
    assert!(err(&mk("out(l \"x\")")).contains("listener does not support out"));
}

#[test]
fn tcp_connection_has_no_subordinate_open() {
    let p = next_port();
    let src = format!(
        "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n\
         c : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])\n\
         open(c)"
    );
    assert!(err(&src).contains("does not establish a subordinate"));
}

#[test]
fn accepted_connection_survives_listener_close() {
    let p = next_port();
    let src = format!(
        "l : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n\
         c : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])\n\
         s : open(l)\n\
         close(l)\n\
         out(c \"still works\")\n\
         out(output in(s))\n\
         close(c)\nclose(s)"
    );
    assert_eq!(out(&src), "still works");
}

#[test]
fn network_descriptor_must_be_opened() {
    let p = next_port();
    assert!(err(&format!("in([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])")).contains("open"));
    assert!(err(&format!("out([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"] \"x\")")).contains("open"));
}

#[test]
fn udp_text_loopback_with_source() {
    let a = next_port();
    let b = next_port();
    let src = format!(
        "sa : open([\"network\" \"udp\" \"127.0.0.1\" {a} \"text\"])\n\
         sb : open([\"network\" \"udp\" \"127.0.0.1\" {b} \"text\"])\n\
         out(sa [\"127.0.0.1\" {b} \"hi udp\"])\n\
         p : in(sb)\n\
         out(output text(p[1]) + \":\" + p[2])\n\
         close(sa)\nclose(sb)"
    );
    assert_eq!(out(&src), format!("{a}:hi udp"));
}

#[test]
fn udp_binary_datagram() {
    let a = next_port();
    let b = next_port();
    let src = format!(
        "sa : open([\"network\" \"udp\" \"127.0.0.1\" {a} \"binary\"])\n\
         sb : open([\"network\" \"udp\" \"127.0.0.1\" {b} \"binary\"])\n\
         out(sa [\"127.0.0.1\" {b} [1 2 3]])\n\
         p : in(sb)\n\
         d : p[2]\n\
         out(output text(size(d)) + \":\" + text(d[0]) + text(d[2]))\n\
         close(sa)\nclose(sb)"
    );
    assert_eq!(out(&src), "3:13");
}

#[test]
fn udp_malformed_output_errors() {
    let a = next_port();
    let src = format!(
        "sa : open([\"network\" \"udp\" \"127.0.0.1\" {a} \"text\"])\nout(sa [\"127.0.0.1\" \"badport\" \"x\"])"
    );
    assert!(err(&src).contains("port"));
}

// --- process domain (Addendum C) --------------------------------------------
//
// The library interpreter carries no program arguments, so
// in(["process" "arguments"]) is the empty Group in these tests.

/// A unique environment variable name, so parallel tests never collide on the
/// shared process environment.
fn env_name(tag: &str) -> String {
    static ENV_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = ENV_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("SIX_TEST_{}_{}_{}", tag, std::process::id(), id)
}

#[test]
fn process_arguments_is_empty_group() {
    // No args supplied to the library interpreter -> [].
    assert_eq!(out("out(output text(size(in([\"process\" \"arguments\"]))))"), "0");
    // Arguments are observational; writing them is invalid.
    assert!(err("out([\"process\" \"arguments\"] [])").contains("observational"));
}

#[test]
fn process_directory_is_nonempty_text_and_settable() {
    // The working directory reads back as non-empty Text.
    assert_eq!(out("d : in([\"process\" \"directory\"])\nout(output text(size(d) > 0))"), "true");
    // Setting it to its current value exercises the out path without moving the
    // shared process cwd (which parallel tests depend on).
    assert_eq!(
        out("d : in([\"process\" \"directory\"])\nr : out([\"process\" \"directory\"] d)\nout(output text(r == ..))"),
        "true"
    );
    assert!(err("out([\"process\" \"directory\"] 5)").contains("must be text"));
}

#[test]
fn process_environment_missing_set_and_remove() {
    let name = env_name("VAR");
    // Missing variable -> ..
    let missing = format!("out(output text(in([\"process\" \"environment\" \"{name}\"]) == ..))");
    assert_eq!(out(&missing), "true");
    // Set then read back.
    let set = format!(
        "r : out([\"process\" \"environment\" \"{name}\"] \"hello\")\nout(output in([\"process\" \"environment\" \"{name}\"]))"
    );
    assert_eq!(out(&set), "hello");
    // Remove (out ..) then read -> ..
    let remove = format!(
        "out([\"process\" \"environment\" \"{name}\"] \"x\")\nout([\"process\" \"environment\" \"{name}\"] ..)\nout(output text(in([\"process\" \"environment\" \"{name}\"]) == ..))"
    );
    assert_eq!(out(&remove), "true");
}

#[test]
fn process_environment_whole_is_group_of_pairs() {
    let name = env_name("WHOLE");
    // A variable we set is visible in the whole-environment listing.
    let src = format!(
        "out([\"process\" \"environment\" \"{name}\"] \"present\")\n\
         env : in([\"process\" \"environment\"])\n\
         :look(g i)\n\
             if\n\
                 i == size(g) >> \"\"\n\
                 g[i][0] == \"{name}\" >> g[i][1]\n\
                 else >> look(g i + 1)\n\
             .\n\
         .\n\
         out(output look(env 0))"
    );
    assert_eq!(out(&src), "present");
}

#[test]
fn process_environment_value_must_be_text_or_empty() {
    let name = env_name("BAD");
    assert!(err(&format!("out([\"process\" \"environment\" \"{name}\"] 5)")).contains("must be text"));
}

#[test]
fn process_current_descriptor_rejects_open_and_close() {
    // ["process"] identifies the current process for exit; open/close/in are invalid.
    assert!(err("open([\"process\" \"arguments\"])").contains("child process descriptor"));
    assert!(err("close([\"process\" \"directory\"])").contains("not a host relationship"));
    assert!(err("in([\"process\"])").contains("out"));
}

#[test]
fn process_exit_validates_code() {
    // A non-integer / negative / non-number code errors before any exit occurs.
    assert!(err("out([\"process\"] 1.5)").contains("non-negative integer"));
    assert!(err("out([\"process\"] (0 - 1))").contains("non-negative integer"));
    assert!(err("out([\"process\"] \"ok\")").contains("must be a number"));
}

#[test]
fn child_text_round_trip_and_exit_status() {
    let src = "child : open([\"process\" \"cat\" [] \"text\"])\n\
               w : out(child[\"input\"] \"hello world\")\n\
               c : close(child[\"input\"])\n\
               line : in(child[\"output\"])\n\
               out(output line)\n\
               out(output \" \")\n\
               out(output text(in(child)))\n\
               z : close(child)";
    assert_eq!(out(src), "hello world 0");
}

#[test]
fn child_stderr_and_nonzero_exit() {
    let src = "child : open([\"process\" \"sh\" [\"-c\" \"echo oops 1>&2 ; exit 3\"] \"text\"])\n\
               c : close(child[\"input\"])\n\
               e : in(child[\"error\"])\n\
               out(output e)\n\
               out(output text(in(child)))\n\
               z : close(child)";
    // stderr text ends with a newline from echo.
    assert_eq!(out(src), "oops\n3");
}

#[test]
fn child_binary_output() {
    let src = "child : open([\"process\" \"printf\" [\"ABC\"] \"binary\"])\n\
               c : close(child[\"input\"])\n\
               bytes : in(child[\"output\"])\n\
               out(output text(bytes[0]))\n\
               out(output \" \")\n\
               out(output text(size(bytes)))\n\
               z : close(child)";
    assert_eq!(out(src), "65 3");
}

#[test]
fn child_output_eof_is_empty() {
    let src = "child : open([\"process\" \"true\" [] \"text\"])\n\
               c : close(child[\"input\"])\n\
               first : in(child[\"output\"])\n\
               out(output text(first == ..))\n\
               z : close(child)";
    assert_eq!(out(src), "true");
}

#[test]
fn child_terminate_yields_abnormal_status() {
    // A killed child has no normal numeric exit status -> in(child) is ..
    let src = "child : open([\"process\" \"sleep\" [\"30\"] \"text\"])\n\
               k : out(child ..)\n\
               out(output text(in(child) == ..))\n\
               z : close(child)";
    assert_eq!(out(src), "true");
}

#[test]
fn child_environment_override_applies() {
    let name = env_name("CHILD");
    // The child echoes exactly the overridden variable, so a single read of its
    // output is the whole value.
    let src = format!(
        "child : open([\"process\" \"sh\" [\"-c\" \"printf %s \\\"${name}\\\"\"] \"text\" [[\"environment\" [[\"{name}\" \"42\"]]]]])\n\
         c : close(child[\"input\"])\n\
         v : in(child[\"output\"])\n\
         s : in(child)\n\
         out(output v)"
    );
    assert_eq!(out(&src), "42");
}

#[test]
fn child_channel_capability_errors() {
    // in on input, out on output/error are all invalid (capability table C.26).
    let base = "child : open([\"process\" \"cat\" [] \"text\"])\n";
    assert!(err(&format!("{base}in(child[\"input\"])")).contains("child input channel does not support in"));
    assert!(err(&format!("{base}out(child[\"output\"] \"x\")")).contains("child output channel does not support out"));
    assert!(err(&format!("{base}out(child[\"error\"] \"x\")")).contains("child error channel does not support out"));
    // Only .. terminates the child lifecycle relationship.
    assert!(err(&format!("{base}out(child \"x\")")).contains("terminate"));
}

#[test]
fn child_options_are_validated() {
    assert!(
        err("open([\"process\" \"cat\" [] \"text\" [[\"directory\" \"/a\"] [\"directory\" \"/b\"]]])")
            .contains("duplicate option")
    );
    assert!(err("open([\"process\" \"cat\" [] \"text\" [[\"bogus\" \"x\"]]])").contains("unknown option"));
    assert!(err("open([\"process\" \"cat\" [] \"text\" [[\"directory\"]]])").contains("two-element Group"));
    assert!(
        err("open([\"process\" \"cat\" [] \"text\" [[\"environment\" [[\"A\" \"1\"] [\"A\" \"2\"]]]]])")
            .contains("duplicate environment override")
    );
}

#[test]
fn child_descriptor_validation() {
    assert!(err("open([\"process\" 5 [] \"text\"])").contains("program must be text"));
    assert!(err("open([\"process\" \"cat\" [1 2] \"text\"])").contains("each argument must be text"));
    assert!(err("open([\"process\" \"cat\" [] \"other\"])").contains("representation"));
    // An unopened child descriptor is not a direct in/out target.
    assert!(err("in([\"process\" \"cat\" [] \"text\"])").contains("open a child"));
    assert!(err("out([\"process\" \"cat\" [] \"text\"] \"x\")").contains("open a child"));
}

#[test]
fn child_deep_copy_strips_associations() {
    // A deep copy of a child keeps visible channel Groups but none of their
    // runtime associations (C.24).
    let src = "child : open([\"process\" \"cat\" [] \"text\"])\n\
               copy :: child\n\
               out(copy[\"input\"] \"x\")";
    assert!(err(src).contains("not a host relationship"));
}

#[test]
fn child_channels_outlive_lifecycle_close() {
    // close(child) releases only the lifecycle relationship; a separately held
    // output channel remains readable (C.22).
    let src = "child : open([\"process\" \"printf\" [\"hi\"] \"text\"])\n\
               o : child[\"output\"]\n\
               c : close(child[\"input\"])\n\
               z : close(child)\n\
               out(output in(o))";
    assert_eq!(out(src), "hi");
}
