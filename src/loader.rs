//! The module loader (PLAN phase 5): turns an entry file into one `Program`
//! holding the stdlib, the entry file, and every module it reaches.
//!
//! The strict file rules:
//!
//! * `.rmo` extension — no other extension is loaded.
//! * One definition per file — a file holds exactly one module, trait or struct.
//! * The file name matches the definition — `SystemUser` lives in
//!   `system_user.rmo`, and `MyApp.Business.SystemUser` at
//!   `src/my_app/business/system_user.rmo`.
//!
//! The naming rule is what lets a namespace be turned into a path, so it binds
//! every file the loader *resolves*. The entry file is named on the command
//! line and resolved from nothing, so it is held to the one-definition rule but
//! free to be called anything — which is what makes a scratch file work.
//!
//! The stdlib is embedded with `include_str!`, so the binary is self-contained;
//! `--stdlib DIR` reads it from disk instead, which is how the stdlib itself is
//! developed. `DIR` is the stdlib root: its modules are `DIR/src/*.rmo` and
//! its own tests `DIR/src/test/*_test.rmo`, exactly like any Ramos project.
//!
//! Cross-module references resolve on demand: the loader walks what it has
//! parsed for module paths, loads the ones it does not know yet, and repeats.
//! A path that names no file is left to the interpreter, which reports
//! `undefined module` at the point of use — a single-segment name may well be
//! an `alias` local name or one of the type modules, neither of which is a file
//! reference.

use crate::ast::*;
use crate::parser::parse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The embedded standard library, as (file stem, source).
///
/// `pub` so other code that needs every module's source without a filesystem
/// round-trip — `ramos learn`'s stdlib index, and the test that checks it
/// stays in sync — can walk the same list this loader embeds, rather than
/// keeping a second one that could drift from it.
pub const STDLIB: &[(&str, &str)] = &[
    ("kernel", include_str!("../stdlib/src/kernel.rmo")),
    ("integer", include_str!("../stdlib/src/integer.rmo")),
    ("float", include_str!("../stdlib/src/float.rmo")),
    ("string", include_str!("../stdlib/src/string.rmo")),
    ("list", include_str!("../stdlib/src/list.rmo")),
    ("map", include_str!("../stdlib/src/map.rmo")),
    ("tuple", include_str!("../stdlib/src/tuple.rmo")),
    ("struct", include_str!("../stdlib/src/struct.rmo")),
    ("json", include_str!("../stdlib/src/json.rmo")),
    ("date", include_str!("../stdlib/src/date.rmo")),
    ("time", include_str!("../stdlib/src/time.rmo")),
    (
        "naive_date_time",
        include_str!("../stdlib/src/naive_date_time.rmo"),
    ),
    ("time_zone", include_str!("../stdlib/src/time_zone.rmo")),
    ("date_time", include_str!("../stdlib/src/date_time.rmo")),
    ("file", include_str!("../stdlib/src/file.rmo")),
    ("dir", include_str!("../stdlib/src/dir.rmo")),
    ("socket", include_str!("../stdlib/src/socket.rmo")),
    (
        "server_socket",
        include_str!("../stdlib/src/server_socket.rmo"),
    ),
    ("http_server", include_str!("../stdlib/src/http_server.rmo")),
    (
        "http_request",
        include_str!("../stdlib/src/http_request.rmo"),
    ),
    ("actor", include_str!("../stdlib/src/actor.rmo")),
    ("global", include_str!("../stdlib/src/global.rmo")),
    ("pool", include_str!("../stdlib/src/pool.rmo")),
    ("config", include_str!("../stdlib/src/config.rmo")),
    ("thread", include_str!("../stdlib/src/thread.rmo")),
    ("test", include_str!("../stdlib/src/test.rmo")),
    ("module", include_str!("../stdlib/src/module.rmo")),
];

