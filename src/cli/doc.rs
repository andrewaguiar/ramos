//! `ramos doc` — generate a stdlib reference and serve it over HTTP on
//! `127.0.0.1:3030` (or `--port`), blocking until killed. Deliberately takes
//! nothing else: a quick preview needs no configuring, and it works from any
//! directory, not just a checkout of this project. It documents `.`'s own
//! `./stdlib` when that exists (plus `./tests/fixtures/features`,
//! `./examples` and `./README.md`, for developing *this* project's own docs)
//! and falls back to the stdlib embedded in the binary otherwise — the same
//! one `run`/`check`/the REPL already load without `--stdlib` — so `ramos
//! doc` run from an arbitrary directory still has the language's own
//! reference to show. Either way it writes into a temp directory it manages
//! itself. Reach for `ramos generate-docs` instead to point any of this
//! somewhere else, or to keep the output.

use super::{err_tag, take_opt};
use ramos::color::Color;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The flags `ramos doc` and `ramos generate-docs` share (everything but
/// `--out`, whose default differs between the two — a temp directory for one,
/// `ramos-docs` for the other — and `--port`, which only `doc` takes).
/// Pulls its recognized flags out of `args`, leaving whatever is left for the
/// caller to reject as unknown.
pub fn build(args: &mut Vec<String>, out_dir: &Path) -> Result<usize, String> {
    let stdlib = take_opt(args, "--stdlib");
    let examples = take_opt(args, "--examples");
    let programs = take_opt(args, "--programs");
    let readme = take_opt(args, "--readme");
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // The doc generator reads modules, which live under the root's `src/`.
    let stdlib_root = match stdlib {
        Some(s) => PathBuf::from(s),
        None => here.join("stdlib"),
    };
    let stdlib_dir = stdlib_root.join("src");
    // Missing sources aren't fatal — the page in question is simply left out.
    let examples_dir = match examples {
        Some(s) => PathBuf::from(s),
        None => here.join("tests").join("fixtures").join("features"),
    };
    let programs_dir = match programs {
        Some(s) => PathBuf::from(s),
        None => here.join("examples"),
    };
    let readme_file = match readme {
        Some(s) => PathBuf::from(s),
        None => here.join("README.md"),
    };
    let opts = ramos::doc::Options {
        examples_dir: Some(&examples_dir),
        programs_dir: Some(&programs_dir),
        readme: Some(&readme_file),
    };
    ramos::doc::generate_with(&stdlib_dir, out_dir, &opts)
}

pub fn doc(args: &[String], color: Color) -> ExitCode {
    let mut args: Vec<String> = args.to_vec();
    let port_arg = take_opt(&mut args, "--port");
    let port = match port_arg.as_deref().map(str::parse::<u16>) {
        None => 3030,
        Some(Ok(p)) => p,
        Some(Err(_)) => {
            eprintln!(
                "{} --port must be a number between 0 and 65535, got `{}`",
                err_tag(color),
                port_arg.unwrap()
            );
            return ExitCode::from(2);
        }
    };
    if !args.is_empty() {
        eprintln!("usage: ramos doc [--port PORT]");
        eprintln!("  --port PORT    port to serve on (default: 3030)");
        return ExitCode::from(2);
    }
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let stdlib_dir = here.join("stdlib").join("src");
    let examples_dir = here.join("tests").join("fixtures").join("features");
    let programs_dir = here.join("examples");
    let readme_file = here.join("README.md");
    let opts = ramos::doc::Options {
        examples_dir: Some(&examples_dir),
        programs_dir: Some(&programs_dir),
        readme: Some(&readme_file),
    };
    let out_dir = std::env::temp_dir().join(format!("ramos-docs-{}", std::process::id()));
    // A local `stdlib/src` (this project's own checkout, or any other Ramos
    // project's) wins when it is there; otherwise there is always the
    // embedded stdlib to fall back to, which is what makes `ramos doc` work
    // anywhere at all.
    let result = if stdlib_dir.is_dir() {
        ramos::doc::generate_with(&stdlib_dir, &out_dir, &opts)
    } else {
        println!("no ./stdlib here — documenting the stdlib built into this binary instead");
        ramos::doc::generate_embedded(&out_dir, &opts)
    };
    if let Err(e) = result {
        eprintln!("{} {e}", err_tag(color));
        return ExitCode::FAILURE;
    }
    println!("serving docs at http://localhost:{port} (Ctrl-C to stop)");
    match ramos::docserver::serve(&out_dir, port) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e}", err_tag(color));
            ExitCode::FAILURE
        }
    }
}
