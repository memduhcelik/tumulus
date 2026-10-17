//! Paths, slices, iteration, recurse, pipe, comma, operators, bindings, builtins, and values.

mod ast;
mod builtin;
mod eval;
mod func;
mod machine;
mod modules;
mod names;
mod parse;
mod sort;
mod stream;
mod update;
mod value;

pub use ast::*;
pub use eval::{eval, eval_stream, eval_with};
pub use modules::set_import_origin;
pub use parse::{compile, compile_with, reads_input};

/// Drop state left by an earlier run in this thread.
pub(crate) fn reset_for_run() {
    modules::clear_cache();
    modules::set_import_origin(None);
    func::reset_scope();
    parse::reset_compile_state();
}

use eval::*;
use parse::*;
use value::*;
