//! Phase 4 acceptance: modules as values, `function`/`helper` visibility, `const`,
//! `alias`, `self`, `Module.function()` resolution, `|.` runtime dispatch, and
//! the `native(str, args)` seam the stdlib's Ramos bodies call through.

use ramos::interp::{run, sink, RuntimeError, Value};
use ramos::lexer::lex;
use ramos::parser::parse;

/// A capturing sink whose bytes can be read back with [`taken`] after the run.
fn capture() -> std::sync::Arc<std::sync::Mutex<Vec<u8>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}
fn taken(buf: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
    String::from_utf8(std::mem::take(&mut *buf.lock().unwrap())).expect("utf8")
}

fn eval(src: &str) -> String {
    match try_eval(src) {
        Ok(v) => v.inspect(),
        Err(e) => panic!("`{src}` failed: {e}"),
    }
}

fn try_eval(src: &str) -> Result<Value, String> {
    let tokens = lex(src).map_err(|e| ramos::diagnostics::render("snippet", src, &e))?;
    let program =
        parse(tokens).map_err(|e| ramos::diagnostics::render_parse("snippet", src, &e))?;
    run(&program, sink(Vec::new())).map_err(|e: RuntimeError| e.message)
}

/// The error a snippet fails with; panics if it unexpectedly succeeds.
fn eval_err(src: &str) -> String {
    match try_eval(src) {
        Err(e) => e,
        Ok(v) => panic!("`{src}` unexpectedly succeeded: {}", v.inspect()),
    }
}

fn output(src: &str) -> String {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    let buf = capture();
    run(&program, buf.clone()).expect("run");
    taken(&buf)
}

const CIRCLE: &str = "\
module Geometry.Shapes.Circle
  function pi()
    3.14

  function twice_pi()
    pi() * 2

  function area(r)
    pi() * r * r

  function described(r)
    \"area #{area(r)} (#{tag()})\"

  helper tag()
    \"shape\"
";

#[test]
fn module_functions_are_called_through_their_path() {
    assert_eq!(
        eval(&format!("{CIRCLE}\nGeometry.Shapes.Circle.area(5)")),
        "78.5"
    );
}

#[test]
fn constants_are_functions_like_any_other() {
    // `const` is gone: a constant is a zero-argument function, so it is called.
    assert_eq!(
        eval(&format!("{CIRCLE}\nGeometry.Shapes.Circle.pi()")),
        "3.14"
    );
    // One may call another of the same module by bare name.
    assert_eq!(
        eval(&format!("{CIRCLE}\nGeometry.Shapes.Circle.twice_pi()")),
        "6.28"
    );
}

#[test]
fn a_module_resolves_its_own_functions_by_bare_name() {
    // `described` calls `area`, the private `tag`, and `pi`, all bare.
    assert_eq!(
        eval(&format!("{CIRCLE}\nGeometry.Shapes.Circle.described(2)")),
        "\"area 12.56 (shape)\""
    );
}

#[test]
fn private_functions_are_only_callable_from_inside_the_module() {
    let err = eval_err(&format!("{CIRCLE}\nGeometry.Shapes.Circle.tag()"));
    assert!(err.contains("is private (`helper`)"), "{err}");
}

#[test]
fn module_functions_lists_public_names_in_declaration_order_and_hides_helper() {
    // Backs `Module.functions`; `tag` is `helper` and must not appear.
    assert_eq!(
        eval(&format!(
            "{CIRCLE}\nnative(\"module_functions\", [Geometry.Shapes.Circle])"
        )),
        "[\"pi\", \"twice_pi\", \"area\", \"described\"]"
    );
}

#[test]
fn module_functions_rejects_a_non_module() {
    let err = eval_err("native(\"module_functions\", [42])");
    assert!(err.contains("expected a Module, got Integer"), "{err}");
}

#[test]
fn module_apply_resolves_a_function_by_name_and_calls_it() {
    // Backs `Module.apply`; answered for the bare call, same as `apply`.
    assert_eq!(
        eval(&format!(
            "{CIRCLE}\nmodule_apply(Geometry.Shapes.Circle, \"area\", [5])"
        )),
        "78.5"
    );
}

#[test]
fn module_apply_enforces_helper_visibility_from_outside() {
    let err = eval_err(&format!(
        "{CIRCLE}\nmodule_apply(Geometry.Shapes.Circle, \"tag\", [])"
    ));
    assert!(err.contains("is private (`helper`)"), "{err}");
}

