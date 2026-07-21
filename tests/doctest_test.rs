//! Phase 8 acceptance: the `@doc` examples in the stdlib are runnable, and
//! `ramos doctest` runs them.

use ramos::doctest::examples_in;
use std::fs;
use std::path::{Path, PathBuf};

fn stdlib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib")
}

/// Run the binary from the repo root. `doctest` moves the process between
/// directories while it works, so it is driven as a child process — that keeps
/// the cd out of the test harness, which runs its tests in parallel threads.
fn doctest(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_ramos"))
        .arg("doctest")
        .args(args)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("failed to run the ramos binary")
}

// ── the acceptance criterion ─────────────────────────────────────────────────

#[test]
fn every_stdlib_doc_example_produces_what_it_claims() {
    let out = doctest(&["--stdlib", "stdlib"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "doctest failed:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains(", 0 failed"), "{stdout}");
    // Every module documents itself, including the ones that touch the
    // filesystem — the sandbox is what made those runnable.
    for module in [
        "actor", "config", "dir", "file", "global", "kernel", "integer", "float", "list", "map",
        "string", "struct", "thread", "tuple",
    ] {
        let source = fs::read_to_string(stdlib_root().join("src").join(format!("{module}.rmo")))
            .expect("stdlib source");
        // `actor` is a trait of declarations, so it carries prose but no `# ==`.
        if module != "actor" {
            assert!(
                !examples_in(module, &source).is_empty(),
                "{module}.rmo carries no runnable examples"
            );
        }
    }
}

#[test]
fn the_run_covers_every_example_the_sources_carry() {
    // Every `# ==` line in a source should become an example, and every example
    // should be run: the two counts and the report's total agree.
    let mut claimed = 0usize;
    let mut extracted = 0usize;
    for (stem, source) in ramos::doctest::sources(&stdlib_root()).expect("stdlib sources") {
        claimed += source.lines().filter(|l| l.contains("# ==")).count();
        extracted += examples_in(&stem, &source).len();
    }
    assert_eq!(extracted, claimed, "extracted vs `# ==` lines");
    assert!(
        claimed > 200,
        "expected plenty of examples, found {claimed}"
    );

    let stdout = String::from_utf8_lossy(&doctest(&["--stdlib", "stdlib"]).stdout).to_string();
    assert!(
        stdout.contains(&format!("{claimed} example(s), {claimed} passed")),
        "{stdout}"
    );
}

// ── the extractor ────────────────────────────────────────────────────────────

#[test]
fn an_example_carries_the_bindings_above_it() {
    let source = "\
  fn f(x)
    # @doc
    #
    #   base = 10
    #   base + 1   # == 11
    x
";
    let examples = examples_in("demo", source);
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0].setup, vec!["base = 10".to_string()]);
    assert_eq!(examples[0].expr, "base + 1");
    assert_eq!(examples[0].expected, "11");
}

#[test]
fn prose_between_snippets_ends_the_scope() {
    let source = "\
  fn f(x)
    # @doc
    #
    #   base = 10
    #
    # And a second snippet, which cannot see `base`:
    #
    #   1 + 1   # == 2
    x
";
    let examples = examples_in("demo", source);
    assert_eq!(examples.len(), 1);
    assert!(examples[0].setup.is_empty(), "{:?}", examples[0].setup);
}

#[test]
fn a_setup_snippet_runs_before_every_example_in_the_file() {
    let source = "\
module Demo
  # @module_doc
  #
  #   # ramos doctest setup
  #   struct Person
  #     attributes
  #       name: nil
  #
  # Then:
  #
  #   Person{name: \"Andrew\"}.name   # == \"Andrew\"

  fn f(x)
    # @doc
    #
    #   Person{}.name   # == nil
    x
";
    let examples = examples_in("demo", source);
    assert_eq!(examples.len(), 2, "the setup itself must not be asserted");
    for example in &examples {
        assert_eq!(
            example.setup.first().map(String::as_str),
            Some("struct Person\n  attributes\n    name: nil"),
            "every example gets the preamble"
        );
    }
}

#[test]
fn a_pipeline_asserts_at_each_stage() {
    let source = "\
  fn f(x)
    # @doc
    #
    #   [1, 2, 3]
    #   | List.map(do n -> n * 2)   # == [2, 4, 6]
    #   | List.sum()                # == 12
    x
";
    let examples = examples_in("demo", source);
    assert_eq!(examples.len(), 2);
    assert_eq!(examples[0].expected, "[2, 4, 6]");
    assert_eq!(examples[1].expected, "12");
    assert!(
        examples[1].expr.contains("List.sum()"),
        "{}",
        examples[1].expr
    );
}

// ── the command ──────────────────────────────────────────────────────────────

#[test]
fn a_drifted_doc_fails_the_run_and_names_the_line() {
    // A doc that claims what the code does not do is the whole point of the
    // command, so it must exit non-zero and say where.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("doctest_drift");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("create project");
    fs::write(
        dir.join("src").join("demo.rmo"),
        "\
module Demo
  fn double(x)
    # @doc
    #
    #   Demo.double(2)   # == 5
    x * 2
",
    )
    .expect("write module");

    let out = doctest(&[dir.to_str().unwrap()]);
    assert!(!out.status.success(), "expected a non-zero exit");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("demo.rmo:5"), "{stdout}");
    assert!(stdout.contains("got 4"), "{stdout}");
    assert!(
        stdout.contains("1 example(s), 0 passed, 1 failed"),
        "{stdout}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn examples_are_sandboxed_from_each_other_and_from_the_tree() {
    // Two examples writing the same relative path must not see each other, and
    // neither may leave anything behind in the directory the run started from.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("doctest_sandbox");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("create project");
    fs::write(
        dir.join("src").join("demo.rmo"),
        "\
module Demo
  fn f(x)
    # @doc
    #
    #   File.write(\"scratch.txt\", \"one\")
    #   File.read(\"scratch.txt\")   # == (:ok, \"one\")
    #
    # A second example, which must not see the first one's file:
    #
    #   File.exists(\"scratch.txt\")   # == false
    x
",
    )
    .expect("write module");

    let out = doctest(&[dir.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("2 example(s), 2 passed"), "{stdout}");
    // The repo root the command ran from is untouched.
    assert!(!Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("scratch.txt")
        .exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_with_no_sources_is_an_error_not_a_pass() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("doctest_empty");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create project");
    let out = doctest(&[dir.to_str().unwrap()]);
    assert!(!out.status.success(), "expected a non-zero exit");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot read"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}
