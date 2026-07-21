//! Lexical scopes (PLAN phase 3).
//!
//! Scopes are `Arc`-linked frames. A binding is an entry in the current frame;
//! rebinding overwrites that entry. Lookups walk outward to the root. The
//! interior mutability lives here, in the scope — never inside a `Value`.
//!
//! `Arc` + `RwLock` rather than `Rc` + `RefCell` so a scope can cross a thread
//! boundary: an actor handler runs on its own thread and needs its captured
//! environment with it.

use super::value::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct Env(Arc<Scope>);

struct Scope {
    vars: RwLock<HashMap<String, Value>>,
    parent: Option<Env>,
}

impl Env {
    pub fn root() -> Env {
        Env(Arc::new(Scope {
            vars: RwLock::new(HashMap::new()),
            parent: None,
        }))
    }

    /// A fresh frame nested in `self` — for a function/lambda call or a block
    /// whose bindings should not leak to the enclosing scope.
    pub fn child(&self) -> Env {
        Env(Arc::new(Scope {
            vars: RwLock::new(HashMap::new()),
            parent: Some(self.clone()),
        }))
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        let mut cur = self.clone();
        loop {
            if let Some(v) = cur.0.vars.read().expect("scope lock").get(name) {
                return Some(v.clone());
            }
            let parent = cur.0.parent.clone();
            match parent {
                Some(p) => cur = p,
                None => return None,
            }
        }
    }

    /// Bind (or rebind) `name` in this frame.
    pub fn set(&self, name: String, value: Value) {
        self.0.vars.write().expect("scope lock").insert(name, value);
    }
}
