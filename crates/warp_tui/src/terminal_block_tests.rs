use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{
    AIAgentActionId, AIConversationId, AgentInteractionMetadata, Appearance, BlockId,
    TerminalModel, TranscriptScope,
};
use warpui::App;
use warpui_core::elements::tui::{Color, Modifier, TuiBufferExt, TuiElement, TuiRect, TuiSize};
use warpui_core::presenter::tui::TuiPresenter;

use super::{
    TerminalBlockElement, block_content_rows, should_render_terminal_block, terminal_block_cursor,
};
use crate::tui_builder::TuiUiBuilder;

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

#[test]
fn top_level_shell_command_row_uses_tinted_background() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (model, block_id) = model_with_finished_block("echo hi");
        let rows = block_content_rows(
            model
                .block_list()
                .block_with_id(&block_id)
                .expect("block should exist"),
        );
        let model = Arc::new(FairMutex::new(model));

        app.read(|ctx| {
            let height = rows.end.saturating_sub(rows.start) as u16;
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                TerminalBlockElement::visible_rows(model, block_id, rows, 12).finish(),
                TuiRect::new(0, 0, 12, height),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let command_row = lines
                .iter()
                .position(|line| line.contains("echo hi"))
                .expect("rendered command row");
            let output_row = lines
                .iter()
                .position(|line| line.contains("output"))
                .expect("rendered output row");
            assert_eq!(lines[command_row].trim_end(), "! echo hi");
            assert_eq!(lines[output_row].trim_end(), "output");
            let builder = TuiUiBuilder::from_app(ctx);
            let background = builder.shell_command_background();
            let prefix_style = builder.shell_command_prefix_style();

            for column in 0..12 {
                assert_eq!(frame.buffer[(column, command_row as u16)].bg, background);
                assert_eq!(frame.buffer[(column, output_row as u16)].bg, Color::Reset);
            }
            let prefix = &frame.buffer[(0, command_row as u16)];
            assert_eq!(prefix.symbol(), "!");
            assert_eq!(Some(prefix.fg), prefix_style.fg);
            assert!(prefix.modifier.contains(Modifier::BOLD));
            assert_eq!(frame.buffer[(1, command_row as u16)].symbol(), " ");
            assert_eq!(frame.buffer[(2, command_row as u16)].symbol(), "e");
            assert_eq!(frame.buffer[(0, output_row as u16)].symbol(), "o");
        });
    });
}

#[test]
fn inline_shell_command_content_keeps_terminal_background() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (model, block_id) = model_with_finished_block("echo hi");
        let rows = block_content_rows(
            model
                .block_list()
                .block_with_id(&block_id)
                .expect("block should exist"),
        );
        let height = rows.end.saturating_sub(rows.start) as u16;
        let model = Arc::new(FairMutex::new(model));

        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                TerminalBlockElement::content(model, block_id).finish(),
                TuiRect::new(0, 0, 12, height),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let command_row = lines
                .iter()
                .position(|line| line.contains("echo hi"))
                .expect("rendered command row");
            assert_eq!(lines[command_row].trim_end(), "echo hi");

            for column in 0..12 {
                assert_eq!(frame.buffer[(column, command_row as u16)].bg, Color::Reset);
            }
        });
    });
}

#[test]
fn shell_command_prefix_handles_single_column_width() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (model, block_id) = model_with_finished_block("echo hi");
        let rows = block_content_rows(
            model
                .block_list()
                .block_with_id(&block_id)
                .expect("block should exist"),
        );
        let model = Arc::new(FairMutex::new(model));

        app.read(|ctx| {
            let height = rows.end.saturating_sub(rows.start) as u16;
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                TerminalBlockElement::visible_rows(model, block_id, rows, 1).finish(),
                TuiRect::new(0, 0, 1, height),
                ctx,
            );
            assert_eq!(frame.buffer.to_lines()[0], "!");
            assert_eq!(
                frame.buffer[(0, 0)].bg,
                TuiUiBuilder::from_app(ctx).shell_command_background()
            );
        });
    });
}

#[test]
fn user_controlled_running_command_submits_cursor_within_window() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_long_running_block("python3", ">>> ");
    let block = model.block_list().active_block();

    // A finished/agent block never owns the inline cursor; an active
    // user-controlled command does when its cursor lands inside the window.
    let in_window = terminal_block_cursor(block, &(0..8), TuiSize::new(40, 8));
    let clipped = terminal_block_cursor(block, &(0..8), TuiSize::new(40, 0));
    assert!(in_window.is_some());
    assert_eq!(clipped, None);
}

#[test]
fn finished_command_does_not_submit_a_cursor() {
    let (model, block_id) = model_with_finished_block("ls");
    let block = model
        .block_list()
        .block_with_id(&block_id)
        .expect("block should exist");
    assert_eq!(
        terminal_block_cursor(block, &(0..8), TuiSize::new(40, 8)),
        None
    );
}
