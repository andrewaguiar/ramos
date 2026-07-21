//! Parser tests: Ramos snippets → expected AST shapes, plus the phase-2
//! acceptance test — the entire stdlib and the all-features fixture parse.

// The README's Geometry example literally uses `3.14`.
#![allow(clippy::approx_constant)]

use ramos::ast::*;
use ramos::lexer::lex;
use ramos::parser::parse;
use std::path::Path;

fn program(src: &str) -> Program {
    let tokens = lex(src).unwrap_or_else(|e| {
        panic!(
            "lex failed: {}",
            ramos::diagnostics::render("<test>", src, &e)
        )
    });
    parse(tokens).unwrap_or_else(|e| {
        panic!(
            "parse failed: {}",
            ramos::diagnostics::render_parse("<test>", src, &e)
        )
    })
}

/// Parse a single statement and return it.
fn stmt(src: &str) -> Stmt {
    let mut p = program(src);
    assert_eq!(p.items.len(), 1, "expected exactly one item");
    match p.items.remove(0) {
        Item::Statement(s) => s,
        other => panic!("expected a statement, got {other:?}"),
    }
}

/// Parse a single expression statement.
fn expr(src: &str) -> Expr {
    match stmt(src) {
        Stmt::Expr(e) => e,
        other => panic!("expected an expression statement, got {other:?}"),
    }
}

fn parse_err(src: &str) -> String {
    let tokens = lex(src).expect("should lex");
    parse(tokens).expect_err("expected a parse error").message
}

fn var(name: &str) -> Expr {
    Expr::Var(name.to_string())
}

fn b(e: Expr) -> Box<Expr> {
    Box::new(e)
}

fn bin(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        left: b(l),
        right: b(r),
    }
}

fn path(segments: &[&str]) -> ModulePath {
    ModulePath(segments.iter().map(|s| s.to_string()).collect())
}

// ── expressions ──────────────────────────────────────────────────────────────

#[test]
fn precedence_arithmetic_binds_tighter_than_comparison() {
    // 1 + 2 * 3 ** 2 == 19  parses as  (1 + (2 * (3 ** 2))) == 19
    assert_eq!(
        expr("1 + 2 * 3 ** 2 == 19"),
        bin(
            BinOp::Eq,
            bin(
                BinOp::Add,
                Expr::Int(1),
                bin(
                    BinOp::Mul,
                    Expr::Int(2),
                    bin(BinOp::Pow, Expr::Int(3), Expr::Int(2))
                ),
            ),
            Expr::Int(19),
        )
    );
}

#[test]
fn unary_minus_and_not() {
    assert_eq!(
        expr("-x ** 2"),
        Expr::Unary {
            op: UnOp::Neg,
            operand: b(bin(BinOp::Pow, var("x"), Expr::Int(2))),
        },
        "`**` binds tighter than unary minus"
    );
    assert_eq!(
        expr("not a and b"),
        bin(
            BinOp::And,
            Expr::Unary {
                op: UnOp::Not,
                operand: b(var("a")),
            },
            var("b"),
        ),
        "`not` binds tighter than `and`"
    );
}

