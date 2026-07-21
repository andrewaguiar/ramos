//! Running on a large stack (PLAN phase 9a).
//!
//! Ramos recurses. The stdlib is written as `[head | tail]` recursion, so
//! `List.map` costs one native frame per element, and the parser is recursive
//! descent, so a deeply nested expression costs frames too. A thread's default
//! stack — 8 MB for the main thread, and as little as 2 MB for a spawned one —
//! runs out early: before this module, `List.map` over a few hundred elements
//! aborted the process.
//!
//! A stack overflow cannot be caught and turned into a diagnostic; the runtime
//! aborts. The mitigation is therefore to have plenty of stack in the first
//! place: everything that evaluates or parses runs on a thread with a large one.
//!
//! `RAMA_STACK_SIZE` overrides the default — `32M`, `512K`, or a plain byte
//! count. It is read once per call, which is often enough for a CLI and cheap
//! enough not to matter.

/// The stack every interpreter thread gets, unless `RAMA_STACK_SIZE` says
/// otherwise. Large enough for deep recursion, small enough to be an ordinary
/// virtual allocation — pages are only committed as they are touched.
pub const DEFAULT_STACK_SIZE: usize = 256 * 1024 * 1024;

/// Run `f` on a thread with a large stack, returning what it returns.
///
/// Scoped, so `f` may borrow its surroundings — the caller keeps writers,
/// programs and buffers where they are, and this only moves the execution.
/// A panic inside `f` propagates to the caller, as it would without the thread.
pub fn with_large_stack<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    let size = stack_size();
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .name("ramos".to_string())
            .stack_size(size)
            .spawn_scoped(scope, f)
        {
            // Propagate a panic rather than swallowing it into a different
            // failure: the caller should see what it would have seen inline.
            Ok(handle) => match handle.join() {
                Ok(value) => value,
                Err(panic) => std::panic::resume_unwind(panic),
            },
            // Out of threads or memory. Nothing useful to fall back to — the
            // caller asked for a big stack because it needs one.
            Err(e) => panic!("cannot start the interpreter thread: {e}"),
        }
    })
}

/// The configured stack size, honouring `RAMA_STACK_SIZE`.
pub fn stack_size() -> usize {
    match std::env::var("RAMA_STACK_SIZE") {
        Ok(text) => parse_size(&text).unwrap_or(DEFAULT_STACK_SIZE),
        Err(_) => DEFAULT_STACK_SIZE,
    }
}

/// `"64M"`, `"512K"`, `"1G"` or a plain byte count. A value that makes no sense
/// is ignored rather than fatal: a typo in an environment variable should not
/// stop a program from running.
fn parse_size(text: &str) -> Option<usize> {
    let text = text.trim();
    let (digits, scale) = match text.chars().last()? {
        'k' | 'K' => (&text[..text.len() - 1], 1024),
        'm' | 'M' => (&text[..text.len() - 1], 1024 * 1024),
        'g' | 'G' => (&text[..text.len() - 1], 1024 * 1024 * 1024),
        _ => (text, 1),
    };
    let n: usize = digits.trim().parse().ok()?;
    let bytes = n.checked_mul(scale)?;
    // A stack of a few kilobytes is a typo, not a tuning choice; refuse it so a
    // fat-fingered value falls back to the default rather than aborting at once.
    (bytes >= 256 * 1024).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_with_and_without_a_suffix() {
        assert_eq!(parse_size("16M"), Some(16 * 1024 * 1024));
        assert_eq!(parse_size("512k"), Some(512 * 1024));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size(" 8388608 "), Some(8 * 1024 * 1024));
    }

    #[test]
    fn nonsense_and_tiny_sizes_are_ignored() {
        assert_eq!(parse_size("plenty"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("64"), None, "a 64-byte stack is a typo");
        assert_eq!(parse_size("64K"), None, "and so is a 64 KB one");
        assert_eq!(parse_size("512K"), Some(512 * 1024), "but 512K is not");
    }

    #[test]
    fn the_closure_runs_and_can_borrow() {
        let mut seen = Vec::new();
        with_large_stack(|| seen.push(42));
        assert_eq!(seen, vec![42]);
    }
}
