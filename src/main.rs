use ramos::color::Color;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

/// `ramos doc` — render the stdlib reference into `docs/`.
fn run_doc(args: &[String]) -> ExitCode {
    let mut args: Vec<String> = args.to_vec();
    let stdlib = take_opt(&mut args, "--stdlib");
    let out = take_opt(&mut args, "--out");
    let examples = take_opt(&mut args, "--examples");
    let programs = take_opt(&mut args, "--programs");
    let readme = take_opt(&mut args, "--readme");
    if !args.is_empty() {
        eprintln!(
            "usage: ramos doc [--stdlib DIR] [--out DIR] [--examples DIR] [--programs DIR] [--readme FILE]"
        );
        eprintln!("  --stdlib DIR   stdlib root, modules read from DIR/src (default: ./stdlib)");
        eprintln!("  --out DIR      where to write HTML (default: ./docs)");
        eprintln!(
            "  --examples DIR feature fixtures for the Examples page (default: ./tests/fixtures/features)"
        );
        eprintln!("  --programs DIR runnable programs for the Programs page (default: ./examples)");
        eprintln!("  --readme FILE  markdown for the guide page (default: ./README.md)");
        return ExitCode::from(2);
    }
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // The doc generator reads modules, which live under the root's `src/`.
    let stdlib_root = match stdlib {
        Some(s) => PathBuf::from(s),
        None => here.join("stdlib"),
    };
    let stdlib_dir = stdlib_root.join("src");
    let out_dir = match out {
        Some(s) => PathBuf::from(s),
        None => here.join("docs"),
    };
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
    match ramos::doc::generate_with(&stdlib_dir, &out_dir, &opts) {
        Ok(n) => {
            println!("generated docs for {n} module(s) in {}", out_dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `ramos doctest [dir]` — run the `# ==` examples in a directory's `@doc`
/// blocks.
///
/// The directory is a project root, so its modules are `DIR/src/*.rmo` — the
/// same shape `--stdlib` names and any Ramos project uses.
///
/// `--stdlib DIR` is what the examples load against, exactly as it is for `run`
/// and `check`: without it they load against the copy embedded in this binary.
/// Pointing it at the directory under test is how the stdlib documents itself
/// against the sources in front of it: `ramos doctest --stdlib stdlib`. A
/// project's *own* modules are reachable either way — they are copied in beside
/// each example.
fn run_doctest(args: &[String], stdlib: Option<String>, quietly: bool) -> ExitCode {
    let args: Vec<String> = args.to_vec();
    if args.len() > 1 {
        eprintln!("usage: ramos doctest [--quietly] [--stdlib DIR] [DIR]");
        eprintln!("  DIR            project root, modules read from DIR/src");
        eprintln!("                 (default: the --stdlib directory, else ./stdlib)");
        eprintln!("  --stdlib DIR   the stdlib the examples load against (default: embedded)");
        return ExitCode::from(2);
    }
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let stdlib_dir = stdlib.map(PathBuf::from);
    // Documenting the stdlib is the common case, so `--stdlib` alone names both
    // the sources under test and what they load against.
    let root = match (args.first(), &stdlib_dir) {
        (Some(s), _) => PathBuf::from(s),
        (None, Some(dir)) => dir.clone(),
        (None, None) => here.join("stdlib"),
    };
    let report = match ramos::doctest::run(&root, stdlib_dir.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut stdout = std::io::stdout();
    if let Err(e) = ramos::doctest::write_report(&report, &mut stdout, quietly) {
        eprintln!("error: could not write output: {e}");
        return ExitCode::FAILURE;
    }
    if report.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `ramos test [file]` — run the tests in one file, or in every test module
/// found under the current directory.
///
/// A file is a test file when it defines a module implementing `Test`. Each
/// file is loaded and run on its own interpreter, so one file's definitions and
/// state cannot leak into another's.
///
/// A run reads as a description of what the suite covers: the module's
/// `@module_doc` heads its section and each test's `@doc` follows its name, so
/// the report says what a test is for and not only that it ran. `--quietly`
/// leaves the docs out, for a run watched by a person who already knows them
/// (or by a CI log that does not need them).
fn run_tests(path: Option<&str>, stdlib: Option<String>, quietly: bool) -> ExitCode {
    let stdlib_dir = stdlib.map(PathBuf::from);
    let files: Vec<PathBuf> = match path {
        Some(p) => vec![PathBuf::from(p)],
        None => {
            let root = Path::new(TEST_ROOT);
            if !root.is_dir() {
                println!("no `{TEST_ROOT}` directory — tests live there");
                return ExitCode::SUCCESS;
            }
            match find_test_files(root) {
                Ok(found) if found.is_empty() => {
                    println!("no test modules found in `{TEST_ROOT}`");
                    return ExitCode::SUCCESS;
                }
                Ok(found) => found,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let color = ramos::color::Color::for_stdout();
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
            eprintln!("error: {e}");
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
                    eprintln!("error: {}: {e}", file.display());
                    return ExitCode::FAILURE;
                }
            };
        let mut current = String::new();
        for outcome in outcomes {
            if outcome.module != current {
                println!(
                    "{}",
                    color.paint(ramos::color::Style::Heading, &outcome.module)
                );
                if let Some(summary) = &docs.module {
                    println!("  {}", color.paint(ramos::color::Style::Dim, summary));
                }
                current = outcome.module.clone();
            }
            match &outcome.failure {
                None => {
                    passed += 1;
                    println!(
                        "  {} {}",
                        color.paint(ramos::color::Style::Str, "ok"),
                        outcome.name
                    );
                }
                Some(why) => {
                    failed += 1;
                    println!(
                        "  {} {}",
                        color.paint(ramos::color::Style::Keyword, "FAIL"),
                        outcome.name
                    );
                    println!("      {why}");
                }
            }
            // Under the result, so the name and its `ok`/`FAIL` still line up
            // down the column.
            if let Some(summary) = docs.functions.get(&outcome.name) {
                println!("     {}", color.paint(ramos::color::Style::Dim, summary));
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

/// Where tests live. A test module follows the same file rules as any other —
/// its namespace is its path — rooted here rather than at `src/`.
const TEST_ROOT: &str = "src/test";

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
    let empty = ramos::ast::Program { items: Vec::new() };
    let Ok(source) = std::fs::read_to_string(path) else {
        return empty;
    };
    let Ok(tokens) = ramos::lexer::lex(&source) else {
        return empty;
    };
    ramos::parser::parse(tokens).unwrap_or(empty)
}

/// `ramos run` / `ramos check` — load the entry file with the stdlib and every
/// module it reaches, then (for `run`) execute it.
fn run_loaded(cmd: &str, path: &str, stdlib: Option<String>, prog_args: &[String]) -> ExitCode {
    let stdlib_dir = stdlib.map(PathBuf::from);
    let program = match ramos::loader::load(Path::new(path), stdlib_dir.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            // Diagnostics arrive pre-rendered (and already end in a newline);
            // the loader's own messages are a single line.
            let text = e.to_string();
            if text.ends_with('\n') {
                eprint!("{text}");
            } else {
                eprintln!("{text}");
            }
            return ExitCode::FAILURE;
        }
    };
    if cmd == "check" {
        println!("{path}: ok ({} items loaded)", program.items.len());
        return ExitCode::SUCCESS;
    }
    // Line-buffered (the default `stdout()` behaviour), not block-buffered: a
    // `Thread.start`ed lambda writes to this same sink live, and its output must
    // reach the terminal as it happens rather than sit in an 8 KB buffer. The
    // sink is shared with those threads, so writes serialize through its mutex.
    let stdout = ramos::interp::sink(std::io::stdout());
    let result = ramos::interp::run_with_args(&program, stdout.clone(), prog_args);
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
            eprintln!("runtime error: {e}");
            ExitCode::FAILURE
        }
        (Ok(_), Err(e)) => {
            eprintln!("error: could not write output: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Everything runs on a thread with a large stack: the parser is recursive
/// descent and the stdlib recurses per element, so the default stack overflows
/// on ordinary input (a few hundred list elements). See `ramos::stack`.
fn main() -> ExitCode {
    ramos::stack::with_large_stack(run_cli)
}

fn run_cli() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let dump = take_flag(&mut args, "--dump");
    let quietly = take_flag(&mut args, "--quietly");
    let stdlib = take_opt(&mut args, "--stdlib");
    // Default to painting a terminal and staying plain when piped; the flags
    // are the override for a pager (`ramos ast --dump f.rmo --color | less -R`).
    let color = if take_flag(&mut args, "--no-color") {
        Color::Never
    } else if take_flag(&mut args, "--color") {
        Color::Always
    } else {
        Color::for_stdout()
    };
    // `doc` is a directory-level command (no `.rmo` file), so handle it before
    // the file-based dispatch below.
    if args.first().map(String::as_str) == Some("doc") {
        return run_doc(&args[1..]);
    }
    // `learn` takes no file — it prints a fixed crash course on the language.
    if args.first().map(String::as_str) == Some("learn") {
        print!("{}", ramos::learn::text());
        return ExitCode::SUCCESS;
    }
    // `repl` takes no file either — an interactive session on stdin.
    if args.first().map(String::as_str) == Some("repl") {
        return ramos::repl::run(stdlib);
    }
    // `doctest` takes an optional stdlib root, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("doctest") {
        return run_doctest(&args[1..], stdlib, quietly);
    }
    // `test` takes an optional file: with one, run that file's tests; without,
    // find every test module under the current directory.
    if args.first().map(String::as_str) == Some("test") {
        return run_tests(args.get(1).map(String::as_str), stdlib, quietly);
    }
    let (cmd, path) = match (args.first().map(String::as_str), args.get(1)) {
        (Some(c @ ("run" | "check" | "lexer" | "ast")), Some(p)) => (c, p.as_str()),
        _ => {
            eprintln!("usage: ramos <command> [args]");
            eprintln!();
            eprintln!(
                "  run <file.rmo>             execute a Ramos program (top-level statements)"
            );
            eprintln!(
                "  learn                      print a crash course on the language: every"
            );
            eprintln!(
                "                             keyword, the syntax, and what not to do"
            );
            eprintln!("  repl                       start an interactive session (persists state)");
            eprintln!("  test [--quietly] [file.rmo]");
            eprintln!(
                "                             run tests (all test modules found, or just one"
            );
            eprintln!(
                "                             file); --quietly drops the @doc lines from the report"
            );
            eprintln!("  doctest [--quietly] [--stdlib DIR] [DIR]");
            eprintln!(
                "                             run the `# ==` examples in DIR/src/*.rmo @doc blocks"
            );
            eprintln!(
                "                             (default: ./stdlib, against the embedded stdlib)"
            );
            eprintln!("  check <file.rmo>           verify the strict rules without running");
            eprintln!("  lexer [--dump] <file.rmo>  debug: print the token stream (--dump adds the raw code)");
            eprintln!(
                "  ast [--dump] <file.rmo>    debug: print the AST (--dump adds the raw code)"
            );
            eprintln!("  doc [--stdlib DIR] [--out DIR]");
            eprintln!(
                "                             generate HTML docs for the stdlib (Hexdocs-style)"
            );
            eprintln!();
            eprintln!("  --color / --no-color   force colour on or off (default: on for a");
            eprintln!("                         terminal; NO_COLOR is honoured)");
            return ExitCode::from(2);
        }
    };
    if !path.ends_with(".rmo") {
        eprintln!("error: Ramos source files must use the `.rmo` extension (got `{path}`)");
        let mut renamed = std::path::PathBuf::from(path);
        renamed.set_extension("rmo");
        eprintln!("  wrong:   ramos run {path}");
        eprintln!("  correct: ramos run {}", renamed.display());
        return ExitCode::FAILURE;
    }
    // `run` and `check` see a whole program — the stdlib plus every module the
    // entry file reaches. The debug commands stay on the single file in front
    // of them, which is the point of `lexer` and `ast`.
    if cmd == "run" || cmd == "check" {
        // Anything after the script path is a program argument, exposed to the
        // program via `get_args` / `get_arg`.
        let prog_args: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_loaded(cmd, path, stdlib, &prog_args);
    }
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read `{path}`: {e}");
            return ExitCode::FAILURE;
        }
    };
    match ramos::lexer::lex(&source) {
        Err(e) => {
            eprint!("{}", ramos::diagnostics::render(path, &source, &e));
            ExitCode::FAILURE
        }
        Ok(tokens) => match cmd {
            "lexer" => {
                if dump {
                    print!("{}", ramos::lexer::dump(&source, &tokens, color));
                } else {
                    print!("{}", ramos::lexer::render(&tokens, color));
                }
                ExitCode::SUCCESS
            }
            // `cmd` is "ast" here — run/check went through the loader above.
            _ => match ramos::parser::parse(tokens) {
                Err(e) => {
                    eprint!("{}", ramos::diagnostics::render_parse(path, &source, &e));
                    ExitCode::FAILURE
                }
                Ok(program) => {
                    if dump {
                        print!("{}", ramos::ast::dump(&source, &program, color));
                    } else {
                        print!("{}", ramos::ast::render(&program, color));
                    }
                    ExitCode::SUCCESS
                }
            },
        },
    }
}
