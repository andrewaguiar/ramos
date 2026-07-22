# AGENTS.md

Notes for AI coding agents (and humans pairing with them) working on the Ramos
interpreter. Read this before editing anything. It is a map of the project, not
a language tour — for the language, read [`README.md`](./README.md); for the
implementation plan and phase status, read [`PLAN.md`](./PLAN.md).

## What this project is

Ramos is a tree-walking interpreter for the Ramos language, written in Rust.

- **Single crate**, library + thin binary. No workspace, **zero dependencies**
  (no `rand`, no `readline`, no `clap` — the REPL's line editing is hand-rolled
  against `stty`, the CLI arg parsing is hand-rolled in `main.rs`). Do not add a
  dependency to solve something that a few lines of Rust can; that is an
  explicit project constraint, not an oversight.
- The **standard library is Ramos source**, not Rust. It lives in
  [`stdlib/src/*.rmo`](./stdlib/src) and is loaded at startup. `Kernel` is
  implicitly in scope everywhere; everything else needs `alias`.
- The language is **dynamically typed** and the only "static" checks are the
  **strict style rules** below. There is no type checker and there will not be
  one in v1.

## Commands (the gates you must keep green)

```sh
cargo test                 # the real gate — ~279 tests across 14 files
cargo fmt                  # formatting is enforced; run before committing
cargo clippy --all-targets # keep it clean
cargo run -- run examples/hello_world.rmo   # run a program
cargo run -- check examples/hello_world.rmo # strict rules only, no execution
cargo run -- test                          # run every src/test/*Test module
cargo run -- test --quietly                # the same, without the @doc lines
cargo run -- doctest --stdlib stdlib       # run the stdlib's own @doc examples
cargo run -- repl                          # interactive REPL on stdin
cargo run -- lexer examples/hello_world.rmo --dump  # token stream (+raw code)
cargo run -- ast    examples/hello_world.rmo --dump  # AST dump (+raw code)
cargo run -- doc    --stdlib stdlib --out docs        # regenerate HTML docs
./release.sh               # release build into dist/
```

The `--dump`, `--color`, `--no-color` flags and `NO_COLOR` env var exist on the
debug commands. When a test fails on lexing/parsing, reproduce it locally with
`ramos lexer ... --dump` or `ramos ast ... --dump` before guessing.

`cargo test` is fast (under a few seconds). Run it after every non-trivial
change. Do not push anything that leaves it red.

## Repo layout

```
src/
  main.rs        CLI: arg parsing + dispatch (run/check/repl/test/doc/lexer/ast)
  lib.rs         re-exports the public modules below
  ast.rs         AST types (Expr, Pattern, Item, Program, …)
  span.rs        Span, SourceId — every node carries one for diagnostics
  diagnostics.rs error rendering (hand-rolled, no deps)
  color.rs       Color enum + terminal detection (NO_COLOR / isatty)
  lexer/         mod.rs (tokens, INDENT/DEDENT) + rules.rs (strict-rule errors)
  parser/        mod.rs — recursive descent over the token stream
  loader.rs      namespace → file path, one-def-per-file, name/path matching
  doc.rs         HTML doc generator (Hexdocs-style) → docs/
  doctest.rs     extracts `# ==` examples from @doc blocks and runs them
  interp/
    mod.rs       re-exports
    value.rs     Value enum (Int/Float/Str/Symbol/Bool/Nil/List/Tuple/Map/…)
    env.rs       Environment — scopes + closures, everything immutable
    pattern.rs   pattern-matching engine (case / destructuring / guards)
    eval.rs      the evaluator: run / run_with_args / run_with_streams / run_tests
    natives.rs   native(str, args) dispatch table — Rust impls of Kernel.* etc.
    freevars.rs  free-variable analysis (used by lambda capture)
  repl/          mod.rs (session loop) + editor.rs (raw-mode line editing)
  stack.rs       runs the interpreter on a large-stack thread (deep recursion)
