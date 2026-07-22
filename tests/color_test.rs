//! Colour in the debug dumps (`ramos lexer --dump`, `ramos ast --dump`).
//!
//! The load-bearing property is that colour is *decorative*: stripping the
//! escapes from a painted dump must give back the plain dump, byte for byte.
//! For the raw-code block that is a real check of the highlighter, which
//! rebuilds the source from token spans — a span that is wrong, or a stretch
//! of source no token covers, shows up here as changed text.

use ramos::color::{Color, Style};
use ramos::lexer::{self, lex};
use ramos::parser::parse;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Remove every `ESC [ ... m` sequence.
fn strip(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths = vec![root.join("example.rmo")];
    let mut features: Vec<PathBuf> = std::fs::read_dir(root.join("features"))
        .expect("cannot read the features fixture dir")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rmo"))
        .collect();
    features.sort();
    paths.append(&mut features);
    paths
}

// ── colour changes nothing but the escapes ───────────────────────────────────

#[test]
fn painting_a_dump_only_adds_escapes() {
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&path).unwrap();
        let tokens = lex(&src).unwrap();

        let painted = lexer::dump(&src, &tokens, Color::Always);
        let plain = lexer::dump(&src, &tokens, Color::Never);
        assert_eq!(
            strip(&painted),
            plain,
            "lexer dump of {name} changed under colour"
        );

        let program = parse(tokens).unwrap();
        let painted = ramos::ast::dump(&src, &program, Color::Always);
        let plain = ramos::ast::dump(&src, &program, Color::Never);
        assert_eq!(
            strip(&painted),
            plain,
            "ast dump of {name} changed under colour"
        );
    }
}

/// The highlighter walks token spans and passes the gaps through, so this is
/// the check that between them they account for the source exactly.
#[test]
fn highlighting_preserves_the_source_exactly() {
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(&path).unwrap();
        let tokens = lex(&src).unwrap();
        assert_eq!(
            strip(&lexer::highlight(&src, &tokens, Color::Always)),
            src,
            "highlighting {name} lost or duplicated source text"
        );
    }
}

#[test]
fn plain_output_carries_no_escapes() {
    let src = "\
module Payments
  function max()
    10
";
    let tokens = lex(src).unwrap();
    let program = parse(tokens.clone()).unwrap();
    for out in [
        lexer::dump(src, &tokens, Color::Never),
        lexer::render(&tokens, Color::Never),
        ramos::ast::dump(src, &program, Color::Never),
        ramos::ast::render(&program, Color::Never),
        lexer::highlight(src, &tokens, Color::Never),
    ] {
        assert!(
            !out.contains('\x1b'),
            "escape leaked into plain output: {out:?}"
        );
    }
}

// ── what gets painted ────────────────────────────────────────────────────────

#[test]
fn paint_wraps_text_and_plain_is_a_no_op() {
    assert_eq!(
        Color::Always.paint(Style::Keyword, "module"),
        "\x1b[35mmodule\x1b[0m"
    );
    assert_eq!(
        Color::Always.paint(Style::Plain, "x"),
        "x",
        "Plain uses the default fg"
    );
    assert_eq!(Color::Never.paint(Style::Keyword, "module"), "module");
}

#[test]
fn source_is_painted_by_token_category() {
    let src = "\
module Payments   # a note
  function max()
    \"ten\"
";
    let tokens = lex(src).unwrap();
    let out = lexer::highlight(src, &tokens, Color::Always);

    assert!(
        out.contains("\x1b[35mmodule\x1b[0m"),
        "keyword magenta:\n{out}"
    );
    assert!(
        out.contains("\x1b[33mPayments\x1b[0m"),
        "module name yellow:\n{out}"
    );
    assert!(
        out.contains("\x1b[32m\"ten\"\x1b[0m"),
        "string green:\n{out}"
    );
    assert!(
        out.contains("\x1b[2m# a note\x1b[0m"),
        "comment dimmed:\n{out}"
    );
    // `max` is an identifier: it keeps the default fg.
    assert!(out.contains("max()"), "identifiers stay unpainted:\n{out}");
}

#[test]
fn a_hash_inside_a_string_is_painted_as_string_not_comment() {
    // The `#` lives inside the string's token, so it never reaches the
    // comment-dimming path.
    let src = "marker = \"# not a comment\"\n";
    let tokens = lex(src).unwrap();
    let out = lexer::highlight(src, &tokens, Color::Always);
    assert!(
        out.contains("\x1b[32m\"# not a comment\"\x1b[0m"),
        "the whole string should be green:\n{out}"
    );
    assert!(
        !out.contains("\x1b[2m"),
        "nothing here is a comment:\n{out}"
    );
}

#[test]
fn the_tree_paints_keywords_literals_and_labels() {
    let src = "x = 1\n";
    let program = parse(lex(src).unwrap()).unwrap();
    let out = ramos::ast::render(&program, Color::Always);
    assert!(
        out.contains("\x1b[1mProgram\x1b[0m"),
        "root is bold:\n{out}"
    );
    assert!(
        out.contains("\x1b[2mpattern\x1b[0m"),
        "labels dimmed:\n{out}"
    );
    assert!(
        out.contains("\x1b[36mInt 1\x1b[0m"),
        "literals cyan:\n{out}"
    );
    assert!(out.contains("Assign"), "{out}");
}

// ── the CLI decides colour from the destination ──────────────────────────────

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/features/struct.rmo");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ramos"));
    cmd.args(args).arg(&fixture);
    cmd.env_remove("NO_COLOR");
    cmd.env_remove("FORCE_COLOR");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to run the ramos binary");
    assert!(
        out.status.success(),
        "`ramos {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn piped_output_is_plain_but_the_color_flag_forces_paint() {
    // A test harness captures stdout through a pipe, which is exactly the
    // "not a terminal" case: no escapes unless asked.
    for args in [
        vec!["ast", "--dump"],
        vec!["lexer", "--dump"],
        vec!["ast"],
        vec!["lexer"],
    ] {
        assert!(
            !run(&args, &[]).contains('\x1b'),
            "`ramos {}` painted a pipe",
            args.join(" ")
        );
    }
    for args in [
        vec!["ast", "--dump", "--color"],
        vec!["lexer", "--dump", "--color"],
    ] {
        assert!(
            run(&args, &[]).contains('\x1b'),
            "`ramos {}` ignored --color",
            args.join(" ")
        );
    }
}

#[test]
fn force_color_paints_a_pipe_and_no_color_and_the_flag_override_it() {
    let args = ["ast", "--dump"];
    assert!(
        run(&args, &[("FORCE_COLOR", "1")]).contains('\x1b'),
        "FORCE_COLOR should paint even a pipe"
    );
    assert!(
        !run(&args, &[("FORCE_COLOR", "1"), ("NO_COLOR", "1")]).contains('\x1b'),
        "NO_COLOR should win over FORCE_COLOR"
    );
    assert!(
        !run(&["ast", "--dump", "--no-color"], &[("FORCE_COLOR", "1")]).contains('\x1b'),
        "--no-color should win over FORCE_COLOR"
    );
    // An explicit flag is a deliberate choice, so it beats the environment.
    assert!(
        run(&["ast", "--dump", "--color"], &[("NO_COLOR", "1")]).contains('\x1b'),
        "--color should win over NO_COLOR"
    );
}
