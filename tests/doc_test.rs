//! `ramos doc` — generates a `docs.json` data file plus a static `index.html`
//! shell that presents it.
//!
//! These tests drive the public `generate` API end-to-end: a real (or
//! synthetic) stdlib directory in, `docs.json` out, then assert on the
//! rendered fragments that matter (signatures, `@param`/`@return`, code
//! blocks, cross-links, lists, and the index) by parsing the JSON and
//! pulling out each page's `body`. The crate ships no JSON dependency (see
//! `src/doc.rs`'s hand-rolled writer), so this file carries a matching
//! hand-rolled reader — just enough to walk the small, fixed shape of
//! `docs.json`.

use std::fs;
use std::path::{Path, PathBuf};

// ── a tiny JSON reader, mirroring the writer in src/doc.rs ──────────────

#[derive(Debug)]
enum Json {
    Null,
    Bool(bool),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn parse(s: &str) -> Json {
        let mut p = JsonParser {
            s: s.as_bytes(),
            pos: 0,
        };
        p.value()
    }

    /// Look up a field on an object; panics on any other shape or a missing key.
    fn get(&self, key: &str) -> &Json {
        match self {
            Json::Object(fields) => fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("missing json field `{key}`")),
            _ => panic!("`{key}`: not an object"),
        }
    }

    fn has(&self, key: &str) -> bool {
        match self {
            Json::Object(fields) => fields.iter().any(|(k, _)| k == key),
            _ => false,
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("not a string: {self:?}"),
        }
    }

    fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(a) => a,
            _ => panic!("not an array: {self:?}"),
        }
    }

    fn as_bool(&self) -> bool {
        match self {
            Json::Bool(b) => *b,
            _ => panic!("not a bool: {self:?}"),
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && (self.s[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Json {
        self.skip_ws();
        match self.s[self.pos] {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Json::Str(self.string()),
            b't' => {
                self.pos += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.pos += 5;
                Json::Bool(false)
            }
            b'n' => {
                self.pos += 4;
                Json::Null
            }
            other => panic!("unexpected byte {other} at {}", self.pos),
        }
    }

    fn object(&mut self) -> Json {
        self.pos += 1; // `{`
        let mut fields = Vec::new();
        self.skip_ws();
        if self.s[self.pos] == b'}' {
            self.pos += 1;
            return Json::Object(fields);
        }
        loop {
            self.skip_ws();
            let key = self.string();
            self.skip_ws();
            assert_eq!(self.s[self.pos], b':');
            self.pos += 1;
            let val = self.value();
            fields.push((key, val));
            self.skip_ws();
            match self.s[self.pos] {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    break;
                }
                other => panic!("bad object at byte {other}"),
            }
        }
        Json::Object(fields)
    }

    fn array(&mut self) -> Json {
        self.pos += 1; // `[`
        let mut items = Vec::new();
        self.skip_ws();
        if self.s[self.pos] == b']' {
            self.pos += 1;
            return Json::Array(items);
        }
        loop {
            items.push(self.value());
            self.skip_ws();
            match self.s[self.pos] {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    break;
                }
                other => panic!("bad array at byte {other}"),
            }
        }
        Json::Array(items)
    }

    fn string(&mut self) -> String {
        self.skip_ws();
        assert_eq!(self.s[self.pos], b'"');
        self.pos += 1;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let c = self.s[self.pos];
            if c == b'"' {
                self.pos += 1;
                break;
            }
            if c == b'\\' {
                self.pos += 1;
                let esc = self.s[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => buf.push(b'"'),
                    b'\\' => buf.push(b'\\'),
                    b'/' => buf.push(b'/'),
                    b'n' => buf.push(b'\n'),
                    b'r' => buf.push(b'\r'),
                    b't' => buf.push(b'\t'),
                    b'u' => {
                        let hex = std::str::from_utf8(&self.s[self.pos..self.pos + 4]).unwrap();
                        let code = u32::from_str_radix(hex, 16).unwrap();
                        self.pos += 4;
                        let mut tmp = [0u8; 4];
                        buf.extend_from_slice(
                            char::from_u32(code)
                                .unwrap()
                                .encode_utf8(&mut tmp)
                                .as_bytes(),
                        );
                    }
                    other => panic!("unsupported escape \\{}", other as char),
                }
            } else {
                buf.push(c);
                self.pos += 1;
            }
        }
        String::from_utf8(buf).expect("valid utf8")
    }
}

