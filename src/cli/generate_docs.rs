//! `ramos generate-docs` — render the stdlib reference into `ramos-docs/`
//! (or `--out DIR`) and exit. The write-to-disk half of what `ramos doc` used
//! to do on its own, for a CI step or anything else that wants the HTML
//! without a server sitting in front of it.

use super::{err_tag, take_opt};
use ramos::color::Color;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

pub fn generate_docs(args: &[String], color: Color) -> ExitCode {
    let mut args: Vec<String> = args.to_vec();
    let out = take_opt(&mut args, "--out");
    let here = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let out_dir = match out {
        Some(s) => PathBuf::from(s),
        None => here.join("ramos-docs"),
    };
    let result = super::doc::build(&mut args, &out_dir);
    if !args.is_empty() {
        eprintln!(
            "usage: ramos generate-docs [--stdlib DIR] [--out DIR] [--examples DIR] [--programs DIR] [--readme FILE]"
        );
        eprintln!("  --stdlib DIR   stdlib root, modules read from DIR/src (default: ./stdlib)");
        eprintln!("  --out DIR      where to write HTML (default: ./ramos-docs)");
        eprintln!(
            "  --examples DIR feature fixtures for the Examples page (default: ./tests/fixtures/features)"
        );
        eprintln!("  --programs DIR runnable programs for the Programs page (default: ./examples)");
        eprintln!("  --readme FILE  markdown for the guide page (default: ./README.md)");
        return ExitCode::from(2);
    }
    match result {
        Ok(n) => {
            println!("generated docs for {n} module(s) in {}", out_dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{} {e}", err_tag(color));
            ExitCode::FAILURE
        }
    }
}
