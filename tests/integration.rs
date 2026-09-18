//! Integration tests exercising Six's semantics against the canonical syntax
//! (see `examples/tiny_inventory.six`).
//!
//! Each test runs a Six program and checks its printed output or the value of
//! its final statement, covering the whole pipeline (lexer → parser →
//! interpreter).

use six::{run, run_capture, Value};

fn out(src: &str) -> String {
    run_capture(src).expect("program should run").1
}

fn err(src: &str) -> String {
    match run(src) {
        Ok(_) => panic!("expected an error but the program succeeded"),
        Err(e) => e.message,
    }
}

fn num(src: &str) -> f64 {
    match run(src).expect("program should run") {
        Value::Number(n) => n,
        other => panic!("expected a number, got {:?}", other),
    }
}

// --- values & names ---------------------------------------------------------

#[test]
fn numbers_and_thousands_separators() {
    assert_eq!(num("1,000"), 1000.0);
    assert_eq!(num("1,000,000"), 1_000_000.0);
    assert_eq!(num("12,345.67"), 12_345.67);
}

#[test]
fn malformed_thousands_separator_errors() {
    assert!(err("x : 10,00").contains("numeric separator"));
    assert!(err("x : 1,00,000").contains("numeric separator"));
}

#[test]
fn new_binding_then_rebind() {
    assert_eq!(out("x : 5\nx = 7\nprint(x)"), "7\n");
}

#[test]
fn redeclaring_in_same_scope_errors() {
    assert!(err("x : 5\nx : 6").contains("already defined"));
}

#[test]
fn rebinding_undefined_errors() {
    assert!(err("x = 7").contains("undefined"));
}

#[test]
fn reading_nonexistent_name_errors() {
    assert!(err("print(y)").contains("undefined name 'y'"));
}

#[test]
fn immutable_uppercase_names() {
    assert!(err("PI : 3.14159\nPI = 3").contains("immutable"));
    assert!(err("WHITE : [255 255 255]\ninsert(WHITE 0)").contains("immutable"));
}

// --- empty vs nonexistent ---------------------------------------------------

#[test]
fn empty_value_exists() {
    assert_eq!(out("x : ..\nprint(x)"), "nil\n");
    assert_eq!(out("x : nil\nprint(x)"), "nil\n");
}

#[test]
fn typed_empty_forms() {
    assert_eq!(num("size(number ..)"), 0.0);
    assert_eq!(num("size(text ..)"), 0.0);
    assert_eq!(out("x : number ..\nprint(x == ..)"), "true\n");
}

#[test]
fn size_of_empties_is_zero() {
    assert_eq!(num("size(..)"), 0.0);
    assert_eq!(num("size(nil)"), 0.0);
    assert_eq!(num("size([])"), 0.0);
}

// --- text -------------------------------------------------------------------

#[test]
fn text_is_positionally_indexable() {
    assert_eq!(out("name : \"Dan\"\nprint(name[0])"), "D\n");
    assert_eq!(out("name : \"Dan\"\nprint(name[$])"), "n\n");
}

#[test]
fn text_keyed_lookup_errors() {
    assert!(err("name : \"Dan\"\nprint(name[\"first\"])").contains("keyed lookup"));
}

#[test]
fn text_position_assignment() {
    assert_eq!(out("s : \"cat\"\ns[0] = \"b\"\nprint(s)"), "bat\n");
}

#[test]
fn concatenation_requires_matching_types() {
    assert_eq!(out("print(\"Hello, \" + \"Dan\")"), "Hello, Dan\n");
    assert!(err("print(\"Age: \" + 46)").contains("mix text"));
    assert_eq!(out("print(\"Age: \" + text(46))"), "Age: 46\n");
}

// --- reference vs value semantics -------------------------------------------

#[test]
fn groups_are_reference_semantic() {
    assert_eq!(out("a : [1 2 3]\nb : a\ninsert(b 4)\nprint(a)"), "[1 2 3 4]\n");
}

#[test]
fn deep_copy_is_independent() {
    assert_eq!(out("a : [1 2 3]\nb :: a\ninsert(b 4)\nprint(a)\nprint(b)"), "[1 2 3]\n[1 2 3 4]\n");
}

