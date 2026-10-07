pub mod fuzzy_match;
pub mod static_commands;

#[cfg(test)]
mod fuzzy_match_tests;

pub use static_commands::{SlashCommandArgumentHint, SlashCommandId, StaticCommand};
