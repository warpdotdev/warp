use std::collections::HashSet;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref WORD_LIST: HashSet<&'static str> = HashSet::from_iter(data::WORD_LIST.lines());
    pub static ref STACK_OVERFLOW_LIST: HashSet<&'static str> =
        HashSet::from_iter(data::STACK_OVERFLOW_LIST.lines());
    pub static ref COMMAND_LIST: HashSet<&'static str> =
        HashSet::from_iter(data::COMMAND_LIST.lines());
}

mod data {
    pub const WORD_LIST: &str = include_str!("../words.txt");
    pub const STACK_OVERFLOW_LIST: &str = include_str!("../stack_overflow.txt");
    pub const COMMAND_LIST: &str = include_str!("../stack_overflow_overlap_command.txt");
}
