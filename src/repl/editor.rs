//! A small line editor for the interactive REPL: history, arrow-key
//! navigation, and a syntax-highlighted input line.
//!
//! Terminal control is done by shelling out to `stty` rather than linking a
//! terminal crate, which keeps the interpreter dependency-free. `stty -g` dumps
//! the current settings as an opaque string, and handing that same string back
//! restores them exactly — so the terminal is left as it was found even if the
//! REPL exits through an error path.
//!
//! Raw mode wraps *reading only*. Evaluation runs with the terminal back in its
//! normal (cooked) mode, so a program's own `print` output needs no special
//! handling: only the few lines drawn by the editor itself have to think about
//! `\r`.
//!
//! Scope: this is a REPL prompt, not a general readline. It assumes the input
//! line fits on one terminal row — an entry long enough to wrap will redraw
//! untidily. Blocks are gathered a line at a time by the caller, so in practice
//! a single line stays short.

use crate::color::{Color, Style};
use crate::lexer;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// What one prompt read produced.
pub enum Input {
    Line(String),
    /// Ctrl-C: abandon the current line (and whatever block it was building).
    Interrupted,
    /// Ctrl-D on an empty line, or end of input.
    Eof,
}

/// Run `stty` against the terminal, returning its stdout on success.
///
/// stdin is inherited so `stty` talks to the same terminal the REPL reads from;
/// stderr is silenced because a failure here is reported by returning `None`.
fn stty(args: &[&str]) -> Option<String> {
    let out = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Raw mode for as long as this value lives. Dropping it restores the terminal.
struct RawMode {
    saved: String,
}

impl RawMode {
    /// `None` when the terminal cannot be put into raw mode (no `stty`, not a
    /// terminal); the caller then falls back to plain line reading.
    fn enable() -> Option<RawMode> {
        let saved = stty(&["-g"])?;
        stty(&["raw", "-echo"])?;
        Some(RawMode { saved })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        stty(&[&self.saved]);
    }
}

/// The keys the editor acts on. Anything else is ignored.
enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Interrupt,
    Eof,
    /// Read failed or the sequence was not one we handle.
    Unknown,
}

pub struct Editor {
    history: Vec<String>,
}

impl Editor {
    pub fn new() -> Editor {
        Editor {
            history: Vec::new(),
        }
    }

    /// Remember an entry for the up-arrow. Consecutive duplicates and blank
    /// lines are dropped, which is what makes holding Up useful.
    pub fn remember(&mut self, line: &str) {
        if line.trim().is_empty() || self.history.last().map(String::as_str) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
    }

