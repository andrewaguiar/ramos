//! `ramos see <module>` — print a stdlib module's Ramos source, verbatim.

use super::err_tag;
use ramos::color::Color;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

/// A name reduced to its bare letters and digits, lowercased — `"HttpRequest"`
/// and `"http_request"` both become `"httprequest"`. `ramos see`'s one job:
/// match a module name written either way (the CamelCase it is called by in
/// Ramos source, or the snake_case file it lives in) against `loader::STDLIB`,
/// without needing a real CamelCase-to-snake_case algorithm — the loader's own
/// naming rule (a module's file is always its name in snake_case) already
/// guarantees the two normalize to the same thing.
fn normalize_module_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// `<module>` is matched against both the file stem (`http_request`) and the
/// CamelCase name it is called by in source (`HttpRequest`) via
/// `normalize_module_name`, so either spelling works.
///
/// `--stdlib DIR` reads `DIR/src/<module>.rmo` from disk instead of the copy
/// embedded in this binary — the same override `run`/`check`/`doctest` take,
/// for developing the stdlib itself.
pub fn see(args: &[String], stdlib: Option<String>, color: Color) -> ExitCode {
    let Some(name) = args.first() else {
        eprintln!("usage: ramos see [--stdlib DIR] <module>");
        return ExitCode::from(2);
    };
    let wanted = normalize_module_name(name);
    let Some((stem, embedded)) = ramos::loader::STDLIB
        .iter()
        .find(|(stem, _)| normalize_module_name(stem) == wanted)
    else {
        let available: Vec<&str> = ramos::loader::STDLIB
            .iter()
            .map(|(stem, _)| *stem)
            .collect();
        eprintln!("{} no stdlib module named `{name}`", err_tag(color));
        eprintln!("  available: {}", available.join(", "));
        return ExitCode::FAILURE;
    };
    match stdlib {
        None => {
            print!("{embedded}");
            ExitCode::SUCCESS
        }
        Some(dir) => {
            let path = Path::new(&dir).join("src").join(format!("{stem}.rmo"));
            match fs::read_to_string(&path) {
                Ok(source) => {
                    print!("{source}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{} cannot read `{}`: {e}", err_tag(color), path.display());
                    ExitCode::FAILURE
                }
            }
        }
    }
}
