//! This module defines CharOrStr, a wrapper struct to deal with cases where we can have either a
//! char or a string. For example, this is used for Cells in Grids where we can have either just a char
//! (normal case) or a String (when we have zerowidth characters in a Cell). This structure helps abstract
//! away a clean API for dealing with both cases.
use std::fmt::{Display, Formatter};

/// Helper enum to represent either a char or a string, with corresponding API.
#[derive(Debug, PartialEq, Copy, Clone)]
pub enum CharOrStr<'a> {
    Char(char),
    Str(&'a str),
}

pub trait PushCharOrStr {
    fn push_char_or_str(&mut self, c: CharOrStr);
}

impl PushCharOrStr for String {
    fn push_char_or_str(&mut self, c: CharOrStr) {
        match c {
            CharOrStr::Char(c) => self.push(c),
            CharOrStr::Str(s) => self.push_str(s),
        }
    }
}

impl Display for CharOrStr<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CharOrStr::Char(c) => write!(f, "{c}"),
            CharOrStr::Str(s) => write!(f, "{s}"),
        }
    }
}