/// Load and parse `docs.json` from a generated output directory.
fn load_docs(out: &Path) -> Json {
    let text = fs::read_to_string(out.join("docs.json")).expect("docs.json");
    Json::parse(&text)
}

/// The rendered HTML fragment for the page keyed `id` (a module name, or
/// `""`/`"guide"`/`"examples"`/`"programs"`).
fn page_body(doc: &Json, id: &str) -> String {
    doc.get("pages").get(id).get("body").as_str().to_string()
}

// ── the real stdlib ──────────────────────────────────────────────────────

/// Read the project stdlib dir, regardless of where `cargo test` runs from.
fn stdlib_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("src")
}

/// A fresh temp output dir, cleaned up when the guard drops.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let mut p = std::env::temp_dir();
        // Make it unique per-process to avoid races when tests run in parallel.
        p.push(format!("ramos-doc-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn generates_a_page_per_real_stdlib_module() {
    let out = TempDir::new("real");
    let count = ramos::doc::generate(&stdlib_dir(), &out.0).expect("generate");
    assert_eq!(
        count, 28,
        "expected Kernel, Integer, Float, List, Map, String, Tuple, Struct, Json, Date, \
         NaiveDateTime, TimeZone, DateTime, Time, File, Dir, Socket, ServerSocket, HttpServer, \
         HttpRequest, HttpResponse, Actor, Global, Pool, Config, Thread, Test, Module"
    );

    // The site is one static shell plus the data it presents, not one HTML
    // file per module.
    assert!(out.0.join("index.html").exists());
    assert!(out.0.join("docs.json").exists());
    assert!(out.0.join("assets").join("style.css").exists());
    assert!(
        !out.0.join("Kernel.html").exists(),
        "modules are pages inside docs.json now, not their own files"
    );

    let doc = load_docs(&out.0);
    let modules = doc.get("modules").as_array();
    let module_names: Vec<&str> = modules.iter().map(Json::as_str).collect();

    // `Actor` and `Test` are traits rather than modules, and `Date`,
    // `NaiveDateTime`, `TimeZone` and `DateTime` are structs rather than
    // modules — all document the same way.
    for name in [
        "Kernel",
        "Integer",
        "Float",
        "List",
        "Map",
        "String",
        "Tuple",
        "Struct",
        "Date",
        "NaiveDateTime",
        "TimeZone",
        "DateTime",
        "Time",
        "File",
        "Dir",
        "Socket",
        "ServerSocket",
        "HttpServer",
        "HttpRequest",
        "HttpResponse",
        "Actor",
        "Global",
        "Pool",
        "Config",
        "Thread",
        "Test",
        "Module",
    ] {
        assert!(module_names.contains(&name), "missing module {name}");
        assert!(doc.get("pages").has(name), "missing page for {name}");
    }
}

#[test]
fn module_groups_cover_every_module_exactly_once_and_the_index_shows_separators() {
    // `MODULE_GROUPS` in src/doc.rs is a hand-maintained table, independent
    // of the AST — this is what stops it drifting silently as modules are
    // added, renamed, or removed.
    let out = TempDir::new("groups");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);

    let modules: Vec<&str> = doc
        .get("modules")
        .as_array()
        .iter()
        .map(Json::as_str)
        .collect();
    let groups = doc.get("module_groups").as_array();

    let mut grouped: Vec<&str> = Vec::new();
    for group in groups {
        assert!(
            !group.get("name").as_str().is_empty(),
            "a module group has no name"
        );
        for m in group.get("modules").as_array() {
            let name = m.as_str();
            assert!(
                !grouped.contains(&name),
                "{name} appears in more than one module group"
            );
            grouped.push(name);
        }
    }
    for name in &modules {
        assert!(grouped.contains(name), "{name} is in no module group");
    }
    assert_eq!(
        grouped.len(),
        modules.len(),
        "module groups and the flat module list disagree on membership"
    );

    // The index page renders one heading per group and a separator between
    // them (`.module-group` with a `.separated` sibling from the second
    // group on) — spot-check rather than every group, so this doesn't have
    // to be rewritten each time a module moves between themes.
    let html = page_body(&doc, "");
    assert!(
        html.contains("<h3 class=\"module-group\">Core</h3>"),
        "index missing the Core group heading"
    );
    assert!(
        html.contains("<h3 class=\"module-group\">Networking</h3>"),
        "index missing the Networking group heading"
    );
}

