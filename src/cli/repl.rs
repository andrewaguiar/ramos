//! `ramos repl` — start an interactive session on stdin. All the session
//! logic (line editing, persisted bindings across entries) lives in
//! `ramos::repl`; this is just the CLI's entry into it.

use ramos::color::Color;
use std::process::ExitCode;

pub fn repl(stdlib: Option<String>, color: Color) -> ExitCode {
    ramos::repl::run(stdlib, color)
}