#[test]
fn append_and_concat_operators() {
    assert_eq!(
        expr(r#""a" <> "b""#),
        bin(
            BinOp::Concat,
            Expr::Str(vec![StrPiece::Lit("a".into())]),
            Expr::Str(vec![StrPiece::Lit("b".into())]),
        )
    );
    // ++ is one operator for list concat and map merge
    assert_eq!(
        expr("{a: 1} ++ {b: 2}"),
        bin(
            BinOp::Append,
            Expr::Map(vec![("a".into(), Expr::Int(1))]),
            Expr::Map(vec![("b".into(), Expr::Int(2))]),
        )
    );
}

#[test]
fn calls_local_module_and_method() {
    assert_eq!(
        expr("print(x)"),
        Expr::Call {
            callee: Callee::Local("print".into()),
            args: vec![var("x")],
        }
    );
    assert_eq!(
        expr("List.map(xs, f)"),
        Expr::Call {
            callee: Callee::Method {
                target: b(Expr::ModuleRef(path(&["List"]))),
                name: "map".into(),
            },
            args: vec![var("xs"), var("f")],
        }
    );
    assert_eq!(
        expr("andrew.hello()"),
        Expr::Call {
            callee: Callee::Method {
                target: b(var("andrew")),
                name: "hello".into(),
            },
            args: vec![],
        }
    );
    // dot access without parens is field/const access, not a call
    assert_eq!(
        expr("Payments.max_amount"),
        Expr::Access {
            target: b(Expr::ModuleRef(path(&["Payments"]))),
            name: "max_amount".into(),
        }
    );
    // namespaced module call
    assert_eq!(
        expr("MyApp.Business.SystemUser.create()"),
        Expr::Call {
            callee: Callee::Method {
                target: b(Expr::ModuleRef(path(&["MyApp", "Business", "SystemUser"]))),
                name: "create".into(),
            },
            args: vec![],
        }
    );
}

#[test]
fn pipe_desugars_to_first_argument() {
    // a | List.map(f) | List.join(s)  ⇒  List.join(List.map(a, f), s)
    assert_eq!(
        expr("a\n| List.map(f)\n| List.join(s)"),
        Expr::Call {
            callee: Callee::Method {
                target: b(Expr::ModuleRef(path(&["List"]))),
                name: "join".into(),
            },
            args: vec![
                Expr::Call {
                    callee: Callee::Method {
                        target: b(Expr::ModuleRef(path(&["List"]))),
                        name: "map".into(),
                    },
                    args: vec![var("a"), var("f")],
                },
                var("s"),
            ],
        }
    );
    assert_eq!(
        parse_err("a\n| 1"),
        "the right side of `|` must be a function call"
    );
}

#[test]
fn pipe_must_start_its_own_line() {
    // `x | f()` on one line is rejected; `x` then `| f()` on the next is the
    // only accepted form — same result, different line count.
    let err = parse_err("map | Map.get(:key, nil)");
    assert!(
        err.contains("cannot share a line"),
        "unexpected message: {err}"
    );
    assert_eq!(
        expr("map\n| Map.get(:key, nil)"),
        Expr::Call {
            callee: Callee::Method {
                target: b(Expr::ModuleRef(path(&["Map"]))),
                name: "get".into(),
            },
            args: vec![var("map"), Expr::Symbol("key".into()), Expr::Nil],
        }
    );
}

#[test]
fn pipe_cannot_appear_inside_brackets() {
    // Newlines inside `(` `[` `{` are just whitespace (implicit line
    // joining), so there is no way to put `|` on "its own line" once inside
    // one — a pipe is only ever a complete statement or an assignment's value.
    let err = parse_err("x = (1\n| Integer.abs())");
    assert!(
        err.contains("cannot share a line"),
        "unexpected message: {err}"
    );
}

#[test]
fn string_interpolation_parses_to_expressions() {
    let e = expr(r#""you have #{1 + 1} messages""#);
    assert_eq!(
        e,
        Expr::Str(vec![
            StrPiece::Lit("you have ".into()),
            StrPiece::Interp(bin(BinOp::Add, Expr::Int(1), Expr::Int(1))),
            StrPiece::Lit(" messages".into()),
        ])
    );
}

#[test]
fn struct_literal_vs_map_literal() {
    assert_eq!(
        expr(r#"Person{name: "Andrew", age: 40}"#),
        Expr::StructLit {
            path: path(&["Person"]),
            fields: vec![
                (
                    "name".into(),
                    Expr::Str(vec![StrPiece::Lit("Andrew".into())])
                ),
                ("age".into(), Expr::Int(40)),
            ],
        }
    );
    assert_eq!(
        expr("Person{}"),
        Expr::StructLit {
            path: path(&["Person"]),
            fields: vec![],
        }
    );
    assert_eq!(
        expr("{name: nil}"),
        Expr::Map(vec![("name".into(), Expr::Nil)])
    );
}

#[test]
fn map_keys_may_be_names_strings_or_integers() {
    use ramos::ast::MapKey;
    // A bare name is a symbol key.
    assert_eq!(
        expr("{a: 1}"),
        Expr::Map(vec![(MapKey::Symbol("a".into()), Expr::Int(1))])
    );
    assert_eq!(
        expr("{\"host\": 1}"),
        Expr::Map(vec![(MapKey::Str("host".into()), Expr::Int(1))])
    );
    assert_eq!(
        expr("{8080: 1}"),
        Expr::Map(vec![(MapKey::Int(8080), Expr::Int(1))])
    );
    // Mixed, in written order.
    assert_eq!(
        expr("{a: 1, \"b\": 2, 3: 4}"),
        Expr::Map(vec![
            (MapKey::Symbol("a".into()), Expr::Int(1)),
            (MapKey::Str("b".into()), Expr::Int(2)),
            (MapKey::Int(3), Expr::Int(4)),
        ])
    );
}

#[test]
fn a_symbol_map_key_keeps_only_the_colon_before_its_value() {
    // `{:a: 1}` writes the symbol twice over; there is one way to write it.
    for src in ["{:a: 1}", "{a: 1, :b: 2}", "{:name: \"andrew\"}"] {
        let err = parse_err(src);
        assert!(
            err.contains("without the leading `:`"),
            "should be rejected: {src}\ngot: {err}"
        );
    }
    // The same rule in a pattern, which shares the key parser.
    let src = "\
case m
  {:a: x} -> x
  _ -> nil
";
    assert!(parse_err(src).contains("without the leading `:`"));
    // A symbol is still a symbol everywhere else — only the key position
    // is constrained.
    assert_eq!(
        expr("{a: :b}"),
        Expr::Map(vec![(
            ramos::ast::MapKey::Symbol("a".into()),
            Expr::Symbol("b".into())
        )])
    );
}

#[test]
fn a_struct_literal_still_takes_only_names_as_fields() {
    // Struct fields are declared names, not arbitrary keys — unlike a map.
    assert!(parse_err("Person{\"name\": 1}").contains("key name"));
    assert!(parse_err("Person{42: 1}").contains("key name"));
}

#[test]
fn lambdas_inline_and_multiline() {
    assert_eq!(
        expr("do x, y -> x + y"),
        Expr::Lambda {
            params: vec!["x".into(), "y".into()],
            body: vec![Stmt::Expr(bin(BinOp::Add, var("x"), var("y")))],
        }
    );
    let src = "\
f =
  do x, y
    z = x + y
    z * 2";
    let Stmt::Assign { value, .. } = stmt(src) else {
        panic!("expected assignment")
    };
    let Expr::Lambda { params, body } = value else {
        panic!("expected lambda")
    };
    assert_eq!(params, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(body.len(), 2);
    assert!(matches!(&body[0], Stmt::Assign { .. }));
}

// ── control flow ─────────────────────────────────────────────────────────────

#[test]
fn case_with_patterns_guards_and_block_arms() {
    let e = expr(
        "\
case x
  1 -> :one
  (:ok, value) -> value
  [head | tail] when head > 0 ->
    y = head * 2
    [y] ++ tail
  _ -> :other",
    );
    let Expr::Case { subject, arms } = e else {
        panic!("expected case")
    };
    assert_eq!(*subject, var("x"));
    assert_eq!(arms.len(), 4);
    assert_eq!(arms[0].pattern, Pattern::Int(1));
    assert_eq!(
        arms[1].pattern,
        Pattern::Tuple(vec![
            Pattern::Symbol("ok".into()),
            Pattern::Binding("value".into())
        ])
    );
    assert_eq!(
        arms[2].pattern,
        Pattern::List {
            elements: vec![Pattern::Binding("head".into())],
            rest: Some(Box::new(Pattern::Binding("tail".into()))),
        }
    );
    assert_eq!(
        arms[2].guard,
        Some(bin(BinOp::Gt, var("head"), Expr::Int(0)))
    );
    assert_eq!(arms[2].body.len(), 2, "block arm body has two statements");
    assert_eq!(arms[3].pattern, Pattern::Wildcard);
}

#[test]
fn cond_arms() {
    let e = expr(
        "\
cond
  x > 0 -> :positive
  x < 0 -> :negative
  true -> :zero",
    );
    let Expr::Cond { arms } = e else {
        panic!("expected cond")
    };
    assert_eq!(arms.len(), 3);
    assert_eq!(arms[2].condition, Expr::Bool(true));
}

#[test]
fn begin_rescue_and_raise_are_no_longer_keywords() {
    // Ramos has no exceptions: failure is a returned `(:error, _)` tuple. The
    // three words that used to spell exception handling are now ordinary
    // identifiers, free for a program to use.
    for name in ["begin", "rescue", "raise"] {
        assert_eq!(expr(name), Expr::Var(name.to_string()));
    }
    assert_eq!(
        stmt("raise = 1"),
        Stmt::Assign {
            pattern: Pattern::Binding("raise".into()),
            value: Expr::Int(1),
        }
    );
}

// ── the trailing `when` modifier ─────────────────────────────────────────────

#[test]
fn trailing_when_builds_a_one_branch_if() {
    let Expr::If {
        condition,
        then_body,
        else_body,
    } = expr("print(x) when ready")
    else {
        panic!("expected an if");
    };
    assert_eq!(*condition, var("ready"));
    assert_eq!(then_body.len(), 1);
    assert_eq!(else_body, None, "a trailing guard has no else half");
}

#[test]
fn trailing_when_does_not_collide_with_a_case_guard() {
    // A guard sits in the arm head, before `->`; the modifier is a statement
    // ending. Both `when`s in one arm must land in the right place.
    let e = expr(
        "\
case xs
  [h] when h > 0 ->
    print(h) when ready
  _ -> :none",
    );
    let Expr::Case { arms, .. } = e else {
        panic!("expected a case");
    };
    assert!(arms[0].guard.is_some(), "arm guard lost");
    // The arm body is the guarded statement, which is itself an `if`.
    assert!(
        matches!(&arms[0].body[..], [Stmt::Expr(Expr::If { .. })]),
        "trailing `when` in the arm body did not build an if: {:?}",
        arms[0].body
    );
}

#[test]
fn trailing_if_is_no_longer_a_modifier() {
    // `if` means the two-branch block and nothing else now.
    assert!(parse_err("print(x) if ready").contains("expected end of line"));
}

#[test]
fn trailing_when_cannot_guard_an_assignment() {
    let err = parse_err("x = 1 when ready");
    assert!(err.contains("cannot guard an assignment"), "{err}");
}

// ── assignment & destructuring ───────────────────────────────────────────────

#[test]
fn field_assignment_desugars_to_struct_put() {
    // `andrew.age = 41` rebinds the name rather than mutating the instance.
    let Stmt::Assign { pattern, value } = stmt("andrew.age = 41") else {
        panic!("expected an assignment");
    };
    assert_eq!(pattern, Pattern::Binding("andrew".into()));
    let Expr::Call {
        callee: Callee::Method { name, .. },
        args,
    } = value
    else {
        panic!("expected a Struct.put call");
    };
    assert_eq!(name, "put");
    assert_eq!(
        args,
        vec![
            Expr::Var("andrew".into()),
            Expr::Symbol("age".into()),
            Expr::Int(41),
        ]
    );
}

#[test]
fn field_assignment_on_self_names_the_working_form() {
    // `self` is a name, so this must not claim there is nothing to rebind.
    let err = parse_err("self.age = 41");
    assert!(err.contains("self.age"), "{err}");
    assert!(err.contains("Struct.put(:age"), "{err}");
}

#[test]
fn field_assignment_needs_a_name_to_rebind() {
    let err = parse_err("f().age = 41");
    assert!(err.contains("must be a variable"), "{err}");
}

#[test]
fn destructuring_assignments() {
    assert_eq!(
        stmt("[head | tail] = [1, 2, 3]"),
        Stmt::Assign {
            pattern: Pattern::List {
                elements: vec![Pattern::Binding("head".into())],
                rest: Some(Box::new(Pattern::Binding("tail".into()))),
            },
            value: Expr::List {
                elements: vec![Expr::Int(1), Expr::Int(2), Expr::Int(3)],
                rest: None,
            },
        }
    );
    assert_eq!(
        stmt(r#"(name, age) = ("Andrew", 40)"#),
        Stmt::Assign {
            pattern: Pattern::Tuple(vec![
                Pattern::Binding("name".into()),
                Pattern::Binding("age".into())
            ]),
            value: Expr::Tuple(vec![
                Expr::Str(vec![StrPiece::Lit("Andrew".into())]),
                Expr::Int(40)
            ]),
        }
    );
    // nested, with wildcards
    let Stmt::Assign { pattern, .. } = stmt("((first, _), [head | _]) = ((1, 2), [3, 4, 5])")
    else {
        panic!("expected assignment")
    };
    assert_eq!(
        pattern,
        Pattern::Tuple(vec![
            Pattern::Tuple(vec![Pattern::Binding("first".into()), Pattern::Wildcard]),
            Pattern::List {
                elements: vec![Pattern::Binding("head".into())],
                rest: Some(Box::new(Pattern::Wildcard)),
            },
        ])
    );
    assert!(parse_err("f(x) = 1").contains("left side of `=`"));
}

// ── definitions ──────────────────────────────────────────────────────────────

#[test]
fn module_with_public_and_private_fns() {
    let p = program(
        "\
module Payments
  fn max_amount()
    10000

  fn validate(amount)
    amount <= Payments.max_amount()

  fnp helper(x)
    x
",
    );
    let Item::Module(m) = &p.items[0] else {
        panic!("expected module")
    };
    assert_eq!(m.name, path(&["Payments"]));
    assert_eq!(m.functions.len(), 3);
    assert!(!m.functions[0].private);
    assert!(m.functions[2].private);
    assert_eq!(m.functions[2].name, "helper");
}

#[test]
fn struct_with_implements_and_attributes() {
    let p = program(
        "\
struct Person
  implements Describable

  attributes
    name: nil
    age: 0

  fn hello(self)
    self.name
",
    );
    let Item::Struct(s) = &p.items[0] else {
        panic!("expected struct")
    };
    assert_eq!(s.name, path(&["Person"]));
    assert_eq!(s.implements, vec![path(&["Describable"])]);
    assert_eq!(
        s.attributes,
        vec![
            ("name".to_string(), Expr::Nil),
            ("age".to_string(), Expr::Int(0))
        ]
    );
    assert_eq!(s.functions.len(), 1);
    assert_eq!(s.functions[0].params, vec!["self".to_string()]);
    // strict: implements below fns is rejected
    let below = "\
struct Bad
  fn x()
    1
  implements Foo
";
    assert!(parse_err(below).contains("top of the struct"));
}

#[test]
fn trait_with_required_function() {
    let p = program(
        "\
trait Helloable
  fn hello(self)
    print(self.name)

  fn is_ok(self)
",
    );
    let Item::Trait(t) = &p.items[0] else {
        panic!("expected trait")
    };
    assert_eq!(t.functions.len(), 2);
    assert!(!t.functions[0].body.is_empty(), "default implementation");
    assert!(t.functions[1].body.is_empty(), "required function");
}

#[test]
fn alias_and_top_level_statements() {
    let p = program("alias Geometry.Circle as Circle\nx = Circle.area(5)\n");
    assert_eq!(
        p.items[0],
        Item::Statement(Stmt::Alias {
            module: path(&["Geometry", "Circle"]),
            name: "Circle".to_string(),
        })
    );
    assert!(matches!(p.items[1], Item::Statement(Stmt::Assign { .. })));
}

#[test]
fn top_level_fn_allowed_fnp_rejected() {
    let p = program("fn greet(name)\n  print(name)\n");
    assert!(matches!(&p.items[0], Item::Function(f) if f.name == "greet"));
    assert!(parse_err("fnp secret()\n  42\n").contains("only allowed inside"));
}

#[test]
fn fn_without_body_is_a_declaration() {
    let p = program("module Kernel\n  fn native(str, args)\n\n  fn print(value)\n    native(\"print\", [value])\n");
    let Item::Module(m) = &p.items[0] else {
        panic!("expected module")
    };
    assert!(m.functions[0].body.is_empty());
    assert_eq!(m.functions[1].body.len(), 1);
}

// ── run ──────────────────────────────────────────────────────────────────────

#[test]
fn run_block_without_a_case_has_no_arms() {
    let Expr::Run { body, arms } = expr("run\n  :ok = check()\n  :done\n") else {
        panic!("expected a Run")
    };
    assert_eq!(body.len(), 2);
    assert!(matches!(body[0], Stmt::Assign { .. }));
    assert!(arms.is_empty(), "a bare `run` has no case arms");
}

#[test]
fn run_may_close_with_a_subjectless_case() {
    let Expr::Run { body, arms } = expr("run\n  :ok = check()\ncase\n  :ok -> 1\n  err -> err\n")
    else {
        panic!("expected a Run")
    };
    assert_eq!(body.len(), 1);
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].pattern, Pattern::Symbol("ok".to_string()));
    assert_eq!(arms[1].pattern, Pattern::Binding("err".to_string()));
}

#[test]
fn a_run_case_takes_no_subject() {
    // `case x` after a `run` is the mistake worth a pointed message: the run's
    // result is already the subject.
    let err = parse_err("run\n  :ok = check()\ncase x\n  :ok -> 1\n");
    assert!(err.contains("takes no subject"), "{err}");
}

#[test]
fn run_requires_an_indented_body() {
    assert!(parse_err("run :ok = check()\n").contains("newline after `run`"));
}

#[test]
fn an_assigned_value_may_be_indented_on_the_next_line() {
    let Stmt::Assign { pattern, value } = stmt("result =\n  run\n    :ok = check()\n") else {
        panic!("expected an assignment")
    };
    assert_eq!(pattern, Pattern::Binding("result".to_string()));
    assert!(matches!(value, Expr::Run { .. }));
    // ...and it is not limited to `run`.
    let Stmt::Assign { value, .. } = stmt("total =\n  1 + 2\n") else {
        panic!("expected an assignment")
    };
    assert!(matches!(value, Expr::Binary { .. }));
}

// ── acceptance: stdlib + fixture ─────────────────────────────────────────────

#[test]
fn parses_the_entire_ramos_stdlib() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("stdlib")
        .join("src");
    for name in ["kernel.rmo", "list.rmo", "string.rmo", "tuple.rmo"] {
        let src = std::fs::read_to_string(dir.join(name)).unwrap();
        let tokens = lex(&src).unwrap();
        let program = parse(tokens).unwrap_or_else(|e| {
            panic!("{}", ramos::diagnostics::render_parse(name, &src, &e));
        });
        assert_eq!(program.items.len(), 1, "{name}: one definition per file");
        let Item::Module(m) = &program.items[0] else {
            panic!("{name}: expected a module definition");
        };
        assert!(
            !m.functions.is_empty(),
            "{name}: module should define functions"
        );
    }
}

#[test]
fn parses_the_all_features_fixture() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example.rmo");
    let src = std::fs::read_to_string(&fixture).unwrap();
    let tokens = lex(&src).unwrap();
    let program = parse(tokens).unwrap_or_else(|e| {
        panic!(
            "{}",
            ramos::diagnostics::render_parse("example.rmo", &src, &e)
        );
    });
    // trait + 2 structs + module + alias + 5 top-level statements
    let kinds: Vec<&str> = program
        .items
        .iter()
        .map(|i| match i {
            Item::Trait(_) => "trait",
            Item::Struct(_) => "struct",
            Item::Module(_) => "module",
            Item::Function(_) => "fn",
            Item::Statement(Stmt::Alias { .. }) => "alias",
            Item::Statement(_) => "stmt",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["trait", "struct", "module", "alias", "stmt", "stmt", "stmt", "stmt", "stmt"]
    );
}

// ── patterns bind each name once ─────────────────────────────────────────────

#[test]
fn a_pattern_cannot_bind_the_same_name_twice() {
    // `(p, p)` has no way to mean "these must be equal" — there is no pin — so
    // it is rejected rather than silently keeping the last value. Equal values
    // are rejected too: the shape is wrong, not the data.
    for src in [
        "(p, p) = (1, 1)",
        "(p, p) = (1, 2)",
        "[p, p] = [1, 2]",
        "[p | p] = [1, 2]",
        "{a: p, b: p} = {a: 1, b: 2}",
        "Person{name: p, age: p} = andrew",
        "[(p, 1), (2, p)] = xs",
        "case x\n  (p, p) -> p\n  _ -> 0",
    ] {
        let err = parse_err(src);
        assert!(
            err.contains("cannot bind `p` twice"),
            "should be rejected: {src}\ngot: {err}"
        );
    }
}

#[test]
fn repeated_wildcards_and_separate_arms_are_fine() {
    // `_` binds nothing, so it may repeat.
    assert!(matches!(stmt("(_, _) = (1, 2)"), Stmt::Assign { .. }));
    assert!(matches!(stmt("[_, _ | _] = xs"), Stmt::Assign { .. }));
    // The rule is per pattern: two arms may each bind `p`.
    let e = expr("case x\n  (p, 9) -> p\n  (p, q) -> p\n  _ -> 0");
    assert!(matches!(e, Expr::Case { .. }));
}

// ── one function per name ────────────────────────────────────────────────────

#[test]
fn a_body_cannot_define_the_same_function_name_twice() {
    // Ramos has no arity overloading, so a second definition is unreachable
    // rather than an alternative. `fn` and `fnp` share the one namespace.
    for src in [
        "module Dup\n  fn twice(x)\n    x\n\n  fn twice(x, y)\n    x\n",
        "module Dup\n  fn helper()\n    1\n\n  fnp helper()\n    2\n",
        "struct Dup\n  attributes\n    a: 1\n\n  fn go(self)\n    1\n\n  fn go(self)\n    2\n",
        "trait Dup\n  fn go(self)\n\n  fn go(self)\n    2\n",
    ] {
        let err = parse_err(src);
        assert!(
            err.contains("more than once"),
            "should be rejected: {src}\ngot: {err}"
        );
    }
}

#[test]
fn the_same_name_in_different_modules_is_fine() {
    let p = program("module A\n  fn go()\n    1\n\nmodule B\n  fn go()\n    2\n");
    assert_eq!(p.items.len(), 2);
}

// ── the optional `end` marker ────────────────────────────────────────────────

#[test]
fn end_markers_produce_the_identical_ast() {
    // `end` is stripped by the lexer, so a program with one parses to exactly
    // the same tree as the same program without — one definition, matching.
    let without_end = "module A\n  fn go(x)\n    x + 1\n";
    let with_one_end = "module A\n  fn go(x)\n    x + 1\n  end\n";
    let with_two_ends = "module A\n  fn go(x)\n    x + 1\n  end\nend\n";
    assert_eq!(program(with_one_end), program(without_end));
    assert_eq!(program(with_two_ends), program(without_end));
}

#[test]
fn end_markers_work_after_every_block_construct() {
    // `if`/`else`, `case`, `cond` and a multiline `do` all dedent the same
    // way a `module`/`fn` does, so `end` is legal after every one of them.
    let without_end = "\
fn f(n)
  result =
    cond
      n > 0 -> :pos
      true -> :other
  case result
    :pos -> :yes
    _ -> :no
";
    let with_ends = "\
fn f(n)
  result =
    cond
      n > 0 -> :pos
      true -> :other
    end
  case result
    :pos -> :yes
    _ -> :no
  end
end
";
    assert_eq!(program(with_ends), program(without_end));
}
