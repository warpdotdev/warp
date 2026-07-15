//! TUI-specific terminal-use control and transcript policy.

use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionId, AIConversationId, BlockId, LongRunningCommandControlState, TerminalModel,
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

pub(super) fn terminal_use_interrupt_action(
    control_state: &LongRunningCommandControlState,
) -> TerminalUseInterruptAction {
    match control_state {
        LongRunningCommandControlState::Agent { .. } => TerminalUseInterruptAction::TakeControl,
        LongRunningCommandControlState::User { .. } => TerminalUseInterruptAction::InterruptCommand,
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

pub(super) fn user_controlled_line_bytes(input: &str) -> Vec<u8> {
    let mut bytes = input.as_bytes().to_vec();
    #[cfg(target_os = "windows")]
    bytes.push(b'\r');
    #[cfg(not(target_os = "windows"))]
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
#[path = "terminal_use_tests.rs"]
mod tests;
