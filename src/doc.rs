//! `ramos doc` — generate documentation for the stdlib.
//!
//! Walks every `.rmo` file in a source directory (default `stdlib/src/`), pairs the
//! authoritative AST (module name + function signatures) with the `@module_doc`
//! and `@doc` comment blocks extracted from the raw text, and renders each
//! module (plus the guide/examples/programs narrative pages) to an HTML
//! fragment. Those fragments are written as data — one `docs.json` — rather
//! than one file per page; a single static `index.html` shell fetches that
//! JSON and presents it, swapping in the right fragment as the URL hash
//! changes. Everything is written into an output directory (default `docs/`).
//!
//! The doc-comment format is documented in the README; in short:
//!
//! ```text
//! module Foo
//!   # @module_doc
//!   #
//!   # First paragraph is the summary.
//!   # Second paragraph is the body. `code spans` work, and indented
//!   #   blocks become <pre> examples.
//!
//!   function bar(x)
//!     # @doc
//!     #
//!     # Summary line.
//!     # @param x: what `x` is
//!     # @return what comes back
//!     native("bar", [x])
//! ```
//!
//! Comments are stripped by the lexer, so this extractor works on raw source
//! text, not the token stream. The AST is used only to authoritatively list
//! the module's functions and their parameters.

use crate::ast::{FnDef, Item, Program};
use crate::color::Color;
use crate::parser::parse;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ── public entry point ───────────────────────────────────────────────────

/// Generate the docs site.
///
/// `source_dir` holds the modules (`kernel.rmo`, `list.rmo`, …) — for the
/// standard library that is `stdlib/src`. Subdirectories are not walked, so a
/// project's `src/test/` is left out of its reference.
/// `out_dir` is where `index.html` (the static shell), `docs.json` (the
/// rendered content it presents), and `assets/` are written.
/// Returns the number of modules documented.
pub fn generate(source_dir: &Path, out_dir: &Path) -> Result<usize, String> {
    generate_with(source_dir, out_dir, &Options::default())
}

/// The optional narrative pages that sit alongside the module reference.
///
/// All three are skipped silently when the path is `None` or missing on
/// disk, so a checkout without fixtures, example programs, or a README still
/// gets a complete reference.
#[derive(Debug, Default, Clone)]
pub struct Options<'a> {
    /// Directory of feature fixtures rendered into `examples.html`.
    pub examples_dir: Option<&'a Path>,
    /// Directory of runnable programs rendered into `programs.html` — files
    /// directly inside it are single-file programs, subdirectories are
    /// multi-file ones.
    pub programs_dir: Option<&'a Path>,
    /// Markdown file rendered into `guide.html` — normally the README.
    pub readme: Option<&'a Path>,
}

/// `generate`, plus whichever narrative pages `opts` asks for.
pub fn generate_with(source_dir: &Path, out_dir: &Path, opts: &Options) -> Result<usize, String> {
    let examples = match opts.examples_dir {
        Some(dir) => collect_examples(dir)?,
        None => Vec::new(),
    };
    let programs = match opts.programs_dir {
        Some(dir) => collect_programs(dir)?,
        None => Vec::new(),
    };
    let guide = match opts.readme {
        Some(path) => read_optional(path)?,
        None => None,
    };

    let mut modules = collect_modules(source_dir)?;
    if modules.is_empty() {
        return Err(format!(
            "no `.rmo` files found in `{}`",
            source_dir.display()
        ));
    }
    modules.sort_by(|a, b| a.name.cmp(&b.name));

    // One shared stylesheet, linked from the shell page.
    fs::create_dir_all(out_dir.join("assets")).map_err(|e| format!("create assets dir: {e}"))?;
    fs::write(out_dir.join("assets").join("style.css"), CSS)
        .map_err(|e| format!("write stylesheet: {e}"))?;
    fs::write(out_dir.join("assets").join("ramos-logo.png"), LOGO_PNG)
        .map_err(|e| format!("write logo: {e}"))?;

    let module_names: Vec<String> = modules.iter().map(|m| m.name.clone()).collect();
    let guides = Guides {
        examples: !examples.is_empty(),
        programs: !programs.is_empty(),
        guide: guide.is_some(),
    };

    // Every page's content, keyed by the id the shell's router looks it up
    // by (a module name, `"guide"`/`"examples"`/`"programs"`, or `""` for the
    // index) — this *is* the site; the shell is just the template around it.
    let mut pages: Vec<(String, Json)> = Vec::new();

    for m in &modules {
        let body = render_module(m, &module_names);
        let title = format!("{} — Ramos", m.name);
        pages.push((m.name.clone(), page_json(&title, Some(&m.name), "", body)));
    }

    if guides.examples {
        let body = render_examples(&examples, &module_names, guides);
        pages.push((
            "examples".to_string(),
            page_json("Examples — Ramos", None, "examples", body),
        ));
    }

    if guides.programs {
        let body = render_programs(&programs, &module_names, guides);
        pages.push((
            "programs".to_string(),
            page_json("Programs — Ramos", None, "examples", body),
        ));
    }

    if let Some(md) = &guide {
        let body = render_guide(md, &module_names);
        pages.push((
            "guide".to_string(),
            page_json("Language guide — Ramos", None, "guide", body),
        ));
    }

    let index_body = render_index(&modules, guides, guide.as_deref());
    pages.push((String::new(), page_json("Ramos", None, "index", index_body)));

    let doc_json = Json::Object(vec![
        (
            "modules".to_string(),
            Json::Array(module_names.into_iter().map(Json::Str).collect()),
        ),
        (
            "guides".to_string(),
            Json::Object(vec![
                ("guide".to_string(), Json::Bool(guides.guide)),
                ("examples".to_string(), Json::Bool(guides.examples)),
                ("programs".to_string(), Json::Bool(guides.programs)),
            ]),
        ),
        ("pages".to_string(), Json::Object(pages)),
    ]);
    fs::write(out_dir.join("docs.json"), doc_json.to_string())
        .map_err(|e| format!("write docs.json: {e}"))?;

    // The shell is static — identical regardless of what's documented — so
    // it's a fixed template rather than something built per module.
    fs::write(out_dir.join("index.html"), SHELL_HTML)
        .map_err(|e| format!("write index.html: {e}"))?;

    Ok(modules.len())
}

/// Which narrative pages exist, so cross-page prose links (e.g. "see the
/// language guide") only appear when the page they'd point to actually does.
#[derive(Debug, Clone, Copy, Default)]
struct Guides {
    guide: bool,
    examples: bool,
    programs: bool,
}

/// Read a file that is allowed not to exist.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

// ── extracted documentation ──────────────────────────────────────────────

/// A fully-documented module, ready to render.
#[derive(Debug, Clone)]
pub struct ModuleDoc {
    pub name: String,
    /// First paragraph of the `@module_doc`, shown on the index and as a
    /// subtitle. Empty when there is no module doc.
    pub summary: String,
    /// Full parsed module description (summary paragraph included), as raw
    /// (unescaped) blocks.
    pub description: Vec<Block>,
    pub functions: Vec<FunctionDoc>,
}

#[derive(Debug, Clone)]
pub struct FunctionDoc {
    pub name: String,
    pub params: Vec<String>,
    pub private: bool,
    /// `function name(a, b)` / `helper name(a, b)`.
    pub signature: String,
    pub summary: String,
    pub description: Vec<Block>,
    pub params_docs: Vec<(String, String)>,
    pub returns: Option<String>,
}

/// A chunk of doc text. Paragraphs hold raw source (escaped + linked at render
/// time so the module context supplies cross-links); code blocks are stored
/// already HTML-escaped because they're shown verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Raw, unescaped paragraph text (code spans are *not* yet resolved).
    Paragraph(String),
    /// Bulleted list; each item is raw text (continuation lines joined).
    List(Vec<String>),
    /// A code example, already HTML-escaped.
    Code(String),
    /// A file-name label marking where a multi-file program's next file
    /// begins — e.g. `account.rmo`. Only ever appears in a `ProgramDoc`.
    FileHeading(String),
}

// ── collecting from disk ─────────────────────────────────────────────────

fn collect_modules(dir: &Path) -> Result<Vec<ModuleDoc>, String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("read stdlib dir `{}`: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rmo"))
        .collect();
    entries.sort();

    let mut out = Vec::new();
    for path in entries {
        let source =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        out.push(build_module(&source, &path)?);
    }
    Ok(out)
}

// ── examples (feature fixtures) ──────────────────────────────────────────

