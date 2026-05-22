use crate::completer::TopLevelCommandCaseSensitivity;
use crate::signatures::CommandRegistry;

/// Returns the name of the argument that should be given at `idx` for the given command.
pub(super) fn argument_name_at_index_for_command(
    command: &str,
    idx: usize,
    command_registry: &CommandRegistry,
    command_case_sensitivity: TopLevelCommandCaseSensitivity,
) -> Option<String> {
    command_registry
        .signature_from_line(command, command_case_sensitivity)
        .and_then(|found_signature| {
            let arguments = found_signature.signature.arguments();
            arguments
                .get(idx)
                .map(|arg| arg.name().unwrap_or_default().to_string())
        })
}
