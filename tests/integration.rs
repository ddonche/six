//! Integration tests exercising Six's semantics against the specification.
//!
//! Each test runs a Six program and checks either its printed output or the
//! value of its final statement, so the whole pipeline (lexer → parser →
//! interpreter) is covered end to end.

use six::{run, run_capture, Value};

/// Assert the captured stdout of a program.
fn out(src: &str) -> String {
    run_capture(src).expect("program should run").1
}

/// Assert a program errors (and return the message).
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
    assert_eq!(out("x : 5\nx = 7\nprint x"), "7\n");
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
    assert!(err("print y").contains("undefined name 'y'"));
}

#[test]
fn immutable_uppercase_names() {
    assert!(err("PI : 3.14159\nPI = 3").contains("immutable"));
    assert!(err("WHITE : [255 255 255]\ninsert WHITE 0").contains("immutable"));
}

// --- empty vs nonexistent ---------------------------------------------------

#[test]
fn empty_value_exists() {
    assert_eq!(out("x : ..\nprint x"), "nil\n");
    assert_eq!(out("x : nil\nprint x"), "nil\n");
}

#[test]
fn size_of_empties_is_zero() {
    assert_eq!(num("size .."), 0.0);
    assert_eq!(num("size nil"), 0.0);
    assert_eq!(num("size []"), 0.0);
    assert_eq!(num("size (text ..)"), 0.0);
}

// --- text -------------------------------------------------------------------

#[test]
fn text_is_positionally_indexable() {
    assert_eq!(out("name : \"Dan\"\nprint name[0]"), "D\n");
    assert_eq!(out("name : \"Dan\"\nprint name[$]"), "n\n");
}

#[test]
fn text_keyed_lookup_errors() {
    assert!(err("name : \"Dan\"\nprint name[\"first\"]").contains("keyed lookup"));
}

#[test]
fn text_position_assignment() {
    assert_eq!(out("s : \"cat\"\ns[0] = \"b\"\nprint s"), "bat\n");
}

#[test]
fn concatenation_requires_matching_types() {
    assert_eq!(out("print (\"Hello, \" + \"Dan\")"), "Hello, Dan\n");
    assert!(err("print (\"Age: \" + 46)").contains("mix text"));
    assert_eq!(out("print (\"Age: \" + text 46)"), "Age: 46\n");
}

// --- reference vs value semantics -------------------------------------------

#[test]
fn groups_are_reference_semantic() {
    let src = "a : [1 2 3]\nb : a\ninsert b 4\nprint a";
    assert_eq!(out(src), "[1 2 3 4]\n");
}

#[test]
fn deep_copy_is_independent() {
    let src = "a : [1 2 3]\nb :: a\ninsert b 4\nprint a\nprint b";
    assert_eq!(out(src), "[1 2 3]\n[1 2 3 4]\n");
}

#[test]
fn simple_values_are_value_semantic() {
    let src = ">> bump: [n] >>\n    n = n + 1\n    n\n.\nx : 5\nprint (bump x)\nprint x";
    assert_eq!(out(src), "6\n5\n");
}

// --- groups & indexing ------------------------------------------------------

#[test]
fn positional_index_out_of_range_errors() {
    assert!(err("g : [1 2 3]\nprint g[5]").contains("out of range"));
}

#[test]
fn final_position_on_empty_errors() {
    assert!(err("print [][$]").contains("final position"));
}

#[test]
fn negative_index_errors() {
    assert!(err("g : [1 2 3]\nprint g[0 - 1]").contains("negative"));
}

// --- keyed groups -----------------------------------------------------------

#[test]
fn keyed_lookup_and_first_match_wins() {
    let src = "p : [[\"k\" 1] [\"k\" 2]]\nprint p[\"k\"]";
    assert_eq!(out(src), "1\n");
}

#[test]
fn missing_keyed_read_errors_but_write_creates() {
    assert!(err("p : [[\"a\" 1]]\nprint p[\"b\"]").contains("no key"));
    let src = "p : [[\"a\" 1]]\np[\"b\"] = 2\nprint p[\"b\"]";
    assert_eq!(out(src), "2\n");
}

#[test]
fn has_distinguishes_existence_from_emptiness() {
    // §10 requires the compound call `text ..` to be parenthesised inline so it
    // reads as one member rather than two.
    let src = "p : [[\"mid\" (text ..)]]\nprint (has? p \"mid\")\nprint (has? p \"nope\")";
    assert_eq!(out(src), "true\nfalse\n");
}

// --- open / splat -----------------------------------------------------------

#[test]
fn splat_opens_a_group_into_arguments() {
    let src = ">> add3: [a b c] >> a + b + c\nnums : [1 2 3]\nprint (add3 <nums>)";
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
    assert!(err("print (1 / 0)").contains("division by zero"));
}