/// One fixture (or, for the Programs page, one runnable program), rendered as
/// a walkthrough: a title, an intro paragraph taken from a header comment,
/// and the rest as alternating prose / code steps.
#[derive(Debug, Clone)]
pub struct ExampleDoc {
    /// Anchor and file stem, e.g. `string_interpolation`.
    pub slug: String,
    /// Display title, e.g. `String interpolation`.
    pub title: String,
    /// First paragraph of the header comment, minus the `foo.rmo — ` prefix.
    pub summary: String,
    /// Prose and code, in source order.
    pub blocks: Vec<Block>,
}

fn collect_examples(dir: &Path) -> Result<Vec<ExampleDoc>, String> {
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rmo"))
            .collect(),
        // A missing examples dir is not an error — the docs just lose the page.
        Err(_) => return Ok(Vec::new()),
    };
    entries.sort();

    let mut out = Vec::new();
    for path in entries {
        let source =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("example")
            .to_string();
        out.push(build_example(&slug, &source));
    }
    Ok(out)
}

// ── programs (examples/ directory) ───────────────────────────────────────

/// Collect `examples/` into one walkthrough per runnable program. A `.rmo`
/// file directly in `dir` is a single-file program (same shape as a feature
/// fixture); a subdirectory is a multi-file program — every `.rmo` file in
/// it, concatenated in one section.
fn collect_programs(dir: &Path) -> Result<Vec<ExampleDoc>, String> {
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        // A missing examples dir is not an error — the docs just lose the page.
        Err(_) => return Ok(Vec::new()),
    };
    entries.sort();

    let mut out = Vec::new();
    for path in entries {
        if path.is_dir() {
            if let Some(doc) = build_program_dir(&path)? {
                out.push(doc);
            }
        } else if path.extension().and_then(|s| s.to_str()) == Some("rmo") {
            let source =
                fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("program")
                .to_string();
            out.push(build_example(&slug, &source));
        }
    }
    Ok(out)
}

/// A multi-file program: every `.rmo` file directly inside `dir`, ordered so
/// the one whose module defines `function main()` — the entrypoint `ramos run`
/// calls — reads last, since it is what puts the other files' definitions to
/// use. Falls back to file order if none defines one. The program's summary
/// comes from that entry file's own header comment; every other file's header
/// comment stays inline as ordinary prose ahead of its code, under a small
/// file-name label.
fn build_program_dir(dir: &Path) -> Result<Option<ExampleDoc>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rmo"))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let source =
            fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        files.push((name, source));
    }

    let entry_index = files
        .iter()
        .position(|(_, source)| defines_main(source))
        .unwrap_or(0);
    let (entry_name, entry_source) = files.remove(entry_index);
    let entry_slug = entry_name.trim_end_matches(".rmo").to_string();
    let entry_doc = build_example(&entry_slug, &entry_source);
    files.push((entry_name, entry_source));

    let mut blocks = Vec::with_capacity(files.len() * 2);
    for (name, source) in &files {
        blocks.push(Block::FileHeading(name.clone()));
        if name.trim_end_matches(".rmo") == entry_slug {
            // Already stripped of its header comment, used as the summary.
            blocks.extend(entry_doc.blocks.clone());
        } else {
            // Kept as a block (unlike the entry file), but still trimmed of
            // its own `name.rmo — ` lead-in — the file-name label just above
            // already says which file this is.
            let mut file_blocks = example_blocks(source);
            if let Some(Block::Paragraph(p)) = file_blocks.first_mut() {
                *p = strip_file_prefix(p, name.trim_end_matches(".rmo"));
            }
            blocks.extend(file_blocks);
        }
    }

    let dir_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("program")
        .to_string();
    Ok(Some(ExampleDoc {
        slug: dir_name.clone(),
        title: title_for_slug(&dir_name),
        summary: entry_doc.summary,
        blocks,
    }))
}

/// Whether `source` parses to a program whose module defines a runnable,
/// public `function main()` — the entrypoint convention `ramos run` uses to decide
/// which file of a multi-file program under `examples/` is the front door.
fn defines_main(source: &str) -> bool {
    let Ok(tokens) = crate::lexer::lex(source) else {
        return false;
    };
    let Ok(program) = parse(tokens) else {
        return false;
    };
    program.items.iter().any(|item| match item {
        Item::Module(m) => m
            .functions
            .iter()
            .any(|f| f.name == "main" && !f.private && !f.body.is_empty()),
        _ => false,
    })
}

/// Turn fixture or program source into blocks: `#` comments become prose,
/// everything else becomes a code block, in the order they appear. The
/// header comment (if any) survives as the first `Paragraph` — callers that
/// want it split out as a summary do that themselves, since a multi-file
/// program shows a supporting file's header inline rather than peeling it
/// off.
fn example_blocks(source: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut comment: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();

    for raw in source.lines() {
        match comment_content(raw) {
            Some(content) => {
                flush_example_code(&mut code, &mut blocks);
                comment.push(content);
            }
            None => {
                if !comment.is_empty() {
                    blocks.extend(parse_blocks(&std::mem::take(&mut comment)));
                }
                // Blank lines inside a code run are kept; leading ones are not.
                if raw.trim().is_empty() && code.is_empty() {
                    continue;
                }
                code.push(raw.trim_end().to_string());
            }
        }
    }
    if !comment.is_empty() {
        blocks.extend(parse_blocks(&comment));
    }
    flush_example_code(&mut code, &mut blocks);
    blocks
}

/// `example_blocks`, with the header comment peeled off as the summary so it
/// can head the section instead of repeating the file name.
fn build_example(slug: &str, source: &str) -> ExampleDoc {
    let mut blocks = example_blocks(source);

    // The header comment opens with `slug.rmo — …`; drop that prefix so the
    // summary reads as a sentence under the heading.
    let mut summary = String::new();
    if let Some(Block::Paragraph(p)) = blocks.first() {
        summary = strip_file_prefix(p, slug);
        blocks.remove(0);
    }

    ExampleDoc {
        slug: slug.to_string(),
        title: title_for_slug(slug),
        summary,
        blocks,
    }
}

/// Drop a leading `slug.rmo — ` / `slug.rmo - ` lead-in from the header text.
fn strip_file_prefix(text: &str, slug: &str) -> String {
    let rest = match text.strip_prefix(&format!("{slug}.rmo")) {
        Some(r) => r.trim_start(),
        None => return text.to_string(),
    };
    let rest = rest
        .strip_prefix('—')
        .or_else(|| rest.strip_prefix("--"))
        .or_else(|| rest.strip_prefix('-'))
        .unwrap_or(rest);
    rest.trim_start().to_string()
}

