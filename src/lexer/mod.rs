//! Ramos lexer: source text → token stream with INDENT/DEDENT/NEWLINE layout
//! tokens (Python-style indent stack) and the lexical strict rules enforced.
//!
//! Layout rules:
//! - blank lines and comment-only lines emit nothing and don't affect indent
//! - inside `(` `[` `{` brackets, newlines are plain whitespace (implicit
//!   line joining), so multi-line literals and argument lists work
//! - string interpolation `#{...}` is lexed eagerly: the string token carries
//!   the sub-token streams of its interpolations

mod rules;

pub use rules::ErrorCode;
use rules::{valid_lower_ident, valid_upper_ident};

use crate::color::{Color, Style};
use crate::span::Span;

#[derive(Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Interp(Vec<Token>),
}

// Interpolations print only their token kinds — spans are noise in dumps.
impl std::fmt::Debug for StrPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrPart::Lit(s) => write!(f, "Lit({s:?})"),
            StrPart::Interp(toks) => {
                write!(f, "Interp(")?;
                f.debug_list()
                    .entries(toks.iter().map(|t| &t.kind))
                    .finish()?;
                write!(f, ")")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // layout
    Newline,
    Indent,
    Dedent,
    Eof,
    // literals
    Int(i64),
    Float(f64),
    Str(Vec<StrPart>),
    /// `D"..."` / `T"..."` / `N"..."` / `U"..."` — a fixed-letter sigil hugging
    /// a string with no space, no interpolation. The letter names the struct
    /// the parser desugars the call to (`Date`, `Time`, `NaiveDateTime`,
    /// `DateTime`); the string is its literal, already escape-processed text.
    Sigil(char, String),
    Symbol(String),
    // identifiers
    Ident(String),
    UpperIdent(String),
    Underscore,
    // keywords
    Module,
    Struct,
    Trait,
    Implements,
    Attributes,
    Function,
    Helper,
    Case,
    Cond,
    If,
    Else,
    Run,
    Do,
    /// Purely decorative: an optional block-closing marker with no semantic
    /// effect (indentation alone determines block structure). Stripped from
    /// the token stream by `lex` before the parser ever sees it — see
    /// `strip_end_markers`.
    End,
    Alias,
    As,
    SelfKw,
    True,
    False,
    Nil,
    When,
    And,
    Or,
    Not,
    // punctuation
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    Arrow,  // ->
    Assign, // =
    // operators
    EqEq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar, // **
    Concat,   // <>
    PlusPlus, // ++
    Pipe,     // |
}

impl TokenKind {
    /// Does this token end a value expression? Decides whether a following
    /// `-` is binary (spacing enforced) or unary (may hug its operand).
    fn ends_value(&self) -> bool {
        matches!(
            self,
            TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Str(_)
                | TokenKind::Sigil(_, _)
                | TokenKind::Symbol(_)
                | TokenKind::Ident(_)
                | TokenKind::UpperIdent(_)
                | TokenKind::Underscore
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Nil
                | TokenKind::SelfKw
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// 1-based source line the token starts on. Tracked independently of
    /// `Newline` tokens, which are suppressed inside `(` `[` `{` — so this is
    /// the only way to tell whether two tokens sit on the same physical line
    /// once bracket depth is nonzero (see the call-argument layout checks).
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub code: ErrorCode,
    pub message: String,
    pub span: Span,
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    strip_end_markers(Lexer::new(source).lex_all()?)
}

/// `end` carries no meaning at all — indentation alone determines block
/// structure, so an `end` line is a no-op the parser never needs to know
/// about. This drops every `end` token (and the newline right after it)
/// before the parser sees the stream, so nothing downstream has to special-
/// case it.
///
/// Requires each `end` to be alone on its line — the token immediately before
/// it (skipping any Indent/Dedent, which carry no content of their own) is a
/// `Newline` or nothing at all, and the token immediately after it is a
/// `Newline` or `Eof`. Anything else (`x = end`, `end()`, two on one line) is
/// almost certainly a mistake — `end` was never meant to be a value — so it
/// is a hard error rather than something silently swallowed wrong.
fn strip_end_markers(tokens: Vec<Token>) -> Result<Vec<Token>, LexError> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if tok.kind != TokenKind::End {
            out.push(tok.clone());
            i += 1;
            continue;
        }
        let alone_before = out
            .iter()
            .rev()
            .find(|t| !matches!(t.kind, TokenKind::Indent | TokenKind::Dedent))
            .is_none_or(|t| t.kind == TokenKind::Newline);
        let alone_after = matches!(
            tokens.get(i + 1).map(|t| &t.kind),
            Some(TokenKind::Newline) | Some(TokenKind::Eof)
        );
        if !alone_before || !alone_after {
            return Err(LexError {
                code: ErrorCode::StrayEnd,
                message: "`end` must be alone on its line — it is a purely \
                          decorative marker for where an indented block \
                          already ends, never a value"
                    .to_string(),
                span: tok.span,
            });
        }
        // Drop `end` itself, and the newline right after it if there is one
        // (there may not be, at EOF with no trailing newline).
        i += if matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokenKind::Newline)) {
            2
        } else {
            1
        };
    }
    Ok(out)
}