/// A load failure, already rendered for display. Lex and parse problems arrive
/// as full diagnostics (file, line, column, source excerpt); the loader's own
/// rule violations as a single line.
#[derive(Debug)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The standard library on its own, as a program of definitions. The REPL
/// registers this before its first entry, so `String.split(...)` works at the
/// prompt exactly as it does in a file.
///
/// `stdlib_dir` overrides the embedded stdlib with sources read from disk.
pub fn stdlib(stdlib_dir: Option<&Path>) -> Result<Program, LoadError> {
    let mut loaded = Loaded::default();
    loaded.add_stdlib(stdlib_dir)?;
    Ok(Program {
        items: loaded.items,
        entry_file: Arc::from(""),
    })
}

/// Load `entry` together with the stdlib and every module either reaches,
/// merged into a single program ready for the interpreter.
///
/// `stdlib_dir` overrides the embedded stdlib with sources read from disk.
pub fn load(entry: &Path, stdlib_dir: Option<&Path>) -> Result<Program, LoadError> {
    let source = read(entry)?;
    load_from_source(entry, &source, stdlib_dir)
}

/// Load `source` as if it were the entry file's text, without reading it from
/// disk — what `ramos run -e CODE` runs. The virtual entry sits in the current
/// directory (`./<eval>`), so a snippet can still reach a project's own
/// modules under `./src` exactly as a real file there would, via the same
/// [`search_roots`] rule; it just skips the file-name checks a real file is
/// held to.
///
/// `stdlib_dir` overrides the embedded stdlib with sources read from disk.
pub fn load_source(source: &str, stdlib_dir: Option<&Path>) -> Result<Program, LoadError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let entry = cwd.join("<eval>");
    load_from_source(&entry, source, stdlib_dir)
}

/// The shared body of [`load`] and [`load_source`]: parse `source` as `entry`,
/// then resolve every module it reaches against the stdlib and `entry`'s
/// search roots.
fn load_from_source(
    entry: &Path,
    source: &str,
    stdlib_dir: Option<&Path>,
) -> Result<Program, LoadError> {
    let mut loaded = Loaded::default();
    loaded.add_stdlib(stdlib_dir)?;

    // The entry file is named on the command line, so nothing has to *find* it
    // by namespace — which is what the file-name rule exists for. It is held to
    // the one-definition rule, but not to the naming one, so a scratch file or
    // a demo may be called whatever its author likes.
    let mut program = parse_source(entry, source)?;
    check_one_definition(entry, &program)?;
    // Roots depend on the entry's own namespace, so read them before the items
    // are moved into the bundle.
    let roots = search_roots(entry, &program);
    stamp_file(&mut program.items, entry);
    loaded.items.extend(program.items);

    // Resolve what the program refers to, then what *those* modules refer to,
    // until nothing new appears.
    loaded.resolve_references(&roots)?;
    Ok(Program {
        items: loaded.items,
        entry_file: entry.display().to_string().into(),
    })
}

/// Everything parsed so far, plus the names already accounted for so a module
/// is never loaded (or searched for) twice.
#[derive(Default)]
struct Loaded {
    items: Vec<Item>,
    defined: HashMap<String, ()>,
    /// Paths already searched for and not found, so a miss costs one look.
    missing: HashMap<String, ()>,
}

impl Loaded {
    /// The stdlib, from disk when a directory is given and from the binary
    /// otherwise.
    fn add_stdlib(&mut self, dir: Option<&Path>) -> Result<(), LoadError> {
        match dir {
            Some(dir) => self.add_stdlib_from_disk(dir),
            None => self.add_embedded_stdlib(),
        }
    }

    fn add_embedded_stdlib(&mut self) -> Result<(), LoadError> {
        for (stem, source) in STDLIB {
            let shown = PathBuf::from(format!("stdlib/src/{stem}.rmo"));
            let program = parse_source(&shown, source)?;
            self.add_file(&shown, program)?;
        }
        Ok(())
    }

