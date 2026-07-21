//! The REPL session (`ramos::interp::Session`): unlike a one-shot `run`, it
//! keeps scope, `fn`s, and `module`s alive across entries. These drive that
//! persistence directly, the way the `ramos repl` loop feeds it one entry at a
//! time.

use ramos::interp::{sink, Session};
use ramos::lexer::lex;
use ramos::parser::parse;

/// A capturing sink whose bytes can be read back with [`taken`] after the run.
fn capture() -> std::sync::Arc<std::sync::Mutex<Vec<u8>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}
fn taken(buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
    String::from_utf8(std::mem::take(&mut *buf.lock().unwrap())).expect("utf8")
}

/// Feed one entry, returning the result's `inspect` form.
fn feed(session: &mut Session, src: &str) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    session
        .eval(&program, sink(Vec::new()))
        .unwrap_or_else(|e| panic!("`{src}` failed: {e}"))
        .inspect()
}

/// Feed one entry, returning what it wrote to stdout.
fn feed_output(session: &mut Session, src: &str) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let buf = capture();
    session.eval(&program, buf.clone()).expect("eval");
    taken(&buf)
}

#[test]
fn bindings_persist_across_entries() {
    let mut s = Session::new();
    assert_eq!(feed(&mut s, "x = 21"), "21");
    assert_eq!(feed(&mut s, "x * 2"), "42");
    // A later entry can build on an earlier binding.
    assert_eq!(feed(&mut s, "y = x + 1"), "22");
    assert_eq!(feed(&mut s, "x + y"), "43");
}

#[test]
fn rebinding_in_a_later_entry_takes_effect() {
    let mut s = Session::new();
    feed(&mut s, "x = 1");
    feed(&mut s, "x = 2");
    assert_eq!(feed(&mut s, "x"), "2");
}

#[test]
fn function_definitions_persist_and_are_callable() {
    let mut s = Session::new();
    // A definition-only entry yields nil, but registers the function.
    let def = "\
fn double(n)
  n * 2
";
    assert_eq!(feed(&mut s, def), "nil");
    assert_eq!(feed(&mut s, "double(21)"), "42");
    // Definitions and bindings coexist.
    feed(&mut s, "base = 100");
    assert_eq!(feed(&mut s, "double(base)"), "200");
}

#[test]
fn print_output_is_written_to_the_sink() {
    let mut s = Session::new();
    assert_eq!(feed_output(&mut s, "print(\"hello\")"), "hello");
    // The value of a `print` entry is nil (the side effect is the output).
    assert_eq!(feed(&mut s, "print(\"x\")"), "nil");
}

#[test]
fn a_failed_entry_does_not_poison_the_session() {
    let mut s = Session::new();
    feed(&mut s, "x = 5");

    // An entry that raises at runtime returns Err, but must not lose `x`.
    let tokens = lex("no_such_function(1)").expect("lex");
    let program = parse(tokens).expect("parse");
    assert!(s.eval(&program, sink(Vec::new())).is_err());

    assert_eq!(feed(&mut s, "x + 1"), "6");
}
