//! `ramos check <file.rmo>` — load the entry file with the stdlib and every
//! module it reaches, same as `run`, but stop before executing it. Verifies
//! the strict rules and that every reachable module resolves, without
//! running a line of the program.

use super::load;
use ramos::color::{Color, Style};
use std::process::ExitCode;

pub fn check(path: &str, stdlib: Option<String>, color: Color) -> ExitCode {
    let stdlib_dir = load::stdlib_dir(stdlib);
    let program = match load::program(path, stdlib_dir.as_deref()) {
        Ok(p) => p,
        Err(code) => return code,
    };
    println!(
        "{path}: {} ({} items loaded)",
        color.paint(Style::Str, "ok"),
        program.items.len()
    );
    ExitCode::SUCCESS
}