    fn add_stdlib_from_disk(&mut self, dir: &Path) -> Result<(), LoadError> {
        // `dir` is the stdlib *root*; its modules live under `src/`, the same
        // layout any Ramos project uses.
        for (stem, _) in STDLIB {
            let path = dir.join("src").join(format!("{stem}.rmo"));
            let source = read(&path)?;
            let program = parse_source(&path, &source)?;
            self.add_file(&path, program)?;
        }
        Ok(())
    }

    /// Add a file that must hold exactly one definition, named after the file.
    fn add_file(&mut self, path: &Path, mut program: Program) -> Result<(), LoadError> {
        check_one_definition(path, &program)?;
        let Some(name) = definition_name(&program) else {
            return Err(LoadError(format!(
                "error: `{}` defines no module, trait or struct",
                path.display()
            )));
        };
        check_name_matches_file(path, name)?;
        self.defined.insert(name.to_string(), ());
        stamp_file(&mut program.items, path);
        self.items.extend(program.items);
        Ok(())
    }

    /// Load every module the current items refer to and that is not defined
    /// yet, repeating until the set stops growing.
    fn resolve_references(&mut self, roots: &[PathBuf]) -> Result<(), LoadError> {
        loop {
            // Definitions present in the items but not registered (the entry
            // file's own modules) still count as defined.
            for item in &self.items {
                if let Some(name) = item_definition_name(item) {
                    self.defined.insert(name.to_string(), ());
                }
            }
            let wanted: Vec<ModulePath> = collect_module_refs(&self.items)
                .into_iter()
                .filter(|p| {
                    let name = p.to_string();
                    !self.defined.contains_key(&name) && !self.missing.contains_key(&name)
                })
                .collect();
            if wanted.is_empty() {
                return Ok(());
            }
            for path in wanted {
                self.load_module(&path, roots)?;
            }
        }
    }

    /// Find and add the file defining `path`, if there is one.
    ///
    /// Not finding it is only an error for a namespaced (multi-segment) path,
    /// which can mean nothing but a file. A bare `Circle` may just as well be
    /// an `alias` local name or a type module, so a miss there is recorded and
    /// left for the interpreter to report if it really is used.
    fn load_module(&mut self, path: &ModulePath, roots: &[PathBuf]) -> Result<(), LoadError> {
        let relative = module_path_to_file(path);
        let candidates: Vec<PathBuf> = roots.iter().map(|r| r.join(&relative)).collect();
        let Some(found) = candidates.iter().find(|c| c.is_file()) else {
            if path.0.len() > 1 {
                let tried: Vec<String> = candidates
                    .iter()
                    .map(|c| format!("  {}", c.display()))
                    .collect();
                return Err(LoadError(format!(
                    "error: cannot find module `{path}`; tried:\n{}",
                    tried.join("\n")
                )));
            }
            self.missing.insert(path.to_string(), ());
            return Ok(());
        };
        let source = read(found)?;
        let program = parse_source(found, &source)?;
        self.add_file(found, program)?;
        // `add_file` records the module under its own canonical declared name,
        // which may differ from the bare aliased name we were asked for here
        // (e.g. requested `WithdrawalError`, declared `Accounts.WithdrawalError`).
        // Record the requested name too, or the next pass sees it as still
        // missing and reloads the same file forever.
        self.defined.insert(path.to_string(), ());
        Ok(())
    }
}

/// Where a user module may live: beside the entry file, and under its `src/`
/// (which is where a namespaced project keeps them).
fn search_roots(entry: &Path, program: &Program) -> Vec<PathBuf> {
    let dir = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut roots = vec![dir.clone()];
    // A namespaced entry sits *inside* the tree its own name describes:
    // `MyApp.User.TestUser` lives at `<root>/my_app/user/test_user.rmo`. Walk
    // back up one directory per namespace segment to find `<root>`, so the
    // entry can reach its siblings by their full names.
    if let Some(name) = definition_name(program) {
        let mut root = dir.clone();
        let mut climbed = true;
        for _ in 1..name.0.len() {
            match root.parent() {
                Some(parent) => root = parent.to_path_buf(),
                None => {
                    climbed = false;
                    break;
                }
            }
        }
        if climbed && root != dir {
            roots.push(root);
        }
    }
    // A test lives under `src/test/`, but the code it exercises lives under
    // `src/`. Offer every ancestor's `src` as a root so a test can reach it.
    let mut ancestor = Some(dir.as_path());
    while let Some(base) = ancestor {
        let src = base.join("src");
        if src.is_dir() && !roots.contains(&src) {
            roots.push(src);
        }
        ancestor = base.parent();
    }
    roots
}

