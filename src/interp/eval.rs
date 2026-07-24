//! The tree-walking evaluator (PLAN phases 3–4).
//!
//! Covered here: literals, operators, string interpolation, `case` / `cond`,
//! lambdas + closures, destructuring assignment, function calls, and —
//! since phase 4 — modules (`Module.f()`, `const`, `helper` visibility, `alias`,
//! `self`), `|.` dispatch on a receiver's runtime type, Kernel's implicit
//! scope, and the `native(str, args)` seam. Structs/traits and
//! `begin`/`rescue` land in later phases and are reported as "not yet" here
//! rather than silently misbehaving.

use super::env::Env;
use super::value::{
    map_key_value, values_equal, Closure, List, Map, StructValue, ThreadHandle, ThreadReply,
    ThreadState, Value,
};
use super::{freevars, natives, pattern};
use crate::ast::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Write;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// A shared, thread-safe output sink. The main interpreter and every thread
/// spawned with `Thread.start` hold a clone, so a spawned lambda's output goes
/// **live** to the same place as the caller's — serialized by the mutex (a whole
/// `print` is one locked write, so lines never split), rather than buffered
/// until the thread is awaited. `errs` is the same, for `eprint`.
pub type Sink = Arc<Mutex<dyn Write + Send>>;

/// Wrap an owned writer as a [`Sink`]. `sink(std::io::stdout())` for the CLI,
/// `sink(Vec::new())` for a throwaway, or build the `Arc<Mutex<W>>` directly
/// when the concrete `W` has to be read back (as a test capturing bytes does).
pub fn sink(writer: impl Write + Send + 'static) -> Sink {
    Arc::new(Mutex::new(writer))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    /// Set when the program called `exit(code)`. This is not a failure — it
    /// rides the error path only because that is how the interpreter unwinds.
    /// A caller must check it before reporting anything: `exit(0)` is success.
    pub exit_code: Option<i64>,
}

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        RuntimeError {
            message: message.into(),
            exit_code: None,
        }
    }

    /// The unwind `exit(code)` raises. Calling `process::exit` instead would
    /// kill a test runner and drop buffered output, so the interpreter returns
    /// to its caller and lets `main` set the status.
    pub fn exit(code: i64) -> Self {
        RuntimeError {
            message: format!("exit({code})"),
            exit_code: Some(code),
        }
    }

    /// True when this is an `exit`, not a real failure.
    pub fn is_exit(&self) -> bool {
        self.exit_code.is_some()
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::new(message))
}

/// Run a program. If it defines an entrypoint (a module exposing `function main()`),
/// that `main()` is called; otherwise the file's top-level statements run in
/// order. Either way the last value is returned. Output from `print` and
/// friends is written to `out` — a [`Sink`], since a `Thread.start`ed lambda
/// shares it and writes live.
pub fn run(program: &Program, out: Sink) -> Result<Value, RuntimeError> {
    run_with_args(program, out, &[])
}

/// Like [`run`], but with the program's command-line arguments made available
/// to `get_args` / `get_arg`. `eprint`/`eprintln` go to the process's stderr;
/// use [`run_with_streams`] to capture them instead.
pub fn run_with_args(program: &Program, out: Sink, argv: &[String]) -> Result<Value, RuntimeError> {
    run_with_streams(program, out, sink(std::io::stderr()), argv)
}

/// Like [`run_with_args`], with `eprint`/`eprintln` directed at `errs` rather
/// than the process's stderr — which is what lets a test assert on them.
pub fn run_with_streams(
    program: &Program,
    out: Sink,
    errs: Sink,
    argv: &[String],
) -> Result<Value, RuntimeError> {
    let mut interp = Interp {
        out,
        errs,
        root: Env::root(),
        funcs: HashMap::new(),
        modules: HashMap::new(),
        structs: HashMap::new(),
        traits: HashMap::new(),
        actors: HashMap::new(),
        aliases: HashMap::new(),
        current_module: None,
        current_file: program.entry_file.clone(),
        call_stack: Vec::new(),
        argv: argv.to_vec(),
    };
    interp.run_program(program)
}

/// One test function's result: where it lives, and why it failed if it did.
pub struct TestOutcome {
    pub module: String,
    pub name: String,
    /// `None` when the test passed.
    pub failure: Option<String>,
}

/// Run every test in `program`.
///
/// A test module is one that implements `Test`; a test is a public function in
/// it whose name begins with `test_`, called with no arguments. A test fails by
/// failing an `assert`, which travels the ordinary error path — so a failure
/// stops that test and the next one still runs.
pub fn run_tests(
    program: &Program,
    out: Sink,
    argv: &[String],
) -> Result<Vec<TestOutcome>, RuntimeError> {
    let mut interp = Interp {
        out,
        errs: sink(std::io::stderr()),
        root: Env::root(),
        funcs: HashMap::new(),
        modules: HashMap::new(),
        structs: HashMap::new(),
        traits: HashMap::new(),
        actors: HashMap::new(),
        aliases: HashMap::new(),
        current_module: None,
        current_file: program.entry_file.clone(),
        call_stack: Vec::new(),
        argv: argv.to_vec(),
    };
    interp.register_items(program)?;

    let mut outcomes = Vec::new();
    // Program order, so a run reads the same way twice.
    for item in &program.items {
        let Item::Module(m) = item else { continue };
        if !m.implements.iter().any(|t| t.to_string() == "Test") {
            continue;
        }
        let name = m.name.to_string();
        let module = interp.module_named(&name)?;
        let tests: Vec<Arc<FnDef>> = module
            .functions
            .iter()
            .filter(|f| !f.private && f.name.starts_with("test_"))
            .map(|f| Arc::new(f.clone()))
            .collect();
        for f in tests {
            let failure = if f.params.is_empty() {
                interp
                    .call_module_fn(module.clone(), f.clone(), Vec::new())
                    .err()
                    .map(|e| e.message)
            } else {
                Some(format!(
                    "a test takes no arguments, but `{}` declares {}",
                    f.name,
                    f.params.len()
                ))
            };
            outcomes.push(TestOutcome {
                module: name.clone(),
                name: f.name.clone(),
                failure,
            });
        }
    }
    // A test may have started actors; let them finish before the run ends.
    interp.drain_all_actors()?;
    Ok(outcomes)
}

/// A persistent interpreter session for the REPL. Unlike [`run`], which builds a
/// fresh interpreter and (if present) calls `main`, a session keeps its scope,
/// `function`s, and `module`s alive across calls so each entry can build on the last.
///
/// Definitions are *registered* (never auto-run), then the entry's top-level
/// statements execute in order; the last value is returned. Bindings live in the
/// shared root scope, so `x = 1` on one line is in scope on the next.
pub struct Session {
    root: Env,
    funcs: HashMap<String, Arc<FnDef>>,
    modules: HashMap<String, Arc<ModuleDef>>,
    structs: HashMap<String, Arc<StructDef>>,
    traits: HashMap<String, Arc<TraitDef>>,
    actors: HashMap<String, ActorHandle>,
    aliases: HashMap<String, String>,
    argv: Vec<String>,
}

impl Session {
    pub fn new() -> Self {
        Session {
            root: Env::root(),
            funcs: HashMap::new(),
            modules: HashMap::new(),
            structs: HashMap::new(),
            traits: HashMap::new(),
            actors: HashMap::new(),
            aliases: HashMap::new(),
            argv: Vec::new(),
        }
    }

