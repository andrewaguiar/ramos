//! Runs the Ramos-language tests under `stdlib/src/test/`.
//!
//! Those files are ordinary Ramos test modules — the same thing `ramos test`
//! runs — so the stdlib is exercised by Ramos code rather than by Rust
//! assertions about it. Wiring them into `cargo test` keeps them honest: a
//! stdlib change that breaks one fails the build.

use std::path::{Path, PathBuf};

fn test_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib/src/test")
}

/// Every `.rmo` file under the Ramos test root.
fn test_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)
            .expect("read test dir")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rmo") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_ramos_written_stdlib_tests_all_pass() {
    let files = test_files(&test_root());
    assert!(!files.is_empty(), "no Ramos test files found");

    let mut failures = Vec::new();
    let mut total = 0;
    for file in &files {
        let program = ramos::loader::load(file, None)
            .unwrap_or_else(|e| panic!("load {}: {e}", file.display()));
        let outcomes = ramos::interp::run_tests(&program, ramos::interp::sink(Vec::new()), &[])
            .unwrap_or_else(|e| panic!("run {}: {e}", file.display()));
        assert!(
            !outcomes.is_empty(),
            "{} defines no tests — a file here is expected to hold some",
            file.display()
        );
        for outcome in outcomes {
            total += 1;
            if let Some(why) = outcome.failure {
                failures.push(format!("{}.{}: {why}", outcome.module, outcome.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {total} Ramos tests failed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