#[test]
fn logical_operators_require_booleans() {
    assert_eq!(out("print (true and false)"), "false\n");
    assert_eq!(out("print (true or false)"), "true\n");
    assert_eq!(out("print (not true)"), "false\n");
    assert!(err("print (5 and true)").contains("boolean"));
}

#[test]
fn no_truthiness_in_conditions() {
    assert!(err("if\n    5 >> print \"x\"\n.").contains("truthiness"));
}

// --- functions & flow -------------------------------------------------------

#[test]
fn functions_and_first_class_values() {
    let src = ">> square: [x] >> x * x\n>> apply: [f n] >> f n\nprint (apply square 6)";
    assert_eq!(out(src), "36\n");
}

#[test]
fn flow_feeds_first_argument() {
    let src = ">> subtract: [x y] >> x - y\nprint (10 >> subtract 3)";
    assert_eq!(out(src), "7\n");
}

#[test]
fn flow_chain() {
    let src = ">> add: [x y] >> x + y\n>> mul: [x y] >> x * y\nprint (10 >> add 5 >> mul 2)";
    assert_eq!(out(src), "30\n");
}

#[test]
fn dot_flow_is_call_sugar() {
    assert_eq!(out("print [1 2 3 4].size"), "4\n");
    assert_eq!(out("print 3.14.text"), "3.14\n");
}

#[test]
fn bare_function_value_has_no_effect() {
    // `print` on its own line evaluates to the function value and prints nothing.
    assert_eq!(out("print\nprint \"hi\""), "hi\n");
}

// --- conditionals -----------------------------------------------------------

#[test]
fn if_first_match_wins() {
    let src = ">> d: [x] >>\n    if\n        x > 10 >> \"big\"\n        x > 5 >> \"medium\"\n        else >> \"small\"\n    .\n.\nprint (d 20)\nprint (d 7)\nprint (d 1)";
    assert_eq!(out(src), "big\nmedium\nsmall\n");
}

#[test]
fn if_any_runs_every_match() {
    let src = "if any\n    3 > 0 >> print \"positive\"\n    3 > 10 >> print \"huge\"\n    else >> print \"none\"\n.";
    assert_eq!(out(src), "positive\n");
}

#[test]
fn conditional_shorthand_matches_words() {
    let src = "x : 20\nm :\n    ?\n        x > 10 >> \"big\"\n        ?? >> \"small\"\n    .\nprint m";
    assert_eq!(out(src), "big\n");
}

#[test]
fn conditional_as_value() {
    let src = "guess : 3\nanswer : 5\nm :\n    if\n        guess < answer >> \"Too low.\"\n        else >> \"Too high.\"\n    .\nprint m";
    assert_eq!(out(src), "Too low.\n");
}

// --- recursion & TCO --------------------------------------------------------

#[test]
fn tail_recursion_is_bounded_stack() {
    // Would overflow a naive recursive interpreter.
    let src =
        ">> sum: [n acc] >>\n    if\n        n == 0 >> acc\n        else >> sum (n - 1) (acc + n)\n    .\n.\nprint (sum 1000000 0)";
    assert_eq!(out(src), "500000500000\n");
}

// --- prelude (Six-implemented stdlib) ---------------------------------------

#[test]
fn prelude_map_filter_fold_find() {
    let src = concat!(
        ">> dbl: [x] >> x * 2\n",
        ">> even?: [x] >> x % 2 == 0\n",
        ">> add: [a b] >> a + b\n",
        "print (map [1 2 3] dbl)\n",
        "print (filter [1 2 3 4] even?)\n",
        "print (fold [1 2 3 4] 0 add)\n",
        "print (find [1 3 4] even?)\n",
    );
    assert_eq!(out(src), "[2 4 6]\n[2 4]\n10\n4\n");
}

#[test]
fn find_missing_yields_empty() {
    let src = ">> big?: [x] >> x > 100\nprint (find [1 2 3] big?)";
    assert_eq!(out(src), "nil\n");
}

// --- word frequency (spec §43) ----------------------------------------------

#[test]
fn word_frequency_counter() {
    let src = concat!(
        ">> count: [words counts i] >>\n",
        "    if\n",
        "        i >= size words >> counts\n",
        "        else >>\n",
        "            word : words[i]\n",
        "            if\n",
        "                counts.has? word >> counts[word] = counts[word] + 1\n",
        "                else >> counts[word] = 1\n",
        "            .\n",
        "            count words counts (i + 1)\n",
        "    .\n",
        ".\n",
        ">> frequencies: [source] >>\n",
        "    words : source.split \" \"\n",
        "    count words [] 0\n",
        ".\n",
        "print (frequencies \"a b a c b a\")\n",
    );
    assert_eq!(out(src), "[[\"a\" 3] [\"b\" 2] [\"c\" 1]]\n");
}