    /// Evaluate one REPL entry, writing any `print` output to `out` and any
    /// `eprint` output to the process's stderr. New definitions and bindings
    /// persist into the session for later entries.
    pub fn eval(&mut self, program: &Program, out: Sink) -> Result<Value, RuntimeError> {
        let mut interp = Interp {
            out,
            errs: sink(std::io::stderr()),
            root: self.root.clone(), // shares the scope: bindings persist
            funcs: std::mem::take(&mut self.funcs),
            modules: std::mem::take(&mut self.modules),
            structs: std::mem::take(&mut self.structs),
            traits: std::mem::take(&mut self.traits),
            actors: std::mem::take(&mut self.actors),
            aliases: std::mem::take(&mut self.aliases),
            current_module: None,
            current_file: program.entry_file.clone(),
            call_stack: Vec::new(),
            argv: self.argv.clone(),
        };
        let result = interp
            .register_items(program)
            .and_then(|()| interp.run_top_level(program));
        // Carry accumulated definitions back out (bindings already live in root).
        self.funcs = std::mem::take(&mut interp.funcs);
        self.modules = std::mem::take(&mut interp.modules);
        self.structs = std::mem::take(&mut interp.structs);
        self.traits = std::mem::take(&mut interp.traits);
        self.actors = std::mem::take(&mut interp.actors);
        self.aliases = std::mem::take(&mut interp.aliases);
        result
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

struct Interp {
    out: Sink,
    /// Where `eprint`/`eprintln` go. Separate from `out` so diagnostics can be
    /// redirected (or captured) without touching the program's own output.
    errs: Sink,
    root: Env,
    funcs: HashMap<String, Arc<FnDef>>,
    /// Every known module, keyed by its full dotted name.
    modules: HashMap<String, Arc<ModuleDef>>,
    /// Every known struct, keyed by name. Each also has a `ModuleDef` view in
    /// `modules`, so `Person.hello(p)` and a struct's own bare calls resolve
    /// through the ordinary module machinery.
    structs: HashMap<String, Arc<StructDef>>,
    /// Every known trait, keyed by name. Traits are not callable themselves —
    /// they exist so an `implements` can be checked and the trait's defaults
    /// folded into the implementer's functions.
    traits: HashMap<String, Arc<TraitDef>>,
    /// Running actors, keyed by the id `start_actor` was given. Each owns a
    /// thread; this side holds only the channel into it.
    actors: HashMap<String, ActorHandle>,
    /// Top-level `alias` statements: local short name → full dotted name. A
    /// module's own `alias` lines live on its `ModuleDef` and take precedence
    /// inside it.
    aliases: HashMap<String, String>,
    /// The module whose function is currently executing, so bare calls resolve
    /// to that module's own `function`/`helper` before falling back to Kernel.
    current_module: Option<Arc<ModuleDef>>,
    /// The file whose code is currently executing — a fresh lambda captures
    /// this into its own `Closure::file`, and entering a named function sets
    /// it to that function's `FnDef::file`, restored on return.
    current_file: Arc<str>,
    /// The call sites active right now, oldest first. Pushed and popped around
    /// every ordinary call; a self-recursive **tail** call updates the top
    /// frame in place instead of pushing, so a trampolined loop's stacktrace
    /// stays one frame deep no matter how many iterations it runs.
    call_stack: Vec<StackFrame>,
    /// The program's command-line arguments (after the script path).
    argv: Vec<String>,
}

/// One entry in [`Interp::call_stack`]: a call site, as the file it is
/// written in and the line it is written on.
#[derive(Clone)]
struct StackFrame {
    file: Arc<str>,
    line: usize,
}

/// One message for an actor's mailbox.
enum ActorMsg {
    /// `call_actor`: the sender is waiting on `reply`.
    Call {
        op: Value,
        args: Value,
        reply: Sender<ActorReply>,
    },
    /// `cast_actor`: nothing is waiting.
    Cast { op: Value, args: Value },
    /// Finish the mailbox and end the thread.
    Stop { done: Sender<ActorReply> },
}

/// What a worker sends back: the handler's result, plus whatever it printed.
///
/// Output is buffered in the worker and written by the receiving thread, so a
/// handler's `print` cannot interleave halfway through a line of the caller's.
struct ActorReply {
    result: Result<Value, RuntimeError>,
    out: Vec<u8>,
    errs: Vec<u8>,
}

/// A running actor, from the outside: the module that handles it and the
/// channel into its mailbox.
///
/// The actor's *state* is deliberately not here — it lives on the actor's own
/// thread, in that thread's own memory, and is reachable only by sending a
/// message. Nothing outside can read or write it.
struct ActorHandle {
    /// The id as it was written, so `list_actors` hands back the same value
    /// `start_actor` was given rather than a rendering of it.
    id: Value,
    module: String,
    /// Messages sent, counted on this side. A cast is counted as sent, not as
    /// handled — the actor may still be working on it.
    calls: u64,
    casts: u64,
    /// Casts sent whose outcome has not been collected yet.
    pending: u64,
    tx: Sender<ActorMsg>,
    /// What finished casts left behind: their output, and any failure. Nobody
    /// was waiting when they ran, so it is collected here and delivered at the
    /// next call or at the end-of-program drain.
    notices: Receiver<ActorReply>,
    join: Option<JoinHandle<()>>,
}

/// The outcome of evaluating a function body in tail position: either a final
/// value, or a self-recursive tail call the trampoline should loop on.
enum Flow {
    Return(Value),
    TailCall(Vec<Value>),
}

/// Identifies the function a trampoline is running, so a bare call in tail
/// position can be recognized as self-recursive.
struct TailTarget {
    name: String,
    module: Option<Arc<ModuleDef>>,
}

/// What a `x.f()` call dispatches to: a module, or a struct instance that
/// passes itself as the first argument.
enum MethodTarget {
    Module(Arc<ModuleDef>),
    Instance(Value, Arc<ModuleDef>),
}

impl MethodTarget {
    fn module(&self) -> &Arc<ModuleDef> {
        match self {
            MethodTarget::Module(m) => m,
            MethodTarget::Instance(_, m) => m,
        }
    }
}

impl Interp {
    /// Register the definitions (`function` / `module`) a program introduces so they
    /// are visible to every statement and to each other. Shared by whole-program
    /// runs and the REPL, where each entry adds to the accumulated definitions.
    fn register_items(&mut self, program: &Program) -> Result<(), RuntimeError> {
        // Traits first: an `implements` is checked as its module or struct
        // registers, and the trait it names may be defined further down the
        // file (or, in the REPL, in this same entry).
        for item in &program.items {
            match item {
                Item::Trait(t) => {
                    self.traits.insert(t.name.to_string(), Arc::new(t.clone()));
                }
                Item::Function(f) => {
                    self.funcs.insert(f.name.clone(), Arc::new(f.clone()));
                }
                _ => {}
            }
        }
        for item in &program.items {
            if let Item::Module(m) = item {
                let functions =
                    self.with_trait_functions(&m.name, &m.implements, &m.functions, &m.aliases)?;
                self.modules.insert(
                    m.name.to_string(),
                    Arc::new(ModuleDef {
                        functions,
                        ..m.clone()
                    }),
                );
            }
        }
        for item in &program.items {
            if let Item::Struct(s) = item {
                let name = s.name.to_string();
                let functions =
                    self.with_trait_functions(&s.name, &s.implements, &s.functions, &s.aliases)?;
                self.structs.insert(name.clone(), Arc::new(s.clone()));
                // The module view carries the struct's functions, so method
                // dispatch, `helper` visibility and bare sibling calls all go
                // through the same code paths modules use.
                self.modules.insert(
                    name,
                    Arc::new(ModuleDef {
                        name: s.name.clone(),
                        // The struct's own `implements` is resolved into
                        // `functions` below, so the view carries none.
                        implements: Vec::new(),
                        aliases: s.aliases.clone(),
                        functions,
                        span: s.span,
                    }),
                );
            }
        }
        Ok(())
    }

    /// The functions a module or struct exposes: its own, plus a default
    /// implementation inherited from each trait it implements for every
    /// function it does not define itself.
    ///
    /// A trait function *without* a body is a requirement rather than a
    /// default, so the implementer has to supply it — unless another of its
    /// traits offers a default of the same name.
    ///
    /// Modules and structs are held to the same contract. A module has no
    /// instance, so it satisfies a trait with plain module functions; that is
    /// what lets `Actor` be implemented by a module.
    fn with_trait_functions(
        &self,
        owner: &ModulePath,
        implements: &[ModulePath],
        own: &[FnDef],
        aliases: &[(String, ModulePath)],
    ) -> Result<Vec<FnDef>, RuntimeError> {
        if implements.is_empty() {
            return Ok(own.to_vec());
        }
        let defines = |name: &str| own.iter().any(|f| f.name == name);
        let mut required: Vec<(FnDef, String)> = Vec::new();
        let mut defaults: Vec<(FnDef, String)> = Vec::new();

        for path in implements {
            // `implements` may name a trait by its bare aliased name — and the
            // `alias` line establishing it can appear anywhere in the same
            // struct or module, including after `implements` — so it is
            // resolved against this owner's own aliases before falling back
            // to the written name.
            let trait_name = if let [short] = path.0.as_slice() {
                aliases
                    .iter()
                    .find(|(n, _)| n == short)
                    .map(|(_, full)| full.to_string())
                    .unwrap_or_else(|| path.to_string())
            } else {
                path.to_string()
            };
            let Some(t) = self.traits.get(&trait_name) else {
                return err(format!(
                    "`{owner}` implements `{trait_name}`, which is not a trait"
                ));
            };
            for f in &t.functions {
                let bucket = if f.body.is_empty() {
                    &mut required
                } else {
                    &mut defaults
                };
                bucket.push((f.clone(), trait_name.clone()));
            }
        }

        let mut functions = own.to_vec();
        let mut inherited: Vec<(String, String)> = Vec::new();
        for (f, trait_name) in defaults {
            if defines(&f.name) {
                continue; // the struct's own function wins
            }
            if let Some((_, first)) = inherited.iter().find(|(name, _)| *name == f.name) {
                return err(format!(
                    "`{owner}` inherits a default `{}` from both `{first}` and \
                     `{trait_name}` — define `{}` on `{owner}` to settle which one applies",
                    f.name, f.name
                ));
            }
            inherited.push((f.name.clone(), trait_name));
            functions.push(f);
        }

        let missing: Vec<String> = required
            .iter()
            .filter(|(f, _)| !defines(&f.name) && !inherited.iter().any(|(n, _)| *n == f.name))
            .map(|(f, trait_name)| {
                format!("`{}({})` from `{trait_name}`", f.name, f.params.join(", "))
            })
            .collect();
        if !missing.is_empty() {
            return err(format!(
                "`{owner}` does not implement {} — a trait function without a body is \
                 required, so everything that implements the trait must define it",
                missing.join(", ")
            ));
        }
        Ok(functions)
    }

    fn run_program(&mut self, program: &Program) -> Result<Value, RuntimeError> {
        // Definitions are visible to every statement and to each other, so
        // register modules and top-level functions before running anything.
        self.register_items(program)?;
        // An entrypoint is a module exposing a public `function main()`.
        let entries: Vec<Arc<ModuleDef>> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Module(m) if m.functions.iter().any(|f| f.name == "main" && !f.private) => {
                    self.modules.get(&m.name.to_string()).cloned()
                }
                _ => None,
            })
            .collect();
        match entries.as_slice() {
            [] => {
                let result = self.run_top_level(program);
                // A cast sent by the last statement still has to run.
                self.drain_all_actors()?;
                result
            }
            [entry] => {
                let main = entry
                    .functions
                    .iter()
                    .find(|f| f.name == "main")
                    .expect("entrypoint has a main");
                let result = self.call_module_fn(entry.clone(), Arc::new(main.clone()), Vec::new());
                self.drain_all_actors()?;
                result
            }
            many => {
                let names: Vec<String> = many.iter().map(|m| m.name.to_string()).collect();
                err(format!(
                    "multiple entrypoints define `main`: {}",
                    names.join(", ")
                ))
            }
        }
    }

