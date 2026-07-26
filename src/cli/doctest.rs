//! `ramos doctest [dir]` — run the `# ==` examples in a directory's `@doc`
//! blocks.

use super::err_tag;
use ramos::color::Color;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

/// The directory is a project root, so its modules are `DIR/src/*.rmo` — the
/// same shape `--stdlib` names and any Ramos project uses. With no `DIR`, it
/// is `.` — the same bare-means-current-directory default as `run` — except
/// when `--stdlib DIR` is given alone, which names both the sources under
/// test and what they load against: `ramos doctest --stdlib stdlib` documents
/// the stdlib against itself without repeating the path.
///
/// `--stdlib DIR` is what the examples load against, exactly as it is for `run`
/// and `check`: without it they load against the copy embedded in this binary.
/// A project's *own* modules are reachable either way — they are copied in
/// beside each example.
pub fn doctest(args: &[String], stdlib: Option<String>, quietly: bool, color: Color) -> ExitCode {
    let args: Vec<String> = args.to_vec();
    if args.len() > 1 {
        eprintln!("usage: ramos doctest [--quietly] [--stdlib DIR] [DIR]");
        eprintln!("  DIR            project root, modules read from DIR/src");
        eprintln!("                 (default: the --stdlib directory, else .)");
        eprintln!("  --stdlib DIR   the stdlib the examples load against (default: embedded)");
        return ExitCode::from(2);
    }
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let stdlib_dir = stdlib.map(PathBuf::from);
    // Documenting the stdlib is the common case, so `--stdlib` alone names both
    // the sources under test and what they load against. Otherwise bare
    // `doctest` is `doctest .`, matching bare `run`.
    let root = match (args.first(), &stdlib_dir) {
        (Some(s), _) => PathBuf::from(s),
        (None, Some(dir)) => dir.clone(),
        (None, None) => here,
    };
    let report = match ramos::doctest::run(&root, stdlib_dir.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e}", err_tag(color));
            return ExitCode::FAILURE;
        }
    };
    let mut stdout = std::io::stdout();
    if let Err(e) = ramos::doctest::write_report(&report, &mut stdout, quietly, color) {
        eprintln!("{} could not write output: {e}", err_tag(color));
        return ExitCode::FAILURE;
    }
    if report.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
