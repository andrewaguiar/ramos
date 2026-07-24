//! Every native primitive the Ramos stdlib delegates to (`native(str, args)`),
//! exercised both as a bare Kernel call and through the `native(...)` seam.
//! The expected column mirrors the stdlib doc examples.

use ramos::interp::{run, run_with_args, sink};
use ramos::lexer::lex;
use ramos::parser::parse;

/// A capturing sink whose bytes can be read back with [`taken`] after the run.
fn capture() -> std::sync::Arc<std::sync::Mutex<Vec<u8>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}
fn taken(buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
    String::from_utf8(std::mem::take(&mut *buf.lock().unwrap())).expect("utf8")
}

/// Evaluate a snippet, returning the last value's `inspect` form.
fn eval(src: &str) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    run(&program, sink(Vec::new()))
        .unwrap_or_else(|e| panic!("`{src}` failed: {e}"))
        .inspect()
}

fn try_eval(src: &str) -> Result<(), String> {
    let tokens = lex(src).map_err(|_| "lex".to_string())?;
    let program = parse(tokens).map_err(|_| "parse".to_string())?;
    run(&program, sink(Vec::new()))
        .map(|_| ())
        .map_err(|e| e.message)
}

/// Evaluate a snippet, returning what it wrote to stdout.
fn output(src: &str) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let buf = capture();
    run(&program, buf.clone()).expect("run");
    taken(&buf)
}

// ── collections: size / at / to_list ─────────────────────────────────────────

#[test]
fn size_counts_lists_tuples_and_maps() {
    assert_eq!(eval("size([1, 2, 3])"), "3");
    assert_eq!(eval("size((:ok, 42))"), "2");
    assert_eq!(eval("size({a: 1, b: 2})"), "2");
    assert!(try_eval("size(\"hi\")").is_err()); // strings use String.length
}

#[test]
fn at_indexes_lists_and_tuples() {
    assert_eq!(eval("at([1, 2, 3], 1)"), "2");
    assert_eq!(eval("at((:ok, 42), 1)"), "42");
    assert_eq!(eval("at([1, 2, 3], 9)"), "nil"); // out of range
    assert_eq!(eval("at([1, 2, 3], -1)"), "nil"); // negatives unsupported
}

#[test]
fn to_list_from_tuples_and_maps() {
    assert_eq!(eval("to_list((1, 2, 3))"), "[1, 2, 3]");
    assert_eq!(eval("to_list({a: 1, b: 2})"), "[1, 2]"); // values, keys dropped
}

#[test]
fn tuple_from_list_builds_at_any_arity() {
    // The only way to build a tuple whose width is not written in the source;
    // `Tuple.set` is derived from it in Ramos rather than being its own native.
    assert_eq!(
        eval("native(\"tuple_from_list\", [[1, 2, 3]])"),
        "(1, 2, 3)"
    );
    assert_eq!(
        eval("native(\"tuple_from_list\", [[1, 2, 3, 4, 5, 6, 7, 8, 9]])"),
        "(1, 2, 3, 4, 5, 6, 7, 8, 9)"
    );
    // The zero-arity tuple has no literal form, so `inspect` is the only view.
    assert_eq!(eval("native(\"tuple_from_list\", [[]])"), "()");
}

#[test]
fn the_map_basics_are_primitives() {
    // Everything that does not take a lambda is a native; `Map` wraps these,
    // and its higher-order functions are Ramos written on top.
    assert_eq!(eval("native(\"map_put\", [{a: 1}, :b, 2])"), "{a: 1, b: 2}");
    // An existing key keeps its position.
    assert_eq!(
        eval("native(\"map_put\", [{a: 1, b: 2}, :a, 9])"),
        "{a: 9, b: 2}"
    );
    assert_eq!(eval("native(\"map_get\", [{a: 1}, :a, 0])"), "1");
    assert_eq!(eval("native(\"map_get\", [{a: 1}, :z, 0])"), "0");
    assert_eq!(eval("native(\"map_delete\", [{a: 1, b: 2}, :a])"), "{b: 2}");
    assert_eq!(eval("native(\"map_delete\", [{a: 1}, :z])"), "{a: 1}");
    assert_eq!(eval("native(\"map_has_key\", [{a: 1}, :a])"), "true");
    assert_eq!(eval("native(\"map_has_key\", [{a: 1}, :z])"), "false");
    assert_eq!(eval("native(\"map_keys\", [{a: 1, b: 2}])"), "[:a, :b]");
    assert_eq!(
        eval("native(\"map_entries\", [{a: 1, b: 2}])"),
        "[(:a, 1), (:b, 2)]"
    );
    assert_eq!(
        eval("native(\"map_from_list\", [[(:a, 1), (:b, 2)]])"),
        "{a: 1, b: 2}"
    );
    assert_eq!(eval("native(\"map_from_list\", [[]])"), "{}");
    // A repeated key keeps its first position and its last value.
    assert_eq!(
        eval("native(\"map_from_list\", [[(:a, 1), (:b, 2), (:a, 9)]])"),
        "{a: 9, b: 2}"
    );
    // from_list only takes pairs.
    assert!(try_eval("native(\"map_from_list\", [[1]])").is_err());
    assert!(try_eval("native(\"map_from_list\", [[(1, 2, 3)]])").is_err());
}

