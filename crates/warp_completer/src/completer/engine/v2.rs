use crate::completer::TopLevelCommandCaseSensitivity;
use crate::signatures::{get_matching_signature_for_input, CommandRegistry};

/// Returns the name of the argument that should be given at `idx` for the given command.
pub(super) fn argument_name_at_index_for_command(
    command: &str,
    idx: usize,
    command_registry: &CommandRegistry,
    // TODO(CORE-2810)
    _command_case_sensitivity: TopLevelCommandCaseSensitivity,
) -> Option<String> {
    get_matching_signature_for_input(command, command_registry).and_then(|(found_signature, _)| {
        found_signature
            .arguments
            .get(idx)
            .map(|arg| arg.name.clone())
    })
}
