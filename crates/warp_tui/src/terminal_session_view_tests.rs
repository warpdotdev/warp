use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionId, AIConversationId, AgentInteractionMetadata, BlockId, TerminalModel,
};
use warpui::EntityIdMap;
use warpui_core::elements::tui::{TuiLayoutContext, TuiViewportWindow, TuiViewportedElement};
use warpui_core::App;

use super::{hide_agent_requested_command_from_top_level, raw_prompt_if_not_blank};
use crate::tui_block_list_viewport_source::TuiBlockListViewportSource;

fn model_with_finished_block(command: &str) -> (TerminalModel, BlockId) {
    let mut model = TerminalModel::mock(None, None);
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
        assert!(block.is_visible(block_list.agent_view_state()));
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
    assert!(!block.is_visible(block_list.agent_view_state()));
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
    assert!(block.is_visible(block_list.agent_view_state()));
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
                .expect("user command block should exist")
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
                    .height(block_list.agent_view_state())
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
fn non_command_prompt_preserves_leading_whitespace() {
    assert_eq!(raw_prompt_if_not_blank("  /compact"), Some("  /compact"));
}

#[test]
fn whitespace_only_prompt_is_ignored() {
    assert_eq!(raw_prompt_if_not_blank(" \t\n"), None);
}
