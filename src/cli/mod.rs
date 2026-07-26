//! The `ramos` CLI: argument parsing and dispatch to one module per
//! subcommand. `run()` is main.rs's entire body — everything else lives here
//! so main.rs stays a one-line binary entrypoint.

mod ast;
mod check;
mod doc;
mod doctest;
mod generate_docs;
mod help;
mod learn;
mod lexer;
mod load;
mod new;
mod repl;
mod run;
mod see;
mod test;
mod version;

use ramos::color::{Color, Style};
use std::env;
use std::path::Path;
use std::process::ExitCode;

/// The crate's own version, from `Cargo.toml` — what `ramos version` and bare
/// `ramos` print.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Paint just the leading `error:` tag, the same convention `ramos test` /
/// `ramos doctest` use for their own `ok`/`FAIL` tags — the message after it
/// stays in the terminal's own color.
fn err_tag(color: Color) -> String {
    color.paint(Style::Keyword, "error:")
}

/// Remove `flag` from `args` if present, reporting whether it was there.
fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

/// Pull the value following `flag` (e.g. `--out docs/`) out of `args`.
fn take_opt(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    let val = args.get(i + 1)?.clone();
    args.remove(i);
    args.remove(i);
    Some(val)
}

/// Everything runs on a thread with a large stack: the parser is recursive
/// descent and the stdlib recurses per element, so the default stack overflows
/// on ordinary input (a few hundred list elements). See `ramos::stack`;
/// `main` wraps this in `with_large_stack`.
pub fn run() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let dump = take_flag(&mut args, "--dump");
    let quietly = take_flag(&mut args, "--quietly");
    let stdlib = take_opt(&mut args, "--stdlib");
    // Only `run` takes `-e`, but it is pulled out here alongside the other
    // options rather than deep in the dispatch below, matching how `--stdlib`
    // and the rest are handled regardless of which command is running.
    let eval = take_opt(&mut args, "-e");
    // Default to painting a terminal and staying plain when piped; the flags
    // are the override for a pager (`ramos ast --dump f.rmo --color | less -R`).
    let color = if take_flag(&mut args, "--no-color") {
        Color::Never
    } else if take_flag(&mut args, "--color") {
        Color::Always
    } else {
        Color::for_stdout()
    };
    // `doc` and `generate-docs` are directory-level commands (no `.rmo`
    // file), so handle them before the file-based dispatch below.
    if args.first().map(String::as_str) == Some("doc") {
        return doc::doc(&args[1..], color);
    }
    if args.first().map(String::as_str) == Some("generate-docs") {
        return generate_docs::generate_docs(&args[1..], color);
    }
    // `new` takes a project name, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("new") {
        return new::new(&args[1..], color);
    }
    // `learn` takes no file — it prints a fixed crash course on the language.
    if args.first().map(String::as_str) == Some("learn") {
        return learn::learn();
    }
    // `version` (and bare `ramos`, below) print the same line.
    if args.first().map(String::as_str) == Some("version") {
        return version::version();
    }
    // `run -e CODE` runs a snippet instead of a `.rmo` file, so it is handled
    // before the file-based `(cmd, path)` dispatch below, which expects a path
    // in `args[1]`.
    if args.first().map(String::as_str) == Some("run") {
        if let Some(code) = &eval {
            let prog_args: Vec<String> = args.iter().skip(1).cloned().collect();
            return run::run_eval(code, stdlib, &prog_args, color);
        }
    }
    // `repl` takes no file either — an interactive session on stdin.
    if args.first().map(String::as_str) == Some("repl") {
        return repl::repl(stdlib, color);
    }
    // `doctest` takes an optional stdlib root, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("doctest") {
        return doctest::doctest(&args[1..], stdlib, quietly, color);
    }
    // `see` takes a module name, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("see") {
        return see::see(&args[1..], stdlib, color);
    }
    // `test` takes an optional file: with one, run that file's tests; without,
    // find every test module under the current directory.
    if args.first().map(String::as_str) == Some("test") {
        return test::test(args.get(1).map(String::as_str), stdlib, quietly, color);
    }
    let (cmd, path) = match (args.first().map(String::as_str), args.get(1)) {
        (Some(c @ ("run" | "check" | "lexer" | "ast")), Some(p)) => (c, p.as_str()),
        // Bare `ramos run` is `ramos run .` — the directory-resolution below
        // finds `.`'s `main.rmo` exactly as it would for any named directory.
        (Some("run"), None) => ("run", "."),
        _ => return help::usage(),
    };
    // `ramos run <dir>` runs that directory's `main.rmo` instead of naming the
    // entrypoint file directly. Since the file was picked by the `main.rmo`
    // name rather than named by the caller, it is held to what that name
    // promises: an entrypoint, not just any runnable script.
    let resolved;
    let mut require_main = false;
    let path = if cmd == "run" && Path::new(path).is_dir() {
        match run::find_main_file(Path::new(path)) {
            Some(found) => {
                resolved = found.display().to_string();
                require_main = true;
                resolved.as_str()
            }
            None => {
                eprintln!("{} no `main.rmo` found under `{path}`", err_tag(color));
                return ExitCode::FAILURE;
            }
        }
    } else {
        path
    };
    if !path.ends_with(".rmo") {
        eprintln!(
            "{} Ramos source files must use the `.rmo` extension (got `{path}`)",
            err_tag(color)
        );
        let mut renamed = std::path::PathBuf::from(path);
        renamed.set_extension("rmo");
        eprintln!("  wrong:   ramos run {path}");
        eprintln!("  correct: ramos run {}", renamed.display());
        return ExitCode::FAILURE;
    }
    // `run` and `check` see a whole program — the stdlib plus every module the
    // entry file reaches. The debug commands stay on the single file in front
    // of them, which is the point of `lexer` and `ast`.
    match cmd {
        "run" => {
            // Anything after the script path is a program argument, exposed to
            // the program via `get_args` / `get_arg`.
            let prog_args: Vec<String> = args.iter().skip(2).cloned().collect();
            run::run(path, stdlib, &prog_args, require_main, color)
        }
        "check" => check::check(path, stdlib, color),
        "lexer" => lexer::lexer(path, dump, color),
        // `cmd` is "ast" here — `run`/`check` returned above.
        _ => ast::ast(path, dump, color),
    }
}