    fn run_top_level(&mut self, program: &Program) -> Result<Value, RuntimeError> {
        let root = self.root.clone();
        let mut last = Value::Nil;
        for item in &program.items {
            if let Item::Statement(stmt) = item {
                last = self.eval_stmt(stmt, &root)?;
            }
        }
        Ok(last)
    }

    fn eval_block(&mut self, block: &Block, env: &Env) -> Result<Value, RuntimeError> {
        let mut last = Value::Nil;
        for stmt in block {
            last = self.eval_stmt(stmt, env)?;
        }
        Ok(last)
    }

    fn eval_stmt(&mut self, stmt: &Stmt, env: &Env) -> Result<Value, RuntimeError> {
        match stmt {
            Stmt::Expr(e) => self.eval_expr(e, env),
            Stmt::Assign { pattern, value } => {
                // A lambda is anonymous and non-recursive: it may not refer to
                // the name it is being bound to. `capture` (below) already
                // makes a closure's captured scope independent of whatever
                // name it ends up assigned to, so this check is no longer
                // what prevents a reference cycle — it is kept as an early,
                // clear error (`f = do x -> f(x)` fails right here) rather
                // than a confusing "undefined variable `f`" wherever the
                // lambda is later called.
                if let (Pattern::Binding(name), Expr::Lambda { params, body }) = (pattern, value) {
                    if freevars::free_names(params, body).iter().any(|n| n == name) {
                        return err(format!(
                            "a lambda cannot refer to itself (`{name}`); \
                             use a named `function` for recursion"
                        ));
                    }
                }
                let v = self.eval_expr(value, env)?;
                match pattern::try_match(pattern, &v)? {
                    Some(binds) => {
                        for (name, bound) in binds {
                            env.set(name, bound);
                        }
                        Ok(v)
                    }
                    None => err(format!(
                        "no match of right-hand side value: {}",
                        v.inspect()
                    )),
                }
            }
            // A top-level `alias` records a short name for later statements to
            // resolve through; it produces no value of its own.
            Stmt::Alias { module, name } => {
                self.aliases.insert(name.clone(), module.to_string());
                Ok(Value::Nil)
            }
        }
    }