#[test]
fn kernel_page_has_signatures_params_returns_and_examples() {
    let out = TempDir::new("kernel");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "Kernel");

    // Function signature anchors use the Elixir-style name/arity.
    assert!(html.contains(r#"id="print/1""#), "missing print/1 anchor");
    assert!(html.contains(r#"id="at/2""#), "missing at/2 anchor");

    // A `@param` line becomes a `<dl class="params">` with the param name.
    assert!(html.contains("<dl class=\"params\">"));
    assert!(
        html.contains("<dt><code>value</code></dt>"),
        "param `value` not rendered"
    );

    // A `@return` line becomes the returns paragraph.
    assert!(html.contains("class=\"returns\""));
    assert!(html.contains("<strong>Returns:</strong>"));

    // A code example round-trips as an escaped <pre>.
    assert!(
        html.contains("&quot;loading&quot;"),
        "code example not escaped"
    );

    // `@module_doc` summary is on the page.
    assert!(html.contains("the implicit, always-in-scope module"));

    // The module page's own heading is carried as page data, not baked into
    // the body — the shell renders it as the `<h1>`.
    assert_eq!(
        doc.get("pages").get("Kernel").get("heading").as_str(),
        "Kernel"
    );
}

#[test]
fn cross_links_target_known_modules() {
    let out = TempDir::new("xref");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "Kernel");

    // `` `String` `` → a code span that links to the String page's route.
    assert!(
        html.contains("<code><a href=\"#/String\">String</a></code>"),
        "module cross-link not rendered"
    );
    // `` `Kernel.print(x)` `` → links to Kernel's route with a member anchor.
    assert!(
        html.contains("href=\"#/Kernel:print\""),
        "member cross-link not rendered"
    );
}

#[test]
fn bulleted_lists_and_emphasis_render() {
    let out = TempDir::new("lists");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "Kernel");

    // The Kernel module doc has a `- console I/O …` bulleted list.
    assert!(html.contains("<ul>"), "no <ul> rendered");
    assert!(
        html.contains("console I/O"),
        "bulleted list content missing"
    );

    // `*bare*` emphasis becomes <em>bare</em>.
    assert!(html.contains("<em>bare</em>"), "emphasis not rendered");
}

#[test]
fn index_lists_every_module_with_its_summary() {
    let out = TempDir::new("index");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "");

    for name in ["Kernel", "List", "String", "Tuple", "File", "Dir"] {
        assert!(
            html.contains(&format!("href=\"#/{name}\"")),
            "index missing link to {name}"
        );
    }
    // Each module's first doc paragraph is its summary.
    assert!(html.contains("operations on persistent, immutable lists"));
}

// ── a synthetic stdlib (edge cases) ──────────────────────────────────────

/// Write a one-module stdlib into `dir` and return its path.
fn write_module(dir: &PathBuf, file: &str, body: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(file), body).unwrap();
}

