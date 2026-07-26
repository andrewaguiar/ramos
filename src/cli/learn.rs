//! `ramos learn` — print a fixed crash course on the language: every keyword,
//! the syntax, and what not to do. Meant to be read as-is (or piped into an
//! agent's context), so it is never colored regardless of `--color`.

use std::process::ExitCode;

pub fn learn() -> ExitCode {
    print!("{}", ramos::learn::text());
    ExitCode::SUCCESS
}
