use ramos::color::{Color, Style};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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

/// The flags `ramos doc` and `ramos generate-docs` share (everything but
/// `--out`, whose default differs between the two — a temp directory for one,
/// `ramos-docs` for the other — and `--port`, which only `doc` takes).
/// Pulls its recognized flags out of `args`, leaving whatever is left for the
/// caller to reject as unknown.
fn generate_docs(args: &mut Vec<String>, out_dir: &Path) -> Result<usize, String> {
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

/// `ramos generate-docs` — render the stdlib reference into `ramos-docs/`
/// (or `--out DIR`) and exit. The write-to-disk half of what `ramos doc` used
/// to do on its own, for a CI step or anything else that wants the HTML
/// without a server sitting in front of it.
fn run_generate_docs(args: &[String], color: Color) -> ExitCode {
    let mut args: Vec<String> = args.to_vec();
    let out = take_opt(&mut args, "--out");
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let out_dir = match out {
        Some(s) => PathBuf::from(s),
        None => here.join("ramos-docs"),
    };
    let result = generate_docs(&mut args, &out_dir);
    if !args.is_empty() {
        eprintln!(
            "usage: ramos generate-docs [--stdlib DIR] [--out DIR] [--examples DIR] [--programs DIR] [--readme FILE]"
        );
        eprintln!("  --stdlib DIR   stdlib root, modules read from DIR/src (default: ./stdlib)");
        eprintln!("  --out DIR      where to write HTML (default: ./ramos-docs)");
        eprintln!(
            "  --examples DIR feature fixtures for the Examples page (default: ./tests/fixtures/features)"
        );
        eprintln!("  --programs DIR runnable programs for the Programs page (default: ./examples)");
        eprintln!("  --readme FILE  markdown for the guide page (default: ./README.md)");
        return ExitCode::from(2);
    }
    match result {
        Ok(n) => {
            println!("generated docs for {n} module(s) in {}", out_dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{} {e}", err_tag(color));
            ExitCode::FAILURE
        }
    }
}

/// `ramos doc` — generate a stdlib reference and serve it over HTTP on
/// `127.0.0.1:3030` (or `--port`), blocking until killed. Deliberately takes
/// nothing else: a quick preview needs no configuring, and it works from any
/// directory, not just a checkout of this project. It documents `.`'s own
/// `./stdlib` when that exists (plus `./tests/fixtures/features`,
/// `./examples` and `./README.md`, for developing *this* project's own docs)
/// and falls back to the stdlib embedded in the binary otherwise — the same
/// one `run`/`check`/the REPL already load without `--stdlib` — so `ramos
/// doc` run from an arbitrary directory still has the language's own
/// reference to show. Either way it writes into a temp directory it manages
/// itself. Reach for `ramos generate-docs` instead to point any of this
/// somewhere else, or to keep the output.
fn run_doc(args: &[String], color: Color) -> ExitCode {
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

/// A name reduced to its bare letters and digits, lowercased — `"HttpRequest"`
/// and `"http_request"` both become `"httprequest"`. `ramos see`'s one job:
/// match a module name written either way (the CamelCase it is called by in
/// Ramos source, or the snake_case file it lives in) against `loader::STDLIB`,
/// without needing a real CamelCase-to-snake_case algorithm — the loader's own
/// naming rule (a module's file is always its name in snake_case) already
/// guarantees the two normalize to the same thing.
fn normalize_module_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// `ramos see <module>` — print a stdlib module's Ramos source, verbatim.
///
/// `<module>` is matched against both the file stem (`http_request`) and the
/// CamelCase name it is called by in source (`HttpRequest`) via
/// `normalize_module_name`, so either spelling works.
///
/// `--stdlib DIR` reads `DIR/src/<module>.rmo` from disk instead of the copy
/// embedded in this binary — the same override `run`/`check`/`doctest` take,
/// for developing the stdlib itself.
fn run_see(args: &[String], stdlib: Option<String>, color: Color) -> ExitCode {
    let Some(name) = args.first() else {
        eprintln!("usage: ramos see [--stdlib DIR] <module>");
        return ExitCode::from(2);
    };
    let wanted = normalize_module_name(name);
    let Some((stem, embedded)) = ramos::loader::STDLIB
        .iter()
        .find(|(stem, _)| normalize_module_name(stem) == wanted)
    else {
        let available: Vec<&str> = ramos::loader::STDLIB.iter().map(|(stem, _)| *stem).collect();
        eprintln!("{} no stdlib module named `{name}`", err_tag(color));
        eprintln!("  available: {}", available.join(", "));
        return ExitCode::FAILURE;
    };
    match stdlib {
        None => {
            print!("{embedded}");
            ExitCode::SUCCESS
        }
        Some(dir) => {
            let path = Path::new(&dir).join("src").join(format!("{stem}.rmo"));
            match fs::read_to_string(&path) {
                Ok(source) => {
                    print!("{source}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{} cannot read `{}`: {e}", err_tag(color), path.display());
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Split a project name like `pet-project` or `pet_project` into lowercase
/// words, for building both the snake_case directory name and the CamelCase
/// module name a `ramos new` project starts with.
///
/// Empty when `name` has no valid words: each `-`/`_`-separated part must be
/// ASCII alphanumeric, and the first must start with a letter — a module name
/// cannot start with a digit.
fn project_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    for part in name.split(['-', '_']) {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Vec::new();
        }
        words.push(part.to_ascii_lowercase());
    }
    match words.first().and_then(|w| w.chars().next()) {
        Some(c) if c.is_ascii_alphabetic() => words,
        _ => Vec::new(),
    }
}

/// `pet` -> `Pet`. Used to turn a project name's words into its CamelCase
/// module name.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `ramos new <name>` — scaffold a new project: `<name>/src/<snake>/main.rmo`
/// defining `module <CamelCase>.Main` with a `function main()` that prints the
/// project name, plus `<name>/.env.dev`, a starter settings file for the
/// `Config` module's `dev` environment. The module lives where the naming
/// rule (see `loader`) says it must, and its file is named `main.rmo`, so
/// `ramos run <name>` finds it straight away.
fn run_new(args: &[String], color: Color) -> ExitCode {
    let (name, rest) = match args.split_first() {
        Some((name, rest)) => (name, rest),
        None => {
            eprintln!("usage: ramos new <project-name>");
            return ExitCode::from(2);
        }
    };
    if !rest.is_empty() {
        eprintln!("usage: ramos new <project-name>");
        return ExitCode::from(2);
    }
    let words = project_words(name);
    if words.is_empty() {
        eprintln!(
            "{} `{name}` is not a valid project name — use letters, digits, `-` or `_`, \
             starting with a letter",
            err_tag(color)
        );
        return ExitCode::from(2);
    }
    let snake_name = words.join("_");
    let module_name: String = words.iter().map(|w| capitalize(w)).collect();

    let root = PathBuf::from(name);
    if root.exists() {
        eprintln!("{} `{}` already exists", err_tag(color), root.display());
        return ExitCode::FAILURE;
    }
    let src_dir = root.join("src").join(&snake_name);
    if let Err(e) = fs::create_dir_all(&src_dir) {
        eprintln!(
            "{} cannot create `{}`: {e}",
            err_tag(color),
            src_dir.display()
        );
        return ExitCode::FAILURE;
    }
    let main_path = src_dir.join("main.rmo");
    let contents =
        format!("module {module_name}.Main\n  function main()\n    println(\"{name}\")\n");
    if let Err(e) = fs::write(&main_path, contents) {
        eprintln!(
            "{} cannot write `{}`: {e}",
            err_tag(color),
            main_path.display()
        );
        return ExitCode::FAILURE;
    }
    println!(
        "{} `{}`",
        color.paint(Style::Str, "created"),
        main_path.display()
    );

    // The `dev` environment's settings — `Config.start()` reads this when
    // `APP_ENV=dev`. It lives at the project root, not under `src/`, since
    // `Config` resolves it against the working directory a program runs from.
    let env_path = root.join(".env.dev");
    let env_contents = format!(
        "# {name} — the `dev` environment's settings, read by `Config.start()`\n\
         # when `APP_ENV=dev` (unset defaults to `.env` instead). See the `Config`\n\
         # module (`ramos doc`) for the file format.\n\
         #\n\
         # [section]\n\
         # key = \"value\"\n"
    );
    if let Err(e) = fs::write(&env_path, env_contents) {
        eprintln!(
            "{} cannot write `{}`: {e}",
            err_tag(color),
            env_path.display()
        );
        return ExitCode::FAILURE;
    }
    println!(
        "{} `{}`",
        color.paint(Style::Str, "created"),
        env_path.display()
    );
    ExitCode::SUCCESS
}

/// `ramos doctest [dir]` — run the `# ==` examples in a directory's `@doc`
/// blocks.
///
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
fn run_doctest(args: &[String], stdlib: Option<String>, quietly: bool, color: Color) -> ExitCode {
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

/// `ramos test [filter]` — run every test module under the nearest `src/test`
/// (walking up from the current directory), optionally narrowed to the files
/// whose name or path contains `filter`.
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
fn run_tests(
    filter: Option<&str>,
    stdlib: Option<String>,
    quietly: bool,
    color: Color,
) -> ExitCode {
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

/// Where tests live. A test module follows the same file rules as any other —
/// its namespace is its path — rooted here rather than at `src/`.
const TEST_ROOT: &str = "src/test";

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

/// The first `main.rmo` under `dir`, breadth-first — what `ramos run <dir>`
/// runs when it is pointed at a directory rather than a `.rmo` file. A
/// shallower `main.rmo` wins over a deeper one; among siblings, the
/// alphabetically first wins, so the search is deterministic even though "the
/// first one found" is otherwise a filesystem-order question.
fn find_main_file(dir: &Path) -> Option<PathBuf> {
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

/// Whether `program` defines a module exposing a public `function main()` —
/// the same definition of "entrypoint" `ramos::interp::run` itself uses.
fn has_entrypoint(program: &ramos::ast::Program) -> bool {
    program.items.iter().any(|item| match item {
        ramos::ast::Item::Module(m) => m.functions.iter().any(|f| f.name == "main" && !f.private),
        _ => false,
    })
}

/// `ramos run` / `ramos check` — load the entry file with the stdlib and every
/// module it reaches, then (for `run`) execute it.
///
/// `require_main` is set when `path` was picked by finding a directory's
/// `main.rmo` rather than named directly: that file is expected to *be* an
/// entrypoint, so — unlike a file named on the command line, which is free to
/// be a bare top-level script (see the README's "Entrypoints" section) — it
/// is an error for it to define no module exposing a public `function
/// main()`, rather than a silent fall-through to running its top-level
/// statements.
fn run_loaded(
    cmd: &str,
    path: &str,
    stdlib: Option<String>,
    prog_args: &[String],
    require_main: bool,
    color: Color,
) -> ExitCode {
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
        println!(
            "{path}: {} ({} items loaded)",
            color.paint(Style::Str, "ok"),
            program.items.len()
        );
        return ExitCode::SUCCESS;
    }
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

/// `ramos run -e CODE` — run `CODE` as a snippet, without a `.rmo` file on
/// disk. `CODE` is loaded exactly like a bare top-level script named on the
/// command line (see [`run_loaded`]'s doc comment): free to hold top-level
/// statements with no `module`, and not required to expose a `function
/// main()`. It can still reach a project's own modules under `./src`, since
/// [`ramos::loader::load_source`] roots the snippet at the current directory.
fn run_eval(code: &str, stdlib: Option<String>, prog_args: &[String], color: Color) -> ExitCode {
    let stdlib_dir = stdlib.map(PathBuf::from);
    let program = match ramos::loader::load_source(code, stdlib_dir.as_deref()) {
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
    run_program(&program, prog_args, color)
}

/// Everything runs on a thread with a large stack: the parser is recursive
/// descent and the stdlib recurses per element, so the default stack overflows
/// on ordinary input (a few hundred list elements). See `ramos::stack`.
fn main() -> ExitCode {
    ramos::stack::with_large_stack(run_cli)
}

/// The crate's own version, from `Cargo.toml` — what `ramos version` and bare
/// `ramos` print.
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn run_cli() -> ExitCode {
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
        return run_doc(&args[1..], color);
    }
    if args.first().map(String::as_str) == Some("generate-docs") {
        return run_generate_docs(&args[1..], color);
    }
    // `new` takes a project name, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("new") {
        return run_new(&args[1..], color);
    }
    // `learn` takes no file — it prints a fixed crash course on the language,
    // meant to be read as-is (or piped into an agent's context), so it is
    // never colored regardless of `--color`.
    if args.first().map(String::as_str) == Some("learn") {
        print!("{}", ramos::learn::text());
        return ExitCode::SUCCESS;
    }
    // `version` (and bare `ramos`, below) print the same line.
    if args.first().map(String::as_str) == Some("version") {
        println!("ramos {VERSION}");
        return ExitCode::SUCCESS;
    }
    // `run -e CODE` runs a snippet instead of a `.rmo` file, so it is handled
    // before the file-based `(cmd, path)` dispatch below, which expects a path
    // in `args[1]`.
    if args.first().map(String::as_str) == Some("run") {
        if let Some(code) = &eval {
            let prog_args: Vec<String> = args.iter().skip(1).cloned().collect();
            return run_eval(code, stdlib, &prog_args, color);
        }
    }
    // `repl` takes no file either — an interactive session on stdin.
    if args.first().map(String::as_str) == Some("repl") {
        return ramos::repl::run(stdlib, color);
    }
    // `doctest` takes an optional stdlib root, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("doctest") {
        return run_doctest(&args[1..], stdlib, quietly, color);
    }
    // `see` takes a module name, not a `.rmo` file.
    if args.first().map(String::as_str) == Some("see") {
        return run_see(&args[1..], stdlib, color);
    }
    // `test` takes an optional file: with one, run that file's tests; without,
    // find every test module under the current directory.
    if args.first().map(String::as_str) == Some("test") {
        return run_tests(args.get(1).map(String::as_str), stdlib, quietly, color);
    }
    let (cmd, path) = match (args.first().map(String::as_str), args.get(1)) {
        (Some(c @ ("run" | "check" | "lexer" | "ast")), Some(p)) => (c, p.as_str()),
        // Bare `ramos run` is `ramos run .` — the directory-resolution below
        // finds `.`'s `main.rmo` exactly as it would for any named directory.
        (Some("run"), None) => ("run", "."),
        _ => {
            eprintln!("ramos {VERSION}");
            eprintln!("usage: ramos <command> [args]");
            eprintln!();
            eprintln!(
                "  run <file.rmo>             execute a Ramos program (top-level statements)"
            );
            eprintln!(
                "  run <dir>                  run that directory's `main.rmo` (the shallowest"
            );
            eprintln!("                             one found, if there is more than one)");
            eprintln!("  run                        same as `run .` — run the current directory's");
            eprintln!("                             `main.rmo`");
            eprintln!(
                "  run -e CODE                run CODE as a snippet, no `.rmo` file needed"
            );
            eprintln!(
                "  new <project-name>         scaffold a project: <name>/src/<snake>/main.rmo"
            );
            eprintln!(
                "                             defining `<CamelCase>.Main`, plus a `.env.dev`"
            );
            eprintln!("                             Config starter");
            eprintln!("  learn                      print a crash course on the language: every");
            eprintln!("                             keyword, the syntax, and what not to do");
            eprintln!("  repl                       start an interactive session (persists state)");
            eprintln!("  version                    print the version");
            eprintln!("  test [--quietly] [filter]");
            eprintln!("                             run every test under the nearest `src/test`");
            eprintln!(
                "                             (walking up from `.`), or just the files whose"
            );
            eprintln!("                             name or path contains filter; --quietly drops");
            eprintln!("                             the @doc lines from the report");
            eprintln!("  doctest [--quietly] [--stdlib DIR] [DIR]");
            eprintln!(
                "                             run the `# ==` examples in DIR/src/*.rmo @doc blocks"
            );
            eprintln!(
                "                             (default: DIR is `.`, against the embedded stdlib;"
            );
            eprintln!(
                "                             `ramos doctest --stdlib stdlib` documents the stdlib)"
            );
            eprintln!("  check <file.rmo>           verify the strict rules without running");
            eprintln!("  lexer [--dump] <file.rmo>  debug: print the token stream (--dump adds the raw code)");
            eprintln!(
                "  ast [--dump] <file.rmo>    debug: print the AST (--dump adds the raw code)"
            );
            eprintln!("  doc [--port PORT]");
            eprintln!(
                "                             generate HTML docs for the stdlib (Hexdocs-style)"
            );
            eprintln!(
                "                             and serve them at http://localhost:3030 (or --port)"
            );
            eprintln!("  generate-docs [--stdlib DIR] [--out DIR]");
            eprintln!(
                "                             generate HTML docs into ./ramos-docs (or --out) and exit"
            );
            eprintln!("  see [--stdlib DIR] <module>");
            eprintln!(
                "                             print a stdlib module's source (e.g. `HttpRequest`"
            );
            eprintln!("                             or `http_request`, either spelling works)");
            eprintln!();
            eprintln!("  --color / --no-color   force colour on or off (default: on for a");
            eprintln!("                         terminal; NO_COLOR is honoured)");
            return ExitCode::from(2);
        }
    };
    // `ramos run <dir>` runs that directory's `main.rmo` instead of naming the
    // entrypoint file directly. Since the file was picked by the `main.rmo`
    // name rather than named by the caller, it is held to what that name
    // promises: an entrypoint, not just any runnable script.
    let resolved;
    let mut require_main = false;
    let path = if cmd == "run" && Path::new(path).is_dir() {
        match find_main_file(Path::new(path)) {
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
    if cmd == "run" || cmd == "check" {
        // Anything after the script path is a program argument, exposed to the
        // program via `get_args` / `get_arg`.
        let prog_args: Vec<String> = args.iter().skip(2).cloned().collect();
        return run_loaded(cmd, path, stdlib, &prog_args, require_main, color);
    }
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} cannot read `{path}`: {e}", err_tag(color));
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