#[test]
fn module_apply_reports_a_missing_function() {
    let err = eval_err(&format!(
        "{CIRCLE}\nmodule_apply(Geometry.Shapes.Circle, \"nope\", [])"
    ));
    assert!(err.contains("has no function `nope`"), "{err}");
}

#[test]
fn alias_drops_the_namespace_prefix() {
    let src = format!("{CIRCLE}\nalias Geometry.Shapes.Circle\nCircle.area(5)");
    assert_eq!(eval(&src), "78.5");
}

#[test]
fn alias_as_renames_to_break_a_collision() {
    let src = format!("{CIRCLE}\nalias Geometry.Shapes.Circle as Round\nRound.area(5)");
    assert_eq!(eval(&src), "78.5");
    // The renamed alias replaces the default short name rather than adding to it.
    let err = eval_err(&format!(
        "{CIRCLE}\nalias Geometry.Shapes.Circle as Round\nCircle.area(5)"
    ));
    assert!(err.contains("undefined module `Circle`"), "{err}");
}

#[test]
fn a_module_aliases_others_inside_its_own_body() {
    let src = "\
module Helper
  function shout(s)
    string_upcase(s)

module Cli
  alias Helper as H

  function main()
    H.shout(\"hi\")
";
    assert_eq!(eval(src), "\"HI\"");
}

#[test]
fn self_refers_to_the_current_module() {
    let src = "\
module Helper
  function shout(s)
    string_upcase(s)

  function twice(s)
    self.shout(s) <> self.shout(s)

Helper.twice(\"hi\")";
    assert_eq!(eval(src), "\"HIHI\"");
    let err = eval_err("self.f()");
    assert!(
        err.contains("`self` is only valid inside a module"),
        "{err}"
    );
}

#[test]
fn a_module_is_itself_a_value() {
    let src = "\
module Helper
  function shout(s)
    string_upcase(s)

m = Helper
m.shout(\"passed around\")";
    assert_eq!(eval(src), "\"PASSED AROUND\"");

    let types = "\
module Helper
  function f()
    1

(is_module(Helper), to_string(Helper))";
    assert_eq!(eval(types), "(true, \"Helper\")");
}

#[test]
fn pipes_thread_the_left_side_as_the_first_argument() {
    let src = "\
module Helper
  function join(a, b)
    a <> b

\"x\"
| Helper.join(\"y\")";
    assert_eq!(eval(src), "\"xy\"");
}

#[test]
fn kernel_is_in_implicit_scope_and_needs_no_alias() {
    // A Ramos-defined Kernel wins over the built-in native table for bare
    // calls, and reaches its host handler through `native`.
    let src = "\
module Kernel
  function print(value)
    native(\"print\", [value])

  function shout(s)
    native(\"string_upcase\", [s])

print(shout(\"hi\"))";
    assert_eq!(output(src), "HI");
}

#[test]
fn the_native_seam_reports_a_bad_shape_or_unknown_handler() {
    let err = eval_err("native(\"no_such_handler\", [])");
    assert!(err.contains("unknown native `no_such_handler`"), "{err}");
    let err = eval_err("native(\"print\", \"not a list\")");
    assert!(err.contains("expected a List, got String"), "{err}");
}

#[test]
fn a_declaration_with_no_body_is_not_callable() {
    let src = "\
module Kernel
  function native(str, args)
    # @doc
    #
    # Declared here, answered by the interpreter.

module M
  function f()
    1

M.g()";
    let err = eval_err(src);
    assert!(err.contains("module `M` has no function `g`"), "{err}");
}

#[test]
fn reading_a_function_without_parens_is_a_pointed_message() {
    let err = eval_err(&format!("{CIRCLE}\nGeometry.Shapes.Circle.area"));
    assert!(err.contains("is a function — call it with"), "{err}");
}

/// The real stdlib sources, concatenated. Phase 5 gives them a proper loader;
/// until then, bundling them into one program is enough to run them — and it
/// exercises the whole phase-4 surface at once (module calls, pipes, Kernel's
/// implicit scope, and every Ramos body reaching its handler via `native`).
fn with_stdlib(src: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("src");
    let mut bundle = String::new();
    for module in ["kernel", "list", "string", "tuple"] {
        let path = dir.join(format!("{module}.rmo"));
        bundle.push_str(&std::fs::read_to_string(&path).expect("stdlib source"));
        bundle.push('\n');
    }
    bundle.push_str(src);
    bundle
}

#[test]
fn the_stdlib_runs_on_the_phase_4_interpreter() {
    let cases = [
        ("List.map([1, 2, 3], do x -> x * 2)", "[2, 4, 6]"),
        ("List.reject([1, 2, 3, 4], do x -> x > 2)", "[1, 2]"),
        ("List.uniq([1, 2, 1, 3, 2])", "[1, 2, 3]"),
        ("List.reduce([1, 2, 3], 0, do acc, x -> acc + x)", "6"),
        ("String.upcase(\"andrew\")", "\"ANDREW\""),
        // Kernel unqualified, and its `size` reaching the native handler.
        ("size([1, 2, 3])", "3"),
        // `|` against the real modules.
        ("\"andrew\"\n| String.upcase()", "\"ANDREW\""),
        ("[3, 1, 2]\n| List.sort()", "[1, 2, 3]"),
        ("[1, 2, 3]\n| List.sum()", "6"),
    ];
    for (src, want) in cases {
        assert_eq!(eval(&with_stdlib(src)), want, "{src}");
    }
}

#[test]
fn dot_access_is_for_structs_not_maps() {
    // A map is addressed by key, not by field: `Map.get` is the one way in.
    let err = eval_err("x = {a: 1}\nx.a");
    assert!(err.contains("cannot read `a` on a Map"), "{err}");
    let err = eval_err("x = {a: 1}\nx.f()");
    assert!(err.contains("cannot call a function on a Map"), "{err}");
}

#[test]
fn a_struct_builds_reads_and_replaces_its_attributes() {
    let src = "\
struct Person
  attributes
    name: nil
    age: 0

  function label(self)
    \"#{self.name}/#{self.age}\"

andrew = Person{name: \"Andrew\", age: 40}";
    // Defaults fill in, the literal overrides, and a method sees `self`.
    assert_eq!(eval(&format!("{src}\nandrew.name")), "\"Andrew\"");
    assert_eq!(eval(&format!("{src}\nPerson{{}}.age")), "0");
    assert_eq!(eval(&format!("{src}\nandrew.label()")), "\"Andrew/40\"");
    // An instance is a value: it inspects as its own literal and knows its type.
    assert_eq!(
        eval(&format!("{src}\ninspect(andrew)")),
        "\"Person{name: \\\"Andrew\\\", age: 40}\""
    );
    assert_eq!(eval(&format!("{src}\ntype_of(andrew)")), "\"Person\"");
    // An undeclared attribute is an error at construction and at read.
    let err = eval_err(&format!("{src}\nPerson{{nickname: \"nope\"}}"));
    assert!(err.contains("has no attribute `nickname`"), "{err}");
    let err = eval_err(&format!("{src}\nandrew.nickname"));
    assert!(err.contains("has no attribute `nickname`"), "{err}");
}

// ── traits ───────────────────────────────────────────────────────────────────

/// A trait with one required function (no body) and one default (with a body),
/// plus a struct that satisfies the requirement.
const GREETABLE: &str = "\
trait Greetable
  function greet(self)

  function loud(self)
    \"#{greet(self)}!\"

struct Dog
  implements Greetable

  attributes
    name: nil

  function greet(self)
    \"Woof from #{self.name}\"
";

#[test]
fn a_struct_inherits_a_traits_default_methods() {
    // `loud` has no definition on Dog, so the trait's default runs — and the
    // default's own call to `greet` resolves to Dog's implementation.
    assert_eq!(
        eval(&format!("{GREETABLE}\nDog{{name: \"Rex\"}}.loud()")),
        "\"Woof from Rex!\""
    );
    assert_eq!(
        eval(&format!("{GREETABLE}\nDog{{name: \"Rex\"}}.greet()")),
        "\"Woof from Rex\""
    );
}

#[test]
fn a_structs_own_function_wins_over_the_traits_default() {
    let src = format!(
        "{GREETABLE}
  function loud(self)
    \"OWN\"
"
    );
    assert_eq!(
        eval(&format!("{src}\nDog{{name: \"Rex\"}}.loud()")),
        "\"OWN\""
    );
}

#[test]
fn implementing_a_trait_requires_its_bodyless_functions() {
    let src = "\
trait Greetable
  function greet(self)

struct Cat
  implements Greetable

  attributes
    name: nil
";
    let err = eval_err(&format!("{src}\nCat{{}}"));
    assert!(err.contains("does not implement"), "{err}");
    assert!(err.contains("greet(self)"), "{err}");
    assert!(err.contains("Greetable"), "{err}");
}

#[test]
fn a_required_function_may_be_satisfied_by_another_traits_default() {
    let src = "\
trait NeedsLabel
  function label(self)

trait HasLabel
  function label(self)
    \"default\"

struct Tag
  implements NeedsLabel
  implements HasLabel

  attributes
    id: 0
";
    assert_eq!(eval(&format!("{src}\nTag{{}}.label()")), "\"default\"");
}

#[test]
fn two_traits_offering_the_same_default_is_ambiguous() {
    let src = "\
trait LoudA
  function shout(self)
    \"A\"

trait LoudB
  function shout(self)
    \"B\"

struct Ox
  implements LoudA
  implements LoudB

  attributes
    id: 0
";
    let err = eval_err(&format!("{src}\nOx{{}}"));
    assert!(err.contains("both `LoudA` and `LoudB`"), "{err}");
    // Defining it on the struct settles the ambiguity.
    // Opens with a newline rather than `"\`, which would eat the indentation
    // of the line that follows it.
    let own = "
  function shout(self)
    \"MINE\"

Ox{}.shout()";
    assert_eq!(eval(&format!("{src}{own}")), "\"MINE\"");
}

#[test]
fn implements_must_name_a_trait() {
    let src = "\
struct Person
  attributes
    name: nil

struct Bird
  implements Person

  attributes
    name: nil
";
    let err = eval_err(&format!("{src}\nBird{{}}"));
    assert!(err.contains("which is not a trait"), "{err}");
}

// ── struct patterns ──────────────────────────────────────────────────────────

const SHAPES: &str = "\
struct Person
  attributes
    name: nil
    age: 0

struct Dog
  attributes
    name: nil
";

#[test]
fn a_struct_pattern_matches_by_name_and_fields() {
    let case = "\
case Person{name: \"Andrew\", age: 40}
  Person{age: 99} -> \"wrong age\"
  Person{name: n, age: 40} -> n
  _ -> \"none\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"Andrew\"");
}

#[test]
fn a_struct_pattern_destructures_in_a_plain_assignment() {
    let src = format!(
        "{SHAPES}
andrew = Person{{name: \"Andrew\", age: 40}}
Person{{name: n, age: a}} = andrew
[n, a]"
    );
    assert_eq!(eval(&src), "[\"Andrew\", 40]");
}

#[test]
fn a_bare_struct_name_is_a_type_test() {
    let case = "\
case Dog{name: \"Rex\"}
  Dog -> \"a dog\"
  _ -> \"none\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"a dog\"");
    // An empty field list is the same test written the other way.
    let case = "\
case Dog{name: \"Rex\"}
  Person{} -> \"wrong\"
  Dog{} -> \"a dog\"
  _ -> \"none\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"a dog\"");
}

#[test]
fn struct_patterns_discriminate_by_struct_not_by_shape() {
    // Person and Dog both declare `name`, so only the name on the pattern
    // tells them apart.
    let case = "\
case Dog{name: \"Rex\"}
  Person{name: n} -> \"person #{n}\"
  Dog{name: n} -> \"dog #{n}\"
  _ -> \"none\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"dog Rex\"");
    // A map is not a struct, and a struct is not a map.
    let case = "\
case {name: \"Andrew\"}
  Person{name: n} -> \"wrong\"
  _ -> \"not a struct\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"not a struct\"");
    let case = "\
case Person{name: \"Andrew\"}
  {name: n} -> \"wrong\"
  _ -> \"not a map\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"not a map\"");
}

#[test]
fn struct_patterns_nest() {
    let case = "\
inner = Person{name: \"Andrew\", age: 40}
case [Dog{name: inner}, 2]
  [Dog{name: Person{name: n}}, x] -> \"#{n}/#{x}\"
  _ -> \"none\"";
    assert_eq!(eval(&format!("{SHAPES}\n{case}")), "\"Andrew/2\"");
}

#[test]
fn a_struct_pattern_binds_nothing_when_the_arm_fails() {
    // Two-phase matching: `n` from the failed arm must not leak into the next.
    let case = "\
n = \"outer\"
r =
  case Person{name: \"Andrew\", age: 40}
    Person{name: n, age: 99} -> \"wrong\"
    _ -> \"fell through\"
[r, n]";
    assert_eq!(
        eval(&format!("{SHAPES}\n{case}")),
        "[\"fell through\", \"outer\"]"
    );
}

#[test]
fn a_struct_pattern_naming_an_undeclared_attribute_is_an_error() {
    // Construction rejects this typo, so matching must too: silently falling
    // through to the next arm would turn it into an invisible branch.
    let src = "\
struct Person
  attributes
    name: nil
    age: 0
";
    let matched = "\
case Person{name: \"A\"}
  Person{nickname: x} -> x
  _ -> :none";
    let err = eval_err(&format!("{src}{matched}"));
    assert!(err.contains("has no attribute `nickname`"), "{err}");
    let err = eval_err(&format!(
        "{src}\nPerson{{nickname: x}} = Person{{name: \"A\"}}"
    ));
    assert!(err.contains("has no attribute `nickname`"), "{err}");
}

#[test]
fn a_missing_map_key_is_an_ordinary_non_match() {
    // A map has no fixed shape, so an absent key is a non-match, not an error —
    // the opposite of a struct, whose attributes are known up front.
    let absent = "\
case {a: 1}
  {zzz: v} -> v
  _ -> :no_match";
    assert_eq!(eval(absent), ":no_match");

    // And a struct pattern against a non-struct simply does not apply.
    let src = "\
struct Person
  attributes
    name: nil
";
    let against_an_int = "\
case 42
  Person{nickname: x} -> x
  _ -> :not_a_struct";
    assert_eq!(eval(&format!("{src}{against_an_int}")), ":not_a_struct");
}

// ── actors ───────────────────────────────────────────────────────────────────

/// A counter actor: the `call` handler is the server half, the plain functions
/// around it the client half.
const COUNTER: &str = "\
trait Actor
  function call(f, args, state, config)

module Counter
  implements Actor

  function call(f, args, state, config)
    case f
      :bump -> (state + 1, state + 1)
      :read -> (state, state)
      :add ->
        [n] = args
        (state + n, state + n)

  function start(id)
    start_actor(id, Counter, 0, {})
";

#[test]
fn an_actor_holds_state_across_calls() {
    let src = format!(
        "{COUNTER}
Counter.start(:c)
call_actor(:c, Counter, :bump, [])
call_actor(:c, Counter, :add, [10])
call_actor(:c, Counter, :read, [])"
    );
    assert_eq!(eval(&src), "11");
}

#[test]
fn actors_with_different_ids_hold_independent_state() {
    let src = format!(
        "{COUNTER}
Counter.start(:a)
Counter.start(:b)
call_actor(:a, Counter, :add, [5])
call_actor(:b, Counter, :add, [100])
(call_actor(:a, Counter, :read, []), call_actor(:b, Counter, :read, []))"
    );
    assert_eq!(eval(&src), "(5, 100)");
}

#[test]
fn config_is_passed_to_every_call_and_never_changes() {
    // The config given at `start_actor` reaches every call untouched, while the
    // state advances.
    let src = "\
trait Actor
  function call(f, args, state, config)

module Stepper
  implements Actor

  function call(f, args, state, config)
    (state + config, state + config)

start_actor(:s, Stepper, 0, 7)
call_actor(:s, Stepper, :go, [])
call_actor(:s, Stepper, :go, [])";
    assert_eq!(eval(src), "14");
}

#[test]
fn starting_requires_a_module_that_implements_actor() {
    let src = "\
module Plain
  function call(f, args, state, config)
    (:ok, state)

start_actor(:p, Plain, 0, {})";
    let err = eval_err(src);
    assert!(err.contains("does not implement `Actor`"), "{err}");
}

#[test]
fn actor_lifecycle_errors_are_named() {
    // not started
    let err = eval_err(&format!("{COUNTER}\ncall_actor(:nope, Counter, :read, [])"));
    assert!(err.contains("is not started"), "{err}");
    // started twice
    let err = eval_err(&format!("{COUNTER}\nCounter.start(:c)\nCounter.start(:c)"));
    assert!(err.contains("is already started"), "{err}");
    // the id belongs to another module
    let src = format!(
        "{COUNTER}
module Other
  implements Actor

  function call(f, args, state, config)
    (:ok, state)

Counter.start(:c)
call_actor(:c, Other, :read, [])"
    );
    let err = eval_err(&src);
    assert!(err.contains("is handled by `Counter`"), "{err}");
}

#[test]
fn a_handler_must_return_a_reply_and_state_pair() {
    let src = "\
trait Actor
  function call(f, args, state, config)

module Bad
  implements Actor

  function call(f, args, state, config)
    :not_a_tuple

start_actor(:b, Bad, 0, {})
call_actor(:b, Bad, :x, [])";
    let err = eval_err(src);
    assert!(err.contains("(reply, new_state) tuple"), "{err}");
}

#[test]
fn an_actor_cannot_reach_other_actors() {
    // Each actor runs on its own thread with its own memory, and that memory
    // does not include the actor registry — so a handler cannot send messages,
    // to another actor or to itself. A self-call would otherwise block the
    // actor waiting on a reply only it could produce.
    let src = "\
trait Actor
  function call(f, args, state, config)

module Loop
  implements Actor

  function call(f, args, state, config)
    call_actor(:l, Loop, :again, [])

start_actor(:l, Loop, 0, {})
call_actor(:l, Loop, :go, [])";
    let err = eval_err(src);
    assert!(err.contains("is not started"), "{err}");
}

#[test]
fn a_module_is_held_to_its_trait_contract_like_a_struct() {
    let src = "\
trait Actor
  function call(f, args, state, config)

module Forgot
  implements Actor

  function other()
    1

Forgot.other()";
    let err = eval_err(src);
    assert!(err.contains("does not implement"), "{err}");
    assert!(err.contains("call(f, args, state, config)"), "{err}");
}

/// A counter whose `cast` is the trait default (a `call` with the reply
/// discarded), plus a logger that overrides `cast` with its own handler.
const CASTABLE: &str = "\
trait Actor
  function cast(f, args, state, config)
    (reply, new_state) = call(f, args, state, config)
    new_state

  function call(f, args, state, config)

module Counter
  implements Actor

  function call(f, args, state, config)
    case f
      :bump -> (state + 1, state + 1)
      :read -> (state, state)
";

#[test]
fn a_cast_returns_ok_without_running_the_handler() {
    // The reply is `:ok` straight away; the state has not moved yet.
    let src = format!(
        "{CASTABLE}
start_actor(:c, Counter, 0, {{}})
cast_actor(:c, Counter, :bump, [])"
    );
    assert_eq!(eval(&src), ":ok");
}

#[test]
fn pending_casts_are_handled_in_order_before_the_next_call() {
    let src = format!(
        "{CASTABLE}
start_actor(:c, Counter, 0, {{}})
cast_actor(:c, Counter, :bump, [])
cast_actor(:c, Counter, :bump, [])
cast_actor(:c, Counter, :bump, [])
call_actor(:c, Counter, :read, [])"
    );
    assert_eq!(eval(&src), "3");
}

#[test]
fn a_trailing_cast_still_runs_when_the_program_ends() {
    // Nothing calls the actor afterwards, so the only chance to handle the
    // message is the end-of-program drain.
    let src = format!(
        "{CASTABLE}
module Probe
  implements Actor

  function call(f, args, state, config)
    (state, state)

  function cast(f, args, state, config)
    print(\"handled\")
    state

start_actor(:p, Probe, 0, {{}})
cast_actor(:p, Probe, :go, [])
print(\"body done\")"
    );
    // The handler's output lands after the body's, not before.
    assert_eq!(output(&src), "body donehandled");
}

#[test]
fn the_default_cast_runs_call_and_discards_the_reply() {
    // Counter defines no `cast`, so the trait default carries the operation.
    let src = format!(
        "{CASTABLE}
start_actor(:c, Counter, 0, {{}})
cast_actor(:c, Counter, :bump, [])
call_actor(:c, Counter, :read, [])"
    );
    assert_eq!(eval(&src), "1");
}

#[test]
fn an_overridden_cast_returns_only_the_new_state() {
    let src = format!(
        "{CASTABLE}
module Doubler
  implements Actor

  function call(f, args, state, config)
    (state, state)

  function cast(f, args, state, config)
    state * 2

start_actor(:d, Doubler, 3, {{}})
cast_actor(:d, Doubler, :go, [])
cast_actor(:d, Doubler, :go, [])
call_actor(:d, Doubler, :read, [])"
    );
    assert_eq!(eval(&src), "12");
}

#[test]
fn cast_lifecycle_errors_match_call() {
    let err = eval_err(&format!(
        "{CASTABLE}\ncast_actor(:nope, Counter, :bump, [])"
    ));
    assert!(err.contains("is not started"), "{err}");
    let src = format!(
        "{CASTABLE}
module Other
  implements Actor

  function call(f, args, state, config)
    (state, state)

start_actor(:c, Counter, 0, {{}})
cast_actor(:c, Other, :bump, [])"
    );
    let err = eval_err(&src);
    assert!(err.contains("is handled by `Counter`"), "{err}");
}

#[test]
fn actors_can_be_listed_and_inspected() {
    let src = format!(
        "{CASTABLE}
start_actor(:alpha, Counter, 0, {{}})
start_actor(:beta, Counter, 0, {{}})
list_actors()"
    );
    // Sorted, so the order does not depend on hashing. Each row is the
    // `(id, module)` pair `call_actor` takes.
    assert_eq!(eval(&src), "[(:alpha, Counter), (:beta, Counter)]");

    let src = format!(
        "{CASTABLE}
start_actor(:alpha, Counter, 0, {{}})
call_actor(:alpha, Counter, :bump, [])
call_actor(:alpha, Counter, :bump, [])
actor_stats(:alpha)"
    );
    assert_eq!(
        eval(&src),
        "{id: :alpha, module: \"Counter\", calls: 2, casts: 0, pending: 0, alive: true}"
    );
}

#[test]
fn is_actor_started_tracks_the_lifecycle() {
    let src = format!(
        "{CASTABLE}
before = is_actor_started(:alpha)
start_actor(:alpha, Counter, 0, {{}})
during = is_actor_started(:alpha)
stop_actor(:alpha)
after = is_actor_started(:alpha)
[before, during, after]"
    );
    assert_eq!(eval(&src), "[false, true, false]");
}

#[test]
fn stopping_an_actor_frees_its_id_and_ends_it() {
    let src = format!(
        "{CASTABLE}
start_actor(:a, Counter, 0, {{}})
call_actor(:a, Counter, :bump, [])
stop_actor(:a)
// a stopped id may be started again, with fresh state
start_actor(:a, Counter, 100, {{}})
call_actor(:a, Counter, :read, [])"
    )
    .replace(
        "// a stopped id may be started again, with fresh state\n",
        "",
    );
    assert_eq!(eval(&src), "100");

    // Sending to a stopped actor is an error, like sending to one never started.
    let err = eval_err(&format!(
        "{CASTABLE}
start_actor(:a, Counter, 0, {{}})
stop_actor(:a)
call_actor(:a, Counter, :read, [])"
    ));
    assert!(err.contains("is not started"), "{err}");
}

#[test]
fn inspecting_an_actor_that_is_not_running_is_an_error() {
    for src in ["actor_stats(:nope)", "stop_actor(:nope)"] {
        let err = eval_err(&format!("{CASTABLE}\n{src}"));
        assert!(err.contains("is not started"), "{src}: {err}");
    }
}

// ── assert & tests ───────────────────────────────────────────────────────────

#[test]
fn assert_passes_quietly_and_fails_with_both_sides() {
    assert_eq!(eval("assert(1 == 1)"), ":ok");
    // Any truthy value passes; only false and nil are falsy.
    assert_eq!(eval("assert([])"), ":ok");

    let err = eval_err("name = \"Andrew\"\nassert(name == \"andrew\")");
    assert!(
        err.contains("expected \"Andrew\" to equal \"andrew\""),
        "{err}"
    );
    let err = eval_err("assert(5 < 3)");
    assert!(err.contains("expected 5 to be less than 3"), "{err}");
    // A non-comparison has no two sides to report.
    let err = eval_err("assert(false)");
    assert_eq!(err, "assertion failed");
    // An explicit message wins over the generated one.
    let err = eval_err("assert(1 == 2, \"ages must match\")");
    assert!(err.contains("ages must match"), "{err}");
}

#[test]
fn a_trait_may_declare_no_functions_at_all() {
    // `Test` is a marker: implementing it means only that a module opted in.
    let src = "\
trait Marker

module Tagged
  implements Marker

  function go()
    42

Tagged.go()";
    assert_eq!(eval(src), "42");
}

/// A test module with one passing test, one failing test, a private helper and
/// a non-`test_` function — only the two tests should run.
const TESTS: &str = "\
trait Test

module MyApp.TestThing
  implements Test

  function test_passes()
    assert(1 == 1)

  function test_fails()
    assert(1 == 2)

  function extra()
    :not_a_test

  helper hidden()
    :also_not
";

fn run_tests(src: &str) -> Vec<(String, String, Option<String>)> {
    let tokens = lex(src).expect("lex");
    let program = parse(tokens).expect("parse");
    ramos::interp::run_tests(&program, sink(Vec::new()), &[])
        .expect("run_tests")
        .into_iter()
        .map(|o| (o.module, o.name, o.failure))
        .collect()
}

#[test]
fn only_public_test_functions_are_run() {
    let outcomes = run_tests(TESTS);
    let names: Vec<&str> = outcomes.iter().map(|(_, n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["test_passes", "test_fails"]);
    assert!(outcomes.iter().all(|(m, _, _)| m == "MyApp.TestThing"));
}

#[test]
fn a_failing_assert_fails_only_its_own_test() {
    let outcomes = run_tests(TESTS);
    assert_eq!(outcomes[0].2, None, "first test should pass");
    let failure = outcomes[1].2.as_deref().expect("second test should fail");
    assert!(failure.contains("expected 1 to equal 2"), "{failure}");
}

#[test]
fn a_module_without_the_test_trait_is_not_run() {
    let src = "\
trait Test

module Plain
  function test_looks_like_one()
    assert(false)
";
    assert!(run_tests(src).is_empty());
}

#[test]
fn a_test_taking_arguments_is_reported_rather_than_called() {
    let src = "\
trait Test

module T
  implements Test

  function test_needs_args(x)
    x
";
    let outcomes = run_tests(src);
    let failure = outcomes[0].2.as_deref().expect("should fail");
    assert!(failure.contains("takes no arguments"), "{failure}");
}

// ── threads ──────────────────────────────────────────────────────────────────
//
// `native("start_thread", ...)` / `native("await_thread", ...)` are the
// intrinsics the `Thread` stdlib module wraps; these exercise them directly,
// without a stdlib loaded. The worker runs on its own large stack, so a
// snippet here only spawns and joins — shallow work on the test thread.

#[test]
fn a_thread_returns_what_its_lambda_produced() {
    let src = "native(\"await_thread\", [native(\"start_thread\", [do -> 6 * 7])])";
    assert_eq!(eval(src), "42");
}

#[test]
fn a_spawned_lambda_carries_its_captured_scope() {
    let src = "\
base = 100
t = native(\"start_thread\", [do -> base + 1])
native(\"await_thread\", [t])";
    assert_eq!(eval(src), "101");
}

#[test]
fn awaiting_a_thread_twice_returns_the_same_value() {
    let src = "\
t = native(\"start_thread\", [do -> 7])
first = native(\"await_thread\", [t])
second = native(\"await_thread\", [t])
(first, second)";
    assert_eq!(eval(src), "(7, 7)");
}

#[test]
fn a_threads_output_is_live_and_the_await_orders_after_it() {
    // Output is live: the thread writes to the same sink as the caller as it
    // runs. The thread's line and the main line race (either order), but
    // everything the thread printed is out by the time the await returns — so
    // "after await" is always last, and each line is whole (a `println` is one
    // locked write).
    let src = "\
t = native(\"start_thread\", [do -> println(\"from thread\")])
println(\"from main\")
native(\"await_thread\", [t])
println(\"after await\")";
    let out = output(src);
    assert!(out.contains("from thread\n"), "{out}");
    assert!(out.contains("from main\n"), "{out}");
    // `await` waits for the thread, so its line precedes anything after it.
    let after = out.find("after await").expect("after await present");
    assert!(out.find("from thread").unwrap() < after, "{out}");
    assert!(out.trim_end().ends_with("after await"), "{out}");
}

#[test]
fn a_failure_inside_a_thread_surfaces_at_the_join() {
    // `at` on a string is a runtime error; it stops the worker and comes back
    // out of `await_thread`.
    let src = "native(\"await_thread\", [native(\"start_thread\", [do -> at(\"hi\", 0)])])";
    let err = eval_err(src);
    assert!(err.contains("Integer") || err.contains("String"), "{err}");
}

#[test]
fn start_thread_rejects_a_lambda_that_takes_arguments() {
    let err = eval_err("native(\"start_thread\", [do x -> x + 1])");
    assert!(err.contains("no arguments"), "{err}");
}

#[test]
fn await_thread_rejects_a_value_that_is_not_a_thread() {
    let err = eval_err("native(\"await_thread\", [42])");
    assert!(err.contains("expects a Thread"), "{err}");
}

#[test]
fn threads_run_in_parallel() {
    // Two 200ms sleeps that overlap finish in well under their 400ms sum. Timed
    // generously so a loaded CI box does not flake.
    use std::time::Instant;
    let src = "\
a = native(\"start_thread\", [do -> sleep(200)])
b = native(\"start_thread\", [do -> sleep(200)])
native(\"await_thread\", [a])
native(\"await_thread\", [b])
:done";
    let start = Instant::now();
    assert_eq!(eval(src), ":done");
    assert!(
        start.elapsed().as_millis() < 380,
        "two overlapping 200ms sleeps took {}ms — not parallel",
        start.elapsed().as_millis()
    );
}
