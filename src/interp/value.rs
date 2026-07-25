//! Runtime values (PLAN phase 3).
//!
//! Everything is immutable: values share structure behind `Rc` and are never
//! mutated in place. "Rebinding" a name is a new entry in a scope, never a
//! write through a value (so no `RefCell` lives inside a `Value`).

use super::env::Env;
use crate::ast::{Block, MapKey, ModuleDef, StructDef};
use std::borrow::Cow;
use std::sync::Arc;

#[derive(Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(Arc<str>),
    Symbol(Arc<str>),
    Bool(bool),
    Nil,
    /// Persistent cons list — `[head | tail]` is O(1), matching the cost model
    /// the stdlib's own recursion is written against.
    List(Arc<List>),
    Tuple(Arc<[Value]>),
    /// Insertion-ordered; small enough that a `Vec` behind `Rc` is plenty.
    Map(Arc<Map>),
    Lambda(Arc<Closure>),
    /// A module is itself a value: it can be bound, passed, and inspected.
    Module(Arc<ModuleDef>),
    /// An instance of a `struct`: its definition plus its field values. Like
    /// every value it is immutable — `Struct.put` returns a new instance.
    Struct(Arc<StructValue>),
    /// A running (or finished) `Thread.start` — a one-shot parallel computation.
    /// The handle is the value you hold and pass around; `Thread.await` waits on
    /// it and hands back what the spawned lambda returned. Shared behind an
    /// `Arc` like every other value, so awaiting reads through a `Mutex`.
    Thread(Arc<ThreadHandle>),
    /// An open TCP connection — from `Socket.connect` or `ServerSocket.accept`.
    /// `send`/`recv`/`close` need to read and eventually tear down the
    /// underlying stream, which the immutable `Value` model does not otherwise
    /// allow — the same exception `Thread` makes, behind a `Mutex` guarding an
    /// `Option` so `close` can drop the stream without dropping the handle.
    Socket(Arc<SocketHandle>),
    /// A bound, listening TCP socket — from `ServerSocket.bind`. Holds the
    /// `TcpListener` the same way `Socket` holds its `TcpStream`.
    ServerSocket(Arc<ServerSocketHandle>),
}

/// The live end of a TCP connection. `None` once `Socket.close` has run.
pub struct SocketHandle {
    pub stream: std::sync::Mutex<Option<std::net::TcpStream>>,
}

/// The live end of a listening socket. `None` once `ServerSocket.close` has run.
pub struct ServerSocketHandle {
    pub listener: std::sync::Mutex<Option<std::net::TcpListener>>,
}

/// The live end of a spawned thread. It owns the join handle and the channel
/// the worker sends its one reply on, behind a `Mutex` so `join` (which may be
/// called from any thread, including another spawned one) is serialized.
pub struct ThreadHandle {
    pub state: std::sync::Mutex<ThreadState>,
}

/// A thread is either still owned by its worker, or finished and reduced to the
/// value it produced. Its buffered output is written exactly once, at the
/// transition, so a second `join` returns the value without printing twice.
pub enum ThreadState {
    Running {
        join: Option<std::thread::JoinHandle<()>>,
        reply: std::sync::mpsc::Receiver<ThreadReply>,
    },
    Finished(Result<Value, String>),
}

/// What a worker sends back: the lambda's result — a value, or the message of
/// the error that stopped it. The thread's output is *not* here: it is written
/// live through the shared sink as it happens, so awaiting only collects the
/// value. (An actor is the opposite — it buffers and delivers its output at the
/// reply, so actor lines never interleave.)
pub struct ThreadReply {
    pub result: Result<Value, String>,
}

/// One struct instance. `fields` holds every attribute the definition declares,
/// in declaration order, so field order is the struct's rather than the
/// literal's.
pub struct StructValue {
    pub def: Arc<StructDef>,
    pub fields: Vec<(String, Value)>,
}

impl StructValue {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A copy with `name` set to `value`. The caller has already checked that
    /// `name` is one of the struct's attributes.
    pub fn with(&self, name: &str, value: Value) -> StructValue {
        let fields = self
            .fields
            .iter()
            .map(|(n, v)| {
                if n == name {
                    (n.clone(), value.clone())
                } else {
                    (n.clone(), v.clone())
                }
            })
            .collect();
        StructValue {
            def: self.def.clone(),
            fields,
        }
    }
}

pub enum List {
    Nil,
    Cons(Value, Arc<List>),
}

pub struct Closure {
    pub params: Vec<String>,
    pub body: Block,
    /// The scope the lambda was written in, captured so its body can still read
    /// the bindings that surrounded it.
    pub env: Env,
    /// The module the lambda was defined in, if any — so bare calls in its body
    /// resolve against that module's functions, exactly as at its definition.
    pub module: Option<Arc<ModuleDef>>,
    /// The file the lambda was written in — a stacktrace frame for a call made
    /// from inside its body names this file, the same way one does for a named
    /// function.
    pub file: Arc<str>,
}

