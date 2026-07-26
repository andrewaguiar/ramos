//! The loader step `run` and `check` share: turn a path (plus an optional
//! `--stdlib` override) into a loaded [`ramos::ast::Program`], printing the
//! loader's own pre-rendered diagnostic and returning an exit code on
//! failure so each caller does not have to repeat that plumbing.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Load `path` against `stdlib_dir` (or the embedded stdlib when `None`).
///
/// Diagnostics arrive pre-rendered (and already end in a newline); the
/// loader's own messages are a single line — either way, this prints exactly
/// what the caller should show and hands back `Err(ExitCode::FAILURE)`.
pub fn program(path: &str, stdlib_dir: Option<&Path>) -> Result<ramos::ast::Program, ExitCode> {
    ramos::loader::load(Path::new(path), stdlib_dir).map_err(|e| {
        let text = e.to_string();
        if text.ends_with('\n') {
            eprint!("{text}");
        } else {
            eprintln!("{text}");
        }
        ExitCode::FAILURE
    })
}

/// `stdlib` as given on the command line, resolved to a path.
pub fn stdlib_dir(stdlib: Option<String>) -> Option<PathBuf> {
    stdlib.map(PathBuf::from)
}