    fn eval_expr(&mut self, expr: &Expr, env: &Env) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Symbol(s) => Ok(Value::Symbol(s.as_str().into())),
            Expr::Str(pieces) => self.eval_str(pieces, env),
            Expr::List { elements, rest } => {
                if rest.is_some() {
                    return err("`[.. | rest]` is only valid on the left of `=`");
                }
                let mut items = Vec::with_capacity(elements.len());
                for e in elements {
                    items.push(self.eval_expr(e, env)?);
                }
                Ok(Value::List(List::from_vec(items)))
            }
            Expr::Tuple(elements) => {
                let mut items = Vec::with_capacity(elements.len());
                for e in elements {
                    items.push(self.eval_expr(e, env)?);
                }
                Ok(Value::Tuple(items.into()))
            }
            Expr::Map(entries) => {
                let mut map = Map::default();
                for (key, value) in entries {
                    let v = self.eval_expr(value, env)?;
                    map = map.put(map_key_value(key), v);
                }
                Ok(Value::Map(Arc::new(map)))
            }
            // A bare name is always a binding: constants are functions, so
            // reading one is a call like any other.
            Expr::Var(name) => match env.get(name) {
                Some(v) => Ok(v),
                None => err(format!("undefined variable `{name}`")),
            },
            Expr::Lambda { params, body } => Ok(Value::Lambda(Arc::new(Closure {
                params: params.clone(),
                body: body.clone(),
                env: capture(params, body, env),
                module: self.current_module.clone(),
                file: self.current_file.clone(),
            }))),
            Expr::Unary { op, operand } => {
                let v = self.eval_expr(operand, env)?;
                eval_unary(*op, v)
            }
            Expr::Binary { op, left, right } => self.eval_binary(*op, left, right, env),
            Expr::Call { callee, args, line } => match callee {
                Callee::Local(name) if name == "assert" => self.eval_assert(args, env),
                Callee::Local(name) => {
                    let arg_vals = self.eval_args(args, env)?;
                    self.with_frame(*line, |me| me.call_local(name, arg_vals, env))
                }
                Callee::Method { target, name } => {
                    // The receiver is evaluated before the arguments, so a
                    // call reads left to right.
                    let receiver = self.method_receiver(target, env)?;
                    let mut arg_vals = Vec::new();
                    // `p.hello()` is `Person.hello(p)`: an instance passes
                    // itself as the first argument, which is the `self` param.
                    if let MethodTarget::Instance(instance, _) = &receiver {
                        arg_vals.push(instance.clone());
                    }
                    arg_vals.extend(self.eval_args(args, env)?);
                    self.with_frame(*line, |me| {
                        me.call_module_method(receiver.module(), name, arg_vals)
                    })
                }
            },
            Expr::Case { subject, arms } => self.eval_case(subject, arms, env),
            Expr::Cond { arms } => self.eval_cond(arms, env),
            Expr::If {
                condition,
                then_body,
                else_body,
            } => match self.select_if_branch(condition, then_body, else_body, env)? {
                Some((body, scope)) => self.eval_block(body, &scope),
                // No `else`, and the condition was falsy.
                None => Ok(Value::Nil),
            },
            Expr::Run { body, arms } => {
                let value = self.eval_run_body(body, env)?;
                if arms.is_empty() {
                    return Ok(value);
                }
                match self.select_case_arm(&value, arms, env)? {
                    Some((body, scope)) => self.eval_block(body, &scope),
                    None => err(format!("no case clause matched: {}", value.inspect())),
                }
            }
            Expr::ModuleRef(path) => Ok(Value::Module(self.resolve_module(path)?)),
            // Inside a struct method `self` is the instance, bound as an
            // ordinary parameter; inside a module function it is the module.
            Expr::SelfRef => match env.get("self") {
                Some(instance) => Ok(instance),
                None => match self.current_module.clone() {
                    Some(m) => Ok(Value::Module(m)),
                    None => err("`self` is only valid inside a module or struct"),
                },
            },
            Expr::Access { target, name } => {
                let value = self.eval_expr(target, env)?;
                match value {
                    Value::Module(m) => err(format!(
                        "`{}.{name}` is a function — call it with `{name}()`",
                        m.name
                    )),
                    Value::Struct(s) => match s.get(name) {
                        Some(v) => Ok(v.clone()),
                        None => err(format!("`{}` has no attribute `{name}`", s.def.name)),
                    },
                    other => err(format!("cannot read `{name}` on a {}", other.type_name())),
                }
            }
            Expr::StructLit { path, fields } => self.eval_struct_lit(path, fields, env),
            Expr::Wildcard => err("`_` is only valid in a pattern"),
        }
    }

    fn eval_args(&mut self, args: &[Expr], env: &Env) -> Result<Vec<Value>, RuntimeError> {
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval_expr(a, env)?);
        }
        Ok(values)
    }

    /// What a `target.f(...)` call dispatches to. A written path resolves
    /// without evaluating anything; anything else is evaluated, and a struct
    /// instance dispatches to its own definition with itself as `self`.
    fn method_receiver(&mut self, target: &Expr, env: &Env) -> Result<MethodTarget, RuntimeError> {
        // `self.f()` inside a struct method is a call on the instance.
        if let Expr::SelfRef = target {
            if let Some(instance) = env.get("self") {
                let module = self.module_for_value(&instance)?;
                return Ok(MethodTarget::Instance(instance, module));
            }
            return self
                .current_module
                .clone()
                .map(MethodTarget::Module)
                .ok_or_else(|| {
                    RuntimeError::new("`self` is only valid inside a module or struct")
                });
        }
        if let Expr::ModuleRef(path) = target {
            return self.resolve_module(path).map(MethodTarget::Module);
        }
        match self.eval_expr(target, env)? {
            Value::Module(m) => Ok(MethodTarget::Module(m)),
            value @ Value::Struct(_) => {
                let module = self.module_for_value(&value)?;
                Ok(MethodTarget::Instance(value, module))
            }
            value => err(format!("cannot call a function on a {}", value.type_name())),
        }
    }

    /// The module view backing a value's own type — for a struct instance, the
    /// one registered alongside its definition.
    fn module_for_value(&self, value: &Value) -> Result<Arc<ModuleDef>, RuntimeError> {
        self.module_named(&value.type_name())
    }

    /// Build an instance: every declared attribute, defaulted, then overridden
    /// by the literal's fields. A field the struct does not declare is an
    /// error rather than a silent extra.
    fn eval_struct_lit(
        &mut self,
        path: &ModulePath,
        fields: &[(String, Expr)],
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        let name = self.resolve_name(path);
        let Some(def) = self.structs.get(&name).cloned() else {
            return err(format!("undefined struct `{name}`"));
        };
        for (field, _) in fields {
            if !def.attributes.iter().any(|(n, _)| n == field) {
                return err(format!("`{name}` has no attribute `{field}`"));
            }
        }
        let mut values = Vec::with_capacity(def.attributes.len());
        for (attr, default) in &def.attributes {
            let value = match fields.iter().find(|(n, _)| n == attr) {
                Some((_, e)) => self.eval_expr(e, env)?,
                // Defaults are expressions, evaluated per construction in the
                // struct's own context so one may call the struct's functions.
                None => {
                    let scope = Env::root();
                    let module = self.modules.get(&name).cloned();
                    self.in_module(module, |me| me.eval_expr(default, &scope))?
                }
            };
            values.push((attr.clone(), value));
        }
        Ok(Value::Struct(Arc::new(StructValue {
            def,
            fields: values,
        })))
    }

    fn eval_str(&mut self, pieces: &[StrPiece], env: &Env) -> Result<Value, RuntimeError> {
        let mut out = String::new();
        for piece in pieces {
            match piece {
                StrPiece::Lit(text) => out.push_str(text),
                StrPiece::Interp(e) => out.push_str(&self.eval_expr(e, env)?.display()),
            }
        }
        Ok(Value::Str(out.into()))
    }

    fn call_local(
        &mut self,
        name: &str,
        args: Vec<Value>,
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        // `native` is the host seam, not an ordinary function: `Kernel.native`
        // is a body-less declaration whose calls the interpreter intercepts.
        // It is matched first so a Kernel body calling it does not resolve to
        // that declaration and fail.
        //
        // A couple of natives run a lambda (`start_thread`, `await_thread`),
        // which needs the evaluator — `natives::dispatch` only has the shared
        // sinks, not `self`, so those are answered here instead of falling
        // through to it.
        if name == "native" {
            if let [Value::Str(native_name), Value::List(inner)] = args.as_slice() {
                let inner_args = inner.to_vec();
                match native_name.as_ref() {
                    "start_thread" => return self.start_thread(inner_args),
                    "await_thread" => return self.await_thread(inner_args),
                    // `natives::dispatch` has no access to `self.call_stack`
                    // (it only gets the shared sinks), so this is answered
                    // here instead — same reason as the two above.
                    "current_stacktrace" => return Ok(self.stacktrace_value()),
                    _ => {}
                }
            }
            return match self.call_native(name, &args) {
                Some(result) => result,
                None => err("unknown native"),
            };
        }
        // `apply` runs a lambda, so it cannot be a native — a native has no way
        // back into the interpreter. Like `native`, it is answered here.
        if name == "apply" {
            return self.call_apply(args);
        }
        // Same reason as `apply`: both run Ramos code, which a native cannot.
        if name == "start_actor" {
            return self.start_actor(args);
        }
        if name == "call_actor" {
            return self.call_actor(args);
        }
        if name == "cast_actor" {
            return self.cast_actor(args);
        }
        if name == "list_actors" {
            return self.list_actors(args);
        }
        if name == "actor_stats" {
            return self.actor_stats(args);
        }
        if name == "is_actor_started" {
            return self.is_actor_started(args);
        }
        if name == "stop_actor" {
            return self.stop_actor(args);
        }
        // Same reason as `apply`: resolving `fn_name` on a module and running
        // it needs the evaluator, which a native cannot reach.
        if name == "module_apply" {
            return self.call_module_apply(args);
        }
        // Resolution order: a local binding holding a lambda, then the current
        // module's own `function`/`helper`, then top-level functions, then the implicit
        // `Kernel` module, and finally the native table directly (which is what
        // answers before a stdlib is loaded).
        if let Some(v) = env.get(name) {
            return match v {
                Value::Lambda(c) => self.apply_lambda(&c, args),
                other => err(format!("`{name}` is a {}, not callable", other.type_name())),
            };
        }
        if let Some(module) = self.current_module.clone() {
            if let Some(f) = module.functions.iter().find(|f| f.name == name) {
                let f = Arc::new(f.clone());
                return self.call_module_fn(module, f, args);
            }
        }
        if let Some(f) = self.funcs.get(name).cloned() {
            return self.apply_fn(&f, args);
        }
        // Kernel is in implicit scope: its public functions are callable
        // unqualified, and it never needs an `alias`.
        if let Some(kernel) = self.modules.get("Kernel").cloned() {
            if let Some(f) = kernel
                .functions
                .iter()
                .find(|f| f.name == name && !f.private)
            {
                let f = Arc::new(f.clone());
                return self.call_module_fn(kernel, f, args);
            }
        }
        if let Some(result) = self.call_native(name, &args) {
            return result;
        }
        err(format!("undefined function `{name}`"))
    }

    /// Dispatch a native. The shared sinks are passed by reference, not locked
    /// here: a native locks one only around its own write, so a blocking native
    /// (`sleep`, `read`) never holds the sink and threads still run in parallel.
    fn call_native(&self, name: &str, args: &[Value]) -> Option<Result<Value, RuntimeError>> {
        natives::call(&self.out, &self.errs, &self.argv, name, args)
    }

    /// `current_stacktrace()`'s value: `self.call_stack`, most recent call
    /// first, main (or wherever the trail runs out) last. The two innermost
    /// frames are always `current_stacktrace`'s own — the call into it, and
    /// the call it makes in turn into `native(...)` — neither is a call
    /// anyone wrote to learn where they are; both are dropped so the first
    /// entry is the real call site the trace is about.
    fn stacktrace_value(&self) -> Value {
        let frames = self
            .call_stack
            .iter()
            .rev()
            .skip(2)
            .map(|f| {
                Value::Tuple(vec![Value::Str(f.file.clone()), Value::Int(f.line as i64)].into())
            })
            .collect();
        Value::List(List::from_vec(frames))
    }

    /// `native("name", [args])` — dispatch to the host handler behind a Kernel
    /// function. This is the one call the interpreter answers itself.
    /// `apply(f, [args])` — call a lambda with its arguments in a list, so a
    /// caller that does not know the arity at the call site can still invoke it.
    /// `start_actor(id, module, initial_state, config)` — start an actor on its
    /// own thread, holding `initial_state`. Returns `:ok`.
    ///
    /// The worker gets its own root scope and its own copy of the definition
    /// tables, so nothing it binds while handling a message is visible anywhere
    /// else. The state lives only on that thread: the sole way to reach it is
    /// to send a message.
    fn start_actor(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id, module, initial_state, config] = args.as_slice() else {
            return err(format!(
                "start_actor expects 4 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Module(m) = module else {
            return err(format!(
                "start_actor expects a Module, got {}",
                module.type_name()
            ));
        };
        // The handler is looked up on this module, so it has to have agreed to
        // answer messages. Checking here means a typo fails at `start`, not at
        // the first call.
        if !m.implements.iter().any(|t| t.to_string() == "Actor") {
            return err(format!(
                "`{}` cannot be started as an actor: it does not implement `Actor`",
                m.name
            ));
        }
        let key = id.inspect();
        if self.actors.contains_key(&key) {
            return err(format!("actor `{key}` is already started"));
        }

        // Everything the worker needs to evaluate, moved in. The definition
        // tables are `Arc`-valued, so this clone is a handful of refcount bumps
        // rather than a deep copy of the program.
        let module_name = m.name.to_string();
        let worker_module_name = module_name.clone();
        let funcs = self.funcs.clone();
        let modules = self.modules.clone();
        let structs = self.structs.clone();
        let traits = self.traits.clone();
        let aliases = self.aliases.clone();
        let argv = self.argv.clone();
        let mut state = initial_state.clone();
        let config = config.clone();
        let handler_module = self.module_named(&module_name)?;

        let (tx, rx) = channel::<ActorMsg>();
        let (notice_tx, notices) = channel::<ActorReply>();
        // A handler recurses like any other Ramos code, so the actor's own thread
        // needs the same large stack the main one runs on (PLAN phase 9a); a
        // spawned thread's default is smaller still.
        let worker = move || {
            // The actor's own memory: a fresh root scope, its own definition
            // tables, and `state` owned right here on the stack.
            while let Ok(msg) = rx.recv() {
                let (callback, op, args, reply) = match msg {
                    ActorMsg::Call { op, args, reply } => ("call", op, args, Some(reply)),
                    ActorMsg::Cast { op, args } => ("cast", op, args, None),
                    ActorMsg::Stop { done } => {
                        let _ = done.send(ActorReply {
                            result: Ok(Value::Symbol("ok".into())),
                            out: Vec::new(),
                            errs: Vec::new(),
                        });
                        return;
                    }
                };
                // An actor still buffers its output and delivers it to whoever
                // collects the reply, so its lines never interleave (a thread is
                // the opposite — live). The buffers are `Arc<Mutex<Vec>>` only
                // because `Interp` now writes through a shared `Sink`; nothing
                // else shares these, so the bytes come straight back out.
                let out_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
                let errs_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
                let result = {
                    let mut interp = Interp {
                        out: out_buf.clone(),
                        errs: errs_buf.clone(),
                        root: Env::root(),
                        funcs: funcs.clone(),
                        modules: modules.clone(),
                        structs: structs.clone(),
                        traits: traits.clone(),
                        actors: HashMap::new(),
                        aliases: aliases.clone(),
                        current_module: None,
                        // Overwritten the instant `call_module_method` enters
                        // the handler function; the actor's own thread has no
                        // Ramos caller of its own to record here.
                        current_file: Arc::from(""),
                        call_stack: Vec::new(),
                        argv: argv.clone(),
                    };
                    interp.call_module_method(
                        &handler_module,
                        callback,
                        vec![op, args, state.clone(), config.clone()],
                    )
                };
                // `call` replies with (reply, new_state); `cast` returns the
                // new state on its own.
                let result = match (callback, result) {
                    ("cast", Ok(new_state)) => {
                        state = new_state;
                        Ok(Value::Symbol("ok".into()))
                    }
                    ("call", Ok(pair)) => match &pair {
                        Value::Tuple(items) if items.len() == 2 => {
                            state = items[1].clone();
                            Ok(items[0].clone())
                        }
                        other => Err(RuntimeError::new(format!(
                            "`{worker_module_name}.call` must return a (reply, new_state) tuple, got {}",
                            other.type_name()
                        ))),
                    },
                    (_, Err(e)) => Err(e),
                    (_, Ok(v)) => Ok(v),
                };
                let out = std::mem::take(&mut *out_buf.lock().unwrap());
                let errs = std::mem::take(&mut *errs_buf.lock().unwrap());
                match reply {
                    Some(reply) => {
                        let _ = reply.send(ActorReply { result, out, errs });
                    }
                    // A cast has nobody waiting, so its output and any failure
                    // surface on the next call or at the end-of-program drain.
                    None => {
                        let _ = notice_tx.send(ActorReply { result, out, errs });
                    }
                }
            }
        };
        // `Builder::spawn` reports an OS failure to make the thread rather than
        // panicking, as the bare `thread::spawn` used to; there is nothing to do
        // but stop, so surface it as a runtime error at the `start_actor` call.
        let join = std::thread::Builder::new()
            .name(format!("ramos-actor-{key}"))
            .stack_size(crate::stack::stack_size())
            .spawn(worker)
            .map_err(|e| RuntimeError::new(format!("cannot start actor `{key}`: {e}")))?;

        self.actors.insert(
            key,
            ActorHandle {
                id: id.clone(),
                module: module_name,
                calls: 0,
                casts: 0,
                pending: 0,
                tx,
                notices,
                join: Some(join),
            },
        );
        Ok(Value::Symbol("ok".into()))
    }

    /// `call_actor(id, module, op, args)` — send one message and wait.
    ///
    /// The handler runs on the actor's thread; this one blocks until the reply
    /// comes back. Anything the handler printed is written here, so a line of
    /// its output never lands inside a line of ours.
    fn call_actor(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id, module, op, op_args] = args.as_slice() else {
            return err(format!(
                "call_actor expects 4 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Module(m) = module else {
            return err(format!(
                "call_actor expects a Module, got {}",
                module.type_name()
            ));
        };
        let key = id.inspect();
        self.check_actor(&key, m)?;
        // Everything cast before this call has already been queued ahead of it
        // on the same channel, so the actor handles them first. Deliver what
        // they left behind before this call's own output.
        self.deliver_notices(&key)?;

        let (reply_tx, reply_rx) = channel::<ActorReply>();
        let sent = self.actors.get(&key).map(|a| {
            a.tx.send(ActorMsg::Call {
                op: op.clone(),
                args: op_args.clone(),
                reply: reply_tx,
            })
        });
        if !matches!(sent, Some(Ok(()))) {
            return err(format!("actor `{key}` has stopped"));
        }
        if let Some(handle) = self.actors.get_mut(&key) {
            handle.calls += 1;
        }
        let reply = reply_rx
            .recv()
            .map_err(|_| RuntimeError::new(format!("actor `{key}` stopped without replying")))?;
        self.write_actor_output(&reply);
        reply.result
    }

    /// `cast_actor(id, module, op, args)` — send one message without waiting.
    ///
    /// Returns `:ok` as soon as the message is on its way. The handler runs on
    /// the actor's own thread, so it may still be running when the next
    /// statement here starts — that is what makes it asynchronous.
    fn cast_actor(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id, module, op, op_args] = args.as_slice() else {
            return err(format!(
                "cast_actor expects 4 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Module(m) = module else {
            return err(format!(
                "cast_actor expects a Module, got {}",
                module.type_name()
            ));
        };
        let key = id.inspect();
        self.check_actor(&key, m)?;
        let sent = self.actors.get(&key).map(|a| {
            a.tx.send(ActorMsg::Cast {
                op: op.clone(),
                args: op_args.clone(),
            })
        });
        if !matches!(sent, Some(Ok(()))) {
            return err(format!("actor `{key}` has stopped"));
        }
        if let Some(handle) = self.actors.get_mut(&key) {
            handle.casts += 1;
            handle.pending += 1;
        }
        Ok(Value::Symbol("ok".into()))
    }

    /// The actor exists and is handled by the module the caller named.
    fn check_actor(&self, key: &str, m: &Arc<ModuleDef>) -> Result<(), RuntimeError> {
        let Some(handle) = self.actors.get(key) else {
            return err(format!("actor `{key}` is not started"));
        };
        if handle.module != m.name.to_string() {
            return err(format!(
                "actor `{key}` is handled by `{}`, not `{}`",
                handle.module, m.name
            ));
        }
        Ok(())
    }

    /// Write a worker's buffered output through to our own streams.
    fn write_actor_output(&mut self, reply: &ActorReply) {
        let _ = self
            .out
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(&reply.out);
        let _ = self
            .errs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(&reply.errs);
    }

    /// Deliver whatever finished casts left behind for one actor: their output,
    /// and the first failure among them.
    fn deliver_notices(&mut self, key: &str) -> Result<(), RuntimeError> {
        let pending: Vec<ActorReply> = match self.actors.get(key) {
            Some(handle) => handle.notices.try_iter().collect(),
            None => return Ok(()),
        };
        let mut failure = None;
        for reply in pending {
            self.write_actor_output(&reply);
            if let Some(handle) = self.actors.get_mut(key) {
                handle.pending = handle.pending.saturating_sub(1);
            }
            if let Err(e) = reply.result {
                failure.get_or_insert(e);
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Wait for every actor to finish what it has been sent, then stop it.
    ///
    /// Called when the program ends so a trailing `cast` is neither dropped nor
    /// silently left running: `Stop` is queued behind the messages already in
    /// the mailbox, so replying to it means the mailbox is drained.
    fn drain_all_actors(&mut self) -> Result<(), RuntimeError> {
        let keys: Vec<String> = self.actors.keys().cloned().collect();
        let mut failure = None;
        for key in keys {
            if let Err(e) = self.stop_one_actor(&key) {
                failure.get_or_insert(e);
            }
        }
        self.actors.clear();
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Let one actor finish its mailbox, then end its thread and forget it.
    ///
    /// Returns `:ok`, or the failure a pending cast left behind — the actor is
    /// stopped either way.
    fn stop_one_actor(&mut self, key: &str) -> Result<Value, RuntimeError> {
        let (done_tx, done_rx) = channel::<ActorReply>();
        let sent = self
            .actors
            .get(key)
            .map(|a| a.tx.send(ActorMsg::Stop { done: done_tx }));
        if matches!(sent, Some(Ok(()))) {
            // The worker answers `Stop` only after everything queued ahead of
            // it has run, so nothing in flight is discarded.
            let _ = done_rx.recv();
        }
        let outcome = self.deliver_notices(key);
        if let Some(handle) = self.actors.get_mut(key) {
            if let Some(join) = handle.join.take() {
                let _ = join.join();
            }
        }
        self.actors.remove(key);
        outcome.map(|()| Value::Symbol("ok".into()))
    }

    /// `list_actors()` — every running actor as an `(id, module)` pair.
    fn list_actors(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        if !args.is_empty() {
            return err(format!(
                "list_actors expects 0 argument(s), got {}",
                args.len()
            ));
        }
        let mut rows: Vec<(String, Value, String)> = self
            .actors
            .iter()
            .map(|(key, handle)| (key.clone(), handle.id.clone(), handle.module.clone()))
            .collect();
        // A HashMap has no order of its own; sort by id so the list is stable
        // from one call to the next.
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let mut pairs = Vec::with_capacity(rows.len());
        for (_, id, module_name) in rows {
            // The module itself, not its name, so the pair can be handed
            // straight back to `call_actor`.
            let module = self.module_named(&module_name)?;
            pairs.push(Value::Tuple(Arc::from(vec![id, Value::Module(module)])));
        }
        Ok(Value::List(List::from_vec(pairs)))
    }

    /// `actor_stats(id)` — a map describing one running actor.
    fn actor_stats(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id] = args.as_slice() else {
            return err(format!(
                "actor_stats expects 1 argument(s), got {}",
                args.len()
            ));
        };
        let key = id.inspect();
        // Collect finished casts first, so `pending` reflects what is actually
        // still outstanding rather than everything ever sent.
        self.deliver_notices(&key)?;
        let Some(handle) = self.actors.get(&key) else {
            return err(format!("actor `{key}` is not started"));
        };
        let alive = handle
            .join
            .as_ref()
            .map(|j| !j.is_finished())
            .unwrap_or(false);
        let entries = vec![
            (Value::Symbol("id".into()), handle.id.clone()),
            (
                Value::Symbol("module".into()),
                Value::Str(handle.module.as_str().into()),
            ),
            (
                Value::Symbol("calls".into()),
                Value::Int(handle.calls as i64),
            ),
            (
                Value::Symbol("casts".into()),
                Value::Int(handle.casts as i64),
            ),
            (
                Value::Symbol("pending".into()),
                Value::Int(handle.pending as i64),
            ),
            (Value::Symbol("alive".into()), Value::Bool(alive)),
        ];
        Ok(Value::Map(Arc::new(Map { entries })))
    }

    /// `is_actor_started(id)` — whether `id` names a running actor.
    fn is_actor_started(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id] = args.as_slice() else {
            return err(format!(
                "is_actor_started expects 1 argument(s), got {}",
                args.len()
            ));
        };
        Ok(Value::Bool(self.actors.contains_key(&id.inspect())))
    }

    /// `stop_actor(id)` — finish what the actor has been sent, then stop it.
    ///
    /// The `Stop` goes on the mailbox behind the messages already queued, so
    /// nothing in flight is discarded.
    fn stop_actor(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [id] = args.as_slice() else {
            return err(format!(
                "stop_actor expects 1 argument(s), got {}",
                args.len()
            ));
        };
        let key = id.inspect();
        if !self.actors.contains_key(&key) {
            return err(format!("actor `{key}` is not started"));
        }
        self.stop_one_actor(&key)
    }

    /// `assert(condition)` / `assert(condition, message)`.
    ///
    /// Returns `:ok` when the condition holds, and fails the test otherwise.
    /// The failure travels the ordinary error path, so it stops the test
    /// function it is in and the runner reports it — Ramos has no exceptions to
    /// throw, and none are needed.
    ///
    /// This is handled here rather than in the native table so it can see the
    /// *expression* it was given, not just the value: an `==` that fails can
    /// then report both sides instead of "assertion failed".
    fn eval_assert(&mut self, args: &[Expr], env: &Env) -> Result<Value, RuntimeError> {
        let (condition, message) = match args {
            [c] => (c, None),
            [c, m] => (c, Some(m)),
            _ => {
                return err(format!(
                    "assert expects 1 or 2 argument(s), got {}",
                    args.len()
                ))
            }
        };
        if self.eval_expr(condition, env)?.is_truthy() {
            return Ok(Value::Symbol("ok".into()));
        }
        if let Some(m) = message {
            let text = self.eval_expr(m, env)?;
            return err(format!("assertion failed: {}", text.display()));
        }
        // A comparison is the common case, and the useful report is both sides.
        if let Expr::Binary { op, left, right } = condition {
            let shown = match op {
                BinOp::Eq => Some("to equal"),
                BinOp::NotEq => Some("not to equal"),
                BinOp::Lt => Some("to be less than"),
                BinOp::Le => Some("to be at most"),
                BinOp::Gt => Some("to be greater than"),
                BinOp::Ge => Some("to be at least"),
                _ => None,
            };
            if let Some(relation) = shown {
                let l = self.eval_expr(left, env)?;
                let r = self.eval_expr(right, env)?;
                return err(format!(
                    "assertion failed: expected {} {relation} {}",
                    l.inspect(),
                    r.inspect()
                ));
            }
        }
        err("assertion failed")
    }

    /// `start_thread(lambda)` — run a zero-argument lambda on its own thread,
    /// returning a handle at once. Nothing is shared with the worker but the
    /// lambda itself: it carries its own captured scope, and the worker gets a
    /// fresh root and its own copy of the definition tables, like an actor.
    ///
    /// The handle is a value — hold it, pass it, and `await_thread` it when you
    /// want the result. The thread's output goes live to the shared sink as it
    /// runs; a thread you never await still runs (and prints), but its *result*
    /// is never collected, and it may be cut off when the program ends.
    fn start_thread(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [f] = args.as_slice() else {
            return err(format!(
                "start_thread expects 1 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Lambda(closure) = f else {
            return err(format!(
                "start_thread expects a Lambda, got {}",
                f.type_name()
            ));
        };
        if !closure.params.is_empty() {
            return err(format!(
                "start_thread expects a lambda of no arguments, got one of {}",
                closure.params.len()
            ));
        }

        // Everything the worker needs, moved in. The tables are `Arc`-valued, so
        // this is a handful of refcount bumps rather than a deep copy. The
        // output sinks are shared, not buffered: a thread writes **live** to the
        // same place as the spawner, serialized by the sink's mutex, so its
        // output appears where it happens rather than waiting for the await.
        let closure = closure.clone();
        let funcs = self.funcs.clone();
        let modules = self.modules.clone();
        let structs = self.structs.clone();
        let traits = self.traits.clone();
        let aliases = self.aliases.clone();
        let argv = self.argv.clone();
        let out = self.out.clone();
        let errs = self.errs.clone();
        let (reply_tx, reply_rx) = channel::<ThreadReply>();

        let worker = move || {
            let result = {
                let mut interp = Interp {
                    out,
                    errs,
                    root: Env::root(),
                    funcs,
                    modules,
                    structs,
                    traits,
                    // A worker cannot see the spawner's actors, the same
                    // isolation an actor handler has.
                    actors: HashMap::new(),
                    aliases,
                    current_module: None,
                    // Overwritten the instant `apply_lambda` enters the
                    // closure's body; the spawned thread has no Ramos caller
                    // of its own to record here.
                    current_file: Arc::from(""),
                    call_stack: Vec::new(),
                    argv,
                };
                interp.apply_lambda(&closure, vec![])
            };
            // The structured error is flattened to its message: a thread failure
            // is reported where it is awaited, not unwound through the program.
            let result = result.map_err(|e| e.message);
            let _ = reply_tx.send(ThreadReply { result });
        };

        // Sized to the large stack, like every interpreter thread — a spawned
        // lambda recurses like any other Ramos code (PLAN phase 9a).
        let join = std::thread::Builder::new()
            .name("ramos-thread".to_string())
            .stack_size(crate::stack::stack_size())
            .spawn(worker)
            .map_err(|e| RuntimeError::new(format!("cannot spawn thread: {e}")))?;

        let handle = ThreadHandle {
            state: std::sync::Mutex::new(ThreadState::Running {
                join: Some(join),
                reply: reply_rx,
            }),
        };
        Ok(Value::Thread(Arc::new(handle)))
    }

    /// `await_thread(handle)` — wait for a spawned thread and return what its
    /// lambda produced, propagating a failure inside it as a runtime error.
    ///
    /// The thread's output was already written live, through the shared sink, as
    /// it happened — so awaiting only collects the value. Awaiting a handle twice
    /// returns the same value the first await saw.
    fn await_thread(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [handle] = args.as_slice() else {
            return err(format!(
                "await_thread expects 1 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Thread(handle) = handle else {
            return err(format!(
                "await_thread expects a Thread, got {}",
                handle.type_name()
            ));
        };
        let mut state = handle
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Already collected: hand back the cached value, output long since
        // written.
        if let ThreadState::Finished(result) = &*state {
            return result.clone().map_err(RuntimeError::new);
        }
        let ThreadState::Running { join, reply } = &mut *state else {
            unreachable!("state is Running here");
        };
        // The worker sends exactly one reply then ends; a channel error means it
        // panicked without sending, which is a bug rather than a Ramos failure.
        let ThreadReply { result } = reply
            .recv()
            .map_err(|_| RuntimeError::new("thread ended without a result (it panicked)"))?;
        if let Some(join) = join.take() {
            let _ = join.join();
        }
        *state = ThreadState::Finished(result.clone());
        result.map_err(RuntimeError::new)
    }

    fn call_apply(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [f, arglist] = args.as_slice() else {
            return err(format!("apply expects 2 argument(s), got {}", args.len()));
        };
        let Value::Lambda(c) = f else {
            return err(format!("apply expects a Lambda, got {}", f.type_name()));
        };
        let Value::List(items) = arglist else {
            return err(format!(
                "apply expects the arguments as a List, got {}",
                arglist.type_name()
            ));
        };
        let c = c.clone();
        let values = items.to_vec();
        self.apply_lambda(&c, values)
    }

    /// `module_apply(m, fn_name, args)` — resolve `fn_name` on module `m` and
    /// call it with `args`, a list. Backs `Module.apply`; enforces `helper`
    /// visibility the same way a written `m.fn_name(...)` call would, via
    /// `call_module_method`.
    fn call_module_apply(&mut self, args: Vec<Value>) -> Result<Value, RuntimeError> {
        let [module, fn_name, arglist] = args.as_slice() else {
            return err(format!(
                "module_apply expects 3 argument(s), got {}",
                args.len()
            ));
        };
        let Value::Module(m) = module else {
            return err(format!(
                "module_apply expects a Module, got {}",
                module.type_name()
            ));
        };
        let Value::Str(fn_name) = fn_name else {
            return err(format!(
                "module_apply expects the function name as a String, got {}",
                fn_name.type_name()
            ));
        };
        let Value::List(items) = arglist else {
            return err(format!(
                "module_apply expects the arguments as a List, got {}",
                arglist.type_name()
            ));
        };
        let m = m.clone();
        let fn_name = fn_name.to_string();
        let values = items.to_vec();
        self.call_module_method(&m, &fn_name, values)
    }

    fn apply_lambda(&mut self, c: &Arc<Closure>, args: Vec<Value>) -> Result<Value, RuntimeError> {
        if args.len() != c.params.len() {
            return err(format!(
                "lambda expects {} argument(s), got {}",
                c.params.len(),
                args.len()
            ));
        }
        let scope = c.env.child();
        for (param, arg) in c.params.iter().zip(args) {
            scope.set(param.clone(), arg);
        }
        // A lambda resolves bare calls against the module it was defined in,
        // and any call inside its body is a call from the file it was
        // written in.
        let saved_file = std::mem::replace(&mut self.current_file, c.file.clone());
        let result = self.in_module(c.module.clone(), |me| me.eval_block(&c.body, &scope));
        self.current_file = saved_file;
        result
    }

    fn apply_fn(&mut self, f: &Arc<FnDef>, args: Vec<Value>) -> Result<Value, RuntimeError> {
        self.run_function(f, None, args)
    }

    /// Call a module's function with `module` as the resolution context for the
    /// bare calls in its body.
    fn call_module_fn(
        &mut self,
        module: Arc<ModuleDef>,
        f: Arc<FnDef>,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        self.run_function(&f, Some(module), args)
    }

    /// Run a named function to completion. Self-recursive **tail** calls loop in
    /// place (constant stack) rather than recursing — the tail-recursive stdlib
    /// (`reduce`, `each`, …) and user loops depend on this. Non-tail and
    /// mutually-recursive calls still use the native stack.
    fn run_function(
        &mut self,
        f: &Arc<FnDef>,
        module: Option<Arc<ModuleDef>>,
        mut args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let display_name = match &module {
            Some(m) => format!("{}.{}", m.name, f.name),
            None => f.name.clone(),
        };
        if f.body.is_empty() {
            return err(format!("`{display_name}` is a declaration with no body"));
        }
        let target = TailTarget {
            name: f.name.clone(),
            module: module.clone(),
        };
        // Any call inside this body — including one a lambda it creates goes
        // on to make — is a call from this function's own file.
        let saved_file = std::mem::replace(&mut self.current_file, f.file.clone());
        let result = self.in_module(module, move |me| loop {
            if args.len() != f.params.len() {
                return err(format!(
                    "{display_name} expects {} argument(s), got {}",
                    f.params.len(),
                    args.len()
                ));
            }
            // A fresh frame per iteration, so the previous one is freed before
            // the next — the trampoline runs in constant space.
            let scope = me.root.child();
            for (param, arg) in f.params.iter().zip(&args) {
                scope.set(param.clone(), arg.clone());
            }
            match me.eval_body_tail(&f.body, &scope, &target)? {
                Flow::Return(value) => return Ok(value),
                Flow::TailCall(next) => args = next,
            }
        });
        self.current_file = saved_file;
        result
    }

    /// Resolve a written module path to its definition. A single-segment path
    /// may be an alias — the current module's own aliases first, then the
    /// top-level ones — before it is taken as a module name of its own.
    /// The full dotted name a written path refers to, following `alias` the
    /// same way [`resolve_module`] does. Used where the target is a struct
    /// rather than a module, so `alias MyApp.User` makes `User{...}` build one.
    ///
    /// [`resolve_module`]: Self::resolve_module
    fn resolve_name(&self, path: &ModulePath) -> String {
        if let [short] = path.0.as_slice() {
            if let Some(current) = &self.current_module {
                if let Some((_, full)) = current.aliases.iter().find(|(n, _)| n == short) {
                    return full.to_string();
                }
            }
            if let Some(full) = self.aliases.get(short) {
                return full.clone();
            }
        }
        path.to_string()
    }

    fn resolve_module(&self, path: &ModulePath) -> Result<Arc<ModuleDef>, RuntimeError> {
        if let [short] = path.0.as_slice() {
            if let Some(current) = &self.current_module {
                if let Some((_, full)) = current.aliases.iter().find(|(n, _)| n == short) {
                    return self.module_named(&full.to_string());
                }
            }
            if let Some(full) = self.aliases.get(short) {
                return self.module_named(full);
            }
        }
        self.module_named(&path.to_string())
    }

    /// Look a module up by its full dotted name.
    fn module_named(&self, name: &str) -> Result<Arc<ModuleDef>, RuntimeError> {
        self.modules
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("undefined module `{name}`")))
    }

    /// True when `module` is the one whose function is currently running —
    /// which is what makes its `helper` functions reachable.
    fn inside_module(&self, module: &ModuleDef) -> bool {
        self.current_module
            .as_ref()
            .is_some_and(|current| current.name == module.name)
    }

    /// Call `module.name(args)`, enforcing `helper` visibility.
    fn call_module_method(
        &mut self,
        module: &Arc<ModuleDef>,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        let Some(f) = module.functions.iter().find(|f| f.name == name) else {
            return err(format!("module `{}` has no function `{name}`", module.name));
        };
        if f.private && !self.inside_module(module) {
            return err(format!(
                "`{}.{name}` is private (`helper`) and can only be called from inside `{}`",
                module.name, module.name
            ));
        }
        let f = Arc::new(f.clone());
        self.call_module_fn(module.clone(), f, args)
    }

    /// Run `body` with `module` as the current resolution context, restoring the
    /// previous one afterward (even on error).
    fn in_module<T>(
        &mut self,
        module: Option<Arc<ModuleDef>>,
        body: impl FnOnce(&mut Self) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let saved = self.current_module.take();
        self.current_module = module;
        let result = body(self);
        self.current_module = saved;
        result
    }

    /// Run `body` with a call-stack frame pushed for `line` (in the file
    /// currently executing), popped afterward regardless of how `body` ends.
    /// This is what every ordinary call — not a self-recursive tail call,
    /// which reuses the frame already on top instead — wraps itself in.
    fn with_frame<T>(
        &mut self,
        line: usize,
        body: impl FnOnce(&mut Self) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        self.call_stack.push(StackFrame {
            file: self.current_file.clone(),
            line,
        });
        let result = body(self);
        self.call_stack.pop();
        result
    }

    /// Evaluate a function/arm body while tracking tail position: a
    /// self-recursive call in the final position yields `Flow::TailCall` for the
    /// trampoline instead of recursing.
    fn eval_body_tail(
        &mut self,
        block: &Block,
        env: &Env,
        target: &TailTarget,
    ) -> Result<Flow, RuntimeError> {
        let Some((last, rest)) = block.split_last() else {
            return Ok(Flow::Return(Value::Nil));
        };
        for stmt in rest {
            self.eval_stmt(stmt, env)?;
        }
        match last {
            Stmt::Expr(e) => self.eval_expr_tail(e, env, target),
            other => Ok(Flow::Return(self.eval_stmt(other, env)?)),
        }
    }

    fn eval_expr_tail(
        &mut self,
        e: &Expr,
        env: &Env,
        target: &TailTarget,
    ) -> Result<Flow, RuntimeError> {
        match e {
            Expr::Call {
                callee: Callee::Local(name),
                args,
                line,
            } if self.is_self_tail_call(name, env, target) => {
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval_expr(arg, env)?);
                }
                // The trampoline loops in place rather than recursing, so
                // this call gets no frame of its own — pushing one per
                // iteration would grow the stacktrace without bound for what
                // is, on the Rust stack, a single call. The frame already on
                // top (from whoever called into this function, if anyone
                // did) is updated in place instead, so a stacktrace taken
                // mid-loop still names the line the loop is currently at.
                if let Some(top) = self.call_stack.last_mut() {
                    top.line = *line;
                }
                Ok(Flow::TailCall(values))
            }
            // Tail position propagates into the taken branch of case / cond.
            Expr::Case { subject, arms } => {
                let subject = self.eval_expr(subject, env)?;
                match self.select_case_arm(&subject, arms, env)? {
                    Some((body, scope)) => self.eval_body_tail(body, &scope, target),
                    None => err(format!("no case clause matched: {}", subject.inspect())),
                }
            }
            Expr::Cond { arms } => match self.select_cond_arm(arms, env)? {
                Some((body, scope)) => self.eval_body_tail(body, &scope, target),
                None => err("no cond clause matched"),
            },
            // Tail position propagates into whichever branch `if` takes.
            Expr::If {
                condition,
                then_body,
                else_body,
            } => match self.select_if_branch(condition, then_body, else_body, env)? {
                Some((body, scope)) => self.eval_body_tail(body, &scope, target),
                None => Ok(Flow::Return(Value::Nil)),
            },
            // A `run` body is never in tail position (its result has to be
            // matched against the arms first), but the arm it picks is.
            Expr::Run { body, arms } if !arms.is_empty() => {
                let value = self.eval_run_body(body, env)?;
                match self.select_case_arm(&value, arms, env)? {
                    Some((body, scope)) => self.eval_body_tail(body, &scope, target),
                    None => err(format!("no case clause matched: {}", value.inspect())),
                }
            }
            _ => Ok(Flow::Return(self.eval_expr(e, env)?)),
        }
    }

    /// A bare call is a self-recursive tail call when its name is the running
    /// function's, isn't shadowed by a local binding, and resolves in the same
    /// context (same module, or both top-level).
    fn is_self_tail_call(&self, name: &str, env: &Env, target: &TailTarget) -> bool {
        name == target.name
            && env.get(name).is_none()
            && match &target.module {
                Some(m) => self
                    .current_module
                    .as_ref()
                    .is_some_and(|cm| Arc::ptr_eq(cm, m)),
                None => self.current_module.is_none(),
            }
    }

    /// Run a `run` block: statements in order, in a scope of their own, until
    /// one of two things happens. A `pattern = value` whose pattern does not
    /// match *is not an error here* — it ends the block early and the
    /// unmatched value is the result. Otherwise the last statement's value is.
    ///
    /// The scope is a child of `env` so that a block which halts part-way
    /// leaves no half-finished bindings behind for the code after it.
    fn eval_run_body(&mut self, body: &Block, env: &Env) -> Result<Value, RuntimeError> {
        let scope = env.child();
        let mut last = Value::Nil;
        for stmt in body {
            match stmt {
                Stmt::Assign { pattern, value } => {
                    let v = self.eval_expr(value, &scope)?;
                    let Some(binds) = pattern::try_match(pattern, &v)? else {
                        return Ok(v); // the mismatch is the block's result
                    };
                    for (name, bound) in binds {
                        scope.set(name, bound);
                    }
                    last = v;
                }
                other => last = self.eval_stmt(other, &scope)?,
            }
        }
        Ok(last)
    }

    fn eval_case(
        &mut self,
        subject: &Expr,
        arms: &[CaseArm],
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        let subject = self.eval_expr(subject, env)?;
        match self.select_case_arm(&subject, arms, env)? {
            Some((body, scope)) => self.eval_block(body, &scope),
            None => err(format!("no case clause matched: {}", subject.inspect())),
        }
    }

    /// Find the first matching (and guard-passing) case arm, returning its body
    /// and a scope holding its bindings — shared by the plain and tail paths.
    fn select_case_arm<'a>(
        &mut self,
        subject: &Value,
        arms: &'a [CaseArm],
        env: &Env,
    ) -> Result<Option<(&'a Block, Env)>, RuntimeError> {
        for arm in arms {
            let Some(binds) = pattern::try_match(&arm.pattern, subject)? else {
                continue;
            };
            let scope = env.child();
            for (name, value) in binds {
                scope.set(name, value);
            }
            if let Some(guard) = &arm.guard {
                if !self.eval_expr(guard, &scope)?.is_truthy() {
                    continue;
                }
            }
            return Ok(Some((&arm.body, scope)));
        }
        Ok(None)
    }

    fn eval_cond(&mut self, arms: &[CondArm], env: &Env) -> Result<Value, RuntimeError> {
        match self.select_cond_arm(arms, env)? {
            Some((body, scope)) => self.eval_block(body, &scope),
            None => err("no cond clause matched"),
        }
    }

    /// The branch an `if` takes, with a scope of its own so its bindings do not
    /// leak. `None` means the condition was falsy and there is no `else`.
    ///
    /// Any value works as a condition: only `false` and `nil` are falsy, the
    /// same rule `cond` and `and`/`or` use.
    fn select_if_branch<'a>(
        &mut self,
        condition: &Expr,
        then_body: &'a Block,
        else_body: &'a Option<Block>,
        env: &Env,
    ) -> Result<Option<(&'a Block, Env)>, RuntimeError> {
        if self.eval_expr(condition, env)?.is_truthy() {
            return Ok(Some((then_body, env.child())));
        }
        Ok(else_body.as_ref().map(|body| (body, env.child())))
    }

    fn select_cond_arm<'a>(
        &mut self,
        arms: &'a [CondArm],
        env: &Env,
    ) -> Result<Option<(&'a Block, Env)>, RuntimeError> {
        for arm in arms {
            if self.eval_expr(&arm.condition, env)?.is_truthy() {
                return Ok(Some((&arm.body, env.child())));
            }
        }
        Ok(None)
    }

    fn eval_binary(
        &mut self,
        op: BinOp,
        left: &Expr,
        right: &Expr,
        env: &Env,
    ) -> Result<Value, RuntimeError> {
        // `and` / `or` short-circuit on the left operand, returning an operand
        // itself (not a coerced bool), matching the README's word operators.
        if let BinOp::And = op {
            let l = self.eval_expr(left, env)?;
            return if l.is_truthy() {
                self.eval_expr(right, env)
            } else {
                Ok(l)
            };
        }
        if let BinOp::Or = op {
            let l = self.eval_expr(left, env)?;
            return if l.is_truthy() {
                Ok(l)
            } else {
                self.eval_expr(right, env)
            };
        }
        let l = self.eval_expr(left, env)?;
        let r = self.eval_expr(right, env)?;
        apply_binop(op, &l, &r)
    }
}

/// A fresh, parentless scope holding just the values a lambda's free names
/// resolve to right now — not a reference to `env` itself.
///
/// A closure that captured `env` directly, then got bound to a name inside
/// that very `env` (`f = do -> ...`), would leave `env` holding the closure
/// and the closure holding `env` right back: an `Arc` cycle. Nothing here
/// collects cycles, so neither side would ever free — every such binding
/// would leak its whole enclosing scope, and the language's own style rule
/// for a multi-line lambda (bind it to a name first) makes that binding
/// common, not a corner case. Capturing by value breaks the cycle
/// unconditionally, with no need to special-case "assigned to its own
/// scope." The trade-off: a closure's view of what it captured is fixed at
/// the moment it is built, not live — a name rebound in `env` after the
/// lambda exists is a rebinding the lambda does not see.
fn capture(params: &[String], body: &Block, env: &Env) -> Env {
    let captured = Env::root();
    for name in freevars::free_names(params, body) {
        if let Some(value) = env.get(&name) {
            captured.set(name, value);
        }
    }
    captured
}

fn eval_unary(op: UnOp, v: Value) -> Result<Value, RuntimeError> {
    match op {
        UnOp::Not => Ok(Value::Bool(!v.is_truthy())),
        UnOp::Neg => match v {
            Value::Int(n) => n
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| RuntimeError::new("integer overflow negating")),
            Value::Float(x) => Ok(Value::Float(-x)),
            other => err(format!("cannot negate a {}", other.type_name())),
        },
    }
}

