//! tests/fixtures/features/ holds one small fixture per Ramos feature, each
//! lifted from the README's tour of that feature.
//!
//! Where example.rmo proves the lexer sees *every* construct at once, these
//! prove each construct stands on its own — and, because they parse, that the
//! README's examples are real Ramos rather than prose. Every fixture must lex
//! clean, parse clean, and actually exercise the feature it is named for.

use ramos::ast;
use ramos::color::Color;
use ramos::lexer::lex;
use ramos::parser::parse;
use std::path::{Path, PathBuf};

/// Every fixture, with the AST nodes it must produce. This table is both the
/// check that a fixture stays on-topic and the index of what is covered; the
/// nodes are matched against `ast::render`, so they name the parser's output
/// rather than the source text.
///
/// A fixture that stops producing its own feature's node — a `pipe.rmo` with
/// no call in it — is a fixture that has quietly stopped testing anything.
const FEATURES: &[(&str, &[&str])] = &[
    // syntax & literals
    ("comments.rmo", &["Assign", "Lit \"# not a comment\""]),
    ("doc_comments.rmo", &["module Greeter", "function greet(name)"]),
    (
        "variables.rmo",
        &["Assign", "Binding x", "Int 100", "Var x"],
    ),
    (
        "literals.rmo",
        &[
            "Int 42",
            "Float 3.14",
            "Lit \"andrew\"",
            "Symbol :symbol",
            "Bool true",
            "Bool false",
            "Nil",
            "List",
            "Tuple",
            "Map",
        ],
    ),
    (
        "sigils.rmo",
        &[
            "Call .parse()",
            "ModuleRef Date",
            "ModuleRef Time",
            "ModuleRef NaiveDateTime",
            "ModuleRef DateTime",
        ],
    ),
    // operators
    (
        "arithmetic.rmo",
        &[
            "Binary +",
            "Binary -",
            "Binary *",
            "Binary /",
            "Binary %",
            "Binary **",
            "Unary -",
        ],
    ),
    (
        "comparison.rmo",
        &[
            "Binary ==",
            "Binary !=",
            "Binary <",
            "Binary >",
            "Binary <=",
            "Binary >=",
        ],
    ),
    ("logical.rmo", &["Binary and", "Binary or", "Unary not"]),
    ("string_concat.rmo", &["Binary <>"]),
    ("list_concat.rmo", &["Binary ++", "List", "rest"]),
    ("map_merge.rmo", &["Binary ++", "Map"]),
    // strings
    (
        "string_interpolation.rmo",
        &["Str", "Lit \"Ola \"", "Interp", "Var name", "Binary +"],
    ),
    (
        "multiline_strings.rmo",
        &["Str", "Interp", "function usage(name)"],
    ),
    // control flow
    (
        "case.rmo",
        &["Case", "subject", "arm", "pattern", "Wildcard"],
    ),
    (
        "cond.rmo",
        &["Cond", "arm", "condition", "Symbol :positive"],
    ),
    (
        "if.rmo",
        &["If", "condition", "then", "else", "Symbol :high"],
    ),
    ("guards.rmo", &["Case", "when", "Binary >"]),
    (
        "run.rmo",
        &[
            "Run",
            "body",
            "Symbol :ok",
            // the trailing subject-less `case` and its arms
            "case",
            "Binding err",
        ],
    ),
    // pattern matching
    (
        "destructuring.rmo",
        &[
            "Assign",
            "pattern",
            "Binding head",
            "rest",
            "Tuple",
            "Map",
            "Struct Person",
            "Wildcard",
        ],
    ),
    // lambdas & pipes
    ("lambdas.rmo", &["Lambda(x, y)", "Lambda()", "Lambda(x)"]),
    (
        "pipe.rmo",
        &["Call .map()", "Call .join()", "Call .filter()"],
    ),
    // modules
    ("module.rmo", &["module PersonUtils", "function hello(person)"]),
    (
        "namespaced_module.rmo",
        &["module MyApp.Business.SystemUser"],
    ),
    (
        "private_functions.rmo",
        &["module Accounts", "function create(name)", "helper normalize(name)"],
    ),
    (
        "alias.rmo",
        &[
            "Alias Geometry as Geo",
            "Alias Geometry.Circle as Circle",
            "Alias Geometry.Rectangle as Rect",
            // no `as`: the local name defaults to the last segment
            "Alias Drawing.Shapes.Ellipse as Ellipse",
            // and a module carries its own aliases
            "module Report",
            "alias Geometry.Circle as Circle",
            "alias Drawing.Shapes.Ellipse as Oval",
        ],
    ),
    // structs & traits
    (
        "struct.rmo",
        &[
            "struct Person",
            "attributes",
            "function hello(self)",
            "Self",
            "StructLit Person",
            "Access .name",
            "Call .update()",
            "Call .keys()",
            "Call .to_map()",
            "Call .is_a()",
        ],
    ),
    (
        "trait.rmo",
        &["trait Helloable", "function is_over_eighteen(self) [declaration]"],
    ),
    (
        "implements.rmo",
        &[
            "struct Person",
            "implements Helloable",
            "implements Describable",
        ],
    ),
    (
        "actor.rmo",
        &[
            "module Cache",
            "implements Actor",
            "function call(f, args, state, config)",
            "function cast(f, args, state, config)",
        ],
    ),
    // errors as values
    (
        "error_tuples.rmo",
        &["Tuple", "Symbol :error", "Symbol :ok", "Case"],
    ),
    (
        "struct_errors.rmo",
        &[
            "struct DeclineReason",
            "StructLit DeclineReason",
            "Struct DeclineReason",
            "Cond",
            "Case",
        ],
    ),
];