stdlib/src/*.rmo  the Ramos stdlib (kernel, list, map, string, …, test)
tests/           Rust integration tests + .rmo fixtures
tests/fixtures/features/*.rmo  one fixture per language feature (used by `doc`)
tests/programs/*.rmo + .out    golden programs: run the binary, diff stdout
editors/neovim/  vim syntax/indent/ftdetect for .rmo
docs/            generated HTML — do not hand-edit, `ramos doc` owns it
```

The pipeline is strictly **lexer → parser → loader → interp**. Each stage owns
its own errors and never reaches into the next stage's internals.

## Public Rust API (the seams to build on)

- `ramos::loader::load(entry: &Path, stdlib_dir: Option<&Path>) -> Result<Program, LoadError>`
  — the single entry point used by `run` / `check`. Loads the stdlib + every
  module the entry file reaches.
- `ramos::loader::stdlib(stdlib_dir)` — load just the stdlib (used by `repl`).
- `ramos::interp::run` / `run_with_args` / `run_with_streams` — execute a loaded
  `Program`. `run_with_streams` is the one tests use, so they can assert on
  stderr.
- `ramos::interp::run_tests` — runs every `function test_*` in modules `implements Test`.
- `ramos::interp::Session` — REPL-style incremental evaluation.
- `ramos::ast::Program` — the whole compiled program; pass it to the interp.

When adding a CLI command, follow the existing pattern in `main.rs`: parse flags
with `take_flag` / `take_opt`, dispatch off `args.first()`, return `ExitCode`.

## Strict rules are load-time errors, not style preferences

This is the single most important thing to internalize. Violating any of these
**stops the interpreter before the program runs**. They are enforced in the
lexer (`src/lexer/rules.rs`) and parser (`src/parser/mod.rs`), each with its own
error code `E0xxx`. If you add a language feature, ask whether a new strict rule
should guard it.

The full list is in the README's "Strict rules" section; the load-bearing ones:

- **2 spaces** per indent level, **never tabs**. Indentation must be a multiple
  of 2 and move exactly one level at a time.
- **Whitespace around binary operators** (`x = 1`, not `x=1`) and **after every
  comma** in args / lists / tuples / maps.
- **Parentheses required on every call** — `foo()`, never bare `foo`. A bare
  identifier is always a variable or field access.
- **Names**: modules are `[a-zA-Z]` CamelCase (`SystemUser`); variables and
  functions are `[a-z_]` snake_case — **no digits, no `?`, no `!`**.
- **One definition per `.rmo` file**, file name matches the definition
  (`SystemUser` → `system_user.rmo`), path follows the namespace
  (`MyApp.Business.SystemUser` → `src/my_app/business/system_user.rmo`).
- **Assigned blocks start on their own line**: when the RHS of `=` is an
  `if` / `case` / `cond` / `run`, a `"""` multiline string, or a `do` with an
  indented body, it goes on the *next* line, indented one level. The
  single-expression `f = do x -> x + 1` is fine — the `->` keeps the value on
  the `=` line, which is the whole point of the rule.
- **A map key carries one `:`** — `{name: 1}`, never `{:name: 1}`.

When you write a `.rmo` fixture or example, follow these or `ramos check` will
reject it. The fixtures under `tests/fixtures/features/` are the canonical
examples of correct style.

## Adding things — where to put them

- **A new native function** (Rust-implemented, like `Kernel.print`):
  `src/interp/natives.rs`. Add the dispatch arm, then expose it from the
  relevant stdlib module in `stdlib/src/*.rmo` (often `kernel.rmo`).
- **A new stdlib function written in Ramos**: add it to the right
  `stdlib/src/<module>.rmo`, following the file's existing style.
- **A new token / lexer rule**: `src/lexer/mod.rs` for the token, and if it's a
  strict rule, an `ErrorCode` variant in `src/lexer/rules.rs` (next free `E0xxx`).
- **A new AST node / parser production**: `src/ast.rs` for the type, then
  `src/parser/mod.rs` for parsing. Remember the strict rules above.
- **A new Value variant**: `src/interp/value.rs` (the enum), then teach
  `type_name`, `inspect`, and `values_equal` about it (`value.rs`); a variant
  with no structural identity (like `Lambda` or `Thread`) just falls through
  `values_equal`'s `_ => false`. `pattern.rs` matches variants by kind, so a new
  one that only ever binds or matches `_` needs nothing there. Keep the
  `Send + Sync` assertion at the bottom of `value.rs` in mind — a variant
  holding a channel or a `JoinHandle` (as `Thread` does) must stay both, or the
  const assertion fails the build. `Thread` (added for the stdlib `Thread`
  module) is the worked example.
- **A new language feature**: also add a fixture under
  `tests/fixtures/features/<name>.rmo` (it feeds the Examples doc page) and at
  least one case in the relevant `tests/*_test.rs`.

## Testing conventions

- Tests are Rust integration tests in `tests/*.rs` — **20 files, ~310 tests**.
- **Ramos snippets in Rust tests are written as multiline strings** — the
  `let src = "\` form, with the program at column zero — whenever the program
  has an indented line. `"case x\n  1 -> :one"` is the thing being replaced;
  a flat one- or two-liner in a table (`"x = 40\nx + 2"`) may stay inline.
  Note `"\` swallows the *next* line's indentation, so a snippet that starts
  indented opens with a bare newline instead.
- There are **no snapshot-testing or external test crates**. Snapshots are
  hand-rolled: assert against the literal expected string and include a
  `dump(...)` in the assertion message for debugging (see `lexer_test.rs`).
- `tests/fixtures/features/*.rmo` are loaded by `feature_fixture_test.rs` and
  `example_fixture_test.rs`; `tests/fixtures/example.rmo` is the catch-all.
- `ramos test` prints each module's `@module_doc` and each test's `@doc` beside
  the results (`ramos::doc::summaries` reads them out of the source text);
  `--quietly` suppresses them. A new Ramos test is expected to carry a `@doc`.
- Ramos-level tests live under `src/test/` (the `TEST_ROOT` constant in
  `main.rs`): a module whose name ends in `Test` and `implements Test`, with
  `function test_*` functions. Run them with `ramos test`.

- **A stdlib `@doc` example is a test.** Every `#   expr   # == value` line is
  run by `ramos doctest` (and by `tests/doctest_test.rs`), in an empty directory
  with the project's modules beside it. So an example must be self-contained:
  write the file before reading it, start the actor before calling it. Shared
  setup goes in a `# ramos doctest setup` snippet in the `@module_doc`, which
  runs before every example in that file. Anything after `# ==` is parsed as
  Ramos — a trailing `(a directory)` note belongs in the prose above.
- **Golden programs** live in `tests/programs/`, one `.out` beside each `.rmo`.
  Record with `UPDATE_GOLDEN=1 cargo test --test program_test`, then *read the
  file before committing it*.

When fixing a bug, **add a regression test** in the matching `tests/*_test.rs`
— the gate is `cargo test`, not "it runs on my machine".

## Conventions to respect

- **No new dependencies.** See "What this project is".
- **Diagnostics carry file/line/column + a source excerpt.** Every error a user
  can hit should render through `diagnostics.rs` using the `Span` on the node.
  Don't `println!` a bare error message from deep in the interp.
- **Values are immutable.** No `RefCell` / `Cell` in `Value` or `Env`.
  "Rebinding" creates a new scope entry; it never mutates. See `src/interp/env.rs`.
- **Equality is structural and strict about types**: `1 == 1.0` is **false**.
  `Int` and `Float` never compare equal implicitly. Don't "fix" this.
- **Truthiness**: only `false` and `nil` are falsy. Everything else is truthy.
- **Lists are cons cells**, `Map` keys are `Integer | String | Symbol` only.
- **`Cargo.lock` is committed** (this crate ships a binary). Do not gitignore it.
- **The interpreter runs on a 256 MB stack** (`src/stack.rs`): the CLI wraps its
  whole body in `with_large_stack`, and every actor thread is sized to match.
  The stdlib recurses once per list element and only self-recursive tail calls
  are trampolined, so ordinary input needs the headroom. `RAMA_STACK_SIZE`
  overrides it. A test that calls `ramos::interp::run*` in-process runs on a
  small test-thread stack and will abort on deep recursion — drive the binary as
  a child process (see `tests/program_test.rs`) when a test recurses deeply.
- **No `cargo-fuzz`.** Fuzzing is hand-rolled in `tests/fuzz_test.rs` (an
  xorshift PRNG through `lex`+`parse` under `catch_unwind`), because the fuzz
  crate is a dependency. Same rule as everything else.
- **Don't hand-edit `docs/`.** It is generated by `ramos doc`; change the doc
  generator (`src/doc.rs`) or the stdlib sources and regenerate.
- **Rust style**: `cargo fmt` before commit, keep clippy clean, keep doc
  comments in the same voice as the existing ones (terse, one-line summary
  then prose).

## Quick orientation for a brand-new change

1. Reproduce the issue / desired behavior with a tiny `.rmo` file.
2. Run `ramos lexer <f> --dump` and `ramos ast <f> --dump` to see where in the
   pipeline it lives.
3. Find the relevant file in the layout above, make the change.
4. Add or update a test in `tests/`, and a fixture if it's a new feature.
5. `cargo fmt && cargo clippy --all-targets && cargo test` — all three green
   before you stop.
