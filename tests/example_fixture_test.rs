//! tests/fixtures/example.rmo packs every Ramos language feature into one file.
//! It must lex clean, and — since it claims *every* feature — every token
//! kind the lexer can produce must actually appear in its stream.

use ramos::lexer::{lex, TokenKind as T};
use std::path::Path;

fn fixture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example.rmo");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn example_fixture_lexes_clean() {
    let src = fixture();
    let tokens = lex(&src).unwrap_or_else(|e| {
        panic!("{}", ramos::diagnostics::render("example.rmo", &src, &e));
    });
    assert_eq!(tokens.last().unwrap().kind, T::Eof);
}

#[test]
fn example_fixture_uses_every_token_kind() {
    let src = fixture();
    let kinds: Vec<T> = lex(&src).unwrap().into_iter().map(|t| t.kind).collect();
    let has = |name: &str, pred: &dyn Fn(&T) -> bool| {
        assert!(
            kinds.iter().any(pred),
            "example.rmo never produces token kind: {name}"
        );
    };

    // layout
    has("Newline", &|k| *k == T::Newline);
    has("Indent", &|k| *k == T::Indent);
    has("Dedent", &|k| *k == T::Dedent);
    has("Eof", &|k| *k == T::Eof);
    // literals & identifiers
    has("Int", &|k| matches!(k, T::Int(_)));
    has("Float", &|k| matches!(k, T::Float(_)));
    has("Str", &|k| matches!(k, T::Str(_)));
    has("Sigil", &|k| matches!(k, T::Sigil(_, _)));
    has("Symbol", &|k| matches!(k, T::Symbol(_)));
    has("Ident", &|k| matches!(k, T::Ident(_)));
    has("UpperIdent", &|k| matches!(k, T::UpperIdent(_)));
    has("Underscore", &|k| *k == T::Underscore);
    // keywords
    has("Module", &|k| *k == T::Module);
    has("Struct", &|k| *k == T::Struct);
    has("Trait", &|k| *k == T::Trait);
    has("Implements", &|k| *k == T::Implements);
    has("Attributes", &|k| *k == T::Attributes);
    has("Fn", &|k| *k == T::Fn);
    has("Fnp", &|k| *k == T::Fnp);
    has("Case", &|k| *k == T::Case);
    has("Cond", &|k| *k == T::Cond);
    has("Run", &|k| *k == T::Run);
    has("Lambda", &|k| *k == T::Do);
    has("Alias", &|k| *k == T::Alias);
    has("As", &|k| *k == T::As);
    has("SelfKw", &|k| *k == T::SelfKw);
    has("True", &|k| *k == T::True);
    has("False", &|k| *k == T::False);
    has("Nil", &|k| *k == T::Nil);
    has("When", &|k| *k == T::When);
    has("And", &|k| *k == T::And);
    has("Or", &|k| *k == T::Or);
    has("Not", &|k| *k == T::Not);
    // punctuation
    has("LParen", &|k| *k == T::LParen);
    has("RParen", &|k| *k == T::RParen);
    has("LBracket", &|k| *k == T::LBracket);
    has("RBracket", &|k| *k == T::RBracket);
    has("LBrace", &|k| *k == T::LBrace);
    has("RBrace", &|k| *k == T::RBrace);
    has("Comma", &|k| *k == T::Comma);
    has("Dot", &|k| *k == T::Dot);
    has("Colon", &|k| *k == T::Colon);
    has("Arrow", &|k| *k == T::Arrow);
    has("Assign", &|k| *k == T::Assign);
    // operators
    has("EqEq", &|k| *k == T::EqEq);
    has("NotEq", &|k| *k == T::NotEq);
    has("Lt", &|k| *k == T::Lt);
    has("Gt", &|k| *k == T::Gt);
    has("Le", &|k| *k == T::Le);
    has("Ge", &|k| *k == T::Ge);
    has("Plus", &|k| *k == T::Plus);
    has("Minus", &|k| *k == T::Minus);
    has("Star", &|k| *k == T::Star);
    has("Slash", &|k| *k == T::Slash);
    has("Percent", &|k| *k == T::Percent);
    has("StarStar", &|k| *k == T::StarStar);
    has("Concat", &|k| *k == T::Concat);
    has("PlusPlus", &|k| *k == T::PlusPlus);
    has("Pipe", &|k| *k == T::Pipe);
}