fn apply_binop(op: BinOp, l: &Value, r: &Value) -> Result<Value, RuntimeError> {
    use BinOp::*;
    use Value::{Float, Int};
    match op {
        Add => match (l, r) {
            (Int(a), Int(b)) => checked(a.checked_add(*b), "+"),
            _ => float_op(l, r, "+", |x, y| x + y),
        },
        Sub => match (l, r) {
            (Int(a), Int(b)) => checked(a.checked_sub(*b), "-"),
            _ => float_op(l, r, "-", |x, y| x - y),
        },
        Mul => match (l, r) {
            (Int(a), Int(b)) => checked(a.checked_mul(*b), "*"),
            _ => float_op(l, r, "*", |x, y| x * y),
        },
        Div => match (l, r) {
            (Int(_), Int(0)) => err("integer division by zero"),
            (Int(a), Int(b)) => Ok(Int(a.wrapping_div(*b))),
            _ => float_op(l, r, "/", |x, y| x / y),
        },
        Mod => match (l, r) {
            (Int(_), Int(0)) => err("integer modulo by zero"),
            // Floor modulo: the result follows the sign of the divisor.
            (Int(a), Int(b)) => Ok(Int((a.wrapping_rem(*b).wrapping_add(*b)).wrapping_rem(*b))),
            _ => float_op(l, r, "%", |x, y| ((x % y) + y) % y),
        },
        Pow => match (l, r) {
            (Int(a), Int(b)) if *b >= 0 => {
                match u32::try_from(*b).ok().and_then(|e| a.checked_pow(e)) {
                    Some(v) => Ok(Int(v)),
                    None => err("integer overflow in `**`"),
                }
            }
            (Int(a), Int(b)) => Ok(Float((*a as f64).powi(*b as i32))),
            _ => float_op(l, r, "**", f64::powf),
        },
        Concat => match (l, r) {
            (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}").into())),
            _ => err(format!(
                "`<>` expects two strings, got {} and {}",
                l.type_name(),
                r.type_name()
            )),
        },
        Append => match (l, r) {
            (Value::List(a), Value::List(b)) => Ok(Value::List(List::concat(a, b))),
            (Value::Map(a), Value::Map(b)) => Ok(Value::Map(Arc::new(Map::merge(a, b)))),
            _ => err(format!(
                "`++` expects two lists or two maps, got {} and {}",
                l.type_name(),
                r.type_name()
            )),
        },
        Eq => Ok(Value::Bool(values_equal(l, r))),
        NotEq => Ok(Value::Bool(!values_equal(l, r))),
        Lt => order(l, r, |o| o == Ordering::Less),
        Gt => order(l, r, |o| o == Ordering::Greater),
        Le => order(l, r, |o| o != Ordering::Greater),
        Ge => order(l, r, |o| o != Ordering::Less),
        // Short-circuiting operators are handled before operands are evaluated.
        And | Or => unreachable!("`and`/`or` are handled in eval_binary"),
    }
}