#[derive(Default)]
pub struct Map {
    pub entries: Vec<(Value, Value)>,
}

impl List {
    pub fn nil() -> Arc<List> {
        Arc::new(List::Nil)
    }

    /// Build a list from elements, preserving order.
    pub fn from_vec(items: Vec<Value>) -> Arc<List> {
        let mut node = List::nil();
        for item in items.into_iter().rev() {
            node = Arc::new(List::Cons(item, node));
        }
        node
    }

    /// Walk the list without materializing it. Prefer this to [`to_vec`] when
    /// the elements are only being consumed once.
    ///
    /// [`to_vec`]: List::to_vec
    pub fn iter(self: &Arc<List>) -> ListIter {
        ListIter { cur: self.clone() }
    }

    pub fn to_vec(self: &Arc<List>) -> Vec<Value> {
        let mut out = Vec::new();
        let mut cur = self.clone();
        while let List::Cons(head, tail) = &*cur {
            out.push(head.clone());
            let tail = tail.clone();
            cur = tail;
        }
        out
    }

    /// Number of elements — walks the spine (O(n) time, O(1) space), without
    /// materializing a `Vec`.
    pub fn count(self: &Arc<List>) -> usize {
        let mut n = 0;
        let mut cur = self.clone();
        while let List::Cons(_, tail) = &*cur {
            n += 1;
            let tail = tail.clone();
            cur = tail;
        }
        n
    }

    /// The element at zero-based `index`, or `None` if out of range — walks at
    /// most `index + 1` cells rather than copying the whole list.
    pub fn get(self: &Arc<List>, index: usize) -> Option<Value> {
        let mut cur = self.clone();
        let mut remaining = index;
        while let List::Cons(head, tail) = &*cur {
            if remaining == 0 {
                return Some(head.clone());
            }
            remaining -= 1;
            let tail = tail.clone();
            cur = tail;
        }
        None
    }

    /// `left ++ right` — left's elements followed by all of right.
    pub fn concat(left: &Arc<List>, right: &Arc<List>) -> Arc<List> {
        let mut node = right.clone();
        for item in left.to_vec().into_iter().rev() {
            node = Arc::new(List::Cons(item, node));
        }
        node
    }
}

/// Yields a cons list's elements from head to tail. Each step clones one `Rc`
/// for the tail and one `Value` for the element — no `Vec` is built.
pub struct ListIter {
    cur: Arc<List>,
}

impl Iterator for ListIter {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        let cur = self.cur.clone();
        match &*cur {
            List::Cons(head, tail) => {
                self.cur = tail.clone();
                Some(head.clone())
            }
            List::Nil => None,
        }
    }
}

impl Map {
    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| values_equal(k, key))
            .map(|(_, v)| v)
    }

    /// Look up a symbol-keyed entry by name — how struct/map field patterns and
    /// map literals address their keys.
    pub fn get_symbol(&self, name: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| matches!(k, Value::Symbol(s) if &**s == name))
            .map(|(_, v)| v)
    }

    /// A new map with `key` set to `value`: existing keys keep their position
    /// (value replaced), new keys are appended.
    pub fn put(&self, key: Value, value: Value) -> Map {
        let mut entries = self.entries.clone();
        match entries.iter_mut().find(|(k, _)| values_equal(k, &key)) {
            Some(slot) => slot.1 = value,
            None => entries.push((key, value)),
        }
        Map { entries }
    }

    /// `left ++ right` — merge with right winning on duplicate keys.
    pub fn merge(left: &Map, right: &Map) -> Map {
        let mut out = Map {
            entries: left.entries.clone(),
        };
        for (k, v) in &right.entries {
            out = out.put(k.clone(), v.clone());
        }
        out
    }
}

