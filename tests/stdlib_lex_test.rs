//! The four Ramos stdlib modules under stdlib/ must pass the lexer (and its
//! strict rules) untouched — they are the acceptance fixture for phase 1.

use std::path::Path;

#[test]
fn lexes_the_entire_ramos_stdlib() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("src");
    for name in [
        "kernel.rmo",
        "list.rmo",
        "string.rmo",
        "tuple.rmo",
        "file.rmo",
        "dir.rmo",
    ] {
        let path = dir.join(name);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let tokens = ramos::lexer::lex(&src).unwrap_or_else(|e| {
            panic!("{}", ramos::diagnostics::render(name, &src, &e));
        });
        assert!(
            tokens.len() > 100,
            "{name}: suspiciously few tokens ({})",
            tokens.len()
        );
        assert_eq!(tokens.last().unwrap().kind, ramos::lexer::TokenKind::Eof);
        // every stdlib file is one module: first real token is `module`
        assert_eq!(tokens[0].kind, ramos::lexer::TokenKind::Module);
    }
}
