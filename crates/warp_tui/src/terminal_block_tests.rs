use warp::tui_export::{
    AIAgentActionId, AIConversationId, AgentInteractionMetadata, BlockId, TerminalModel,
    TranscriptScope,
};

use super::should_render_terminal_block;

/// Builds a mock model with a single simulated (started + finished) block and
/// returns the model together with that block's id.
fn model_with_finished_block(command: &str) -> (TerminalModel, BlockId) {
    let mut model = TerminalModel::mock(None, None);
    model
        .block_list_mut()
        .set_transcript_scope(TranscriptScope::Unfiltered);
    model.simulate_block(command, "output\r\n");
    let block_id = model
        .block_list()
        .blocks()
        .iter()
        .rev()
        .find(|block| block.finished())
        .expect("simulated block should exist")
        .id()
        .clone();
    (model, block_id)
}

/// Tags the block with the given id as an agent-requested command, matching the
/// interaction mode set once a long-running agent command becomes
/// agent-monitored: it keeps its requested-command action id, but
/// `should_hide_block` has flipped to `false` (see
/// `InteractionMode::to_agent_monitored`).
fn mark_agent_monitored_command(model: &mut TerminalModel, block_id: &BlockId) {
    let action_id: AIAgentActionId = "action".to_owned().into();
    let conversation_id = AIConversationId::new();
    model
        .block_list_mut()
        .mut_block_from_id(block_id)
        .expect("block should exist")
        .set_agent_interaction_mode(AgentInteractionMetadata::new(
            Some(action_id),
            conversation_id,
            None,
            None,
            false,
            false,
        ));
}

#[test]
fn agent_monitored_command_block_is_not_rendered_at_top_level() {
    let (mut model, block_id) = model_with_finished_block("cargo build");
    mark_agent_monitored_command(&mut model, &block_id);

    let block_list = model.block_list();
    let block = block_list
        .block_with_id(&block_id)
        .expect("block should exist");

    // Sanity: this is an agent-requested command whose hide flag is off, so it
    // is otherwise "visible" and would leak into the top-level transcript.
    assert!(block.is_agent_requested_command());
    assert!(block.is_visible(block_list.transcript_scope()));

    // Regression: an agent's command is rendered inline inside its agent
    // block's shell-command view, so it must NOT also appear as a standalone
    // terminal block in the transcript (the "shows up twice" bug).
    assert!(!should_render_terminal_block(block, block_list));
}

#[test]
fn hidden_agent_requested_command_block_is_not_rendered_at_top_level() {
    let (mut model, block_id) = model_with_finished_block("echo hi");
    let action_id: AIAgentActionId = "action".to_owned().into();
    let conversation_id = AIConversationId::new();
    model
        .block_list_mut()
        .mut_block_from_id(&block_id)
        .expect("block should exist")
        .set_agent_interaction_mode(AgentInteractionMetadata::new_hidden(
            action_id,
            conversation_id,
        ));

    let block_list = model.block_list();
    let block = block_list
        .block_with_id(&block_id)
        .expect("block should exist");

    assert!(block.is_agent_requested_command());
    assert!(!should_render_terminal_block(block, block_list));
}

#[test]
fn user_command_block_is_rendered_at_top_level() {
    let (model, block_id) = model_with_finished_block("ls");
    let block_list = model.block_list();
    let block = block_list
        .block_with_id(&block_id)
        .expect("block should exist");

    assert!(!block.is_agent_requested_command());
    assert!(should_render_terminal_block(block, block_list));
}
