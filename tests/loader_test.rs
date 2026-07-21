//! Phase 5 acceptance: the module loader, its strict file rules, and the
//! stdlib's own `@doc` examples running against it.

use ramos::interp::{run_with_args, sink, Value};

/// A capturing sink whose bytes can be read back with [`taken`] after the run.
fn capture() -> std::sync::Arc<std::sync::Mutex<Vec<u8>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}
fn taken(buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
    String::from_utf8(std::mem::take(&mut *buf.lock().unwrap())).expect("utf8")
}
use ramos::loader::load;
use std::fs;
use std::path::{Path, PathBuf};

/// A throwaway project directory under the target dir, so tests never write
/// into the source tree and never collide with each other.
struct Project(PathBuf);

impl Project {
    fn new(name: &str) -> Project {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create project dir");
        Project(dir)
    }

    /// Write `contents` to `relative`, creating parent directories.
    fn file(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&path, contents).expect("write source");
        path
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Load and run `entry`, returning what it wrote to stdout.
fn run_file(entry: &Path) -> Result<String, String> {
    let program = load(entry, None).map_err(|e| e.to_string())?;
    let buf = capture();
    run_with_args(&program, buf.clone(), &[]).map_err(|e| e.message)?;
    Ok(taken(&buf))
}

/// Load and run `entry`, returning its final value.
fn value_of(entry: &Path) -> Result<Value, String> {
    value_with_argv(entry, &[])
}

/// Like [`value_of`], with command-line arguments for `get_args` / `get_arg`.
fn value_with_argv(entry: &Path, argv: &[String]) -> Result<Value, String> {
    let program = load(entry, None).map_err(|e| e.to_string())?;
    run_with_args(&program, sink(Vec::new()), argv).map_err(|e| e.message)
}

#[test]
fn an_entrypoint_runs_against_the_embedded_stdlib() {
    let p = Project::new("entrypoint");
    let entry = p.file(
        "app.rmo",
        "\
module App
  fn main()
    println(List.map([1, 2, 3], do x -> x * 2))
    println(String.upcase(\"andrew\"))
",
    );
    assert_eq!(run_file(&entry).unwrap(), "[2, 4, 6]\nANDREW\n");
}

#[test]
fn a_script_of_bare_statements_still_runs() {
    let p = Project::new("script");
    let entry = p.file("script.rmo", "println(List.sum([1, 2, 3]))\n");
    assert_eq!(run_file(&entry).unwrap(), "6\n");
}

#[test]
fn a_namespaced_module_is_found_by_its_path() {
    let p = Project::new("namespaced");
    let entry = p.file(
        "cli.rmo",
        "\
module Cli
  alias MyApp.Business.Greeter

  fn main()
    Greeter.greet_all([\"Andrew\", \"Ana\"])
",
    );
    p.file(
        "src/my_app/business/greeter.rmo",
        "\
module MyApp.Business.Greeter
  fn greet_all(names)
    List.map(names, do n -> \"Ola #{n}\")
",
    );
    assert_eq!(
        value_of(&entry).unwrap().inspect(),
        "[\"Ola Andrew\", \"Ola Ana\"]"
    );
}

#[test]
fn a_module_beside_the_entry_file_is_found_too() {
    let p = Project::new("flat");
    let entry = p.file(
        "app.rmo",
        "\
module App
  fn main()
    Greeter.hi()
",
    );
    p.file(
        "greeter.rmo",
        "\
module Greeter
  fn hi()
    \"hi\"
",
    );
    assert_eq!(value_of(&entry).unwrap().inspect(), "\"hi\"");
}

#[test]
fn loading_is_transitive() {
    let p = Project::new("transitive");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.One

  fn main()
    One.go()
",
    );
    p.file(
        "src/my/one.rmo",
        "\
module My.One
  alias My.Two

  fn go()
    Two.value()
",
    );
    p.file(
        "src/my/two.rmo",
        "\
module My.Two
  fn value()
    42
",
    );
    assert_eq!(value_of(&entry).unwrap().inspect(), "42");
}

#[test]
fn a_file_holds_exactly_one_definition() {
    let p = Project::new("two_defs");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.Pair

  fn main()
    Pair.f()
",
    );
    p.file(
        "src/my/pair.rmo",
        "\
module My.Pair
  fn f()
    1

module My.Other
  fn g()
    2
",
    );
    let err = run_file(&entry).unwrap_err();
    assert!(err.contains("holds 2 definitions"), "{err}");
    assert!(err.contains("My.Pair, My.Other"), "{err}");
}

#[test]
fn the_file_name_must_match_the_definition() {
    let p = Project::new("name_mismatch");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.Greeter

  fn main()
    Greeter.hi()
",
    );
    p.file(
        "src/my/greeter.rmo",
        "\
module My.Salutation
  fn hi()
    \"hi\"
",
    );
    let err = run_file(&entry).unwrap_err();
    assert!(err.contains("belongs in `salutation.rmo`"), "{err}");
}

#[test]
fn the_entry_file_is_not_held_to_the_name_rule() {
    // The naming rule is what lets the loader *find* a module by its namespace.
    // The entry file is named on the command line, so there is nothing to find
    // — a scratch file or a feature demo may be called anything.
    let p = Project::new("entry_name");
    let entry = p.file(
        "scratch.rmo",
        "\
module App
  fn main()
    41 + 1
",
    );
    assert_eq!(value_of(&entry).unwrap().inspect(), "42");
}

#[test]
fn a_namespaced_module_that_names_no_file_is_a_load_error() {
    let p = Project::new("missing");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.Absent.Thing

  fn main()
    Thing.f()
",
    );
    let err = run_file(&entry).unwrap_err();
    assert!(
        err.contains("cannot find module `My.Absent.Thing`"),
        "{err}"
    );
    // The message names where it looked.
    assert!(err.contains("my/absent/thing.rmo"), "{err}");
}

#[test]
fn a_bare_name_that_names_no_file_is_left_to_the_interpreter() {
    // A single segment may be an alias local name or a type module, so a miss
    // is not a load error — it surfaces at the point of use instead.
    let p = Project::new("bare_missing");
    let entry = p.file(
        "app.rmo",
        "\
module App
  fn main()
    Absent.f()
",
    );
    let err = run_file(&entry).unwrap_err();
    assert_eq!(err, "undefined module `Absent`");
}

#[test]
fn the_stdlib_can_be_read_from_disk_instead_of_the_binary() {
    let p = Project::new("stdlib_flag");
    let entry = p.file("app.rmo", "println(String.upcase(\"andrew\"))\n");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("stdlib");
    let program = load(&entry, Some(&dir)).expect("load with --stdlib");
    let buf = capture();
    run_with_args(&program, buf.clone(), &[]).expect("run");
    assert_eq!(taken(&buf), "ANDREW\n");
}

#[test]
fn map_keys_of_every_literal_type_reach_the_stdlib() {
    let p = Project::new("map_keys");
    let cases = [
        (
            "Map.get({\"host\": \"local\"}, \"host\", \"?\")",
            "\"local\"",
        ),
        ("Map.get({8080: :http}, 8080, nil)", ":http"),
        // A string key and the symbol shorthand are different keys.
        ("Map.get({a: 1}, \"a\", :missing)", ":missing"),
        ("Map.keys({a: 1, \"b\": 2, 3: 4})", "[:a, \"b\", 3]"),
        (
            "Map.put({\"host\": 1}, 8080, :port)",
            "{\"host\": 1, 8080: :port}",
        ),
        ("Map.has_key({\"host\": 1}, \"host\")", "true"),
        ("Map.delete({a: 1, 2: :b}, 2)", "{a: 1}"),
    ];
    for (src, want) in cases {
        let entry = p.file("check.rmo", &format!("{src}\n"));
        assert_eq!(value_of(&entry).unwrap().inspect(), want, "{src}");
    }
}

#[test]
fn the_map_module_covers_its_whole_surface() {
    let p = Project::new("map_api");
    let cases = [
        // readers
        ("Map.entries({a: 1, b: 2})", "[(:a, 1), (:b, 2)]"),
        ("Map.keys({a: 1, b: 2})", "[:a, :b]"),
        // Kernel covers counting and the values.
        ("size({a: 1, b: 2})", "2"),
        ("to_list({a: 1, b: 2})", "[1, 2]"),
        ("Map.has_key({a: 1}, :a)", "true"),
        ("Map.get({a: 1}, :b, :none)", ":none"),
        // builders
        ("Map.from_list([(:a, 1), (:b, 2)])", "{a: 1, b: 2}"),
        ("Map.from_list([])", "{}"),
        ("Map.delete({a: 1, b: 2}, :a)", "{b: 2}"),
        ("Map.delete({a: 1}, :z)", "{a: 1}"),
        ("Map.update({a: 1}, :a, do v -> v + 1)", "{a: 2}"),
        ("Map.update({a: 1}, :z, do v -> v + 1)", "{a: 1}"),
        // merging is the `++` operator, not a function
        ("{a: 1} ++ {a: 9, b: 2}", "{a: 9, b: 2}"),
        // filters
        ("Map.filter({a: 1, b: 2}, do k, v -> v > 1)", "{b: 2}"),
        ("Map.reject({a: 1, b: 2}, do k, v -> v > 1)", "{a: 1}"),
        // mappers
        ("Map.map_values({a: 1}, do v -> v * 10)", "{a: 10}"),
        ("Map.map_keys({1: :a}, do k -> k * 10)", "{10: :a}"),
        ("Map.map_entries({a: 1}, do k, v -> (k, v + 1))", "{a: 2}"),
        // round trip
        (
            "Map.from_list(Map.entries({a: 1, \"b\": 2, 3: :c}))",
            "{a: 1, \"b\": 2, 3: :c}",
        ),
    ];
    for (src, want) in cases {
        let entry = p.file("check.rmo", &format!("{src}\n"));
        assert_eq!(value_of(&entry).unwrap().inspect(), want, "{src}");
    }
}

#[test]
fn a_map_key_must_be_an_integer_string_or_symbol_at_runtime() {
    let p = Project::new("map_key_rule");
    // Reached through the stdlib, not just the native.
    let entry = p.file("check.rmo", "Map.put({}, true, 1)\n");
    let err = value_of(&entry).err().unwrap();
    assert!(
        err.contains("a Map key must be an Integer, String or Symbol, got Bool"),
        "{err}"
    );
    // `group_by` with a boolean predicate is the common way to hit it.
    let entry = p.file(
        "check.rmo",
        "List.group_by([1, 2], do x -> Integer.is_even(x))\n",
    );
    let err = value_of(&entry).err().unwrap();
    assert!(err.contains("a Map key must be"), "{err}");
    // Yielding a symbol instead is the fix the docs point at.
    let entry = p.file(
        "check.rmo",
        "\
parity = do x -> cond
  Integer.is_even(x) -> :even
  true -> :odd

List.group_by([1, 2, 3, 4], parity)
",
    );
    assert_eq!(
        value_of(&entry).unwrap().inspect(),
        "{odd: [1, 3], even: [2, 4]}"
    );
}

#[test]
fn tuple_and_list_gained_their_missing_builders() {
    let p = Project::new("tuple_list_api");
    let cases = [
        ("Tuple.from_list([1, 2, 3])", "(1, 2, 3)"),
        ("to_list((1, 2, 3))", "[1, 2, 3]"),
        ("size((1, 2, 3))", "3"),
        // `set` is Ramos, built on to_list/from_list — arity never changes.
        ("Tuple.set((1, 2, 3), 1, :x)", "(1, :x, 3)"),
        ("Tuple.set((1, 2, 3), 0, :a)", "(:a, 2, 3)"),
        ("Tuple.set((1, 2, 3), 2, :c)", "(1, 2, :c)"),
        ("Tuple.set((1, 2, 3), 9, :x)", "(1, 2, 3)"),
        ("Tuple.set((1, 2, 3), -1, :x)", "(1, 2, 3)"),
        // appending is the `++` operator, not a function
        ("[1, 2] ++ [3]", "[1, 2, 3]"),
        ("[] ++ [:only]", "[:only]"),
    ];
    for (src, want) in cases {
        let entry = p.file("check.rmo", &format!("{src}\n"));
        assert_eq!(value_of(&entry).unwrap().inspect(), want, "{src}");
    }
}

// ── Global: the stdlib's own actor ───────────────────────────────────────────

#[test]
fn global_holds_values_across_calls() {
    let p = Project::new("global_state");
    let src = "\
Global.start()
Global.put(\"key\", \"v2\")
Global.put(:port, 8080)
(Global.get(\"key\"), Global.get(:port))
";
    let entry = p.file("app.rmo", src);
    assert_eq!(value_of(&entry).unwrap().inspect(), "(\"v2\", 8080)");
}

#[test]
fn global_reads_a_key_it_never_set_as_nil() {
    let p = Project::new("global_missing");
    let entry = p.file("app.rmo", "Global.start()\nGlobal.get(:nope)\n");
    assert_eq!(value_of(&entry).unwrap().inspect(), "nil");
}

#[test]
fn global_clear_drops_the_key() {
    // `clear` is a cast, so it returns `:ok` at once — but the actor takes its
    // mailbox in order, so the `get` behind it already sees the key gone.
    let p = Project::new("global_clear");
    let src = "\
Global.start()
Global.put(:tmp, 1)
Global.clear(:tmp)
Global.get(:tmp)
";
    let entry = p.file("app.rmo", src);
    assert_eq!(value_of(&entry).unwrap().inspect(), "nil");
}

#[test]
fn global_is_an_ordinary_actor() {
    let p = Project::new("global_actor");
    let src = "\
Global.start()
(is_actor_started(:global), stop_actor(:global), is_actor_started(:global))
";
    let entry = p.file("app.rmo", src);
    assert_eq!(value_of(&entry).unwrap().inspect(), "(true, :ok, false)");
}

// ── Config: the stdlib's read-only settings actor ────────────────────────────

/// `Config` reads a relative path and consults `APP_ENV`, so its tests run the
/// binary as a child process: the working directory and the environment are the
/// inputs, and neither can be set per-test inside one shared process.
fn ramos_in(dir: &Path, app_env: Option<&str>, entry: &str) -> std::process::Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ramos"));
    cmd.args(["run", entry]).current_dir(dir);
    match app_env {
        Some(value) => cmd.env("APP_ENV", value),
        None => cmd.env_remove("APP_ENV"),
    };
    cmd.output().expect("failed to run the ramos binary")
}

const ENV_FILE: &str = "\
# a settings file
[database]
host = \"db.internal\"
port = 5432
password = 'p#ss'   # not part of the value
url = \"postgres://db#main\"

[server]
name = plain value   # trimmed off
";

#[test]
fn config_reads_the_file_app_env_names() {
    let p = Project::new("config_prod");
    p.file(".env.prod", ENV_FILE);
    let src = "\
println(Config.path())
println(Config.start())
println(Config.get(\"database\", \"host\"))
println(Config.get(\"database\", \"port\"))
println(Config.get(\"database\", \"password\"))
println(Config.get(\"database\", \"url\"))
println(Config.get(\"server\", \"name\"))
println(inspect(Config.get(\"database\", \"nope\")))
println(inspect(Config.get(\"nope\", \"host\")))
";
    p.file("app.rmo", src);
    // The name is downcased, so `PROD` finds `.env.prod`.
    let out = ramos_in(&p.0, Some("PROD"), "app.rmo");
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        ".env.prod\n:ok\ndb.internal\n5432\np#ss\npostgres://db#main\nplain value\nnil\nnil\n"
    );
}

#[test]
fn config_falls_back_to_plain_env_without_app_env() {
    let p = Project::new("config_default");
    p.file(".env", "[server]\nname = fallback\n");
    p.file(
        "app.rmo",
        "println(Config.path())\nConfig.start()\nprintln(Config.get(\"server\", \"name\"))\n",
    );
    let out = ramos_in(&p.0, None, "app.rmo");
    assert_eq!(String::from_utf8_lossy(&out.stdout), ".env\nfallback\n");
}

#[test]
fn config_reports_a_missing_file_rather_than_starting() {
    let p = Project::new("config_missing");
    p.file(
        "app.rmo",
        "println(Config.path())\nprintln(Config.start())\n",
    );
    let out = ramos_in(&p.0, Some("staging"), "app.rmo");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        ".env.staging\n(:error, :enoent)\n"
    );
}

// ── the CLI drives the loader ────────────────────────────────────────────────

fn ramos(args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_ramos"))
        .args(args)
        .output()
        .expect("failed to run the ramos binary")
}

#[test]
fn the_cli_runs_and_checks_a_real_project() {
    let p = Project::new("cli_project");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.Business.Greeter

  fn main()
    println(Greeter.greet(\"Andrew\"))
",
    );
    p.file(
        "src/my/business/greeter.rmo",
        "\
module My.Business.Greeter
  fn greet(name)
    String.upcase(\"ola #{name}\")
",
    );
    let entry = entry.to_str().unwrap();

    let out = ramos(&["check", entry]);
    assert!(
        out.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = ramos(&["run", entry]);
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "OLA ANDREW\n");
}

#[test]
fn the_cli_reports_a_load_failure_and_exits_nonzero() {
    let p = Project::new("cli_bad");
    let entry = p.file(
        "app.rmo",
        "\
module App
  alias My.Absent.Thing

  fn main()
    Thing.f()
",
    );
    let out = ramos(&["check", entry.to_str().unwrap()]);
    assert!(!out.status.success(), "expected a non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot find module `My.Absent.Thing`"),
        "{stderr}"
    );
}

#[test]
fn the_cli_passes_program_arguments_through() {
    let p = Project::new("cli_args");
    let entry = p.file("args.rmo", "println(get_arg(0))\n");
    let out = ramos(&["run", entry.to_str().unwrap(), "alice", "42"]);
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "alice\n");
}
