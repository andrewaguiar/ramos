//! The usage text bare `ramos` (or an unrecognized command) prints.

use std::process::ExitCode;

pub fn usage() -> ExitCode {
    eprintln!("ramos {}", super::VERSION);
    eprintln!("usage: ramos <command> [args]");
    eprintln!();
    eprintln!("  run <file.rmo>             execute a Ramos program (top-level statements)");
    eprintln!("  run <dir>                  run that directory's `main.rmo` (the shallowest");
    eprintln!("                             one found, if there is more than one)");
    eprintln!("  run                        same as `run .` — run the current directory's");
    eprintln!("                             `main.rmo`");
    eprintln!("  run -e CODE                run CODE as a snippet, no `.rmo` file needed");
    eprintln!("  new <project-name>         scaffold a project: <name>/src/<snake>/main.rmo");
    eprintln!("                             defining `<CamelCase>.Main`, plus a `.env.dev`");
    eprintln!("                             Config starter");
    eprintln!("  learn                      print a crash course on the language: every");
    eprintln!("                             keyword, the syntax, and what not to do");
    eprintln!("  repl                       start an interactive session (persists state)");
    eprintln!("  version                    print the version");
    eprintln!("  test [--quietly] [filter]");
    eprintln!("                             run every test under the nearest `src/test`");
    eprintln!("                             (walking up from `.`), or just the files whose");
    eprintln!("                             name or path contains filter; --quietly drops");
    eprintln!("                             the @doc lines from the report");
    eprintln!("  doctest [--quietly] [--stdlib DIR] [DIR]");
    eprintln!("                             run the `# ==` examples in DIR/src/*.rmo @doc blocks");
    eprintln!("                             (default: DIR is `.`, against the embedded stdlib;");
    eprintln!("                             `ramos doctest --stdlib stdlib` documents the stdlib)");
    eprintln!("  check <file.rmo>           verify the strict rules without running");
    eprintln!(
        "  lexer [--dump] <file.rmo>  debug: print the token stream (--dump adds the raw code)"
    );
    eprintln!("  ast [--dump] <file.rmo>    debug: print the AST (--dump adds the raw code)");
    eprintln!("  doc [--port PORT]");
    eprintln!("                             generate HTML docs for the stdlib (Hexdocs-style)");
    eprintln!("                             and serve them at http://localhost:3030 (or --port)");
    eprintln!("  generate-docs [--stdlib DIR] [--out DIR]");
    eprintln!(
        "                             generate HTML docs into ./ramos-docs (or --out) and exit"
    );
    eprintln!("  see [--stdlib DIR] <module>");
    eprintln!("                             print a stdlib module's source (e.g. `HttpRequest`");
    eprintln!("                             or `http_request`, either spelling works)");
    eprintln!();
    eprintln!("  --color / --no-color   force colour on or off (default: on for a");
    eprintln!("                         terminal; NO_COLOR is honoured)");
    ExitCode::from(2)
}
