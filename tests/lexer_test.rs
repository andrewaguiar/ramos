//! Lexer tests driven by real Ramos code — README examples, stdlib idioms, and
//! one negative test per lexical strict rule.

// The README's Geometry example literally uses `3.14`; the token stream must match it.
#![allow(clippy::approx_constant)]

use ramos::color::Color;
use ramos::lexer::{dump, lex, ErrorCode, StrPart, Token, TokenKind as T};

fn tokens(src: &str) -> Vec<Token> {
    lex(src).unwrap_or_else(|e| {
        panic!(
            "lex failed: {}",
            ramos::diagnostics::render("<test>", src, &e)
        )
    })
}

fn kinds(src: &str) -> Vec<T> {
    tokens(src).into_iter().map(|t| t.kind).collect()
}

/// Assert the exact token stream; on mismatch the panic message carries the
/// raw-code + tokens dump so the failure is readable without re-running.
fn assert_kinds(src: &str, expected: Vec<T>) {
    let toks = tokens(src);
    let got: Vec<T> = toks.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(got, expected, "\n{}", dump(src, &toks, Color::Never));
}

fn err_code(src: &str) -> ErrorCode {
    lex(src).expect_err("expected a lex error").code
}

fn ident(s: &str) -> T {
    T::Ident(s.to_string())
}

fn upper(s: &str) -> T {
    T::UpperIdent(s.to_string())
}

// ── happy path ──────────────────────────────────────────────────────────────

#[test]
fn hello_world_with_interpolation() {
    let toks = lex(r#"print("Ola #{name}")"#).unwrap();
    assert_eq!(toks[0].kind, ident("print"));
    assert_eq!(toks[1].kind, T::LParen);
    match &toks[2].kind {
        T::Str(parts) => {
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0], StrPart::Lit("Ola ".to_string()));
            match &parts[1] {
                StrPart::Interp(inner) => {
                    let inner_kinds: Vec<_> = inner.iter().map(|t| t.kind.clone()).collect();
                    assert_eq!(inner_kinds, vec![ident("name")]);
                }
                other => panic!("expected interpolation, got {other:?}"),
            }
        }
        other => panic!("expected string token, got {other:?}"),
    }
    assert_eq!(toks[3].kind, T::RParen);
    assert_eq!(toks[4].kind, T::Newline);
    assert_eq!(toks[5].kind, T::Eof);
}

#[test]
fn module_with_indentation() {
    let src = "\
module Geometry
  function area(r)
    r * r * 3.14
";
    assert_kinds(
        src,
        vec![
            T::Module,
            upper("Geometry"),
            T::Newline,
            T::Indent,
            T::Function,
            ident("area"),
            T::LParen,
            ident("r"),
            T::RParen,
            T::Newline,
            T::Indent,
            ident("r"),
            T::Star,
            ident("r"),
            T::Star,
            T::Float(3.14),
            T::Newline,
            T::Dedent,
            T::Dedent,
            T::Eof,
        ],
    );
}

#[test]
fn case_with_guard_and_block_arm() {
    // README "Control flow": block arm bodies keep the trailing `->`.
    let src = "\
case list
  [] -> []
  [head | tail] when head > 0 ->
    doubled = head * 2
    [doubled] ++ keep(tail)
  [_ | tail] -> keep(tail)
";
    let ks = kinds(src);
    let count = |k: &T| ks.iter().filter(|x| *x == k).count();
    assert_eq!(count(&T::Indent), 2, "case body + block arm body");
    assert_eq!(count(&T::Dedent), 2);
    assert_eq!(count(&T::Arrow), 3);
    assert_eq!(count(&T::When), 1);
    assert_eq!(count(&T::Pipe), 2, "cons patterns");
    assert_eq!(count(&T::PlusPlus), 1);
    assert_eq!(count(&T::Underscore), 1);
}

