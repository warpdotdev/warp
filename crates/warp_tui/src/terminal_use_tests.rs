use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionId, AIConversationId, AgentInteractionMetadata, BlockId,
    LongRunningCommandControlState, TaskId, TerminalModel, TranscriptScope, UserTakeOverReason,
};
use warpui::EntityIdMap;
use warpui_core::App;
use warpui_core::elements::tui::{TuiLayoutContext, TuiViewportWindow, TuiViewportedElement};

use super::{
    TerminalUseInterruptAction, TuiInputTarget, hide_agent_requested_command_from_top_level,
    inline_process_owns_input, terminal_use_conversation_to_resume, terminal_use_interrupt_action,
    tui_input_target, tui_input_target_for_state,
};
use crate::tui_block_list_viewport_source::TuiBlockListViewportSource;

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

#[test]
fn ordinary_long_running_command_owns_inline_input() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("cat", "");

    assert!(inline_process_owns_input(&model));
}
#[test]
fn shell_startup_routes_input_by_bootstrap_stage() {
    let mut model = TerminalModel::mock(None, None);
    assert_eq!(tui_input_target(&model), TuiInputTarget::AgentEditor);

    model.block_list_mut().reinit_shell();
    assert_eq!(tui_input_target(&model), TuiInputTarget::Disabled);
    assert_eq!(
        tui_input_target_for_state(false, true, false, false, false),
        TuiInputTarget::Disabled,
        "silent startup-script execution should keep the bootstrap editor visible",
    );
    assert_eq!(
        tui_input_target_for_state(false, true, true, false, false),
        TuiInputTarget::Pty,
        "visible startup-script output should accept interactive PTY input",
    );
}

#[test]
fn submit_policy_blocks_bootstrap_but_allows_ready_prompt() {
    assert!(
        !tui_input_target_for_state(false, false, false, false, false).agent_editor_owns_input(),
        "bootstrap submission must remain disabled"
    );
    assert!(
        tui_input_target_for_state(false, false, false, true, false).agent_editor_owns_input(),
        "the normal prompt must accept submission"
    );
}
#[test]
fn agent_command_owns_input_only_after_user_takeover() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("cargo build", "Compiling");
    let conversation_id = AIConversationId::new();
    let task_id = TaskId::new("terminal-use-task".to_owned());
    {
        let block = model.block_list_mut().active_block_mut();
        block.set_agent_interaction_mode_for_requested_command(
            AIAgentActionId::from("requested-command".to_owned()),
            Some(task_id.clone()),
            conversation_id,
        );
    }
    assert!(!inline_process_owns_input(&model));

    {
        let block = model.block_list_mut().active_block_mut();
        block
            .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
            .expect("command should become agent monitored");
    }
    assert!(!inline_process_owns_input(&model));

    {
        let block = model.block_list_mut().active_block_mut();
        block
            .take_over_control_for_user(UserTakeOverReason::Manual)
            .expect("user takeover should succeed");
    }
    assert!(inline_process_owns_input(&model));
}

#[test]
fn tagged_in_agent_keeps_editor_input_visible() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("cat", "");
    model
        .block_list_mut()
        .active_block_mut()
        .set_is_agent_tagged_in(true);

    assert!(!inline_process_owns_input(&model));
}

fn mark_visible_agent_requested_command(
    model: &mut TerminalModel,
    block_id: &BlockId,
    action_id: &AIAgentActionId,
) {
    model
        .block_list_mut()
        .mut_block_from_id(block_id)
        .expect("block should exist")
        .set_agent_interaction_mode(AgentInteractionMetadata::new(
            Some(action_id.clone()),
            AIConversationId::new(),
            None,
            None,
            false,
            false,
        ));
    model
        .block_list_mut()
        .set_visibility_of_block_for_ai_action(action_id, true);
}

#[test]
fn spawned_agent_requested_command_has_zero_top_level_height() {
    let action_id: AIAgentActionId = "action".to_owned().into();
    let (mut model, block_id) = model_with_finished_block("cargo build");
    mark_visible_agent_requested_command(&mut model, &block_id, &action_id);
    let model = Arc::new(FairMutex::new(model));

    {
        let model = model.lock();
        let block_list = model.block_list();
        let block = block_list
            .block_with_id(&block_id)
            .expect("block should exist");
        assert!(block.is_visible(block_list.transcript_scope()));
        assert!(block_list.block_heights().summary().height.as_f64() > 0.0);
    }

    assert!(hide_agent_requested_command_from_top_level(
        &model,
        Some(&action_id),
    ));

    let model = model.lock();
    let block_list = model.block_list();
    let block = block_list
        .block_with_id(&block_id)
        .expect("block should exist");
    assert!(!block.is_visible(block_list.transcript_scope()));
    assert_eq!(block_list.block_heights().summary().height.as_f64(), 0.0);
}

