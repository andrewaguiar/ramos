//! Ramos AST, produced by the parser (PLAN phase 2).
//!
//! Design notes:
//! - `|` pipes are desugared at parse time (`a | M.f(b)` becomes `M.f(a, b)`),
//!   so no pipe survives into the tree.
//! - Spans live on definitions and errors, not on every expression; the
//!   evaluator's stacktraces (phase 7) will add call-site spans where needed.

use crate::color::{Color, Style};
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModulePath(pub Vec<String>);

impl std::fmt::Display for ModulePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Module(ModuleDef),
    Struct(StructDef),
    Trait(TraitDef),
    /// Top-level function (README's error-handling examples use these).
    Function(FnDef),
    Statement(Stmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
    pub name: ModulePath,
    /// Traits this module implements. A module has no instance, so it satisfies
    /// a trait with plain module functions — which is what makes `Actor` work.
    pub implements: Vec<ModulePath>,
    /// `alias Some.Module as Name` lines at the top of the body, as
    /// (local name, full path) — the module's own short names for others.
    pub aliases: Vec<(String, ModulePath)>,
    pub functions: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: ModulePath,
    pub implements: Vec<ModulePath>,
    pub attributes: Vec<(String, Expr)>,
    pub functions: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef {
    pub name: ModulePath,
    /// A function with an empty body is a *required* function.
    pub functions: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<String>,
    /// Empty for declarations (trait requirements, native seams like
    /// `Kernel.native`).
    pub body: Block,
    pub private: bool, // fnp
    pub span: Span,
}

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `pattern = expr` — plain `x = 1` binds, tuple/list/map/struct shapes
    /// destructure.
    Assign {
        pattern: Pattern,
        value: Expr,
    },
    Expr(Expr),
    Alias {
        module: ModulePath,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Nil,
    Symbol(String),
    /// Literal/interpolation pieces, in order.
    Str(Vec<StrPiece>),
    /// `rest` is only produced on the left of `=` (cons destructuring);
    /// a plain literal has `rest: None`.
    List {
        elements: Vec<Expr>,
        rest: Option<Box<Expr>>,
    },
    Tuple(Vec<Expr>),
    Map(Vec<(MapKey, Expr)>),
    StructLit {
        path: ModulePath,
        fields: Vec<(String, Expr)>,
    },
    Var(String),
    /// `_` — valid only where an expression is converted to a pattern.
    Wildcard,
    SelfRef,
    ModuleRef(ModulePath),
    /// Dot access without parens: a struct field.
    Access {
        target: Box<Expr>,
        name: String,
    },
    Call {
        callee: Callee,
        args: Vec<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Lambda {
        params: Vec<String>,
        body: Block,
    },
    Case {
        subject: Box<Expr>,
        arms: Vec<CaseArm>,
    },
    Cond {
        arms: Vec<CondArm>,
    },
    /// `if cond` / `else` — exactly two branches. `else_body` is `None` when
    /// there is no `else`, and the expression is then `nil` on the branch not
    /// taken. There is no `else if`; a chain of conditions is `cond`.
    If {
        condition: Box<Expr>,
        then_body: Block,
        else_body: Option<Block>,
    },
    /// `run` — a block that halts on the first failed match, yielding the
    /// value that failed instead of raising. `arms` holds the optional
    /// trailing `case` (written without a subject: the block's result is the
    /// subject); it is empty when there is none.
    Run {
        body: Block,
        arms: Vec<CaseArm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPiece {
    Lit(String),
    Interp(Expr),
}

/// A key written in a map literal or map pattern. Keys are literals — they are
/// matched and looked up by value, so an expression would have nothing to do
/// but be evaluated first, and a bare name reads as a symbol shorthand.
///
/// A symbol key is written `{name: 1}` — the trailing `:` is the only one, and
/// `{:name: 1}` is rejected; `{"host": 1}` and `{42: 1}` key by string and
/// integer. A map built at runtime may still hold
/// keys of any type (`List.group_by` produces boolean keys) — those simply
/// have no literal form.
#[derive(Debug, Clone, PartialEq)]
pub enum MapKey {
    Symbol(String),
    Str(String),
    Int(i64),
}

impl std::fmt::Display for MapKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapKey::Symbol(name) => write!(f, "{name}"),
            MapKey::Str(s) => write!(f, "{s:?}"),
            MapKey::Int(n) => write!(f, "{n}"),
        }
    }
}

/// A bare name is the symbol shorthand, so `"a".into()` is `:a` — which is what
/// makes `{a: 1}` and `MapKey::from("a")` agree.
impl From<&str> for MapKey {
    fn from(name: &str) -> MapKey {
        MapKey::Symbol(name.to_string())
    }
}

impl From<i64> for MapKey {
    fn from(n: i64) -> MapKey {
        MapKey::Int(n)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// `foo(...)` — a local function, falling back to `Kernel`.
    Local(String),
    /// `target.f(...)` — module function or struct method.
    Method { target: Box<Expr>, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg, // -x
    Not, // not x
}

impl UnOp {
    pub fn symbol(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "not",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat, // <>  (strings)
    Append, // ++  (list concat / map merge)
    Eq,
    NotEq,
    Lt,
    Gt,
    Le,
    Ge,
    And, // short-circuits
    Or,  // short-circuits
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Concat => "<>",
            BinOp::Append => "++",
            BinOp::Eq => "==",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaseArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CondArm {
    pub condition: Expr,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Binding(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Nil,
    Symbol(String),
    /// Plain strings only — no interpolation in patterns.
    Str(String),
    Tuple(Vec<Pattern>),
    List {
        elements: Vec<Pattern>,
        rest: Option<Box<Pattern>>,
    },
    Map(Vec<(MapKey, Pattern)>),
    /// `Name{...}`; a bare `Name` in rescue position is `fields: []`.
    Struct {
        path: ModulePath,
        fields: Vec<(String, Pattern)>,
    },
}

// ── debug dump ───────────────────────────────────────────────────────────

/// Debug rendering of a parse result: a fenced "Raw Code:" block with the
/// source, then a fenced "AST" block holding an indented tree. Shaped like
/// `lexer::dump` so `ramos lexer --dump` and `ramos ast --dump` read alike.
///
/// This is a debug view, not a pretty-printer: it shows the tree the parser
/// built (pipes already desugared), not source you could feed back to `ramos`.
pub fn dump(source: &str, program: &Program, color: Color) -> String {
    let mut out = color.paint(Style::Heading, "Raw Code:");
    out.push_str("\n```\n");
    // The source parsed, so it lexes; the fallback is only for a caller that
    // hands us source the program didn't come from.
    match crate::lexer::lex(source) {
        Ok(tokens) => out.push_str(&crate::lexer::highlight(source, &tokens, color)),
        Err(_) => out.push_str(source),
    }
    if !source.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n\n");
    out.push_str(&color.paint(Style::Heading, "AST"));
    out.push_str("\n```\n");
    out.push_str(&render(program, color));
    out.push_str("```\n");
    out
}

/// The indented tree on its own, without the dump's surrounding blocks.
pub fn render(program: &Program, color: Color) -> String {
    let mut printer = Printer {
        out: String::new(),
        depth: 0,
        color,
    };
    printer.program(program);
    printer.out
}

struct Printer {
    out: String,
    depth: usize,
    color: Color,
}

impl Printer {
    fn line(&mut self, style: Style, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(&self.color.paint(style, text));
        self.out.push('\n');
    }

    /// A header line followed by its children, one level deeper.
    fn node(&mut self, style: Style, header: &str, children: impl FnOnce(&mut Self)) {
        self.line(style, header);
        self.depth += 1;
        children(self);
        self.depth -= 1;
    }

    /// A structural label — `pattern`, `args`, `body` — naming the children
    /// under it rather than a node of its own.
    fn label(&mut self, name: &str, children: impl FnOnce(&mut Self)) {
        self.node(Style::Dim, name, children);
    }

    fn program(&mut self, program: &Program) {
        self.node(Style::Heading, "Program", |p| {
            for item in &program.items {
                p.item(item);
            }
        });
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Module(m) => self.node(Style::Keyword, &format!("module {}", m.name), |p| {
                for name in &m.implements {
                    p.line(Style::Keyword, &format!("implements {name}"));
                }
                for (name, path) in &m.aliases {
                    p.line(Style::Keyword, &format!("alias {path} as {name}"));
                }
                for f in &m.functions {
                    p.fn_def(f);
                }
            }),
            Item::Struct(s) => self.node(Style::Keyword, &format!("struct {}", s.name), |p| {
                for name in &s.implements {
                    p.line(Style::Keyword, &format!("implements {name}"));
                }
                if !s.attributes.is_empty() {
                    p.node(Style::Keyword, "attributes", |p| p.fields(&s.attributes));
                }
                for f in &s.functions {
                    p.fn_def(f);
                }
            }),
            Item::Trait(t) => self.node(Style::Keyword, &format!("trait {}", t.name), |p| {
                for f in &t.functions {
                    p.fn_def(f);
                }
            }),
            Item::Function(f) => self.fn_def(f),
            Item::Statement(s) => self.stmt(s),
        }
    }

    fn fn_def(&mut self, f: &FnDef) {
        let keyword = if f.private { "fnp" } else { "fn" };
        let head = format!("{keyword} {}({})", f.name, f.params.join(", "));
        if f.body.is_empty() {
            self.line(Style::Keyword, &format!("{head} [declaration]"));
        } else {
            self.node(Style::Keyword, &head, |p| p.block(&f.body));
        }
    }

    fn block(&mut self, block: &Block) {
        for stmt in block {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { pattern, value } => self.node(Style::Plain, "Assign", |p| {
                p.label("pattern", |p| p.pattern(pattern));
                p.label("value", |p| p.expr(value));
            }),
            Stmt::Expr(e) => self.expr(e),
            Stmt::Alias { module, name } => {
                self.line(Style::Keyword, &format!("Alias {module} as {name}"))
            }
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int(n) => self.line(Style::Literal, &format!("Int {n}")),
            Expr::Float(x) => self.line(Style::Literal, &format!("Float {x:?}")),
            Expr::Bool(b) => self.line(Style::Literal, &format!("Bool {b}")),
            Expr::Nil => self.line(Style::Literal, "Nil"),
            Expr::Symbol(s) => self.line(Style::Literal, &format!("Symbol :{s}")),
            Expr::Str(pieces) => self.node(Style::Str, "Str", |p| {
                for piece in pieces {
                    match piece {
                        StrPiece::Lit(text) => p.line(Style::Str, &format!("Lit {text:?}")),
                        StrPiece::Interp(e) => p.node(Style::Dim, "Interp", |p| p.expr(e)),
                    }
                }
            }),
            Expr::List { elements, rest } => self.node(Style::Plain, "List", |p| {
                for e in elements {
                    p.expr(e);
                }
                if let Some(rest) = rest {
                    p.label("rest", |p| p.expr(rest));
                }
            }),
            Expr::Tuple(elements) => self.node(Style::Plain, "Tuple", |p| {
                for e in elements {
                    p.expr(e);
                }
            }),
            Expr::Map(fields) => self.node(Style::Plain, "Map", |p| p.map_entries(fields)),
            Expr::StructLit { path, fields } => {
                self.node(Style::Type, &format!("StructLit {path}"), |p| {
                    p.fields(fields)
                })
            }
            Expr::Var(name) => self.line(Style::Plain, &format!("Var {name}")),
            Expr::Wildcard => self.line(Style::Plain, "Wildcard"),
            Expr::SelfRef => self.line(Style::Keyword, "Self"),
            Expr::ModuleRef(path) => self.line(Style::Type, &format!("ModuleRef {path}")),
            Expr::Access { target, name } => {
                self.node(Style::Plain, &format!("Access .{name}"), |p| p.expr(target))
            }
            Expr::Call { callee, args } => match callee {
                Callee::Local(name) => {
                    self.node(Style::Plain, &format!("Call {name}()"), |p| p.args(args))
                }
                Callee::Method { target, name } => {
                    self.node(Style::Plain, &format!("Call .{name}()"), |p| {
                        p.label("target", |p| p.expr(target));
                        p.args(args);
                    })
                }
            },
            Expr::Unary { op, operand } => {
                self.node(Style::Plain, &format!("Unary {}", op.symbol()), |p| {
                    p.expr(operand)
                })
            }
            Expr::Binary { op, left, right } => {
                self.node(Style::Plain, &format!("Binary {}", op.symbol()), |p| {
                    p.expr(left);
                    p.expr(right);
                })
            }
            Expr::Lambda { params, body } => self.node(
                Style::Keyword,
                &format!("Lambda({})", params.join(", ")),
                |p| p.block(body),
            ),
            Expr::Case { subject, arms } => self.node(Style::Keyword, "Case", |p| {
                p.label("subject", |p| p.expr(subject));
                p.case_arms(arms);
            }),
            Expr::Run { body, arms } => self.node(Style::Keyword, "Run", |p| {
                p.label("body", |p| p.block(body));
                if !arms.is_empty() {
                    p.node(Style::Keyword, "case", |p| p.case_arms(arms));
                }
            }),
            Expr::Cond { arms } => self.node(Style::Keyword, "Cond", |p| {
                for arm in arms {
                    p.label("arm", |p| {
                        p.label("condition", |p| p.expr(&arm.condition));
                        p.label("body", |p| p.block(&arm.body));
                    });
                }
            }),
            Expr::If {
                condition,
                then_body,
                else_body,
            } => self.node(Style::Keyword, "If", |p| {
                p.label("condition", |p| p.expr(condition));
                p.label("then", |p| p.block(then_body));
                if let Some(body) = else_body {
                    p.label("else", |p| p.block(body));
                }
            }),
        }
    }

    fn pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard => self.line(Style::Plain, "Wildcard"),
            Pattern::Binding(name) => self.line(Style::Plain, &format!("Binding {name}")),
            Pattern::Int(n) => self.line(Style::Literal, &format!("Int {n}")),
            Pattern::Float(x) => self.line(Style::Literal, &format!("Float {x:?}")),
            Pattern::Bool(b) => self.line(Style::Literal, &format!("Bool {b}")),
            Pattern::Nil => self.line(Style::Literal, "Nil"),
            Pattern::Symbol(s) => self.line(Style::Literal, &format!("Symbol :{s}")),
            Pattern::Str(s) => self.line(Style::Str, &format!("Str {s:?}")),
            Pattern::Tuple(elements) => self.node(Style::Plain, "Tuple", |p| {
                for e in elements {
                    p.pattern(e);
                }
            }),
            Pattern::List { elements, rest } => self.node(Style::Plain, "List", |p| {
                for e in elements {
                    p.pattern(e);
                }
                if let Some(rest) = rest {
                    p.label("rest", |p| p.pattern(rest));
                }
            }),
            Pattern::Map(fields) => {
                self.node(Style::Plain, "Map", |p| p.map_entry_patterns(fields))
            }
            Pattern::Struct { path, fields } => {
                self.node(Style::Type, &format!("Struct {path}"), |p| {
                    p.field_patterns(fields)
                })
            }
        }
    }

    /// The arms of a `case`, shared by `Case` and the `case` a `run` may end
    /// with.
    fn case_arms(&mut self, arms: &[CaseArm]) {
        for arm in arms {
            self.label("arm", |p| {
                p.label("pattern", |p| p.pattern(&arm.pattern));
                if let Some(guard) = &arm.guard {
                    p.node(Style::Keyword, "when", |p| p.expr(guard));
                }
                p.label("body", |p| p.block(&arm.body));
            });
        }
    }

    fn args(&mut self, args: &[Expr]) {
        if args.is_empty() {
            return;
        }
        self.label("args", |p| {
            for arg in args {
                p.expr(arg);
            }
        });
    }

    fn fields(&mut self, fields: &[(String, Expr)]) {
        for (name, value) in fields {
            self.label(&format!("{name}:"), |p| p.expr(value));
        }
    }

    fn field_patterns(&mut self, fields: &[(String, Pattern)]) {
        for (name, value) in fields {
            self.label(&format!("{name}:"), |p| p.pattern(value));
        }
    }

    fn map_entries(&mut self, entries: &[(MapKey, Expr)]) {
        for (key, value) in entries {
            self.label(&format!("{key}:"), |p| p.expr(value));
        }
    }

    fn map_entry_patterns(&mut self, entries: &[(MapKey, Pattern)]) {
        for (key, value) in entries {
            self.label(&format!("{key}:"), |p| p.pattern(value));
        }
    }
}
