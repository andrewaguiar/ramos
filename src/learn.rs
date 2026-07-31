//! `ramos learn` — a self-contained crash course, meant to be read by a
//! person or piped straight into an AI agent's context. It teaches the whole
//! language (every keyword, the literals, the operators) and then, since
//! Ramos enforces a single way to write everything, teaches the mistakes: a
//! wrong/correct pair for every strict rule that stops the interpreter
//! before a program runs.
//!
//! Rendered as Markdown — a reader shouldn't need the README, and a program
//! piping this into a prompt gets clean structure (headings, fenced code)
//! rather than hand-aligned columns.

use crate::lexer::ErrorCode;

pub fn text() -> String {
    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(&keywords());
    out.push_str(LITERALS_AND_OPERATORS);
    out.push_str(&example_section());
    out.push_str(&strict_rules());
    out.push_str(&stdlib_section());
    out.push_str(FOOTER);
    out
}

fn code(src: &str) -> String {
    format!("```ramos\n{src}\n```\n\n")
}

const HEADER: &str = "\
# Ramos — a crash course (`ramos learn`)

## What it is

Ramos is a small, indentation-driven, immutable language. Two spaces per
indent level, no tabs, no `{ }`, no `;` — and almost every layout choice a
style guide would otherwise argue about is instead a hard error at load
time, before your program runs (`ramos check` reports these without
executing anything). Modules hold functions; structs hold typed, immutable
records; `case` / `cond` / `if` branch; `do` builds lambdas; actors hold
state behind message passing, each on its own thread. There is deliberately
one way to write each of these — see **What not to do** below for the rules
that enforce it.

## Running a file