/// `string_interpolation` → `String interpolation`.
fn title_for_slug(slug: &str) -> String {
    let spaced = slug.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

/// Like `flush_code`, but without the common-indent trim: fixture code is
/// already at column zero and its indentation is the point.
fn flush_example_code(code: &mut Vec<String>, blocks: &mut Vec<Block>) {
    while code.last().is_some_and(|l| l.trim().is_empty()) {
        code.pop();
    }
    if code.is_empty() {
        return;
    }
    blocks.push(Block::Code(highlight_html(&code.join("\n"))));
    code.clear();
}

fn build_module(source: &str, path: &Path) -> Result<ModuleDoc, String> {
    let tokens = crate::lexer::lex(source)
        .map_err(|e| format!("lex {}: {e}", path.display(), e = e.message))?;
    let program: Program =
        parse(tokens).map_err(|e| format!("parse {}: {e}", path.display(), e = e.message))?;

    // A stdlib file is a single top-level `module X` — or a `trait X` or
    // `struct X`, which document the same way: a name and a list of
    // functions. A struct's attributes are not rendered specially; a module
    // documenting one (like `Date`) describes its shape in prose instead.
    let (name, defined_functions) = program
        .items
        .iter()
        .find_map(|it| match it {
            Item::Module(m) => Some((m.name.to_string(), &m.functions)),
            Item::Trait(t) => Some((t.name.to_string(), &t.functions)),
            Item::Struct(s) => Some((s.name.to_string(), &s.functions)),
            _ => None,
        })
        .ok_or_else(|| {
            format!(
                "{}: no `module`, `trait`, or `struct` declaration",
                path.display()
            )
        })?;

    // Pull doc text out of the raw source: per function name, the doc block
    // (if any), plus the module doc block.
    let extracted = extract_docs(source);

    let (mod_summary, mod_desc) = match extracted.module_doc {
        Some(raw) => parse_doc_text(&raw),
        None => (String::new(), Vec::new()),
    };

    let mut functions = Vec::with_capacity(defined_functions.len());
    for f in defined_functions {
        let doc = match extracted.function_docs.get(&f.name) {
            Some(raw) => parse_function_doc(raw),
            None => ParsedFnDoc {
                summary: String::new(),
                blocks: Vec::new(),
                params: Vec::new(),
                returns: None,
            },
        };
        functions.push(make_function_doc(
            f,
            doc.summary,
            doc.blocks,
            doc.params,
            doc.returns,
        ));
    }

    Ok(ModuleDoc {
        name,
        summary: mod_summary,
        description: mod_desc,
        functions,
    })
}

fn make_function_doc(
    f: &FnDef,
    summary: String,
    description: Vec<Block>,
    params_docs: Vec<(String, String)>,
    returns: Option<String>,
) -> FunctionDoc {
    let keyword = if f.private { "helper" } else { "function" };
    let signature = format!("{} {}({})", keyword, f.name, f.params.join(", "));
    FunctionDoc {
        name: f.name.clone(),
        params: f.params.clone(),
        private: f.private,
        signature,
        summary,
        description,
        params_docs,
        returns,
    }
}

/// The one-line doc summaries in a source file, as plain text.
///
/// The HTML pipeline above is the full treatment; this is the small one, for
/// callers that want the prose without a page around it — `ramos test` prints
/// it above each module and test.
#[derive(Debug, Clone, Default)]
pub struct Summaries {
    /// First paragraph of the `@module_doc`, if the file has one.
    pub module: Option<String>,
    /// First paragraph of each function's `@doc`, keyed by function name.
    pub functions: HashMap<String, String>,
}

/// Pull the `@module_doc` / `@doc` summaries out of Ramos source.
///
/// Docs live in comments, so this reads the raw text rather than the AST —
/// the same extraction the doc generator runs, stopping at the first
/// paragraph. A file with no doc comments yields empty summaries.
pub fn summaries(source: &str) -> Summaries {
    let extracted = extract_docs(source);
    let module = extracted
        .module_doc
        .map(|raw| parse_doc_text(&raw).0)
        .filter(|s| !s.is_empty());
    let functions = extracted
        .function_docs
        .iter()
        .map(|(name, raw)| (name.clone(), parse_function_doc(raw).summary))
        .filter(|(_, summary)| !summary.is_empty())
        .collect();
    Summaries { module, functions }
}

// ── raw-text doc extraction ──────────────────────────────────────────────

/// Per-source extraction result.
struct Extracted {
    /// The `@module_doc` block, comment markers stripped, in order.
    module_doc: Option<Vec<String>>,
    /// Per-function `@doc` blocks, keyed by function name.
    function_docs: HashMap<String, Vec<String>>,
}

/// Walk raw source line-by-line and pull out the `@module_doc` / `@doc` blocks.
///
/// The rule mirrors how the comments are written: a comment block marked
/// `@module_doc` that appears *before the first function head* is the module
/// doc; a comment block marked `@doc` attaches to the most recently seen
/// `function`/`helper` head. Comments not marked with either tag are ignored, which
/// keeps incidental comments (the ones explaining a tricky `case` arm) out of
/// the reference.
fn extract_docs(source: &str) -> Extracted {
    let mut module_doc: Option<Vec<String>> = None;
    let mut current_fn: Option<String> = None;
    let mut function_docs: HashMap<String, Vec<String>> = HashMap::new();

    // A buffer for the in-progress comment block: cleaned lines (no leading
    // `#`, no single leading space), in order.
    let mut buf: Vec<String> = Vec::new();
    let mut buf_marker: Option<Marker> = None;
    // `buf.is_empty()` alone can't tell "block not started" from "marker line
    // consumed, no tail yet", so this flag marks that we've entered a comment
    // block on this pass.
    let mut in_block = false;
    let mut module_doc_taken = false;

    for raw in source.lines() {
        if let Some(content) = comment_content(raw) {
            if !in_block {
                // First line of a fresh block — it may carry a marker.
                in_block = true;
                if let Some(marker) = marker_for(&content) {
                    buf_marker = Some(marker);
                    // The marker line itself isn't doc text, but a tail after
                    // the marker (e.g. `# @doc quick note`) is kept.
                    if let Some(rest) = tail_after_marker(&content, marker) {
                        buf.push(rest);
                    }
                    continue;
                }
                buf.push(content);
            } else {
                buf.push(content);
            }
            continue;
        }

        // Non-comment line. Flush whatever block we were accumulating.
        flush_block(
            std::mem::take(&mut buf_marker),
            std::mem::take(&mut buf),
            &mut module_doc,
            &mut current_fn,
            &mut function_docs,
            &mut module_doc_taken,
        );
        in_block = false;

        let trimmed = raw.trim_start();
        if after_keyword(trimmed, "module").is_some() {
            // `module Foo` — module head. Nothing else to do here; the
            // `@module_doc` we capture next belongs to the module.
            continue;
        }
        if let Some((name, _private)) = fn_head(trimmed) {
            current_fn = Some(name);
        }
        // Any other code line leaves `current_fn` as-is, so a `@doc` block
        // still attaches to the right function even with code between the
        // `function` head and its doc (rare, but tolerated).
    }

    // Flush a trailing block at EOF.
    flush_block(
        std::mem::take(&mut buf_marker),
        std::mem::take(&mut buf),
        &mut module_doc,
        &mut current_fn,
        &mut function_docs,
        &mut module_doc_taken,
    );

    Extracted {
        module_doc,
        function_docs,
    }
}

fn flush_block(
    marker: Option<Marker>,
    lines: Vec<String>,
    module_doc: &mut Option<Vec<String>>,
    current_fn: &mut Option<String>,
    function_docs: &mut HashMap<String, Vec<String>>,
    module_doc_taken: &mut bool,
) {
    let Some(marker) = marker else { return };
    if lines.is_empty() {
        return;
    }
    match marker {
        Marker::ModuleDoc => {
            // Only the *first* `@module_doc` block counts as the module
            // description; later ones (e.g. a stray marker inside a function)
            // are ignored.
            if !*module_doc_taken {
                *module_doc = Some(lines);
                *module_doc_taken = true;
            }
        }
        Marker::Doc => {
            if let Some(name) = current_fn {
                // The first `@doc` for a given function wins.
                function_docs.entry(name.clone()).or_insert(lines);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    ModuleDoc,
    Doc,
}

/// If `line` is a `#` comment, return its content with the leading `#` and one
/// optional space removed. Otherwise return `None`.
fn comment_content(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix('#')?;
    // Strip exactly one leading space (the conventional `# `), keeping any
    // further indentation so code examples (two-space indent) survive.
    let content = trimmed.strip_prefix(' ').unwrap_or(trimmed);
    Some(content.to_string())
}

fn marker_for(content: &str) -> Option<Marker> {
    let t = content.trim_start();
    if t == "@module_doc" || t.starts_with("@module_doc ") {
        Some(Marker::ModuleDoc)
    } else if t == "@doc" || t.starts_with("@doc ") {
        Some(Marker::Doc)
    } else {
        None
    }
}

/// Text after `@module_doc`/`@doc` on the same line, if any. Most authors put
/// the marker on its own line, but `# @doc quick one-liner` is allowed.
fn tail_after_marker(content: &str, marker: Marker) -> Option<String> {
    let tag = match marker {
        Marker::ModuleDoc => "@module_doc",
        Marker::Doc => "@doc",
    };
    let rest = content.trim_start().strip_prefix(tag)?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let rest = rest.trim_end();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// If `line` starts with `keyword` followed by whitespace, return the rest.
fn after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let line = line.strip_prefix(keyword)?;
    let line = line.strip_prefix(char::is_whitespace)?;
    Some(line)
}

/// Parse a `function`/`helper name(params)` head. Returns `(name, is_private)`.
/// Requires a word boundary after the keyword, so `functionality` isn't
/// mistaken for `function`.
pub(crate) fn fn_head(line: &str) -> Option<(String, bool)> {
    let (rest, private) = if let Some(r) = line.strip_prefix("helper") {
        (r, true)
    } else if let Some(r) = line.strip_prefix("function") {
        (r, false)
    } else {
        return None;
    };
    // The keyword must be followed by whitespace (a word boundary).
    let rest = rest.strip_prefix(char::is_whitespace)?;
    let rest = rest.trim_start();
    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), private))
}

// ── parsing doc text into blocks ─────────────────────────────────────────

/// Split a `@module_doc` block into (summary, blocks).
///
/// The summary is the first paragraph (raw, single line via join).
fn parse_doc_text(lines: &[String]) -> (String, Vec<Block>) {
    let blocks = parse_blocks(lines);
    let summary = first_paragraph(&blocks).unwrap_or_default();
    (summary, blocks)
}

/// Result of peeling the tags out of a `@doc` block.
struct ParsedFnDoc {
    summary: String,
    blocks: Vec<Block>,
    params: Vec<(String, String)>,
    returns: Option<String>,
}

/// Split a `@doc` block, peeling `@param`/`@return` lines out of the prose.
fn parse_function_doc(lines: &[String]) -> ParsedFnDoc {
    let mut prose: Vec<String> = Vec::new();
    let mut params: Vec<(String, String)> = Vec::new();
    let mut returns: Option<String> = None;
    for line in lines {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("@param") {
            let rest = rest.trim_start();
            // `name: description`
            if let Some(colon) = rest.find(':') {
                let name = rest[..colon].trim().to_string();
                let desc = rest[colon + 1..].trim().to_string();
                if !name.is_empty() {
                    params.push((name, desc));
                    continue;
                }
            }
            // Bare `@param name` with no description.
            let name = rest.trim().to_string();
            if !name.is_empty() {
                params.push((name, String::new()));
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("@return") {
            returns = Some(rest.trim().to_string());
            continue;
        }
        prose.push(line.clone());
    }

    let blocks = parse_blocks(&prose);
    let summary = first_paragraph(&blocks).unwrap_or_default();
    ParsedFnDoc {
        summary,
        blocks,
        params,
        returns,
    }
}

fn first_paragraph(blocks: &[Block]) -> Option<String> {
    blocks.iter().find_map(|b| match b {
        Block::Paragraph(p) => Some(p.clone()),
        _ => None,
    })
}

/// Group cleaned doc lines into paragraphs, lists, and code blocks.
///
/// - A line whose trimmed form starts with `- ` is a list item; indented
///   lines that follow without their own `- ` are continuations of that item.
/// - Any other line with leading whitespace (relative to the `# `) is code.
/// - Blank lines separate blocks.
/// - A paragraph's lines are joined with a single space; inline markup
///   (escaping, code spans, cross-links) is applied at render time.
fn parse_blocks(lines: &[String]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();
    let mut list: Vec<String> = Vec::new();

    for line in lines {
        if line.trim().is_empty() {
            flush_para(&mut para, &mut blocks);
            flush_code(&mut code, &mut blocks);
            flush_list(&mut list, &mut blocks);
            continue;
        }
        let item_text = line.trim_start().strip_prefix("- ");
        if let Some(item) = item_text {
            // A new list item. Close any open paragraph or code block first.
            flush_para(&mut para, &mut blocks);
            flush_code(&mut code, &mut blocks);
            list.push(item.trim_end().to_string());
        } else if line.starts_with(' ') {
            // Indented.
            if !list.is_empty() {
                // Continuation of the most recent list item.
                let last = list.last_mut().unwrap();
                let cont = line.trim();
                if !cont.is_empty() {
                    last.push(' ');
                    last.push_str(cont);
                }
            } else {
                // Genuine code line. Close any open paragraph.
                flush_para(&mut para, &mut blocks);
                code.push(line.to_string());
            }
        } else {
            // Prose.
            flush_code(&mut code, &mut blocks);
            flush_list(&mut list, &mut blocks);
            para.push(line.trim_end().to_string());
        }
    }
    flush_para(&mut para, &mut blocks);
    flush_code(&mut code, &mut blocks);
    flush_list(&mut list, &mut blocks);
    blocks
}

fn flush_para(para: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if para.is_empty() {
        return;
    }
    let joined = para.join(" ");
    if !joined.is_empty() {
        blocks.push(Block::Paragraph(joined));
    }
    para.clear();
}

fn flush_code(code: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if code.is_empty() {
        return;
    }
    // Trim the common leading indentation from the block so a two-space
    // indented example doesn't render with extra padding.
    let indent = common_indent(code);
    let joined = code
        .iter()
        .map(|l| {
            if l.len() >= indent {
                &l[indent..]
            } else {
                l.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    blocks.push(Block::Code(highlight_html(&joined)));
    code.clear();
}

fn flush_list(list: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if list.is_empty() {
        return;
    }
    blocks.push(Block::List(std::mem::take(list)));
}

/// Longest run of leading spaces shared by every (non-empty) line.
fn common_indent(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0)
}

// ── tiny inline markdown ─────────────────────────────────────────────────

/// Escape text and resolve `` `code` `` spans and `*emphasis*`, linking
/// `Mod`/`Mod.fn` when `modules` lists `Mod`. This is the single place HTML
/// escaping happens for prose, so paragraphs can be stored raw until render.
fn render_inline(text: &str, modules: &[String]) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '`' {
            flush_plain(&mut out, &mut buf);
            // Read until the closing backtick.
            let mut code = String::new();
            for cc in chars.by_ref() {
                if cc == '`' {
                    break;
                }
                code.push(cc);
            }
            out.push_str(&render_code_span(&code, modules));
        } else if c == '*' {
            flush_plain(&mut out, &mut buf);
            // Read until the closing `*`.
            let mut emph = String::new();
            for cc in chars.by_ref() {
                if cc == '*' {
                    break;
                }
                emph.push(cc);
            }
            out.push_str(&format!("<em>{}</em>", html_escape(&emph)));
        } else {
            buf.push(c);
        }
    }
    flush_plain(&mut out, &mut buf);
    out
}

fn flush_plain(out: &mut String, buf: &mut String) {
    if !buf.is_empty() {
        out.push_str(&html_escape(buf));
        buf.clear();
    }
}

fn render_code_span(code: &str, modules: &[String]) -> String {
    let target = link_target(code, modules);
    let escaped = html_escape(code);
    match target {
        Some(href) => format!("<code><a href=\"{href}\">{escaped}</a></code>"),
        None => format!("<code>{escaped}</code>"),
    }
}

/// `Mod` → `#/Mod`; `Mod.fn` → `#/Mod:fn` — a route the shell's client-side
/// router resolves against `docs.json`, rather than a filename, since a
/// module no longer has a page of its own on disk. Only dotted forms link —
/// bare lowercase names (`print(x)`-style) stay plain, since `Kernel` is
/// implicit and a bare name rarely names a module.
fn link_target(code: &str, modules: &[String]) -> Option<String> {
    let code = code.trim();
    if code.is_empty() {
        return None;
    }
    if let Some((mod_name, rest)) = code.split_once('.') {
        if modules.iter().any(|m| m == mod_name) {
            let member = rest.trim();
            if member.is_empty() {
                return Some(format!("#/{mod_name}"));
            }
            let end = member
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(member.len());
            let member_name = &member[..end];
            return Some(format!("#/{mod_name}:{member_name}"));
        }
        return None;
    }
    // A bare capitalized name could be a module.
    if modules.iter().any(|m| m == code) {
        return Some(format!("#/{code}"));
    }
    None
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a `<pre><code>` block's contents: syntax-highlighted when `code`
/// lexes as Ramos, plain escaped text otherwise. The fallback matters because
/// not every fenced block shown verbatim *is* a complete Ramos program — the
/// README also fences shell commands, and a doc-comment example is sometimes
/// a bare expression — so a block the lexer chokes on (an indentation rule it
/// enforces that shell output doesn't follow, say) still renders instead of
/// failing the whole doc build.
fn highlight_html(code: &str) -> String {
    match crate::lexer::lex(code) {
        Ok(tokens) => crate::lexer::highlight(code, &tokens, Color::Html),
        Err(_) => html_escape(code),
    }
}

// ── markdown (the README guide) ──────────────────────────────────────────

/// A rendered heading, kept for the page's table of contents.
struct Heading {
    level: u8,
    slug: String,
    text: String,
}

/// Where relative links in the README point once the README is served from
/// `docs/`. `[stdlib/](stdlib/)` only resolves inside the repository, so links
/// are rewritten to the repository browser; taken from Cargo.toml so it can't
/// drift from the crate metadata.
const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

/// Render a Markdown subset — the one the README actually uses — to HTML.
///
/// Supported: ATX headings, fenced code, tables, blockquotes, `-`/`*` lists,
/// thematic breaks, paragraphs, and inline `code`/`**strong**`/`*em*`/links.
/// Raw HTML blocks (the README's centred logo and title) are dropped, since
/// the page supplies its own header. Returns the HTML and its headings.
fn render_markdown(source: &str, modules: &[String]) -> (String, Vec<Heading>) {
    let mut out = String::new();
    let mut headings = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut list: Vec<String> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    let mut table: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();
    let mut in_code = false;

    // Closing every open block before starting a new one keeps the state
    // machine flat: each branch below opens exactly one kind of block.
    macro_rules! close_open_blocks {
        () => {
            flush_md_para(&mut para, &mut out, modules);
            flush_md_list(&mut list, &mut out, modules);
            flush_md_quote(&mut quote, &mut out, modules);
            flush_md_table(&mut table, &mut out, modules);
        };
    }

    for raw in source.lines() {
        let trimmed = raw.trim_start();

        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_code {
                out.push_str(&format!(
                    "<pre><code>{}</code></pre>\n",
                    highlight_html(&code.join("\n"))
                ));
                code.clear();
                in_code = false;
            } else {
                close_open_blocks!();
                in_code = true;
                // The README tags its fences `elixir` (GitHub has no `ramos`
                // grammar, and the syntax is close enough) purely for its own
                // rendering; every block here gets the same Ramos lexer-based
                // highlighting regardless, so the tag is unused.
                let _lang = rest.trim();
            }
            continue;
        }
        if in_code {
            code.push(raw.to_string());
            continue;
        }

        if trimmed.is_empty() {
            close_open_blocks!();
            continue;
        }

        if let Some((level, text)) = md_heading(trimmed) {
            close_open_blocks!();
            let slug = slugify(&text);
            out.push_str(&format!(
                "<h{level} id=\"{slug}\">{}</h{level}>\n",
                render_md_inline(&text, modules),
            ));
            headings.push(Heading {
                level,
                slug,
                text: text.clone(),
            });
            continue;
        }

        if trimmed.starts_with("---") && trimmed.chars().all(|c| c == '-' || c.is_whitespace()) {
            close_open_blocks!();
            out.push_str("<hr>\n");
            continue;
        }

        // A raw-HTML block line — the centred logo/title header. Dropped.
        if trimmed.starts_with('<') {
            close_open_blocks!();
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            flush_md_para(&mut para, &mut out, modules);
            flush_md_list(&mut list, &mut out, modules);
            flush_md_table(&mut table, &mut out, modules);
            quote.push(rest.trim_start().to_string());
            continue;
        }

        // A table row: `| a | b |`. The `| --- |` separator is dropped by the
        // flush, which treats the first row as the header.
        if trimmed.starts_with('|') {
            flush_md_para(&mut para, &mut out, modules);
            flush_md_list(&mut list, &mut out, modules);
            flush_md_quote(&mut quote, &mut out, modules);
            table.push(trimmed.to_string());
            continue;
        }

        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush_md_para(&mut para, &mut out, modules);
            flush_md_quote(&mut quote, &mut out, modules);
            flush_md_table(&mut table, &mut out, modules);
            list.push(item.trim().to_string());
            continue;
        }
        // An indented line under an open list item continues it.
        if !list.is_empty() && raw.starts_with(' ') {
            let last = list.last_mut().unwrap();
            last.push(' ');
            last.push_str(trimmed.trim_end());
            continue;
        }

        flush_md_list(&mut list, &mut out, modules);
        flush_md_quote(&mut quote, &mut out, modules);
        flush_md_table(&mut table, &mut out, modules);
        para.push(trimmed.trim_end().to_string());
    }

    if in_code && !code.is_empty() {
        out.push_str(&format!(
            "<pre><code>{}</code></pre>\n",
            highlight_html(&code.join("\n"))
        ));
    }
    close_open_blocks!();

    (out, headings)
}

/// The markdown between a heading line (exclusive) and whatever heading
/// follows it (exclusive, any level) — used to lift a self-contained excerpt
/// of the README (its Philosophy intro, Hello-world snippet, Keywords table)
/// onto the home page without hand-duplicating the prose. Fenced code is
/// tracked so a `#` comment inside an example (as in the Hello-world snippet)
/// isn't mistaken for the next heading.
fn markdown_section<'a>(source: &'a str, heading_line: &str) -> &'a str {
    let marker = format!("{heading_line}\n");
    let start = match source.find(&marker) {
        Some(i) => i + marker.len(),
        None => return "",
    };
    let rest = &source[start..];

    let mut in_code = false;
    let mut pos = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_start();
        if trimmed.starts_with("```") {
            in_code = !in_code;
        } else if !in_code && md_heading(trimmed).is_some() {
            return &rest[..pos];
        }
        pos += line.len();
    }
    rest
}