/// Debug rendering of a lex result: a fenced "Raw Code:" block with the
/// source, then a fenced "Lexer tokens" block listing one token per line.
/// Used by `ramos lexer --dump` and by test failure messages.
pub fn dump(source: &str, tokens: &[Token], color: Color) -> String {
    let mut out = color.paint(Style::Heading, "Raw Code:");
    out.push_str("\n```\n");
    out.push_str(&highlight(source, tokens, color));
    if !source.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");
    out.push_str(&color.paint(Style::Heading, "Lexer tokens"));
    out.push_str("\n```\n[\n");
    for tok in tokens {
        let text = format!("{:?}", tok.kind);
        out.push_str(&format!(
            "  {},\n",
            color.paint(list_style(&tok.kind), &text)
        ));
    }
    out.push_str("]\n```\n");
    out
}

/// The token list on its own, one per line, without the dump's blocks.
pub fn render(tokens: &[Token], color: Color) -> String {
    let mut out = String::new();
    for tok in tokens {
        let text = format!("{:?}", tok.kind);
        out.push_str(&color.paint(list_style(&tok.kind), &text));
        out.push('\n');
    }
    out
}

/// Re-render `source` with every token painted by category. The text *between*
/// tokens is whitespace and comments — the lexer emits no tokens for either —
/// so it is passed through, with comments dimmed.
///
/// Layout tokens are skipped: their spans cover indentation and newlines,
/// where a colour would be invisible anyway.
pub fn highlight(source: &str, tokens: &[Token], color: Color) -> String {
    if color == Color::Never {
        return source.to_string();
    }
    let mut out = String::new();
    let mut pos = 0;
    for tok in tokens {
        let Some(style) = source_style(&tok.kind) else {
            continue;
        };
        let (start, end) = (tok.span.start, tok.span.end);
        // Guard the slicing: spans from a mismatched source, and the repeated
        // spans a run of Dedents shares, must not panic or backtrack.
        if start < pos || end > source.len() || start >= end {
            continue;
        }
        out.push_str(&gap(&source[pos..start], color));
        out.push_str(&color.paint(style, &source[start..end]));
        pos = end;
    }
    out.push_str(&gap(&source[pos..], color));
    out
}

/// Text between two tokens: whitespace, plus any comment. A `#` here always
/// opens a comment — one inside a string lives in that string's token.
///
/// Routed through `color.paint(Style::Plain, _)` rather than pushed raw: for
/// `Color::Html` that is what HTML-escapes it (whitespace never needs it, but
/// a comment's own text might contain `<`/`&`); for the ANSI targets `Plain`
/// is a no-op, so this changes nothing there.
fn gap(text: &str, color: Color) -> String {
    if !text.contains('#') {
        return color.paint(Style::Plain, text);
    }
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match line.find('#') {
            Some(at) => {
                out.push_str(&color.paint(Style::Plain, &line[..at]));
                out.push_str(&color.paint(Style::Dim, &line[at..]));
            }
            None => out.push_str(&color.paint(Style::Plain, line)),
        }
    }
    out
}

/// How a token is painted in the raw code; `None` for the layout tokens, which
/// the highlighter passes over.
fn source_style(kind: &TokenKind) -> Option<Style> {
    use TokenKind as T;
    Some(match kind {
        T::Newline | T::Indent | T::Dedent | T::Eof => return None,
        T::Str(_) | T::Sigil(_, _) => Style::Str,
        T::Int(_) | T::Float(_) | T::Symbol(_) | T::True | T::False | T::Nil => Style::Literal,
        T::UpperIdent(_) => Style::Type,
        T::Module
        | T::Struct
        | T::Trait
        | T::Implements
        | T::Attributes
        | T::Function
        | T::Helper
        | T::Case
        | T::Cond
        | T::If
        | T::Else
        | T::Run
        | T::Do
        | T::End
        | T::Alias
        | T::As
        | T::SelfKw
        | T::When
        | T::And
        | T::Or
        | T::Not => Style::Keyword,
        _ => Style::Plain,
    })
}

