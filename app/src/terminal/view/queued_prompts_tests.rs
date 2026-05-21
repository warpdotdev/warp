//! Tests for the auto-fire drain logic that runs from [`super::TerminalView::drain_queued_prompts`].
//!
//! `TerminalView` orchestrates the input editor and the singleton `QueuedQueryModel` on
//! `FinishedReceivingOutput`. The lightweight tests below exercise the per-conversation singleton
//! semantics directly; the heavier tests construct a full `TerminalView` to validate the V2
//! cloud-mode integration paths.
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, ViewContext, ViewHandle};

use super::TerminalView;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::UserQueryMode;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::{
    AutofireAction, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatusUpdate,
    QueuedQuery, QueuedQueryModel, QueuedQueryOrigin,
};
use crate::features::FeatureFlag;
use crate::server::server_api::ai::SpawnAgentRequest;
use crate::terminal::input::Event as InputEvent;
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn user_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::QueueSlashCommand)
}

fn add_window_with_cloud_mode_terminal(app: &mut App) -> ViewHandle<TerminalView> {
    let tips_model = app.add_model(|_| Default::default());
    let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test_with_cloud_mode(tips_model, None, true, ctx)
    });
    terminal.update(app, |view, _| {
        view.model.lock().set_is_dummy_cloud_mode_session(true);
    });
    terminal
}

fn cloud_spawn_request(prompt: &str) -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: prompt.to_owned(),
        mode: UserQueryMode::Normal,
        config: None,
        title: None,
        team: None,
        agent_identity_uid: None,
        skill: None,
        attachments: vec![],
        interactive: None,
        parent_run_id: None,
        runtime_skills: vec![],
        referenced_attachments: vec![],
        conversation_id: None,
        initial_snapshot_token: None,
        snapshot_disabled: None,
    }
}

fn enter_cloud_setup_with_conversation(
    view: &mut TerminalView,
    ctx: &mut ViewContext<TerminalView>,
) -> AIConversationId {
    view.model
        .lock()
        .set_shared_session_status(SharedSessionStatus::ViewPending);
    view.enter_ambient_agent_setup(None, ctx);
    view.ai_context_model
        .as_ref(ctx)
        .selected_conversation_id(ctx)
        .expect("cloud setup should select a conversation")
}

/// Returns the queue rows for `view`'s active conversation, looked up against the
/// `QueuedQueryModel` singleton. Empty when no conversation is selected.
fn queue_texts(
    view: &TerminalView,
    ctx: &ViewContext<TerminalView>,
) -> Vec<(String, QueuedQueryOrigin)> {
    let Some(conversation_id) = view
        .ai_context_model
        .as_ref(ctx)
        .selected_conversation_id(ctx)
    else {
        return Vec::new();
    };
    QueuedQueryModel::as_ref(ctx)
        .queue(conversation_id)
        .iter()
        .map(|query| (query.text().to_owned(), query.origin()))
        .collect()
}

fn with_singleton<F>(test: F)
where
    F: FnOnce(App, warpui::ModelHandle<QueuedQueryModel>, AIConversationId) + 'static,
{
    App::test((), |app| async move {
        let _ = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let model = app.add_singleton_model(QueuedQueryModel::new);
        test(app, model, AIConversationId::new());
    });
}

#[test]
fn complete_drain_pops_head_and_returns_submit_action() {
    // On Complete, the next queued prompt fires via Submit.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn dispatched_cloud_prompt_uses_locked_queue_row_when_v2_is_enabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(cloud_spawn_request("write tests"), ctx);
                });
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::DispatchedAgent, ctx);

            assert_eq!(
                queue_texts(view, ctx),
                vec![(
                    "write tests".to_owned(),
                    QueuedQueryOrigin::InitialCloudMode
                )]
            );
            assert!(view.pending_user_query_view_id.is_none());
        });
    });
}

