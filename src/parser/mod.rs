//! Recursive-descent parser: token stream → AST (PLAN phase 2).
//!
//! Operator precedence, loosest to tightest:
//!   pipe (`|`) < or < and < not < comparison
//!   < `<>` / `++` < `+` `-` < `*` `/` `%` < unary `-` < `**` < postfix `.`
//!
//! Three forms are desugared here rather than reaching the tree: `|` (the lhs
//! becomes the call's first argument, or — for `| .f(x)` — the call's dot
//! target instead), `x.field = v` (a `Struct.put` call rebinding `x`), and a
//! sigil (`N"..."` becomes `NaiveDateTime.parse("...")`) — so the evaluator
//! sees one pipe-free, one-assignment, sigil-free shape.

use crate::ast::*;
use crate::diagnostics::Example;
use crate::lexer::{StrPart, Token, TokenKind as T};
use crate::span::Span;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    /// A wrong/correct snippet pair, for a violation of a named strict rule.
    /// `None` for a plain syntax error, which has no "correct" alternative to
    /// show — just a missing or unexpected token.
    pub example: Option<Example>,
}

pub fn parse(tokens: Vec<Token>) -> Result<Program, ParseError> {
    Parser::new(tokens).parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// True right after a block construct (case/cond/begin/multiline lambda)
    /// consumed its DEDENT — such expressions terminate a statement without a
    /// NEWLINE token of their own.
    just_closed_block: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        assert!(
            matches!(tokens.last().map(|t| &t.kind), Some(T::Eof)),
            "token stream must end with Eof"
        );
        Parser {
            tokens,
            pos: 0,
            just_closed_block: false,
        }
    }

    // ── program & definitions ────────────────────────────────────────────

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();
        while !self.check(&T::Eof) {
            items.push(self.parse_item()?);
        }
        Ok(Program {
            items,
            entry_file: Arc::from(""),
        })
    }

    fn parse_item(&mut self) -> Result<Item, ParseError> {
        match self.peek() {
            T::Module => Ok(Item::Module(self.parse_module()?)),
            T::Struct => Ok(Item::Struct(self.parse_struct()?)),
            T::Trait => Ok(Item::Trait(self.parse_trait()?)),
            T::Function => Ok(Item::Function(self.parse_fn(false)?)),
            T::Helper => Err(self.err_here_ex(
                "helpers (`helper`) are only allowed inside a module or struct",
                Example {
                    wrong: "helper log(x)\n  x",
                    correct: "module Payments\n  helper log(x)\n    x",
                },
            )),
            _ => Ok(Item::Statement(self.parse_stmt()?)),
        }
    }

    fn parse_module(&mut self) -> Result<ModuleDef, ParseError> {
        let span = self.span();
        self.expect(&T::Module, "`module`")?;
        let name = self.parse_module_path()?;
        self.expect(&T::Newline, "a newline after the module name")?;
        self.expect(&T::Indent, "an indented module body")?;
        // `implements` then `alias` sit at the top of the body, before any
        // definition — the same order a struct uses.
        let mut implements = Vec::new();
        while self.check(&T::Implements) {
            self.bump();
            implements.push(self.parse_module_path()?);
            self.expect(&T::Newline, "a newline after `implements`")?;
        }
        let aliases = self.parse_aliases(&name)?;
        let mut functions = Vec::new();
        while !self.check(&T::Dedent) {
            match self.peek() {
                T::Function => functions.push(self.parse_fn(false)?),
                T::Helper => functions.push(self.parse_fn(true)?),
                T::Alias => {
                    return Err(self.err_here_ex(
                        "`alias` must be at the top of the module body",
                        Example {
                            wrong: "module Payments\n  function charge()\n    1\n\n  alias Geometry.Circle",
                            correct: "module Payments\n  alias Geometry.Circle\n\n  function charge()\n    1",
                        },
                    ))
                }
                T::Implements => {
                    return Err(self.err_here_ex(
                        "`implements` must be at the top of the module body",
                        Example {
                            wrong: "module Payments\n  function charge()\n    1\n\n  implements Actor",
                            correct: "module Payments\n  implements Actor\n\n  function charge()\n    1",
                        },
                    ))
                }
                _ => {
                    return Err(self.err_here(
                        "only `implements`, `alias`, `function` and `helper` definitions are allowed \
                         inside a module",
                    ))
                }
            }
        }
        self.bump(); // Dedent
        check_unique_fn_names(&name, &functions)?;
        check_helpers_dont_call_functions(&name, &functions)?;
        Ok(ModuleDef {
            name,
            implements,
            aliases,
            functions,
            span,
        })
    }

    fn parse_struct(&mut self) -> Result<StructDef, ParseError> {
        let span = self.span();
        self.expect(&T::Struct, "`struct`")?;
        let name = self.parse_module_path()?;
        self.expect(&T::Newline, "a newline after the struct name")?;
        self.expect(&T::Indent, "an indented struct body")?;
        let mut implements = Vec::new();
        while self.check(&T::Implements) {
            self.bump();
            implements.push(self.parse_module_path()?);
            self.expect(&T::Newline, "a newline after `implements`")?;
        }
        // `implements` then `alias` sit at the top of the body, before any
        // definition — the same order a module uses.
        let aliases = self.parse_aliases(&name)?;
        let mut attributes = Vec::new();
        if self.check(&T::Attributes) {
            self.bump();
            self.expect(&T::Newline, "a newline after `attributes`")?;
            self.expect(&T::Indent, "an indented attributes block")?;
            while !self.check(&T::Dedent) {
                let field = self.expect_ident("an attribute name")?;
                self.expect(&T::Colon, "`:` after the attribute name")?;
                let default = self.parse_expr()?;
                attributes.push((field, default));
                self.stmt_end()?;
            }
            self.bump(); // Dedent
        }
        let mut functions = Vec::new();
        while !self.check(&T::Dedent) {
            match self.peek() {
                T::Function => functions.push(self.parse_fn(false)?),
                T::Helper => functions.push(self.parse_fn(true)?),
                T::Implements => {
                    return Err(self.err_here_ex(
                        "`implements` must be at the top of the struct body",
                        Example {
                            wrong: "struct Account\n  attributes\n    balance: 0\n\n  implements Reportable",
                            correct: "struct Account\n  implements Reportable\n\n  attributes\n    balance: 0",
                        },
                    ))
                }
                T::Alias => {
                    return Err(self.err_here_ex(
                        "`alias` must be at the top of the struct body",
                        Example {
                            wrong: "struct Account\n  attributes\n    balance: 0\n\n  alias Geometry.Circle",
                            correct: "struct Account\n  alias Geometry.Circle\n\n  attributes\n    balance: 0",
                        },
                    ))
                }
                T::Attributes => {
                    return Err(self.err_here_ex(
                        "`attributes` must appear once, before any function definitions",
                        Example {
                            wrong: "struct Account\n  function total(self)\n    self.balance\n\n  attributes\n    balance: 0",
                            correct: "struct Account\n  attributes\n    balance: 0\n\n  function total(self)\n    self.balance",
                        },
                    ))
                }
                _ => return Err(self.err_here(
                    "only `implements`, `alias`, `attributes`, `function` and `helper` are allowed inside a struct",
                )),
            }
        }
        self.bump(); // Dedent
        check_unique_fn_names(&name, &functions)?;
        check_helpers_dont_call_functions(&name, &functions)?;
        Ok(StructDef {
            name,
            implements,
            aliases,
            attributes,
            functions,
            span,
        })
    }

    fn parse_trait(&mut self) -> Result<TraitDef, ParseError> {
        let span = self.span();
        self.expect(&T::Trait, "`trait`")?;
        let name = self.parse_module_path()?;
        self.expect(&T::Newline, "a newline after the trait name")?;
        // A trait with no functions is a *marker*: it declares a contract of
        // nothing, so `implements` says only that a module opted in. `Test` is
        // one — being run is the whole meaning.
        if !self.check(&T::Indent) {
            check_unique_fn_names(&name, &[])?;
            return Ok(TraitDef {
                name,
                functions: Vec::new(),
                span,
            });
        }
        self.bump(); // Indent
        let mut functions = Vec::new();
        while !self.check(&T::Dedent) {
            match self.peek() {
                T::Function => functions.push(self.parse_fn(false)?),
                _ => {
                    return Err(self.err_here_ex(
                        "only `function` definitions are allowed inside a trait",
                        Example {
                            wrong: "trait Shape\n  helper area(self)",
                            correct: "trait Shape\n  function area(self)",
                        },
                    ))
                }
            }
        }
        self.bump(); // Dedent
        check_unique_fn_names(&name, &functions)?;
        Ok(TraitDef {
            name,
            functions,
            span,
        })
    }

    fn parse_fn(&mut self, private: bool) -> Result<FnDef, ParseError> {
        let span = self.span();
        self.bump(); // function | helper
        let name = self.expect_ident("a function name")?;
        self.expect(&T::LParen, "`(` — function definitions require parentheses")?;
        let mut params = Vec::new();
        if !self.check(&T::RParen) {
            loop {
                params.push(self.expect_param()?);
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        self.expect(&T::RParen, "`)` after the parameter list")?;
        self.expect(&T::Newline, "a newline after the function head")?;
        let body = if self.check(&T::Indent) {
            self.bump();
            self.parse_block_until_dedent()?
        } else {
            Vec::new() // declaration: trait requirement or native seam
        };
        Ok(FnDef {
            name,
            params,
            body,
            private,
            span,
            // Stamped for real by the loader, which knows the file; a
            // snippet parsed directly (a test, the REPL) leaves it empty.
            file: Arc::from(""),
        })
    }

    /// Every `alias` line at the top of a module or struct body.
    ///
    /// Two aliases that land on the same local name would leave the first
    /// silently shadowing the second everywhere it is used — the same kind of
    /// quiet surprise a duplicate function name is refused for. `as` exists
    /// precisely to give one of them a different name, so a collision without
    /// it is a parse error rather than a silent pick.
    fn parse_aliases(
        &mut self,
        owner: &ModulePath,
    ) -> Result<Vec<(String, ModulePath)>, ParseError> {
        let mut aliases: Vec<(String, ModulePath)> = Vec::new();
        while self.check(&T::Alias) {
            let span = self.span();
            let (name, module) = self.parse_alias()?;
            if let Some((_, first)) = aliases.iter().find(|(n, _)| *n == name) {
                return Err(self.err_at_ex(
                    span,
                    &format!(
                        "`{owner}` aliases both `{first}` and `{module}` as `{name}` — \
                         give one an `as` name to tell them apart"
                    ),
                    Example {
                        wrong: "alias MyApp.Business.Account\nalias MyApp.System.Account",
                        correct: "alias MyApp.Business.Account\nalias MyApp.System.Account as SystemAccount",
                    },
                ));
            }
            aliases.push((name, module));
        }
        Ok(aliases)
    }

    /// `alias Geometry.Shapes.Circle [as Name]` — returns (local name, path).
    /// Without `as`, the local name is the path's last segment, which is what
    /// most aliases want; `as` exists to break a collision between two of them.
    fn parse_alias(&mut self) -> Result<(String, ModulePath), ParseError> {
        self.expect(&T::Alias, "`alias`")?;
        let module = self.parse_module_path()?;
        let name = if self.eat(&T::As) {
            self.expect_upper("an alias name")?
        } else {
            module
                .0
                .last()
                .expect("a module path has at least one segment")
                .clone()
        };
        self.stmt_end()?;
        Ok((name, module))
    }

    // ── statements & blocks ──────────────────────────────────────────────

    fn parse_block_until_dedent(&mut self) -> Result<Block, ParseError> {
        let mut stmts = Vec::new();
        while !self.check(&T::Dedent) {
            stmts.push(self.parse_stmt()?);
        }
        self.bump(); // Dedent
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        if self.check(&T::Alias) {
            let (name, module) = self.parse_alias()?;
            return Ok(Stmt::Alias { module, name });
        }
        let expr = self.parse_expr()?;
        let stmt = if self.check(&T::Assign) {
            let at = self.span();
            let line = self.line();
            self.bump();
            // `andrew.age = 41` is field-assignment sugar, not a pattern.
            if let Expr::Access { target, name } = expr {
                let value = self.parse_assign_rhs()?;
                self.field_assign(*target, name, value, at, line)?
            } else {
                let pattern = self.expr_to_pattern(expr, at)?;
                check_bindings_are_unique(&pattern, None, at)?;
                let value = self.parse_assign_rhs()?;
                Stmt::Assign { pattern, value }
            }
        } else {
            Stmt::Expr(expr)
        };
        let stmt = self.parse_when_modifier(stmt)?;
        self.stmt_end()?;
        Ok(stmt)
    }

    /// `andrew.age = 41` — sugar for `andrew = Struct.put(andrew, :age, 41)`.
    ///
    /// It is a *rebinding*, not a mutation: the instance is untouched and the
    /// name is bound to a new one, which is why the left side has to be a plain
    /// variable. `f().x = 1` has no name to rebind, so it is rejected rather
    /// than silently discarding the result.
    fn field_assign(
        &mut self,
        target: Expr,
        field: String,
        value: Expr,
        at: Span,
        line: usize,
    ) -> Result<Stmt, ParseError> {
        let binding = match target {
            Expr::Var(binding) => binding,
            // `self` is a name, so the "nothing to rebind" message would be
            // wrong here. Rebinding it would bind a new instance to the
            // *local* `self` and leave the caller's untouched — quietly doing
            // nothing — so name the form that does work instead.
            Expr::SelfRef => {
                return Err(self.err_at_ex(
                    at,
                    &format!(
                        "`self.{field} = ...` does not update the caller's instance — \
                         rebinding `self` is local to this function. Return a new instance \
                         instead, as in `self | Struct.put(:{field}, ...)`"
                    ),
                    Example {
                        wrong: "function withdraw(self, amount)\n  self.balance = self.balance - amount",
                        correct: "function withdraw(self, amount)\n  self | Struct.put(:balance, self.balance - amount)",
                    },
                ));
            }
            _ => {
                return Err(self.err_at_ex(
                    at,
                    "the left of a field assignment must be a variable — \
                     `andrew.age = 41` rebinds `andrew`, so there has to be a name \
                     to rebind",
                    Example {
                        wrong: "find_user().age = 41",
                        correct: "andrew = find_user()\nandrew.age = 41",
                    },
                ));
            }
        };
        Ok(Stmt::Assign {
            pattern: Pattern::Binding(binding.clone()),
            value: Expr::Call {
                callee: Callee::Method {
                    target: Box::new(Expr::ModuleRef(ModulePath(vec!["Struct".to_string()]))),
                    name: "put".to_string(),
                },
                args: vec![Expr::Var(binding), Expr::Symbol(field), value],
                line,
            },
        })
    }

    /// The trailing `when`: `print(x) when ready`, one statement guarded by one
    /// condition. It builds the same `Expr::If` the block form does, so there is
    /// one conditional in the tree and nothing downstream has to know.
    ///
    /// `when` rather than `if` so that `if` means exactly one thing — the
    /// two-branch block — and the guard reads as the guard it already is in a
    /// `case` arm. The two never collide: a guard is parsed inside an arm head,
    /// before `->`, and an arm body takes no modifier at all.
    ///
    /// Assignment is rejected: the guarded statement runs in a child scope, so
    /// the binding in `x = 1 when ready` could never escape to the next line,
    /// and a binding that silently goes nowhere is worse than a parse error.
    fn parse_when_modifier(&mut self, stmt: Stmt) -> Result<Stmt, ParseError> {
        // A block expression consumes its own line ending, so a `when` sitting
        // after one starts the *next* statement — only a `when` still on this
        // line is a modifier.
        if self.just_closed_block || !self.check(&T::When) {
            return Ok(stmt);
        }
        let at = self.span();
        if matches!(stmt, Stmt::Assign { .. }) {
            return Err(self.err_at_ex(
                at,
                "a trailing `when` cannot guard an assignment — the binding would not \
                 escape the branch; use a block `if` around it",
                Example {
                    wrong: "x = 1 when ready",
                    correct: "if ready\n  x = 1",
                },
            ));
        }
        self.bump();
        let condition = self.parse_expr()?;
        Ok(Stmt::Expr(Expr::If {
            condition: Box::new(condition),
            then_body: vec![stmt],
            else_body: None,
        }))
    }

    /// The value of an assignment: normally the rest of the line, but a block
    /// construct too tall to sit after `=` may start on the next line, indented
    /// one level (`result =` / newline / `run`).
    fn parse_assign_rhs(&mut self) -> Result<Expr, ParseError> {
        if !(self.check(&T::Newline) && matches!(self.nth(1), T::Indent)) {
            return self.parse_expr();
        }
        self.bump(); // Newline
        self.bump(); // Indent
        let value = self.parse_expr()?;
        // A block construct ate its own trailing NEWLINE with its DEDENT.
        if !self.just_closed_block {
            self.expect(&T::Newline, "end of line after the assigned value")?;
        }
        self.expect(&T::Dedent, "the indented value to be a single expression")?;
        self.just_closed_block = true;
        Ok(value)
    }

    /// A statement ends with NEWLINE, or implicitly when a block construct
    /// just consumed its DEDENT, or at DEDENT/EOF.
    fn stmt_end(&mut self) -> Result<(), ParseError> {
        if self.eat(&T::Newline) {
            return Ok(());
        }
        if self.just_closed_block || self.check(&T::Dedent) || self.check(&T::Eof) {
            self.just_closed_block = false;
            return Ok(());
        }
        Err(self.err_here("expected end of line"))
    }

    // ── expressions ──────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe()
    }

    /// `|` always starts its own line: `x` then `| f()` on the next, never
    /// `x | f()`. Consuming it therefore always happens right after
    /// consuming the NEWLINE that precedes it; a bare `Pipe` reached any
    /// other way shared a line with its left-hand side and is rejected below.
    fn parse_pipe(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_or()?;
        loop {
            if self.check(&T::Newline) && matches!(self.nth(1), T::Pipe) {
                self.bump(); // the NEWLINE — `|` itself is consumed below
            } else if self.check(&T::Pipe) {
                return Err(self.err_here_ex(
                    "`|` cannot share a line with its left-hand side: \
                     put it on the next line, at the same indentation",
                    Example {
                        wrong: "map | Map.get(:key, nil)",
                        correct: "map\n| Map.get(:key, nil)",
                    },
                ));
            } else {
                break;
            }
            let at = self.span();
            self.bump(); // the Pipe
                         // `| .method(args)` / `| .field` — sugar for `left.method(args)` /
                         // `left.field`: a pipe chain that dispatches on whatever flowed
                         // in, without repeating its name. Unlike the general `| f(args)`
                         // rule below (which *adds* `left` as `f`'s first argument), this
                         // makes `left` the dot's target — the very tree `left.method(args)`
                         // builds directly, so it goes through the same runtime-type
                         // dispatch an OO-style call does, and method dispatch's own
                         // self-prepending at eval time only happens once.
            if self.check(&T::Dot) {
                let line = self.line();
                self.bump();
                let name = self.expect_ident("a field or function name after `.`")?;
                left = if self.check(&T::LParen) {
                    let args = self.parse_call_args(false)?;
                    Expr::Call {
                        callee: Callee::Method {
                            target: Box::new(left),
                            name,
                        },
                        args,
                        line,
                    }
                } else {
                    Expr::Access {
                        target: Box::new(left),
                        name,
                    }
                };
                continue;
            }
            let rhs = self.parse_or()?;
            left = match rhs {
                Expr::Call {
                    callee,
                    mut args,
                    line,
                } => {
                    args.insert(0, left);
                    Expr::Call { callee, args, line }
                }
                _ => {
                    return Err(self.err_at_ex(
                        at,
                        "the right side of `|` must be a function call",
                        Example {
                            wrong: "x\n| 1",
                            correct: "x\n| Integer.abs()",
                        },
                    ))
                }
            };
        }
        Ok(left)
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while self.eat(&T::Or) {
            let right = self.parse_and()?;
            left = bin(BinOp::Or, left, right);
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not()?;
        while self.eat(&T::And) {
            let right = self.parse_not()?;
            left = bin(BinOp::And, left, right);
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if self.eat(&T::Not) {
            let operand = self.parse_not()?;
            return Ok(Expr::Unary {
                op: UnOp::Not,
                operand: Box::new(operand),
            });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_concat()?;
        loop {
            let op = match self.peek() {
                T::EqEq => BinOp::Eq,
                T::NotEq => BinOp::NotEq,
                T::Lt => BinOp::Lt,
                T::Gt => BinOp::Gt,
                T::Le => BinOp::Le,
                T::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let right = self.parse_concat()?;
            left = bin(op, left, right);
        }
        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                T::Concat => BinOp::Concat,
                T::PlusPlus => BinOp::Append,
                _ => break,
            };
            self.bump();
            let right = self.parse_additive()?;
            left = bin(op, left, right);
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                T::Plus => BinOp::Add,
                T::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let right = self.parse_multiplicative()?;
            left = bin(op, left, right);
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                T::Star => BinOp::Mul,
                T::Slash => BinOp::Div,
                T::Percent => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let right = self.parse_unary()?;
            left = bin(op, left, right);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat(&T::Minus) {
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                operand: Box::new(operand),
            });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_postfix()?;
        if self.eat(&T::StarStar) {
            let exp = self.parse_unary()?; // right-associative
            return Ok(bin(BinOp::Pow, base, exp));
        }
        Ok(base)
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        while self.check(&T::Dot) {
            let line = self.line();
            self.bump();
            let name = self.expect_ident("a field or function name after `.`")?;
            if self.check(&T::LParen) {
                let args = self.parse_call_args(false)?;
                expr = Expr::Call {
                    callee: Callee::Method {
                        target: Box::new(expr),
                        name,
                    },
                    args,
                    line,
                };
            } else {
                expr = Expr::Access {
                    target: Box::new(expr),
                    name,
                };
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            T::Int(n) => {
                self.bump();
                Ok(Expr::Int(n))
            }
            T::Float(x) => {
                self.bump();
                Ok(Expr::Float(x))
            }
            T::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            T::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            T::Nil => {
                self.bump();
                Ok(Expr::Nil)
            }
            T::Symbol(s) => {
                self.bump();
                Ok(Expr::Symbol(s))
            }
            T::Str(parts) => {
                let at = self.span();
                self.bump();
                Ok(Expr::Str(self.convert_str_parts(parts, at)?))
            }
            T::Sigil(letter, text) => {
                let line = self.line();
                self.bump();
                Ok(sigil_call(letter, text, line))
            }
            T::Underscore => {
                self.bump();
                Ok(Expr::Wildcard)
            }
            T::SelfKw => {
                self.bump();
                Ok(Expr::SelfRef)
            }
            T::Ident(name) => {
                let line = self.line();
                self.bump();
                if self.check(&T::LParen) {
                    let args = self.parse_call_args(is_actor_message_fn(&name))?;
                    Ok(Expr::Call {
                        callee: Callee::Local(name),
                        args,
                        line,
                    })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            T::UpperIdent(_) => {
                let path = self.parse_module_path()?;
                if self.check(&T::LBrace) {
                    let fields = self.parse_field_exprs()?;
                    Ok(Expr::StructLit { path, fields })
                } else {
                    Ok(Expr::ModuleRef(path))
                }
            }
            T::LParen => {
                self.bump();
                let first = self.parse_expr()?;
                if self.eat(&T::Comma) {
                    let mut elements = vec![first];
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.eat(&T::Comma) {
                            break;
                        }
                    }
                    self.expect(&T::RParen, "`)` to close the tuple")?;
                    Ok(Expr::Tuple(elements))
                } else {
                    self.expect(&T::RParen, "`)` to close the group")?;
                    Ok(first)
                }
            }
            T::LBracket => {
                self.bump();
                if self.eat(&T::RBracket) {
                    return Ok(Expr::List {
                        elements: Vec::new(),
                        rest: None,
                    });
                }
                let mut elements = Vec::new();
                let mut rest = None;
                loop {
                    // below pipe precedence: `|` inside brackets is cons
                    elements.push(self.parse_or()?);
                    if self.eat(&T::Comma) {
                        continue;
                    }
                    if self.eat(&T::Pipe) {
                        rest = Some(Box::new(self.parse_or()?));
                    }
                    break;
                }
                self.expect(&T::RBracket, "`]` to close the list")?;
                Ok(Expr::List { elements, rest })
            }
            T::LBrace => {
                let entries = self.parse_map_entries()?;
                Ok(Expr::Map(entries))
            }
            T::Do => self.parse_lambda(),
            T::Case => self.parse_case(),
            T::Cond => self.parse_cond(),
            T::If => self.parse_if(),
            T::Run => self.parse_run(),
            _ => Err(self.err_here("expected an expression")),
        }
    }

    /// A map literal key: `name` (a symbol), a string, or an integer.
    /// Struct fields are always plain names, so they keep `parse_field_exprs`.
    ///
    /// A symbol key carries exactly one `:`, the one that separates it from its
    /// value. `{:name: 1}` writes the same key twice over and is rejected — the
    /// language gives one way to write a thing, and `{name: 1}` is it.
    fn parse_map_key(&mut self) -> Result<MapKey, ParseError> {
        match self.peek().clone() {
            T::Ident(name) => {
                self.bump();
                Ok(MapKey::Symbol(name))
            }
            T::Symbol(name) => Err(self.err_here_ex(
                &format!(
                    "a map key writes its symbol without the leading `:` — \
                     use `{name}: ...`, not `:{name}: ...`"
                ),
                Example {
                    wrong: "{:name: 1}",
                    correct: "{name: 1}",
                },
            )),
            T::Int(n) => {
                self.bump();
                Ok(MapKey::Int(n))
            }
            T::Str(parts) => {
                // Only a plain string is a key — interpolation would make the
                // key depend on runtime state, which a literal cannot.
                self.bump();
                match parts.as_slice() {
                    [] => Ok(MapKey::Str(String::new())),
                    [StrPart::Lit(text)] => Ok(MapKey::Str(text.clone())),
                    _ => Err(self.err_here_ex(
                        "a map key string cannot interpolate — use a plain string",
                        Example {
                            wrong: "{\"#{key}\": 1}",
                            correct: "{\"key\": 1}",
                        },
                    )),
                }
            }
            _ => Err(self.err_here("a map key: a name, string, integer or symbol")),
        }
    }

    /// `{key: expr, ...}` — a map literal.
    fn parse_map_entries(&mut self) -> Result<Vec<(MapKey, Expr)>, ParseError> {
        self.expect(&T::LBrace, "`{`")?;
        let mut entries = Vec::new();
        if !self.check(&T::RBrace) {
            loop {
                let key = self.parse_map_key()?;
                self.expect(&T::Colon, "`:` after the key")?;
                entries.push((key, self.parse_expr()?));
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        self.expect(&T::RBrace, "`}`")?;
        Ok(entries)
    }

    /// `{name: expr, ...}` — struct literals, whose fields are always names.
    fn parse_field_exprs(&mut self) -> Result<Vec<(String, Expr)>, ParseError> {
        self.expect(&T::LBrace, "`{`")?;
        let mut fields = Vec::new();
        if !self.check(&T::RBrace) {
            loop {
                let key = self.expect_ident("a key name")?;
                self.expect(&T::Colon, "`:` after the key")?;
                fields.push((key, self.parse_expr()?));
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        self.expect(&T::RBrace, "`}`")?;
        Ok(fields)
    }

    /// Strict layout rules for a call's arguments:
    ///
    /// - a `do` lambda passed directly as an argument must fit on one line —
    ///   a multi-line one is written first as `callback = do ...`, then
    ///   passed by name, so the call site stays scannable
    /// - once the arguments spill past one line, every argument starts its
    ///   own line — the first cannot share the line with `(`, and no two can
    ///   share a line with each other
    /// - `start_actor`/`call_actor`/`cast_actor` never take a `do` lambda,
    ///   even nested in a list/tuple/map literal argument — an actor's
    ///   handler runs on another thread with a fresh root scope, so a lambda
    ///   sent to it could never reach the bindings it closed over
    fn parse_call_args(&mut self, is_actor_call: bool) -> Result<Vec<Expr>, ParseError> {
        let open = self.expect(&T::LParen, "`(` — function calls require parentheses")?;
        let mut args = Vec::new();
        let mut arg_starts: Vec<Token> = Vec::new();
        if !self.check(&T::RParen) {
            loop {
                let start = self.tokens[self.pos].clone();
                let arg = self.parse_expr()?;
                if matches!(arg, Expr::Lambda { .. }) {
                    let end_line = self.tokens[self.pos - 1].line;
                    if end_line != start.line {
                        return Err(self.err_at_ex(
                            start.span,
                            "a `do` lambda passed directly as a call argument must fit on \
                             one line — bind it to a name first (`callback = do ...`), then \
                             pass `callback`",
                            Example {
                                wrong: "SomeProcess.process_and_call_back(\n  [1, 2, 3],\n  do x ->\n    print(x)\n)",
                                correct: "callback =\n  do x\n    print(x)\n\nSomeProcess.process_and_call_back([1, 2, 3], callback)",
                            },
                        ));
                    }
                }
                if is_actor_call && expr_embeds_lambda(&arg) {
                    return Err(self.err_at_ex(
                        start.span,
                        "a `do` lambda cannot be passed to an actor — its handler runs on \
                         another thread with a fresh root scope, so it could never reach the \
                         bindings the lambda closed over",
                        Example {
                            wrong: "call_actor(:cache, Cache, :process, [do x -> x + 1])",
                            correct: "call_actor(:cache, Cache, :process, [x])\n\n# the actor's own `call` does the work:\n# function call(f, args, state, config)\n#   case f\n#     :process ->\n#       [x] = args\n#       (x + 1, state)",
                        },
                    ));
                }
                arg_starts.push(start);
                args.push(arg);
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        let close = self.expect(&T::RParen, "`)` after the arguments")?;
        if open.line != close.line {
            if let Some(first) = arg_starts.first() {
                if first.line == open.line {
                    return Err(self.err_at_ex(
                        first.span,
                        "once a call's arguments spill past one line, the first argument \
                         cannot share the line with `(` — give it its own line, or put the \
                         whole call on one line",
                        Example {
                            wrong: "SomeProcess.process([1, 2],\n  \"a\"\n)",
                            correct: "SomeProcess.process(\n  [1, 2],\n  \"a\"\n)",
                        },
                    ));
                }
            }
            for pair in arg_starts.windows(2) {
                if pair[0].line == pair[1].line {
                    return Err(self.err_at_ex(
                        pair[1].span,
                        "once a call's arguments spill past one line, every argument is on \
                         its own line",
                        Example {
                            wrong: "SomeProcess.process(\n  [1, 2], \"a\",\n  \"b\"\n)",
                            correct: "SomeProcess.process(\n  [1, 2],\n  \"a\",\n  \"b\"\n)",
                        },
                    ));
                }
            }
        }
        Ok(args)
    }

    /// `do x, y -> x + y`, or `do x, y` with an indented body.
    ///
    /// `do` is the keyword; the value it builds is a lambda, which is what the
    /// AST, the `Lambda` type name and `is_lambda` all call it.
    fn parse_lambda(&mut self) -> Result<Expr, ParseError> {
        self.expect(&T::Do, "`do`")?;
        let mut params = Vec::new();
        while !matches!(self.peek(), T::Arrow | T::Newline) {
            params.push(self.expect_param()?);
            if !self.eat(&T::Comma) {
                break;
            }
        }
        let body = if self.eat(&T::Arrow) {
            vec![Stmt::Expr(self.parse_expr()?)]
        } else {
            self.expect(&T::Newline, "`->` or an indented lambda body")?;
            self.expect(&T::Indent, "an indented lambda body")?;
            let block = self.parse_block_until_dedent()?;
            self.just_closed_block = true;
            block
        };
        Ok(Expr::Lambda { params, body })
    }

    fn parse_case(&mut self) -> Result<Expr, ParseError> {
        self.expect(&T::Case, "`case`")?;
        let subject = self.parse_expr()?;
        self.expect(&T::Newline, "a newline after the case subject")?;
        let arms = self.parse_case_arms()?;
        self.just_closed_block = true;
        Ok(Expr::Case {
            subject: Box::new(subject),
            arms,
        })
    }

    /// The indented arms of a `case`, consuming the INDENT and its DEDENT.
    /// Shared by `case` and the subject-less `case` a `run` may end with.
    fn parse_case_arms(&mut self) -> Result<Vec<CaseArm>, ParseError> {
        self.expect(&T::Indent, "at least one indented case arm")?;
        let mut arms = Vec::new();
        while !self.check(&T::Dedent) {
            let at = self.span();
            let pattern = self.parse_pattern()?;
            // `pattern = name` binds the whole value the arm matched to
            // `name`, alongside whatever `pattern` itself destructures —
            // `Person{name: n} = p -> ...` gets both `n` and `p`. Checked
            // for the same collision a repeated pattern name is: binding
            // `name` to the whole value while `pattern` also binds it (in
            // part or in full) under the same name can never make sense.
            let bind = if self.check(&T::Assign) {
                self.bump();
                Some(self.expect_ident("a name after `=`")?)
            } else {
                None
            };
            check_bindings_are_unique(&pattern, bind.as_deref(), at)?;
            let guard = if self.eat(&T::When) {
                Some(self.parse_or()?)
            } else {
                None
            };
            self.expect(&T::Arrow, "`->` after the pattern")?;
            let body = self.parse_arm_body()?;
            arms.push(CaseArm {
                pattern,
                bind,
                guard,
                body,
            });
        }
        self.bump(); // Dedent
        Ok(arms)
    }

    /// `run` body, optionally followed by a subject-less `case` at the same
    /// indentation — the block's result is that case's subject.
    fn parse_run(&mut self) -> Result<Expr, ParseError> {
        self.expect(&T::Run, "`run`")?;
        self.expect(&T::Newline, "a newline after `run`")?;
        self.expect(&T::Indent, "an indented run body")?;
        let body = self.parse_block_until_dedent()?;
        let mut arms = Vec::new();
        if self.check(&T::Case) {
            self.bump();
            self.expect(
                &T::Newline,
                "a newline after `case` — a `case` closing a `run` takes no \
                 subject, since the run's result is the subject",
            )?;
            arms = self.parse_case_arms()?;
        }
        self.just_closed_block = true;
        Ok(Expr::Run { body, arms })
    }

    fn parse_cond(&mut self) -> Result<Expr, ParseError> {
        self.expect(&T::Cond, "`cond`")?;
        self.expect(&T::Newline, "a newline after `cond`")?;
        self.expect(&T::Indent, "at least one indented cond arm")?;
        let mut arms = Vec::new();
        while !self.check(&T::Dedent) {
            let condition = self.parse_or()?;
            self.expect(&T::Arrow, "`->` after the condition")?;
            let body = self.parse_arm_body()?;
            arms.push(CondArm { condition, body });
        }
        self.bump(); // Dedent
        self.just_closed_block = true;
        Ok(Expr::Cond { arms })
    }

    /// `if cond` + indented block, optionally followed by `else` and its own
    /// indented block.
    ///
    /// `if` is strictly two-branch: there is no `else if`, and no inline `->`
    /// form. More than two branches is what `cond` is for.
    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        self.expect(&T::If, "`if`")?;
        let condition = self.parse_expr()?;
        self.expect(&T::Newline, "a newline after the `if` condition")?;
        self.expect(&T::Indent, "an indented `if` body")?;
        let then_body = self.parse_block_until_dedent()?;
        let else_body = self.parse_else_body()?;
        self.just_closed_block = true;
        Ok(Expr::If {
            condition: Box::new(condition),
            then_body,
            else_body,
        })
    }

    /// The `else` half, if there is one.
    fn parse_else_body(&mut self) -> Result<Option<Block>, ParseError> {
        if !self.check(&T::Else) {
            return Ok(None);
        }
        self.bump(); // else
        if self.check(&T::If) {
            return Err(self.err_here(
                "`else if` is not valid: `if` has exactly two branches — use `cond` \
                 for a chain of conditions",
            ));
        }
        self.expect(&T::Newline, "a newline after `else`")?;
        self.expect(&T::Indent, "an indented `else` body")?;
        Ok(Some(self.parse_block_until_dedent()?))
    }

    /// Arm body: either an inline expression ending the line, or `->` at end
    /// of line followed by an indented block.
    fn parse_arm_body(&mut self) -> Result<Block, ParseError> {
        if self.eat(&T::Newline) {
            self.expect(&T::Indent, "an indented arm body")?;
            let block = self.parse_block_until_dedent()?;
            Ok(block)
        } else {
            let expr = self.parse_expr()?;
            self.stmt_end()?;
            Ok(vec![Stmt::Expr(expr)])
        }
    }

    // ── patterns ─────────────────────────────────────────────────────────

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        match self.peek().clone() {
            T::Underscore => {
                self.bump();
                Ok(Pattern::Wildcard)
            }
            T::Ident(name) => {
                self.bump();
                Ok(Pattern::Binding(name))
            }
            T::Int(n) => {
                self.bump();
                Ok(Pattern::Int(n))
            }
            T::Float(x) => {
                self.bump();
                Ok(Pattern::Float(x))
            }
            T::True => {
                self.bump();
                Ok(Pattern::Bool(true))
            }
            T::False => {
                self.bump();
                Ok(Pattern::Bool(false))
            }
            T::Nil => {
                self.bump();
                Ok(Pattern::Nil)
            }
            T::Symbol(s) => {
                self.bump();
                Ok(Pattern::Symbol(s))
            }
            T::Minus => {
                self.bump();
                match self.peek().clone() {
                    T::Int(n) => {
                        self.bump();
                        Ok(Pattern::Int(-n))
                    }
                    T::Float(x) => {
                        self.bump();
                        Ok(Pattern::Float(-x))
                    }
                    _ => Err(self.err_here("expected a number after `-` in a pattern")),
                }
            }
            T::Str(parts) => {
                let at = self.span();
                self.bump();
                match parts.as_slice() {
                    [StrPart::Lit(s)] => Ok(Pattern::Str(s.clone())),
                    _ => Err(self.err_at_ex(
                        at,
                        "string patterns cannot contain interpolation",
                        Example {
                            wrong: "case greeting\n  \"hi #{name}\" -> :ok",
                            correct: "case greeting\n  \"hi\" -> :ok",
                        },
                    )),
                }
            }
            T::LParen => {
                self.bump();
                let mut elements = vec![self.parse_pattern()?];
                while self.eat(&T::Comma) {
                    elements.push(self.parse_pattern()?);
                }
                self.expect(&T::RParen, "`)` to close the tuple pattern")?;
                if elements.len() < 2 {
                    return Err(self.err_here("tuple patterns need at least two elements"));
                }
                Ok(Pattern::Tuple(elements))
            }
            T::LBracket => {
                self.bump();
                if self.eat(&T::RBracket) {
                    return Ok(Pattern::List {
                        elements: Vec::new(),
                        rest: None,
                    });
                }
                let mut elements = Vec::new();
                let mut rest = None;
                loop {
                    elements.push(self.parse_pattern()?);
                    if self.eat(&T::Comma) {
                        continue;
                    }
                    if self.eat(&T::Pipe) {
                        rest = Some(Box::new(self.parse_pattern()?));
                    }
                    break;
                }
                self.expect(&T::RBracket, "`]` to close the list pattern")?;
                Ok(Pattern::List { elements, rest })
            }
            T::LBrace => {
                let entries = self.parse_map_entry_patterns()?;
                Ok(Pattern::Map(entries))
            }
            T::UpperIdent(_) => {
                let path = self.parse_module_path()?;
                let fields = if self.check(&T::LBrace) {
                    self.parse_field_patterns()?
                } else {
                    Vec::new() // bare `Name` matches any instance of the struct
                };
                Ok(Pattern::Struct { path, fields })
            }
            _ => Err(self.err_here("expected a pattern")),
        }
    }

    /// `{key: pattern, ...}` — a map pattern, keyed like a map literal.
    fn parse_map_entry_patterns(&mut self) -> Result<Vec<(MapKey, Pattern)>, ParseError> {
        self.expect(&T::LBrace, "`{`")?;
        let mut entries = Vec::new();
        if !self.check(&T::RBrace) {
            loop {
                let key = self.parse_map_key()?;
                self.expect(&T::Colon, "`:` after the key")?;
                entries.push((key, self.parse_pattern()?));
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        self.expect(&T::RBrace, "`}`")?;
        Ok(entries)
    }

    fn parse_field_patterns(&mut self) -> Result<Vec<(String, Pattern)>, ParseError> {
        self.expect(&T::LBrace, "`{`")?;
        let mut fields = Vec::new();
        if !self.check(&T::RBrace) {
            loop {
                let key = self.expect_ident("a key name")?;
                self.expect(&T::Colon, "`:` after the key")?;
                fields.push((key, self.parse_pattern()?));
                if !self.eat(&T::Comma) {
                    break;
                }
            }
        }
        self.expect(&T::RBrace, "`}`")?;
        Ok(fields)
    }

    /// Convert an already-parsed expression into an assignment pattern.
    fn expr_to_pattern(&self, expr: Expr, at: Span) -> Result<Pattern, ParseError> {
        expr_to_pattern(expr, at)
    }
}

/// The three `Kernel` functions that hand a message to an actor's thread.
fn is_actor_message_fn(name: &str) -> bool {
    matches!(name, "start_actor" | "call_actor" | "cast_actor")
}

/// Whether `e` is a `do` lambda, or a list/tuple/map literal that carries one
/// as a value. Stops at a nested call: whatever that call returns is a new
/// value, unrelated to any lambda that went into building it.
fn expr_embeds_lambda(e: &Expr) -> bool {
    match e {
        Expr::Lambda { .. } => true,
        Expr::List { elements, rest } => {
            elements.iter().any(expr_embeds_lambda)
                || rest.as_deref().is_some_and(expr_embeds_lambda)
        }
        Expr::Tuple(xs) => xs.iter().any(expr_embeds_lambda),
        Expr::Map(entries) => entries.iter().any(|(_, v)| expr_embeds_lambda(v)),
        _ => false,
    }
}

/// Convert an already-parsed expression into an assignment pattern.
/// Reject a body that defines the same function name twice.
///
/// Ramos does not overload on arity: a name resolves to exactly one function, so
/// a second definition is unreachable rather than an alternative. Silently
/// keeping the first is the kind of quiet surprise the rest of the language
/// refuses, so the collision is named — including between `function` and `helper`, which
/// share one namespace.
fn check_unique_fn_names(owner: &ModulePath, functions: &[FnDef]) -> Result<(), ParseError> {
    let mut seen: Vec<&str> = Vec::new();
    for f in functions {
        if seen.contains(&f.name.as_str()) {
            return Err(ParseError {
                message: format!(
                    "`{owner}` defines `{}` more than once — a name resolves to one \
                     function, and Ramos does not overload on arity",
                    f.name
                ),
                span: f.span,
                example: Some(Example {
                    wrong: "function twice(x)\n  x + x\n\nfunction twice(x, y)\n  x + y",
                    correct: "function twice(x, y)\n  x + y",
                }),
            });
        }
        seen.push(&f.name);
    }
    Ok(())
}

/// Reject a `helper`'s body calling one of the module's own `function`s — by
/// bare name or `self.name()` — directly.
///
/// A helper exists to be called *by* a function, breaking its body into
/// smaller pieces without widening what the module exposes. Letting a helper
/// call back into the public surface it is meant to serve blurs that
/// direction, so the boundary is named the same way a duplicate name is: at
/// parse time, once every function in the body is known.
fn check_helpers_dont_call_functions(
    owner: &ModulePath,
    functions: &[FnDef],
) -> Result<(), ParseError> {
    let public: Vec<&str> = functions
        .iter()
        .filter(|f| !f.private)
        .map(|f| f.name.as_str())
        .collect();
    for f in functions.iter().filter(|f| f.private) {
        let mut bound: Vec<String> = f.params.clone();
        if let Some(called) = block_calls_public(&f.body, &mut bound, &public) {
            return Err(ParseError {
                message: format!(
                    "`{owner}.{}` is a `helper` and calls `{owner}.{called}`, a `function` \
                     in the same module — a helper may call other helpers, but not call \
                     back into the module's public functions",
                    f.name
                ),
                span: f.span,
                example: Some(Example {
                    wrong: "module Payments\n  function charge(amount)\n    1\n\n  helper log(amount)\n    charge(amount)",
                    correct: "module Payments\n  function charge(amount)\n    log(amount)\n\n  helper log(amount)\n    amount",
                }),
            });
        }
    }
    Ok(())
}

/// The first name among `public` that `block` calls directly (bare, or
/// `self.name()`) without it being shadowed by a parameter or local binding.
/// Mirrors [`crate::interp::freevars::free_names`]'s bound-name tracking, but
/// looks only at call targets, not every free reference.
fn block_calls_public(block: &Block, bound: &mut Vec<String>, public: &[&str]) -> Option<String> {
    let base = bound.len();
    let mut found = None;
    for stmt in block {
        found = match stmt {
            Stmt::Expr(e) => expr_calls_public(e, bound, public),
            Stmt::Assign { pattern, value } => {
                let hit = expr_calls_public(value, bound, public);
                collect_pattern_names(pattern, bound);
                hit
            }
            Stmt::Alias { .. } => None,
        };
        if found.is_some() {
            break;
        }
    }
    bound.truncate(base);
    found
}

fn expr_calls_public(e: &Expr, bound: &mut Vec<String>, public: &[&str]) -> Option<String> {
    match e {
        Expr::Call { callee, args, .. } => {
            let hit = match callee {
                Callee::Local(name) => (!bound.iter().any(|b| b == name)
                    && public.contains(&name.as_str()))
                .then(|| name.clone()),
                Callee::Method { target, name } => {
                    let self_call = matches!(target.as_ref(), Expr::SelfRef);
                    if self_call && public.contains(&name.as_str()) {
                        Some(name.clone())
                    } else {
                        expr_calls_public(target, bound, public)
                    }
                }
            };
            hit.or_else(|| {
                args.iter()
                    .find_map(|a| expr_calls_public(a, bound, public))
            })
        }
        Expr::Access { target, .. } => expr_calls_public(target, bound, public),
        Expr::Unary { operand, .. } => expr_calls_public(operand, bound, public),
        Expr::Binary { left, right, .. } => expr_calls_public(left, bound, public)
            .or_else(|| expr_calls_public(right, bound, public)),
        Expr::List { elements, rest } => elements
            .iter()
            .find_map(|x| expr_calls_public(x, bound, public))
            .or_else(|| {
                rest.as_deref()
                    .and_then(|r| expr_calls_public(r, bound, public))
            }),
        Expr::Tuple(xs) => xs.iter().find_map(|x| expr_calls_public(x, bound, public)),
        Expr::Map(entries) => entries
            .iter()
            .find_map(|(_, v)| expr_calls_public(v, bound, public)),
        Expr::StructLit { fields, .. } => fields
            .iter()
            .find_map(|(_, v)| expr_calls_public(v, bound, public)),
        Expr::Str(pieces) => pieces.iter().find_map(|p| match p {
            StrPiece::Interp(e) => expr_calls_public(e, bound, public),
            StrPiece::Lit(_) => None,
        }),
        Expr::Lambda { params, body } => {
            let base = bound.len();
            bound.extend(params.iter().cloned());
            let hit = block_calls_public(body, bound, public);
            bound.truncate(base);
            hit
        }
        Expr::Case { subject, arms } => expr_calls_public(subject, bound, public).or_else(|| {
            arms.iter()
                .find_map(|arm| case_arm_calls_public(arm, bound, public))
        }),
        Expr::Cond { arms } => arms.iter().find_map(|arm| {
            expr_calls_public(&arm.condition, bound, public)
                .or_else(|| block_calls_public(&arm.body, bound, public))
        }),
        Expr::If {
            condition,
            then_body,
            else_body,
        } => expr_calls_public(condition, bound, public)
            .or_else(|| block_calls_public(then_body, bound, public))
            .or_else(|| {
                else_body
                    .as_ref()
                    .and_then(|b| block_calls_public(b, bound, public))
            }),
        Expr::Run { body, arms } => block_calls_public(body, bound, public).or_else(|| {
            arms.iter()
                .find_map(|arm| case_arm_calls_public(arm, bound, public))
        }),
        _ => None,
    }
}

fn case_arm_calls_public(
    arm: &CaseArm,
    bound: &mut Vec<String>,
    public: &[&str],
) -> Option<String> {
    let base = bound.len();
    collect_pattern_names(&arm.pattern, bound);
    // `pattern = name` shadows `name` locally too, same as any name the
    // pattern itself binds.
    if let Some(name) = &arm.bind {
        bound.push(name.clone());
    }
    let hit = arm
        .guard
        .as_ref()
        .and_then(|g| expr_calls_public(g, bound, public))
        .or_else(|| block_calls_public(&arm.body, bound, public));
    bound.truncate(base);
    hit
}

fn collect_pattern_names(pattern: &Pattern, bound: &mut Vec<String>) {
    match pattern {
        Pattern::Binding(n) => bound.push(n.clone()),
        Pattern::Tuple(ps) => ps.iter().for_each(|p| collect_pattern_names(p, bound)),
        Pattern::List { elements, rest } => {
            elements
                .iter()
                .for_each(|p| collect_pattern_names(p, bound));
            if let Some(r) = rest {
                collect_pattern_names(r, bound);
            }
        }
        Pattern::Map(entries) => entries
            .iter()
            .for_each(|(_, p)| collect_pattern_names(p, bound)),
        Pattern::Struct { fields, .. } => {
            fields
                .iter()
                .for_each(|(_, p)| collect_pattern_names(p, bound));
        }
        _ => {}
    }
}

/// Reject a pattern that binds the same name twice — including, for a `case`
/// arm, its own `= name` whole-value binding colliding with a name the
/// pattern itself binds.
///
/// `(p, p)` reads as "these two must be equal", but Ramos has no way to ask for
/// that comparison — there is no pin operator — so the second `p` would simply
/// rebind and the pattern could never fail. Rather than silently keeping the
/// last value, the collision is named. `_` may repeat: it binds nothing.
fn check_bindings_are_unique(
    pattern: &Pattern,
    bind: Option<&str>,
    at: Span,
) -> Result<(), ParseError> {
    let mut seen: Vec<&str> = Vec::new();
    if let Some(name) = first_repeated_binding(pattern, &mut seen) {
        return Err(ParseError {
            message: format!(
                "a pattern cannot bind `{name}` twice — each name in a pattern binds one \
                 value, and there is no way to ask that the two be equal"
            ),
            span: at,
            example: Some(Example {
                wrong: "(p, p) = (1, 2)",
                correct: "(p, q) = (1, 2)",
            }),
        });
    }
    if let Some(name) = bind {
        if seen.contains(&name) {
            return Err(ParseError {
                message: format!(
                    "a pattern cannot bind `{name}` twice — `= {name}` already names the \
                     whole value, so the pattern binding the same name to part of it can \
                     never make sense"
                ),
                span: at,
                example: Some(Example {
                    wrong: "Person{name: n} = n -> n",
                    correct: "Person{name: n} = p -> n",
                }),
            });
        }
    }
    Ok(())
}

/// The first binding name that appears twice anywhere in `pattern`.
fn first_repeated_binding<'p>(pattern: &'p Pattern, seen: &mut Vec<&'p str>) -> Option<&'p str> {
    match pattern {
        Pattern::Binding(name) => {
            if seen.contains(&name.as_str()) {
                return Some(name);
            }
            seen.push(name);
            None
        }
        Pattern::Tuple(pats) => pats.iter().find_map(|p| first_repeated_binding(p, seen)),
        Pattern::List { elements, rest } => elements
            .iter()
            .find_map(|p| first_repeated_binding(p, seen))
            .or_else(|| {
                rest.as_deref()
                    .and_then(|r| first_repeated_binding(r, seen))
            }),
        Pattern::Map(entries) => entries
            .iter()
            .find_map(|(_, p)| first_repeated_binding(p, seen)),
        Pattern::Struct { fields, .. } => fields
            .iter()
            .find_map(|(_, p)| first_repeated_binding(p, seen)),
        // Wildcards and literals bind nothing.
        _ => None,
    }
}

fn expr_to_pattern(expr: Expr, at: Span) -> Result<Pattern, ParseError> {
    let invalid = |what: &str, example: Option<Example>| ParseError {
        message: format!("{what} cannot appear on the left side of `=`"),
        span: at,
        example,
    };
    Ok(match expr {
        Expr::Var(name) => Pattern::Binding(name),
        Expr::Wildcard => Pattern::Wildcard,
        Expr::Int(n) => Pattern::Int(n),
        Expr::Float(x) => Pattern::Float(x),
        Expr::Bool(b) => Pattern::Bool(b),
        Expr::Nil => Pattern::Nil,
        Expr::Symbol(s) => Pattern::Symbol(s),
        Expr::Str(pieces) => match pieces.as_slice() {
            [StrPiece::Lit(s)] => Pattern::Str(s.clone()),
            _ => {
                return Err(invalid(
                    "a string with interpolation",
                    Some(Example {
                        wrong: "\"total: #{n}\" = line",
                        correct: "\"total: \" = line",
                    }),
                ))
            }
        },
        Expr::Tuple(elements) => Pattern::Tuple(
            elements
                .into_iter()
                .map(|e| expr_to_pattern(e, at))
                .collect::<Result<_, _>>()?,
        ),
        Expr::List { elements, rest } => Pattern::List {
            elements: elements
                .into_iter()
                .map(|e| expr_to_pattern(e, at))
                .collect::<Result<_, _>>()?,
            rest: match rest {
                Some(r) => Some(Box::new(expr_to_pattern(*r, at)?)),
                None => None,
            },
        },
        Expr::Map(fields) => Pattern::Map(
            fields
                .into_iter()
                .map(|(k, v)| Ok((k, expr_to_pattern(v, at)?)))
                .collect::<Result<_, ParseError>>()?,
        ),
        Expr::StructLit { path, fields } => Pattern::Struct {
            path,
            fields: fields
                .into_iter()
                .map(|(k, v)| Ok((k, expr_to_pattern(v, at)?)))
                .collect::<Result<_, ParseError>>()?,
        },
        Expr::Unary {
            op: UnOp::Neg,
            operand,
        } => match *operand {
            Expr::Int(n) => Pattern::Int(-n),
            Expr::Float(x) => Pattern::Float(-x),
            _ => {
                return Err(invalid(
                    "this expression",
                    Some(Example {
                        wrong: "-x = 5",
                        correct: "x = -5",
                    }),
                ))
            }
        },
        _ => {
            return Err(invalid(
                "this expression",
                Some(Example {
                    wrong: "f(x) = 1",
                    correct: "x = f(1)",
                }),
            ))
        }
    })
}

impl Parser {
    // ── string interpolation ─────────────────────────────────────────────

    fn convert_str_parts(
        &self,
        parts: Vec<StrPart>,
        at: Span,
    ) -> Result<Vec<StrPiece>, ParseError> {
        parts
            .into_iter()
            .map(|p| match p {
                StrPart::Lit(s) => Ok(StrPiece::Lit(s)),
                StrPart::Interp(mut tokens) => {
                    let end = tokens.last().map(|t| t.span).unwrap_or(at);
                    let end_line = tokens.last().map(|t| t.line).unwrap_or(1);
                    tokens.push(Token {
                        kind: T::Eof,
                        span: end,
                        line: end_line,
                    });
                    let mut sub = Parser::new(tokens);
                    let expr = sub.parse_expr()?;
                    if !sub.check(&T::Eof) {
                        return Err(sub.err_here("unexpected token in interpolation"));
                    }
                    Ok(StrPiece::Interp(expr))
                }
            })
            .collect()
    }

    // ── token helpers ────────────────────────────────────────────────────

    fn parse_module_path(&mut self) -> Result<ModulePath, ParseError> {
        let mut segments = vec![self.expect_upper("a module name")?];
        while self.check(&T::Dot) && matches!(self.nth(1), T::UpperIdent(_)) {
            self.bump();
            segments.push(self.expect_upper("a module name segment")?);
        }
        Ok(ModulePath(segments))
    }

    fn peek(&self) -> &T {
        &self.tokens[self.pos].kind
    }

    fn nth(&self, n: usize) -> &T {
        let i = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[i].kind
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    /// The 1-based source line the current token starts on — what a
    /// stacktrace frame records for a call site.
    fn line(&self) -> usize {
        self.tokens[self.pos].line
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        self.just_closed_block = false;
        tok
    }

    fn check(&self, kind: &T) -> bool {
        self.peek() == kind
    }

    fn eat(&mut self, kind: &T) -> bool {
        if self.check(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &T, what: &str) -> Result<Token, ParseError> {
        if self.check(kind) {
            Ok(self.bump())
        } else {
            Err(self.err_here(&format!("expected {what}, found {:?}", self.peek())))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            T::Ident(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err_here(&format!("expected {what}, found {other:?}"))),
        }
    }

    /// Function/lambda parameters: identifiers, plus `self`.
    fn expect_param(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            T::Ident(name) => {
                self.bump();
                Ok(name)
            }
            T::SelfKw => {
                self.bump();
                Ok("self".to_string())
            }
            other => Err(self.err_here(&format!("expected a parameter name, found {other:?}"))),
        }
    }

    fn expect_upper(&mut self, what: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            T::UpperIdent(name) => {
                self.bump();
                Ok(name)
            }
            other => Err(self.err_here(&format!("expected {what}, found {other:?}"))),
        }
    }

    fn err_here(&self, message: &str) -> ParseError {
        ParseError {
            message: message.to_string(),
            span: self.span(),
            example: None,
        }
    }

    /// Like [`Self::err_here`], plus the wrong/correct snippet pair shown
    /// under the diagnostic for a named strict rule.
    fn err_here_ex(&self, message: &str, example: Example) -> ParseError {
        ParseError {
            message: message.to_string(),
            span: self.span(),
            example: Some(example),
        }
    }

    /// Like [`Self::err_at`], plus the wrong/correct snippet pair shown under
    /// the diagnostic for a named strict rule.
    fn err_at_ex(&self, span: Span, message: &str, example: Example) -> ParseError {
        ParseError {
            message: message.to_string(),
            span,
            example: Some(example),
        }
    }
}

fn bin(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// `N"..."` → `NaiveDateTime.parse("...")`. The lexer already checked
/// `letter` against this same set, so every other value is unreachable.
fn sigil_call(letter: char, text: String, line: usize) -> Expr {
    let module = match letter {
        'D' => "Date",
        'T' => "Time",
        'N' => "NaiveDateTime",
        'U' => "DateTime",
        _ => unreachable!("the lexer only emits sigils for D, T, N, U"),
    };
    Expr::Call {
        callee: Callee::Method {
            target: Box::new(Expr::ModuleRef(ModulePath(vec![module.to_string()]))),
            name: "parse".to_string(),
        },
        args: vec![Expr::Str(vec![StrPiece::Lit(text)])],
        line,
    }
}
