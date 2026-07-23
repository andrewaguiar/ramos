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
    /// Every variant, in `E0xxx` order — for anything that walks the full set
    /// (`ramos learn`'s "what not to do" section, the test asserting every
    /// code carries an example).
    pub const ALL: [ErrorCode; 16] = [
        ErrorCode::TabInIndentation,
        ErrorCode::BadIndentation,
        ErrorCode::InvalidModuleName,
        ErrorCode::InvalidIdentifier,
        ErrorCode::NoSpaceAroundOperator,
        ErrorCode::NoSpaceAfterComma,
        ErrorCode::UnterminatedString,
        ErrorCode::InvalidEscape,
        ErrorCode::InvalidNumber,
        ErrorCode::UnexpectedChar,
        ErrorCode::InvalidSymbol,
        ErrorCode::UnterminatedInterpolation,
        ErrorCode::BadMultilineString,
        ErrorCode::BlockOnAssignmentLine,
        ErrorCode::UnknownSigil,
        ErrorCode::StrayEnd,
    ];
}

impl ErrorCode {
    /// A short, human-facing name for the rule — used where the code stands
    /// alone with its example and needs a label (`ramos learn`), rather than
    /// a specific violation's own message.
    pub fn title(self) -> &'static str {
        match self {
            ErrorCode::TabInIndentation => "tabs are never allowed in indentation",
            ErrorCode::BadIndentation => "indentation is 2 spaces, one level at a time",
            ErrorCode::InvalidModuleName => "module names are CamelCase, letters only",
            ErrorCode::InvalidIdentifier => "variables/functions are a-z and _ only",
            ErrorCode::NoSpaceAroundOperator => "whitespace is required around binary operators",
            ErrorCode::NoSpaceAfterComma => "whitespace is required after a comma",
            ErrorCode::UnterminatedString => "a string literal must close on the line it opens",
            ErrorCode::InvalidEscape => "only a fixed set of escape sequences is valid",
            ErrorCode::InvalidNumber => "a numeric literal cannot be followed by a letter",
            ErrorCode::UnexpectedChar => "no symbolic operators — words instead (and/or/not)",
            ErrorCode::InvalidSymbol => "symbols are a-z and _ only",
            ErrorCode::UnterminatedInterpolation => "a `#{...}` interpolation must close",
            ErrorCode::BadMultilineString => "a `\"\"\"` string has a fixed shape",
            ErrorCode::BlockOnAssignmentLine => "an assigned block starts on the next line",
            ErrorCode::UnknownSigil => "a sigil letter is one of D, T, N, U",
            ErrorCode::StrayEnd => "`end` must be alone on its line",
        }
    }

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

    /// A canonical wrong/correct pair for this code, shown under the
    /// diagnostic. Some codes cover more than one message (`BadIndentation`
    /// has three, `UnexpectedChar` five); one representative pair per code is
    /// enough to show the shape of the fix.
    pub fn example(self) -> Option<crate::diagnostics::Example> {
        use crate::diagnostics::Example;
        Some(match self {
            ErrorCode::TabInIndentation => Example {
                wrong: "function main()\n\tx = 1",
                correct: "function main()\n  x = 1",
            },
            ErrorCode::BadIndentation => Example {
                wrong: "function main()\n   x = 1",
                correct: "function main()\n  x = 1",
            },
            ErrorCode::InvalidModuleName => Example {
                wrong: "module system_user",
                correct: "module SystemUser",
            },
            ErrorCode::InvalidIdentifier => Example {
                wrong: "user1 = 1",
                correct: "user_one = 1",
            },
            ErrorCode::NoSpaceAroundOperator => Example {
                wrong: "x=1",
                correct: "x = 1",
            },
            ErrorCode::NoSpaceAfterComma => Example {
                wrong: "function test(n,x)",
                correct: "function test(n, x)",
            },
            ErrorCode::UnterminatedString => Example {
                wrong: "greeting = \"hello",
                correct: "greeting = \"hello\"",
            },
            ErrorCode::InvalidEscape => Example {
                wrong: "\"a\\qtab\"",
                correct: "\"a\\ttab\"",
            },
            ErrorCode::InvalidNumber => Example {
                wrong: "weight = 5kg",
                correct: "weight = 5",
            },
            ErrorCode::UnexpectedChar => Example {
                wrong: "true && false",
                correct: "true and false",
            },
            ErrorCode::InvalidSymbol => Example {
                wrong: ":User",
                correct: ":user",
            },
            ErrorCode::UnterminatedInterpolation => Example {
                wrong: "\"hi #{name\"",
                correct: "\"hi #{name}\"",
            },
            ErrorCode::BadMultilineString => Example {
                wrong: "banner =\n  \"\"\"text\n  \"\"\"",
                correct: "banner =\n  \"\"\"\n    text\n  \"\"\"",
            },
            ErrorCode::BlockOnAssignmentLine => Example {
                wrong: "result = case value\n  true -> :ok\n  _ -> :error",
                correct: "result =\n  case value\n    true -> :ok\n    _ -> :error",
            },
            ErrorCode::UnknownSigil => Example {
                wrong: "X\"2024-01-01\"",
                correct: "D\"2024-01-01\"",
            },
            ErrorCode::StrayEnd => Example {
                wrong: "x = end",
                correct: "x = 1",
            },
        })
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