fn read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path)
        .map_err(|e| LoadError(format!("error: cannot read `{}`: {e}", path.display())))
}

fn parse_source(path: &Path, source: &str) -> Result<Program, LoadError> {
    let shown = path.display().to_string();
    let tokens = crate::lexer::lex(source)
        .map_err(|e| LoadError(crate::diagnostics::render(&shown, source, &e)))?;
    parse(tokens).map_err(|e| LoadError(crate::diagnostics::render_parse(&shown, source, &e)))
}

/// Record `path` as the file every function in `items` was defined in — what a
/// stacktrace frame names for a call made from inside one of them, or from a
/// lambda one of them creates (a lambda inherits the file of whatever
/// function is running when it is built; see `Interp::current_file`).
fn stamp_file(items: &mut [Item], path: &Path) {
    let file: Arc<str> = path.display().to_string().into();
    for item in items {
        match item {
            Item::Module(m) => {
                for f in &mut m.functions {
                    f.file = file.clone();
                }
            }
            Item::Struct(s) => {
                for f in &mut s.functions {
                    f.file = file.clone();
                }
            }
            Item::Trait(t) => {
                for f in &mut t.functions {
                    f.file = file.clone();
                }
            }
            Item::Function(f) => f.file = file.clone(),
            Item::Statement(_) => {}
        }
    }
}

/// The name of a program's single definition, if it has one.
fn definition_name(program: &Program) -> Option<&ModulePath> {
    program.items.iter().find_map(item_definition_name)
}

fn item_definition_name(item: &Item) -> Option<&ModulePath> {
    match item {
        Item::Module(m) => Some(&m.name),
        Item::Struct(s) => Some(&s.name),
        Item::Trait(t) => Some(&t.name),
        Item::Function(_) | Item::Statement(_) => None,
    }
}

fn check_one_definition(path: &Path, program: &Program) -> Result<(), LoadError> {
    let names: Vec<&ModulePath> = program
        .items
        .iter()
        .filter_map(item_definition_name)
        .collect();
    if names.len() > 1 {
        let list: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        return Err(LoadError(format!(
            "error: `{}` holds {} definitions ({}) — a file holds exactly one \
             module, trait or struct\n\
             \x20 wrong:   account.rmo holding both `module Account` and `module Ledger`\n\
             \x20 correct: `module Account` in account.rmo, `module Ledger` in ledger.rmo",
            path.display(),
            names.len(),
            list.join(", "),
        )));
    }
    Ok(())
}

fn check_name_matches_file(path: &Path, name: &ModulePath) -> Result<(), LoadError> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let last = name
        .0
        .last()
        .expect("a module path has at least one segment");
    let expected = camel_to_snake(last);
    if stem != expected {
        return Err(LoadError(format!(
            "error: `{}` defines `{name}`, which belongs in `{expected}.rmo` \
             — the file name must match the definition\n\
             \x20 wrong:   `module {name}` in {}\n\
             \x20 correct: `module {name}` in {expected}.rmo",
            path.display(),
            path.display(),
        )));
    }
    Ok(())
}

/// The file a module belongs in, relative to its root:
/// `MyApp.Business.SystemUser` → `my_app/business/system_user.rmo`.
///
/// Public so a caller enforcing the naming rule on its own entry files — as
/// `ramos test` does — spells the expected path the same way the loader does.
pub fn expected_file(path: &ModulePath) -> PathBuf {
    module_path_to_file(path)
}

