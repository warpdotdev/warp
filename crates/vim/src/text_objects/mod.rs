//! This module is for text-objects, e.g. `diw` and the like.
//!
//! See https://vimdoc.sourceforge.net/htmldoc/motion.html#text-objects
//! or enter ":help text-objects" in Vim.
mod block;
mod line;
mod paragraph;
mod quote;
mod word;
pub use line::*;
pub use paragraph::*;
pub use quote::*;
pub use word::*;

pub use self::block::*;