#[test]
fn spawned_user_command_keeps_its_top_level_height() {
    let (model, block_id) = model_with_finished_block("sleep 10");
    let model = Arc::new(FairMutex::new(model));
    let height_before = model
        .lock()
        .block_list()
        .block_heights()
        .summary()
        .height
        .as_f64();

    assert!(!hide_agent_requested_command_from_top_level(&model, None));

    let model = model.lock();
    let block_list = model.block_list();
    let block = block_list
        .block_with_id(&block_id)
        .expect("block should exist");
    assert!(block.is_visible(block_list.transcript_scope()));
    assert_eq!(
        block_list.block_heights().summary().height.as_f64(),
        height_before
    );
}

#[test]
fn hidden_agent_requested_command_leaves_no_viewport_gap() {
    App::test((), |app| async move {
        app.read(|app| {
            let action_id: AIAgentActionId = "action".to_owned().into();
            let mut model = TerminalModel::mock(None, None);
            model.simulate_block("cargo build", "agent output\r\n");
            model.simulate_block("echo done", "done\r\n");
            let agent_block_id = model
                .block_list()
                .blocks()
                .iter()
                .find(|block| block.command_to_string().contains("cargo build"))
                .expect("agent command block should exist")
                .id()
                .clone();
            let user_block_id = model
                .block_list()
                .blocks()
                .iter()
                .find(|block| block.command_to_string().contains("echo done"))
                .expect("user block should exist")
                .id()
                .clone();
            mark_visible_agent_requested_command(&mut model, &agent_block_id, &action_id);
            let model = Arc::new(FairMutex::new(model));

            assert!(hide_agent_requested_command_from_top_level(
                &model,
                Some(&action_id),
            ));

            let expected_height = {
                let model = model.lock();
                let block_list = model.block_list();
                block_list
                    .block_with_id(&user_block_id)
                    .expect("user block should exist")
                    .height(block_list.transcript_scope())
                    .as_f64()
                    .ceil() as usize
            };
            let source =
                TuiBlockListViewportSource::new(model, Rc::new(RefCell::new(HashMap::new())));
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let content = source.visible_items(
                TuiViewportWindow {
                    scroll_top: 0,
                    viewport_height: u16::MAX,
                },
                80,
                &mut layout_ctx,
                app,
            );

            assert_eq!(content.content_height, expected_height);
            assert_eq!(content.items.len(), 1);
            assert_eq!(content.items[0].origin_y, 0);
        });
    });
}

#[test]
fn terminal_use_interrupt_follows_takeover_then_process_interrupt_policy() {
    let agent = LongRunningCommandControlState::Agent {
        is_blocked: false,
        should_hide_responses: false,
    };
    assert_eq!(
        terminal_use_interrupt_action(Some(&agent), true),
        Some(TerminalUseInterruptAction::TakeControl)
    );

    let stopped = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Stop {
            should_auto_resume: true,
        },
    };
    assert_eq!(
        terminal_use_interrupt_action(Some(&stopped), true),
        Some(TerminalUseInterruptAction::InterruptCommand)
    );

    let manual = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::Manual,
    };
    assert_eq!(
        terminal_use_interrupt_action(Some(&manual), true),
        Some(TerminalUseInterruptAction::InterruptCommand)
    );

    let transferred = LongRunningCommandControlState::User {
        reason: UserTakeOverReason::TransferFromAgent {
            reason: "enter password".to_owned(),
        },
    };
    assert_eq!(
        terminal_use_interrupt_action(Some(&transferred), true),
        Some(TerminalUseInterruptAction::InterruptCommand)
    );

    assert_eq!(
        terminal_use_interrupt_action(None, true),
        Some(TerminalUseInterruptAction::InterruptCommand)
    );
    assert_eq!(terminal_use_interrupt_action(None, false), None);
}

#[test]
fn completed_user_controlled_requested_command_resumes_unless_tearing_down() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("cargo build", "Compiling");
    let conversation_id = AIConversationId::new();
    let task_id = TaskId::new("terminal-use-task".to_owned());
    let block_id = {
        let block = model.block_list_mut().active_block_mut();
        block.set_agent_interaction_mode_for_requested_command(
            AIAgentActionId::from("requested-command".to_owned()),
            Some(task_id.clone()),
            conversation_id,
        );
        block
            .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
            .expect("command should become agent monitored");
        block
            .take_over_control_for_user(UserTakeOverReason::Stop {
                should_auto_resume: true,
            })
            .expect("user takeover should succeed");
        block.id().clone()
    };

    assert_eq!(
        terminal_use_conversation_to_resume(&model, &block_id),
        Some(conversation_id)
    );

    model
        .block_list_mut()
        .active_block_mut()
        .set_user_control_for_teardown();
    assert_eq!(terminal_use_conversation_to_resume(&model, &block_id), None);
}
