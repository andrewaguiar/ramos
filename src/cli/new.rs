//! `ramos new <name>` — scaffold a new project.

use super::err_tag;
use ramos::color::{Color, Style};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

/// Split a project name like `pet-project` or `pet_project` into lowercase
/// words, for building both the snake_case directory name and the CamelCase
/// module name a `ramos new` project starts with.
///
/// Empty when `name` has no valid words: each `-`/`_`-separated part must be
/// ASCII alphanumeric, and the first must start with a letter — a module name
/// cannot start with a digit.
fn project_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    for part in name.split(['-', '_']) {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Vec::new();
        }
        words.push(part.to_ascii_lowercase());
    }
    match words.first().and_then(|w| w.chars().next()) {
        Some(c) if c.is_ascii_alphabetic() => words,
        _ => Vec::new(),
    }
}

/// `pet` -> `Pet`. Used to turn a project name's words into its CamelCase
/// module name.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `ramos new <name>` — scaffold a new project: `<name>/src/<snake>/main.rmo`
/// defining `module <CamelCase>.Main` with a `function main()` that prints the
/// project name, plus `<name>/.env.dev`, a starter settings file for the
/// `Config` module's `dev` environment. The module lives where the naming
/// rule (see `loader`) says it must, and its file is named `main.rmo`, so
/// `ramos run <name>` finds it straight away.
pub fn new(args: &[String], color: Color) -> ExitCode {
    let (name, rest) = match args.split_first() {
        Some((name, rest)) => (name, rest),
        None => {
            eprintln!("usage: ramos new <project-name>");
            return ExitCode::from(2);
        }
    };
    if !rest.is_empty() {
        eprintln!("usage: ramos new <project-name>");
        return ExitCode::from(2);
    }
    let words = project_words(name);
    if words.is_empty() {
        eprintln!(
            "{} `{name}` is not a valid project name — use letters, digits, `-` or `_`, \
             starting with a letter",
            err_tag(color)
        );
        return ExitCode::from(2);
    }
    let snake_name = words.join("_");
    let module_name: String = words.iter().map(|w| capitalize(w)).collect();

    let root = PathBuf::from(name);
    if root.exists() {
        eprintln!("{} `{}` already exists", err_tag(color), root.display());
        return ExitCode::FAILURE;
    }
    let src_dir = root.join("src").join(&snake_name);
    if let Err(e) = fs::create_dir_all(&src_dir) {
        eprintln!(
            "{} cannot create `{}`: {e}",
            err_tag(color),
            src_dir.display()
        );
        return ExitCode::FAILURE;
    }
    let main_path = src_dir.join("main.rmo");
    let contents =
        format!("module {module_name}.Main\n  function main()\n    println(\"{name}\")\n");
    if let Err(e) = fs::write(&main_path, contents) {
        eprintln!(
            "{} cannot write `{}`: {e}",
            err_tag(color),
            main_path.display()
        );
        return ExitCode::FAILURE;
    }
    println!(
        "{} `{}`",
        color.paint(Style::Str, "created"),
        main_path.display()
    );

    // The `dev` environment's settings — `Config.start()` reads this when
    // `APP_ENV=dev`. It lives at the project root, not under `src/`, since
    // `Config` resolves it against the working directory a program runs from.
    let env_path = root.join(".env.dev");
    let env_contents = format!(
        "# {name} — the `dev` environment's settings, read by `Config.start()`\n\
         # when `APP_ENV=dev` (unset defaults to `.env` instead). See the `Config`\n\
         # module (`ramos doc`) for the file format.\n\
         #\n\
         # [section]\n\
         # key = \"value\"\n"
    );
    if let Err(e) = fs::write(&env_path, env_contents) {
        eprintln!(
            "{} cannot write `{}`: {e}",
            err_tag(color),
            env_path.display()
        );
        return ExitCode::FAILURE;
    }
    println!(
        "{} `{}`",
        color.paint(Style::Str, "created"),
        env_path.display()
    );
    ExitCode::SUCCESS
}