#[test]
fn function_without_doc_still_appears() {
    let src = std::env::temp_dir().join(format!("ramos-doc-src-nodoc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&src);
    write_module(
        &src,
        "m.rmo",
        "\
module M
  # @module_doc
  #
  # A tiny module.

  function undocumented(x)
    x
",
    );

    let out = TempDir::new("nodoc");
    let count = ramos::doc::generate(&src, &out.0).expect("generate");
    assert_eq!(count, 1);
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "M");
    // The function still gets a signature + anchor, just no prose.
    assert!(html.contains(r#"id="undocumented/1""#));
    assert!(html.contains("function undocumented(x)"));
    // Module doc is present.
    assert!(html.contains("A tiny module."));

    let _ = fs::remove_dir_all(&src);
}

#[test]
fn private_functions_get_a_badge() {
    let src = std::env::temp_dir().join(format!("ramos-doc-src-priv-{}", std::process::id()));
    let _ = fs::remove_dir_all(&src);
    write_module(
        &src,
        "m.rmo",
        "\
module M
  # @module_doc
  # Summary.

  helper hidden(a, b)
    a
",
    );

    let out = TempDir::new("priv");
    ramos::doc::generate(&src, &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "M");
    assert!(
        html.contains("helper hidden(a, b)"),
        "helper signature wrong"
    );
    assert!(
        html.contains(r#"<span class="badge private">private</span>"#),
        "private badge missing"
    );

    let _ = fs::remove_dir_all(&src);
}

#[test]
fn doc_marker_on_same_line_as_tag_is_kept() {
    let src = std::env::temp_dir().join(format!("ramos-doc-src-inline-{}", std::process::id()));
    let _ = fs::remove_dir_all(&src);
    write_module(
        &src,
        "m.rmo",
        "\
module M
  # @module_doc One-line module summary.

  function f(x)
    # @doc Inline doc.
    # @param x: an arg
    # @return x again
    x
",
    );

    let out = TempDir::new("inline");
    ramos::doc::generate(&src, &out.0).unwrap();
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "M");
    assert!(html.contains("One-line module summary."));
    assert!(html.contains("Inline doc."));
    assert!(html.contains("<dt><code>x</code></dt>"));
    assert!(html.contains("x again"));

    let _ = fs::remove_dir_all(&src);
}

// ── the guide page (README) ──────────────────────────────────────────────

/// The README that backs the `guide` page.
fn readme_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("README.md")
}

/// `generate_with`, spelled once.
fn generate_with(out: &Path, examples: Option<&Path>, readme: Option<&Path>) {
    generate_with_programs(out, examples, None, readme);
}

/// Like `generate_with`, but also naming a programs directory — for the
/// Programs-page tests, which the plain `generate_with` doesn't need to touch.
fn generate_with_programs(
    out: &Path,
    examples: Option<&Path>,
    programs: Option<&Path>,
    readme: Option<&Path>,
) {
    let opts = ramos::doc::Options {
        examples_dir: examples,
        programs_dir: programs,
        readme,
    };
    ramos::doc::generate_with(&stdlib_dir(), out, &opts).unwrap();
}

#[test]
fn guide_page_renders_the_readmes_markdown() {
    let out = TempDir::new("guide");
    generate_with(&out.0, None, Some(&readme_file()));
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "guide");

    // Headings get GitHub-style slugs, so the README's own `#anchor` links
    // still resolve on the rendered page.
    assert!(html.contains("<h3 id=\"pattern-matching\">"));
    assert!(html.contains("href=\"#entrypoints\""));

    // Fenced code becomes an escaped <pre>, tables become tables, and
    // blockquotes survive.
    assert!(html.contains("<pre><code>"));
    assert!(html.contains("<table>"));
    assert!(html.contains("<blockquote>"));

    // Inline markup is converted rather than leaking as text.
    assert!(html.contains("<strong>immutable</strong>"));
    assert!(!html.contains("```"), "fence markers leaked into the HTML");

    // A repo-relative link is rewritten to somewhere that exists off-site.
    assert!(
        html.contains("href=\"https://github.com/andrewaguiar/ramos/src/branch/main/stdlib/\""),
        "relative README link not rewritten"
    );
}

#[test]
fn guide_flag_is_set_and_index_links_to_it() {
    // The sidebar is built client-side by the shell from `docs.json`'s
    // `guides` flags and `modules` list, rather than rendered per page — so
    // "linked from the sidebar" is now a property of that flag, not HTML on
    // every page.
    let out = TempDir::new("guide-links");
    generate_with(&out.0, None, Some(&readme_file()));
    let doc = load_docs(&out.0);

    assert!(doc.get("guides").get("guide").as_bool());
    assert!(page_body(&doc, "").contains("href=\"#/guide\""));
}

#[test]
fn a_missing_readme_drops_the_guide_page() {
    let out = TempDir::new("no-guide");
    let missing = std::env::temp_dir().join("ramos-no-such-readme.md");
    generate_with(&out.0, None, Some(&missing));
    let doc = load_docs(&out.0);

    assert!(!doc.get("guides").get("guide").as_bool());
    assert!(!doc.get("pages").has("guide"));
    assert!(!page_body(&doc, "").contains("#/guide"));
}

// ── the Examples page ────────────────────────────────────────────────────

/// The feature fixtures that back the `examples` page.
fn features_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("features")
}

