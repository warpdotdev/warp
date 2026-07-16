use warp::tui_export::{
    AIConversationId, Appearance, BlocklistAIHistoryModel, ConversationStatus,
    ReceivedMessageDisplay,
};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, EntityId};

use super::{agent_message_section_id, agent_status, render_agent_message};
use crate::agent_block::CollapsibleSectionStates;
use crate::orchestration_model::TuiOrchestrationModel;
use crate::status::TuiStatusState;
use crate::tui_builder::TuiUiBuilder;
const INFRA_RUN_ID: &str = "00000000-0000-0000-0000-000000000001";
const UI_RUN_ID: &str = "00000000-0000-0000-0000-000000000002";
const PARENT_RUN_ID: &str = "00000000-0000-0000-0000-000000000003";

/// Registers the appearance and history models needed by the renderer.
fn register_models(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::default());
    app.add_singleton_model(|_| TuiOrchestrationModel::new_for_test());
}

/// Creates one parent conversation for child-message tests.
fn add_parent(app: &mut App) -> AIConversationId {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(EntityId::new(), false, false, false, ctx);
            history
                .conversation_mut(&conversation_id)
                .expect("parent conversation was just created")
                .set_run_id(PARENT_RUN_ID.to_string());
            conversation_id
        })
    })
}

/// Creates a named child with a sender run ID and lifecycle status.
fn add_child(
    app: &mut App,
    parent_id: AIConversationId,
    name: &str,
    run_id: &str,
    status: ConversationStatus,
) -> AIConversationId {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let surface_id = EntityId::new();
            let conversation_id =
                history.start_new_conversation(surface_id, false, false, false, ctx);
            let conversation = history
                .conversation_mut(&conversation_id)
                .expect("child conversation was just created");
            conversation.set_agent_name(name.to_owned());
            conversation.set_run_id(run_id.to_owned());
            history.set_parent_for_conversation(conversation_id, parent_id);
            history.update_conversation_status(surface_id, conversation_id, status, ctx);
            conversation_id
        })
    })
}

#[test]
fn parent_sender_renders_as_orchestrator_in_child_transcript() {
    App::test((), |mut app| async move {
        register_models(&mut app);
        let parent_id = add_parent(&mut app);
        let child_id = add_child(
            &mut app,
            parent_id,
            "ui-implementer",
            UI_RUN_ID,
            ConversationStatus::InProgress,
        );

        app.read(|ctx| {
            let states = CollapsibleSectionStates::default();
            let received = message(PARENT_RUN_ID, "instruction", "Hi from the orchestrator");
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_agent_message(&states, &received, child_id, ctx),
                TuiRect::new(0, 0, 80, 2),
                ctx,
            );
            let header = frame.buffer.to_lines()[0].clone();
            assert!(header.contains("Orchestrator"), "{header}");
            assert!(!header.contains("Unknown agent"), "{header}");
        });
    });
}

/// Builds one received message payload.
fn message(sender: &str, subject: &str, body: &str) -> ReceivedMessageDisplay {
    ReceivedMessageDisplay {
        message_id: format!("message-{sender}"),
        sender_agent_id: sender.to_owned(),
        addresses: vec!["parent-run".to_owned()],
        subject: subject.to_owned(),
        message_body: body.to_owned(),
    }
}

#[test]
fn conversation_statuses_map_to_shared_tui_statuses() {
    assert_eq!(
        agent_status(&ConversationStatus::InProgress),
        TuiStatusState::Running
    );
    assert_eq!(
        agent_status(&ConversationStatus::TransientError),
        TuiStatusState::Running
    );
    assert_eq!(
        agent_status(&ConversationStatus::WaitingForEvents),
        TuiStatusState::Running
    );
    assert_eq!(
        agent_status(&ConversationStatus::Blocked {
            blocked_action: "approval".to_owned(),
        }),
        TuiStatusState::Blocked
    );
    assert_eq!(
        agent_status(&ConversationStatus::Success),
        TuiStatusState::Succeeded
    );
    assert_eq!(
        agent_status(&ConversationStatus::Error),
        TuiStatusState::Failed
    );
    assert_eq!(
        agent_status(&ConversationStatus::Cancelled),
        TuiStatusState::Cancelled
    );
}

