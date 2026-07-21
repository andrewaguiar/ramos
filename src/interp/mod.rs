//! The tree-walking interpreter (PLAN phases 3–4): runtime values, scopes, the
//! pattern-matching engine, module resolution, and the evaluator.

mod env;
mod eval;
mod freevars;
mod natives;
mod pattern;
mod value;

pub use eval::{
    run, run_tests, run_with_args, run_with_streams, sink, RuntimeError, Session, Sink, TestOutcome,
};
pub use value::Value;
