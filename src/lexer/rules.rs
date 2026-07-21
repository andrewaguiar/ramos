//! Lexical strict rules. Ramos "fails the interpreter" on style violations, so
//! each rule is a hard error with its own code (see README "Strict rules").

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// E0001 — tabs are never allowed in indentation
    TabInIndentation,
    /// E0002 — indentation must be a multiple of 2 and move one level at a time
    BadIndentation,
    /// E0003 — module segments are CamelCase, letters only
    InvalidModuleName,
    /// E0004 — variables/functions use only `a-z` and `_` (no digits, `?`, `!`)
    InvalidIdentifier,
    /// E0005 — binary operators require whitespace on both sides
    NoSpaceAroundOperator,
    /// E0006 — `,` requires whitespace after it
    NoSpaceAfterComma,
    /// E0007 — string literal not closed before newline/EOF
    UnterminatedString,
    /// E0008 — unknown escape sequence in a string
    InvalidEscape,
    /// E0009 — malformed numeric literal
    InvalidNumber,
    /// E0010 — character has no meaning in Ramos
    UnexpectedChar,
    /// E0011 — symbols use only `a-z` and `_`
    InvalidSymbol,
    /// E0012 — `#{` interpolation not closed before newline/EOF
    UnterminatedInterpolation,
    /// E0013 — malformed multiline string (`"""` placement or content indent)
    BadMultilineString,
    /// E0014 — a block whose value is assigned must start on the next line
    BlockOnAssignmentLine,
    /// E0015 — a letter hugging a string is not one of the fixed sigils
    UnknownSigil,
    /// E0016 — `end` is a purely decorative block marker; it must be alone on
    /// its line, nothing else
    StrayEnd,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::TabInIndentation => "E0001",
            ErrorCode::BadIndentation => "E0002",
            ErrorCode::InvalidModuleName => "E0003",
            ErrorCode::InvalidIdentifier => "E0004",
            ErrorCode::NoSpaceAroundOperator => "E0005",
            ErrorCode::NoSpaceAfterComma => "E0006",
            ErrorCode::UnterminatedString => "E0007",
            ErrorCode::InvalidEscape => "E0008",
            ErrorCode::InvalidNumber => "E0009",
            ErrorCode::UnexpectedChar => "E0010",
            ErrorCode::InvalidSymbol => "E0011",
            ErrorCode::UnterminatedInterpolation => "E0012",
            ErrorCode::BadMultilineString => "E0013",
            ErrorCode::BlockOnAssignmentLine => "E0014",
            ErrorCode::UnknownSigil => "E0015",
            ErrorCode::StrayEnd => "E0016",
        }
    }
}

/// Variables, functions, and symbols: `a-z` and `_` only.
pub fn valid_lower_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

/// Module segments: CamelCase, `a-zA-Z` only.
pub fn valid_upper_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase()) && s.chars().all(|c| c.is_ascii_alphabetic())
}
