//! `ramos test [filter]` — run every test module under the nearest `src/test`
//! (walking up from the current directory), optionally narrowed to the files
//! whose name or path contains `filter`.

use super::err_tag;
use ramos::color::{Color, Style};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Where tests live. A test module follows the same file rules as any other —
/// its namespace is its path — rooted here rather than at `src/`.
const TEST_ROOT: &str = "src/test";

/// A file is a test file when it defines a module implementing `Test`. Each
/// file is loaded and run on its own interpreter, so one file's definitions and
/// state cannot leak into another's.
///
/// A run reads as a description of what the suite covers: the module's
/// `@module_doc` heads its section and each test's `@doc` follows its name, so
/// the report says what a test is for and not only that it ran. `--quietly`
/// leaves the docs out, for a run watched by a person who already knows them
/// (or by a CI log that does not need them).
pub fn test(filter: Option<&str>, stdlib: Option<String>, quietly: bool, color: Color) -> ExitCode {
    let stdlib_dir = stdlib.map(PathBuf::from);
    let Some(root) = find_test_root() else {
        println!("no `{TEST_ROOT}` directory found in `.` or any parent — tests live there");
        return ExitCode::SUCCESS;
    };
    let mut files = match find_test_files(&root) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("{} {e}", err_tag(color));
            return ExitCode::FAILURE;
        }
    };
    if let Some(needle) = filter {
        files.retain(|f| f.to_string_lossy().contains(needle));
    }
    if files.is_empty() {
        match filter {
            Some(needle) => println!(
                "no test file under `{}` has `{needle}` in its name or path",
                root.display()
            ),
            None => println!("no test modules found in `{}`", root.display()),
        }
        return ExitCode::SUCCESS;
    }

    let (mut passed, mut failed) = (0usize, 0usize);
    for file in &files {
        let program = match ramos::loader::load(file, stdlib_dir.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = check_test_module(file, &parse_only(file)) {
            eprintln!("{} {e}", err_tag(color));
            return ExitCode::FAILURE;
        }
        // Docs live in comments, so they come from the file's own text rather
        // than from the loaded program.
        let docs = if quietly {
            ramos::doc::Summaries::default()
        } else {
            match std::fs::read_to_string(file) {
                Ok(source) => ramos::doc::summaries(&source),
                // The loader already read this file successfully; a failure
                // here is not worth failing the run over, only the docs.
                Err(_) => ramos::doc::Summaries::default(),
            }
        };
        let outcomes =
            match ramos::interp::run_tests(&program, ramos::interp::sink(std::io::stdout()), &[]) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("{} {}: {e}", err_tag(color), file.display());
                    return ExitCode::FAILURE;
                }
            };
        let mut current = String::new();
        for outcome in outcomes {
            if outcome.module != current {
                println!("{}", color.paint(Style::Heading, &outcome.module));
                if let Some(summary) = &docs.module {
                    println!("  {}", color.paint(Style::Dim, summary));
                }
                current = outcome.module.clone();
            }
            match &outcome.failure {
                None => {
                    passed += 1;
                    println!("  {} {}", color.paint(Style::Str, "ok"), outcome.name);
                }
                Some(why) => {
                    failed += 1;
                    println!("  {} {}", color.paint(Style::Keyword, "FAIL"), outcome.name);
                    println!("      {why}");
                }
            }
            // Under the result, so the name and its `ok`/`FAIL` still line up
            // down the column.
            if let Some(summary) = docs.functions.get(&outcome.name) {
                println!("     {}", color.paint(Style::Dim, summary));
            }
        }
    }

    let total = passed + failed;
    println!();
    println!("{total} test(s): {passed} passed, {failed} failed");
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// The nearest `src/test`, walking up from the current directory — so `ramos
/// test` finds a project's tests from anywhere inside it, not only from its
/// root. `None` when no ancestor (up to the filesystem root) has one.
fn find_test_root() -> Option<PathBuf> {
    let mut dir = env::current_dir().ok()?;
    loop {
        let candidate = dir.join(TEST_ROOT);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Every `.rmo` file under `dir`, in path order.
///
/// Everything here is expected to be a test, so a file that is not one is
/// reported by [`check_test_module`] rather than quietly skipped — a test that
/// does not run is worse than one that fails.
fn find_test_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .map_err(|e| format!("cannot read `{}`: {e}", current.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !name.starts_with('.') {
                    stack.push(path);
                }
            } else if path.extension().and_then(|s| s.to_str()) == Some("rmo") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// The rules a test module is held to, beyond the ordinary file rules.
///
/// Its name must end in `Test`, so a test reads as one at its call site and in
/// a failure report, and it must implement the `Test` trait, so being run is
/// something a module opts into rather than something its name causes.
fn check_test_module(path: &Path, program: &ramos::ast::Program) -> Result<(), String> {
    let module = program.items.iter().find_map(|item| match item {
        ramos::ast::Item::Module(m) => Some(m),
        _ => None,
    });
    let Some(module) = module else {
        return Err(format!(
            "`{}`: a test file defines a module",
            path.display()
        ));
    };
    let name = module.name.to_string();
    let last = module.name.0.last().expect("a module path has a segment");
    if !last.ends_with("Test") {
        return Err(format!(
            "`{}` defines `{name}`, which must end in `Test` — that is what marks it a test module",
            path.display()
        ));
    }
    if !module.implements.iter().any(|t| t.to_string() == "Test") {
        return Err(format!(
            "`{name}` must `implements Test` to be run — being a test is opted into, not inferred \
             from the name"
        ));
    }
    // The same namespace-is-the-path rule every other file follows, rooted at
    // `src/test/`. The loader exempts entry files from it, and every test file
    // is loaded as one, so it is checked here.
    let expected = ramos::loader::expected_file(&module.name);
    if !path.ends_with(&expected) {
        return Err(format!(
            "`{}` defines `{name}`, which belongs at `{}/{}` — a test file's path is its \
             namespace",
            path.display(),
            TEST_ROOT,
            expected.display()
        ));
    }
    Ok(())
}

/// Parse one file on its own, for the checks that look at what it declares
/// rather than at the whole loaded program. A parse failure here has already
/// been reported by the loader, so an empty program is enough.
fn parse_only(path: &Path) -> ramos::ast::Program {
    let empty = ramos::ast::Program {
        items: Vec::new(),
        entry_file: std::sync::Arc::from(""),
    };
    let Ok(source) = std::fs::read_to_string(path) else {
        return empty;
    };
    let Ok(tokens) = ramos::lexer::lex(&source) else {
        return empty;
    };
    ramos::parser::parse(tokens).unwrap_or(empty)
}
