//! Acceptance suite for the frozen Six surface.
//!
//! One holistic pass over the whole contract: the twelve builtins (six core,
//! six host), the absence of `print`/`input`, and a representative end-to-end
//! program for every host capability — the runtime channels, entropy/time, and
//! the file, network, process, and device domains. The finer-grained behavior
//! of each domain lives in tests/host.rs; this file is the spec-level check
//! that the pieces are present and cohere.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use six::{run, run_capture, run_capture_io, Value};

/// The rendered value of a program's final statement.
fn shown(src: &str) -> String {
    six::format::display(&run(src).expect("program should run"))
}

/// Everything a program wrote to the output channel.
fn printed(src: &str) -> String {
    run_capture(src).expect("program should run").1
}

fn err(src: &str) -> String {
    match run(src) {
        Ok(_) => panic!("expected an error but the program succeeded"),
        Err(e) => e.message,
    }
}

// --- the frozen builtin surface ---------------------------------------------

#[test]
fn six_core_builtins_are_present() {
    // size, insert, remove, has, number, text.
    assert_eq!(shown("size([1 2 3])"), "3");
    assert_eq!(shown("g : [1 2]\ninsert(g 3)\ng"), "[1 2 3]");
    assert_eq!(shown("g : [1 2 3]\nremove(g 0)\ng"), "[2 3]");
    assert_eq!(shown("has([[\"k\" 1]] \"k\")"), "true");
    assert_eq!(shown("number(\"42\")"), "42");
    assert_eq!(shown("text(42)"), "42");
}

#[test]
fn six_host_builtins_are_present() {
    // open, in, out, close, entropy, time — each reachable and behaving.
    assert_eq!(printed("out(output \"hi\")"), "hi");
    assert_eq!(shown("x : in(input)\nx == .."), "true"); // EOF at the harness
    assert!(matches!(run("entropy()").unwrap(), Value::Number(_)));
    assert!(matches!(run("time(\"utc\")").unwrap(), Value::Number(_)));
    // open/close operate on relationships; opening the input channel is an error
    // (proof the primitive is wired), and closing output is accepted.
    assert!(err("open(input)").contains("subordinate"));
    assert_eq!(shown("close(output)\n.."), "nil");
}

#[test]
fn print_and_input_are_not_builtins() {
    // print is gone entirely; `input` is the runtime channel Group, not a
    // callable — calling it is a type error, not a name lookup.
    assert!(err("print(\"x\")").contains("undefined name 'print'"));
    assert!(err("input(\"x\")").contains("cannot call a Group"));
}

// --- runtime channels & the association law ---------------------------------

#[test]
fn runtime_channels_and_association_law() {
    // output and error are distinct; neither reads.
    let (_v, o, e) = run_capture_io("out(output \"a\")\nout(error \"b\")").unwrap();
    assert_eq!((o.as_str(), e.as_str()), ("a", "b"));
    assert!(err("in(output)").contains("output channel"));
    // Aliasing shares the relationship; :: deep-copy strips it; == is identity.
    assert!(err("o : output\nclose(o)\nout(output \"x\")").contains("closed"));
    assert!(err("c :: output\nout(c \"x\")").contains("not a host relationship"));
    assert_eq!(shown("output == output"), "true");
    assert_eq!(shown("c :: output\nc == output"), "false");
}

// --- entropy & time ---------------------------------------------------------

#[test]
fn entropy_and_time_observations() {
    // entropy is an integer in range; steady time never goes backwards.
    match run("entropy()").unwrap() {
        Value::Number(n) => assert!(n.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&n)),
        other => panic!("entropy should be a number, got {:?}", other),
    }
    assert_eq!(shown("a : time(\"steady\")\nb : time(\"steady\")\nb - a >= 0"), "true");
    assert!(err("time(\"bogus\")").contains("utc"));
}

// --- file domain ------------------------------------------------------------

struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Self {
        static C: AtomicUsize = AtomicUsize::new(0);
        let id = C.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("six_accept_{}_{}_{}_{}", tag, std::process::id(), nanos, id));
        std::fs::create_dir_all(&path).unwrap();
        Dir { path }
    }
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
fn file_domain_end_to_end() {
    let d = Dir::new("file");
    let f = d.lit("note.txt");
    // write, read back, then remove and confirm absence — all through in/out.
    let src = format!(
        "out([\"file\" {f} \"text\"] \"hello\")\n\
         seen : in([\"file\" {f} \"text\"])\n\
         out([\"file\" {f}] ..)\n\
         gone : in([\"file\" {f} \"text\"])\n\
         out(output seen)\n\
         out(output text(gone == ..))"
    );
    assert_eq!(printed(&src), "hellotrue");
}

// --- network domain ---------------------------------------------------------

fn next_port() -> u16 {
    static C: AtomicUsize = AtomicUsize::new(0);
    let n = C.fetch_add(1, Ordering::SeqCst);
    20000 + ((std::process::id() as usize % 20000) + n) as u16 % 20000
}

#[test]
fn network_domain_end_to_end() {
    let p = next_port();
    // Listener + loopback client, one text message each way, in a single process.
    let src = format!(
        "server : open([\"network\" \"tcp\" \"listener\" \"127.0.0.1\" {p} \"text\"])\n\
         client : open([\"network\" \"tcp\" \"127.0.0.1\" {p} \"text\"])\n\
         session : open(server)\n\
         out(client \"ping\")\n\
         got : in(session)\n\
         out(output got)\n\
         close(client)\n\
         close(session)\n\
         close(server)"
    );
    assert_eq!(printed(&src), "ping");
}

// --- process domain ---------------------------------------------------------

#[test]
fn process_domain_end_to_end() {
    // Current-process arguments observation (empty in the library), plus a child
    // round-trip: write to stdin, read stdout, await a zero exit.
    assert_eq!(shown("size(in([\"process\" \"arguments\"]))"), "0");
    let src = "child : open([\"process\" \"cat\" [] \"text\"])\n\
               out(child[\"input\"] \"echo\")\n\
               close(child[\"input\"])\n\
               out(output in(child[\"output\"]))\n\
               out(output text(in(child)))\n\
               close(child)";
    assert_eq!(printed(src), "echo0");
}

// --- device domain ----------------------------------------------------------

#[test]
fn device_domain_end_to_end() {
    // Discovery is in-only and empty in v1; opening a kind reports no adapter.
    assert_eq!(shown("size(in([\"device\"]))"), "0");
    assert!(err("open([\"device\" \"serial\" \"serial-0\"])").contains("unknown device adapter"));
}

// --- a whole userland program on the frozen surface -------------------------

#[test]
fn a_complete_program_runs_on_the_frozen_surface() {
    // No print, no input, no standard library: userland map + a line writer over
    // the output channel, producing observable output.
    let src = "\
:line(s)\n\
    out(output s)\n\
    out(output \"\\n\")\n\
.\n\
:map(list f i result)\n\
    if\n\
        i >= size(list) >> result\n\
        else >>\n\
            insert(result f(list[i]))\n\
            map(list f (i + 1) result)\n\
    .\n\
.\n\
:double(x)\n\
    x * 2\n\
.\n\
doubled : map([1 2 3] double 0 [])\n\
line(text(doubled[0]))\n\
line(text(doubled[1]))\n\
line(text(doubled[2]))";
    assert_eq!(printed(src), "2\n4\n6\n");
}
