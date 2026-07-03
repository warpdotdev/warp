//! This crate contains helper abstractions for dealing with JavaScript values and functions from
//! Rust.
mod convert;
mod js_function;
pub use convert::*;
pub use js_function::*;