/// `## Title` → `(2, "Title")`.
fn md_heading(line: &str) -> Option<(u8, String)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim().to_string()))
}

fn flush_md_para(para: &mut Vec<String>, out: &mut String, modules: &[String]) {
    if para.is_empty() {
        return;
    }
    out.push_str(&format!(
        "<p>{}</p>\n",
        render_md_inline(&para.join(" "), modules)
    ));
    para.clear();
}

fn flush_md_list(list: &mut Vec<String>, out: &mut String, modules: &[String]) {
    if list.is_empty() {
        return;
    }
    out.push_str("<ul>\n");
    for item in list.iter() {
        out.push_str(&format!("  <li>{}</li>\n", render_md_inline(item, modules)));
    }
    out.push_str("</ul>\n");
    list.clear();
}

fn flush_md_quote(quote: &mut Vec<String>, out: &mut String, modules: &[String]) {
    if quote.is_empty() {
        return;
    }
    out.push_str("<blockquote>\n");
    // A blank line inside a quote is written as a bare `>`, which arrives here
    // as an empty string — that's the paragraph break.
    let mut para: Vec<String> = Vec::new();
    for line in quote.iter() {
        if line.is_empty() {
            flush_md_para(&mut para, out, modules);
        } else {
            para.push(line.clone());
        }
    }
    flush_md_para(&mut para, out, modules);
    out.push_str("</blockquote>\n");
    quote.clear();
}