fn checked(result: Option<i64>, sym: &str) -> Result<Value, RuntimeError> {
    result
        .map(Value::Int)
        .ok_or_else(|| RuntimeError::new(format!("integer overflow in `{sym}`")))
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

/// Apply a float operation, widening ints — the fallback once an operand is a
/// `Float` (or reporting a type error if either side isn't numeric).
fn float_op(
    l: &Value,
    r: &Value,
    sym: &str,
    f: impl Fn(f64, f64) -> f64,
) -> Result<Value, RuntimeError> {
    match (as_f64(l), as_f64(r)) {
        (Some(x), Some(y)) => Ok(Value::Float(f(x, y))),
        _ => err(format!(
            "`{sym}` expects numbers, got {} and {}",
            l.type_name(),
            r.type_name()
        )),
    }
}

fn order(l: &Value, r: &Value, pick: impl Fn(Ordering) -> bool) -> Result<Value, RuntimeError> {
    match compare(l, r) {
        Some(o) => Ok(Value::Bool(pick(o))),
        None => err(format!(
            "cannot compare {} and {}",
            l.type_name(),
            r.type_name()
        )),
    }
}

fn compare(l: &Value, r: &Value) -> Option<Ordering> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
        _ => match (as_f64(l), as_f64(r)) {
            (Some(x), Some(y)) => x.partial_cmp(&y),
            _ => None,
        },
    }
}
