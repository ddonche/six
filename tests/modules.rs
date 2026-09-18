//! Tests for the `@` module system (Minimal Module Runtime Specification).
//!
//! Each test writes a small module graph into a unique temp directory and runs
//! the entry file through `six::run_file_capture`, which resolves `@` imports
//! relative to the file's directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use six::run_file_capture;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory that cleans itself up when dropped.
struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("six_mod_{}_{}_{}", std::process::id(), nanos, id));
        std::fs::create_dir_all(&dir).unwrap();
        Sandbox { dir }
    }

    fn write(&self, name: &str, source: &str) {
        std::fs::write(self.dir.join(name), source).unwrap();
    }

    fn run(&self, entry: &str) -> String {
        run_file_capture(&self.dir.join(entry)).expect("program should run").1
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const HERO: &str = "name : \"Conan\"\nhealth : 100\n\n:damage(amount)\n    health = health - amount\n.\n";

#[test]
fn basic_import_and_live_binding() {
    let s = Sandbox::new();
    s.write("hero.six", HERO);
    s.write(
        "main.six",
        "@hero\nprint(hero[\"name\"])\nprint(hero[\"health\"])\nhero[\"damage\"](20)\nprint(hero[\"health\"])",
    );
    assert_eq!(s.run("main.six"), "Conan\n100\n80\n");
}

#[test]
fn external_rebinding() {
    let s = Sandbox::new();
    s.write("hero.six", HERO);
    s.write(
        "main.six",
        "@hero\nhero[\"health\"] = 50\nhero[\"damage\"](10)\nprint(hero[\"health\"])",
    );
    assert_eq!(s.run("main.six"), "40\n");
}

#[test]
fn function_retrieval_shares_environment() {
    let s = Sandbox::new();
    s.write("hero.six", HERO);
    s.write(
        "main.six",
        "@hero\ndamage : hero[\"damage\"]\ndamage(20)\ndamage(20)\nprint(hero[\"health\"])",
    );
    // The locally-bound function operates on the same hero.six environment.
    assert_eq!(s.run("main.six"), "60\n");
}

#[test]
fn collision_isolation() {
    let s = Sandbox::new();
    s.write("hero.six", HERO);
    s.write("enemy.six", "name : \"Goblin\"\nhealth : 30\n\n:damage(amount)\n    health = health - amount\n.\n");
    s.write(
        "main.six",
        "@hero\n@enemy\nhero[\"damage\"](10)\nenemy[\"damage\"](20)\nprint(hero[\"health\"])\nprint(enemy[\"health\"])",
    );
    assert_eq!(s.run("main.six"), "90\n10\n");
}

#[test]
fn nested_dependencies_load_transitively() {
    let s = Sandbox::new();
    s.write("item.six", "kind : \"sword\"\n:power()\n    10\n.\n");
    s.write("inventory.six", "@item\nslots : 8\n:describe()\n    item[\"kind\"]\n.\n");
    s.write("hero.six", "@inventory\ntitle : \"Hero\"\n");
    s.write(
        "game.six",
        "@hero\nprint(hero[\"title\"])\nprint(hero[\"inventory\"][\"describe\"]())\nprint(hero[\"inventory\"][\"item\"][\"power\"]())",
    );
    assert_eq!(s.run("game.six"), "Hero\nsword\n10\n");
}

#[test]
fn shared_module_identity() {
    let s = Sandbox::new();
    s.write("counter.six", "value : 0\n:bump()\n    value = value + 1\n.\n");
    s.write("a.six", "@counter\n:tick()\n    counter[\"bump\"]()\n.\n");
    s.write("b.six", "@counter\n:current()\n    counter[\"value\"]\n.\n");
    s.write(
        "main.six",
        "@a\n@b\n@counter\na[\"tick\"]()\ncounter[\"bump\"]()\nprint(b[\"current\"]())\nprint(a[\"counter\"] == counter)",
    );
    // a, b and the top level share one counter: two bumps => 2, same instance.
    assert_eq!(s.run("main.six"), "2\ntrue\n");
}

#[test]
fn module_size_and_has() {
    let s = Sandbox::new();
    s.write("hero.six", HERO);
    s.write(
        "main.six",
        "@hero\nprint(size(hero))\nprint(has(hero \"health\"))\nprint(has(hero \"mana\"))",
    );
    // hero.six has three top-level bindings: name, health, damage.
    assert_eq!(s.run("main.six"), "3\ntrue\nfalse\n");
}

#[test]
fn missing_module_errors() {
    let s = Sandbox::new();
    s.write("main.six", "@ghost\nprint(1)");
    let err = run_file_capture(&s.dir.join("main.six")).unwrap_err();
    assert!(err.message.contains("cannot import 'ghost'"), "got: {}", err.message);
}

// --- "@ is special; what it produces is not" --------------------------------
//
// After import, a module is an ordinary Group. These tests prove that every
// ordinary Group operation works on it, with no module-specific behavior.

#[test]
fn module_is_an_ordinary_group_positional_indexing() {
    // (2) Positional indexing works exactly as on any Group: index 0 is the
    // first ["name" value] pair, which is itself a two-member Group.
    let s = Sandbox::new();
    s.write("m.six", "name : \"Conan\"\nhealth : 100\n");
    s.write(
        "main.six",
        "@m\nfirst : m[0]\nprint(first[0])\nprint(first[1])\nprint(size(m[$]))",
    );
    assert_eq!(s.run("main.six"), "name\nConan\n2\n");
}

#[test]
fn module_opens_with_splat() {
    // (3) <module> opens the Group into separate arguments — here, its pairs.
    let s = Sandbox::new();
    s.write("m.six", "a : 1\nb : 2\n");
    s.write(
        "main.six",
        ":pair-key(p q)\n    p[0] + \" and \" + q[0]\n.\n@m\nprint(pair-key(<m>))",
    );
    assert_eq!(s.run("main.six"), "a and b\n");
}

#[test]
fn module_supports_insert_remove_size_has() {
    // (4) insert / remove / size / has all work through ordinary Group behavior.
    let s = Sandbox::new();
    s.write("m.six", "health : 100\n");
    s.write(
        "main.six",
        concat!(
            "@m\n",
            "print(size(m))\n",
            "print(has(m \"health\"))\n",
            "insert(m [\"mana\" 50])\n",   // append a new pair like any Group
            "print(size(m))\n",
            "print(m[\"mana\"])\n",
            "remove(m 0)\n",               // drop the first pair positionally
            "print(has(m \"health\"))\n",
        ),
    );
    assert_eq!(s.run("main.six"), "1\ntrue\n2\n50\nfalse\n");
}

#[test]
fn module_deep_copy_is_ordinary() {
    // (5) :: uses ordinary Group deep-copy: the copy is independent.
    let s = Sandbox::new();
    s.write("m.six", "health : 100\n");
    s.write(
        "main.six",
        "@m\nsnapshot :: m\nm[\"health\"] = 5\nprint(snapshot[\"health\"])\nprint(m[\"health\"])",
    );
    assert_eq!(s.run("main.six"), "100\n5\n");
}

#[test]
fn module_equality_is_reference_identity() {
    // (6) == uses ordinary Group reference/identity behavior.
    let s = Sandbox::new();
    s.write("m.six", "x : 1\n");
    s.write(
        "main.six",
        "@m\nalias : m\ncopy :: m\nprint(alias == m)\nprint(copy == m)",
    );
    assert_eq!(s.run("main.six"), "true\nfalse\n");
}

#[test]
fn external_uppercase_keyed_write_is_legal() {
    // (7) hero["UPPERCASE"] = value is ordinary keyed mutation and is legal,
    // even though the Group is another program's top-level environment.
    let s = Sandbox::new();
    s.write("consts.six", "MAX : 100\n");
    s.write("main.six", "@consts\nconsts[\"MAX\"] = 5\nprint(consts[\"MAX\"])");
    assert_eq!(s.run("main.six"), "5\n");
}

#[test]
fn internal_uppercase_rebind_is_illegal() {
    // (8) Internal UPPERCASE = value remains illegal binding rebinding.
    let s = Sandbox::new();
    s.write("consts.six", "MAX : 100\n\n:bump()\n    MAX = 5\n.\n");
    s.write("main.six", "@consts\nconsts[\"bump\"]()");
    let err = run_file_capture(&s.dir.join("main.six")).unwrap_err();
    assert!(err.message.contains("immutable"), "got: {}", err.message);
}

#[test]
fn builtins_are_read_only() {
    // The builtins scope is read-only runtime infrastructure.
    assert!(six::run("print = 5").unwrap_err().message.contains("builtin"));
}