impl Value {
    /// The name of the module this value belongs to, per the README's Types
    /// table. Used in type-error messages.
    pub fn type_name(&self) -> Cow<'static, str> {
        match self {
            Value::Int(_) => Cow::Borrowed("Integer"),
            Value::Float(_) => Cow::Borrowed("Float"),
            Value::Str(_) => Cow::Borrowed("String"),
            Value::Symbol(_) => Cow::Borrowed("Symbol"),
            Value::Bool(_) => Cow::Borrowed("Bool"),
            Value::Nil => Cow::Borrowed("Nil"),
            Value::List(_) => Cow::Borrowed("List"),
            Value::Tuple(_) => Cow::Borrowed("Tuple"),
            Value::Map(_) => Cow::Borrowed("Map"),
            Value::Lambda(_) => Cow::Borrowed("Lambda"),
            Value::Module(_) => Cow::Borrowed("Module"),
            // A struct's type is its own name — `Person`, not `Struct` — which
            // is what the Types table means by a struct dispatching to itself.
            Value::Struct(s) => Cow::Owned(s.def.name.to_string()),
            Value::Thread(_) => Cow::Borrowed("Thread"),
            Value::Socket(_) => Cow::Borrowed("Socket"),
            Value::ServerSocket(_) => Cow::Borrowed("ServerSocket"),
        }
    }

    /// Only `false` and `nil` are falsy — everything else (0, "", []) is truthy.
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Bool(false) | Value::Nil)
    }

    /// Human-facing text (what `print`, `to_string`, and interpolation emit):
    /// only strings render bare — a symbol keeps its colon, so `to_string(:ok)`
    /// is `":ok"`, per the Kernel stdlib docs.
    pub fn display(&self) -> String {
        match self {
            Value::Str(s) => s.to_string(),
            _ => self.inspect(),
        }
    }

    /// Debug-facing text: strings are quoted and symbols keep their colon, so a
    /// collection's elements stay unambiguous.
    pub fn inspect(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(x) => format_float(*x),
            Value::Str(s) => format!("{s:?}"),
            Value::Symbol(s) => format!(":{s}"),
            Value::Bool(b) => b.to_string(),
            Value::Nil => "nil".to_string(),
            Value::List(list) => {
                let parts: Vec<String> = list.to_vec().iter().map(Value::inspect).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(Value::inspect).collect();
                format!("({})", parts.join(", "))
            }
            Value::Map(map) => {
                let parts: Vec<String> = map
                    .entries
                    .iter()
                    // Keys that have a literal form render as one, so the
                    // output reads back as source. Anything else (a bool key
                    // from `List.group_by`, say) falls back to `k => v`.
                    .map(|(k, v)| match k {
                        Value::Symbol(name) => format!("{name}: {}", v.inspect()),
                        Value::Str(_) | Value::Int(_) => {
                            format!("{}: {}", k.inspect(), v.inspect())
                        }
                        other => format!("{} => {}", other.inspect(), v.inspect()),
                    })
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            Value::Lambda(c) => format!("#lambda/{}", c.params.len()),
            Value::Module(m) => m.name.to_string(),
            Value::Struct(s) => {
                let parts: Vec<String> = s
                    .fields
                    .iter()
                    .map(|(n, v)| format!("{n}: {}", v.inspect()))
                    .collect();
                format!("{}{{{}}}", s.def.name, parts.join(", "))
            }
            // A thread has no structural identity worth showing — like a lambda.
            Value::Thread(_) => "#thread".to_string(),
            Value::Socket(_) => "#socket".to_string(),
            Value::ServerSocket(_) => "#server_socket".to_string(),
        }
    }
}

/// The runtime value a key written in a map literal or pattern denotes. Shared
/// by construction and matching, so `{"host": 1}` is found by `{"host": h}`.
pub fn map_key_value(key: &MapKey) -> Value {
    match key {
        MapKey::Symbol(name) => Value::Symbol(name.as_str().into()),
        MapKey::Str(s) => Value::Str(s.as_str().into()),
        MapKey::Int(n) => Value::Int(*n),
    }
}

/// Structural equality (`==`). An `Integer` and a `Float` never compare equal
/// (`1 == 1.0` is false); maps compare irrespective of insertion order.
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Symbol(x), Value::Symbol(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Nil, Value::Nil) => true,
        (Value::List(x), Value::List(y)) => {
            let (xs, ys) = (x.to_vec(), y.to_vec());
            xs.len() == ys.len() && xs.iter().zip(&ys).all(|(a, b)| values_equal(a, b))
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b))
        }
        (Value::Map(x), Value::Map(y)) => {
            x.entries.len() == y.entries.len()
                && x.entries
                    .iter()
                    .all(|(k, v)| y.get(k).is_some_and(|yv| values_equal(v, yv)))
        }
        // A module is identified by its name — there is only ever one of each.
        (Value::Module(x), Value::Module(y)) => x.name == y.name,
        // Two instances are equal when they are the same struct with equal
        // fields, so equality stays structural all the way down.
        (Value::Struct(x), Value::Struct(y)) => {
            x.def.name == y.def.name
                && x.fields.len() == y.fields.len()
                && x.fields
                    .iter()
                    .zip(y.fields.iter())
                    .all(|((xn, xv), (yn, yv))| xn == yn && values_equal(xv, yv))
        }
        // Lambdas, threads and sockets have no meaningful structural identity.
        _ => false,
    }
}

/// Floats always show a decimal point, so `2.0` never renders as `2` (which
/// would read as an `Integer`). `pub(crate)` so `natives.rs` can reuse it for
/// `json_format` — a JSON number is written the same way, once NaN/Infinity
/// (not valid JSON) are ruled out by the caller.
pub(crate) fn format_float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

// Actors run their handlers on their own threads, so every value that can be a
// message, a reply, or actor state has to cross a thread boundary. These fail
// to compile if an `Rc` or a `RefCell` creeps back into the value graph.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Value>();
    assert_send_sync::<super::env::Env>();
};