#[test]
fn examples_page_renders_one_section_per_fixture() {
    let out = TempDir::new("examples");
    generate_with(&out.0, Some(&features_dir()), None);
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "examples");

    let fixtures = fs::read_dir(features_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rmo"))
        .count();
    assert_eq!(
        html.matches("<section class=\"example\"").count(),
        fixtures,
        "every fixture should get a section"
    );

    // The header comment becomes the summary, the code becomes a block, and
    // the `foo.rmo — ` lead-in is peeled off rather than shown.
    assert!(html.contains("id=\"pipe\""));
    assert!(html.contains("<h2>Pipe "));
    assert!(
        !html.contains("pipe.rmo —"),
        "file-name lead-in should be stripped"
    );
    // Fixture code lexes clean, so it comes out syntax-highlighted rather
    // than as one plain-escaped run: `List` (a type), the string literal, and
    // the trailing `# ==` doctest comment each get their own span.
    assert!(
        html.contains("<span class=\"tok-type\">List</span>.join("),
        "fixture code should be syntax-highlighted"
    );
    assert!(
        html.contains("<span class=\"tok-str\">&quot;, &quot;</span>"),
        "the string literal should be escaped and highlighted"
    );
}

#[test]
fn examples_flag_is_set_and_index_links_to_it() {
    let out = TempDir::new("examples-links");
    generate_with(&out.0, Some(&features_dir()), None);
    let doc = load_docs(&out.0);

    assert!(doc.get("guides").get("examples").as_bool());
    assert!(page_body(&doc, "").contains("href=\"#/examples\""));
}

#[test]
fn without_fixtures_there_is_no_examples_page_or_link() {
    let out = TempDir::new("no-examples");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);

    assert!(!doc.get("guides").get("examples").as_bool());
    assert!(!doc.get("pages").has("examples"));
    assert!(!page_body(&doc, "").contains("#/examples"));
}

// ── the Programs page ─────────────────────────────────────────────────────

/// The runnable programs that back the `programs` page — the real
/// `examples/` directory, files and subdirectories alike.
fn programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

/// One section per top-level entry under `examples/`: a `.rmo` file, or a
/// subdirectory holding a multi-file program.
fn program_entry_count(dir: &Path) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() || p.extension().and_then(|s| s.to_str()) == Some("rmo"))
        .count()
}

#[test]
fn programs_page_renders_one_section_per_program() {
    let out = TempDir::new("programs");
    generate_with_programs(&out.0, None, Some(&programs_dir()), None);
    let doc = load_docs(&out.0);
    let html = page_body(&doc, "programs");

    assert_eq!(
        html.matches("<section class=\"example\"").count(),
        program_entry_count(&programs_dir()),
        "every top-level file or directory under examples/ should get a section"
    );

    // The single-file `hello_world.rmo` renders like a feature fixture does.
    assert!(html.contains("id=\"hello_world\""));
    assert!(html.contains("<h2>Hello world "));

    // The multi-file `structs/` program shows each of its files under its own
    // file-name label, the entry file (`main.rmo`, the one with `function main()`)
    // last, and its header comment used as the section summary rather than
    // repeated under its own label.
    assert!(html.contains("id=\"structs\""));
    let structs_start = html.find("id=\"structs\"").unwrap();
    let structs_end = html[structs_start..]
        .find("</section>")
        .map(|i| structs_start + i)
        .unwrap();
    let section = &html[structs_start..structs_end];
    let label = |name: &str| format!("<p class=\"program-file\"><code>{name}</code></p>");
    assert!(section.contains(&label("account.rmo")));
    assert!(section.contains(&label("reportable.rmo")));
    assert!(section.contains(&label("withdrawal_error.rmo")));
    let main_label_pos = section.find(&label("main.rmo")).expect("main.rmo label");
    let account_label_pos = section
        .find(&label("account.rmo"))
        .expect("account.rmo label");
    assert!(
        main_label_pos > account_label_pos,
        "the entry file should read last, after the definitions it uses"
    );
}

