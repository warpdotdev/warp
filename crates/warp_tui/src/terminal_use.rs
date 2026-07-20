//! TUI-specific terminal-use control and transcript policy.

use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionId, AIConversationId, Block, BlockId, LongRunningCommandControlState,
    TerminalModel,
};

/// Keeps an agent-requested command's canonical block out of the TUI's
/// top-level transcript. The shell-command action embeds the block's terminal
/// content inside its own disclosure, so the canonical block must have zero
/// layout height even after the shared CLI-subagent transition unhides it for
/// the GUI's adjacent-block presentation.
pub(super) fn hide_agent_requested_command_from_top_level(
    model: &Arc<FairMutex<TerminalModel>>,
    action_id: Option<&AIAgentActionId>,
) -> bool {
    let Some(action_id) = action_id else {
        return false;
    };
    model
        .lock()
        .block_list_mut()
        .set_visibility_of_block_for_ai_action(action_id, false);
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalUseInterruptAction {
    TakeControl,
    InterruptCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiInputTarget {
    /// No editable destination is ready, such as while Warp injects its
    /// bootstrap script or waits for the first post-bootstrap prompt. The
    /// session view renders the editor with a `Starting shell...` indicator,
    /// but submission is disabled and ordinary input is not forwarded to the
    /// PTY.
    Disabled,
    /// The foreground terminal process owns input during visible shell startup
    /// script interactions, alt-screen applications, or user-controlled
    /// long-running commands. The agent editor is hidden, the session view is
    /// focused, and terminal content forwards key, paste, and supported pointer
    /// events to the PTY.
    Pty,
    /// The shell is at an ordinary prompt and no foreground process owns
    /// input. The agent editor, menus, and footer are rendered, and focus moves
    /// to the editor unless a blocking interaction takes precedence.
    AgentEditor,
}

impl TuiInputTarget {
    pub(super) fn agent_editor_owns_input(self) -> bool {
        matches!(self, Self::AgentEditor)
    }

    pub(super) fn pty_owns_input(self) -> bool {
        matches!(self, Self::Pty)
    }
}

fn tui_input_target_for_state(
    alt_screen_active: bool,
    script_execution: bool,
    startup_script_visible: bool,
    bootstrap_precmd_done: bool,
    inline_process_owns_input: bool,
) -> TuiInputTarget {
    if alt_screen_active
        || (script_execution && startup_script_visible)
        || inline_process_owns_input
    {
        TuiInputTarget::Pty
    } else if !bootstrap_precmd_done {
        TuiInputTarget::Disabled
    } else {
        TuiInputTarget::AgentEditor
    }
}

pub(super) fn tui_input_target(terminal_model: &TerminalModel) -> TuiInputTarget {
    let block_list = terminal_model.block_list();
    tui_input_target_for_state(
        terminal_model.is_alt_screen_active(),
        block_list.is_script_execution(),
        block_list
            .active_block()
            .is_visible(block_list.transcript_scope()),
        block_list.is_bootstrapping_precmd_done(),
        inline_process_owns_input(terminal_model),
    )
}

pub(super) fn terminal_use_interrupt_action(
    control_state: Option<&LongRunningCommandControlState>,
    process_owns_input: bool,
) -> Option<TerminalUseInterruptAction> {
    match control_state {
        Some(LongRunningCommandControlState::Agent { .. }) => {
            Some(TerminalUseInterruptAction::TakeControl)
        }
        Some(LongRunningCommandControlState::User { .. }) => {
            Some(TerminalUseInterruptAction::InterruptCommand)
        }
        None if process_owns_input => Some(TerminalUseInterruptAction::InterruptCommand),
        None => None,
    }
}

pub(super) fn terminal_use_conversation_to_resume(
    terminal_model: &TerminalModel,
    block_id: &BlockId,
) -> Option<AIConversationId> {
    let metadata = terminal_model
        .block_list()
        .block_with_id(block_id)?
        .agent_interaction_metadata()?;
    (metadata.requested_command_action_id().is_some()
        && metadata
            .long_running_control_state()
            .is_some_and(LongRunningCommandControlState::should_auto_resume))
    .then_some(*metadata.conversation_id())
}

/// Whether a running inline command, rather than Warp's editor or agent, owns
/// keyboard input.
pub(super) fn user_controls_running_command(block: &Block) -> bool {
    block.is_active_and_long_running()
        && block.is_bootstrapped()
        && !block.is_in_band_command_block()
        && !block.is_agent_driving_command()
        && !block.is_agent_tagged_in()
}

pub(super) fn inline_process_owns_input(terminal_model: &TerminalModel) -> bool {
    user_controls_running_command(terminal_model.block_list().active_block())
}

#[cfg(test)]
#[path = "terminal_use_tests.rs"]
mod tests;
