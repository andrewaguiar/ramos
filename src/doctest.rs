//! The doctest runner (PLAN phase 8): the `# ==` lines in `@doc` blocks, run as
//! tests.
//!
//! The stdlib documents every function with runnable examples:
//!
//! ```text
//!     #   List.map([1, 2, 3], do x -> x * 2)   # == [2, 4, 6]
//! ```
//!
//! Each of those is a claim the interpreter can check, so `ramos doctest` reads
//! them back out and runs them. A doc that drifts from its implementation fails
//! the build rather than misleading a reader.
//!
//! Every example runs as its own program, in its own empty directory. The
//! sandbox is what lets `File` and `Dir` document themselves: an example may
//! write, read back, and assert, without touching the tree it was run from and
//! without seeing what the example before it left behind.

use crate::interp::{run_with_args, sink, Value};
use crate::loader::load;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One assertion lifted out of a `@doc` block: the program to run, the value it
/// claims, and the argv the surrounding snippet asked for.
#[derive(Debug, Clone)]
pub struct Example {
    /// The file stem the example came from — `list` for `list.rmo`.
    pub module: String,
    /// The 1-based line the asserted expression starts on.
    pub line: usize,
    /// Statements that set the assertion up (bindings from earlier in the same
    /// snippet), already joined.
    pub setup: Vec<String>,
    pub expr: String,
    pub expected: String,
    pub argv: Vec<String>,
}

/// One example that did not produce what it claimed.
#[derive(Debug, Clone)]
pub struct Failure {
    pub module: String,
    pub line: usize,
    pub expr: String,
    pub expected: String,
    /// What happened instead — a value, or the error that stopped it.
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.rmo:{}: {}  # == {}  -> {}",
            self.module, self.line, self.expr, self.expected, self.detail
        )
    }
}

/// What a run of the whole directory came to.
#[derive(Debug, Default)]
pub struct Report {
    pub passed: usize,
    pub failures: Vec<Failure>,
    /// The modules that carried at least one example, in the order run.
    pub modules: Vec<(String, usize)>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.passed + self.failures.len()
    }
}

/// Extract the runnable examples from a Ramos source.
///
/// A `@doc` code line is a comment indented past the `#`. Consecutive code
/// lines form one *snippet* and share a scope, so a binding on an earlier line
/// is in scope later. Within a snippet a line beginning with `|` continues the
/// previous one — a pipeline may assert its value at each stage, so the same
/// pipeline yields one example per `# ==` it carries.
///
/// A `# ramos run app.rmo alice 42` line sets the argv the snippet assumes.
///
/// A snippet headed by `# ramos doctest setup` is the file's *preamble*: it is
/// never asserted itself, and it runs before every example in the file. That is
/// what lets `Struct` document itself against a `Person` declared once, instead
/// of redeclaring it in eight `@doc` blocks. It has to appear before the
/// examples that need it, which in practice means the `@module_doc`.
pub fn examples_in(module: &str, source: &str) -> Vec<Example> {
    let mut out = Vec::new();
    let mut preamble: Vec<String> = Vec::new();
    let mut in_preamble = false;
    let mut setup: Vec<String> = Vec::new();
    let mut current: Option<(usize, String)> = None; // (line, joined statement)
    let mut base_indent = 0usize; // indentation of the current logical line
    let mut argv: Vec<String> = Vec::new();

    // Finish the pending logical line, keeping it as setup for what follows.
    let flush = |current: &mut Option<(usize, String)>, setup: &mut Vec<String>| {
        if let Some((_, text)) = current.take() {
            setup.push(text);
        }
    };

    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        let Some(body) = line.strip_prefix('#') else {
            continue;
        };
        // A code line is indented past the `#`; anything else is prose and ends
        // the snippet.
        if !body.starts_with("   ") {
            flush(&mut current, &mut setup);
            if in_preamble {
                // The preamble ends where its snippet does; keep the lines.
                preamble.append(&mut setup);
                in_preamble = false;
            }
            setup.clear();
            argv.clear();
            continue;
        }
        // Strip exactly the `#   ` marker, keeping any indentation beyond it —
        // an example may be a multi-line `cond` or `case`, and Ramos is
        // indentation-sensitive.
        let code_line = body[3..].trim_end();
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        // `# ramos doctest setup` — this snippet sets up every example below.
        if body == "# ramos doctest setup" {
            flush(&mut current, &mut setup);
            setup.clear();
            in_preamble = true;
            continue;
        }
        // `# ramos run app.rmo alice 42` — the invocation the snippet assumes.
        if let Some(invocation) = body.strip_prefix("# ramos run ") {
            argv = invocation
                .split_whitespace()
                .skip(1)
                .map(String::from)
                .collect();
            continue;
        }
        // An expectation may sit on its own line, asserting the line above it.
        if let Some(expected) = body.strip_prefix("# ==") {
            if let Some((line, text)) = &current {
                if !in_preamble {
                    push_example(
                        &mut out, module, *line, &preamble, &setup, text, expected, &argv,
                    );
                }
            }
            continue;
        }
        if body.starts_with('#') {
            continue; // a comment inside the example
        }
        // Keep the indentation on the code itself; decide on the trimmed form.
        let (code, expected) = match code_line.split_once("# ==") {
            Some((code, expected)) => (code.trim_end(), Some(expected)),
            None => (code_line, None),
        };
        let trimmed = code.trim_start();
        let indent = code.len() - trimmed.len();
        // A logical line continues onto a leading `|` (a pipeline stage) or
        // onto anything indented further than it (a `cond`/`case` body).
        let continues = match &current {
            Some((_, _)) => trimmed.starts_with('|') || indent > base_indent,
            None => false,
        };
        if continues {
            if let Some((_, text)) = &mut current {
                text.push('\n');
                text.push_str(code);
            }
        } else {
            flush(&mut current, &mut setup);
            base_indent = indent;
            current = Some((i + 1, code.to_string()));
        }
        if let Some(expected) = expected {
            if let Some((line, text)) = &current {
                if !in_preamble {
                    push_example(
                        &mut out, module, *line, &preamble, &setup, text, expected, &argv,
                    );
                }
            }
        }
    }
    out
}

