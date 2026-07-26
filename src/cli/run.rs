//! `ramos run` — load the entry file with the stdlib and every module it
//! reaches, then execute it. Also backs `ramos run -e CODE` (a snippet with
//! no file on disk) and the shared tail both forms run once loaded.

use super::{err_tag, load};
use ramos::color::{Color, Style};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `ramos run <file.rmo>` / `ramos run <dir>`.
///
/// `require_main` is set when `path` was picked by finding a directory's
/// `main.rmo` rather than named directly: that file is expected to *be* an
/// entrypoint, so — unlike a file named on the command line, which is free to
/// be a bare top-level script (see the README's "Entrypoints" section) — it
/// is an error for it to define no module exposing a public `function
/// main()`, rather than a silent fall-through to running its top-level
/// statements.
pub fn run(
    path: &str,
    stdlib: Option<String>,
    prog_args: &[String],
    require_main: bool,
    color: Color,
) -> ExitCode {
    let stdlib_dir = load::stdlib_dir(stdlib);
    let program = match load::program(path, stdlib_dir.as_deref()) {
        Ok(p) => p,
        Err(code) => return code,
    };
    if require_main && !has_entrypoint(&program) {
        eprintln!(
            "{} `{path}` defines no module exposing a public `function main()` — \
             a directory is only run through its `main.rmo`, so that file has to be a \
             real entrypoint",
            err_tag(color)
        );
        return ExitCode::FAILURE;
    }
    run_program(&program, prog_args, color)
}

/// `ramos run -e CODE` — run `CODE` as a snippet, without a `.rmo` file on
/// disk. `CODE` is loaded exactly like a bare top-level script named on the
/// command line (see [`run`]'s doc comment): free to hold top-level
/// statements with no `module`, and not required to expose a `function
/// main()`. It can still reach a project's own modules under `./src`, since
/// [`ramos::loader::load_source`] roots the snippet at the current directory.
pub fn run_eval(
    code: &str,
    stdlib: Option<String>,
    prog_args: &[String],
    color: Color,
) -> ExitCode {
    let stdlib_dir = load::stdlib_dir(stdlib);
    let program = match ramos::loader::load_source(code, stdlib_dir.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            let text = e.to_string();
            if text.ends_with('\n') {
                eprint!("{text}");
            } else {
                eprintln!("{text}");
            }
            return ExitCode::FAILURE;
        }
    };
    run_program(&program, prog_args, color)
}

/// Execute an already-loaded program and translate the result into an exit
/// code — the shared tail of `ramos run <file>` and `ramos run -e CODE`, which
/// differ only in how the `Program` was built.
fn run_program(program: &ramos::ast::Program, prog_args: &[String], color: Color) -> ExitCode {
    // Line-buffered (the default `stdout()` behaviour), not block-buffered: a
    // `Thread.start`ed lambda writes to this same sink live, and its output must
    // reach the terminal as it happens rather than sit in an 8 KB buffer. The
    // sink is shared with those threads, so writes serialize through its mutex.
    let stdout = ramos::interp::sink(std::io::stdout());
    let result = ramos::interp::run_with_args(program, stdout.clone(), prog_args);
    let flushed = stdout.lock().unwrap_or_else(|e| e.into_inner()).flush();
    // An `exit(code)` unwinds as an error carrying its status — it is not a
    // failure, so check for it before reporting anything.
    if let Err(e) = &result {
        if let Some(code) = e.exit_code {
            return ExitCode::from(code.rem_euclid(256) as u8);
        }
    }
    match (result, flushed) {
        (Ok(_), Ok(())) => ExitCode::SUCCESS,
        (Err(e), _) => {
            // Matches the tag `ramos repl` paints for the same message.
            eprintln!("{} {e}", color.paint(Style::Heading, "runtime error:"));
            ExitCode::FAILURE
        }
        (Ok(_), Err(e)) => {
            eprintln!("{} could not write output: {e}", err_tag(color));
            ExitCode::FAILURE
        }
    }
}

/// Whether `program` defines a module exposing a public `function main()` —
/// the same definition of "entrypoint" `ramos::interp::run` itself uses.
fn has_entrypoint(program: &ramos::ast::Program) -> bool {
    program.items.iter().any(|item| match item {
        ramos::ast::Item::Module(m) => m.functions.iter().any(|f| f.name == "main" && !f.private),
        _ => false,
    })
}

/// The first `main.rmo` under `dir`, breadth-first — what `ramos run <dir>`
/// runs when it is pointed at a directory rather than a `.rmo` file. A
/// shallower `main.rmo` wins over a deeper one; among siblings, the
/// alphabetically first wins, so the search is deterministic even though "the
/// first one found" is otherwise a filesystem-order question.
pub fn find_main_file(dir: &Path) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(dir.to_path_buf());
    while let Some(current) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        let mut subdirs = Vec::new();
        for path in children {
            if path.is_dir() {
                let hidden = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'));
                if !hidden {
                    subdirs.push(path);
                }
            } else if path.file_name().and_then(|n| n.to_str()) == Some("main.rmo") {
                return Some(path);
            }
        }
        queue.extend(subdirs);
    }
    None
}