#[test]
fn simple_values_are_value_semantic() {
    let src = ":bump(n)\n    n = n + 1\n    n\n.\nx : 5\nprint(bump(x))\nprint(x)";
    assert_eq!(out(src), "6\n5\n");
}

// --- groups & indexing ------------------------------------------------------

#[test]
fn positional_index_out_of_range_errors() {
    assert!(err("g : [1 2 3]\nprint(g[5])").contains("out of range"));
}

#[test]
fn final_position_on_empty_errors() {
    assert!(err("print([][$])").contains("final position"));
}

#[test]
fn negative_index_errors() {
    assert!(err("g : [1 2 3]\nprint(g[0 - 1])").contains("negative"));
}

// --- keyed groups -----------------------------------------------------------

#[test]
fn keyed_lookup_and_first_match_wins() {
    assert_eq!(out("p : [[\"k\" 1] [\"k\" 2]]\nprint(p[\"k\"])"), "1\n");
}

#[test]
fn missing_keyed_read_errors_but_write_creates() {
    assert!(err("p : [[\"a\" 1]]\nprint(p[\"b\"])").contains("no key"));
    assert_eq!(out("p : [[\"a\" 1]]\np[\"b\"] = 2\nprint(p[\"b\"])"), "2\n");
}

#[test]
fn has_distinguishes_existence_from_emptiness() {
    let src = "p : [[\"mid\" (text ..)]]\nprint(has?(p \"mid\"))\nprint(has?(p \"nope\"))";
    assert_eq!(out(src), "true\nfalse\n");
}

#[test]
fn nested_keyed_assignment_by_reference() {
    let src = "g : [[[\"qty\" 1]]]\ng[0][\"qty\"] = 5\nprint(g[0][\"qty\"])";
    assert_eq!(out(src), "5\n");
}

// --- open / splat -----------------------------------------------------------

#[test]
fn splat_opens_a_group_into_arguments() {
    let src = ":add3(a b c)\n    a + b + c\n.\nnums : [1 2 3]\nprint(add3(<nums>))";
    assert_eq!(out(src), "6\n");
}

// --- operations -------------------------------------------------------------

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(num("2 + 3 * 4"), 14.0);
    assert_eq!(num("(2 + 3) * 4"), 20.0);
    assert_eq!(num("10 % 3"), 1.0);
}

#[test]
fn division_by_zero_errors() {
    assert!(err("print(1 / 0)").contains("division by zero"));
}

#[test]
fn logical_operators_require_booleans() {
    assert_eq!(out("print(true and false)"), "false\n");
    assert_eq!(out("print(true or false)"), "true\n");
    assert_eq!(out("print(not true)"), "false\n");
    assert!(err("print(5 and true)").contains("boolean"));
}

#[test]
fn no_truthiness_in_conditions() {
    assert!(err("if\n    5 >> print(\"x\")\n.").contains("truthiness"));
}

// --- postfix numeric operators ----------------------------------------------

#[test]
fn postfix_round_ceil_floor() {
    assert_eq!(num("3.14159~2"), 3.14);
    assert_eq!(num("5.7~"), 6.0);
    assert_eq!(num("(11 / 6)^"), 2.0);
    assert_eq!(num("(14 / 6)_"), 2.0);
}

#[test]
fn postfix_square_sqrt_power() {
    assert_eq!(num("5**"), 25.0);
    assert_eq!(num("9//"), 3.0);
    assert_eq!(num("2**10"), 1024.0);
}

#[test]
fn increment_and_decrement() {
    assert_eq!(out("x : 5\nx++\nx++\nprint(x)"), "7\n");
    assert_eq!(out("x : 5\nx--\nprint(x)"), "4\n");
    let src = "g : [[[\"qty\" 1]]]\ng[0][\"qty\"]++\nprint(g[0][\"qty\"])";
    assert_eq!(out(src), "2\n");
}

// --- functions & flow -------------------------------------------------------

#[test]
fn functions_and_first_class_values() {
    let src = ":square(x)\n    x * x\n.\n:apply(f n)\n    f(n)\n.\nprint(apply(square 6))";
    assert_eq!(out(src), "36\n");
}

#[test]
fn closures_capture_environment() {
    // `at-most` returns a predicate closing over `limit` (tiny_inventory §).
    let src = ":at-most(limit)\n    :matches(x)\n        x <= limit\n    .\n    matches\n.\np : at-most(3)\nprint(p(2))\nprint(p(9))";
    assert_eq!(out(src), "true\nfalse\n");
}