/// `MyApp.Business.SystemUser` → `my_app/business/system_user.rmo`.
fn module_path_to_file(path: &ModulePath) -> PathBuf {
    let mut out = PathBuf::new();
    for segment in &path.0 {
        out.push(camel_to_snake(segment));
    }
    out.set_extension("rmo");
    out
}

/// `SystemUser` → `system_user`. Module names are `[a-zA-Z]` only, so a new
/// word is simply an uppercase letter.
fn camel_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ── finding the module paths a program mentions ──────────────────────────────

fn collect_module_refs(items: &[Item]) -> Vec<ModulePath> {
    let mut found = Vec::new();
    for item in items {
        match item {
            Item::Module(m) => {
                for (_, path) in &m.aliases {
                    found.push(path.clone());
                }
                for f in &m.functions {
                    block_refs(&f.body, &mut found);
                }
            }
            Item::Struct(s) => {
                found.extend(s.implements.iter().cloned());
                for (_, path) in &s.aliases {
                    found.push(path.clone());
                }
                for (_, default) in &s.attributes {
                    expr_refs(default, &mut found);
                }
                for f in &s.functions {
                    block_refs(&f.body, &mut found);
                }
            }
            Item::Trait(t) => {
                for f in &t.functions {
                    block_refs(&f.body, &mut found);
                }
            }
            Item::Function(f) => block_refs(&f.body, &mut found),
            Item::Statement(s) => stmt_refs(s, &mut found),
        }
    }
    found
}

fn block_refs(block: &Block, out: &mut Vec<ModulePath>) {
    for stmt in block {
        stmt_refs(stmt, out);
    }
}

fn stmt_refs(stmt: &Stmt, out: &mut Vec<ModulePath>) {
    match stmt {
        Stmt::Expr(e) => expr_refs(e, out),
        Stmt::Assign { value, .. } => expr_refs(value, out),
        Stmt::Alias { module, .. } => out.push(module.clone()),
    }
}

fn expr_refs(e: &Expr, out: &mut Vec<ModulePath>) {
    match e {
        Expr::ModuleRef(path) => out.push(path.clone()),
        Expr::StructLit { path, fields } => {
            out.push(path.clone());
            fields.iter().for_each(|(_, v)| expr_refs(v, out));
        }
        Expr::Call { callee, args, .. } => {
            if let Callee::Method { target, .. } = callee {
                expr_refs(target, out);
            }
            args.iter().for_each(|a| expr_refs(a, out));
        }
        Expr::Access { target, .. } => expr_refs(target, out),
        Expr::Unary { operand, .. } => expr_refs(operand, out),
        Expr::Binary { left, right, .. } => {
            expr_refs(left, out);
            expr_refs(right, out);
        }
        Expr::List { elements, rest } => {
            elements.iter().for_each(|x| expr_refs(x, out));
            if let Some(r) = rest {
                expr_refs(r, out);
            }
        }
        Expr::Tuple(xs) => xs.iter().for_each(|x| expr_refs(x, out)),
        Expr::Map(fields) => fields.iter().for_each(|(_, v)| expr_refs(v, out)),
        Expr::Str(pieces) => pieces.iter().for_each(|p| {
            if let StrPiece::Interp(e) = p {
                expr_refs(e, out);
            }
        }),
        Expr::Lambda { body, .. } => block_refs(body, out),
        Expr::Case { subject, arms } => {
            expr_refs(subject, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    expr_refs(g, out);
                }
                block_refs(&arm.body, out);
            }
        }
        Expr::Cond { arms } => {
            for arm in arms {
                expr_refs(&arm.condition, out);
                block_refs(&arm.body, out);
            }
        }
        Expr::If {
            condition,
            then_body,
            else_body,
        } => {
            expr_refs(condition, out);
            block_refs(then_body, out);
            if let Some(body) = else_body {
                block_refs(body, out);
            }
        }
        Expr::Run { body, arms } => {
            block_refs(body, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    expr_refs(g, out);
                }
                block_refs(&arm.body, out);
            }
        }
        _ => {}
    }
}
