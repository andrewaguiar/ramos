//! The native primitives behind `Kernel` (PLAN phase 4's `native(str, args)`
//! table). These implement everything the Ramos stdlib delegates to the host:
//! console I/O, collection ops, conversions, type predicates, and the
//! character-level `string_*` operations used by the `String` module.
//!
//! `native(name, args)` is the seam: the stdlib calls `native("string_upcase",
//! [s])`, and [`call`] dispatches by name. Because `Kernel` is implicitly in
//! scope, the plain Kernel functions (`print`, `size`, `is_integer`, …) also
//! resolve here as bare calls, even before the Ramos stdlib is loaded.

use super::eval::{RuntimeError, Sink};
use super::value::{values_equal, List, Map, StructValue, Value};
use std::fs;
use std::io::Write;
use std::sync::Arc;

/// The program's command-line arguments (everything after the script path),
/// passed to `get_args` / `get_arg`.
type Argv<'a> = &'a [String];

/// Dispatch a bare Kernel call. Returns `None` when `name` is not a native, so
/// the caller can fall through to "undefined function".
pub fn call(
    out: &Sink,
    errs: &Sink,
    argv: Argv,
    name: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    if name == "native" {
        return Some(native_seam(out, errs, argv, args));
    }
    dispatch(out, errs, argv, name, args)
}

/// `native(name, args)` — unwrap the `[name, arglist]` shape the stdlib passes
/// and re-dispatch by name.
fn native_seam(out: &Sink, errs: &Sink, argv: Argv, args: &[Value]) -> Result<Value, RuntimeError> {
    let [name, arglist] = args else {
        return arity("native", 2, args.len());
    };
    let name = as_str(name)?;
    let inner = as_list(arglist)?.to_vec();
    match dispatch(out, errs, argv, name, &inner) {
        Some(result) => result,
        None => err(format!("unknown native `{name}`")),
    }
}