fn flush_md_table(table: &mut Vec<String>, out: &mut String, modules: &[String]) {
    if table.is_empty() {
        return;
    }
    let rows: Vec<Vec<String>> = table.iter().map(|r| md_table_cells(r)).collect();
    table.clear();

    let mut body = rows.iter();
    let Some(header) = body.next() else { return };

    out.push_str("<div class=\"table-wrap\">\n<table>\n<thead>\n  <tr>");
    for cell in header {
        out.push_str(&format!("<th>{}</th>", render_md_inline(cell, modules)));
    }
    out.push_str("</tr>\n</thead>\n<tbody>\n");
    for row in body {
        // Skip the `| --- | --- |` alignment row.
        if row
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        out.push_str("  <tr>");
        for cell in row {
            out.push_str(&format!("<td>{}</td>", render_md_inline(cell, modules)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</div>\n");
}

/// `| a | b |` → `["a", "b"]`.
fn md_table_cells(row: &str) -> Vec<String> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    row.split('|').map(|c| c.trim().to_string()).collect()
}

/// GitHub-style anchor slug, so the README's own `#section` links keep working
/// on the rendered page: lowercase, punctuation dropped, spaces to hyphens.
fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' || c == '_' {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Inline Markdown: `` `code` ``, `**strong**`, `*em*`, and `[text](href)`.
/// Code spans still get the stdlib cross-linking, so `` `List.map` `` in the
/// README lands on the reference page.
fn render_md_inline(text: &str, modules: &[String]) -> String {
    let mut out = String::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '`' => {
                flush_plain(&mut out, &mut plain);
                let (code, next) = take_until(&chars, i + 1, '`');
                out.push_str(&render_code_span(&code, modules));
                i = next;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                flush_plain(&mut out, &mut plain);
                let (strong, next) = take_until_str(&chars, i + 2, "**");
                out.push_str(&format!(
                    "<strong>{}</strong>",
                    render_md_inline(&strong, modules)
                ));
                i = next;
            }
            '*' => {
                flush_plain(&mut out, &mut plain);
                let (emph, next) = take_until(&chars, i + 1, '*');
                out.push_str(&format!("<em>{}</em>", render_md_inline(&emph, modules)));
                i = next;
            }
            // `![alt](src)` — the README's only image is the dropped logo, but
            // handle it rather than leaking the syntax as text.
            '!' if chars.get(i + 1) == Some(&'[') => {
                flush_plain(&mut out, &mut plain);
                let (alt, after_alt) = take_until(&chars, i + 2, ']');
                if chars.get(after_alt) == Some(&'(') {
                    let (src, next) = take_until(&chars, after_alt + 1, ')');
                    out.push_str(&format!(
                        "<img src=\"{}\" alt=\"{}\">",
                        html_escape(&src),
                        html_escape(&alt)
                    ));
                    i = next;
                } else {
                    plain.push('!');
                    i += 1;
                }
            }
            '[' => {
                flush_plain(&mut out, &mut plain);
                let (label, after_label) = take_until(&chars, i + 1, ']');
                if chars.get(after_label) == Some(&'(') {
                    let (href, next) = take_until(&chars, after_label + 1, ')');
                    out.push_str(&format!(
                        "<a href=\"{}\">{}</a>",
                        html_escape(&resolve_md_link(&href)),
                        render_md_inline(&label, modules)
                    ));
                    i = next;
                } else {
                    plain.push('[');
                    i += 1;
                }
            }
            c => {
                plain.push(c);
                i += 1;
            }
        }
    }
    flush_plain(&mut out, &mut plain);
    out
}

/// Collect chars until `end`, returning the text and the index after it. An
/// unclosed span runs to the end of the text.
fn take_until(chars: &[char], start: usize, end: char) -> (String, usize) {
    let mut buf = String::new();
    let mut i = start;
    while i < chars.len() {
        if chars[i] == end {
            return (buf, i + 1);
        }
        buf.push(chars[i]);
        i += 1;
    }
    (buf, i)
}

/// `take_until` for a two-char terminator (`**`).
fn take_until_str(chars: &[char], start: usize, end: &str) -> (String, usize) {
    let end: Vec<char> = end.chars().collect();
    let mut buf = String::new();
    let mut i = start;
    while i < chars.len() {
        if chars[i..].starts_with(&end[..]) {
            return (buf, i + end.len());
        }
        buf.push(chars[i]);
        i += 1;
    }
    (buf, i)
}

/// Point a README link at something that exists from `docs/`: in-page anchors
/// and absolute URLs are left alone, repo-relative paths go to the repository.
fn resolve_md_link(href: &str) -> String {
    let href = href.trim();
    if href.starts_with('#') || href.contains("://") || href.starts_with("mailto:") {
        return href.to_string();
    }
    format!(
        "{}/src/branch/main/{}",
        REPO_URL.trim_end_matches('/'),
        href
    )
}

// ── HTML rendering ───────────────────────────────────────────────────────

fn render_module(m: &ModuleDoc, all_modules: &[String]) -> String {
    let modules = all_modules.to_vec();

    // Module summary — the first paragraph of the description.
    let summary_block = if m.summary.is_empty() {
        String::new()
    } else {
        format!(
            "<section class=\"module-summary\">\n\
             <h2>Summary</h2>\n  <p>{}</p>\n</section>\n",
            render_inline(&m.summary, &modules)
        )
    };

    // Full description.
    let mut description = String::new();
    if !m.description.is_empty() {
        description.push_str("<section class=\"module-description\">\n");
        description.push_str("<h2>Module documentation</h2>\n");
        for block in &m.description {
            description.push_str(&render_block(block, &modules));
            description.push('\n');
        }
        description.push_str("</section>\n");
    }

    // Functions summary table.
    let fns_summary = if m.functions.is_empty() {
        String::new()
    } else {
        let mut s = String::from("<section class=\"functions-summary\">\n<h2>Functions</h2>\n");
        for f in &m.functions {
            let anchor = anchor(&f.name, f.params.len());
            let private = if f.private { " private" } else { "" };
            s.push_str(&format!(
                "  <div class=\"fn-row{private}\">\n    \
                 <a href=\"#{anchor}\" class=\"fn-sig\"><code>{}</code></a>\n    \
                 <span class=\"fn-summary\">{}</span>\n  </div>\n",
                html_escape(&f.signature),
                render_inline(&f.summary, &modules),
            ));
        }
        s.push_str("</section>\n");
        s
    };

    // Detailed function docs.
    let mut fns_detail = String::new();
    if !m.functions.is_empty() {
        fns_detail.push_str("<section class=\"functions-detail\">\n");
        for f in &m.functions {
            fns_detail.push_str(&render_function(f, &modules));
        }
        fns_detail.push_str("</section>\n");
    }

    format!("{summary_block}{description}{fns_summary}{fns_detail}")
}

fn render_function(f: &FunctionDoc, modules: &[String]) -> String {
    let anchor = anchor(&f.name, f.params.len());
    let private_badge = if f.private {
        " <span class=\"badge private\">private</span>"
    } else {
        ""
    };

    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"fn-detail\" id=\"{anchor}\">\n  \
         <h3 class=\"fn-title\"><code>{}</code>{private_badge} \
         <a class=\"anchor\" href=\"#{anchor}\">¶</a></h3>\n",
        html_escape(&f.signature),
    ));

    for block in &f.description {
        out.push_str(&render_block(block, modules));
        out.push('\n');
    }

    if !f.params_docs.is_empty() {
        out.push_str("<dl class=\"params\">\n");
        for (name, desc) in &f.params_docs {
            out.push_str(&format!(
                "  <dt><code>{}</code></dt>\n  <dd>{}</dd>\n",
                html_escape(name),
                render_inline(desc, modules),
            ));
        }
        out.push_str("</dl>\n");
    }

    if let Some(r) = &f.returns {
        out.push_str(&format!(
            "<p class=\"returns\"><strong>Returns:</strong> {}</p>\n",
            render_inline(r, modules),
        ));
    }

    out.push_str("</div>\n");
    out
}

