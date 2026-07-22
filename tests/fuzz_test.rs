//! Phase 9 fuzzing: the lexer and parser must not panic, on any input.
//!
//! Their contract is total — every string either lexes and parses, or comes
//! back as a `LexError`/`ParseError`. Never a panic, never a hang, whatever the
//! bytes. Indentation makes that easy to get wrong: INDENT/DEDENT bookkeeping,
//! a run of dedents closing several blocks at once, an unterminated string or
//! interpolation, a stray control character. So this hammers both stages with
//! generated input and asserts only that they *return*.
//!
//! No `cargo-fuzz`: it pulls in `libfuzzer-sys`, and this crate takes no
//! dependencies (see AGENTS.md). The generator is a hand-rolled xorshift, the
//! same as the REPL hand-rolls its line editing — deterministic, so a failure
//! reproduces from the seed printed in the assertion, and dependency-free.

use ramos::lexer::lex;
use ramos::parser::parse;
use std::panic;

/// A tiny deterministic PRNG (xorshift64*). Seeded per run so a failure names a
/// seed the assertion can hand back for reproduction.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Feed one input through the whole front end, catching a panic as a failure.
/// Returns `Err(message)` if either stage panicked, `Ok(())` otherwise —
/// including when they returned a `LexError` or `ParseError`, which is a fine
/// answer to garbage.
fn lex_then_parse(src: &str) -> Result<(), String> {
    let outcome = panic::catch_unwind(|| {
        if let Ok(tokens) = lex(src) {
            let _ = parse(tokens);
        }
    });
    outcome.map_err(|payload| {
        let what = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic".to_string());
        format!("panicked: {what}")
    })
}

/// Run `attempts` generated inputs from `seed`, each built by `build`, and fail
/// with the offending input and the seed to reproduce it.
fn hammer(seed: u64, attempts: usize, mut build: impl FnMut(&mut Rng) -> String) {
    // Muffle the default panic hook so a caught panic does not spew a backtrace
    // for every garbage input; the assertion below reports what matters.
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let mut rng = Rng(seed);
    let mut failure = None;
    for _ in 0..attempts {
        let src = build(&mut rng);
        if let Err(what) = lex_then_parse(&src) {
            failure = Some((src, what));
            break;
        }
    }
    panic::set_hook(previous);
    if let Some((src, what)) = failure {
        panic!("seed {seed}: {what}\ninput was:\n{src:?}");
    }
}

/// The bytes most likely to trip up an indentation-sensitive, string-rich
/// lexer: whitespace of every kind, the string and interpolation delimiters,
/// operators, brackets, and a couple of letters and digits.
const INTERESTING: &[u8] = b" \t\n\r\"'#{}()[]:.,|-+*/<>=abcAB012\\";

#[test]
fn random_bytes_never_panic_the_front_end() {
    hammer(0x9E37_79B9_7F4A_7C15, 20_000, |rng| {
        let len = rng.below(64);
        (0..len).map(|_| rng.next() as u8 as char).collect()
    });
}

#[test]
fn interesting_bytes_never_panic_the_front_end() {
    hammer(0xD1B5_4A32_D192_ED03, 40_000, |rng| {
        let len = rng.below(80);
        (0..len)
            .map(|_| INTERESTING[rng.below(INTERESTING.len())] as char)
            .collect()
    });
}

#[test]
fn indentation_stress_never_panics() {
    // Lines of varying indentation ending in block openers and stray dedents —
    // the shape that exercises the INDENT/DEDENT stack hardest.
    let openers = [
        "module M", "function f()", "case x", "cond", "if a", "  x = 1", "|", "\"\"\"",
    ];
    hammer(0x0BAD_C0DE_1234_5678, 40_000, |rng| {
        let lines = rng.below(8);
        let mut src = String::new();
        for _ in 0..lines {
            for _ in 0..rng.below(4) {
                src.push_str("  ");
            }
            src.push_str(openers[rng.below(openers.len())]);
            src.push('\n');
        }
        src
    });
}

#[test]
fn mutating_real_snippets_never_panics() {
    // Start from valid Ramos and corrupt it — a byte flipped, dropped, or
    // doubled. Mutations of real input reach states pure noise rarely does: a
    // string opened and never closed, an indent off by one space.
    let seeds = [
        "module M\n  function f(x)\n    case x\n      1 -> :one\n      _ -> :other\n",
        "x = \"hi #{name}\"\n[a, b | rest] = list\n",
        "cond\n  a > 1 -> :big\n  true -> :small\n",
        "greet = do name -> \"Ola #{name}\"\n",
    ];
    hammer(0xFEED_FACE_CAFE_BEEF, 40_000, |rng| {
        let mut bytes = seeds[rng.below(seeds.len())].as_bytes().to_vec();
        for _ in 0..1 + rng.below(4) {
            if bytes.is_empty() {
                break;
            }
            let at = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[at] = INTERESTING[rng.below(INTERESTING.len())],
                1 => {
                    bytes.remove(at);
                }
                _ => bytes.insert(at, INTERESTING[rng.below(INTERESTING.len())]),
            }
        }
        // A mutation may split a UTF-8 sequence; the front end takes `&str`, so
        // fall back to a lossy view rather than testing something it never sees.
        String::from_utf8(bytes.clone())
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned())
    });
}
