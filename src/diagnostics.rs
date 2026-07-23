use crate::lexer::LexError;
use crate::parser::ParseError;
use crate::span::{line_col, line_text, Span};

/// A short wrong/correct pair of Ramos snippets illustrating a strict rule,
/// shown under a diagnostic so the fix is visible without a trip to the
/// README. Not every error carries one — only violations of a *named* strict
/// rule do; a plain syntax error (an unclosed `)`, a stray token) has no
/// "correct" alternative to contrast it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    pub wrong: &'static str,
    pub correct: &'static str,
}

/// Render a lex error with file, position, and a caret under the offending source.
pub fn render(file: &str, source: &str, err: &LexError) -> String {
    render_parts(
        file,
        source,
        err.code.as_str(),
        &err.message,
        err.span,
        err.code.example(),
    )
}

/// Render a parse error. Parse errors share the single code E0100.
pub fn render_parse(file: &str, source: &str, err: &ParseError) -> String {
    render_parts(file, source, "E0100", &err.message, err.span, err.example)
}

fn render_parts(
    file: &str,
    source: &str,
    code: &str,
    message: &str,
    span: Span,
    example: Option<Example>,
) -> String {
    let (line, col) = line_col(source, span.start);
    let text = line_text(source, line);
    let gutter = format!("{line}");
    let pad = " ".repeat(gutter.len());
    let caret_pad = " ".repeat(col.saturating_sub(1));
    let caret_len = {
        let span_chars = source
            .get(span.start..span.end)
            .map(|s| s.chars().count())
            .unwrap_or(1);
        span_chars.max(1)
    };
    let carets = "^".repeat(caret_len);
    let mut out = format!(
        "error[{code}]: {message}\n{pad}--> {file}:{line}:{col}\n{pad} |\n{gutter} | {text}\n{pad} | {caret_pad}{carets}\n",
    );
    if let Some(Example { wrong, correct }) = example {
        out.push_str(&format!(
            "{pad} |\n{pad} | wrong:\n{}\n{pad} |\n{pad} | correct:\n{}\n",
            indent_block(wrong, &pad),
            indent_block(correct, &pad),
        ));
    }
    out
}

/// Indent every line of a multi-line example snippet under the gutter, so it
/// lines up with the `-->`/`|` furniture above it instead of sitting flush
/// left.
fn indent_block(block: &str, pad: &str) -> String {
    block
        .lines()
        .map(|line| {
            if line.is_empty() {
                format!("{pad} |")
            } else {
                format!("{pad} |   {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
