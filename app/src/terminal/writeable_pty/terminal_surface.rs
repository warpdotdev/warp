use std::borrow::Cow;

use async_channel::Sender;
#[cfg(unix)]
use warpui::AppContext;
use warpui::{Entity, View, ViewContext};

use crate::ai::agent::AIAgentPtyWriteMode;
#[cfg(unix)]
use crate::terminal::event::AfterBlockCompletedEvent;
use crate::terminal::model::completions::ShellCompletion;
#[cfg(unix)]
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::{ShellLaunchData, SizeUpdate};

/// A normalized request from a terminal UI surface to the PTY controller.
///
/// This is the narrow vocabulary that `TerminalManager` uses to drive the PTY
/// without knowing the concrete UI implementation. It only contains actions
/// meaningful to the PTY/session boundary: process control, byte writes,
/// resizing, command execution, and native shell completions.
pub(crate) enum PtyIntent {
    CtrlD,
    ShutdownPty,
    WriteBytes(Cow<'static, [u8]>),
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    Resize(SizeUpdate),
    ExecuteCommand(ExecuteCommandEvent),
    RunNativeShellCompletions {
        buffer_text: String,
        results_tx: Sender<Vec<ShellCompletion>>,
    },
}

/// Event types that can be projected into an [`Option<PtyIntent>`].
pub(crate) trait PtyIntentEvent {
    /// Projects this event into a PTY/session intent, or `None` if it is not a
    /// PTY-driving event.
    fn pty_intent(&self) -> Option<PtyIntent>;
}

/// A terminal frontend surface driven by `TerminalManager`.
///
/// Each surface defines how its own event type collapses into a PTY/session intent.
pub(crate) trait TerminalSurface: View + 'static
where
    <Self as Entity>::Event: PtyIntentEvent,
{
    /// Whether the local manager should start polling termios for a password prompt
    /// after the given block starts.
    #[cfg(unix)]
    fn should_start_password_prompt_polling(&self, command: &str, ctx: &AppContext) -> bool;

    /// Whether the local manager should stop password-prompt polling for this completed block.
    #[cfg(unix)]
    fn should_stop_password_prompt_polling(&self, completed: &AfterBlockCompletedEvent) -> bool;

    /// Called once the shell starter has been determined and the PTY event loop
    /// has started, so the surface can react to shell launch metadata.
    #[cfg(feature = "local_tty")]
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>);

    /// Called when the active shell launch data is updated (e.g. shell indicator metadata).
    fn on_active_shell_launch_data_updated(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    );

    /// Called when the PTY fails to spawn so the surface can surface the error.
    #[cfg(feature = "local_tty")]
    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>);

    /// Called when termios indicates a likely password prompt is blocking the active block.
    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        block_index: Option<BlockIndex>,
        ctx: &mut ViewContext<Self>,
    );

    /// Called when the block the poller was tracking completes.
    #[cfg(unix)]
    fn on_polled_block_completed(
        &mut self,
        completed: &AfterBlockCompletedEvent,
        ctx: &mut ViewContext<Self>,
    );
}
