use async_channel::Sender;
pub use warp_terminal::shell::{shell_escape_single_quotes, shell_quote_arg};

use crate::terminal::model::session::command_executor::{
    InBandCommand, InBandCommandCancelledEvent,
};

/// Set of events sent by command executors.
pub enum ExecutorCommandEvent {
    /// The command should be executed.
    ExecuteCommand {
        command: InBandCommand,
        /// A Sender that can be used to signal that the command has been cancelled.
        /// Lets us unblock the command in the executor.
        cancel_tx: Sender<InBandCommandCancelledEvent>,
    },
    /// The command identified by `id` should be cancelled.
    CancelCommand { id: String },
}