#[test]
fn operators_from_the_readme() {
    assert_kinds(
        "x = 1 + 2 * 3 ** 2",
        vec![
            ident("x"),
            T::Assign,
            T::Int(1),
            T::Plus,
            T::Int(2),
            T::Star,
            T::Int(3),
            T::StarStar,
            T::Int(2),
            T::Newline,
            T::Eof,
        ],
    );
    let ks = kinds("a = \"a\" <> \"b\"\nb = [1, 2] ++ [3, 4]\nc = {a: 1} ++ {b: 2}\n");
    assert_eq!(ks.iter().filter(|k| **k == T::Concat).count(), 1);
    assert_eq!(ks.iter().filter(|k| **k == T::PlusPlus).count(), 2);
    assert_eq!(
        kinds("ok = 1 <= 2 and 2 >= 1 or not (1 != 2)")
            .iter()
            .filter(|k| matches!(k, T::Le | T::Ge | T::And | T::Or | T::Not | T::NotEq))
            .count(),
        6
    );
}

#[test]
fn unary_minus_hugs_but_binary_needs_spaces() {
    assert_kinds(
        "x = -5",
        vec![
            ident("x"),
            T::Assign,
            T::Minus,
            T::Int(5),
            T::Newline,
            T::Eof,
        ],
    );
    // line-start unary (README: `-7 % 3`)
    assert_kinds(
        "-7 % 3",
        vec![
            T::Minus,
            T::Int(7),
            T::Percent,
            T::Int(3),
            T::Newline,
            T::Eof,
        ],
    );
    assert_kinds(
        "f(-5)",
        vec![
            ident("f"),
            T::LParen,
            T::Minus,
            T::Int(5),
            T::RParen,
            T::Newline,
            T::Eof,
        ],
    );
    // binary minus with a hugging operand is a strict-rule violation
    assert_eq!(err_code("x -5"), ErrorCode::NoSpaceAroundOperator);
}

