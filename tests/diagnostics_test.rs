//! Every strict-rule violation renders with a wrong/correct snippet pair
//! under the diagnostic, alongside the message. A plain syntax error (a
//! missing token, an unclosed paren) has no such pair — there is no
//! "correct" alternative to a truncated program.

use ramos::diagnostics::{render, render_parse};
use ramos::lexer::{lex, ErrorCode};
use ramos::parser::parse;

/// Every lexer error code names a strict rule (see `src/lexer/rules.rs`), so
/// every one of them carries a canonical example. This guards a future
/// `ErrorCode` variant from being added without one.
#[test]
fn every_lexer_error_code_has_an_example() {
    for code in ErrorCode::ALL {
        assert!(
            code.example().is_some(),
            "{code:?} ({}) has no wrong/correct example",
            code.as_str()
        );
    }
}

/// A lexer diagnostic renders the example under the message.
#[test]
fn a_lex_error_renders_with_its_example() {
    let src = "module Test\n  function main()\n    x=1\n";
    let err = lex(src).expect_err("missing whitespace around `=` should fail to lex");
    let rendered = render("<test>", src, &err);
    assert!(rendered.contains("wrong:"), "{rendered}");
    assert!(rendered.contains("correct:"), "{rendered}");
    assert!(rendered.contains("x = 1"), "{rendered}");
}

/// A parser diagnostic for a named strict rule renders the same way.
#[test]
fn a_strict_rule_parse_error_renders_with_its_example() {
    let src = "module Dup\n  function twice(x)\n    x\n\n  function twice(x, y)\n    x + y\n";
    let tokens = lex(src).expect("lex");
    let err = parse(tokens).expect_err("a duplicate function name should fail to parse");
    let rendered = render_parse("<test>", src, &err);
    assert!(rendered.contains("wrong:"), "{rendered}");
    assert!(rendered.contains("correct:"), "{rendered}");
}

/// A plain syntax error — not a named strict rule — has no example to show.
#[test]
fn a_generic_syntax_error_renders_without_an_example() {
    let src = "module Test\n  function main()\n    x = \n";
    let tokens = lex(src).expect("lex");
    let err = parse(tokens).expect_err("a dangling `=` should fail to parse");
    let rendered = render_parse("<test>", src, &err);
    assert!(
        !rendered.contains("wrong:") && !rendered.contains("correct:"),
        "a generic syntax error should not carry an example: {rendered}"
    );
}

/// The example block never leaves trailing whitespace on a line.
#[test]
fn rendered_examples_have_no_trailing_whitespace() {
    let src = "module Test\n  function main()\n    x=1\n";
    let err = lex(src).expect_err("missing whitespace around `=` should fail to lex");
    let rendered = render("<test>", src, &err);
    for line in rendered.lines() {
        assert_eq!(line, line.trim_end(), "trailing whitespace in: {line:?}");
    }
}