| Command | What it does |
| --- | --- |
| `ramos run file.rmo` | execute it (calls that file's `main()`) |
| `ramos check file.rmo` | verify the strict rules without running |
| `ramos test` | run every test module under `src/test` |
| `ramos repl` | interactive session, state persists |
| `ramos doc` | generate the stdlib's HTML reference |
| `ramos lexer` / `ramos ast <file>` | debug: print the token stream / AST |

## Keywords

All 25, none redundant.

";

/// One keyword entry: its written form, a one-line description, and a
/// runnable example.
struct Keyword {
    signature: &'static str,
    blurb: &'static str,
    example: &'static str,
}

const KEYWORDS: &[Keyword] = &[
    Keyword {
        signature: "module Name",
        blurb: "A namespace of functions.",
        example: "module Payments\n  function charge(amount)\n    amount * 1.03",
    },
    Keyword {
        signature: "struct Name",
        blurb: "A typed, immutable record.",
        example: "struct Account\n  attributes\n    balance: 0",
    },
    Keyword {
        signature: "trait Name",
        blurb: "A contract of functions a struct/module opts into.",
        example: "trait Shape\n  function area(self)",
    },
    Keyword {
        signature: "implements Trait",
        blurb: "Opt into a trait — first thing in the body.",
        example: "struct Account\n  implements Reportable",
    },
    Keyword {
        signature: "attributes",
        blurb: "A struct's fields and their defaults.",
        example: "attributes\n  balance: 0\n  owner: nil",
    },
    Keyword {
        signature: "function name(...)",
        blurb: "A public function.",
        example: "function charge(amount)\n  amount * 1.03",
    },
    Keyword {
        signature: "helper name(...)",
        blurb: "A private function: callable from this module's own functions/helpers, \
                 never from outside, and never calls back into one of its own module's \
                 public functions.",
        example: "helper log(amount)\n  print(\"charged #{amount}\")",
    },
    Keyword {
        signature: "return expr",
        blurb: "Exit the current `function`/`helper` body early with a value — always one, \
                 `return nil` spells the empty one. Only valid as a direct statement of that \
                 body: not at the top level, not inside a `do` lambda, and not nested inside \
                 an `if`/`case`/`cond`/`run` — a trailing `when` still reaches it, but a \
                 branch-dependent value is what `cond` is for.",
        example: "function classify(n)\n  return :zero when n == 0\n  :other",
    },
    Keyword {
        signature: "alias Path [as Name]",
        blurb: "A short local name for a module — inside a module or struct body, or \
                 at the top level.",
        example: "alias Geometry.Shapes.Circle",
    },
    Keyword {
        signature: "as Name",
        blurb: "Rename what `alias` binds (default: the last path segment).",
        example: "alias Geometry.Shapes.Circle as C",
    },
    Keyword {
        signature: "case value",
        blurb: "Match a value against patterns, top to bottom. An arm may also bind the \
                 whole matched value with `pattern = name`, for when the value has no name \
                 of its own — the result of a call, say.",
        example: "case x\n  0 -> :zero\n  n when n > 0 -> :positive\n  _ -> :negative\n\n\
                  case fetch()\n  (:ok, v) = whole -> (v, whole)",
    },
    Keyword {
        signature: "if cond / else",
        blurb: "Exactly two branches — no `else if` (use `cond`).",
        example: "if x > 0\n  :positive\nelse\n  :zero_or_less",
    },
    Keyword {
        signature: "cond",
        blurb: "A chain of boolean conditions, first match wins.",
        example: "cond\n  x > 0 -> :positive\n  x < 0 -> :negative\n  true  -> :zero",
    },
    Keyword {
        signature: "run",
        blurb: "Run matches in sequence, halt on the first failure.",
        example: "run\n  (:ok, user) = find_user(id)\n  (:ok, receipt) = charge(user)\n  receipt",
    },
    Keyword {
        signature: "do params [-> expr]",
        blurb: "An anonymous function (a lambda).",
        example: "add = do x, y -> x + y",
    },
    Keyword {
        signature: "end",
        blurb: "Optional, decorative block-closer — never required, must be alone on \
                 its line.",
        example: "if ready\n  go()\nend",
    },
    Keyword {
        signature: "when",
        blurb: "A guard: on a `case` arm, or trailing a statement.",
        example: "print(\"big\") when x > 3",
    },
    Keyword {
        signature: "and / or / not",
        blurb: "Logical operators — no `&&` `||` `!`.",
        example: "ready and not stopped",
    },
    Keyword {
        signature: "true / false / nil",
        blurb: "The boolean and absence values.",
        example: "found = false\nvalue = nil",
    },
    Keyword {
        signature: "self",
        blurb: "The current struct instance or module, inside its own functions.",
        example: "function total(self)\n  self.balance",
    },
    Keyword {
        signature: "_",
        blurb: "Wildcard — matches anything, binds nothing.",
        example: "case x\n  _ -> :whatever",
    },
];

fn keywords() -> String {
    let mut out = String::new();
    for kw in KEYWORDS {
        out.push_str(&format!("### `{}`\n\n{}\n\n", kw.signature, kw.blurb));
        out.push_str(&code(kw.example));
    }
    out
}

const LITERALS_AND_OPERATORS: &str = "\
## Literals

| Literal | Type |
| --- | --- |
| `42` | Integer |
| `3.14` | Float |
| `\"hi #{name}\"` | String, with `#{...}` interpolation |
| `:ok` | Symbol |
| `[1, 2, 3]` | List (`[head \\| tail]` destructures) |
| `(1, 2, 3)` | Tuple |
| `{a: 1, b: 2}` | Map |
| `Account{balance: 0}` | Struct literal (declared fields only, by name) |
| `D\"2024-01-01\"` | Date sigil |
| `T\"12:00:00\"` | Time sigil |
| `N\"2024-01-01 12:00:00\"` | NaiveDateTime sigil |
| `U\"2024-01-01T12:00:00Z\"` | DateTime sigil |

## Operators

| Operator | Meaning |
| --- | --- |
| `+ - * / % ** -x` | arithmetic (int / int truncates toward zero) |
| `== != < > <= >=` | comparison |
| `and or not` | logical — short-circuit, return an operand, not a bool |
| `<>` | string concat |
| `++` | list concat / map merge (right side wins on a key clash) |
| `\\|` | pipe: lhs becomes the first argument of the call after it — must start its own line |

";

const PATTERNS_INTRO: &str = "\
## Pattern matching & destructuring

The same patterns work in `=`, `case`, and function parameters.

";

const PATTERNS_EXAMPLE: &str =
    "(a, b) = (1, 2)\n[first | rest] = [1, 2, 3]\n{name: n} = {name: \"andrew\"}\nAccount{balance: b} = account";

const ERROR_HANDLING_INTRO: &str = "\
## Error handling

No exceptions, no `raise`, no `catch`. A fallible call returns `(:ok, value)`
on success; on failure it returns whatever `exception` or `error` (both
`Kernel`, called bare) built. `exception(type, message)` gives
`(:error, (type, message))` — the common case, an expected business-logic
failure. `error(type, message)` gives `(:error, (type, message, stacktrace))`
— the same, plus `current_stacktrace()`, for a failure worth tracing back to
where it happened. `run` chains a sequence of fallible steps and stops at the
first that doesn't match `(:ok, _)` — the failing value becomes the block's
result, so a `case` with **no subject** right after it handles both the
success and the failure path at the same indentation:

";

const RUN_CASE_EXAMPLE: &str = "\
run
  (:ok, user)    = find_user(id)
  (:ok, receipt) = charge(user)
  (:ok, receipt)
case
  (:ok, receipt) -> print(\"charged #{receipt.amount}\")
  err            -> print(\"failed: #{err}\")";

const ACTORS: &str = "\
## Actors

Stateful, message-passing concurrency, each on its own thread. A module
`implements Actor` and defines `call(f, args, state, config)`, returning
`(reply, new_state)`. `start_actor`, `call_actor`, `cast_actor` (Kernel
functions, called bare) drive it. A `do` lambda can never be one of the
args — see **What not to do**.

";

/// The complete program shown under "A complete program", used both to
/// build the display text and (via [`example_program`]) to actually run in a
/// test, so this can't silently drift out of sync with the language.
pub fn example_program() -> &'static str {
    "module App
  function main()
    users = [{name: \"Andrew\", age: 40}, {name: \"Alex\", age: 8}]
    adults =
      users
      | List.filter(do u -> Map.get(u, :age, nil) >= 18)
      | List.map(do u -> Map.get(u, :name, nil))
    print(\"adults: #{adults}\")   # adults: [\"Andrew\"]
"
}

fn example_section() -> String {
    let mut out = String::from(PATTERNS_INTRO);
    out.push_str(&code(PATTERNS_EXAMPLE));
    out.push_str(ERROR_HANDLING_INTRO);
    out.push_str(&code(RUN_CASE_EXAMPLE));
    out.push_str(ACTORS);
    out.push_str("## A complete program\n\n");
    out.push_str(&code(example_program().trim_end()));
    out
}

const FOOTER: &str = "\
## Where to go next

| Where | What |
| --- | --- |
| `README.md` | the full tour, including the standard library |
| `ramos doc` | generated stdlib reference (`docs/index.html`) |
| `stdlib/src/*.rmo` | the standard library, written in Ramos itself |
| `ramos check file.rmo` | shows this exact wrong/correct format for any strict-rule violation in your own code |
";

/// Every strict rule, as a wrong/correct pair — the lexer's, from
/// `ErrorCode::ALL` (so this can never drift from what the lexer actually
/// enforces), then a curated set of the parser's, which has no equivalent
/// registry to walk.
fn strict_rules() -> String {
    let mut out = String::from(
        "## What not to do\n\n\
         Every rule below is a load-time error, not a lint — the interpreter refuses to \
         run a program that breaks one. `ramos check` reports these with the same \
         wrong/correct shape for whatever your own file gets wrong.\n\n",
    );
    for code_id in ErrorCode::ALL {
        let example = code_id
            .example()
            .expect("every ErrorCode carries a wrong/correct example");
        out.push_str(&format!(
            "### {} — {}\n\nWrong:\n\n",
            code_id.as_str(),
            code_id.title()
        ));
        out.push_str(&code(example.wrong));
        out.push_str("Correct:\n\n");
        out.push_str(&code(example.correct));
    }
    for (title, wrong, correct) in PARSER_RULES {
        out.push_str(&format!("### {title}\n\nWrong:\n\n"));
        out.push_str(&code(wrong));
        out.push_str("Correct:\n\n");
        out.push_str(&code(correct));
    }
    out
}

/// Parser-enforced strict rules have no `ErrorCode`-style registry to walk
/// (their `ParseError`s share one E0100 code), so this is a hand-picked set —
/// the rules most likely to surprise someone new to the language.
const PARSER_RULES: &[(&str, &str, &str)] = &[
    (
        "a pipe starts its own line",
        "map | Map.get(:key, nil)",
        "map\n| Map.get(:key, nil)",
    ),
    (
        "a `do` lambda passed directly as a call argument fits on one line",
        "SomeProcess.process_and_call_back(\n  [1, 2, 3],\n  do x ->\n    print(x)\n)",
        "callback =\n  do x\n    print(x)\n\nSomeProcess.process_and_call_back([1, 2, 3], callback)",
    ),
    (
        "once a call's arguments spill past one line, every argument is on its own line",
        "SomeProcess.process([1, 2],\n  \"a\"\n)",
        "SomeProcess.process(\n  [1, 2],\n  \"a\"\n)",
    ),
    (
        "no `do` lambda reaches an actor (start_actor/call_actor/cast_actor)",
        "call_actor(:cache, Cache, :process, [do x -> x + 1])",
        "call_actor(:cache, Cache, :process, [x])  # the actor's own `call` does the work",
    ),
    (
        "a helper may not call back into its own module's public functions",
        "module Payments\n  function charge(amount)\n    1\n\n  helper log(amount)\n    charge(amount)",
        "module Payments\n  function charge(amount)\n    log(amount)\n\n  helper log(amount)\n    amount",
    ),
    (
        "a name resolves to exactly one function — no arity overloading",
        "function twice(x)\n  x + x\n\nfunction twice(x, y)\n  x + y",
        "function twice(x, y)\n  x + y",
    ),
    (
        "a map key's symbol carries exactly one leading `:`",
        "{:name: 1}",
        "{name: 1}",
    ),
    (
        "a pattern cannot bind the same name twice",
        "(p, p) = (1, 2)",
        "(p, q) = (1, 2)",
    ),
    (
        "`return` cannot cross a lambda boundary, even one nested in the same function",
        "function greet(name)\n  List.each([name], do n ->\n    return n)",
        "function greet(name)\n  return name when name != nil\n  \"stranger\"",
    ),
    (
        "`return` cannot nest inside a written `if`/`case`/`cond`/`run` — only a \
         trailing `when` reaches it",
        "function grade(score)\n  if score > 8\n    return :high\n  :other",
        "function grade(score)\n  return :high when score > 8\n  :other",
    ),
];

/// Every module and public function `ramos run` merges in automatically —
/// walked straight off `loader::STDLIB`, the same embedded sources the
/// interpreter loads, so this can never list a module or a signature that
/// isn't really there. Signature and one-line summary only; `ramos doc` (or
/// `stdlib/src/*.rmo` itself) has the full prose and runnable examples.
fn stdlib_section() -> String {
    let mut out = String::from(
        "## Standard library\n\n\
         Every module below is merged into every program automatically — no \
         `alias` needed for a bare `Kernel` call, and every other module is \
         reachable by name. This is the index: signature and one-line \
         summary. `ramos doc` renders the full reference (every parameter, \
         every runnable example); `stdlib/src/*.rmo` is the source, since \
         the stdlib is written in Ramos itself.\n\n",
    );
    let mut modules: Vec<(String, Vec<(String, String)>)> = crate::loader::STDLIB
        .iter()
        .map(|(_, source)| stdlib_module_entry(source))
        .collect();
    modules.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, functions) in &modules {
        out.push_str(&format!("### `{name}`\n\n"));
        if functions.is_empty() {
            out.push_str("_No public functions._\n\n");
            continue;
        }
        for (signature, summary) in functions {
            if summary.is_empty() {
                out.push_str(&format!("- `{signature}`\n"));
            } else {
                out.push_str(&format!("- `{signature}` — {summary}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// A stdlib module's name and every public function's `name(params)`
/// signature paired with its `@doc` summary (empty when it has none) — parsed
/// from the same source `ramos run` embeds, not hand-copied, so it can't drift
/// from the real module.
fn stdlib_module_entry(source: &str) -> (String, Vec<(String, String)>) {
    let tokens = crate::lexer::lex(source).expect("embedded stdlib module lexes");
    let program = crate::parser::parse(tokens).expect("embedded stdlib module parses");
    let (name, defined_functions) = program
        .items
        .iter()
        .find_map(|it| match it {
            crate::ast::Item::Module(m) => Some((m.name.to_string(), &m.functions)),
            crate::ast::Item::Trait(t) => Some((t.name.to_string(), &t.functions)),
            crate::ast::Item::Struct(s) => Some((s.name.to_string(), &s.functions)),
            _ => None,
        })
        .expect("embedded stdlib module defines a module, trait, or struct");

    let summaries = crate::doc::summaries(source);
    let functions = defined_functions
        .iter()
        .filter(|f| !f.private)
        .map(|f| {
            let signature = format!("{}({})", f.name, f.params.join(", "));
            let summary = summaries
                .functions
                .get(&f.name)
                .cloned()
                .unwrap_or_default();
            (signature, summary)
        })
        .collect();
    (name, functions)
}