/// Record one `# ==` claim, unless it is empty (`# ==` with nothing after it).
#[allow(clippy::too_many_arguments)]
fn push_example(
    out: &mut Vec<Example>,
    module: &str,
    line: usize,
    preamble: &[String],
    setup: &[String],
    expr: &str,
    expected: &str,
    argv: &[String],
) {
    let expected = expected.trim();
    if expected.is_empty() {
        return;
    }
    let mut lines = preamble.to_vec();
    lines.extend_from_slice(setup);
    out.push(Example {
        module: module.to_string(),
        line,
        setup: lines,
        expr: expr.to_string(),
        expected: expected.to_string(),
        argv: argv.to_vec(),
    });
}

/// The `.rmo` sources of a stdlib-shaped directory: `<dir>/src/*.rmo`, sorted,
/// as (file stem, source). Subdirectories are not walked, so a module's own
/// `src/test/` is left to `ramos test`.
pub fn sources(source_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let dir = source_dir.join("src");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rmo"))
        .collect();
    entries.sort();
    let mut out = Vec::new();
    for path in entries {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
        out.push((stem, source));
    }
    Ok(out)
}

/// Run every example in `source_dir`, reporting what did not hold.
///
/// `stdlib_dir` is the stdlib each example loads against — normally
/// `source_dir` itself, so the sources under test are the ones documenting
/// themselves.
pub fn run(source_dir: &Path, stdlib_dir: Option<&Path>) -> Result<Report, String> {
    // Every example runs from its own directory, so the paths this function was
    // handed have to survive the move: resolve them before the first `cd`.
    let source_dir = absolute(source_dir)?;
    let stdlib_dir = match stdlib_dir {
        Some(d) => Some(absolute(d)?),
        None => None,
    };
    let home =
        std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;

    let mut report = Report::default();
    // The project's own modules are copied in beside the examples, so an
    // example may call what its file documents: the loader searches an entry's
    // directory and every ancestor's `src/`, and the sandbox now has one.
    let sandbox = Sandbox::new(&source_dir)?;
    for (module, source) in sources(&source_dir)? {
        let examples = examples_in(&module, &source);
        if examples.is_empty() {
            continue;
        }
        report.modules.push((module.clone(), examples.len()));
        for example in &examples {
            match run_one(example, &sandbox, stdlib_dir.as_deref(), &home) {
                Ok(()) => report.passed += 1,
                Err(detail) => report.failures.push(Failure {
                    module: example.module.clone(),
                    line: example.line,
                    expr: example.expr.clone(),
                    expected: example.expected.clone(),
                    detail,
                }),
            }
        }
    }
    Ok(report)
}

