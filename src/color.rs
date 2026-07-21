//! Styling for the debug dumps (`ramos lexer --dump`, `ramos ast --dump`) and
//! for the syntax-highlighted code blocks `ramos doc` embeds in its HTML.
//!
//! Colour is a parameter, never a global: the CLI paints when stdout is a
//! terminal and stays plain when it is piped or redirected, so
//! `ramos ast --dump f.rmo > out.txt` and the test suite both see clean text.
//!
//! The same five styles cover every target — a keyword is the same category
//! in the raw code as the node it produces in the tree below it, and the same
//! category again in a generated doc page.

/// How to render a styled span: ANSI escapes for a terminal, `<span>` tags for
/// `ramos doc`'s HTML, or neither. `Never` is the default so a caller that
/// forgets to think about colour gets plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Never,
    Always,
    /// HTML output: `paint` wraps in `<span class="tok-*">` and HTML-escapes
    /// the text (styled or not), since unlike a terminal a `<`/`&` in the
    /// source would otherwise corrupt the page.
    Html,
}

impl Color {
    /// Paint a terminal, stay plain otherwise. Honours `NO_COLOR`
    /// (<https://no-color.org>), and `FORCE_COLOR` for piping into a pager.
    pub fn for_stdout() -> Color {
        use std::io::IsTerminal;
        if std::env::var_os("NO_COLOR").is_some() {
            Color::Never
        } else if std::env::var_os("FORCE_COLOR").is_some() || std::io::stdout().is_terminal() {
            Color::Always
        } else {
            Color::Never
        }
    }

    pub fn paint(self, style: Style, text: &str) -> String {
        match self {
            Color::Never => text.to_string(),
            Color::Always => match style.ansi_code() {
                Some(code) => format!("\x1b[{code}m{text}\x1b[0m"),
                None => text.to_string(),
            },
            Color::Html => match style.css_class() {
                Some(class) => format!("<span class=\"{class}\">{}</span>", html_escape(text)),
                None => html_escape(text),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Section headings of a dump ("Raw Code:", "AST").
    Heading,
    /// Keywords, and the AST nodes they produce.
    Keyword,
    /// Module, struct and trait names — anything CamelCase.
    Type,
    /// Numbers, symbols, `true`/`false`/`nil`.
    Literal,
    /// String literals.
    Str,
    /// Comments, layout tokens, and the tree's structural labels.
    Dim,
    /// Identifiers, punctuation, operators — the terminal's own foreground.
    Plain,
}

impl Style {
    fn ansi_code(self) -> Option<&'static str> {
        Some(match self {
            Style::Heading => "1",
            Style::Keyword => "35",
            Style::Type => "33",
            Style::Literal => "36",
            Style::Str => "32",
            Style::Dim => "2",
            Style::Plain => return None,
        })
    }

    /// The `docs/assets/style.css` class for this style, or `None` for a span
    /// not worth wrapping (a dump heading never appears in generated HTML;
    /// plain text needs no class, just escaping).
    fn css_class(self) -> Option<&'static str> {
        Some(match self {
            Style::Keyword => "tok-kw",
            Style::Type => "tok-type",
            Style::Literal => "tok-lit",
            Style::Str => "tok-str",
            Style::Dim => "tok-com",
            Style::Heading | Style::Plain => return None,
        })
    }
}

/// Escape the five HTML-significant characters. `ramos doc`'s own copy (in
/// `doc.rs`) is the one everything else there uses; this one exists so
/// `Color::Html` doesn't need a dependency in the other direction.
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
