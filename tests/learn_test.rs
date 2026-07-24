//! `ramos learn` prints a crash course meant for a person or an AI agent to
//! read cold. These tests guard it against drifting out of sync with the
//! language it describes: the one runnable example is actually run, and the
//! text is free of the kind of cosmetic defect (trailing whitespace) that
//! would look sloppy piped into a prompt.

use ramos::interp::run;

#[test]
fn the_complete_program_example_runs_and_matches_its_claimed_output() {
    let src = ramos::learn::example_program();
    let dir = std::env::temp_dir().join(format!("ramos-learn-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let file = dir.join("app.rmo");
    std::fs::write(&file, src).expect("write scratch file");
    // Runs against the real stdlib, exactly like `ramos run` does — the
    // example uses `List.filter`/`List.map`/`Map.get`, which a bare `parse`
    // + `run` (no stdlib merged in) would not resolve.
    let program = ramos::loader::load(&file, None).unwrap_or_else(|e| panic!("load: {e}"));
    let _ = std::fs::remove_dir_all(&dir);

    let buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    run(&program, buf.clone()).expect("run");
    let out = String::from_utf8(std::mem::take(&mut *buf.lock().unwrap())).expect("utf8");
    // The claimed output lives in a trailing `# ...` comment on the example's
    // last line, so this assertion and the printed text can't quietly drift
    // apart from each other.
    // `#{...}` interpolation also contains `#`, so anchor on the trailing
    // `# ` (hash-space) comment marker specifically, from the end.
    let claimed = src
        .rsplit_once("# ")
        .map(|(_, c)| c.trim())
        .expect("the example ends with a `# ...` comment stating its output");
    assert_eq!(out.trim(), claimed);
}

#[test]
fn the_text_has_no_trailing_whitespace() {
    for line in ramos::learn::text().lines() {
        assert_eq!(line, line.trim_end(), "trailing whitespace in: {line:?}");
    }
}

#[test]
fn the_text_covers_every_keyword() {
    let text = ramos::learn::text();
    let words: std::collections::HashSet<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    for keyword in [
        "module",
        "struct",
        "trait",
        "implements",
        "attributes",
        "function",
        "helper",
        "alias",
        "case",
        "if",
        "else",
        "cond",
        "run",
        "do",
        "end",
        "when",
        "and",
        "or",
        "not",
        "true",
        "false",
        "nil",
        "self",
        "_",
    ] {
        assert!(words.contains(keyword), "missing keyword: {keyword}");
    }
}

#[test]
fn the_text_includes_every_lexer_strict_rule_code() {
    let text = ramos::learn::text();
    for code in ramos::lexer::ErrorCode::ALL {
        assert!(text.contains(code.as_str()), "missing {}", code.as_str());
    }
}