/// Run one example in a fresh directory, returning what went wrong.
fn run_one(
    example: &Example,
    sandbox: &Sandbox,
    stdlib_dir: Option<&Path>,
    home: &Path,
) -> Result<(), String> {
    // Each example is its own program: its setup, then `expr == expected`.
    let mut src = String::new();
    for line in &example.setup {
        src.push_str(line);
        src.push('\n');
    }
    // Bind first: the expression may span lines (a pipeline continues onto `|`
    // lines), which reads as one statement but not inside parentheses.
    src.push_str(&format!(
        "actual_value =\n  {}\nactual_value == ({})\n",
        indented(&example.expr),
        example.expected
    ));

    let dir = sandbox.fresh()?;
    let entry = dir.join("doctest.rmo");
    fs::write(&entry, &src).map_err(|e| format!("cannot write the example: {e}"))?;

    // The example may name relative paths (`File.write("out.txt", …)`), so it
    // runs *inside* its own directory and the process returns home afterwards.
    // Nothing here is parallel, which is what makes a process-wide cd safe.
    std::env::set_current_dir(&dir).map_err(|e| format!("cannot enter the sandbox: {e}"))?;
    let outcome = load_and_run(&entry, stdlib_dir, &example.argv);
    std::env::set_current_dir(home).map_err(|e| format!("cannot leave the sandbox: {e}"))?;

    match outcome {
        Ok(Value::Bool(true)) => Ok(()),
        // The comparison itself came back false, so re-run the expression alone
        // to report what it actually produced rather than a bare `false`.
        Ok(_) => Err(actual_value(example, &dir, stdlib_dir, home)),
        Err(e) => Err(e),
    }
}

/// Evaluate just the example's expression, to say what it produced. Falls back
/// to `false` — what the comparison returned — if even that cannot be run.
fn actual_value(example: &Example, dir: &Path, stdlib_dir: Option<&Path>, home: &Path) -> String {
    let mut src = String::new();
    for line in &example.setup {
        src.push_str(line);
        src.push('\n');
    }
    src.push_str(&example.expr);
    src.push('\n');
    let entry = dir.join("doctest_actual.rmo");
    if fs::write(&entry, &src).is_err() {
        return "false".to_string();
    }
    if std::env::set_current_dir(dir).is_err() {
        return "false".to_string();
    }
    let outcome = load_and_run(&entry, stdlib_dir, &example.argv);
    let _ = std::env::set_current_dir(home);
    match outcome {
        Ok(value) => format!("got {}", value.inspect()),
        Err(e) => e,
    }
}

fn load_and_run(entry: &Path, stdlib_dir: Option<&Path>, argv: &[String]) -> Result<Value, String> {
    let program = load(entry, stdlib_dir).map_err(|e| e.to_string().trim_end().to_string())?;
    // A doctest checks the value; its output is discarded.
    run_with_args(&program, sink(Vec::new()), argv).map_err(|e| e.message)
}

/// Indent an expression's continuation lines to sit under the `=` of the
/// binding it becomes, since an assigned block starts on its own line.
fn indented(expr: &str) -> String {
    let mut lines = expr.lines();
    let first = lines.next().unwrap_or_default().to_string();
    let rest: Vec<String> = lines.map(|l| format!("  {l}")).collect();
    if rest.is_empty() {
        first
    } else {
        format!("{first}\n{}", rest.join("\n"))
    }
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| format!("cannot resolve `{}`: {e}", path.display()))
}

/// A directory of per-example sandboxes, removed when the run ends.
struct Sandbox {
    root: PathBuf,
    next: std::cell::Cell<usize>,
}

impl Sandbox {
    fn new(source_dir: &Path) -> Result<Sandbox, String> {
        let root = std::env::temp_dir().join(format!("ramos-doctest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        fs::create_dir_all(&src).map_err(|e| format!("cannot create `{}`: {e}", src.display()))?;
        // Copied rather than referenced: an example runs from its own directory,
        // and the loader looks for modules relative to that.
        for (stem, source) in sources(source_dir)? {
            let path = src.join(format!("{stem}.rmo"));
            fs::write(&path, source)
                .map_err(|e| format!("cannot write `{}`: {e}", path.display()))?;
        }
        Ok(Sandbox {
            root,
            next: std::cell::Cell::new(0),
        })
    }

    /// An empty directory no example has used yet, one level under the root so
    /// the project's `src/` is on the loader's search path. Examples are
    /// sandboxed from each other as well as from the tree:
    /// `File.exists("out.txt")` must be answered by this example's own setup,
    /// never by the one before it.
    fn fresh(&self) -> Result<PathBuf, String> {
        let n = self.next.get();
        self.next.set(n + 1);
        let dir = self.root.join(n.to_string());
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create `{}`: {e}", dir.display()))?;
        Ok(dir)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Write a report the way `ramos test` writes its own.
pub fn write_report(report: &Report, out: &mut impl io::Write, quietly: bool) -> io::Result<()> {
    if !quietly {
        for (module, count) in &report.modules {
            writeln!(out, "{module}.rmo: {count} example(s)")?;
        }
        if !report.modules.is_empty() {
            writeln!(out)?;
        }
    }
    for failure in &report.failures {
        writeln!(out, "{failure}")?;
    }
    if !report.failures.is_empty() {
        writeln!(out)?;
    }
    writeln!(
        out,
        "{} example(s), {} passed, {} failed",
        report.total(),
        report.passed,
        report.failures.len()
    )
}
