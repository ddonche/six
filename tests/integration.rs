//! Integration tests exercising Six's semantics against the canonical syntax
//! (see `examples/tiny_inventory.six`).
//!
//! Each test runs a Six program and checks the value of its final statement
//! (`shown`), or the text it emits through the runtime output channel
//! (`printed`), covering the whole pipeline (lexer → parser → interpreter).
//! Six has no `print`: a program observes a value by leaving it as the final
//! statement, or by writing it with `out(output …)`.

use six::{run, run_capture, Value};

/// The rendered value of the program's final statement — the ordinary way to
/// observe a result now that `print` is gone.
fn shown(src: &str) -> String {
    six::format::display(&run(src).expect("program should run"))
}

/// Everything the program emitted through the runtime output channel.
fn printed(src: &str) -> String {
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
    assert_eq!(shown("x : 5\nx = 7\nx"), "7");
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
    assert!(err("y").contains("undefined name 'y'"));
}

#[test]
fn immutable_uppercase_names() {
    assert!(err("PI : 3.14159\nPI = 3").contains("immutable"));
    assert!(err("WHITE : [255 255 255]\ninsert(WHITE 0)").contains("immutable"));
}

// --- empty vs nonexistent ---------------------------------------------------

#[test]
fn empty_value_exists() {
    assert_eq!(shown("x : ..\nx"), "nil");
    assert_eq!(shown("x : nil\nx"), "nil");
}

#[test]
fn typed_empty_forms() {
    assert_eq!(num("size(number ..)"), 0.0);
    assert_eq!(num("size(text ..)"), 0.0);
    assert_eq!(shown("x : number ..\nx == .."), "true");
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
    assert_eq!(shown("name : \"Dan\"\nname[0]"), "D");
    assert_eq!(shown("name : \"Dan\"\nname[$]"), "n");
}

#[test]
fn text_keyed_lookup_errors() {
    assert!(err("name : \"Dan\"\nname[\"first\"]").contains("keyed lookup"));
}

#[test]
fn text_position_assignment() {
    assert_eq!(shown("s : \"cat\"\ns[0] = \"b\"\ns"), "bat");
}

#[test]
fn concatenation_requires_matching_types() {
    assert_eq!(shown("\"Hello, \" + \"Dan\""), "Hello, Dan");
    assert!(err("\"Age: \" + 46").contains("mix text"));
    assert_eq!(shown("\"Age: \" + text(46)"), "Age: 46");
}

// --- reference vs value semantics -------------------------------------------

#[test]
fn groups_are_reference_semantic() {
    assert_eq!(shown("a : [1 2 3]\nb : a\ninsert(b 4)\na"), "[1 2 3 4]");
}

#[test]
fn deep_copy_is_independent() {
    let prefix = "a : [1 2 3]\nb :: a\ninsert(b 4)\n";
    assert_eq!(shown(&format!("{prefix}a")), "[1 2 3]");
    assert_eq!(shown(&format!("{prefix}b")), "[1 2 3 4]");
}

#[test]
fn simple_values_are_value_semantic() {
    let prefix = ":bump(n)\n    n = n + 1\n    n\n.\nx : 5\n";
    assert_eq!(shown(&format!("{prefix}bump(x)")), "6");
    assert_eq!(shown(&format!("{prefix}x")), "5");
}

// --- groups & indexing ------------------------------------------------------

#[test]
fn positional_index_out_of_range_errors() {
    assert!(err("g : [1 2 3]\ng[5]").contains("out of range"));
}

#[test]
fn final_position_on_empty_errors() {
    assert!(err("[][$]").contains("final position"));
}

#[test]
fn negative_index_errors() {
    assert!(err("g : [1 2 3]\ng[0 - 1]").contains("negative"));
}

// --- keyed groups -----------------------------------------------------------

#[test]
fn keyed_lookup_and_first_match_wins() {
    assert_eq!(shown("p : [[\"k\" 1] [\"k\" 2]]\np[\"k\"]"), "1");
}

#[test]
fn missing_keyed_read_errors_but_write_creates() {
    assert!(err("p : [[\"a\" 1]]\np[\"b\"]").contains("no key"));
    assert_eq!(shown("p : [[\"a\" 1]]\np[\"b\"] = 2\np[\"b\"]"), "2");
}