fn render_block(block: &Block, modules: &[String]) -> String {
    match block {
        Block::Paragraph(p) => format!("<p>{}</p>", render_inline(p, modules)),
        Block::List(items) => {
            let mut s = String::from("<ul>");
            for item in items {
                s.push_str(&format!("<li>{}</li>", render_inline(item, modules)));
            }
            s.push_str("</ul>");
            s
        }
        Block::Code(c) => format!("<pre><code>{c}</code></pre>"),
        Block::FileHeading(name) => {
            format!(
                "<p class=\"program-file\"><code>{}</code></p>",
                html_escape(name)
            )
        }
    }
}

/// The `guide` page's body: the README, with a contents list of its sections.
/// The sidebar itself is built client-side in the shell from `docs.json`'s
/// `modules` and `guides` fields, rather than rendered per page here.
fn render_guide(markdown: &str, module_names: &[String]) -> String {
    let (content, headings) = render_markdown(markdown, module_names);

    let mut body = String::from("<h1>Language guide</h1>\n");
    let top: Vec<&Heading> = headings.iter().filter(|h| h.level == 2).collect();
    if !top.is_empty() {
        body.push_str("<ul class=\"guide-toc\">\n");
        for h in top {
            body.push_str(&format!(
                "  <li><a href=\"#{slug}\">{text}</a></li>\n",
                slug = html_escape(&h.slug),
                text = render_md_inline(&h.text, module_names),
            ));
        }
        body.push_str("</ul>\n");
    }
    body.push_str(&content);

    body
}

/// The `examples` page's body: a contents list, then one section per fixture.
fn render_examples(examples: &[ExampleDoc], module_names: &[String], guides: Guides) -> String {
    let modules = module_names.to_vec();

    let mut body = String::new();
    body.push_str("<h1>Examples</h1>\n");
    body.push_str(
        "<p>One short program per language feature, each taken straight from \
         the fixtures in <code>tests/fixtures/features/</code> — so every \
         snippet here lexes and parses as real Ramos.</p>\n",
    );
    if guides.guide {
        body.push_str(
            "<p>For the prose version of the same ground, see the \
             <a href=\"#/guide\">language guide</a>.</p>\n",
        );
    }

    body.push_str("<ul class=\"example-toc\">\n");
    for e in examples {
        body.push_str(&format!(
            "  <li><a href=\"#{slug}\">{title}</a> <span class=\"example-summary\">{summary}</span></li>\n",
            slug = html_escape(&e.slug),
            title = html_escape(&e.title),
            summary = render_inline(&e.summary, &modules),
        ));
    }
    body.push_str("</ul>\n");

    for e in examples {
        body.push_str(&format!(
            "<section class=\"example\" id=\"{slug}\">\n  \
             <h2>{title} <a class=\"anchor\" href=\"#{slug}\">¶</a></h2>\n",
            slug = html_escape(&e.slug),
            title = html_escape(&e.title),
        ));
        if !e.summary.is_empty() {
            body.push_str(&format!(
                "  <p class=\"example-summary\">{}</p>\n",
                render_inline(&e.summary, &modules),
            ));
        }
        for block in &e.blocks {
            body.push_str(&render_block(block, &modules));
            body.push('\n');
        }
        body.push_str("</section>\n");
    }

    body
}

