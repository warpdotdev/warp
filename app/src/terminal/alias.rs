use std::sync::Arc;

use smol_str::SmolStr;
use warp_completer::parsers::simple::all_parsed_commands;

use crate::terminal::model::session::Session;

/// Contains information about an aliased command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AliasedCommand {
    /// The alias itself.
    pub alias: SmolStr,
    /// The value that the alias is mapped to.
    pub alias_value: String,
}

/// Returns whether the alias can be expanded by Warp given its value.
///
/// We don't expand on any alias that starts with itself, as it leads to
/// cases where the alias is expanded twice: once as the user types in the
/// editor and again by the shell when the command is entered.
// TODO: CORE-240 Don't expand if any command in the alias value is equal
// to the alias itself.
pub fn is_expandable_alias(alias: &str, alias_value: &str) -> bool {
    if let Some(command_token) = alias_value.split_whitespace().next() {
        return alias != command_token;
    }
    // If the alias value is empty, we don't expand.
    false
}

/// Searches the source text for any aliases in a command position. Returns
/// information about the alias if found.
fn check_for_alias(source: &str, session: Arc<Session>) -> Option<AliasedCommand> {
    let mut all_commands_iterator =
        all_parsed_commands(source, session.shell_family().escape_char());
    all_commands_iterator.find_map(|command| {
        let first_token = command.parts.first()?;
        let alias_value = session.alias_value(&first_token.item)?;
        is_expandable_alias(&first_token.item, alias_value).then(|| AliasedCommand {
            alias: first_token.item.as_str().into(),
            alias_value: alias_value.into(),
        })
    })
}

pub async fn check_for_alias_async(source: &str, session: Arc<Session>) -> Option<AliasedCommand> {
    check_for_alias(source, session)
}

#[cfg(test)]
#[path = "alias_tests.rs"]
pub mod tests;
