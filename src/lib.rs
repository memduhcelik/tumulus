//! Read JSON and write it back in jq's default or compact layout.

mod error;
mod number;
mod parse;
mod print;
mod value;

pub use error::Error;
pub use parse::write_input;