fn dispatch(
    out: &Sink,
    errs: &Sink,
    argv: Argv,
    name: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let result = match name {
        // ── console I/O ──────────────────────────────────────────────────
        "print" => write_out(out, args, false),
        "println" => write_out(out, args, true),
        "new_line" => arity_then(name, args, 0, |_| emit(out, "\n")),
        "eprint" => write_errs(errs, args, false),
        "eprintln" => write_errs(errs, args, true),
        "read" => arity_then(name, args, 0, |_| read_line(out)),
        // No portable no-echo read without a dependency; fall back to `read`.
        "read_password" => arity_then(name, args, 0, |_| read_line(out)),
        "read_all" => arity_then(name, args, 0, |_| read_all(out)),
        "get_env" => one(name, args, |v| {
            let key = as_str(v)?;
            Ok(std::env::var(key)
                .map(|s| Value::Str(s.into()))
                .unwrap_or(Value::Nil))
        }),

        // ── process / command line ───────────────────────────────────────
        "get_args" => arity_then(name, args, 0, |_| {
            let items = argv.iter().map(|s| Value::Str(s.as_str().into())).collect();
            Ok(Value::List(List::from_vec(items)))
        }),
        "get_arg" => one(name, args, |v| {
            let i = as_int(v)?;
            Ok(match usize::try_from(i) {
                Ok(i) if i < argv.len() => Value::Str(argv[i].as_str().into()),
                _ => Value::Nil, // out of range or negative
            })
        }),
        "sleep" => one(name, args, |v| {
            let ms = as_int(v)?;
            if ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            }
            Ok(Value::Nil)
        }),
        "exit" => one(name, args, |v| {
            // Unwinds rather than calling `process::exit`, which would kill a
            // test runner and drop whatever output is still buffered.
            Err(RuntimeError::exit(as_int(v)?))
        }),

        // ── time ─────────────────────────────────────────────────────────
        "now" => arity_then(name, args, 0, |_| Ok(Value::Int(unix_millis()))),
        "monotonic" => arity_then(name, args, 0, |_| Ok(Value::Int(monotonic_millis()))),

        // ── calendar (backs the `Date` struct) ──────────────────────────
        // The proleptic Gregorian <-> epoch-day conversion is fiddly integer
        // math better done once here than reimplemented per caller; `Date`
        // itself is otherwise pure Ramos built on these two.
        "date_to_epoch_day" => three(name, args, date_to_epoch_day),
        "date_from_epoch_day" => one(name, args, date_from_epoch_day),

        // ── randomness ───────────────────────────────────────────────────
        "random" => arity_then(name, args, 0, |_| {
            // 53 bits is exactly what an f64 mantissa holds, so every value in
            // [0, 1) is reachable and none is favoured by rounding.
            let bits = next_random() >> 11;
            Ok(Value::Float(bits as f64 / (1u64 << 53) as f64))
        }),
        "random_int" => two(name, args, random_int),

        // ── collections ──────────────────────────────────────────────────
        "size" => one(name, args, size),
        "at" => two(name, args, at),
        "to_list" => one(name, args, to_list),
        "tuple_from_list" => one(name, args, tuple_from_list),
        // The map basics are primitives. The dividing line is the lambda: a
        // native cannot call back into the interpreter, so everything taking a
        // predicate or transform (`filter`, `reject`, `update`, the `map_*`
        // mappers) is written in Ramos on top of these.
        "map_put" => three(name, args, map_put),
        "map_get" => three(name, args, map_get),
        "map_delete" => two(name, args, map_delete),
        "map_has_key" => two(name, args, map_has_key),
        "map_keys" => one(name, args, map_keys),
        "map_entries" => one(name, args, map_entries),
        "map_from_list" => one(name, args, map_from_list),

        // ── conversions ──────────────────────────────────────────────────
        "to_string" => one(name, args, |v| Ok(Value::Str(v.display().into()))),
        // The debug rendering the REPL and error messages use: strings quoted,
        // symbols keeping their colon.
        "inspect" => one(name, args, |v| Ok(Value::Str(v.inspect().into()))),
        // The module a value belongs to — a struct instance answers with its
        // own name. A `String`, not a `Symbol`: module names are CamelCase and
        // symbol names are snake_case, so `:Integer` is not writable.
        "type_of" => one(name, args, |v| Ok(Value::Str(v.type_name().into()))),
        "to_integer" => one(name, args, to_integer),
        "to_float" => one(name, args, to_float),
        // No `to_symbol`: a symbol lives as long as the program, so building
        // one from runtime data is an unbounded leak. Symbols are literals.
        //
        // No `current_stacktrace` here: it needs `Interp::call_stack`, which
        // this free function has no access to — `call_local` answers it
        // directly instead, the same way it does `apply` and `start_actor`.

        // ── type predicates ──────────────────────────────────────────────
        "is_integer" => pred(name, args, |v| matches!(v, Value::Int(_))),
        "is_float" => pred(name, args, |v| matches!(v, Value::Float(_))),
        "is_string" => pred(name, args, |v| matches!(v, Value::Str(_))),
        "is_symbol" => pred(name, args, |v| matches!(v, Value::Symbol(_))),
        "is_bool" => pred(name, args, |v| matches!(v, Value::Bool(_))),
        "is_nil" => pred(name, args, |v| matches!(v, Value::Nil)),
        "is_list" => pred(name, args, |v| matches!(v, Value::List(_))),
        "is_tuple" => pred(name, args, |v| matches!(v, Value::Tuple(_))),
        "is_map" => pred(name, args, |v| matches!(v, Value::Map(_))),
        "is_lambda" => pred(name, args, |v| matches!(v, Value::Lambda(_))),
        "is_module" => pred(name, args, |v| matches!(v, Value::Module(_))),
        "is_struct" => pred(name, args, |v| matches!(v, Value::Struct(_))),

        // ── module primitives ────────────────────────────────────────────
        // A module's function list lives on the `Value` itself (a `ModuleDef`
        // it already holds), so listing them needs no interpreter access —
        // unlike `module_apply` (in `eval.rs`), which has to run one.
        "module_functions" => one(name, args, module_functions),

        // ── float primitives ─────────────────────────────────────────────
        // Everything else in `Float` is built on `**`/comparisons; these
        // transcendental functions have no operator to fall back on, so they
        // are the module's only natives.
        "float_log" => one(name, args, |v| Ok(Value::Float(as_float(v)?.ln()))),
        "float_log_two" => one(name, args, |v| Ok(Value::Float(as_float(v)?.log2()))),
        "float_log_ten" => one(name, args, |v| Ok(Value::Float(as_float(v)?.log10()))),
        "float_exp" => one(name, args, |v| Ok(Value::Float(as_float(v)?.exp()))),
        "float_sin" => one(name, args, |v| Ok(Value::Float(as_float(v)?.sin()))),
        "float_cos" => one(name, args, |v| Ok(Value::Float(as_float(v)?.cos()))),
        "float_tan" => one(name, args, |v| Ok(Value::Float(as_float(v)?.tan()))),

        // ── struct primitives ────────────────────────────────────────────
        // Attributes are fixed at definition time, so these never add or drop
        // a field: `struct_put` on an undeclared name is an error, not a new
        // attribute. That is what keeps an instance's shape its struct's.
        "struct_get" => three(name, args, struct_get),
        "struct_put" => three(name, args, struct_put),
        "struct_keys" => one(name, args, struct_keys),
        "struct_values" => one(name, args, struct_values),
        "struct_to_map" => one(name, args, struct_to_map),
        "struct_is_a" => two(name, args, struct_is_a),

        // ── string primitives ────────────────────────────────────────────
        "string_length" => one(name, args, |v| {
            Ok(Value::Int(as_str(v)?.chars().count() as i64))
        }),
        "string_at" => two(name, args, string_at),
        "string_slice" => three(name, args, string_slice),
        "string_contains" => two(name, args, |s, sub| {
            Ok(Value::Bool(as_str(s)?.contains(as_str(sub)?)))
        }),
        "string_starts_with" => two(name, args, |s, p| {
            Ok(Value::Bool(as_str(s)?.starts_with(as_str(p)?)))
        }),
        "string_ends_with" => two(name, args, |s, p| {
            Ok(Value::Bool(as_str(s)?.ends_with(as_str(p)?)))
        }),
        "string_find" => two(name, args, string_find),
        "string_replace" => three(name, args, string_replace),
        "string_split" => two(name, args, string_split),
        "string_upcase" => one(name, args, |v| {
            Ok(Value::Str(as_str(v)?.to_ascii_uppercase().into()))
        }),
        "string_downcase" => one(name, args, |v| {
            Ok(Value::Str(as_str(v)?.to_ascii_lowercase().into()))
        }),
        "string_trim" => one(name, args, |v| Ok(Value::Str(as_str(v)?.trim().into()))),
        "string_trim_left" => one(name, args, |v| {
            Ok(Value::Str(as_str(v)?.trim_start().into()))
        }),
        "string_trim_right" => one(name, args, |v| Ok(Value::Str(as_str(v)?.trim_end().into()))),
        "string_reverse" => one(name, args, |v| {
            Ok(Value::Str(
                as_str(v)?.chars().rev().collect::<String>().into(),
            ))
        }),
        "string_repeat" => two(name, args, string_repeat),
        "string_to_list" => one(name, args, string_to_list),
        "string_from_list" => one(name, args, string_from_list),

        // ── file I/O ─────────────────────────────────────────────────────
        // Fallible ops return `(:ok, value)` / `(:error, reason)` (or the bare
        // `:ok` symbol) rather than raising, so callers pattern-match. Every
        // primitive works on paths — there are no open handles.
        "file_read" => one(name, args, file_read),
        "file_write" => two(name, args, |p, c| file_write_all(p, c, false)),
        "file_append" => two(name, args, |p, c| file_write_all(p, c, true)),
        "file_size" => one(name, args, file_size),
        "file_remove" => one(name, args, file_remove),
        "file_copy" => two(name, args, file_copy),
        "file_move" => two(name, args, file_move),
        "file_rename" => two(name, args, file_rename),
        "file_touch" => one(name, args, file_touch),
        "file_chmod" => two(name, args, file_chmod),

        // ── paths & directories ──────────────────────────────────────────
        // `path_exists` / `path_is_dir` are shared by `File` and `Dir`.
        "path_exists" => one(name, args, |v| {
            Ok(Value::Bool(std::path::Path::new(as_str(v)?).exists()))
        }),
        "path_is_file" => one(name, args, |v| {
            Ok(Value::Bool(std::path::Path::new(as_str(v)?).is_file()))
        }),
        "path_is_dir" => one(name, args, |v| {
            Ok(Value::Bool(std::path::Path::new(as_str(v)?).is_dir()))
        }),
        "dir_current" => arity_then(name, args, 0, |_| dir_current()),
        "dir_change" => one(name, args, dir_change),
        "dir_make" => one(name, args, dir_make),
        "dir_list" => one(name, args, dir_list),

        _ => return None,
    };
    Some(result)
}

