//! `ramos version` (and bare `ramos`) — print the version.

use std::process::ExitCode;

pub fn version() -> ExitCode {
    println!("ramos {}", super::VERSION);
    ExitCode::SUCCESS
}