#[test]
fn flow_feeds_first_argument() {
    let src = ":subtract(x y)\n    x - y\n.\nprint(10 >> subtract(3))";
    assert_eq!(out(src), "7\n");
}

#[test]
fn flow_chain_across_lines() {
    let src = ":double(x)\n    x * 2\n.\n:inc(x)\n    x + 1\n.\nr :\n    10 >>\n    double() >>\n    inc()\nprint(r)";
    assert_eq!(out(src), "21\n");
}

#[test]
fn dot_flow_is_call_sugar() {
    assert_eq!(out("print([1 2 3 4].size)"), "4\n");
}

#[test]
fn bare_function_value_has_no_effect() {
    assert_eq!(out("print\nprint(\"hi\")"), "hi\n");
}

// --- conditionals -----------------------------------------------------------

#[test]
fn if_first_match_wins() {
    let src = ":d(x)\n    if\n        x > 10 >> \"big\"\n        x > 5 >> \"medium\"\n        else >> \"small\"\n    .\n.\nprint(d(20))\nprint(d(7))\nprint(d(1))";
    assert_eq!(out(src), "big\nmedium\nsmall\n");
}

#[test]
fn if_any_runs_every_match() {
    let src = "if any\n    3 > 0 >> print(\"positive\")\n    3 > 10 >> print(\"huge\")\n    else >> print(\"none\")\n.";
    assert_eq!(out(src), "positive\n");
}

#[test]
fn conditional_shorthand_matches_words() {
    let src = "x : 20\nm :\n    ?\n        x > 10 >> \"big\"\n        ?? >> \"small\"\n    .\nprint(m)";
    assert_eq!(out(src), "big\n");
}

#[test]
fn conditional_as_value() {
    let src = "guess : 3\nanswer : 5\nm :\n    if\n        guess < answer >> \"Too low.\"\n        else >> \"Too high.\"\n    .\nprint(m)";
    assert_eq!(out(src), "Too low.\n");
}

// --- recursion & TCO --------------------------------------------------------

#[test]
fn tail_recursion_is_bounded_stack() {
    let src = ":sum(n acc)\n    if\n        n == 0 >> acc\n        else >> sum((n - 1) (acc + n))\n    .\n.\nprint(sum(1000000 0))";
    assert_eq!(out(src), "500000500000\n");
}

// --- comments ---------------------------------------------------------------

#[test]
fn block_and_line_comments() {
    let src = "## this is\na block comment ##\nx : 5 # trailing line comment\nprint(x)";
    assert_eq!(out(src), "5\n");
}

// --- a program that builds a keyed structure --------------------------------

#[test]
fn word_frequency_counter() {
    // `split` is userland Six, not a builtin — the program defines its own.
    let src = concat!(
        ":split(str sep)\n",
        "    _split(str sep [] \"\" 0)\n",
        ".\n",
        ":_split(str sep result current i)\n",
        "    if\n",
        "        i >= size(str) >>\n",
        "            insert(result current)\n",
        "            result\n",
        "        str[i] == sep >>\n",
        "            insert(result current)\n",
        "            _split(str sep result \"\" (i + 1))\n",
        "        else >>\n",
        "            _split(str sep result (current + str[i]) (i + 1))\n",
        "    .\n",
        ".\n",
        ":count(words counts i)\n",
        "    if\n",
        "        i >= size(words) >> counts\n",
        "        else >>\n",
        "            word : words[i]\n",
        "            if\n",
        "                has?(counts word) >> counts[word] = counts[word] + 1\n",
        "                else >> counts[word] = 1\n",
        "            .\n",
        "            count(words counts (i + 1))\n",
        "    .\n",
        ".\n",
        ":frequencies(source)\n",
        "    words : split(source \" \")\n",
        "    count(words [] 0)\n",
        ".\n",
        "print(frequencies(\"a b a c b a\"))\n",
    );
    assert_eq!(out(src), "[[\"a\" 3] [\"b\" 2] [\"c\" 1]]\n");
}

#[test]
fn split_is_userland_not_a_builtin() {
    // There is no `split` in the runtime; calling it undefined is an error.
    assert!(err("print(split(\"a b\" \" \"))").contains("undefined name 'split'"));
}