// ── collection primitives ────────────────────────────────────────────────────

fn size(v: &Value) -> Result<Value, RuntimeError> {
    let n = match v {
        Value::List(l) => l.count(), // walk the spine, no Vec
        Value::Tuple(t) => t.len(),  // O(1): tuples are arrays
        Value::Map(m) => m.entries.len(),
        Value::Str(_) => return err("size on a String is not allowed — use String.length"),
        other => {
            return err(format!(
                "size expects a List, Tuple, or Map, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Value::Int(n as i64))
}

fn at(collection: &Value, index: &Value) -> Result<Value, RuntimeError> {
    // A negative index never matches; `usize::try_from` rejects it.
    let index = usize::try_from(as_int(index)?).ok();
    match collection {
        // Tuples are arrays: O(1) direct index. Lists walk `index` cells.
        Value::Tuple(t) => Ok(index.and_then(|i| t.get(i).cloned()).unwrap_or(Value::Nil)),
        Value::List(l) => Ok(index.and_then(|i| l.get(i)).unwrap_or(Value::Nil)),
        Value::Str(_) => err("at on a String is not allowed — use String.at"),
        other => err(format!(
            "at expects a List or Tuple, got {}",
            other.type_name()
        )),
    }
}

fn to_list(v: &Value) -> Result<Value, RuntimeError> {
    let items = match v {
        Value::List(l) => return Ok(Value::List(l.clone())),
        Value::Tuple(t) => t.to_vec(),
        Value::Map(m) => m.entries.iter().map(|(_, val)| val.clone()).collect(),
        other => {
            return err(format!(
                "to_list expects a Tuple, Map, or List, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Value::List(List::from_vec(items)))
}

/// The names of the public (non-`helper`) functions a module defines, in
/// declaration order. Backs `Module.functions`.
fn module_functions(v: &Value) -> Result<Value, RuntimeError> {
    match v {
        Value::Module(m) => {
            let names = m
                .functions
                .iter()
                .filter(|f| !f.private)
                .map(|f| Value::Str(f.name.as_str().into()))
                .collect();
            Ok(Value::List(List::from_vec(names)))
        }
        other => err(format!("expected a Module, got {}", other.type_name())),
    }
}

fn as_float(v: &Value) -> Result<f64, RuntimeError> {
    match v {
        Value::Float(x) => Ok(*x),
        other => err(format!("expected a Float, got {}", other.type_name())),
    }
}

fn as_map(v: &Value) -> Result<&Arc<Map>, RuntimeError> {
    match v {
        Value::Map(m) => Ok(m),
        other => err(format!("expected a Map, got {}", other.type_name())),
    }
}

/// The one gate on what may be a map key: an `Integer`, `String` or `Symbol` —
/// the three types a map literal can write. Holding the runtime to the same set
/// keeps every map expressible as source, and keeps key equality to types with
/// an obvious identity.
fn check_key(key: &Value) -> Result<(), RuntimeError> {
    if matches!(key, Value::Int(_) | Value::Str(_) | Value::Symbol(_)) {
        return Ok(());
    }
    err(format!(
        "a Map key must be an Integer, String or Symbol, got {}",
        key.type_name()
    ))
}

/// A new map with `key` set to `value`. An existing key keeps its position.
/// The instance behind a struct value, or a typed error.
fn as_struct(value: &Value) -> Result<&Arc<StructValue>, RuntimeError> {
    match value {
        Value::Struct(s) => Ok(s),
        other => err(format!("expected a struct, got {}", other.type_name())),
    }
}

/// An attribute name written as a symbol (`:age`), matching how `Map` keys and
/// struct fields are written in source.
fn as_field(key: &Value) -> Result<&str, RuntimeError> {
    match key {
        Value::Symbol(name) => Ok(name),
        other => err(format!(
            "a struct attribute is a symbol like `:name`, got {}",
            other.type_name()
        )),
    }
}

fn struct_get(value: &Value, key: &Value, default: &Value) -> Result<Value, RuntimeError> {
    let s = as_struct(value)?;
    Ok(s.get(as_field(key)?)
        .cloned()
        .unwrap_or_else(|| default.clone()))
}

fn struct_put(value: &Value, key: &Value, new: &Value) -> Result<Value, RuntimeError> {
    let s = as_struct(value)?;
    let field = as_field(key)?;
    if s.get(field).is_none() {
        return err(format!("`{}` has no attribute `{field}`", s.def.name));
    }
    Ok(Value::Struct(Arc::new(s.with(field, new.clone()))))
}

fn struct_keys(value: &Value) -> Result<Value, RuntimeError> {
    let s = as_struct(value)?;
    let keys: Vec<Value> = s
        .fields
        .iter()
        .map(|(n, _)| Value::Symbol(n.as_str().into()))
        .collect();
    Ok(Value::List(List::from_vec(keys)))
}

fn struct_values(value: &Value) -> Result<Value, RuntimeError> {
    let s = as_struct(value)?;
    let values: Vec<Value> = s.fields.iter().map(|(_, v)| v.clone()).collect();
    Ok(Value::List(List::from_vec(values)))
}

fn struct_to_map(value: &Value) -> Result<Value, RuntimeError> {
    let s = as_struct(value)?;
    let entries = s
        .fields
        .iter()
        .map(|(name, v)| (Value::Symbol(name.as_str().into()), v.clone()))
        .collect();
    Ok(Value::Map(Arc::new(Map { entries })))
}

/// Whether `value` is an instance of the struct named by `module`.
fn struct_is_a(value: &Value, module: &Value) -> Result<Value, RuntimeError> {
    let wanted = match module {
        Value::Module(m) => m.name.to_string(),
        Value::Str(s) => s.to_string(),
        other => return err(format!("expected a struct name, got {}", other.type_name())),
    };
    Ok(Value::Bool(
        matches!(value, Value::Struct(s) if s.def.name.to_string() == wanted),
    ))
}

fn map_put(map: &Value, key: &Value, value: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    check_key(key)?;
    Ok(Value::Map(Arc::new(map.put(key.clone(), value.clone()))))
}

/// The value under `key`, or `default` when the map has no such key.
fn map_get(map: &Value, key: &Value, default: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    Ok(map.get(key).cloned().unwrap_or_else(|| default.clone()))
}

/// A new map without `key`. Removing an absent key is not an error.
fn map_delete(map: &Value, key: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    let entries = map
        .entries
        .iter()
        .filter(|(k, _)| !values_equal(k, key))
        .cloned()
        .collect();
    Ok(Value::Map(Arc::new(Map { entries })))
}

fn map_has_key(map: &Value, key: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    Ok(Value::Bool(map.get(key).is_some()))
}

/// The map's keys, in insertion order.
fn map_keys(map: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    let keys = map.entries.iter().map(|(k, _)| k.clone()).collect();
    Ok(Value::List(List::from_vec(keys)))
}

/// The map's `(key, value)` pairs, in insertion order — what the Ramos-side
/// `filter`, `reject` and the `map_*` mappers walk.
fn map_entries(map: &Value) -> Result<Value, RuntimeError> {
    let map = as_map(map)?;
    let pairs = map
        .entries
        .iter()
        .map(|(k, v)| Value::Tuple([k.clone(), v.clone()].into()))
        .collect();
    Ok(Value::List(List::from_vec(pairs)))
}

/// A map built from a list of `(key, value)` tuples, in order — the inverse of
/// `map_entries`. A repeated key keeps its first position and its last value.
fn map_from_list(pairs: &Value) -> Result<Value, RuntimeError> {
    // Fill one `entries` vector in place. Going through `Map::put` per pair
    // would clone the whole vector each time, making this quadratic in
    // *allocations* rather than just in key comparisons.
    let mut entries: Vec<(Value, Value)> = Vec::new();
    for pair in as_list(pairs)?.iter() {
        let Value::Tuple(items) = &pair else {
            return err(format!(
                "from_list expects (key, value) tuples, got {}",
                pair.type_name()
            ));
        };
        let [key, value] = items.as_ref() else {
            return err(format!(
                "from_list expects (key, value) tuples, got one of {} element(s)",
                items.len()
            ));
        };
        check_key(key)?;
        match entries.iter_mut().find(|(k, _)| values_equal(k, key)) {
            Some(slot) => slot.1 = value.clone(),
            None => entries.push((key.clone(), value.clone())),
        }
    }
    Ok(Value::Map(Arc::new(Map { entries })))
}

/// A tuple of the list's elements, in order. Tuples are fixed-arity, so this is
/// the only way to build one whose width is not written in the source.
fn tuple_from_list(list: &Value) -> Result<Value, RuntimeError> {
    let items = as_list(list)?.to_vec();
    Ok(Value::Tuple(items.into()))
}

// ── conversions ──────────────────────────────────────────────────────────────

fn to_integer(v: &Value) -> Result<Value, RuntimeError> {
    Ok(match v {
        Value::Int(n) => Value::Int(*n),
        Value::Float(x) => Value::Int(*x as i64), // truncates toward zero
        Value::Str(s) => s
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or(Value::Nil),
        _ => Value::Nil,
    })
}

fn to_float(v: &Value) -> Result<Value, RuntimeError> {
    Ok(match v {
        Value::Int(n) => Value::Float(*n as f64),
        Value::Float(x) => Value::Float(*x),
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or(Value::Nil),
        _ => Value::Nil,
    })
}

// ── string primitives (indexes count Unicode code points) ────────────────────

fn string_at(s: &Value, index: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let i = as_int(index)?;
    // Walk to the index rather than materializing a `Vec<char>`: the cost is
    // the same O(i) scan either way, without allocating the whole string again.
    Ok(match usize::try_from(i) {
        Ok(i) => match s.chars().nth(i) {
            Some(c) => Value::Str(c.to_string().into()),
            None => Value::Nil,
        },
        Err(_) => Value::Nil, // negative index
    })
}

fn string_slice(s: &Value, start: &Value, count: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let start = as_int(start)?.max(0) as usize;
    let count = as_int(count)?.max(0) as usize;
    // One pass, and only the slice is allocated — not a char per input byte.
    let slice: String = s.chars().skip(start).take(count).collect();
    Ok(Value::Str(slice.into()))
}

fn string_find(s: &Value, sub: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let sub = as_str(sub)?;
    if sub.is_empty() {
        return Ok(Value::Int(0));
    }
    Ok(match s.find(sub) {
        // Byte offset -> code-point index.
        Some(byte) => Value::Int(s[..byte].chars().count() as i64),
        None => Value::Nil,
    })
}

fn string_replace(s: &Value, pattern: &Value, replacement: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let pattern = as_str(pattern)?;
    let replacement = as_str(replacement)?;
    // An empty pattern would match between every char; leave the string as-is.
    let out = if pattern.is_empty() {
        s.to_string()
    } else {
        s.replace(pattern, replacement)
    };
    Ok(Value::Str(out.into()))
}

fn string_split(s: &Value, sep: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let sep = as_str(sep)?;
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars()
            .map(|c| Value::Str(c.to_string().into()))
            .collect()
    } else {
        s.split(sep).map(|p| Value::Str(p.into())).collect()
    };
    Ok(Value::List(List::from_vec(parts)))
}

fn string_repeat(s: &Value, n: &Value) -> Result<Value, RuntimeError> {
    let s = as_str(s)?;
    let n = as_int(n)?;
    let out = if n <= 0 {
        String::new()
    } else {
        s.repeat(n as usize)
    };
    Ok(Value::Str(out.into()))
}

fn string_to_list(v: &Value) -> Result<Value, RuntimeError> {
    let chars = as_str(v)?
        .chars()
        .map(|c| Value::Str(c.to_string().into()))
        .collect();
    Ok(Value::List(List::from_vec(chars)))
}

fn string_from_list(v: &Value) -> Result<Value, RuntimeError> {
    let mut out = String::new();
    for piece in as_list(v)?.iter() {
        out.push_str(as_str(&piece)?);
    }
    Ok(Value::Str(out.into()))
}

// ── file I/O ─────────────────────────────────────────────────────────────────
//
// The seam is deliberately effect-explicit: every primitive that can fail
// returns a value the caller matches on — `(:ok, x)`, `(:error, reason)`, or a
// bare `:ok` / `:eof` — instead of raising. A `RuntimeError` is reserved for
// misuse (wrong argument *type*), matching the rest of this module.

/// Read a whole file to a string: `(:ok, contents)` or `(:error, reason)`.
fn file_read(path: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::read_to_string(as_str(path)?) {
        Ok(text) => ok(Value::Str(text.into())),
        Err(e) => io_error(&e),
    })
}

/// Write (truncating) or append `contents` to `path`. Returns `:ok` or
/// `(:error, reason)`. The file is created if missing.
fn file_write_all(path: &Value, contents: &Value, append: bool) -> Result<Value, RuntimeError> {
    let path = as_str(path)?;
    let contents = as_str(contents)?;
    let result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .and_then(|mut f| f.write_all(contents.as_bytes()));
    Ok(match result {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// The size of `path` in bytes: `(:ok, bytes)` or `(:error, reason)`.
fn file_size(path: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::metadata(as_str(path)?) {
        Ok(meta) => ok(Value::Int(meta.len() as i64)),
        Err(e) => io_error(&e),
    })
}

/// Delete a file: `:ok` or `(:error, reason)`.
fn file_remove(path: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::remove_file(as_str(path)?) {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Copy `src` to `dest` (overwriting `dest`): `:ok` or `(:error, reason)`.
fn file_copy(src: &Value, dest: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::copy(as_str(src)?, as_str(dest)?) {
        Ok(_) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Rename `src` to `dest` with `fs::rename` — atomic within a filesystem, but it
/// fails across devices (see `mv`). `:ok` or `(:error, reason)`.
fn file_rename(src: &Value, dest: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::rename(as_str(src)?, as_str(dest)?) {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Move `src` to `dest`. Tries a rename first, then falls back to copy + delete
/// so it works across filesystems where `rename` cannot. `:ok`/`(:error, ...)`.
fn file_move(src: &Value, dest: &Value) -> Result<Value, RuntimeError> {
    let src = as_str(src)?;
    let dest = as_str(dest)?;
    if fs::rename(src, dest).is_ok() {
        return Ok(sym("ok"));
    }
    let result = fs::copy(src, dest).and_then(|_| fs::remove_file(src));
    Ok(match result {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Create `path` if it does not exist, and bump its modified time to now (like
/// `touch(1)`). Existing content is preserved. `:ok` or `(:error, reason)`.
fn file_touch(path: &Value) -> Result<Value, RuntimeError> {
    let result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false) // touch must preserve existing content
        .open(as_str(path)?)
        .and_then(|f| f.set_modified(std::time::SystemTime::now()));
    Ok(match result {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Set the permission bits of `path`. `mode` is the numeric permission (e.g.
/// `493` for `rwxr-xr-x`). `:ok` or `(:error, reason)`; `(:error, :unsupported)`
/// on non-Unix platforms.
fn file_chmod(path: &Value, mode: &Value) -> Result<Value, RuntimeError> {
    let path = as_str(path)?;
    let mode = as_int(mode)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode as u32);
        Ok(match fs::set_permissions(path, perms) {
            Ok(()) => sym("ok"),
            Err(e) => io_error(&e),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(error(sym("unsupported")))
    }
}

/// The process's current working directory: `(:ok, path)` or `(:error, reason)`.
fn dir_current() -> Result<Value, RuntimeError> {
    Ok(match std::env::current_dir() {
        Ok(path) => ok(Value::Str(path.to_string_lossy().into())),
        Err(e) => io_error(&e),
    })
}

/// Change the process's current working directory: `:ok` or `(:error, reason)`.
fn dir_change(path: &Value) -> Result<Value, RuntimeError> {
    Ok(match std::env::set_current_dir(as_str(path)?) {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// Create a directory, including any missing parents (like `mkdir -p`). Succeeds
/// if it already exists. `:ok` or `(:error, reason)`.
fn dir_make(path: &Value) -> Result<Value, RuntimeError> {
    Ok(match fs::create_dir_all(as_str(path)?) {
        Ok(()) => sym("ok"),
        Err(e) => io_error(&e),
    })
}

/// List a directory's entries by name (not full paths), sorted:
/// `(:ok, names)` or `(:error, reason)`.
fn dir_list(path: &Value) -> Result<Value, RuntimeError> {
    let entries = match fs::read_dir(as_str(path)?) {
        Ok(entries) => entries,
        Err(e) => return Ok(io_error(&e)),
    };
    let mut names = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(e) => return Ok(io_error(&e)),
        }
    }
    names.sort();
    let items = names.into_iter().map(|n| Value::Str(n.into())).collect();
    Ok(ok(Value::List(List::from_vec(items))))
}

// Result-shape helpers shared by the file natives.

fn sym(name: &str) -> Value {
    Value::Symbol(name.into())
}

fn ok(value: Value) -> Value {
    Value::Tuple(vec![sym("ok"), value].into())
}

fn error(reason: Value) -> Value {
    Value::Tuple(vec![sym("error"), reason].into())
}

/// `(:error, reason)` with `reason` a POSIX-ish symbol derived from the error.
fn io_error(e: &std::io::Error) -> Value {
    use std::io::ErrorKind;
    let reason = match e.kind() {
        ErrorKind::NotFound => "enoent",
        ErrorKind::PermissionDenied => "eacces",
        ErrorKind::AlreadyExists => "eexist",
        _ => "io_error",
    };
    error(sym(reason))
}

// ── console output ───────────────────────────────────────────────────────────

/// Lock a sink and run `f` with its writer. The lock is held only for `f` — the
/// actual write — never across a blocking native like `sleep` or `read`, so
/// threads sharing the sink still run in parallel. A poisoned mutex (a panicked
/// thread) is recovered rather than propagated: output should not stop.
fn with_locked<T>(
    sink: &Sink,
    f: impl FnOnce(&mut dyn Write) -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut guard = sink.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut *guard)
}

fn write_out(out: &Sink, args: &[Value], newline: bool) -> Result<Value, RuntimeError> {
    let mut text: String = args.iter().map(Value::display).collect();
    if newline {
        text.push('\n');
    }
    emit(out, &text)
}

/// Write without flushing. `println` builds its text and newline into one call,
/// so the whole line is written under one lock — a spawned thread's line never
/// lands inside another's. Output is flushed where it is observable: line-by-
/// line by the terminal, before reading input, and once the program ends.
fn emit(out: &Sink, text: &str) -> Result<Value, RuntimeError> {
    with_locked(out, |w| w.write_all(text.as_bytes()))
        .map_err(|e| RuntimeError::new(format!("could not write output: {e}")))?;
    Ok(Value::Nil)
}

fn write_errs(errs: &Sink, args: &[Value], newline: bool) -> Result<Value, RuntimeError> {
    let mut text: String = args.iter().map(Value::display).collect();
    if newline {
        text.push('\n');
    }
    // Diagnostics are worth seeing immediately, and there are far fewer of
    // them than of `print` calls, so stderr stays unbuffered.
    with_locked(errs, |w| {
        w.write_all(text.as_bytes()).and_then(|()| w.flush())
    })
    .map_err(|e| RuntimeError::new(format!("could not write to stderr: {e}")))?;
    Ok(Value::Nil)
}

/// All of stdin, to end of input — the whole-pipe counterpart to `read`.
fn read_all(out: &Sink) -> Result<Value, RuntimeError> {
    with_locked(out, |w| w.flush())
        .map_err(|e| RuntimeError::new(format!("could not write output: {e}")))?;
    let mut text = String::new();
    match std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
        Ok(_) => Ok(Value::Str(text.into())),
        Err(e) => err(format!("could not read input: {e}")),
    }
}

// ── calendar ─────────────────────────────────────────────────────────────────
//
// Howard Hinnant's `days_from_civil` / `civil_from_days`
// (http://howardhinnant.github.io/date_algorithms.html): a closed-form,
// proleptic-Gregorian conversion between (year, month, day) and a day count
// from 1970-01-01, correct for any year (including before 1970) without a
// calendar table.

fn date_to_epoch_day(y: &Value, m: &Value, d: &Value) -> Result<Value, RuntimeError> {
    Ok(Value::Int(days_from_civil(
        as_int(y)?,
        as_int(m)?,
        as_int(d)?,
    )))
}

fn date_from_epoch_day(z: &Value) -> Result<Value, RuntimeError> {
    let (y, m, d) = civil_from_days(as_int(z)?);
    Ok(Value::Tuple(
        vec![Value::Int(y), Value::Int(m), Value::Int(d)].into(),
    ))
}

/// Days since 1970-01-01 for the proleptic Gregorian date `(y, m, d)`. `m` is
/// expected to be `1..=12` (the Ramos side validates that); `d` is not
/// range-checked, so a day outside the month rolls over — `Date.new` on the
/// Ramos side leans on this to normalize rather than reject.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // day of year, may run outside [0, 365] when `d` does
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of `days_from_civil`: the proleptic Gregorian `(y, m, d)` for
/// `z` days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        // Only reachable if the clock is set before 1970.
        .unwrap_or(0)
}

/// Milliseconds since the first call — a clock that only moves forward, so it
/// is the one to measure elapsed time with. `now()` can jump when the system
/// clock is adjusted.
fn monotonic_millis() -> i64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as i64
}

/// xorshift64*, seeded from the clock on first use. Not cryptographic — it is
/// here for sampling, shuffling and test data, not for secrets.
fn next_random() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut seed = STATE.load(Ordering::Relaxed);
    if seed == 0 {
        // Any non-zero seed will do; xorshift is stuck at zero.
        seed = (unix_millis() as u64) | 1;
    }
    seed ^= seed >> 12;
    seed ^= seed << 25;
    seed ^= seed >> 27;
    STATE.store(seed, Ordering::Relaxed);
    seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// A uniform integer in `low..=high`. Bounds may arrive either way round.
fn random_int(low: &Value, high: &Value) -> Result<Value, RuntimeError> {
    let (a, b) = (as_int(low)?, as_int(high)?);
    let (low, high) = if a <= b { (a, b) } else { (b, a) };
    // `+ 1` for an inclusive range; as u128 so `i64::MIN..=i64::MAX` cannot
    // overflow the width.
    let span = (high as i128 - low as i128 + 1) as u128;
    let offset = (next_random() as u128) % span;
    Ok(Value::Int((low as i128 + offset as i128) as i64))
}

fn read_line(out: &Sink) -> Result<Value, RuntimeError> {
    // A prompt written with `print` carries no newline, so push it to the
    // terminal before blocking on input. The lock is released before the read.
    with_locked(out, |w| w.flush())
        .map_err(|e| RuntimeError::new(format!("could not write output: {e}")))?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Ok(Value::Nil), // end of input
        Ok(_) => {
            let trimmed = line.trim_end_matches(['\n', '\r']);
            Ok(Value::Str(trimmed.into()))
        }
        Err(e) => err(format!("could not read input: {e}")),
    }
}

// ── argument helpers ─────────────────────────────────────────────────────────

fn one(
    name: &str,
    args: &[Value],
    f: impl FnOnce(&Value) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    match args {
        [a] => f(a),
        _ => arity(name, 1, args.len()),
    }
}

fn two(
    name: &str,
    args: &[Value],
    f: impl FnOnce(&Value, &Value) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    match args {
        [a, b] => f(a, b),
        _ => arity(name, 2, args.len()),
    }
}

fn three(
    name: &str,
    args: &[Value],
    f: impl FnOnce(&Value, &Value, &Value) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    match args {
        [a, b, c] => f(a, b, c),
        _ => arity(name, 3, args.len()),
    }
}

fn arity_then(
    name: &str,
    args: &[Value],
    n: usize,
    f: impl FnOnce(&[Value]) -> Result<Value, RuntimeError>,
) -> Result<Value, RuntimeError> {
    if args.len() == n {
        f(args)
    } else {
        arity(name, n, args.len())
    }
}

fn pred(name: &str, args: &[Value], f: impl FnOnce(&Value) -> bool) -> Result<Value, RuntimeError> {
    one(name, args, |v| Ok(Value::Bool(f(v))))
}

fn as_str(v: &Value) -> Result<&str, RuntimeError> {
    match v {
        Value::Str(s) => Ok(s),
        other => err(format!("expected a String, got {}", other.type_name())),
    }
}

fn as_int(v: &Value) -> Result<i64, RuntimeError> {
    match v {
        Value::Int(n) => Ok(*n),
        other => err(format!("expected an Integer, got {}", other.type_name())),
    }
}

fn as_list(v: &Value) -> Result<&Arc<List>, RuntimeError> {
    match v {
        Value::List(l) => Ok(l),
        other => err(format!("expected a List, got {}", other.type_name())),
    }
}

fn arity<T>(name: &str, expected: usize, got: usize) -> Result<T, RuntimeError> {
    err(format!("{name} expects {expected} argument(s), got {got}"))
}

fn err<T>(message: impl Into<String>) -> Result<T, RuntimeError> {
    Err(RuntimeError::new(message))
}