fn features_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/features")
}

fn read(name: &str) -> String {
    let path = features_dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Lex + parse a fixture, rendering its AST. Panics with a full diagnostic —
/// these fixtures double as documentation, so a failure should read like one.
fn render(name: &str) -> String {
    let src = read(name);
    let tokens =
        lex(&src).unwrap_or_else(|e| panic!("{}", ramos::diagnostics::render(name, &src, &e)));
    let program = parse(tokens)
        .unwrap_or_else(|e| panic!("{}", ramos::diagnostics::render_parse(name, &src, &e)));
    ast::render(&program, Color::Never)
}

fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(features_dir())
        .expect("cannot read the features fixture dir")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rmo"))
        .collect();
    names.sort();
    names
}

// ── every fixture lexes, parses, and exercises its feature ───────────────────

#[test]
fn every_fixture_parses_and_exercises_its_feature() {
    for (name, required) in FEATURES {
        let out = render(name);
        for node in *required {
            assert!(
                out.contains(node),
                "{name} no longer produces the AST node it exists to test: {node}\n\
                 rendered tree:\n{out}"
            );
        }
    }
}

/// The table is the index of what's covered, so a fixture that isn't listed is
/// a fixture nothing checks — and a listed fixture that's gone is a stale row.
#[test]
fn the_feature_table_and_the_directory_agree() {
    let on_disk = fixture_names();
    let mut listed: Vec<String> = FEATURES.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(
        listed, on_disk,
        "the FEATURES table and tests/fixtures/features/ have drifted apart"
    );
}

// ── the fixtures stay small and self-describing ──────────────────────────────

#[test]
fn every_fixture_is_small_and_names_itself() {
    for name in fixture_names() {
        let src = read(&name);
        let lines = src.lines().count();
        assert!(
            lines <= 30,
            "{name} is {lines} lines — feature fixtures are meant to be small \
             enough to read at a glance; split it"
        );
        let header = src.lines().next().unwrap_or_default();
        assert!(
            header.starts_with(&format!("# {name} —")),
            "{name} should open with `# {name} — <what it shows>`, found: {header:?}"
        );
    }
}

// ── features that are defined by what they *don't* produce ───────────────────

#[test]
fn doc_comments_leave_no_trace_in_the_ast() {
    // `@module_doc` / `@doc` blocks are plain comments: they document the
    // definition below them, and the parser never sees them.
    let out = render("doc_comments.rmo");
    for marker in ["@module_doc", "@doc", "Greeter — builds greetings"] {
        assert!(
            !out.contains(marker),
            "doc comment leaked into the AST: {marker}"
        );
    }
    assert!(out.contains("function greet(name)"), "{out}");
}

#[test]
fn comments_leave_no_trace_in_the_ast() {
    let out = render("comments.rmo");
    assert!(
        !out.contains("trail a statement"),
        "comment leaked into the AST:\n{out}"
    );
    // ...but a `#` inside a string is content, not a comment.
    assert!(out.contains("Lit \"# not a comment\""), "{out}");
}

#[test]
fn pipes_are_desugared_away() {
    // `|` is rewritten into a plain call at parse time, so no pipe survives
    // into the tree.
    let piped = render("pipe.rmo");
    assert!(piped.contains("Call .map()"), "{piped}");
    assert!(piped.contains("Call .join()"), "{piped}");
}

// ── the fixtures are reachable from the CLI ──────────────────────────────────

/// The fixtures are language *fragments*: they name modules that do not exist
/// (`Geometry.Circle`) and carry a definition named for the feature rather than
/// the file. So they go through the parse-level commands only — `check` loads a
/// whole program, which a fragment by design is not. `loader_test.rs` covers
/// `check` against a real project.
#[test]
fn cli_can_dump_every_fixture() {
    for name in fixture_names() {
        let path = features_dir().join(&name);
        for args in [vec!["ast", "--dump"], vec!["lexer", "--dump"]] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_ramos"))
                .args(&args)
                .arg(&path)
                .output()
                .expect("failed to run the ramos binary");
            assert!(
                out.status.success(),
                "`ramos {} {name}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