#[test]
fn programs_flag_is_set_and_index_links_to_it() {
    let out = TempDir::new("programs-links");
    generate_with_programs(&out.0, None, Some(&programs_dir()), None);
    let doc = load_docs(&out.0);

    assert!(doc.get("guides").get("programs").as_bool());
    assert!(page_body(&doc, "").contains("href=\"#/programs\""));
}

#[test]
fn without_programs_there_is_no_programs_page_or_link() {
    let out = TempDir::new("no-programs");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let doc = load_docs(&out.0);

    assert!(!doc.get("guides").get("programs").as_bool());
    assert!(!doc.get("pages").has("programs"));
    assert!(!page_body(&doc, "").contains("#/programs"));
}

#[test]
fn empty_source_dir_is_an_error() {
    let src = std::env::temp_dir().join(format!("ramos-doc-src-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&src);
    fs::create_dir_all(&src).unwrap();

    let out = TempDir::new("empty");
    let err = ramos::doc::generate(&src, &out.0).unwrap_err();
    assert!(err.contains("no `.rmo` files"), "unexpected error: {err}");

    let _ = fs::remove_dir_all(&src);
}

// ── page metadata (heading / content_class) ──────────────────────────────

#[test]
fn narrative_pages_carry_no_generic_heading() {
    // Only module pages use the shell's generic `<h1 class="module-header">`
    // — the guide/examples/programs/index pages write their own `<h1>` inline
    // in the body, so their JSON `heading` is null.
    let out = TempDir::new("headings");
    generate_with_programs(
        &out.0,
        Some(&features_dir()),
        Some(&programs_dir()),
        Some(&readme_file()),
    );
    let doc = load_docs(&out.0);
    let pages = doc.get("pages");

    for id in ["", "guide", "examples", "programs"] {
        assert!(
            pages.get(id).get("heading").is_null(),
            "{id} heading should be null"
        );
    }
    assert_eq!(pages.get("Kernel").get("heading").as_str(), "Kernel");
}

// ── plain-text summaries (what `ramos test` prints) ───────────────────────────

#[test]
fn summaries_pull_the_module_and_function_docs_as_plain_text() {
    let src = "\
module MathTest
  # @module_doc
  #
  # Arithmetic on the sizes a cart deals in.
  #
  # A second paragraph, which the summary stops before.
  implements Test

  function test_addition()
    # @doc
    # Adding two positives keeps their sign.
    assert(1 + 1 == 2)

  function test_undocumented()
    assert(true)
";
    let docs = ramos::doc::summaries(src);
    assert_eq!(
        docs.module.as_deref(),
        Some("Arithmetic on the sizes a cart deals in.")
    );
    assert_eq!(
        docs.functions.get("test_addition").map(String::as_str),
        Some("Adding two positives keeps their sign.")
    );
    // A function with no `@doc` has no entry, rather than an empty one — the
    // report prints nothing for it.
    assert!(!docs.functions.contains_key("test_undocumented"));
}

#[test]
fn summaries_of_undocumented_source_are_empty() {
    let src = "\
module Bare
  function f()
    1
";
    let docs = ramos::doc::summaries(src);
    assert_eq!(docs.module, None);
    assert!(docs.functions.is_empty());
}
