mod cli;

/// Everything runs on a thread with a large stack: the parser is recursive
/// descent and the stdlib recurses per element, so the default stack overflows
/// on ordinary input (a few hundred list elements). See `ramos::stack`.
fn main() -> std::process::ExitCode {
    ramos::stack::with_large_stack(cli::run)
}