#[test]
fn a_map_key_must_be_an_integer_string_or_symbol() {
    // The rule is enforced where maps are built, so it holds however the key
    // was produced — not just for literals.
    assert_eq!(eval("native(\"map_put\", [{}, 1, :int])"), "{1: :int}");
    assert_eq!(
        eval("native(\"map_put\", [{}, \"s\", :str])"),
        "{\"s\": :str}"
    );
    assert_eq!(eval("native(\"map_put\", [{}, :sym, 1])"), "{sym: 1}");
    for bad in ["true", "nil", "1.5", "[1]", "(1, 2)", "{}"] {
        let err = try_eval(&format!("native(\"map_put\", [{{}}, {bad}, 1])"))
            .expect_err(&format!("`{bad}` should not be a valid key"));
        assert!(
            err.contains("a Map key must be an Integer, String or Symbol"),
            "{bad}: {err}"
        );
        // `from_list` is the other way into a map, and holds the same line.
        let err = try_eval(&format!("native(\"map_from_list\", [[({bad}, 1)]])"))
            .expect_err(&format!("`{bad}` should not be a valid key"));
        assert!(
            err.contains("a Map key must be an Integer, String or Symbol"),
            "{bad}: {err}"
        );
    }
}

// ── conversions ──────────────────────────────────────────────────────────────

#[test]
fn conversions_match_the_docs() {
    assert_eq!(eval("to_integer(\"42\")"), "42");
    assert_eq!(eval("to_integer(3.9)"), "3"); // truncates toward zero
    assert_eq!(eval("to_integer(\"nope\")"), "nil");
    assert_eq!(eval("to_float(3)"), "3.0");
    assert_eq!(eval("to_float(\"3.14\")"), "3.14");
    // to_string, and the symbol-keeps-its-colon rule.
    assert_eq!(output("print(to_string(42))"), "42");
    assert_eq!(output("print(to_string(:ok))"), ":ok");
    assert_eq!(output("print(:done)"), ":done");
}

// ── float transcendentals (back the `Float` module) ─────────────────────────

#[test]
fn float_transcendental_natives_match_known_values() {
    assert_eq!(eval("native(\"float_log\", [1.0])"), "0.0");
    assert_eq!(eval("native(\"float_log_two\", [8.0])"), "3.0");
    assert_eq!(eval("native(\"float_log_ten\", [100.0])"), "2.0");
    assert_eq!(eval("native(\"float_exp\", [0.0])"), "1.0");
    assert_eq!(eval("native(\"float_sin\", [0.0])"), "0.0");
    assert_eq!(eval("native(\"float_cos\", [0.0])"), "1.0");
    assert_eq!(eval("native(\"float_tan\", [0.0])"), "0.0");
}

#[test]
fn float_transcendental_natives_reject_a_non_float() {
    let err = try_eval("native(\"float_log\", [4])").unwrap_err();
    assert!(err.contains("expected a Float, got Integer"), "{err}");
}

#[test]
fn there_is_no_to_symbol() {
    // Symbols are literals only: one lives as long as the program does, so
    // minting them from runtime data would be an unbounded leak.
    assert!(try_eval("to_symbol(\"ok\")").is_err());
}