    /// Read one line, editing it in place. Falls back to a plain read when raw
    /// mode is unavailable, so a terminal we cannot drive still works.
    pub fn read_line(&mut self, prompt: &str, color: Color) -> Input {
        let Some(_raw) = RawMode::enable() else {
            return read_plain(prompt);
        };

        let mut buf: Vec<char> = Vec::new();
        let mut cursor = 0usize;
        // Where Up/Down are in the history. `history.len()` is "the line being
        // typed", held in `draft` while browsing so it comes back on Down.
        let mut index = self.history.len();
        let mut draft = String::new();

        let mut stdin = std::io::stdin().lock();
        redraw(prompt, &buf, cursor, color);
        loop {
            match read_key(&mut stdin) {
                Key::Char(c) => {
                    buf.insert(cursor, c);
                    cursor += 1;
                }
                Key::Enter => {
                    // Leave the finished line on screen and move off it. The
                    // caller re-enters cooked mode before anything else prints.
                    print!("\r\n");
                    let _ = std::io::stdout().flush();
                    return Input::Line(buf.into_iter().collect());
                }
                Key::Backspace => {
                    if cursor > 0 {
                        cursor -= 1;
                        buf.remove(cursor);
                    }
                }
                Key::Delete => {
                    if cursor < buf.len() {
                        buf.remove(cursor);
                    }
                }
                Key::Left => cursor = cursor.saturating_sub(1),
                Key::Right => cursor = (cursor + 1).min(buf.len()),
                Key::Home => cursor = 0,
                Key::End => cursor = buf.len(),
                Key::Up => {
                    if index > 0 {
                        if index == self.history.len() {
                            draft = buf.iter().collect();
                        }
                        index -= 1;
                        buf = self.history[index].chars().collect();
                        cursor = buf.len();
                    }
                }
                Key::Down => {
                    if index < self.history.len() {
                        index += 1;
                        let text = if index == self.history.len() {
                            draft.clone()
                        } else {
                            self.history[index].clone()
                        };
                        buf = text.chars().collect();
                        cursor = buf.len();
                    }
                }
                Key::Interrupt => {
                    print!("\r\n");
                    let _ = std::io::stdout().flush();
                    return Input::Interrupted;
                }
                Key::Eof => {
                    // Ctrl-D mid-line is a delete-forward, as in a shell; only
                    // an empty line ends the session.
                    if buf.is_empty() {
                        return Input::Eof;
                    }
                    if cursor < buf.len() {
                        buf.remove(cursor);
                    }
                }
                Key::Unknown => {}
            }
            redraw(prompt, &buf, cursor, color);
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Repaint the prompt line and put the cursor back where it belongs.
///
/// `\r` returns to column 0 and `\x1b[K` clears to end of line, so the whole
/// line is rewritten each keystroke — simpler than tracking what changed, and
/// fast enough at prompt sizes. ANSI colour codes occupy no columns, so the
/// cursor is positioned from the *plain* character count.
fn redraw(prompt: &str, buf: &[char], cursor: usize, color: Color) {
    let text: String = buf.iter().collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "\r\x1b[K{prompt}{}", highlight(&text, color));
    let back = buf.len() - cursor;
    if back > 0 {
        let _ = write!(out, "\x1b[{back}D");
    }
    let _ = out.flush();
}

/// Paint the input line with the same highlighter the dumps use.
///
/// A half-typed line often does not lex — `x = ` ends mid-assignment — so a lex
/// failure falls back to plain text rather than showing an error at the prompt.
pub fn highlight(text: &str, color: Color) -> String {
    if color == Color::Never || text.is_empty() {
        return text.to_string();
    }
    match lexer::lex(text) {
        Ok(tokens) => lexer::highlight(text, &tokens, color),
        Err(_) => text.to_string(),
    }
}

/// Paint an evaluated result. `inspect()` renders values as Ramos literals, so
/// the highlighter reads it the same way it reads source.
pub fn paint_result(text: &str, color: Color) -> String {
    match lexer::lex(text) {
        Ok(tokens) => lexer::highlight(text, &tokens, color),
        // Not every `inspect` output is lexable Ramos (a closure, say), so fall
        // back to a single literal-coloured span.
        Err(_) => color.paint(Style::Literal, text),
    }
}

/// One keypress. Escape sequences arrive as several bytes, so `\x1b` pulls the
/// rest of the sequence before deciding.
fn read_key(stdin: &mut impl Read) -> Key {
    let Some(b) = read_byte(stdin) else {
        return Key::Eof;
    };
    match b {
        0x03 => Key::Interrupt,
        0x04 => Key::Eof,
        0x01 => Key::Home, // Ctrl-A
        0x05 => Key::End,  // Ctrl-E
        b'\r' | b'\n' => Key::Enter,
        0x7f | 0x08 => Key::Backspace,
        0x1b => read_escape(stdin),
        // Any other control byte is a key we do not handle.
        b if b < 0x20 => Key::Unknown,
        b => match read_char(stdin, b) {
            Some(c) => Key::Char(c),
            None => Key::Unknown,
        },
    }
}

/// The tail of an escape sequence, after `\x1b`. Handles the `CSI` forms the
/// arrow, Home/End and Delete keys send.
fn read_escape(stdin: &mut impl Read) -> Key {
    match read_byte(stdin) {
        Some(b'[') => {}
        // `\x1bO` prefixes Home/End on some terminals (application mode).
        Some(b'O') => {
            return match read_byte(stdin) {
                Some(b'H') => Key::Home,
                Some(b'F') => Key::End,
                _ => Key::Unknown,
            }
        }
        _ => return Key::Unknown,
    }
    match read_byte(stdin) {
        Some(b'A') => Key::Up,
        Some(b'B') => Key::Down,
        Some(b'C') => Key::Right,
        Some(b'D') => Key::Left,
        Some(b'H') => Key::Home,
        Some(b'F') => Key::End,
        // Numeric forms end with `~`: `3~` is Delete, `1~`/`7~` Home, `4~`/`8~` End.
        Some(n @ b'0'..=b'9') => {
            let mut last = n;
            while let Some(b) = read_byte(stdin) {
                if b == b'~' {
                    break;
                }
                last = b;
            }
            match last {
                b'3' => Key::Delete,
                b'1' | b'7' => Key::Home,
                b'4' | b'8' => Key::End,
                _ => Key::Unknown,
            }
        }
        _ => Key::Unknown,
    }
}

fn read_byte(stdin: &mut impl Read) -> Option<u8> {
    let mut b = [0u8; 1];
    match stdin.read(&mut b) {
        Ok(1) => Some(b[0]),
        _ => None,
    }
}

/// Finish a UTF-8 character whose leading byte is `first`, pulling however many
/// continuation bytes it declares.
fn read_char(stdin: &mut impl Read, first: u8) -> Option<char> {
    let extra = match first {
        0x00..=0x7f => 0,
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        _ => return None, // stray continuation byte
    };
    let mut bytes = vec![first];
    for _ in 0..extra {
        bytes.push(read_byte(stdin)?);
    }
    std::str::from_utf8(&bytes).ok()?.chars().next()
}

/// The non-raw path: one line from stdin, prompt written plainly. Used when the
/// terminal cannot be driven, and by piped input.
fn read_plain(prompt: &str) -> Input {
    use std::io::BufRead;
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Input::Eof,
        Ok(_) => Input::Line(line.trim_end_matches(['\n', '\r']).to_string()),
    }
}
