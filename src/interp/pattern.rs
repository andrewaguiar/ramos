//! The pattern-matching engine (PLAN phase 3), shared by `case`, destructuring
//! assignment, and (later) `rescue`.
//!
//! Matching is two-phase: `try_match` collects bindings into a list and returns
//! them only on a full match, so a partial match never pollutes the scope of
//! the arm that failed.

use super::eval::RuntimeError;
use super::value::{map_key_value, List, Map, StructValue, Value};
use crate::ast::{MapKey, Pattern};

/// Returns the bindings a successful match produces, or `None` if the pattern
/// does not match the value.
///
/// The `Err` case is not "did not match" — it is a pattern that could never
/// match anything, such as a struct pattern naming an attribute the struct does
/// not declare. Those are reported rather than quietly falling through to the
/// next arm, which would turn a typo into a silent branch.
pub fn try_match(
    pattern: &Pattern,
    value: &Value,
) -> Result<Option<Vec<(String, Value)>>, RuntimeError> {
    let mut binds = Vec::new();
    if match_into(pattern, value, &mut binds)? {
        Ok(Some(binds))
    } else {
        Ok(None)
    }
}

fn match_into(
    pattern: &Pattern,
    value: &Value,
    binds: &mut Vec<(String, Value)>,
) -> Result<bool, RuntimeError> {
    Ok(match pattern {
        Pattern::Wildcard => true,
        Pattern::Binding(name) => {
            binds.push((name.clone(), value.clone()));
            true
        }
        Pattern::Int(n) => matches!(value, Value::Int(m) if m == n),
        Pattern::Float(x) => matches!(value, Value::Float(y) if y == x),
        Pattern::Bool(b) => matches!(value, Value::Bool(v) if v == b),
        Pattern::Nil => matches!(value, Value::Nil),
        Pattern::Symbol(s) => matches!(value, Value::Symbol(v) if &**v == s),
        Pattern::Str(s) => matches!(value, Value::Str(v) if &**v == s),
        Pattern::Tuple(pats) => match value {
            Value::Tuple(items) if items.len() == pats.len() => {
                let mut all = true;
                for (p, v) in pats.iter().zip(items.iter()) {
                    if !match_into(p, v, binds)? {
                        all = false;
                        break;
                    }
                }
                all
            }
            _ => false,
        },
        Pattern::List { elements, rest } => match value {
            Value::List(list) => match_list(elements, rest.as_deref(), list, binds)?,
            _ => false,
        },
        Pattern::Map(fields) => match value {
            Value::Map(map) => match_map(fields, map, binds)?,
            _ => false,
        },
        Pattern::Struct { path, fields } => match value {
            Value::Struct(s) => match_struct(path, fields, s, binds)?,
            _ => false,
        },
    })
}

/// `Name{field: pat}` — the value must be an instance of that struct, and every
/// field written must match. A bare `Name` (no fields, as in `rescue`) is the
/// type test on its own.
///
/// The name is compared as written, exactly as struct construction resolves it,
/// so `Name{...}` builds and destructures the same instances.
fn match_struct(
    path: &crate::ast::ModulePath,
    fields: &[(String, Pattern)],
    value: &std::sync::Arc<StructValue>,
    binds: &mut Vec<(String, Value)>,
) -> Result<bool, RuntimeError> {
    if value.def.name.to_string() != path.to_string() {
        return Ok(false);
    }
    // A subset match, like a map pattern: unwritten attributes are ignored.
    for (name, subpat) in fields {
        // An instance holds every attribute its struct declares, so a missing
        // one means the pattern named an attribute that does not exist. That
        // can never match, so it is a mistake to report rather than a branch
        // to skip — construction rejects the same typo.
        let Some(v) = value.get(name) else {
            return Err(RuntimeError::new(format!(
                "`{}` has no attribute `{name}`",
                value.def.name
            )));
        };
        if !match_into(subpat, &v.clone(), binds)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn match_list(
    elements: &[Pattern],
    rest: Option<&Pattern>,
    list: &std::sync::Arc<List>,
    binds: &mut Vec<(String, Value)>,
) -> Result<bool, RuntimeError> {
    let mut cur = list.clone();
    for elpat in elements {
        let (head, tail) = match &*cur {
            List::Cons(head, tail) => (head.clone(), tail.clone()),
            List::Nil => return Ok(false),
        };
        if !match_into(elpat, &head, binds)? {
            return Ok(false);
        }
        cur = tail;
    }
    match rest {
        Some(rest_pat) => match_into(rest_pat, &Value::List(cur), binds),
        None => Ok(matches!(&*cur, List::Nil)),
    }
}

fn match_map(
    entries: &[(MapKey, Pattern)],
    map: &Map,
    binds: &mut Vec<(String, Value)>,
) -> Result<bool, RuntimeError> {
    // A map pattern is a subset match: every key written must be present, extra
    // keys in the value are ignored. Unlike a struct, a map has no fixed shape,
    // so a key that is not there is an ordinary non-match.
    for (key, subpat) in entries {
        let Some(v) = map.get(&map_key_value(key)) else {
            return Ok(false);
        };
        if !match_into(subpat, &v.clone(), binds)? {
            return Ok(false);
        }
    }
    Ok(true)
}
