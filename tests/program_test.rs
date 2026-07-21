//! Phase 8 golden tests: whole programs under `tests/programs/`, each with the
//! stdout it is expected to print.
//!
//! These are end-to-end in a way the unit tests are not — a program is loaded
//! with the stdlib, resolves its own modules, and runs to completion. A change
//! that alters what a real program prints shows up here as a diff, whatever
//! layer of the pipeline caused it.
//!
//! Adding one: drop `tests/programs/<name>.rmo` in, run
//! `UPDATE_GOLDEN=1 cargo test --test program_test` to record its output, and
//! read the generated `<name>.out` before committing it — an unread golden file
//! asserts nothing except that the code kept doing whatever it did.

use std::fs;
use std::path::{Path, PathBuf};

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("programs")
}

/// The `.rmo` entry files, sorted. Only the top level: `src/` under it holds
/// the modules those programs reach, not programs of their own.
fn program_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(programs_dir())
        .expect("read tests/programs")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rmo"))
        .collect();
    files.sort();
    files
}

/// Run a program the way a user would: the built binary, from the repo root.
///
/// A child process rather than an in-process `load` + `run`: the stdlib
/// recurses once per element, and a test thread's stack is far smaller than the
/// main thread's, so a program that runs fine for a user would abort the test
/// harness. Driving the binary also makes the golden file assert what the
/// shipped command prints, which is what it claims to be.
fn run_program(entry: &Path) -> Result<String, String> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ramos"))
        .args(["run", entry.to_str().expect("utf8 path")])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .map_err(|e| format!("cannot run the ramos binary: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

#[test]
fn every_program_prints_what_its_golden_file_says() {
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let files = program_files();
    assert!(!files.is_empty(), "no programs in tests/programs");

    let mut failures = Vec::new();
    for entry in &files {
        let golden = entry.with_extension("out");
        let actual = match run_program(entry) {
            Ok(out) => out,
            Err(e) => {
                failures.push(format!("{}: did not run: {e}", entry.display()));
                continue;
            }
        };
        if update {
            fs::write(&golden, &actual).expect("write golden file");
            continue;
        }
        let expected = match fs::read_to_string(&golden) {
            Ok(text) => text,
            Err(_) => {
                failures.push(format!(
                    "{}: no golden file — run with UPDATE_GOLDEN=1 to record one",
                    golden.display()
                ));
                continue;
            }
        };
        if actual != expected {
            failures.push(format!(
                "{}:\n  expected: {:?}\n  actual:   {:?}",
                entry.display(),
                expected,
                actual
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} programs differ from their golden output:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn every_program_has_a_golden_file() {
    // A program with no recorded output would pass the test above by accident
    // once the file is missing, so the pairing is asserted on its own.
    for entry in program_files() {
        let golden = entry.with_extension("out");
        assert!(
            golden.is_file(),
            "{} has no golden output file",
            entry.display()
        );
        assert!(
            !fs::read_to_string(&golden).unwrap_or_default().is_empty(),
            "{} is empty",
            golden.display()
        );
    }
}
