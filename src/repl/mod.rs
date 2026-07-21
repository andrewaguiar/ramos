//! `ramos repl` — an interactive session.
//!
//! Bindings, `fn`s, `struct`s and `module`s persist across entries: the session
//! accumulates definitions, so a struct defined at one prompt is constructible
//! at the next. A single complete line runs on Enter; a block (or any
//! definition) is gathered until a blank line submits it.
//!
//! On a terminal, input is read through [`editor`], which adds history, arrow
//! navigation and syntax colour. Piped input skips all of that and reads plain
//! lines, so `echo 'x = 1' | ramos repl` behaves as it always did.

mod editor;

use crate::color::{Color, Style};
use crate::interp::{sink, Session};
use editor::{Editor, Input};
use std::path::PathBuf;
use std::process::ExitCode;

pub fn run(stdlib: Option<String>) -> ExitCode {
    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal();
    let color = if interactive {
        Color::for_stdout()
    } else {
        Color::Never
    };
    let mut session = Session::new();

    // The stdlib is definitions only, so evaluating it registers `String`,
    // `List`, … and runs nothing. Without this the prompt would have no
    // modules at all.
    let stdlib_dir = stdlib.map(PathBuf::from);
    match crate::loader::stdlib(stdlib_dir.as_deref()) {
        Ok(program) => {
            if let Err(e) = session.eval(&program, sink(Vec::new())) {
                eprintln!("error: cannot load the stdlib: {e}");
                return ExitCode::FAILURE;
            }
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    if interactive {
        println!("Ramos REPL. Expressions run on Enter; blocks submit on a blank line.");
        println!("Up/Down walk history, Ctrl-C clears the line, Ctrl-D or :quit exits.");
    }

    let mut ed = Editor::new();
    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() {
            "ramos> "
        } else {
            "  ...> "
        };
        let line = match read(&mut ed, prompt, color, interactive) {
            Input::Line(l) => l,
            Input::Interrupted => {
                // Abandon whatever block was being gathered, keep the session.
                buffer.clear();
                continue;
            }
            Input::Eof => {
                if interactive {
                    println!();
                }
                if !buffer.trim().is_empty() {
                    eval_entry(&mut session, &buffer, color);
                }
                break;
            }
        };

        // `:quit` / `:q` only at a fresh prompt (otherwise it's ordinary input).
        if buffer.is_empty() && matches!(line.trim(), ":quit" | ":q") {
            break;
        }
        if line.trim().is_empty() {
            // A blank line submits whatever block has been gathered.
            if !buffer.trim().is_empty() {
                let input = std::mem::take(&mut buffer);
                ed.remember(input.trim_end());
                eval_entry(&mut session, &input, color);
            } else {
                buffer.clear();
            }
            continue;
        }

        let first_line = buffer.is_empty();
        buffer.push_str(&line);
        buffer.push('\n');
        // A complete single line runs at once; a block-opener or an as-yet
        // incomplete line waits for more input (and a blank line to submit).
        if first_line && !opens_a_block(&buffer) && parses(&buffer) {
            let input = std::mem::take(&mut buffer);
            ed.remember(input.trim_end());
            eval_entry(&mut session, &input, color);
        }
    }
    ExitCode::SUCCESS
}

/// One line of input. Only a terminal gets the editor; piped stdin stays on the
/// plain path so scripted input and tests are unaffected.
fn read(ed: &mut Editor, prompt: &str, color: Color, interactive: bool) -> Input {
    if interactive {
        return ed.read_line(prompt, color);
    }
    use std::io::BufRead;
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Input::Eof,
        Ok(_) => Input::Line(line.trim_end_matches(['\n', '\r']).to_string()),
    }
}

/// True if `src` lexes and parses as a complete program — used to tell a
/// finished single line (`1 + 2`) from the start of a block.
fn parses(src: &str) -> bool {
    match crate::lexer::lex(src) {
        Ok(tokens) => crate::parser::parse(tokens).is_ok(),
        Err(_) => false,
    }
}

/// Whether the first word starts a definition or multi-line construct, which the
/// REPL should always gather over several lines (a bodyless `fn f(x)` parses on
/// its own, but the user is almost certainly about to type its body).
fn opens_a_block(src: &str) -> bool {
    matches!(
        src.split_whitespace().next(),
        Some("fn" | "fnp" | "module" | "struct" | "trait")
    )
}

/// Lex, parse, and evaluate one entry against the running `session`, printing
/// the result (or a diagnostic) but never aborting the loop.
fn eval_entry(session: &mut Session, input: &str, color: Color) {
    let tokens = match crate::lexer::lex(input) {
        Ok(t) => t,
        Err(e) => {
            eprint!("{}", crate::diagnostics::render("repl", input, &e));
            return;
        }
    };
    let program = match crate::parser::parse(tokens) {
        Ok(p) => p,
        Err(e) => {
            eprint!("{}", crate::diagnostics::render_parse("repl", input, &e));
            return;
        }
    };
    // A definition-only entry (no statements) has no interesting value to show.
    let has_statement = program
        .items
        .iter()
        .any(|item| matches!(item, crate::ast::Item::Statement(_)));
    let out = sink(std::io::stdout());
    let flush = || {
        let _ = out.lock().unwrap_or_else(|e| e.into_inner()).flush();
    };
    match session.eval(&program, out.clone()) {
        Ok(value) => {
            flush();
            if has_statement {
                println!("{}", editor::paint_result(&value.inspect(), color));
            }
        }
        Err(e) => {
            flush();
            eprintln!("{} {e}", color.paint(Style::Heading, "runtime error:"));
        }
    }
}