#[test]
fn inspect_is_the_debug_form_to_string_the_display_form() {
    // They differ only for a string at the top level — nested, both quote.
    assert_eq!(eval("inspect(\"hi\")"), "\"\\\"hi\\\"\"");
    assert_eq!(eval("to_string(\"hi\")"), "\"hi\"");
    assert_eq!(eval("inspect([1, \"two\"])"), "\"[1, \\\"two\\\"]\"");
    assert_eq!(eval("inspect(:ok)"), "\":ok\"");
    assert_eq!(eval("inspect(nil)"), "\"nil\"");
    assert_eq!(eval("inspect(2.0)"), "\"2.0\"");
}

#[test]
fn type_of_names_the_module_a_value_belongs_to() {
    let cases = [
        ("42", "Integer"),
        ("3.14", "Float"),
        ("\"hi\"", "String"),
        (":ok", "Symbol"),
        ("true", "Bool"),
        ("nil", "Nil"),
        ("[1]", "List"),
        ("(1, 2)", "Tuple"),
        ("{a: 1}", "Map"),
        ("do x -> x", "Lambda"),
    ];
    for (src, want) in cases {
        assert_eq!(
            eval(&format!("type_of({src})")),
            format!("\"{want}\""),
            "{src}"
        );
    }
    // A `String`, because `:Integer` is not a symbol anyone could write —
    // symbol names are snake_case, module names CamelCase.
    assert!(try_eval("x = :Integer").is_err());
}

#[test]
fn exit_unwinds_with_its_code_rather_than_killing_the_process() {
    // If this called `process::exit`, it would take the test runner with it.
    let src = "println(\"before\")\nexit(3)\nprintln(\"never\")";
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let buf = capture();
    let e = run(&program, buf.clone())
        .err()
        .expect("exit unwinds as an error");
    assert_eq!(e.exit_code, Some(3));
    assert!(e.is_exit());
    // Output written before the exit is still there.
    assert_eq!(taken(&buf), "before\n");
}

#[test]
fn eprint_writes_to_the_error_stream_not_stdout() {
    let src = "print(\"out\")\neprintln(\"err\")\nprint(\"more\")";
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let (out, errs) = (capture(), capture());
    ramos::interp::run_with_streams(&program, out.clone(), errs.clone(), &[]).expect("run");
    assert_eq!(taken(&out), "outmore");
    assert_eq!(taken(&errs), "err\n");
}

#[test]
fn apply_calls_a_lambda_with_its_arguments_in_a_list() {
    assert_eq!(eval("apply(do x, y -> x + y, [1, 2])"), "3");
    assert_eq!(eval("apply(do -> 42, [])"), "42");
    // A captured binding is still visible, as in any other call.
    assert_eq!(eval("n = 10\napply(do x -> x + n, [5])"), "15");
    // Arity is still checked.
    assert!(try_eval("apply(do x -> x, [1, 2])").is_err());
    // And the shape is.
    assert!(try_eval("apply(42, [])").is_err());
    assert!(try_eval("apply(do x -> x, 1)").is_err());
}

#[test]
fn the_clocks_move_the_way_they_promise() {
    // `now` is a wall clock, so it sits well after the epoch.
    assert_eq!(eval("now() > 1600000000000"), "true");
    // `monotonic` only ever moves forward.
    assert_eq!(eval("a = monotonic()\nb = monotonic()\nb >= a"), "true");
}