#[test]
fn has_distinguishes_existence_from_emptiness() {
    let prefix = "p : [[\"mid\" (text ..)]]\n";
    assert_eq!(shown(&format!("{prefix}has(p \"mid\")")), "true");
    assert_eq!(shown(&format!("{prefix}has(p \"nope\")")), "false");
}

#[test]
fn nested_keyed_assignment_by_reference() {
    assert_eq!(shown("g : [[[\"qty\" 1]]]\ng[0][\"qty\"] = 5\ng[0][\"qty\"]"), "5");
}

// --- open / splat -----------------------------------------------------------

#[test]
fn splat_opens_a_group_into_arguments() {
    assert_eq!(shown(":add3(a b c)\n    a + b + c\n.\nnums : [1 2 3]\nadd3(<nums>)"), "6");
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
    assert!(err("1 / 0").contains("division by zero"));
}

#[test]
fn logical_operators_require_booleans() {
    assert_eq!(shown("true and false"), "false");
    assert_eq!(shown("true or false"), "true");
    assert_eq!(shown("not true"), "false");
    assert!(err("5 and true").contains("boolean"));
}

#[test]
fn no_truthiness_in_conditions() {
    assert!(err("if\n    5 >> out(output \"x\")\n.").contains("truthiness"));
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
    assert_eq!(shown("x : 5\nx++\nx++\nx"), "7");
    assert_eq!(shown("x : 5\nx--\nx"), "4");
    assert_eq!(shown("g : [[[\"qty\" 1]]]\ng[0][\"qty\"]++\ng[0][\"qty\"]"), "2");
}

// --- functions & flow -------------------------------------------------------

#[test]
fn functions_and_first_class_values() {
    assert_eq!(shown(":square(x)\n    x * x\n.\n:apply(f n)\n    f(n)\n.\napply(square 6)"), "36");
}

#[test]
fn closures_capture_environment() {
    // `at-most` returns a predicate closing over `limit` (tiny_inventory §).
    let prefix = ":at-most(limit)\n    :matches(x)\n        x <= limit\n    .\n    matches\n.\np : at-most(3)\n";
    assert_eq!(shown(&format!("{prefix}p(2)")), "true");
    assert_eq!(shown(&format!("{prefix}p(9)")), "false");
}

#[test]
fn flow_feeds_first_argument() {
    assert_eq!(shown(":subtract(x y)\n    x - y\n.\n10 >> subtract(3)"), "7");
}

#[test]
fn flow_chain_across_lines() {
    let src = ":double(x)\n    x * 2\n.\n:inc(x)\n    x + 1\n.\nr :\n    10 >>\n    double() >>\n    inc()\nr";
    assert_eq!(shown(src), "21");
}

#[test]
fn dot_flow_is_call_sugar() {
    assert_eq!(shown("[1 2 3 4].size"), "4");
}

#[test]
fn bare_function_value_has_no_effect() {
    // A bare builtin name evaluates to the function value and is discarded; the
    // following statement is what emits.
    assert_eq!(printed("out\nout(output \"hi\")"), "hi");
}

// --- conditionals -----------------------------------------------------------

#[test]
fn if_first_match_wins() {
    let prefix = ":d(x)\n    if\n        x > 10 >> \"big\"\n        x > 5 >> \"medium\"\n        else >> \"small\"\n    .\n.\n";
    assert_eq!(shown(&format!("{prefix}d(20)")), "big");
    assert_eq!(shown(&format!("{prefix}d(7)")), "medium");
    assert_eq!(shown(&format!("{prefix}d(1)")), "small");
}

#[test]
fn if_any_runs_every_match() {
    let src = "if any\n    3 > 0 >> out(output \"positive\")\n    3 > 10 >> out(output \"huge\")\n    else >> out(output \"none\")\n.";
    assert_eq!(printed(src), "positive");
}

#[test]
fn conditional_shorthand_matches_words() {
    let src = "x : 20\nm :\n    ?\n        x > 10 >> \"big\"\n        ?? >> \"small\"\n    .\nm";
    assert_eq!(shown(src), "big");
}

#[test]
fn conditional_as_value() {
    let src = "guess : 3\nanswer : 5\nm :\n    if\n        guess < answer >> \"Too low.\"\n        else >> \"Too high.\"\n    .\nm";
    assert_eq!(shown(src), "Too low.");
}

// --- recursion & TCO --------------------------------------------------------

