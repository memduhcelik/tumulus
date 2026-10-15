//! Paths, slices, iteration, recurse, pipe, comma, operators, bindings, builtins, and values.

mod ast;
mod builtin;
mod eval;
mod func;
mod machine;
mod parse;
mod stream;
mod update;
mod value;

pub use ast::*;
pub use eval::{eval, eval_with};
pub use parse::{compile, compile_with, reads_input};

use eval::*;
use parse::*;
use value::*;
