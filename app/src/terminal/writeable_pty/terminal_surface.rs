//! [`TerminalSurface`]: the view-agnostic abstraction the terminal manager
//! drives. Both the GUI [`TerminalView`](crate::terminal::TerminalView) and the
//! headless TUI's root view implement it, so a single `TerminalManager<S>` can
//! own and drive either one without depending on the concrete GUI view.

use std::borrow::Cow;

#[cfg(unix)]
use warpui::AppContext;
use warpui::{Entity, ViewContext};

use crate::ai::agent::AIAgentPtyWriteMode;
#[cfg(unix)]
use crate::terminal::event::BlockCompletedEvent;
use crate::terminal::model::completions::ShellCompletion;
#[cfg(unix)]
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::{ShellLaunchData, SizeUpdate};

/// A single PTY-driving intent produced by a [`TerminalSurface`] from one of its
/// view events. The manager's wiring translates each intent into the
/// corresponding `PtyController` call, so surfaces never touch the controller
/// directly. Mirrors the arms the GUI's view-to-PTY wiring has always handled.
pub(crate) enum PtySurfaceIntent {
    /// Send end-of-transmission (Ctrl-D) to the PTY.
    CtrlD,
    /// Shut down the PTY.
    ShutdownPty,
    /// Write raw bytes to the PTY (keystroke passthrough).
    WriteBytes(Cow<'static, [u8]>),
    /// Write agent-originated bytes to the PTY under the given write mode.
    WriteAgentInput {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    /// Resize the PTY to match a new grid size.
    Resize(SizeUpdate),
    /// Execute a command in the session's shell.
    ExecuteCommand(ExecuteCommandEvent),
    /// Run native shell completions for the given input buffer, returning
    /// results on the provided channel.
    RunNativeShellCompletions {
        buffer_text: String,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
    },
}

/// A terminal front-end that a `TerminalManager<S>` can drive without knowing
/// the concrete view type. The required [`pty_intent`](Self::pty_intent) maps a
/// surface event to an optional [`PtySurfaceIntent`]; the lifecycle hooks let
/// the manager notify the surface of session events.
///
/// Every method is required: each surface must consciously handle (or
/// explicitly no-op) each session event rather than silently inheriting a
/// default.
pub(crate) trait TerminalSurface: Entity + Sized + 'static {
    /// Translates a surface event into the PTY intent it should drive, if any.
    fn pty_intent(event: &Self::Event) -> Option<PtySurfaceIntent>;

    /// Called once the shell has been determined and its PTY spawned.
    fn on_shell_determined(&mut self, ctx: &mut ViewContext<Self>);

    /// Called with the resolved shell launch data (used for shell indicators).
    fn on_active_shell_launch_data_updated(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    );

    /// Called when the PTY fails to spawn, with the underlying error.
    fn on_pty_spawn_failed(&mut self, error: anyhow::Error, ctx: &mut ViewContext<Self>);

    /// Whether the manager should run the password-prompt attributes poller
    /// while a block is executing. The poller mechanism is owned by the
    /// manager; this lets each surface decide whether it wants the front-end
    /// reactions (e.g. notifications, SSH drag-and-drop) gated behind it.
    #[cfg(unix)]
    fn wants_password_poll(&self, ctx: &AppContext) -> bool;

    /// Called when the attributes poller detects what looks like a password
    /// prompt, so the surface can react (e.g. notify the user or drive an SSH
    /// upload). `block_index` is the block that was running when polling began.
    #[cfg(unix)]
    fn on_possible_password_prompt(
        &mut self,
        block_index: Option<BlockIndex>,
        ctx: &mut ViewContext<Self>,
    );

    /// Called when a block completes (while the poller capability is wired), so
    /// the surface can react to completion of a polled block (e.g. finishing an
    /// SSH file upload).
    #[cfg(unix)]
    fn on_block_completed(&mut self, completed: &BlockCompletedEvent, ctx: &mut ViewContext<Self>);
}
