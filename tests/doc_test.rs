//! `ramos doc` — generates HTML reference for the stdlib.
//!
//! These tests drive the public `generate` API end-to-end: a real (or
//! synthetic) stdlib directory in, HTML files out, then assert on the rendered
//! fragments that matter (signatures, `@param`/`@return`, code blocks,
//! cross-links, lists, and the index).

use std::fs;
use std::path::{Path, PathBuf};

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

// ── the real stdlib ──────────────────────────────────────────────────────

#[test]
fn generates_a_page_per_real_stdlib_module() {
    let out = TempDir::new("real");
    let count = ramos::doc::generate(&stdlib_dir(), &out.0).expect("generate");
    assert_eq!(
        count, 21,
        "expected Kernel, Integer, Float, List, Map, String, Tuple, Struct, Date, \
         NaiveDateTime, TimeZone, DateTime, Time, File, Dir, Actor, Global, Config, Thread, \
         Test, Module"
    );

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
        "Actor",
        "Global",
        "Config",
        "Thread",
        "Test",
        "Module",
    ] {
        let page = out.0.join(format!("{name}.html"));
        assert!(page.exists(), "missing {name}.html");
    }
    assert!(out.0.join("index.html").exists());
    assert!(out.0.join("assets").join("style.css").exists());
}

#[test]
fn kernel_page_has_signatures_params_returns_and_examples() {
    let out = TempDir::new("kernel");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let html = fs::read_to_string(out.0.join("Kernel.html")).unwrap();

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
}

#[test]
fn cross_links_target_known_modules() {
    let out = TempDir::new("xref");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let html = fs::read_to_string(out.0.join("Kernel.html")).unwrap();

    // `` `String` `` → a code span that links to String.html.
    assert!(
        html.contains("<code><a href=\"String.html\">String</a></code>"),
        "module cross-link not rendered"
    );
    // `` `Kernel.print(x)` `` → links to Kernel.html#print.
    assert!(
        html.contains("href=\"Kernel.html#print\""),
        "member cross-link not rendered"
    );
}

#[test]
fn bulleted_lists_and_emphasis_render() {
    let out = TempDir::new("lists");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();
    let html = fs::read_to_string(out.0.join("Kernel.html")).unwrap();

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
    let html = fs::read_to_string(out.0.join("index.html")).unwrap();

    for name in ["Kernel", "List", "String", "Tuple", "File", "Dir"] {
        assert!(
            html.contains(&format!("href=\"{name}.html\"")),
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
    let html = fs::read_to_string(out.0.join("M.html")).unwrap();
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
    let html = fs::read_to_string(out.0.join("M.html")).unwrap();
    assert!(html.contains("helper hidden(a, b)"), "helper signature wrong");
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
    let html = fs::read_to_string(out.0.join("M.html")).unwrap();
    assert!(html.contains("One-line module summary."));
    assert!(html.contains("Inline doc."));
    assert!(html.contains("<dt><code>x</code></dt>"));
    assert!(html.contains("x again"));

    let _ = fs::remove_dir_all(&src);
}

// ── the guide page (README) ──────────────────────────────────────────────

/// The README that backs `guide.html`.
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
    let html = fs::read_to_string(out.0.join("guide.html")).expect("guide.html");

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

    // A repo-relative link is rewritten to somewhere that exists from docs/.
    assert!(
        html.contains("href=\"https://github.com/andrewaguiar/ramos/src/branch/main/stdlib/\""),
        "relative README link not rewritten"
    );
}

#[test]
fn guide_page_is_linked_from_the_sidebar_and_index() {
    let out = TempDir::new("guide-links");
    generate_with(&out.0, None, Some(&readme_file()));

    assert!(fs::read_to_string(out.0.join("List.html"))
        .unwrap()
        .contains("href=\"guide.html\""));
    assert!(fs::read_to_string(out.0.join("index.html"))
        .unwrap()
        .contains("href=\"guide.html\""));
}

#[test]
fn a_missing_readme_drops_the_guide_page() {
    let out = TempDir::new("no-guide");
    let missing = std::env::temp_dir().join("ramos-no-such-readme.md");
    generate_with(&out.0, None, Some(&missing));

    assert!(!out.0.join("guide.html").exists());
    assert!(!fs::read_to_string(out.0.join("index.html"))
        .unwrap()
        .contains("guide.html"));
}

// ── the Examples page ────────────────────────────────────────────────────

/// The feature fixtures that back `examples.html`.
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
    let html = fs::read_to_string(out.0.join("examples.html")).expect("examples.html");

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
fn examples_page_is_linked_from_the_sidebar_and_index() {
    let out = TempDir::new("examples-links");
    generate_with(&out.0, Some(&features_dir()), None);

    let module_page = fs::read_to_string(out.0.join("List.html")).unwrap();
    assert!(module_page.contains("href=\"examples.html\""));

    let index = fs::read_to_string(out.0.join("index.html")).unwrap();
    assert!(index.contains("href=\"examples.html\""));
}

#[test]
fn without_fixtures_there_is_no_examples_page_or_link() {
    let out = TempDir::new("no-examples");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();

    assert!(!out.0.join("examples.html").exists());
    let index = fs::read_to_string(out.0.join("index.html")).unwrap();
    assert!(!index.contains("examples.html"));
}

// ── the Programs page ─────────────────────────────────────────────────────

/// The runnable programs that back `programs.html` — the real `examples/`
/// directory, files and subdirectories alike.
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
    let html = fs::read_to_string(out.0.join("programs.html")).expect("programs.html");

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
fn programs_page_is_linked_from_the_sidebar_and_index() {
    let out = TempDir::new("programs-links");
    generate_with_programs(&out.0, None, Some(&programs_dir()), None);

    let module_page = fs::read_to_string(out.0.join("List.html")).unwrap();
    assert!(module_page.contains("href=\"programs.html\""));

    let index = fs::read_to_string(out.0.join("index.html")).unwrap();
    assert!(index.contains("href=\"programs.html\""));
}

#[test]
fn without_programs_there_is_no_programs_page_or_link() {
    let out = TempDir::new("no-programs");
    ramos::doc::generate(&stdlib_dir(), &out.0).unwrap();

    assert!(!out.0.join("programs.html").exists());
    let index = fs::read_to_string(out.0.join("index.html")).unwrap();
    assert!(!index.contains("programs.html"));
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
