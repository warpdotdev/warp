use std::borrow::Cow;

use async_channel::Sender;
#[cfg(unix)]
use warpui::AppContext;
use warpui::{Entity, View, ViewContext};

use crate::ai::agent::AIAgentPtyWriteMode;
#[cfg(unix)]
use crate::terminal::event::BlockCompletedEvent;
use crate::terminal::model::completions::ShellCompletion;
#[cfg(unix)]
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::{ShellLaunchData, SizeUpdate};

/// A normalized request from a terminal UI surface to the PTY controller.
///
/// This is the intentionally narrow vocabulary that `TerminalManager` uses to
/// drive the PTY without knowing the concrete UI implementation. It only
/// contains actions meaningful to the PTY/session boundary: process control,
/// byte writes, resizing, command execution, and native shell completions.
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

/// A UI surface driven by `TerminalManager` for a terminal frontend.
///
/// `TerminalView` is the only implementation in this PR. A future TUI root can
/// implement the same contract without making `TerminalManager` depend on the
/// GUI view type. Each surface defines how its own event type collapses into a
/// PTY/session intent via `From<&Self::Event> for Option<PtyIntent>`.
pub(crate) trait TerminalSurface: View + 'static
where
    for<'a> Option<PtyIntent>: From<&'a <Self as Entity>::Event>,
{
    /// Called once the shell starter has been determined and the PTY event loop
    /// has started, so the surface can react to shell launch metadata.
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>);

    /// Called when the active shell launch data is updated (e.g. shell indicator metadata).
    fn on_active_shell_launch_data_updated(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    );

    /// Called when the PTY fails to spawn so the surface can surface the error.
    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>);

    /// Whether the local manager should poll termios for a password prompt after a block starts.
    #[cfg(unix)]
    fn should_poll_for_password_prompt(&self, ctx: &AppContext) -> bool;

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
        completed: &BlockCompletedEvent,
        ctx: &mut ViewContext<Self>,
    );
}
