//! `ramos ast [--dump] <file.rmo>` — debug command: print the AST a file
//! parses to, optionally alongside the raw source (`--dump`). Stays on the
//! single named file, unlike `run`/`check`, which load the whole program —
//! that single-file view is the point of this command.

use super::err_tag;
use ramos::color::Color;
use std::fs;
use std::process::ExitCode;

pub fn ast(path: &str, dump: bool, color: Color) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} cannot read `{path}`: {e}", err_tag(color));
            return ExitCode::FAILURE;
        }
    };
    let tokens = match ramos::lexer::lex(&source) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprint!("{}", ramos::diagnostics::render(path, &source, &e));
            return ExitCode::FAILURE;
        }
    };
    match ramos::parser::parse(tokens) {
        Err(e) => {
            eprint!("{}", ramos::diagnostics::render_parse(path, &source, &e));
            ExitCode::FAILURE
        }
        Ok(program) => {
            if dump {
                print!("{}", ramos::ast::dump(&source, &program, color));
            } else {
                print!("{}", ramos::ast::render(&program, color));
            }
            ExitCode::SUCCESS
        }
    }
}
