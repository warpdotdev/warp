use async_channel::Sender;

use crate::terminal::model::session::command_executor::{
    InBandCommand, InBandCommandCancelledEvent,
};
use crate::terminal::model::tmux::commands::TmuxCommand;
use crate::terminal::shell::ShellType;

/// Set of events sent by command executors.
pub enum ExecutorCommandEvent {
    /// The command should be executed.
    ExecuteCommand {
        command: InBandCommand,
        /// A Sender that can be used to signal that the command has been cancelled.
        /// Lets us unblock the command in the executor.
        cancel_tx: Sender<InBandCommandCancelledEvent>,
    },
    ExecuteTmuxCommand(TmuxCommand),
    /// The command identified by `id` should be cancelled.
    CancelCommand {
        id: String,
    },
}

pub fn shell_escape_single_quotes(command: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Fish => {
            // Backslash-escape single quotes for Fish.
            command.replace('\'', r"\'")
        }
        ShellType::PowerShell => {
            // In powershell we escape single quotes using two single quotes ''
            command.replace('\'', "''")
        }
        _ => {
            // For Bash and Zsh, replace each single quote with a '"'"' sequence.
            // The first single quote completes the single quoted string to the left,
            // the next three characters: "'" evaluate to a literal single quote in
            // bash/zsh, and then the final single quote starts a new single-quoted
            // string to the right. Effectively, this concatenates the left
            // single-quoted string, a literal single quote char, and the right
            // single-quoted string.
            command.replace('\'', r#"'"'"'"#)
        }
    }
}

/// Quotes a single shell argument so it is passed as data instead of being
/// interpreted as shell syntax.
///
/// Use this for complete interpolated arguments in generated command strings,
/// not for fragments that intentionally contain operators, pipes, or flags.
pub fn shell_quote_arg(value: &str, shell_type: ShellType) -> String {
    format!("'{}'", shell_escape_single_quotes(value, shell_type))
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