#[test]
fn tail_recursion_is_bounded_stack() {
    let src = ":sum(n acc)\n    if\n        n == 0 >> acc\n        else >> sum((n - 1) (acc + n))\n    .\n.\nsum(1000000 0)";
    assert_eq!(shown(src), "500000500000");
}

// --- comments ---------------------------------------------------------------

#[test]
fn block_and_line_comments() {
    let src = "## this is\na block comment ##\nx : 5 # trailing line comment\nx";
    assert_eq!(shown(src), "5");
}

#[test]
fn unterminated_block_comment_errors() {
    assert!(err("## never closed\n1").contains("unterminated block comment"));
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
        "                has(counts word) >> counts[word] = counts[word] + 1\n",
        "                else >> counts[word] = 1\n",
        "            .\n",
        "            count(words counts (i + 1))\n",
        "    .\n",
        ".\n",
        ":frequencies(source)\n",
        "    words : split(source \" \")\n",
        "    count(words [] 0)\n",
        ".\n",
        "frequencies(\"a b a c b a\")\n",
    );
    assert_eq!(shown(src), "[[\"a\" 3] [\"b\" 2] [\"c\" 1]]");
}

#[test]
fn split_is_userland_not_a_builtin() {
    // There is no `split` in the runtime; calling it undefined is an error.
    assert!(err("split(\"a b\" \" \")").contains("undefined name 'split'"));
}

// --- REPL clear support (an interpreter capability, not language syntax) -----

#[test]
fn clear_binding_allows_a_fresh_definition() {
    use six::Interpreter;
    let mut interp = Interpreter::new();
    interp.run(&six::parse("x : 5").unwrap()).unwrap();
    // Redeclaring is illegal while the name exists.
    assert!(interp.run(&six::parse("x : 7").unwrap()).is_err());
    // clear removes it (true), and a second clear finds nothing (false).
    assert!(interp.clear_binding("x"));
    assert!(!interp.clear_binding("x"));
    // Now the name is free to define again.
    interp.run(&six::parse("x : 7").unwrap()).unwrap();
    assert!(matches!(interp.run(&six::parse("x").unwrap()).unwrap(), Value::Number(n) if n == 7.0));
}

// --- host observations: entropy and time -----------------------------------

#[test]
fn entropy_is_integer_in_range() {
    for _ in 0..500 {
        match run("entropy()").unwrap() {
            Value::Number(n) => {
                assert!(n.fract() == 0.0, "entropy not integer-valued: {}", n);
                assert!((0.0..=9_007_199_254_740_991.0).contains(&n), "out of range: {}", n);
            }
            other => panic!("entropy should be a number, got {:?}", other),
        }
    }
}

#[test]
fn entropy_varies() {
    let first = num("entropy()");
    let mut differs = false;
    for _ in 0..30 {
        if num("entropy()") != first {
            differs = true;
            break;
        }
    }
    assert!(differs, "entropy returned a constant value");
}

#[test]
fn time_utc_is_integer_microseconds() {
    let n = num("time(\"utc\")");
    assert!(n.fract() == 0.0, "utc not integer-valued: {}", n);
    // Comfortably after 2020-01-01 in microseconds.
    assert!(n > 1_577_836_800_000_000.0, "utc suspiciously small: {}", n);
}

#[test]
fn time_steady_never_decreases_and_is_integer() {
    let n = num("a : time(\"steady\")\nb : time(\"steady\")\nb - a");
    assert!(n >= 0.0, "steady time decreased: {}", n);
    assert!(num("time(\"steady\")").fract() == 0.0, "steady not integer-valued");
}

#[test]
fn time_invalid_mode_errors() {
    assert!(err("time(\"bogus\")").contains("utc"));
    assert!(err("time(5)").contains("mode"));
    assert!(err("time()").contains("expects"));
}

#[test]
fn clear_all_resets_but_keeps_builtins() {
    use six::Interpreter;
    let mut interp = Interpreter::new();
    interp.run(&six::parse(":sq(n)\n    n * n\n.\ny : 3").unwrap()).unwrap();
    interp.clear_all();
    // User bindings are gone...
    assert!(interp.run(&six::parse("y").unwrap()).is_err());
    // ...but builtins remain and the names are free to define again.
    interp.run(&six::parse(":sq(n)\n    n + n\n.\nout(output text(sq(4)))").unwrap()).unwrap();
}