#[test]
fn running_child_message_matches_the_design_layout_and_styles() {
    App::test((), |mut app| async move {
        register_models(&mut app);
        let parent_id = add_parent(&mut app);
        add_child(
            &mut app,
            parent_id,
            "infrastructure-bot",
            INFRA_RUN_ID,
            ConversationStatus::InProgress,
        );

        app.read(|ctx| {
            let states = CollapsibleSectionStates::default();
            let message = message(INFRA_RUN_ID, "progress", "Starting to build infrastructure");
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_agent_message(&states, &message, parent_id, ctx),
                TuiRect::new(0, 0, 80, 2),
                ctx,
            );
            let lines = frame
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .collect::<Vec<_>>();
            assert!(lines[0].starts_with("● "));
            assert!(lines[0].contains(" infrastructure-bot  ▸"));
            assert!(lines.iter().all(|line| !line.contains("Starting to build")));

            let builder = TuiUiBuilder::from_app(ctx);
            assert_eq!(
                frame.buffer[(0, 0)].fg,
                TuiStatusState::Running
                    .glyph_style(&builder)
                    .fg
                    .expect("running status has a foreground")
            );
            assert_eq!(frame.buffer[(2, 0)].fg, frame.buffer[(4, 0)].fg);
            assert!(frame.buffer[(4, 0)].modifier.contains(Modifier::BOLD));

            states.set_collapsed(agent_message_section_id(&message), false);
            let expanded = presenter.present_element(
                render_agent_message(&states, &message, parent_id, ctx),
                TuiRect::new(0, 0, 80, 2),
                ctx,
            );
            assert!(expanded.buffer.to_lines()[0].contains(" ▾"));
            assert_eq!(
                expanded.buffer.to_lines()[1].trim_end(),
                "    Starting to build infrastructure"
            );
            assert_eq!(
                expanded.buffer[(4, 1)].fg,
                builder
                    .muted_text_style()
                    .fg
                    .expect("muted text has a foreground")
            );
        });
    });
}

#[test]
fn message_preview_wraps_with_a_hanging_indent_and_falls_back_to_subject() {
    App::test((), |mut app| async move {
        register_models(&mut app);
        let parent_id = add_parent(&mut app);
        add_child(
            &mut app,
            parent_id,
            "ui-implementer",
            UI_RUN_ID,
            ConversationStatus::Success,
        );

        app.read(|ctx| {
            let states = CollapsibleSectionStates::default();
            let received = message(
                UI_RUN_ID,
                "progress",
                "Starting to implement a responsive interface",
            );
            states.set_collapsed(agent_message_section_id(&received), false);
            let mut presenter = TuiPresenter::new();
            let wrapped = presenter.present_element(
                render_agent_message(&states, &received, parent_id, ctx),
                TuiRect::new(0, 0, 24, 4),
                ctx,
            );
            let wrapped_lines = wrapped
                .buffer
                .to_lines()
                .into_iter()
                .map(|line| line.trim_end().to_owned())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            assert!(wrapped_lines[0].starts_with("✓ "));
            assert!(wrapped_lines[1..]
                .iter()
                .all(|line| line.starts_with("    ")));

            let fallback = presenter.present_element(
                render_agent_message(
                    &states,
                    &message(UI_RUN_ID, "Finished verification", "   "),
                    parent_id,
                    ctx,
                ),
                TuiRect::new(0, 0, 40, 2),
                ctx,
            );
            assert_eq!(
                fallback.buffer.to_lines()[1].trim_end(),
                "    Finished verification"
            );
        });
    });
}