/// How a token is painted in the token list, where layout tokens are the
/// structure and so read best dimmed.
fn list_style(kind: &TokenKind) -> Style {
    use TokenKind as T;
    match kind {
        T::Newline | T::Indent | T::Dedent | T::Eof => Style::Dim,
        other => source_style(other).unwrap_or(Style::Plain),
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    indent_stack: Vec<usize>,
    bracket_depth: usize,
    /// 1-based line the lexer's cursor is currently on. Advanced on every
    /// `\n` consumed, including inside brackets (where `Newline` tokens are
    /// suppressed) and inside multiline strings.
    line: usize,
    at_line_start: bool,
    /// Last real (non-layout) token, used for unary-minus and map-key `:` decisions.
    prev_kind: Option<TokenKind>,
    /// Whitespace (or line start) immediately before the next token.
    had_space: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            tokens: Vec::new(),
            indent_stack: vec![0],
            bracket_depth: 0,
            line: 1,
            at_line_start: true,
            prev_kind: None,
            had_space: true,
        }
    }

    fn lex_all(mut self) -> Result<Vec<Token>, LexError> {
        loop {
            if self.at_line_start && self.bracket_depth == 0 && !self.handle_line_start()? {
                break; // EOF
            }
            self.skip_inline_ws();
            match self.peek() {
                None => break,
                Some(b'\n') => {
                    let at = self.pos;
                    self.pos += 1;
                    self.line += 1;
                    if self.bracket_depth == 0 {
                        self.push(TokenKind::Newline, Span::new(at, at + 1));
                        self.at_line_start = true;
                        self.prev_kind = None;
                        self.had_space = true;
                    } else {
                        self.had_space = true;
                    }
                }
                Some(b'#') => self.skip_comment(),
                Some(_) => {
                    let tok = self.next_token()?;
                    self.check_block_starts_on_its_own_line(&tok)?;
                    match tok.kind {
                        TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                            self.bracket_depth += 1
                        }
                        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                            self.bracket_depth = self.bracket_depth.saturating_sub(1)
                        }
                        _ => {}
                    }
                    self.prev_kind = Some(tok.kind.clone());
                    self.had_space = false;
                    self.tokens.push(tok);
                }
            }
        }
        // EOF: terminate the last logical line, close all open blocks.
        let end = Span::new(self.src.len(), self.src.len());
        if self
            .tokens
            .last()
            .is_some_and(|t| t.kind != TokenKind::Newline)
        {
            self.push(TokenKind::Newline, end);
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.push(TokenKind::Dedent, end);
        }
        self.push(TokenKind::Eof, end);
        Ok(self.tokens)
    }

    /// Consume blank/comment lines, then measure indentation of the next code
    /// line and emit Indent/Dedent. Returns false at EOF.
    fn handle_line_start(&mut self) -> Result<bool, LexError> {
        loop {
            let line_begin = self.pos;
            let mut width = 0usize;
            loop {
                match self.peek() {
                    Some(b' ') => {
                        width += 1;
                        self.pos += 1;
                    }
                    Some(b'\t') => {
                        return Err(self.err(
                            ErrorCode::TabInIndentation,
                            "tabs are not allowed in indentation; use exactly 2 spaces per level",
                            Span::new(self.pos, self.pos + 1),
                        ));
                    }
                    _ => break,
                }
            }
            match self.peek() {
                None => return Ok(false),
                Some(b'\n') => {
                    self.pos += 1;
                    self.line += 1;
                    continue; // blank line
                }
                Some(b'\r') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'#') => {
                    self.skip_comment();
                    continue; // comment-only line
                }
                Some(_) => {
                    let span = Span::new(line_begin, self.pos);
                    if !width.is_multiple_of(2) {
                        return Err(self.err(
                            ErrorCode::BadIndentation,
                            format!("indentation must be a multiple of 2 spaces (found {width})"),
                            span,
                        ));
                    }
                    let cur = *self.indent_stack.last().unwrap();
                    if width > cur {
                        if width != cur + 2 {
                            return Err(self.err(
                                ErrorCode::BadIndentation,
                                format!(
                                    "indentation may only increase one level (2 spaces) at a time (jumped from {cur} to {width})"
                                ),
                                span,
                            ));
                        }
                        self.indent_stack.push(width);
                        self.push(TokenKind::Indent, span);
                    } else if width < cur {
                        while *self.indent_stack.last().unwrap() > width {
                            self.indent_stack.pop();
                            self.push(TokenKind::Dedent, span);
                        }
                        if *self.indent_stack.last().unwrap() != width {
                            return Err(self.err(
                                ErrorCode::BadIndentation,
                                format!("dedent to {width} spaces matches no enclosing indentation level"),
                                span,
                            ));
                        }
                    }
                    self.at_line_start = false;
                    self.prev_kind = None;
                    self.had_space = true;
                    return Ok(true);
                }
            }
        }
    }

    /// Lex exactly one non-layout token starting at a non-space, non-newline,
    /// non-comment byte. Shared by the main loop and interpolation lexing.
    fn next_token(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let start_line = self.line;
        let kind = match self.bytes[self.pos] {
            b'"' => {
                if self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') {
                    self.lex_multiline_string()?
                } else {
                    self.lex_string()?
                }
            }
            b'0'..=b'9' => self.lex_number()?,
            b'a'..=b'z' | b'_' => self.lex_lower()?,
            // A single uppercase letter hugging a `"` (no space) is a sigil,
            // not the start of a longer `UpperIdent` — `lex_upper` would stop
            // scanning right there anyway, so this is unambiguous.
            b'A'..=b'Z' if self.peek_at(1) == Some(b'"') => self.lex_sigil()?,
            b'A'..=b'Z' => self.lex_upper()?,
            b':' => self.lex_colon_or_symbol()?,
            b'(' => {
                self.pos += 1;
                TokenKind::LParen
            }
            b')' => {
                self.pos += 1;
                TokenKind::RParen
            }
            b'[' => {
                self.pos += 1;
                TokenKind::LBracket
            }
            b']' => {
                self.pos += 1;
                TokenKind::RBracket
            }
            b'{' => {
                self.pos += 1;
                TokenKind::LBrace
            }
            b'}' => {
                self.pos += 1;
                TokenKind::RBrace
            }
            b'.' => {
                self.pos += 1;
                // Field access. Unlike the binary operators, it needs no
                // surrounding whitespace, so `andrew.name` lexes as written.
                TokenKind::Dot
            }
            b',' => {
                self.pos += 1;
                if !matches!(self.peek(), Some(b' ') | Some(b'\n') | Some(b'\r')) {
                    return Err(self.err(
                        ErrorCode::NoSpaceAfterComma,
                        "missing whitespace after `,`",
                        Span::new(start, start + 1),
                    ));
                }
                TokenKind::Comma
            }
            b'=' => {
                if self.peek_at(1) == Some(b'=') {
                    self.spaced_op(2, TokenKind::EqEq, "==")?
                } else {
                    self.spaced_op(1, TokenKind::Assign, "=")?
                }
            }
            b'!' => {
                if self.peek_at(1) == Some(b'=') {
                    self.spaced_op(2, TokenKind::NotEq, "!=")?
                } else {
                    return Err(self.err(
                        ErrorCode::UnexpectedChar,
                        "unexpected `!`; Ramos uses the word `not` for logical negation",
                        Span::new(start, start + 1),
                    ));
                }
            }
            b'<' => match self.peek_at(1) {
                Some(b'>') => self.spaced_op(2, TokenKind::Concat, "<>")?,
                Some(b'=') => self.spaced_op(2, TokenKind::Le, "<=")?,
                _ => self.spaced_op(1, TokenKind::Lt, "<")?,
            },
            b'>' => match self.peek_at(1) {
                Some(b'=') => self.spaced_op(2, TokenKind::Ge, ">=")?,
                _ => self.spaced_op(1, TokenKind::Gt, ">")?,
            },
            b'+' => {
                if self.peek_at(1) == Some(b'+') {
                    self.spaced_op(2, TokenKind::PlusPlus, "++")?
                } else {
                    self.spaced_op(1, TokenKind::Plus, "+")?
                }
            }
            b'-' => {
                if self.peek_at(1) == Some(b'>') {
                    self.spaced_op(2, TokenKind::Arrow, "->")?
                } else if self.prev_kind.as_ref().is_some_and(|k| k.ends_value()) {
                    self.spaced_op(1, TokenKind::Minus, "-")?
                } else {
                    // unary minus may hug its operand: -5, f(-1)
                    self.pos += 1;
                    TokenKind::Minus
                }
            }
            b'*' => {
                if self.peek_at(1) == Some(b'*') {
                    self.spaced_op(2, TokenKind::StarStar, "**")?
                } else {
                    self.spaced_op(1, TokenKind::Star, "*")?
                }
            }
            b'/' => self.spaced_op(1, TokenKind::Slash, "/")?,
            b'%' => self.spaced_op(1, TokenKind::Percent, "%")?,
            b'&' => {
                return Err(self.err(
                    ErrorCode::UnexpectedChar,
                    if self.peek_at(1) == Some(b'&') {
                        "unexpected `&&`; Ramos uses the word `and` for logical conjunction"
                    } else {
                        "unexpected `&`; Ramos uses the word `and` for logical conjunction"
                    },
                    Span::new(
                        start,
                        start + if self.peek_at(1) == Some(b'&') { 2 } else { 1 },
                    ),
                ));
            }
            b'|' => {
                // `|` is the pipe operator, so `||` would otherwise lex as two
                // pipes and report a spacing error — misleading enough that a
                // user would try `x | | y`. Name the replacement instead.
                if self.peek_at(1) == Some(b'|') {
                    return Err(self.err(
                        ErrorCode::UnexpectedChar,
                        "unexpected `||`; Ramos uses the word `or` for logical disjunction",
                        Span::new(start, start + 2),
                    ));
                }
                self.spaced_op(1, TokenKind::Pipe, "|")?
            }
            other => {
                return Err(self.err(
                    ErrorCode::UnexpectedChar,
                    format!("unexpected character `{}`", other as char),
                    Span::new(start, start + 1),
                ));
            }
        };
        Ok(Token {
            kind,
            span: Span::new(start, self.pos),
            line: start_line,
        })
    }

    /// Strict rule: binary operators require whitespace on both sides.
    /// A newline after the operator counts (block arms end lines with `->`).
    fn spaced_op(&mut self, len: usize, kind: TokenKind, sym: &str) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let after_ok = matches!(
            self.peek_at(len),
            Some(b' ') | Some(b'\n') | Some(b'\r') | None
        );
        if !self.had_space || !after_ok {
            return Err(self.err(
                ErrorCode::NoSpaceAroundOperator,
                format!("missing whitespace around `{sym}`"),
                Span::new(start, start + len),
            ));
        }
        self.pos += len;
        Ok(kind)
    }

    fn lex_number(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.peek_at(1), Some(b'0'..=b'9')) {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let raw = &self.src[start..self.pos];
        if matches!(
            self.peek(),
            Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'_')
        ) {
            return Err(self.err(
                ErrorCode::InvalidNumber,
                format!("invalid number literal starting with `{raw}`"),
                Span::new(start, self.pos + 1),
            ));
        }
        if is_float {
            Ok(TokenKind::Float(raw.parse().unwrap()))
        } else {
            raw.parse().map(TokenKind::Int).map_err(|_| {
                self.err(
                    ErrorCode::InvalidNumber,
                    format!("integer literal `{raw}` is out of range"),
                    Span::new(start, self.pos),
                )
            })
        }
    }

    fn lex_lower(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'0'..=b'9') | Some(b'_')
        ) {
            self.pos += 1;
        }
        // Fold a trailing `?`/`!` into the identifier for a precise error —
        // unless it's the start of `!=`, which is the operator's problem.
        while matches!(self.peek(), Some(b'?'))
            || (self.peek() == Some(b'!') && self.peek_at(1) != Some(b'='))
        {
            self.pos += 1;
        }
        let raw = &self.src[start..self.pos];
        if !valid_lower_ident(raw) {
            return Err(self.err(
                ErrorCode::InvalidIdentifier,
                format!("invalid identifier `{raw}`: only `a-z` and `_` are allowed (no digits, `?` or `!`)"),
                Span::new(start, self.pos),
            ));
        }
        Ok(match raw {
            "module" => TokenKind::Module,
            "struct" => TokenKind::Struct,
            "trait" => TokenKind::Trait,
            "implements" => TokenKind::Implements,
            "attributes" => TokenKind::Attributes,
            "function" => TokenKind::Function,
            "helper" => TokenKind::Helper,
            "case" => TokenKind::Case,
            "cond" => TokenKind::Cond,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "run" => TokenKind::Run,
            "do" => TokenKind::Do,
            "end" => TokenKind::End,
            "alias" => TokenKind::Alias,
            "as" => TokenKind::As,
            "self" => TokenKind::SelfKw,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "nil" => TokenKind::Nil,
            "when" => TokenKind::When,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "_" => TokenKind::Underscore,
            _ => TokenKind::Ident(raw.to_string()),
        })
    }

    fn lex_upper(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'0'..=b'9') | Some(b'_')
        ) {
            self.pos += 1;
        }
        let raw = &self.src[start..self.pos];
        if !valid_upper_ident(raw) {
            return Err(self.err(
                ErrorCode::InvalidModuleName,
                format!("invalid module name `{raw}`: module names are CamelCase and may only use letters"),
                Span::new(start, self.pos),
            ));
        }
        Ok(TokenKind::UpperIdent(raw.to_string()))
    }

    /// A sigil: one letter from the fixed set, immediately followed by a
    /// string with no interpolation — `N"2024-01-01 00:00:00"`. The letter is
    /// checked here, against the same fixed set the parser desugars: `D`
    /// (`Date`), `T` (`Time`), `N` (`NaiveDateTime`), `U` (`DateTime`).
    fn lex_sigil(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let letter = self.bytes[self.pos] as char;
        if !matches!(letter, 'D' | 'T' | 'N' | 'U') {
            return Err(self.err(
                ErrorCode::UnknownSigil,
                format!(
                    "unknown sigil `{letter}\"...\"` — sigils are `D` (Date), `T` (Time), \
                     `N` (NaiveDateTime), `U` (DateTime)"
                ),
                Span::new(start, start + 1),
            ));
        }
        self.pos += 1;
        let text = self.lex_sigil_body()?;
        Ok(TokenKind::Sigil(letter, text))
    }

    /// The quoted text of a sigil: the same escapes as an ordinary string
    /// (`\n`, `\t`, `\"`, `\\`), but no `#{...}` interpolation and no
    /// multi-line form — a sigil is always a single literal.
    fn lex_sigil_body(&mut self) -> Result<String, LexError> {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut text = String::new();
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.err(
                    ErrorCode::UnterminatedString,
                    "unterminated sigil literal",
                    Span::new(start, self.pos),
                ));
            };
            match ch {
                '\n' => {
                    return Err(self.err(
                        ErrorCode::UnterminatedString,
                        "unterminated sigil literal (sigils may not span lines)",
                        Span::new(start, self.pos),
                    ));
                }
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\\' => {
                    let esc_at = self.pos;
                    self.pos += 1;
                    let Some(esc) = self.peek_char() else {
                        return Err(self.err(
                            ErrorCode::UnterminatedString,
                            "unterminated sigil literal",
                            Span::new(start, self.pos),
                        ));
                    };
                    let replacement = match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '"' => '"',
                        '\\' => '\\',
                        other => {
                            return Err(self.err(
                                ErrorCode::InvalidEscape,
                                format!("invalid escape sequence `\\{other}`"),
                                Span::new(esc_at, self.pos + other.len_utf8()),
                            ));
                        }
                    };
                    text.push(replacement);
                    self.pos += esc.len_utf8();
                }
                other => {
                    text.push(other);
                    self.pos += other.len_utf8();
                }
            }
        }
        Ok(text)
    }

    /// `:` is a map-key separator when it hugs a preceding identifier
    /// (`{name: ...}`), otherwise `:foo` is a symbol literal.
    fn lex_colon_or_symbol(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let prev_is_key = !self.had_space && matches!(self.prev_kind, Some(TokenKind::Ident(_)));
        if !prev_is_key && matches!(self.peek_at(1), Some(b'a'..=b'z') | Some(b'_')) {
            self.pos += 1;
            let name_start = self.pos;
            while matches!(
                self.peek(),
                Some(b'a'..=b'z')
                    | Some(b'A'..=b'Z')
                    | Some(b'0'..=b'9')
                    | Some(b'_')
                    | Some(b'?')
                    | Some(b'!')
            ) {
                self.pos += 1;
            }
            let raw = &self.src[name_start..self.pos];
            if !valid_lower_ident(raw) {
                return Err(self.err(
                    ErrorCode::InvalidSymbol,
                    format!("invalid symbol `:{raw}`: only `a-z` and `_` are allowed"),
                    Span::new(start, self.pos),
                ));
            }
            Ok(TokenKind::Symbol(raw.to_string()))
        } else {
            self.pos += 1;
            Ok(TokenKind::Colon)
        }
    }

    fn lex_string(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut parts: Vec<StrPart> = Vec::new();
        let mut cur = String::new();
        loop {
            let Some(ch) = self.peek_char() else {
                return Err(self.err(
                    ErrorCode::UnterminatedString,
                    "unterminated string literal",
                    Span::new(start, self.pos),
                ));
            };
            match ch {
                '\n' => {
                    return Err(self.err(
                        ErrorCode::UnterminatedString,
                        "unterminated string literal (strings may not span lines)",
                        Span::new(start, self.pos),
                    ));
                }
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\\' => {
                    let esc_at = self.pos;
                    self.pos += 1;
                    let Some(esc) = self.peek_char() else {
                        return Err(self.err(
                            ErrorCode::UnterminatedString,
                            "unterminated string literal",
                            Span::new(start, self.pos),
                        ));
                    };
                    let replacement = match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '"' => '"',
                        '\\' => '\\',
                        '#' => '#', // \#{ suppresses interpolation
                        other => {
                            return Err(self.err(
                                ErrorCode::InvalidEscape,
                                format!("invalid escape sequence `\\{other}`"),
                                Span::new(esc_at, self.pos + other.len_utf8()),
                            ));
                        }
                    };
                    cur.push(replacement);
                    self.pos += esc.len_utf8();
                }
                '#' if self.peek_at(1) == Some(b'{') => {
                    if !cur.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut cur)));
                    }
                    self.pos += 2;
                    let toks = self.lex_interp(start)?;
                    parts.push(StrPart::Interp(toks));
                }
                other => {
                    cur.push(other);
                    self.pos += other.len_utf8();
                }
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(cur));
        }
        Ok(TokenKind::Str(parts))
    }

    /// Multiline string, positioned at the first of three quotes.
    ///
    /// Strict shape (E0013 on violation):
    /// - the opening `"""` is immediately followed by a newline
    /// - content lines are indented one level (2 spaces) past the line that
    ///   opened the string; that prefix is stripped, deeper spaces are content
    /// - blank lines need no indentation and contribute a bare `\n`
    /// - the closing `"""` stands alone on its line, at the same indentation
    ///   as the opening line
    /// - it never opens on the tail of an assignment (E0014, the same rule
    ///   `case` / `cond` / `if` / `run` follow)
    ///
    /// Every content line contributes its trailing `\n`, so the value always
    /// ends with a newline. Escapes and `#{...}` interpolation work as in
    /// single-line strings.
    fn lex_multiline_string(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        self.check_multiline_string_starts_on_its_own_line(start)?;
        let open_indent = {
            let line_start = self.src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
            self.bytes[line_start..]
                .iter()
                .take_while(|b| **b == b' ')
                .count()
        };
        let required = open_indent + 2;
        self.pos += 3;
        if !matches!(self.peek(), Some(b'\n')) {
            return Err(self.err(
                ErrorCode::BadMultilineString,
                "opening `\"\"\"` must be immediately followed by a newline",
                Span::new(start, self.pos),
            ));
        }
        self.pos += 1;
        let mut parts: Vec<StrPart> = Vec::new();
        let mut cur = String::new();
        loop {
            // measure this line's indentation
            let mut width = 0usize;
            loop {
                match self.peek() {
                    Some(b' ') => {
                        width += 1;
                        self.pos += 1;
                    }
                    Some(b'\t') => {
                        return Err(self.err(
                            ErrorCode::TabInIndentation,
                            "tabs are not allowed in indentation; use exactly 2 spaces per level",
                            Span::new(self.pos, self.pos + 1),
                        ));
                    }
                    _ => break,
                }
            }
            match self.peek() {
                None => {
                    return Err(self.err(
                        ErrorCode::UnterminatedString,
                        "unterminated multiline string (missing closing `\"\"\"`)",
                        Span::new(start, self.pos),
                    ));
                }
                Some(b'\n') => {
                    // blank line: contributes a bare newline, indent not required
                    cur.push('\n');
                    self.pos += 1;
                    self.line += 1;
                    continue;
                }
                _ => {}
            }
            // closing delimiter?
            if width == open_indent
                && self.peek() == Some(b'"')
                && self.peek_at(1) == Some(b'"')
                && self.peek_at(2) == Some(b'"')
            {
                let close_at = self.pos;
                self.pos += 3;
                if !matches!(self.peek(), None | Some(b'\n') | Some(b'\r')) {
                    return Err(self.err(
                        ErrorCode::BadMultilineString,
                        "closing `\"\"\"` must stand alone on its line",
                        Span::new(close_at, self.pos),
                    ));
                }
                break;
            }
            if width < required {
                return Err(self.err(
                    ErrorCode::BadMultilineString,
                    format!(
                        "multiline string content must be indented {required} spaces (one level past the opening line); found {width}"
                    ),
                    Span::new(self.pos, self.pos + 1),
                ));
            }
            // spaces beyond the required prefix are content
            for _ in required..width {
                cur.push(' ');
            }
            // content of this line, with escapes and interpolation
            loop {
                let Some(ch) = self.peek_char() else {
                    return Err(self.err(
                        ErrorCode::UnterminatedString,
                        "unterminated multiline string (missing closing `\"\"\"`)",
                        Span::new(start, self.pos),
                    ));
                };
                match ch {
                    '\n' => {
                        cur.push('\n');
                        self.pos += 1;
                        self.line += 1;
                        break;
                    }
                    '\\' => {
                        let esc_at = self.pos;
                        self.pos += 1;
                        let Some(esc) = self.peek_char() else {
                            return Err(self.err(
                                ErrorCode::UnterminatedString,
                                "unterminated multiline string (missing closing `\"\"\"`)",
                                Span::new(start, self.pos),
                            ));
                        };
                        let replacement = match esc {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '"' => '"',
                            '\\' => '\\',
                            '#' => '#',
                            other => {
                                return Err(self.err(
                                    ErrorCode::InvalidEscape,
                                    format!("invalid escape sequence `\\{other}`"),
                                    Span::new(esc_at, self.pos + other.len_utf8()),
                                ));
                            }
                        };
                        cur.push(replacement);
                        self.pos += esc.len_utf8();
                    }
                    '#' if self.peek_at(1) == Some(b'{') => {
                        if !cur.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut cur)));
                        }
                        self.pos += 2;
                        let toks = self.lex_interp(start)?;
                        parts.push(StrPart::Interp(toks));
                    }
                    other => {
                        cur.push(other);
                        self.pos += other.len_utf8();
                    }
                }
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(cur));
        }
        Ok(TokenKind::Str(parts))
    }

    /// Lex the token stream of one `#{...}` interpolation, positioned just
    /// past the `#{`. Consumes the closing `}`.
    fn lex_interp(&mut self, str_start: usize) -> Result<Vec<Token>, LexError> {
        let saved_prev = self.prev_kind.take();
        let saved_space = self.had_space;
        self.had_space = true;
        let mut out: Vec<Token> = Vec::new();
        let mut brace_depth = 0usize;
        let result = loop {
            self.skip_inline_ws();
            match self.peek() {
                None | Some(b'\n') => {
                    break Err(self.err(
                        ErrorCode::UnterminatedInterpolation,
                        "unterminated `#{...}` interpolation",
                        Span::new(str_start, self.pos),
                    ));
                }
                Some(b'}') if brace_depth == 0 => {
                    self.pos += 1;
                    break Ok(());
                }
                Some(b'#') => {
                    break Err(self.err(
                        ErrorCode::UnexpectedChar,
                        "`#` is not allowed inside string interpolation",
                        Span::new(self.pos, self.pos + 1),
                    ));
                }
                Some(_) => match self.next_token() {
                    Ok(tok) => {
                        match tok.kind {
                            TokenKind::LBrace => brace_depth += 1,
                            TokenKind::RBrace => brace_depth -= 1,
                            _ => {}
                        }
                        self.prev_kind = Some(tok.kind.clone());
                        self.had_space = false;
                        out.push(tok);
                    }
                    Err(e) => break Err(e),
                },
            }
        };
        self.prev_kind = saved_prev;
        self.had_space = saved_space;
        result.map(|_| out)
    }

    // ── helpers ──────────────────────────────────────────────────────────

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\r')) {
            self.pos += 1;
            self.had_space = true;
        }
    }

    fn skip_comment(&mut self) {
        while self.peek().is_some_and(|b| b != b'\n') {
            self.pos += 1;
        }
    }

    fn push(&mut self, kind: TokenKind, span: Span) {
        self.tokens.push(Token {
            kind,
            span,
            line: self.line,
        });
    }

    /// E0014 — `result = case x` puts a multi-line block on the tail of an
    /// assignment, so the value being assigned starts on one line and finishes
    /// several lines below, out of line with its own `=`. The block starts on
    /// the next line instead, indented one level.
    ///
    /// `prev_kind` is cleared at every newline, so it still holding `=` is
    /// exactly what "on the same line as the `=`" means. `do` is not covered:
    /// it assigns a function, not the value of a block, and
    /// `f = do x -> x + 1` is the idiomatic one-liner.
    fn check_block_starts_on_its_own_line(&self, tok: &Token) -> Result<(), LexError> {
        let keyword = match tok.kind {
            TokenKind::Case => "case",
            TokenKind::Cond => "cond",
            TokenKind::If => "if",
            TokenKind::Run => "run",
            // `f = do x -> x + 1` is a whole value on one line, so it is not
            // what the rule is about. Without the `->` the body is a block
            // below the `=`, exactly like a `case`, and the rule applies.
            TokenKind::Do if !self.rest_of_line_has_arrow() => "do",
            _ => return Ok(()),
        };
        if self.prev_kind != Some(TokenKind::Assign) {
            return Ok(());
        }
        Err(self.err(
            ErrorCode::BlockOnAssignmentLine,
            format!(
                "`{keyword}` cannot start on the same line as `=`: put the \
                 block on the next line, indented one level"
            ),
            tok.span,
        ))
    }

    /// Is there an `->` left on this line? Distinguishes `do x -> x + 1` from
    /// the `do x` whose body is indented below it.
    ///
    /// Between the `do` and its `->` a lambda head holds only parameter names
    /// and commas, so the scan stops at the first `#` or `"` — an arrow past
    /// either is inside a comment or a string, not the head's own.
    fn rest_of_line_has_arrow(&self) -> bool {
        let rest = &self.src[self.pos..];
        let line = match rest.find(['\n', '#', '"']) {
            Some(end) => &rest[..end],
            None => rest,
        };
        line.contains("->")
    }

    /// E0014 for `"""` — the same rule as above, for the one literal that is
    /// also several lines tall. `doc = """` leaves the assignment open across
    /// every content line; the string opens on the next line instead, and its
    /// content follows from there.
    ///
    /// This is checked before the string is lexed, so the error points at the
    /// opening quotes rather than at whatever the content turned out to be.
    fn check_multiline_string_starts_on_its_own_line(&self, at: usize) -> Result<(), LexError> {
        if self.prev_kind != Some(TokenKind::Assign) {
            return Ok(());
        }
        Err(self.err(
            ErrorCode::BlockOnAssignmentLine,
            "a multiline string cannot start on the same line as `=`: put the \
             opening `\"\"\"` on the next line, indented one level",
            Span::new(at, at + 3),
        ))
    }

    fn err(&self, code: ErrorCode, message: impl Into<String>, span: Span) -> LexError {
        LexError {
            code,
            message: message.into(),
            span,
        }
    }
}
