//! Free names of a lambda body — the names it uses but does not bind itself.
//!
//! This backs one rule: a lambda may not refer to the name it is bound to
//! (`f = do x -> f(x)`). Lambdas are anonymous and non-recursive — a named
//! `function` is how you recurse.
//!
//! Beyond the language rule, forbidding self-reference removes the one
//! reference cycle a closure could otherwise form: a lambda stored under a name
//! in the very scope it captured, whose body reaches back to that name, would
//! keep its scope alive forever under `Rc`.

use crate::ast::*;

/// The names a lambda with these `params` and `body` uses freely — variables it
/// reads and bare functions it calls that it does not bind itself. Order is the
/// order encountered; a name may repeat.
pub fn free_names(params: &[String], body: &Block) -> Vec<String> {
    let mut bound: Vec<String> = params.to_vec();
    let mut found = Vec::new();
    block_free(&mut bound, body, &mut found);
    found
}

fn block_free(bound: &mut Vec<String>, block: &Block, out: &mut Vec<String>) {
    let base = bound.len();
    for stmt in block {
        match stmt {
            Stmt::Expr(e) => expr_free(bound, e, out),
            Stmt::Assign { pattern, value } => {
                // The value is evaluated before the pattern binds, so a
                // reference in it still sees the outer name.
                expr_free(bound, value, out);
                collect_pattern(pattern, bound);
            }
            Stmt::Alias { .. } => {}
            // Structurally unreachable inside a lambda body — the parser
            // never lets `return` appear there — but the match still has to
            // be exhaustive over every `Stmt`.
            Stmt::Return(e) => expr_free(bound, e, out),
        }
    }
    bound.truncate(base);
}

fn expr_free(bound: &mut Vec<String>, e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Var(n) => push_free(n, bound, out),
        Expr::Call { callee, args, .. } => {
            match callee {
                Callee::Local(n) => push_free(n, bound, out),
                Callee::Method { target, .. } => expr_free(bound, target, out),
            }
            args.iter().for_each(|a| expr_free(bound, a, out));
        }
        Expr::Access { target, .. } => expr_free(bound, target, out),
        Expr::Unary { operand, .. } => expr_free(bound, operand, out),
        Expr::Binary { left, right, .. } => {
            expr_free(bound, left, out);
            expr_free(bound, right, out);
        }
        Expr::List { elements, rest } => {
            elements.iter().for_each(|x| expr_free(bound, x, out));
            if let Some(r) = rest {
                expr_free(bound, r, out);
            }
        }
        Expr::Tuple(xs) => xs.iter().for_each(|x| expr_free(bound, x, out)),
        Expr::Map(entries) => entries.iter().for_each(|(_, v)| expr_free(bound, v, out)),
        Expr::StructLit { fields, .. } => {
            fields.iter().for_each(|(_, v)| expr_free(bound, v, out));
        }
        Expr::Str(pieces) => pieces.iter().for_each(|p| match p {
            StrPiece::Interp(e) => expr_free(bound, e, out),
            StrPiece::Lit(_) => {}
        }),
        // Nested lambda: its params shadow within its own body. Names still
        // free there are free here too, so a self-reference buried in an inner
        // lambda still counts.
        Expr::Lambda { params, body } => {
            with_bindings(bound, params.iter().cloned(), |b| block_free(b, body, out));
        }
        Expr::Case { subject, arms } => {
            expr_free(bound, subject, out);
            arms.iter().for_each(|arm| case_arm_free(bound, arm, out));
        }
        Expr::Cond { arms } => arms.iter().for_each(|arm| {
            expr_free(bound, &arm.condition, out);
            block_free(bound, &arm.body, out);
        }),
        Expr::If {
            condition,
            then_body,
            else_body,
        } => {
            expr_free(bound, condition, out);
            block_free(bound, then_body, out);
            if let Some(body) = else_body {
                block_free(bound, body, out);
            }
        }
        Expr::Run { body, arms } => {
            block_free(bound, body, out);
            arms.iter().for_each(|arm| case_arm_free(bound, arm, out));
        }
        // Literals and other leaves bind nothing and reference nothing.
        _ => {}
    }
}

fn case_arm_free(bound: &mut Vec<String>, arm: &CaseArm, out: &mut Vec<String>) {
    let mut names = Vec::new();
    collect_pattern(&arm.pattern, &mut names);
    // `pattern = name` binds `name` to the whole value too — bound in the
    // guard and body exactly like any name the pattern itself destructures.
    if let Some(name) = &arm.bind {
        names.push(name.clone());
    }
    with_bindings(bound, names, |b| {
        if let Some(g) = &arm.guard {
            expr_free(b, g, out);
        }
        block_free(b, &arm.body, out);
    });
}

fn push_free(name: &str, bound: &[String], out: &mut Vec<String>) {
    if !bound.iter().any(|b| b == name) {
        out.push(name.to_string());
    }
}

/// Run `f` with `names` pushed onto `bound`, restoring `bound` afterward.
fn with_bindings(
    bound: &mut Vec<String>,
    names: impl IntoIterator<Item = String>,
    f: impl FnOnce(&mut Vec<String>),
) {
    let base = bound.len();
    bound.extend(names);
    f(bound);
    bound.truncate(base);
}

fn collect_pattern(pattern: &Pattern, bound: &mut Vec<String>) {
    match pattern {
        Pattern::Binding(n) => bound.push(n.clone()),
        Pattern::Tuple(ps) => ps.iter().for_each(|p| collect_pattern(p, bound)),
        Pattern::List { elements, rest } => {
            elements.iter().for_each(|p| collect_pattern(p, bound));
            if let Some(r) = rest {
                collect_pattern(r, bound);
            }
        }
        Pattern::Map(entries) => entries.iter().for_each(|(_, p)| collect_pattern(p, bound)),
        Pattern::Struct { fields, .. } => {
            fields.iter().for_each(|(_, p)| collect_pattern(p, bound));
        }
        _ => {}
    }
}