/// The `programs` page's body: a contents list, then one section per runnable
/// program under `examples/`. Shares the Examples page's markup and CSS
/// classes (`example`/`example-toc`/`example-summary`) — the only structural
/// difference is a multi-file program's `Block::FileHeading` labels between
/// its files' code.
fn render_programs(programs: &[ExampleDoc], module_names: &[String], guides: Guides) -> String {
    let modules = module_names.to_vec();

    let mut body = String::new();
    body.push_str("<h1>Programs</h1>\n");
    body.push_str(
        "<p>Complete, runnable programs from <code>examples/</code> — each with an \
         entrypoint, run with <code>ramos run</code>. Where the Examples page shows \
         one language feature in isolation, these show several working together.</p>\n",
    );
    if guides.examples {
        body.push_str(
            "<p>For one small program per language feature instead, see the \
             <a href=\"#/examples\">Examples</a> page.</p>\n",
        );
    }
    if guides.guide {
        body.push_str(
            "<p>New to the language? Start with the \
             <a href=\"#/guide\">language guide</a>.</p>\n",
        );
    }

    body.push_str("<ul class=\"example-toc\">\n");
    for p in programs {
        body.push_str(&format!(
            "  <li><a href=\"#{slug}\">{title}</a> <span class=\"example-summary\">{summary}</span></li>\n",
            slug = html_escape(&p.slug),
            title = html_escape(&p.title),
            summary = render_inline(&p.summary, &modules),
        ));
    }
    body.push_str("</ul>\n");

    for p in programs {
        body.push_str(&format!(
            "<section class=\"example\" id=\"{slug}\">\n  \
             <h2>{title} <a class=\"anchor\" href=\"#{slug}\">¶</a></h2>\n",
            slug = html_escape(&p.slug),
            title = html_escape(&p.title),
        ));
        if !p.summary.is_empty() {
            body.push_str(&format!(
                "  <p class=\"example-summary\">{}</p>\n",
                render_inline(&p.summary, &modules),
            ));
        }
        for block in &p.blocks {
            body.push_str(&render_block(block, &modules));
            body.push('\n');
        }
        body.push_str("</section>\n");
    }

    body
}

fn render_index(modules: &[ModuleDoc], guides: Guides, guide_md: Option<&str>) -> String {
    let names: Vec<String> = modules.iter().map(|m| m.name.clone()).collect();

    let mut body = String::new();
    body.push_str("<h1>Ramos</h1>\n");
    body.push_str(
        "<p>A small, indentation-driven language that enforces a strict, \
         opinionated style so Ramos code looks the same everywhere.</p>\n",
    );

    if let Some(md) = guide_md {
        body.push_str("<h2 id=\"philosophy\">Philosophy</h2>\n");
        body.push_str(&render_markdown(markdown_section(md, "## Philosophy"), &names).0);
        if guides.guide {
            body.push_str(
                "<p><a href=\"#/guide:philosophy\">Read more in the language guide</a>.</p>\n",
            );
        }

        body.push_str("<h2 id=\"quick-hello-world\">Quick hello world</h2>\n");
        body.push_str(&render_markdown(markdown_section(md, "## Hello, world"), &names).0);

        body.push_str("<h2 id=\"keywords\">Keywords</h2>\n");
        body.push_str(&render_markdown(markdown_section(md, "## Keywords"), &names).0);
    }

    body.push_str("<h2 id=\"standard-library\">Standard library</h2>\n");
    body.push_str(
        "<p>This is the reference for the modules that ship with Ramos. \
         <code>Kernel</code> is always in scope; everything else is reached \
         via its module name or an <code>alias</code>.</p>\n",
    );
    // Each guide contributes one step, worded to read naturally whichever
    // steps are actually present; joined below rather than matched on every
    // 2^3 combination.
    let mut steps: Vec<String> = Vec::new();
    if guides.guide {
        steps.push("start with the <a href=\"#/guide\">language guide</a>".to_string());
    }
    if guides.examples {
        steps.push(
            "browse the <a href=\"#/examples\">examples</a> — one small program \
             per feature"
                .to_string(),
        );
    }
    if guides.programs {
        steps.push(
            "look at the <a href=\"#/programs\">programs</a> page for several \
             features working together"
                .to_string(),
        );
    }
    if let Some((first, rest)) = steps.split_first() {
        let mut first_letter = first.chars();
        let capitalized = match first_letter.next() {
            Some(c) => c.to_uppercase().collect::<String>() + first_letter.as_str(),
            None => first.clone(),
        };
        let joined = match rest {
            [] => capitalized,
            [only] => format!("{capitalized}, then {only}"),
            [middle, last] => format!("{capitalized}, then {middle}, or {last}"),
            _ => unreachable!("at most three guides exist"),
        };
        body.push_str(&format!("<p>New to the language? {joined}.</p>\n"));
    }
    body.push_str("<ul class=\"module-list\">\n");
    for m in modules {
        body.push_str(&format!(
            "  <li>\n    <a href=\"#/{name}\" class=\"module-name\">{name}</a>\n    \
             <p class=\"module-summary\">{summary}</p>\n  </li>\n",
            name = html_escape(&m.name),
            summary = render_inline(&m.summary, &names),
        ));
    }
    body.push_str("</ul>\n");

    body
}

/// One page's rendered content, ready to embed in `docs.json`. `heading`
/// becomes the shell's `<h1 class="module-header">` — used only by module
/// pages; the narrative pages (guide/examples/programs/index) carry their own
/// `<h1>` inline in `body` and pass `None` here. `content_class` is added to
/// the shell's content section so the stylesheet's per-page rules still apply.
fn page_json(title: &str, heading: Option<&str>, content_class: &str, body: String) -> Json {
    Json::Object(vec![
        ("title".to_string(), Json::Str(title.to_string())),
        (
            "heading".to_string(),
            match heading {
                Some(h) => Json::Str(h.to_string()),
                None => Json::Null,
            },
        ),
        (
            "content_class".to_string(),
            Json::Str(content_class.to_string()),
        ),
        ("body".to_string(), Json::Str(body)),
    ])
}

fn anchor(name: &str, arity: usize) -> String {
    format!("{name}/{arity}")
}

// ── JSON ─────────────────────────────────────────────────────────────────

/// The crate carries no dependencies, and `docs.json`'s shape is small and
/// fixed, so this hand-rolls just enough JSON to write it rather than pulling
/// in a serializer crate for one file.
enum Json {
    Null,
    Bool(bool),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Str(s) => {
                out.push('"');
                json_escape_into(s, out);
                out.push('"');
            }
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    json_escape_into(key, out);
                    out.push_str("\":");
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::new();
        self.write(&mut out);
        f.write_str(&out)
    }
}

fn json_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

// ── the static shell ────────────────────────────────────────────────────

/// The single HTML page every generated site ships: identical regardless of
/// what's documented, since the actual content lives in `docs.json` and this
/// just fetches it, builds the sidebar from its `modules`/`guides` fields,
/// and swaps the content section's markup in as the URL hash changes.
///
/// Route format: `#/<page>` navigates, where `<page>` is a module name,
/// `guide`/`examples`/`programs`, or empty for the index; `#/<page>:<anchor>`
/// additionally scrolls to an element by id once the page has loaded. A bare
/// `#<anchor>` (no leading slash) is left alone — that's an in-page jump to a
/// heading or function anchor on the page already showing, and the browser
/// handles it natively.
const SHELL_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Ramos</title>
  <link rel="stylesheet" href="assets/style.css">
</head>
<body>
<main class="layout">
  <nav class="sidebar" id="sidebar">
    <a class="brand" href="#/"><img src="assets/ramos-logo.png" alt="" class="brand-logo"></a>
    <div id="sidebar-nav"></div>
  </nav>
  <section class="content" id="content">
    <p>Loading…</p>
  </section>