#[test]
fn dispatched_cloud_followup_uses_locked_queue_row_when_v2_is_enabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");
        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.enter_viewing_existing_session(task_id, ctx);
                    model.submit_cloud_followup("follow up".to_owned(), ctx);
                });
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::FollowupDispatched, ctx);

            assert_eq!(
                queue_texts(view, ctx),
                vec![("follow up".to_owned(), QueuedQueryOrigin::InitialCloudMode)]
            );
            assert!(view.pending_user_query_view_id.is_none());
        });
    });
}

#[test]
fn cloud_setup_cleanup_events_remove_the_locked_queue_row() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            let active_block_id = view.model.lock().block_list().active_block_id().clone();
            let cleanup_events = [
                AmbientAgentViewModelEvent::HarnessCommandStarted {
                    block_id: active_block_id,
                },
                AmbientAgentViewModelEvent::Failed {
                    error_message: "failed setup".to_owned(),
                },
                AmbientAgentViewModelEvent::Cancelled,
                AmbientAgentViewModelEvent::NeedsGithubAuth,
                AmbientAgentViewModelEvent::HandoffSnapshotUploadFailed {
                    error_message: "upload failed".to_owned(),
                },
            ];

            for event in cleanup_events {
                view.enqueue_initial_cloud_mode_prompt("initial".to_owned(), ctx)
                    .expect("active conversation should accept cloud queue rows");
                view.handle_ambient_agent_event(&event, ctx);
                assert!(
                    QueuedQueryModel::as_ref(ctx)
                        .queue(conversation_id)
                        .is_empty(),
                    "event should remove locked cloud row: {event:?}"
                );
            }
        });
    });
}

#[test]
fn cloud_setup_enter_queues_followup_input_when_v2_is_enabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(cloud_spawn_request("initial"), ctx);
                });

            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("queue this next", ctx);
                input.input_enter(ctx);
            });

            let queued_rows = queue_texts(view, ctx);
            assert!(queued_rows.iter().any(|(text, origin)| {
                text == "queue this next" && *origin == QueuedQueryOrigin::AutoQueueToggle
            }));
            assert!(view.input.as_ref(ctx).buffer_text(ctx).is_empty());
        });
    });
}

#[test]
fn cloud_setup_enter_remains_blocked_when_v2_is_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(false);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(cloud_spawn_request("initial"), ctx);
                });

            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("blocked prompt", ctx);
                input.input_enter(ctx);
            });

            assert!(QueuedQueryModel::as_ref(ctx)
                .queue(conversation_id)
                .is_empty());
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "blocked prompt");
        });
    });
}

#[test]
fn terminal_cloud_status_transition_drains_once_through_cloud_followup_input_event() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _handoff = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");
        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.enter_viewing_existing_session(task_id, ctx);
                });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::NotShared);
            view.pending_cloud_followup_task_id = Some(task_id);
            QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                model.append(
                    conversation_id,
                    QueuedQuery::new(
                        "queued cloud follow up".to_owned(),
                        QueuedQueryOrigin::AutoQueueToggle,
                    ),
                    ctx,
                );
            });
            conversation_id
        });

        let followup_events = Rc::new(RefCell::new(Vec::<String>::new()));
        let input = terminal.read(&app, |view, _| view.input.clone());
        let followup_events_for_subscription = followup_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&input, move |_, event: &InputEvent, _| {
                if let InputEvent::SubmitCloudFollowup { prompt } = event {
                    followup_events_for_subscription
                        .borrow_mut()
                        .push(prompt.clone());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            let history_model = BlocklistAIHistoryModel::handle(ctx);
            let terminal_view_id = view.view_id;
            view.handle_ai_history_model_event(
                history_model.clone(),
                &BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id,
                    terminal_view_id,
                    update: ConversationStatusUpdate::Changed {
                        prev_status: ConversationStatus::InProgress,
                    },
                    new_status: ConversationStatus::Success,
                },
                ctx,
            );
            view.handle_ai_history_model_event(
                history_model,
                &BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id,
                    terminal_view_id,
                    update: ConversationStatusUpdate::Changed {
                        prev_status: ConversationStatus::Success,
                    },
                    new_status: ConversationStatus::Success,
                },
                ctx,
            );
        });

        assert_eq!(
            followup_events.borrow().as_slice(),
            ["queued cloud follow up"]
        );
    });
}