#[test]
fn pipes_continue_across_lines() {
    // README "Greet" example shape: leading-pipe continuation lines.
    let src = "\
module Greet
  function hi_all(people)
    people
    | List.map(do p -> p.name)
    | List.join(\", \")
";
    let ks = kinds(src);
    assert_eq!(ks.iter().filter(|k| **k == T::Pipe).count(), 2);
    assert!(ks.contains(&T::Do));
    assert!(ks.contains(&upper("List")));
}

#[test]
fn struct_literal_and_map_keys() {
    assert_kinds(
        "andrew = Person{name: \"Andrew\", age: 40}",
        vec![
            ident("andrew"),
            T::Assign,
            upper("Person"),
            T::LBrace,
            ident("name"),
            T::Colon,
            T::Str(vec![StrPart::Lit("Andrew".to_string())]),
            T::Comma,
            ident("age"),
            T::Colon,
            T::Int(40),
            T::RBrace,
            T::Newline,
            T::Eof,
        ],
    );
    // `:` hugging an identifier is a map key; after whitespace it's a symbol
    assert_kinds(
        "m = {status: :ok}",
        vec![
            ident("m"),
            T::Assign,
            T::LBrace,
            ident("status"),
            T::Colon,
            T::Symbol("ok".to_string()),
            T::RBrace,
            T::Newline,
            T::Eof,
        ],
    );
}

#[test]
fn tagged_tuples_and_symbols() {
    assert_kinds(
        "(:ok, value)",
        vec![
            T::LParen,
            T::Symbol("ok".to_string()),
            T::Comma,
            ident("value"),
            T::RParen,
            T::Newline,
            T::Eof,
        ],
    );
}

#[test]
fn comments_and_blank_lines_do_not_affect_layout() {
    let src = "\
# leading comment

module Doc
  # @module_doc
  #
  # Doc — indented comment lines are ignored.

  function value()
    42  # trailing comment
";
    assert_kinds(
        src,
        vec![
            T::Module,
            upper("Doc"),
            T::Newline,
            T::Indent,
            T::Function,
            ident("value"),
            T::LParen,
            T::RParen,
            T::Newline,
            T::Indent,
            T::Int(42),
            T::Newline,
            T::Dedent,
            T::Dedent,
            T::Eof,
        ],
    );
}

#[test]
fn string_escapes_including_suppressed_interpolation() {
    let toks = lex(r#"s = "a\nb\t\"q\" c\\ \#{raw} #{x}""#).unwrap();
    match &toks[2].kind {
        T::Str(parts) => {
            assert_eq!(
                parts[0],
                StrPart::Lit("a\nb\t\"q\" c\\ #{raw} ".to_string())
            );
            assert!(matches!(parts[1], StrPart::Interp(_)));
        }
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn interpolation_with_expressions_and_nested_braces() {
    let toks = lex(r#"print("you have #{1 + 1} in #{ {a: 1} }")"#).unwrap();
    let T::Str(parts) = &toks[2].kind else {
        panic!("expected string")
    };
    let StrPart::Interp(first) = &parts[1] else {
        panic!("expected interpolation")
    };
    let first_kinds: Vec<_> = first.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(first_kinds, vec![T::Int(1), T::Plus, T::Int(1)]);
    let StrPart::Interp(second) = &parts[3] else {
        panic!("expected second interpolation")
    };
    let second_kinds: Vec<_> = second.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        second_kinds,
        vec![T::LBrace, ident("a"), T::Colon, T::Int(1), T::RBrace]
    );
}

#[test]
fn newlines_inside_brackets_are_joined() {
    let src = "x = [1,\n  2,\n  3]\n";
    assert_kinds(
        src,
        vec![
            ident("x"),
            T::Assign,
            T::LBracket,
            T::Int(1),
            T::Comma,
            T::Int(2),
            T::Comma,
            T::Int(3),
            T::RBracket,
            T::Newline,
            T::Eof,
        ],
    );
}

#[test]
fn case_with_struct_pattern() {
    let src = "\
case result
  Response{retryable: true} -> :retry
  _ -> :done
";
    let ks = kinds(src);
    assert!(ks.contains(&upper("Response")));
    assert!(ks.contains(&T::Underscore));
    assert_eq!(ks.iter().filter(|k| **k == T::Indent).count(), 1);
}

#[test]
fn missing_trailing_newline_is_fine() {
    assert_kinds(
        "x = 1",
        vec![ident("x"), T::Assign, T::Int(1), T::Newline, T::Eof],
    );
}

#[test]
fn multiline_strings() {
    // Assigned, so the string opens on the line after the `=` (E0014).
    let src = "\
str =
  \"\"\"
    Hi there

    This is a multiline string
  \"\"\"

print(str)
";
    let toks = tokens(src);
    assert_eq!(toks[0].kind, ident("str"));
    assert_eq!(toks[1].kind, T::Assign);
    assert_eq!(toks[2].kind, T::Newline);
    assert_eq!(toks[3].kind, T::Indent);
    assert_eq!(
        toks[4].kind,
        T::Str(vec![StrPart::Lit(
            "Hi there\n\nThis is a multiline string\n".to_string()
        )])
    );
    assert_eq!(toks[5].kind, T::Newline);
    assert_eq!(toks[6].kind, T::Dedent);
    assert_eq!(toks[7].kind, ident("print"));

    // empty multiline string
    let src = "\
s =
  \"\"\"
  \"\"\"
";
    let toks = tokens(src);
    assert_eq!(toks[4].kind, T::Str(vec![StrPart::Lit(String::new())]));
}

#[test]
fn multiline_string_indent_stripping_and_interpolation() {
    // opened at indent 4 (the assigned value's own line, inside a function): content
    // at 6, and the extra spaces past that are content
    let src = "\
function help()
  text =
    \"\"\"
      Usage: #{name}
        indented extra
    \"\"\"
  text
";
    let toks = tokens(src);
    let str_tok = toks
        .iter()
        .find(|t| matches!(t.kind, T::Str(_)))
        .expect("no string token");
    let T::Str(parts) = &str_tok.kind else {
        unreachable!()
    };
    assert_eq!(parts[0], StrPart::Lit("Usage: ".to_string()));
    let StrPart::Interp(inner) = &parts[1] else {
        panic!("expected interpolation, got {:?}", parts[1])
    };
    assert_eq!(inner[0].kind, ident("name"));
    assert_eq!(parts[2], StrPart::Lit("\n  indented extra\n".to_string()));
}

#[test]
fn multiline_string_strict_rules() {
    // content must be indented one level past the opening line
    assert_eq!(
        err_code("\"\"\"\nHi\n\"\"\"\n"),
        ErrorCode::BadMultilineString
    );
    // the opening `"""` must end its line
    assert_eq!(
        err_code("\"\"\"Hi\n\"\"\"\n"),
        ErrorCode::BadMultilineString
    );
    // the closing `"""` must stand alone on its line
    assert_eq!(
        err_code("\"\"\"\n  hi\n\"\"\" <> \"x\"\n"),
        ErrorCode::BadMultilineString
    );
    // missing closing delimiter
    assert_eq!(err_code("\"\"\"\n  hi\n"), ErrorCode::UnterminatedString);
}

#[test]
fn an_assigned_multiline_string_starts_on_its_own_line() {
    // The same E0014 `case` and `run` answer to: the value of an assignment
    // never hangs off the end of its `=`.
    for src in [
        "s = \"\"\"\n  hi\n\"\"\"\n",
        "module A\n  function f()\n    s = \"\"\"\n      hi\n    \"\"\"\n",
    ] {
        assert_eq!(
            err_code(src),
            ErrorCode::BlockOnAssignmentLine,
            "should be rejected:\n{src}"
        );
    }

    // Written the way the rule asks for, and everywhere an assignment is not.
    for src in [
        "s =\n  \"\"\"\n    hi\n  \"\"\"\n",
        "function help()\n  \"\"\"\n    usage\n  \"\"\"\n",
        "case x\n  1 ->\n    \"\"\"\n      one\n    \"\"\"\n",
    ] {
        assert!(lex(src).is_ok(), "should lex clean:\n{src}");
    }
}

#[test]
fn dump_prints_raw_code_and_tokens() {
    let src = "x = 1 + 2\n";
    let expected = "\
Raw Code:
```
x = 1 + 2
```

Lexer tokens
```
[
  Ident(\"x\"),
  Assign,
  Int(1),
  Plus,
  Int(2),
  Newline,
  Eof,
]
```
";
    assert_eq!(dump(src, &tokens(src), Color::Never), expected);
    // source without a trailing newline still produces a well-formed block
    assert_eq!(
        dump("x = 1", &tokens("x = 1"), Color::Never),
        expected
            .replace("1 + 2", "1")
            .replace("  Plus,\n  Int(2),\n", "")
    );
}

// ── strict rules (one negative test each) ───────────────────────────────────

#[test]
fn strict_no_tabs_in_indentation() {
    assert_eq!(
        err_code("module Foo\n\tfn bar()\n"),
        ErrorCode::TabInIndentation
    );
}

#[test]
fn strict_indent_must_be_multiple_of_two() {
    assert_eq!(
        err_code("module Foo\n   function bar()\n"),
        ErrorCode::BadIndentation
    );
}

#[test]
fn strict_indent_one_level_at_a_time() {
    assert_eq!(
        err_code("module Foo\n    function bar()\n"),
        ErrorCode::BadIndentation
    );
}

#[test]
fn strict_module_names_are_camel_case() {
    assert_eq!(err_code("alias My_App as M"), ErrorCode::InvalidModuleName);
    assert_eq!(err_code("alias App2 as A"), ErrorCode::InvalidModuleName);
}

#[test]
fn strict_identifiers_lowercase_only() {
    assert_eq!(err_code("myVar = 1"), ErrorCode::InvalidIdentifier);
    assert_eq!(err_code("x2 = 1"), ErrorCode::InvalidIdentifier);
    assert_eq!(err_code("valid? = true"), ErrorCode::InvalidIdentifier);
    assert_eq!(err_code("save! = true"), ErrorCode::InvalidIdentifier);
}

#[test]
fn strict_whitespace_around_operators() {
    assert_eq!(err_code("x=1"), ErrorCode::NoSpaceAroundOperator);
    assert_eq!(err_code("y = 1+2"), ErrorCode::NoSpaceAroundOperator);
    assert_eq!(err_code("y = 1 +2"), ErrorCode::NoSpaceAroundOperator);
    assert_eq!(err_code("y = 1<>2"), ErrorCode::NoSpaceAroundOperator);
    assert_eq!(err_code("a|b"), ErrorCode::NoSpaceAroundOperator);
}

#[test]
fn strict_assigned_blocks_start_on_their_own_line() {
    for src in [
        "r = case x\n  1 -> :one\n",
        "r = cond\n  true -> 1\n",
        "r = run\n  :ok = check()\n",
        "module A\n  function f()\n    c = cond\n      true -> 1\n",
    ] {
        assert_eq!(
            err_code(src),
            ErrorCode::BlockOnAssignmentLine,
            "should be rejected:\n{src}"
        );
    }
}

#[test]
fn an_assigned_do_with_a_block_body_starts_on_its_own_line() {
    // No `->` on the `=` line means the body is a block below it, which is
    // what the rule is about; the arrow form ends on its own line and is left
    // alone (covered below).
    for src in [
        "f = do x\n  x + 1\n",
        "f = do x, y\n  z = x + y\n  z * 2\n",
        "module A\n  function g()\n    f = do x\n      x + 1\n",
        // An `->` inside a comment or a string is not the head's own arrow.
        "f = do x  # maps x -> y\n  x + 1\n",
    ] {
        assert_eq!(
            err_code(src),
            ErrorCode::BlockOnAssignmentLine,
            "should be rejected:\n{src}"
        );
    }
    // Written the way the rule asks for.
    assert!(lex("f =\n  do x\n    x + 1\n").is_ok());
    // And the arrow form keeps working even when its body holds an arrow.
    assert!(lex("f = do x -> \"a -> b\"\n").is_ok());
}

#[test]
fn a_block_on_the_next_line_is_the_accepted_form() {
    // The same programs, written the way the rule asks for.
    for src in [
        "r =\n  case x\n    1 -> :one\n",
        "r =\n  cond\n    true -> 1\n",
        "r =\n  run\n    :ok = check()\n",
    ] {
        assert!(lex(src).is_ok(), "should lex clean:\n{src}");
    }
}

#[test]
fn the_assigned_block_rule_leaves_other_constructs_alone() {
    // A `do` whose `->` keeps it on one line is a value, not a block.
    assert!(lex("f = do x -> x + 1\n").is_ok());
    assert!(lex("f = do x, y -> x + y\n").is_ok());
    assert!(lex("f = do -> 42\n").is_ok());
    // A block is fine as an arm body — that `->` is not an `=`.
    assert!(lex("case x\n  1 ->\n    cond\n      true -> 2\n").is_ok());
    // `==` is not `=`.
    assert!(lex("y = a == b\n").is_ok());
}

#[test]
fn strict_whitespace_after_comma() {
    assert_eq!(err_code("f(a,b)"), ErrorCode::NoSpaceAfterComma);
}

#[test]
fn unterminated_string_and_bad_escape() {
    assert_eq!(err_code("x = \"abc"), ErrorCode::UnterminatedString);
    assert_eq!(err_code("x = \"a\\q\""), ErrorCode::InvalidEscape);
    // interpolation cut off by end of line / EOF
    assert_eq!(
        err_code("x = \"a #{b\ny = 2\n"),
        ErrorCode::UnterminatedInterpolation
    );
    assert_eq!(
        err_code("x = \"a #{b"),
        ErrorCode::UnterminatedInterpolation
    );
    // a `"` inside `#{...}` opens a nested string; unterminated, it's E0007
    assert_eq!(err_code("x = \"a #{b\""), ErrorCode::UnterminatedString);
}

#[test]
fn invalid_symbols_and_numbers() {
    assert_eq!(err_code("x = :foo1"), ErrorCode::InvalidSymbol);
    assert_eq!(err_code("x = 42abc"), ErrorCode::InvalidNumber);
}

#[test]
fn bang_suggests_not() {
    let err = lex("x = !true").unwrap_err();
    assert_eq!(err.code, ErrorCode::UnexpectedChar);
    assert!(err.message.contains("not"));
}

#[test]
fn ampersand_suggests_and() {
    for src in ["x = true && false", "x = true & false"] {
        let err = lex(src).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnexpectedChar);
        assert!(err.message.contains("and"), "{src}: {}", err.message);
    }
}

#[test]
fn double_pipe_suggests_or() {
    // `|` is the pipe operator, so this must not fall through to a spacing error.
    let err = lex("x = true || false").unwrap_err();
    assert_eq!(err.code, ErrorCode::UnexpectedChar);
    assert!(err.message.contains("or"), "{}", err.message);
}

#[test]
fn single_pipe_still_lexes_as_the_pipe_operator() {
    assert!(lex("x = list | List.sort()").is_ok());
}

// ── a larger, README-faithful program ───────────────────────────────────────

#[test]
fn payments_module_end_to_end() {
    let src = "\
module Payments
  function max_amount()
    10000

  function validate(amount)
    cond
      amount <= 0 -> raise BusinessError{message: \"must be positive\", code: 555}
      amount > Payments.max_amount() -> :too_high
      true -> :ok

  helper double_all(list)
    list
    | List.map(do x -> x * 2)
    | List.join(\", \")
";
    let ks = kinds(src);
    let count = |f: &dyn Fn(&T) -> bool| ks.iter().filter(|k| f(k)).count();
    assert!(ks.contains(&T::Helper));
    assert!(ks.contains(&upper("BusinessError")));
    assert_eq!(count(&|k| *k == T::Pipe), 2);
    assert_eq!(count(&|k| matches!(k, T::Symbol(_))), 2);
    assert_eq!(count(&|k| *k == T::Arrow), 4, "3 cond arms + 1 lambda");
    // module(0) → body(2) → function body(4) → cond arms(6); helper body dedents back
    assert_eq!(count(&|k| *k == T::Indent), count(&|k| *k == T::Dedent));
}

#[test]
fn a_float_requires_a_digit_immediately_after_the_dot() {
    // `1.5` keeps its decimal point; `1..5` does not start a float — a
    // float's `.` must be followed by a digit, so two dots in a row lex as
    // two separate `Dot` tokens (field access) rather than one.
    assert_kinds("1.5", vec![T::Float(1.5), T::Newline, T::Eof]);
    assert_kinds(
        "1..5",
        vec![T::Int(1), T::Dot, T::Dot, T::Int(5), T::Newline, T::Eof],
    );
}

// ── the optional `end` marker ─────────────────────────────────────────────

#[test]
fn a_lone_end_line_vanishes_from_the_token_stream() {
    // `end` is purely decorative — indentation alone closes the block — so
    // the token stream with one is identical to the token stream without it.
    let without_end = "function f()\n  1\n";
    let with_end = "function f()\n  1\nend\n";
    assert_eq!(kinds(with_end), kinds(without_end));
}

#[test]
fn nested_end_markers_all_vanish() {
    let without_end = "module A\n  function f()\n    1\n";
    let with_ends = "module A\n  function f()\n    1\n  end\nend\n";
    assert_eq!(kinds(with_ends), kinds(without_end));
}

#[test]
fn end_without_a_trailing_newline_still_vanishes() {
    // The lexer synthesizes a final newline before EOF when the source
    // doesn't end in one; `end` should disappear the same way either way.
    assert_eq!(kinds("function f()\n  1\nend"), kinds("function f()\n  1\n"));
}

#[test]
fn end_sharing_a_line_with_anything_else_is_an_error() {
    assert_eq!(err_code("x = end"), ErrorCode::StrayEnd);
    assert_eq!(err_code("println(1) end"), ErrorCode::StrayEnd);
    assert_eq!(err_code("end end"), ErrorCode::StrayEnd);
}