</main>
<script>
(function () {
  "use strict";
  var content = document.getElementById("content");
  var sidebarNav = document.getElementById("sidebar-nav");
  var data = null;

  function escapeHtml(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // "#/Kernel:print" -> {page: "Kernel", anchor: "print"}; "#/" -> {page: "", anchor: null}.
  function parseHash() {
    var h = location.hash;
    if (h.indexOf("#/") !== 0) {
      return null;
    }
    h = h.slice(2);
    var sep = h.indexOf(":");
    if (sep === -1) {
      return { page: h, anchor: null };
    }
    return { page: h.slice(0, sep), anchor: h.slice(sep + 1) };
  }

  function buildSidebar() {
    var html = "";
    var guideLinks = [
      ["guide", "Language guide", data.guides.guide],
      ["examples", "Examples", data.guides.examples],
      ["programs", "Programs", data.guides.programs],
    ];
    if (data.guides.guide || data.guides.examples || data.guides.programs) {
      html += "<h3>Guides</h3>\n<ul>\n";
      guideLinks.forEach(function (entry) {
        if (!entry[2]) return;
        html +=
          '  <li><a href="#/' + entry[0] + '" data-page="' + entry[0] + '">' +
          entry[1] + "</a></li>\n";
      });
      html += "</ul>\n";
    }
    html += "<h3>Modules</h3>\n<ul>\n";
    data.modules.forEach(function (name) {
      html +=
        '  <li><a href="#/' + name + '" data-page="' + name + '">' + name + "</a></li>\n";
    });
    html += "</ul>\n";
    sidebarNav.innerHTML = html;
  }

  function highlightSidebar(pageId) {
    var links = document.querySelectorAll("#sidebar a[data-page]");
    for (var i = 0; i < links.length; i++) {
      var current = links[i].getAttribute("data-page") === pageId;
      links[i].classList.toggle("current", current);
    }
  }

  function render() {
    if (!data) return;
    var route = parseHash() || { page: "", anchor: null };
    var page = data.pages[route.page];
    if (!page) {
      page = data.pages[""];
      route = { page: "", anchor: null };
    }
    document.title = page.title;
    content.className = "content" + (page.content_class ? " " + page.content_class : "");
    var headingHtml = page.heading
      ? '<h1 class="module-header">' + escapeHtml(page.heading) + "</h1>\n"
      : "";
    content.innerHTML = headingHtml + page.body;
    highlightSidebar(route.page);
    if (route.anchor) {
      var el = document.getElementById(route.anchor);
      if (el) el.scrollIntoView();
    } else {
      window.scrollTo(0, 0);
    }
  }

  window.addEventListener("hashchange", function () {
    // A bare in-page anchor (no "#/" prefix) is the browser's own scroll —
    // routing should leave it alone rather than re-rendering the page away
    // out from under it.
    if (parseHash() === null) return;
    render();
  });

  fetch("docs.json")
    .then(function (r) { return r.json(); })
    .then(function (json) {
      data = json;
      buildSidebar();
      render();
    })
    .catch(function (err) {
      content.innerHTML =
        "<p>Could not load <code>docs.json</code>: " + escapeHtml(String(err)) +
        "</p><p>This page needs to be served over HTTP (not opened as a " +
        "<code>file://</code> URL) for the fetch to succeed.</p>";
    });
})();
</script>
</body>
</html>
"##;

// ── stylesheet ───────────────────────────────────────────────────────────

/// The Ramos logo, embedded so the docs are self-contained regardless of
/// where `ramos doc` runs from.
const LOGO_PNG: &[u8] = include_bytes!("../ramos-logo.png");

const CSS: &str = r#"
:root {
--bg: #f6fbf7;
--fg: #142b18;
--muted: #5b6d60;
--accent: #1a7f37;
--accent-soft: #e3f3e6;
--border: #d3e5d7;
--code-bg: #eef6f0;
--private: #b3184a;
--link: #1a7f37;
--syn-keyword: #cf222e;
--syn-type: #953800;
--syn-str: #0a3069;
--syn-lit: #0550ae;
}

* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  font-size: 16px;
  line-height: 1.6;
  color: var(--fg);
  background: var(--bg);
}
a { color: var(--link); text-decoration: none; }
a:hover { text-decoration: underline; }
code, pre {
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
  font-size: 0.9em;
}
code {
  background: var(--code-bg);
  padding: 0.15em 0.4em;
  border-radius: 4px;
}
pre {
  background: var(--code-bg);
  padding: 0.9em 1em;
  border-radius: 6px;
  overflow-x: auto;
  border: 1px solid var(--border);
  line-height: 1.45;
}
pre code { background: none; padding: 0; }
/* Syntax highlighting inside a code example — see `Color::Html` in
   src/color.rs, which paints the same five categories the `--dump` CLI
   flags do. Only ever appears inside a highlighted <pre><code>, so the
   selector doesn't need to be more specific than the class itself. */
.tok-kw { color: var(--syn-keyword); }
.tok-type { color: var(--syn-type); }
.tok-str { color: var(--syn-str); }
.tok-lit { color: var(--syn-lit); }
.tok-com { color: var(--muted); font-style: italic; }
.layout {
  display: grid;
  grid-template-columns: 240px 1fr;
  margin: 0 auto;
  min-height: 100vh;
}
.sidebar {
  border-right: 1px solid var(--border);
  padding: 1.5rem 1rem;
  position: sticky;
  top: 0;
  align-self: start;
  height: 100vh;
  overflow-y: auto;
  background: #081a10;
}
.sidebar .brand {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 700;
  font-size: 1.2rem;
  color: #4ade80;
}
.sidebar .brand:hover { text-decoration: none; }
.sidebar .brand-logo {
  width: 60px;
  height: 60px;
  border-radius: 6px;
}
.sidebar h3 {
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #8fbf9c;
  margin: 1.5rem 0 0.5rem;
}
.sidebar ul { list-style: none; margin: 0; padding: 0; }
.sidebar li { margin: 0.1rem 0; }
.sidebar a { color: #d8f3df; }
.sidebar a.current { color: #4ade80; font-weight: 600; }
.content { padding: 2rem 2.5rem; min-width: 0; }
.content.index .module-header { display: none; }
.module-header {
  color: var(--accent);
  border-bottom: 2px solid var(--border);
  padding-bottom: 0.4rem;
  margin-top: 0;
}
.content h2 {
  border-bottom: 1px solid var(--border);
  padding-bottom: 0.3rem;
  margin-top: 2.5rem;
}
.content > section:first-of-type h2 { margin-top: 1rem; }
.module-summary { font-size: 1.1rem; color: var(--muted); }
.content ul { margin: 0.5rem 0; padding-left: 1.5rem; }
.content li { margin: 0.2rem 0; }
.content li > code:first-child { margin-right: 0.4em; }
.functions-summary .fn-row {
  display: flex;
  gap: 1rem;
  align-items: baseline;
  padding: 0.4rem 0;
}
.functions-summary .fn-row.private .fn-sig { color: var(--private); }
.functions-summary .fn-sig code { background: var(--accent-soft); color: var(--accent); }
.functions-summary .fn-summary { color: var(--muted); }
.functions-summary .fn-row.private .fn-sig code { color: var(--private); background: transparent; }
.functions-detail .fn-detail {
  margin: 1.5rem 0 2.5rem;
  padding-bottom: 1.5rem;
  border-bottom: 1px solid var(--border);
}
.functions-detail .fn-detail:last-child { border-bottom: none; }
.fn-title code { background: var(--accent-soft); color: var(--accent); font-size: 1.05em; }
.fn-title .anchor {
  margin-left: 0.5rem;
  color: var(--muted);
  opacity: 0;
  text-decoration: none;
}
.fn-detail:hover .anchor { opacity: 1; }
.badge {
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  padding: 0.1em 0.5em;
  border-radius: 10px;
  vertical-align: middle;
}
.badge.private { background: transparent; color: var(--private); border: 1px solid var(--private); }
dl.params { margin: 0.5rem 0; }
dl.params dt { font-weight: 600; margin-top: 0.5rem; }
dl.params dd { margin: 0 0 0 1rem; color: var(--muted); }
.returns { margin-top: 0.75rem; }
.guide .guide-toc {
  list-style: none;
  padding: 0;
  margin: 1.5rem 0 2.5rem;
  columns: 2;
  column-gap: 2rem;
}
.guide .guide-toc li { padding: 0.2rem 0; break-inside: avoid; }
.guide h2 { color: var(--accent); }
.guide h3 { margin-top: 2rem; }
.guide blockquote {
  margin: 1rem 0;
  padding: 0.1rem 1rem;
  border-left: 3px solid var(--border);
  color: var(--muted);
}
.guide hr { border: none; border-top: 1px solid var(--border); margin: 2rem 0; }
.guide img { max-width: 100%; }
.table-wrap { overflow-x: auto; margin: 1rem 0; }
.table-wrap table { border-collapse: collapse; width: 100%; }
.table-wrap th, .table-wrap td {
  border: 1px solid var(--border);
  padding: 0.4rem 0.7rem;
  text-align: left;
  vertical-align: top;
}
.table-wrap th { background: var(--code-bg); }
.examples .example-toc {
  list-style: none;
  padding: 0;
  margin: 1.5rem 0 2.5rem;
}
.examples .example-toc li {
  padding: 0.35rem 0;
  border-bottom: 1px solid var(--border);
}
.examples .example-toc a { font-weight: 600; }
.examples .example-summary { color: var(--muted); }
.examples .example { scroll-margin-top: 1rem; }
.examples .example > h2 { color: var(--accent); }
.examples .example > p.example-summary { font-size: 1.05rem; }
.examples .example .anchor {
  color: var(--muted);
  opacity: 0;
  text-decoration: none;
  font-size: 0.8em;
}
.examples .example:hover .anchor { opacity: 1; }
.examples .example .program-file {
  margin: 1.75rem 0 0.5rem;
  font-size: 0.85rem;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--muted);
}
.index .module-list {
  list-style: none;
  padding: 0;
  margin: 1.5rem 0;
}
.index .module-list li {
  padding: 0.75rem 0;
  border-bottom: 1px solid var(--border);
}
.index .module-name { font-weight: 600; font-size: 1.1rem; }
.index .module-summary { color: var(--muted); margin: 0.25rem 0 0; }
@media (max-width: 720px) {
  .layout { grid-template-columns: 1fr; }
  .sidebar { position: static; max-height: none; border-right: none; border-bottom: 1px solid var(--border); }
}
"#;
