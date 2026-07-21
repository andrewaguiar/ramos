use crate::lexer::LexError;
use crate::parser::ParseError;
use crate::span::{line_col, line_text, Span};

/// Render a lex error with file, position, and a caret under the offending source.
pub fn render(file: &str, source: &str, err: &LexError) -> String {
    render_parts(file, source, err.code.as_str(), &err.message, err.span)
}

/// Render a parse error. Parse errors share the single code E0100.
pub fn render_parse(file: &str, source: &str, err: &ParseError) -> String {
    render_parts(file, source, "E0100", &err.message, err.span)
}

fn render_parts(file: &str, source: &str, code: &str, message: &str, span: Span) -> String {
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
    format!(
        "error[{code}]: {message}\n{pad}--> {file}:{line}:{col}\n{pad} |\n{gutter} | {text}\n{pad} | {caret_pad}{carets}\n",
    )
}