#[test]
fn date_epoch_conversions_round_trip_and_match_known_values() {
    // The epoch itself, and the day just before it (crossing 1970 backward
    // exercises the negative-year branch of the conversion).
    assert_eq!(eval(r#"native("date_to_epoch_day", [1970, 1, 1])"#), "0");
    assert_eq!(eval(r#"native("date_to_epoch_day", [1969, 12, 31])"#), "-1");
    assert_eq!(
        eval(r#"native("date_from_epoch_day", [0])"#),
        "(1970, 1, 1)"
    );
    assert_eq!(
        eval(r#"native("date_from_epoch_day", [-1])"#),
        "(1969, 12, 31)"
    );
    // Round-trips for a spread of dates, including a leap day.
    for (y, m, d) in [(2024, 2, 29), (2000, 1, 1), (1, 1, 1), (-1, 6, 15)] {
        let epoch = format!(r#"native("date_to_epoch_day", [{y}, {m}, {d}])"#);
        let back = format!(r#"native("date_from_epoch_day", [{epoch}])"#);
        assert_eq!(eval(&back), format!("({y}, {m}, {d})"));
    }
    // An out-of-range day rolls into the next/previous month rather than
    // producing a nonsense date — `Date.new` leans on this to normalize.
    assert_eq!(
        eval(r#"native("date_from_epoch_day", [native("date_to_epoch_day", [2024, 3, 0])])"#),
        "(2024, 2, 29)"
    );
}

#[test]
fn random_stays_inside_its_range() {
    assert_eq!(eval("r = random()\nr >= 0.0 and r < 1.0"), "true");
    assert_eq!(eval("type_of(random())"), "\"Float\"");
    // Inclusive at both ends, and the bounds may arrive either way round.
    assert_eq!(eval("d = random_int(1, 6)\nd >= 1 and d <= 6"), "true");
    assert_eq!(eval("d = random_int(6, 1)\nd >= 1 and d <= 6"), "true");
    assert_eq!(eval("random_int(7, 7)"), "7");
    // Successive draws differ — a stuck PRNG would fail this.
    assert_eq!(
        eval("a = random_int(1, 1000000)\nb = random_int(1, 1000000)\na != b"),
        "true"
    );
}

// ── type predicates ──────────────────────────────────────────────────────────

#[test]
fn type_predicates() {
    assert_eq!(eval("is_integer(1)"), "true");
    assert_eq!(eval("is_integer(1.0)"), "false");
    assert_eq!(eval("is_float(1.0)"), "true");
    assert_eq!(eval("is_string(\"x\")"), "true");
    assert_eq!(eval("is_symbol(:x)"), "true");
    assert_eq!(eval("is_bool(true)"), "true");
    assert_eq!(eval("is_nil(nil)"), "true");
    assert_eq!(eval("is_list([1])"), "true");
    assert_eq!(eval("is_tuple((1, 2))"), "true");
    assert_eq!(eval("is_map({a: 1})"), "true");
    assert_eq!(eval("is_lambda(do x -> x)"), "true");
    // No Module/Struct values exist yet.
    assert_eq!(eval("is_module(1)"), "false");
    assert_eq!(eval("is_struct(1)"), "false");
}

// ── string primitives (bare calls) ───────────────────────────────────────────

#[test]
fn string_primitives_that_return_strings() {
    let cases = [
        ("string_upcase(\"hello\")", "HELLO"),
        ("string_downcase(\"HELLO\")", "hello"),
        ("string_trim(\"  hi  \")", "hi"),
        ("string_trim_left(\"  hi  \")", "hi  "),
        ("string_trim_right(\"  hi  \")", "  hi"),
        ("string_reverse(\"hello\")", "olleh"),
        ("string_repeat(\"ab\", 3)", "ababab"),
        ("string_repeat(\"ab\", 0)", ""),
        ("string_slice(\"hello world\", 0, 5)", "hello"),
        ("string_slice(\"hello\", 3, 99)", "lo"), // count clipped to the tail
        ("string_replace(\"a-b-c\", \"-\", \"_\")", "a_b_c"),
        ("string_at(\"hello\", 1)", "e"),
        ("string_from_list([\"a\", \"b\", \"c\"])", "abc"),
    ];
    for (src, want) in cases {
        assert_eq!(output(&format!("print({src})")), want, "{src}");
    }
}

#[test]
fn string_primitives_that_return_scalars_and_lists() {
    assert_eq!(eval("string_length(\"hello\")"), "5");
    assert_eq!(eval("string_at(\"hello\", 9)"), "nil");
    assert_eq!(eval("string_contains(\"hello world\", \"world\")"), "true");
    assert_eq!(eval("string_starts_with(\"hello\", \"he\")"), "true");
    assert_eq!(eval("string_ends_with(\"hello\", \"lo\")"), "true");
    assert_eq!(eval("string_find(\"hello world\", \"world\")"), "6");
    assert_eq!(eval("string_find(\"hello\", \"z\")"), "nil");
    assert_eq!(
        eval("string_split(\"a,b,c\", \",\")"),
        "[\"a\", \"b\", \"c\"]"
    );
    assert_eq!(eval("string_to_list(\"abc\")"), "[\"a\", \"b\", \"c\"]");
}

// ── the native(name, args) seam ──────────────────────────────────────────────

#[test]
fn the_native_seam_dispatches_by_name() {
    assert_eq!(eval("native(\"size\", [[1, 2, 3]])"), "3");
    assert_eq!(eval("native(\"string_length\", [\"hello\"])"), "5");
    assert_eq!(output("print(native(\"string_upcase\", [\"hi\"]))"), "HI");
    assert!(try_eval("native(\"nope\", [])").is_err()); // unknown primitive
    assert!(try_eval("native(\"size\")").is_err()); // wrong arity to native itself
}

// ── environment ──────────────────────────────────────────────────────────────

#[test]
fn get_env_reads_process_environment() {
    std::env::set_var("RAMA_NATIVE_TEST", "yes");
    assert_eq!(eval("get_env(\"RAMA_NATIVE_TEST\")"), "\"yes\"");
    assert_eq!(eval("get_env(\"RAMA_DEFINITELY_UNSET_VAR\")"), "nil");
}

// ── command-line arguments: get_args / get_arg ───────────────────────────────

/// Evaluate a snippet with program arguments, returning the last value's
/// `inspect` form.
fn eval_argv(src: &str, argv: &[&str]) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    run_with_args(&program, sink(Vec::new()), &owned)
        .expect("run")
        .inspect()
}

#[test]
fn get_args_returns_the_argument_list() {
    assert_eq!(
        eval_argv("get_args()", &["alice", "42"]),
        "[\"alice\", \"42\"]"
    );
    assert_eq!(eval_argv("get_args()", &[]), "[]");
}

#[test]
fn get_arg_indexes_the_argument_list() {
    let argv = &["alice", "42"];
    assert_eq!(eval_argv("get_arg(0)", argv), "\"alice\"");
    assert_eq!(eval_argv("get_arg(1)", argv), "\"42\"");
    assert_eq!(eval_argv("get_arg(9)", argv), "nil"); // out of range
    assert_eq!(eval_argv("get_arg(-1)", argv), "nil"); // negatives unsupported
}

#[test]
fn sleep_pauses_and_returns_nil() {
    assert_eq!(eval("sleep(0)"), "nil"); // non-positive returns immediately
    let start = std::time::Instant::now();
    assert_eq!(eval("sleep(25)"), "nil");
    assert!(
        start.elapsed() >= std::time::Duration::from_millis(20),
        "sleep(25) returned too soon: {:?}",
        start.elapsed()
    );
}

// ── file I/O ─────────────────────────────────────────────────────────────────

/// A fresh, unique temp path (removed if a previous run left it behind).
fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("ramos-natives-{}-{name}.txt", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn file_write_then_read_round_trips() {
    let path = temp_path("roundtrip");
    let p = path.display();
    assert_eq!(
        eval(&format!(r#"native("file_write", ["{p}", "hello\n"])"#)),
        ":ok"
    );
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{p}"])"#)),
        r#"(:ok, "hello\n")"#
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_read_missing_is_error_enoent() {
    let path = temp_path("missing"); // removed by temp_path, never created
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{}"])"#, path.display())),
        "(:error, :enoent)"
    );
}

#[test]
fn file_append_adds_without_truncating() {
    let path = temp_path("append");
    let p = path.display();
    assert_eq!(
        eval(&format!(r#"native("file_write", ["{p}", "a\n"])"#)),
        ":ok"
    );
    assert_eq!(
        eval(&format!(r#"native("file_append", ["{p}", "b\n"])"#)),
        ":ok"
    );
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{p}"])"#)),
        r#"(:ok, "a\nb\n")"#
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_size_reports_byte_length() {
    let path = temp_path("size");
    let p = path.display();
    eval(&format!(r#"native("file_write", ["{p}", "hello"])"#)); // 5 bytes
    assert_eq!(
        eval(&format!(r#"native("file_size", ["{p}"])"#)),
        "(:ok, 5)"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn file_is_present_and_remove() {
    let path = temp_path("present");
    let p = path.display();
    assert_eq!(eval(&format!(r#"native("path_exists", ["{p}"])"#)), "false");
    eval(&format!(r#"native("file_write", ["{p}", "x"])"#));
    assert_eq!(eval(&format!(r#"native("path_exists", ["{p}"])"#)), "true");
    assert_eq!(eval(&format!(r#"native("file_remove", ["{p}"])"#)), ":ok");
    assert_eq!(eval(&format!(r#"native("path_exists", ["{p}"])"#)), "false");
}

#[test]
fn file_copy_duplicates_and_keeps_source() {
    let src = temp_path("cp_src");
    let dst = temp_path("cp_dst");
    let (s, d) = (src.display(), dst.display());
    eval(&format!(r#"native("file_write", ["{s}", "data"])"#));
    assert_eq!(
        eval(&format!(r#"native("file_copy", ["{s}", "{d}"])"#)),
        ":ok"
    );
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{d}"])"#)),
        r#"(:ok, "data")"#
    );
    assert_eq!(eval(&format!(r#"native("path_exists", ["{s}"])"#)), "true");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn file_move_relocates_and_removes_source() {
    let src = temp_path("mv_src");
    let dst = temp_path("mv_dst");
    let (s, d) = (src.display(), dst.display());
    eval(&format!(r#"native("file_write", ["{s}", "payload"])"#));
    assert_eq!(
        eval(&format!(r#"native("file_move", ["{s}", "{d}"])"#)),
        ":ok"
    );
    assert_eq!(eval(&format!(r#"native("path_exists", ["{s}"])"#)), "false");
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{d}"])"#)),
        r#"(:ok, "payload")"#
    );
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn file_rename_changes_the_path() {
    let src = temp_path("rn_src");
    let dst = temp_path("rn_dst");
    let (s, d) = (src.display(), dst.display());
    eval(&format!(r#"native("file_write", ["{s}", "x"])"#));
    assert_eq!(
        eval(&format!(r#"native("file_rename", ["{s}", "{d}"])"#)),
        ":ok"
    );
    assert_eq!(eval(&format!(r#"native("path_exists", ["{s}"])"#)), "false");
    assert_eq!(eval(&format!(r#"native("path_exists", ["{d}"])"#)), "true");
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn file_touch_creates_then_preserves_content() {
    let path = temp_path("touch");
    let p = path.display();
    assert_eq!(eval(&format!(r#"native("path_exists", ["{p}"])"#)), "false");
    assert_eq!(eval(&format!(r#"native("file_touch", ["{p}"])"#)), ":ok");
    assert_eq!(eval(&format!(r#"native("path_exists", ["{p}"])"#)), "true");
    // Touching an existing file keeps its contents.
    eval(&format!(r#"native("file_write", ["{p}", "keep"])"#));
    assert_eq!(eval(&format!(r#"native("file_touch", ["{p}"])"#)), ":ok");
    assert_eq!(
        eval(&format!(r#"native("file_read", ["{p}"])"#)),
        r#"(:ok, "keep")"#
    );
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn file_chmod_sets_permission_bits() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_path("chmod");
    let p = path.display();
    eval(&format!(r#"native("file_write", ["{p}", "x"])"#));
    // 420 == 0o644 (rw-r--r--).
    assert_eq!(
        eval(&format!(r#"native("file_chmod", ["{p}", 420])"#)),
        ":ok"
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
    let _ = std::fs::remove_file(&path);
}

// ── paths & directories ──────────────────────────────────────────────────────

/// A fresh, unique temp directory path (removed if a previous run left it).
fn temp_dir_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("ramos-natives-dir-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

#[test]
fn path_predicates_distinguish_files_and_dirs() {
    let file = temp_path("pred_file");
    let dir = temp_dir_path("pred_dir");
    let (f, d) = (file.display(), dir.display());
    std::fs::write(&file, "x").unwrap();
    std::fs::create_dir_all(&dir).unwrap();

    assert_eq!(eval(&format!(r#"native("path_exists", ["{f}"])"#)), "true");
    assert_eq!(eval(&format!(r#"native("path_is_file", ["{f}"])"#)), "true");
    assert_eq!(eval(&format!(r#"native("path_is_dir", ["{f}"])"#)), "false");

    assert_eq!(eval(&format!(r#"native("path_exists", ["{d}"])"#)), "true");
    assert_eq!(eval(&format!(r#"native("path_is_dir", ["{d}"])"#)), "true");
    assert_eq!(
        eval(&format!(r#"native("path_is_file", ["{d}"])"#)),
        "false"
    );

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dir_make_creates_nested_directories() {
    let base = temp_dir_path("mkdir");
    let nested = base.join("a").join("b").join("c");
    let n = nested.display();
    assert_eq!(eval(&format!(r#"native("dir_make", ["{n}"])"#)), ":ok");
    assert_eq!(eval(&format!(r#"native("path_is_dir", ["{n}"])"#)), "true");
    // Idempotent: making it again is still :ok.
    assert_eq!(eval(&format!(r#"native("dir_make", ["{n}"])"#)), ":ok");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn dir_list_returns_sorted_entry_names() {
    let dir = temp_dir_path("ls");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("b.txt"), "").unwrap();
    std::fs::write(dir.join("a.txt"), "").unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    assert_eq!(
        eval(&format!(r#"native("dir_list", ["{}"])"#, dir.display())),
        r#"(:ok, ["a.txt", "b.txt", "sub"])"#
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dir_list_missing_is_error() {
    let dir = temp_dir_path("ls_missing");
    assert_eq!(
        eval(&format!(r#"native("dir_list", ["{}"])"#, dir.display())),
        "(:error, :enoent)"
    );
}

#[test]
fn dir_current_matches_the_process_cwd_and_change_validates() {
    // cwd is read-only here: assert it succeeds and matches the process cwd.
    let expected = std::env::current_dir().unwrap();
    assert_eq!(
        eval(r#"native("dir_current", [])"#),
        format!("(:ok, {:?})", expected.to_string_lossy())
    );
    // `cd .` exercises the success path without moving the process-wide cwd
    // (which parallel tests share).
    assert_eq!(eval(r#"native("dir_change", ["."])"#), ":ok");
    // A missing directory is an error and leaves the cwd untouched.
    let missing = temp_dir_path("cd_missing");
    assert_eq!(
        eval(&format!(
            r#"native("dir_change", ["{}"])"#,
            missing.display()
        )),
        "(:error, :enoent)"
    );
}

// ── networking (TCP) ─────────────────────────────────────────────────────────

#[test]
fn server_socket_bind_reports_a_nonzero_local_port_and_closes() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
(:ok, port) = native(\"server_socket_local_port\", [server])
close = native(\"server_socket_close\", [server])
(port > 0, close)";
    assert_eq!(eval(src), "(true, :ok)");
}

#[test]
fn socket_connect_accept_send_and_recv_round_trip() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
(:ok, port) = native(\"server_socket_local_port\", [server])
t = native(\"start_thread\", [do -> native(\"socket_connect\", [\"127.0.0.1\", port])])
(:ok, conn) = native(\"server_socket_accept\", [server])
(:ok, client) = native(\"await_thread\", [t])
native(\"socket_send\", [conn, \"hello\"])
native(\"socket_recv\", [client, 5])";
    assert_eq!(eval(src), r#"(:ok, "hello")"#);
}

#[test]
fn socket_recv_returns_empty_string_once_the_peer_closes() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
(:ok, port) = native(\"server_socket_local_port\", [server])
t = native(\"start_thread\", [do -> native(\"socket_connect\", [\"127.0.0.1\", port])])
(:ok, conn) = native(\"server_socket_accept\", [server])
(:ok, client) = native(\"await_thread\", [t])
native(\"socket_close\", [client])
native(\"socket_recv\", [conn, 16])";
    assert_eq!(eval(src), r#"(:ok, "")"#);
}

#[test]
fn socket_peer_address_names_the_bound_server() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
(:ok, port) = native(\"server_socket_local_port\", [server])
(:ok, socket) = native(\"socket_connect\", [\"127.0.0.1\", port])
(:ok, addr) = native(\"socket_peer_address\", [socket])
addr == \"127.0.0.1:#{port}\"";
    assert_eq!(eval(src), "true");
}

#[test]
fn socket_send_and_recv_after_close_are_error_closed() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
(:ok, port) = native(\"server_socket_local_port\", [server])
(:ok, socket) = native(\"socket_connect\", [\"127.0.0.1\", port])
native(\"socket_close\", [socket])
native(\"socket_close\", [socket])
native(\"socket_send\", [socket, \"x\"])";
    assert_eq!(eval(src), "(:error, :closed)");
}

#[test]
fn server_socket_accept_after_close_is_error_closed() {
    let src = "\
(:ok, server) = native(\"server_socket_bind\", [\"127.0.0.1\", 0])
native(\"server_socket_close\", [server])
native(\"server_socket_close\", [server])
native(\"server_socket_accept\", [server])";
    assert_eq!(eval(src), "(:error, :closed)");
}
