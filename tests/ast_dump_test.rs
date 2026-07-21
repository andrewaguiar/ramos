//! Tests for the AST debug view (`ramos ast [--dump]`): the tree rendering
//! itself, and — mirroring example_fixture_test's token-kind sweep — the claim
//! that tests/fixtures/example.rmo exercises every AST node the parser builds.

use ramos::ast;
use ramos::color::Color;
use ramos::lexer::lex;
use ramos::parser::parse;
use std::path::Path;
use std::process::Command;

fn render(src: &str) -> String {
    let tokens = lex(src).unwrap_or_else(|e| {
        panic!(
            "lex failed: {}",
            ramos::diagnostics::render("<test>", src, &e)
        )
    });
    let program = parse(tokens).unwrap_or_else(|e| {
        panic!(
            "parse failed: {}",
            ramos::diagnostics::render_parse("<test>", src, &e)
        )
    });
    ast::render(&program, Color::Never)
}

fn fixture_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example.rmo")
}

fn fixture_render() -> String {
    let src = std::fs::read_to_string(fixture_path()).unwrap();
    render(&src)
}

// ── the tree rendering ───────────────────────────────────────────────────────

#[test]
fn renders_an_indented_tree() {
    assert_eq!(
        render("x = 1 + 2\n"),
        "\
Program
  Assign
    pattern
      Binding x
    value
      Binary +
        Int 1
        Int 2
"
    );
}

#[test]
fn renders_definitions_with_their_heads() {
    assert_eq!(
        render(
            "\
module Payments
  fn max()
    10

  fnp helper(a, b)
    a

  fn native(s)
"
        ),
        "\
Program
  module Payments
    fn max()
      Int 10
    fnp helper(a, b)
      Var a
    fn native(s) [declaration]
"
    );
}

#[test]
fn nesting_depth_tracks_the_tree() {
    // Each level is exactly two spaces, and closing a node pops back out.
    let out = render("f(g(1), 2)\n");
    assert_eq!(
        out,
        "\
Program
  Call f()
    args
      Call g()
        args
          Int 1
      Int 2
"
    );
}

#[test]
fn shows_pipes_already_desugared() {
    // `|` is gone by parse time — the dump must show the rewritten call...
    let out = render("a\n| List.map(f)\n");
    assert!(
        !out.contains("Pipe "),
        "`|` should not survive parsing:\n{out}"
    );
    assert!(out.contains("Call .map()"), "{out}");
    assert!(out.contains("Var a"), "{out}");
}

#[test]
fn omits_the_args_node_when_a_call_has_none() {
    assert_eq!(
        render("f()\n"),
        "\
Program
  Call f()
"
    );
}

#[test]
fn distinguishes_struct_literals_from_maps() {
    assert!(render("Person{age: 1}\n").contains("StructLit Person"));
    assert!(render("{age: 1}\n").contains("Map"));
}

#[test]
fn renders_case_arms_with_guards() {
    let out = render(
        "\
case x
  [h | t] when h > 0 -> h
  _ -> nil
",
    );
    assert_eq!(
        out,
        "\
Program
  Case
    subject
      Var x
    arm
      pattern
        List
          Binding h
          rest
            Binding t
      when
        Binary >
          Var h
          Int 0
      body
        Var h
    arm
      pattern
        Wildcard
      body
        Nil
"
    );
}

// ── the dump wrapper ─────────────────────────────────────────────────────────

#[test]
fn dump_wraps_the_source_and_the_tree() {
    let src = "x = 1\n";
    let tokens = lex(src).unwrap();
    let program = parse(tokens).unwrap();
    let out = ast::dump(src, &program, Color::Never);
    assert_eq!(
        out,
        "\
Raw Code:
```
x = 1
```

AST
```
Program
  Assign
    pattern
      Binding x
    value
      Int 1
```
"
    );
}

#[test]
fn dump_terminates_a_source_that_lacks_a_final_newline() {
    let src = "x = 1";
    let tokens = lex(src).unwrap();
    let program = parse(tokens).unwrap();
    assert!(ast::dump(src, &program, Color::Never).contains("x = 1\n```"));
}

// ── acceptance: the fixture exercises every node ─────────────────────────────

#[test]
fn example_fixture_uses_every_ast_node() {
    let out = fixture_render();
    let has = |node: &str| {
        assert!(
            out.contains(node),
            "example.rmo never produces AST node: {node}"
        );
    };

    // items & definitions
    has("Program");
    has("module Examples");
    has("struct Person");
    has("trait Describable");
    has("implements Describable");
    has("attributes");
    has("fn pi()");
    has("fnp sum_all(list)");
    has("[declaration]");
    // statements
    has("Assign");
    has("Alias Examples as Ex");
    // literals
    has("Int ");
    has("Float ");
    has("Bool ");
    has("Nil");
    has("Symbol :");
    has("Str");
    has("Lit ");
    has("Interp");
    has("List");
    has("Tuple");
    has("Map");
    has("StructLit Person");
    // references
    has("Var ");
    has("Wildcard");
    has("Self");
    has("ModuleRef List");
    has("Access .name");
    // calls
    has("Call print()");
    has("Call .describe()");
    has("Call .put()");
    has("args");
    // operators
    has("Unary -");
    has("Unary not");
    has("Binary +");
    has("Binary **");
    has("Binary <>");
    has("Binary ++");
    has("Binary ==");
    has("Binary and");
    has("Binary or");
    // control flow
    has("Lambda(x, y)");
    has("Case");
    has("subject");
    has("when");
    has("Cond");
    has("condition");
    // patterns
    has("Binding head");
    has("rest");
    has("Struct Person");
}

#[test]
fn every_stdlib_module_renders() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("src");
    for name in ["kernel.rmo", "list.rmo", "string.rmo", "tuple.rmo"] {
        let src = std::fs::read_to_string(dir.join(name)).unwrap();
        let out = render(&src);
        assert!(out.starts_with("Program\n"), "{name}: {out}");
        assert!(out.lines().count() > 10, "{name}: suspiciously small tree");
    }
}

// ── the CLI ──────────────────────────────────────────────────────────────────

#[test]
fn cli_ast_prints_the_tree_and_dump_adds_the_source() {
    let fixture = fixture_path();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_ramos"))
            .args(args)
            .arg(&fixture)
            // Pin the colour: a developer with FORCE_COLOR set must not turn
            // this into a comparison against escape codes.
            .env("NO_COLOR", "1")
            .output()
            .expect("failed to run the ramos binary");
        assert!(
            out.status.success(),
            "`ramos {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };

    let plain = run(&["ast"]);
    assert!(plain.starts_with("Program\n"), "{plain}");
    assert!(!plain.contains("Raw Code:"));
    assert_eq!(plain, fixture_render(), "`ramos ast` == ast::render");

    let dumped = run(&["ast", "--dump"]);
    assert!(dumped.starts_with("Raw Code:\n```\n"), "{dumped}");
    assert!(dumped.contains("\nAST\n```\nProgram\n"), "{dumped}");
    assert!(dumped.contains("trait Describable"), "{dumped}");
}

#[test]
fn cli_ast_reports_a_parse_error_and_fails() {
    let bad = std::env::temp_dir().join("ramos_ast_cli_bad.rmo");
    std::fs::write(&bad, "case x\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ramos"))
        .arg("ast")
        .arg(&bad)
        .output()
        .expect("failed to run the ramos binary");
    let _ = std::fs::remove_file(&bad);
    assert!(!out.status.success(), "a parse error must exit non-zero");
    assert!(out.stdout.is_empty(), "no tree on a parse error");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("case arm"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