#[test]
fn complete_drain_with_first_row_in_edit_mode_returns_pop_from_edit_mode() {
    // When the first row is being edited, drain produces a PopFromEditMode action carrying the
    // row's last-committed text (per spec, NOT any uncommitted live-editor buffer text).
    with_singleton(|mut app, model, conv| {
        let id_a = model.update(&mut app, |m, ctx| m.append(conv, user_query("first"), ctx));
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("second"), ctx);
            m.enter_edit_mode(conv, id_a, ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        match action {
            Some(AutofireAction::PopFromEditMode { text }) => assert_eq!(text, "first"),
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        // Edit mode is cleared after pop.
        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), None);
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_non_empty_input_preserves_edited_head_row() {
    // The host skips autofire when the queue head is being edited and the input already contains
    // text, which leaves the queued row in place for the next completion.
    with_singleton(|mut app, model, conv| {
        let id_a = model.update(&mut app, |m, ctx| m.append(conv, user_query("first"), ctx));
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("second"), ctx);
            m.enter_edit_mode(conv, id_a, ctx);
        });

        let simulated_input_is_non_empty = true;
        if !(simulated_input_is_non_empty
            && model.read(&app, |m, _| m.first_row_is_in_edit_mode(conv)))
        {
            model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        }

        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), Some(id_a));
            assert_eq!(m.queue(conv).len(), 2);
            assert_eq!(m.queue(conv)[0].text(), "first");
            assert_eq!(m.queue(conv)[1].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_empty_queue_returns_none() {
    with_singleton(|mut app, model, conv| {
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        assert!(action.is_none());
    });
}

#[test]
fn error_or_cancel_drain_pops_front_when_input_is_empty() {
    // On Error/Cancelled with an empty input, the next queued prompt's text is restored to the
    // input by popping it (which the host then writes into the buffer).
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let popped = model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        let popped = popped.expect("queue had a head");
        assert_eq!(popped.text(), "first");
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn error_or_cancel_drain_leaves_queue_intact_when_input_is_non_empty() {
    // When the input is non-empty, the drain skips popping so the queue remains intact.
    //
    // The host (`TerminalView`) gates the pop on input-empty. We model that here by simply not
    // popping when the simulated input is non-empty, and asserting the queue remains unchanged.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let simulated_input_is_non_empty = true;
        if !simulated_input_is_non_empty {
            model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        }

        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 2);
            assert_eq!(m.queue(conv)[0].text(), "first");
        });
    });
}

#[test]
fn complete_drain_after_error_drain_continues_with_next_row() {
    // After an Error/Cancelled drain pops one row and the user later submits successfully, the
    // *next* Complete drain pops the following row.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
            m.append(conv, user_query("third"), ctx);
        });

        // Error: input is empty, pop "first" and restore to input.
        let popped = model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        assert_eq!(
            popped.map(|q| q.text().to_owned()),
            Some("first".to_owned())
        );

        // Complete: pop "second".
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "second"),
            other => panic!("expected Submit(\"second\"), got {other:?}"),
        }

        // Complete again: pop "third".
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "third"),
            other => panic!("expected Submit(\"third\"), got {other:?}"),
        }

        // Queue is now empty; the next drain returns None.
        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv, ctx));
        assert!(action.is_none());
    });
}

#[test]
fn drain_is_isolated_per_conversation() {
    // A drain for conversation A must not pop rows from conversation B.
    with_singleton(|mut app, model, conv_a| {
        let conv_b = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv_a, user_query("a-first"), ctx);
            m.append(conv_b, user_query("b-first"), ctx);
        });

        let action = model.update(&mut app, |m, ctx| m.pop_for_autofire(conv_a, ctx));
        match action {
            Some(AutofireAction::Submit { text }) => assert_eq!(text, "a-first"),
            other => panic!("expected Submit(\"a-first\"), got {other:?}"),
        }
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv_a).len(), 0);
            assert_eq!(m.queue(conv_b).len(), 1);
            assert_eq!(m.queue(conv_b)[0].text(), "b-first");
        });
    });
}
