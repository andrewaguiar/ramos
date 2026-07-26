//! `ramos lexer [--dump] <file.rmo>` — debug command: print the token
//! stream a file lexes to, optionally alongside the raw source (`--dump`).

use super::err_tag;
use ramos::color::Color;
use std::fs;
use std::process::ExitCode;

pub fn lexer(path: &str, dump: bool, color: Color) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} cannot read `{path}`: {e}", err_tag(color));
            return ExitCode::FAILURE;
        }
    };
    match ramos::lexer::lex(&source) {
        Err(e) => {
            eprint!("{}", ramos::diagnostics::render(path, &source, &e));
            ExitCode::FAILURE
        }
        Ok(tokens) => {
            if dump {
                print!("{}", ramos::lexer::dump(&source, &tokens, color));
            } else {
                print!("{}", ramos::lexer::render(&tokens, color));
            }
            ExitCode::SUCCESS
        }
    }
}
