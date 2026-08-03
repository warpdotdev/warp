//! Tests for the auto-fire drain logic that runs from [`super::TerminalView::drain_queued_prompts`].
//!
//! `TerminalView` orchestrates the input editor and the singleton `QueuedQueryModel` on
//! `FinishedReceivingOutput`. The lightweight tests below exercise the per-conversation singleton
//! semantics directly; the heavier tests construct a full `TerminalView` to validate the V2
//! cloud-mode integration paths.
use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use warp_cli::agent::Harness;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity, TypedActionView, ViewContext, ViewHandle};

use super::TerminalView;
use super::queued_prompts_panel::{
    QueuedPromptsPanelAction, QueuedPromptsPanelEvent, QueuedPromptsPanelView,
};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{ImageContext, UserQueryMode};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::block::FinishReason;
use crate::ai::blocklist::{
    AutofireAction, BlocklistAIControllerEvent, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    ConversationStatusUpdate, PendingAttachment, QueuedQuery, QueuedQueryId, QueuedQueryModel,
    QueuedQueryOrigin, ResponseStreamId,
};
use crate::features::FeatureFlag;
use crate::search::slash_command_menu::static_commands::commands;
use crate::server::server_api::ai::SpawnAgentRequest;
use crate::terminal::input::{Event as InputEvent, Input};
use crate::terminal::shared_session::SharedSessionStatus;
use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn user_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::QueueSlashCommand)
}

fn command_query(text: &str) -> QueuedQuery {
    QueuedQuery::new_command(text.to_owned(), QueuedQueryOrigin::AutoQueueToggle)
}

fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

fn query_with_attachments(text: &str, attachments: Vec<PendingAttachment>) -> QueuedQuery {
    QueuedQuery::new_with_attachments(
        text.to_owned(),
        QueuedQueryOrigin::QueueSlashCommand,
        attachments,
    )
}

/// Mirrors `TerminalView::drain_queued_prompts`' Complete path at the model level: peek the head
/// row's action, then remove the fired row (both `AutofireAction` variants carry the row id).
fn drain_one(
    model: &warpui::ModelHandle<QueuedQueryModel>,
    app: &mut App,
    conv: AIConversationId,
) -> Option<AutofireAction> {
    model.update(app, |m, ctx| {
        let action = m.peek_autofire(conv);
        if let Some(
            AutofireAction::Submit { query_id, .. }
            | AutofireAction::PopFromEditMode { query_id, .. },
        ) = &action
        {
            m.remove_fired_row(conv, *query_id, ctx);
        }
        action
    })
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
        prompt: Some(prompt.to_owned()),
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
        orchestration_handoff: None,
    }
}

/// A promptless cloud spawn request (`prompt: None`), modeling an empty-prompt
/// local-to-cloud handoff where the agent skips its initial turn.
fn promptless_cloud_spawn_request() -> SpawnAgentRequest {
    SpawnAgentRequest {
        prompt: None,
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
        orchestration_handoff: None,
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
    App::test((), |mut app| async move {
        // `QueuedQueryModel::new` reads and subscribes to `AISettings`, so settings
        // must be registered before it.
        initialize_settings_for_tests(&mut app);
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

        let action = drain_one(&model, &mut app, conv);
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "first"),
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
    // Events that always retire the locked initial Cloud Mode row, regardless of
    // CloudModeSetupV2. The V2 row removal is aligned with the legacy pending-user-query
    // block removal: these four events removed the legacy block under both V2-off and
    // V2-on, and now do the same for the V2 queue row.
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
fn failed_event_keeps_locked_queue_row_under_cloud_mode_setup_v2() {
    // Under CloudModeSetupV2, `Failed` keeps the legacy pending-user-query block in place
    // (alongside the failure tombstone). The V2 queue-row removal is gated on the same
    // condition, so the locked initial row stays so the user can review or retry.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            enter_cloud_setup_with_conversation(view, ctx);
            view.enqueue_initial_cloud_mode_prompt("initial".to_owned(), ctx)
                .expect("active conversation should accept cloud queue rows");
            view.handle_ambient_agent_event(
                &AmbientAgentViewModelEvent::Failed {
                    error_message: "failed setup".to_owned(),
                },
                ctx,
            );
            assert_eq!(
                queue_texts(view, ctx),
                vec![("initial".to_owned(), QueuedQueryOrigin::InitialCloudMode)]
            );
        });
    });
}

#[test]
fn failed_event_removes_locked_queue_row_without_cloud_mode_setup_v2() {
    // Without CloudModeSetupV2, the legacy pending-user-query block is removed on `Failed`.
    // The V2 queue-row removal follows the same gate.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(false);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.enqueue_initial_cloud_mode_prompt("initial".to_owned(), ctx)
                .expect("active conversation should accept cloud queue rows");
            view.handle_ambient_agent_event(
                &AmbientAgentViewModelEvent::Failed {
                    error_message: "failed setup".to_owned(),
                },
                ctx,
            );
            assert!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .is_empty()
            );
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
fn cloud_setup_enter_does_not_queue_followup_for_third_party_harness() {
    // Third-party (non-Oz) harness runs don't support prompt queueing, so an enter during
    // setup must not queue the follow-up. It falls through to being blocked, leaving the
    // typed text in the buffer (same observable outcome as the V2-disabled path).
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);
        let _agent_harness = FeatureFlag::AgentHarness.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(cloud_spawn_request("initial"), ctx);
                    model.set_harness(Harness::Claude, ctx);
                });

            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("do not queue this", ctx);
                input.input_enter(ctx);
            });

            assert!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .is_empty()
            );
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "do not queue this");
        });
    });
}

#[test]
fn cloud_setup_enter_queues_followup_while_setup_commands_run() {
    // Once the cloud session starts, the run is `AgentRunning` while environment setup
    // commands execute (still pre-first-exchange). Submitting in this window must queue the
    // follow-up, not send it as a live prompt the sharer would drop.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
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
                    // Session has started: the run moves to AgentRunning.
                    model.enter_viewing_existing_session(task_id, ctx);
                });
            // Environment setup commands are running (pre-first-exchange).
            view.model
                .lock()
                .block_list_mut()
                .set_is_executing_oz_environment_startup_commands(true);

            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("queue during setup", ctx);
                input.input_enter(ctx);
            });

            let queued_rows = queue_texts(view, ctx);
            assert!(queued_rows.iter().any(|(text, origin)| {
                text == "queue during setup" && *origin == QueuedQueryOrigin::AutoQueueToggle
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

            assert!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .is_empty()
            );
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
                    terminal_surface_id: terminal_view_id,
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
                    terminal_surface_id: terminal_view_id,
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
fn promptless_setup_complete_auto_sends_queued_prompt_to_viewer() {
    // A promptless handoff run (`request.prompt == None`) never fires a first
    // turn, so the normal completion drain never runs. When the cloud setup
    // phase completes, the prompt the user queued during setup must be sent to
    // the live shared session (viewer path -> `Event::SendAgentPrompt`).
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(promptless_cloud_spawn_request(), ctx);
                });
            QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                model.append(
                    conversation_id,
                    QueuedQuery::new(
                        "queued during setup".to_owned(),
                        QueuedQueryOrigin::AutoQueueToggle,
                    ),
                    ctx,
                );
            });
            conversation_id
        });

        let sent_prompts = Rc::new(RefCell::new(Vec::<String>::new()));
        let input = terminal.read(&app, |view, _| view.input.clone());
        let sent_prompts_for_subscription = sent_prompts.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&input, move |_, event: &InputEvent, _| {
                if let InputEvent::SendAgentPrompt { prompt, .. } = event {
                    sent_prompts_for_subscription
                        .borrow_mut()
                        .push(prompt.clone());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.maybe_drain_queue_after_promptless_setup(ctx);
        });

        assert_eq!(sent_prompts.borrow().as_slice(), ["queued during setup"]);
        terminal.read(&app, |_, ctx| {
            assert!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .is_empty()
            );
        });
    });
}

#[test]
fn promptless_setup_complete_with_initial_prompt_does_not_drain_queue() {
    // A run that carried an initial prompt (`request.prompt == Some(..)`) runs a
    // first turn and drains its queue on completion, so the setup-complete
    // marker must NOT drain it early.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            view.ambient_agent_view_model()
                .expect("cloud terminal should have an ambient model")
                .update(ctx, |model, ctx| {
                    model.spawn_agent_with_request(cloud_spawn_request("initial prompt"), ctx);
                });
            QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                model.append(
                    conversation_id,
                    QueuedQuery::new(
                        "queued during setup".to_owned(),
                        QueuedQueryOrigin::AutoQueueToggle,
                    ),
                    ctx,
                );
            });
            conversation_id
        });

        // The `DispatchedAgent` subscription enqueues the non-empty initial
        // prompt as an `InitialCloudMode` row once the spawn update flushes, so
        // the queue holds both that row and the prompt queued during setup.
        // Snapshot the queue before the drain to assert the drain leaves it
        // untouched.
        let queue_before = terminal.read(&app, |_, ctx| {
            QueuedQueryModel::as_ref(ctx)
                .queue(conversation_id)
                .iter()
                .map(|q| q.text().to_owned())
                .collect::<Vec<_>>()
        });
        assert!(
            queue_before.iter().any(|t| t == "queued during setup"),
            "setup-queued prompt should be present before the drain"
        );

        terminal.update(&mut app, |view, ctx| {
            view.maybe_drain_queue_after_promptless_setup(ctx);
        });

        // The initial-prompt run is not promptless, so the drain is a no-op:
        // the queue is identical before and after.
        terminal.read(&app, |_, ctx| {
            let queue_after = QueuedQueryModel::as_ref(ctx)
                .queue(conversation_id)
                .iter()
                .map(|q| q.text().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(queue_after, queue_before);
        });
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

        let action = drain_one(&model, &mut app, conv);
        match action {
            Some(AutofireAction::PopFromEditMode {
                text, is_command, ..
            }) => {
                assert_eq!(text, "first");
                assert!(!is_command);
            }
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
fn complete_drain_of_edited_command_restores_text_in_shell_mode() {
    // A command row being edited when the agent finishes cleanly is popped into the input in
    // shell mode, so the restored text stays a command rather than being submitted as a prompt.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        // Entering agent view puts the input in agent (AI) mode, so the drain must actively
        // switch it to shell mode for the restored command.
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            })
        });
        terminal.read(&app, |view, ctx| {
            assert!(view.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });

        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            let id = model.append(conversation_id, command_query("echo 1"), ctx);
            model.enter_edit_mode(conversation_id, id, ctx);
        });

        terminal.update(&mut app, |view, ctx| {
            view.drain_queued_prompts(conversation_id, FinishReason::Complete, ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), "echo 1");
            assert!(!view.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert!(model.queue(conversation_id).is_empty());
        });
    });
}

#[test]
fn error_drain_of_command_restores_text_in_shell_mode() {
    // On a non-clean finish, the head command is popped into the empty input in shell mode, so a
    // restored command stays a command.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        // The cancel restore path only fires for the conversation the user is viewing; entering
        // agent view makes `conversation_id` active and puts the input in agent (AI) mode.
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            })
        });

        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(conversation_id, command_query("echo 1"), ctx);
        });

        terminal.update(&mut app, |view, ctx| {
            view.drain_queued_prompts(conversation_id, FinishReason::Cancelled, ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), "echo 1");
            assert!(!view.ai_input_model.as_ref(ctx).is_ai_input_enabled());
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert!(model.queue(conversation_id).is_empty());
        });
    });
}

/// Verifies failed command auto-fire keeps the row queued when the input has a draft.
#[test]
fn complete_drain_keeps_command_row_when_dispatch_fails_with_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        let query_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(conversation_id, command_query("echo 1"), ctx)
        });

        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.replace_buffer_content("draft in progress", ctx);
            });
            view.drain_queued_prompts(conversation_id, FinishReason::Complete, ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                "draft in progress"
            );
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            let queue = model.queue(conversation_id);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].id(), query_id);
            assert_eq!(queue[0].text(), "echo 1");
            assert!(queue[0].is_command());
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
            drain_one(&model, &mut app, conv);
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
fn commit_edit_saves_current_editor_text_for_lrc_row() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        let query_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                conversation_id,
                QueuedQuery::new(
                    "stale committed".to_owned(),
                    QueuedQueryOrigin::LrcAutoQueue,
                ),
                ctx,
            )
        });
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.enter_edit_mode(conversation_id, query_id, ctx);
        });

        let queued_prompts_panel = terminal.read(&app, |view, ctx| {
            view.input()
                .as_ref(ctx)
                .queued_prompts_panel()
                .cloned()
                .expect("queue panel should exist")
        });
        queued_prompts_panel.update(&mut app, |panel, ctx| {
            panel.set_edit_buffer_text_for_test("edited before finish", ctx);
            panel.commit_edit(ctx);
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            let queue = model.queue(conversation_id);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].id(), query_id);
            assert_eq!(queue[0].text(), "edited before finish");
            assert_eq!(model.editing_row(conversation_id), None);
        });
    });
}

#[test]
fn lrc_finish_commits_edited_lrc_row_before_sending() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        let query_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                conversation_id,
                QueuedQuery::new(
                    "stale committed".to_owned(),
                    QueuedQueryOrigin::LrcAutoQueue,
                ),
                ctx,
            )
        });
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.enter_edit_mode(conversation_id, query_id, ctx);
        });

        let queued_prompts_panel = terminal.read(&app, |view, ctx| {
            view.input()
                .as_ref(ctx)
                .queued_prompts_panel()
                .cloned()
                .expect("queue panel should exist")
        });
        queued_prompts_panel.update(&mut app, |panel, ctx| {
            panel.set_edit_buffer_text_for_test("edited before finish", ctx);
        });

        let edit_commit_count = Rc::new(RefCell::new(0));
        let edit_commit_count_for_subscription = edit_commit_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &QueuedQueryModel::handle(ctx),
                move |_, event: &crate::ai::blocklist::QueuedQueryEvent, _| {
                    if matches!(
                        event,
                        crate::ai::blocklist::QueuedQueryEvent::EditCommitted { .. }
                    ) {
                        *edit_commit_count_for_subscription.borrow_mut() += 1;
                    }
                },
            );
        });

        let ai_query_count = Rc::new(RefCell::new(0));
        let input = terminal.read(&app, |view, _| view.input().clone());
        let ai_query_count_for_subscription = ai_query_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&input, move |_, event: &InputEvent, _| {
                if matches!(event, InputEvent::ExecuteAIQuery) {
                    *ai_query_count_for_subscription.borrow_mut() += 1;
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.send_lrc_queued_prompts(conversation_id, ctx);
        });

        assert_eq!(*edit_commit_count.borrow(), 1);
        assert_eq!(*ai_query_count.borrow(), 1);
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert!(model.queue(conversation_id).is_empty());
        });
    });
}

#[test]
fn lrc_finish_queued_compact_and_sends_followup_after_summary() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);
        let _summarization = FeatureFlag::SummarizationConversationCommand.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx),
                None
            );
        });

        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                conversation_id,
                QueuedQuery::new_with_attachments(
                    format!("{} follow up", commands::COMPACT_AND.name),
                    QueuedQueryOrigin::LrcAutoQueue,
                    vec![image_attachment("queued-context.png")],
                ),
                ctx,
            );
        });

        terminal.update(&mut app, |view, ctx| {
            view.send_lrc_queued_prompts(conversation_id, ctx);
        });

        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            let queue = model.queue(conversation_id);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].text(), "follow up");
            assert_eq!(queue[0].origin(), QueuedQueryOrigin::CompactAndSlashCommand);
            assert_eq!(queue[0].attachments().len(), 1);
        });

        let ai_query_count = Rc::new(RefCell::new(0));
        let input = terminal.read(&app, |view, _| view.input().clone());
        let ai_query_count_for_subscription = ai_query_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&input, move |_, event: &InputEvent, _| {
                if matches!(event, InputEvent::ExecuteAIQuery) {
                    *ai_query_count_for_subscription.borrow_mut() += 1;
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.drain_queued_prompts(conversation_id, FinishReason::Complete, ctx);
        });

        assert_eq!(*ai_query_count.borrow(), 1);
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert!(model.queue(conversation_id).is_empty());
        });
    });
}
#[test]
fn complete_drain_with_empty_queue_returns_none() {
    with_singleton(|mut app, model, conv| {
        let action = drain_one(&model, &mut app, conv);
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
fn enqueue_followup_prompt_appends_compact_and_row_when_v2_is_enabled() {
    // /compact-and follow-ups land in the queue with the CompactAndSlashCommand origin under V2.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);

            view.enqueue_followup_prompt(
                "follow up after summarize".to_owned(),
                QueuedQueryOrigin::CompactAndSlashCommand,
                conversation_id,
                ctx,
            );

            assert_eq!(
                queue_texts(view, ctx),
                vec![(
                    "follow up after summarize".to_owned(),
                    QueuedQueryOrigin::CompactAndSlashCommand
                )]
            );
            assert!(view.pending_user_query_view_id.is_none());
        });
    });
}

#[test]
fn enqueue_followup_prompt_appends_fork_and_compact_row_when_v2_is_enabled() {
    // /fork-and-compact follow-ups land in the queue with the ForkAndCompactSlashCommand origin.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);

            view.enqueue_followup_prompt(
                "work on the forked branch".to_owned(),
                QueuedQueryOrigin::ForkAndCompactSlashCommand,
                conversation_id,
                ctx,
            );

            assert_eq!(
                queue_texts(view, ctx),
                vec![(
                    "work on the forked branch".to_owned(),
                    QueuedQueryOrigin::ForkAndCompactSlashCommand
                )]
            );
        });
    });
}

#[test]
fn enqueue_followup_prompt_uses_supplied_conversation_id_when_v2_is_enabled() {
    // /fork-and-compact passes the newly forked conversation id directly, which can differ from
    // the currently selected conversation. The helper must respect that explicit id.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let selected_conversation_id = enter_cloud_setup_with_conversation(view, ctx);
            let other_conversation_id = AIConversationId::new();
            assert_ne!(selected_conversation_id, other_conversation_id);

            view.enqueue_followup_prompt(
                "goes to the forked id".to_owned(),
                QueuedQueryOrigin::ForkAndCompactSlashCommand,
                other_conversation_id,
                ctx,
            );

            assert!(queue_texts(view, ctx).is_empty());
            let other_queue = QueuedQueryModel::as_ref(ctx).queue(other_conversation_id);
            assert_eq!(other_queue.len(), 1);
            assert_eq!(other_queue[0].text(), "goes to the forked id");
            assert_eq!(
                other_queue[0].origin(),
                QueuedQueryOrigin::ForkAndCompactSlashCommand
            );
        });
    });
}

#[test]
fn enqueue_followup_prompt_falls_back_to_pending_block_when_v2_is_disabled() {
    // With V2 off, the helper must call into the legacy send_user_query_after_next_conversation_finished
    // path: no row gets appended to the queue model, and the queued_prompt_callback is armed so the
    // pending-user-query block lifecycle continues to handle the follow-up exactly as today.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);
        let _cloud_mode_setup_v2 = FeatureFlag::CloudModeSetupV2.override_enabled(true);
        let _queued_prompts_v2 = FeatureFlag::QueuedPromptsV2.override_enabled(false);
        let _pending_user_query_indicator =
            FeatureFlag::PendingUserQueryIndicator.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_cloud_setup_with_conversation(view, ctx);

            view.enqueue_followup_prompt(
                "legacy follow up".to_owned(),
                QueuedQueryOrigin::CompactAndSlashCommand,
                conversation_id,
                ctx,
            );

            assert!(
                QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .is_empty()
            );
            assert!(view.queued_prompt_callback.is_some());
            assert!(view.pending_user_query_view_id.is_some());
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
        let action = drain_one(&model, &mut app, conv);
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "second"),
            other => panic!("expected Submit(\"second\"), got {other:?}"),
        }

        // Complete again: pop "third".
        let action = drain_one(&model, &mut app, conv);
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "third"),
            other => panic!("expected Submit(\"third\"), got {other:?}"),
        }

        // Queue is now empty; the next drain returns None.
        let action = drain_one(&model, &mut app, conv);
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

        let action = drain_one(&model, &mut app, conv_a);
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "a-first"),
            other => panic!("expected Submit(\"a-first\"), got {other:?}"),
        }
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv_a).len(), 0);
            assert_eq!(m.queue(conv_b).len(), 1);
            assert_eq!(m.queue(conv_b)[0].text(), "b-first");
        });
    });
}

#[test]
fn send_now_action_emits_row_kind_and_leaves_rows_for_host_to_fire() {
    // Clicking "send now" emits a SendNow event identifying the row and whether it is a command,
    // but leaves the row in the queue so the host can dispatch it and remove it afterward. The
    // locked initial cloud-mode row is rejected by the model (covered by
    // `initial_cloud_mode_head_rejects_user_mutations_and_autofire`) and has its button disabled
    // in the panel, so it needs no separate panel test.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // The panel keys its queue lookups on the history model's active conversation for its
        // terminal view, so seed one and build the panel as a child of that terminal view.
        let (panel, conversation_id, _) = build_panel_with_active_conversation(&mut app);

        let (prompt_id, command_id) =
            QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
                let prompt_id = model.append(conversation_id, user_query("send me now"), ctx);
                let command_id = model.append(conversation_id, command_query("echo 1"), ctx);
                (prompt_id, command_id)
            });

        let send_now_events = Rc::new(RefCell::new(Vec::<(
            AIConversationId,
            QueuedQueryId,
            String,
            bool,
        )>::new()));
        let send_now_events_for_subscription = send_now_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&panel, move |_, event: &QueuedPromptsPanelEvent, _| {
                if let QueuedPromptsPanelEvent::SendNow {
                    conversation_id,
                    query_id,
                    text,
                    is_command,
                } = event
                {
                    send_now_events_for_subscription.borrow_mut().push((
                        *conversation_id,
                        *query_id,
                        text.clone(),
                        *is_command,
                    ));
                }
            });
        });

        panel.update(&mut app, |panel, ctx| {
            panel.handle_action(&QueuedPromptsPanelAction::SendNow(prompt_id), ctx);
            panel.handle_action(&QueuedPromptsPanelAction::SendNow(command_id), ctx);
        });

        assert_eq!(
            send_now_events.borrow().as_slice(),
            [
                (conversation_id, prompt_id, "send me now".to_owned(), false),
                (conversation_id, command_id, "echo 1".to_owned(), true)
            ]
        );
        // The panel leaves each row in place; the host removes it after firing.
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.queue(conversation_id).len(), 2);
        });
    });
}

#[test]
fn send_now_disabled_for_all_rows_while_initial_cloud_mode_row_is_present() {
    // While the locked initial cloud-mode prompt sits at the head (cloud environment setup),
    // every queued row's "send now" is disabled — there is no live agent to receive it yet. Once
    // that row is removed (the agent picked up the prompt), the remaining follow-up rows are
    // re-enabled.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (panel, conversation_id, _) = build_panel_with_active_conversation(&mut app);

        // The locked initial cloud-mode prompt, plus a follow-up queued during setup.
        let (initial_id, followup_id) =
            QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
                let initial_id = model.append(
                    conversation_id,
                    QueuedQuery::new("initial".to_owned(), QueuedQueryOrigin::InitialCloudMode),
                    ctx,
                );
                let followup_id = model.append(conversation_id, user_query("follow up"), ctx);
                (initial_id, followup_id)
            });

        // During setup, both rows' "send now" is disabled.
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(initial_id, ctx),
                Some(true)
            );
            assert_eq!(
                panel.send_now_button_disabled_for_test(followup_id, ctx),
                Some(true)
            );
        });

        // The agent picks up the prompt — the locked initial row is removed.
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.remove_initial_cloud_mode_row(conversation_id, ctx);
        });

        // The remaining follow-up row's "send now" is re-enabled.
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(followup_id, ctx),
                Some(false)
            );
        });
    });
}

#[test]
fn copying_locked_initial_cloud_mode_prompt_copies_full_prompt_to_clipboard() {
    // The locked initial cloud-mode prompt can't be edited or deleted, so its row offers a Copy
    // action instead. Firing it (the same action the Copy button dispatches) puts the full,
    // untruncated prompt — long, multiline content included — on the clipboard and leaves the row
    // in the queue.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (panel, conversation_id, _) = build_panel_with_active_conversation(&mut app);

        let long_prompt = format!("line one\nline two\n{}", "x".repeat(1000));
        let long_prompt_for_assert = long_prompt.clone();
        let initial_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                conversation_id,
                QueuedQuery::new(long_prompt, QueuedQueryOrigin::InitialCloudMode),
                ctx,
            )
        });

        panel.update(&mut app, |panel, ctx| {
            panel.handle_action(&QueuedPromptsPanelAction::CopyRow(initial_id), ctx);
        });

        app.update(|ctx| {
            assert_eq!(ctx.clipboard().read().plain_text, long_prompt_for_assert);
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.queue(conversation_id).len(), 1);
        });
    });
}

/// Builds a panel keyed to a fresh terminal view with an active conversation, mirroring the
/// construction the host `Input` performs. Returns the panel, the conversation id, and the
/// host input.
///
/// When the QueueSlashCommand flag is on, the host input already owns a panel for this
/// terminal view, and that panel is returned instead of constructing a second one: two panels
/// on the same terminal view both react to `EditEntered` and fight over edit-editor focus, and
/// the loser's `Blurred` handler commits the edit on the shared model out from under the test.
fn build_panel_with_active_conversation(
    app: &mut App,
) -> (
    ViewHandle<QueuedPromptsPanelView>,
    AIConversationId,
    ViewHandle<Input>,
) {
    let terminal = add_window_with_terminal(app, None);
    let terminal_view_id = terminal.read(app, |view, _| view.view_id);
    let conversation_id = BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
        history.set_active_conversation_id(id, terminal_view_id, ctx);
        id
    });
    let input = terminal.read(app, |view, _| view.input.clone());
    if let Some(panel) = input.read(app, |input, _| input.queued_prompts_panel().cloned()) {
        return (panel, conversation_id, input);
    }
    let (suggestions_mode_model, host_editor) = input.read(app, |input, _| {
        (
            input.suggestions_mode_model().clone(),
            input.editor().clone(),
        )
    });
    let cli_subagent_controller =
        terminal.read(app, |view, _| view.cli_subagent_controller.clone());
    let panel = terminal.update(app, |_, ctx| {
        ctx.add_view(move |ctx| {
            QueuedPromptsPanelView::new(
                terminal_view_id,
                suggestions_mode_model,
                cli_subagent_controller,
                host_editor,
                ctx,
            )
        })
    });
    (panel, conversation_id, input)
}

#[test]
fn redetermine_terminal_focus_preserves_focused_queued_prompt_editor() {
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });
        let input = terminal.read(&app, |view, _| view.input.clone());
        let panel = input
            .read(&app, |input, _| input.queued_prompts_panel().cloned())
            .expect("queue flag should create a queued prompts panel");
        let row_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(conversation_id, user_query("edit me"), ctx)
        });

        panel.update(&mut app, |panel, ctx| {
            panel.handle_action(&QueuedPromptsPanelAction::StartEditingRow(row_id), ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert!(panel.is_inline_edit_editor_focused(ctx));
        });

        terminal.update(&mut app, |view, ctx| {
            assert!(
                !view.redetermine_terminal_focus(ctx),
                "focused queued-prompt edits should hold focus during async focus reconciliation"
            );
        });

        panel.read(&app, |panel, ctx| {
            assert!(panel.is_inline_edit_editor_focused(ctx));
        });
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert_eq!(model.editing_row(conversation_id), Some(row_id));
        });
    });
}

#[test]
fn can_send_prompt_gates_buttons_and_hint_while_nonempty_input_gates_only_the_hint() {
    // When the host reports prompts cannot be sent (read-only shared-session viewer), every
    // row's send-now button is disabled and the enter hint hides. A non-empty input hides the
    // hint but leaves the buttons alone.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);

        let (panel, conversation_id, input) = build_panel_with_active_conversation(&mut app);
        let row_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(conversation_id, user_query("send me"), ctx)
        });

        // Default: sendable, hint shown.
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(row_id, ctx),
                Some(false)
            );
            assert!(panel.enter_hint_shown_for_test(ctx));
        });

        // Sending unavailable: button disabled and hint hidden.
        panel.update(&mut app, |panel, ctx| {
            panel.set_can_send_prompt(false, ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(row_id, ctx),
                Some(true)
            );
            assert!(!panel.enter_hint_shown_for_test(ctx));
        });

        // Sending available again: button re-enabled and hint restored.
        panel.update(&mut app, |panel, ctx| {
            panel.set_can_send_prompt(true, ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(row_id, ctx),
                Some(false)
            );
            assert!(panel.enter_hint_shown_for_test(ctx));
        });

        // Non-empty input: hint hidden, button stays enabled. The panel reads the host
        // editor's emptiness live, so writing into the input buffer is enough.
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("draft", ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert_eq!(
                panel.send_now_button_disabled_for_test(row_id, ctx),
                Some(false)
            );
            assert!(!panel.enter_hint_shown_for_test(ctx));
        });
    });
}

#[test]
fn enter_hint_hidden_during_inline_edit_and_for_locked_head() {
    // The enter hint hides while a row is in inline edit mode and while the locked initial
    // cloud-mode prompt sits at the head of the queue.
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);

        let (panel, conversation_id, _) = build_panel_with_active_conversation(&mut app);
        let row_id = QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(conversation_id, user_query("editable"), ctx)
        });

        panel.read(&app, |panel, ctx| {
            assert!(panel.enter_hint_shown_for_test(ctx));
        });

        // Inline edit hides the hint; cancelling restores it.
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.enter_edit_mode(conversation_id, row_id, ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert!(!panel.enter_hint_shown_for_test(ctx));
        });
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.cancel_edit(conversation_id, ctx);
        });
        panel.read(&app, |panel, ctx| {
            assert!(panel.enter_hint_shown_for_test(ctx));
        });

        // A locked initial cloud-mode head row hides the hint.
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.remove_by_id(conversation_id, row_id, ctx);
            model.append(
                conversation_id,
                QueuedQuery::new("initial".to_owned(), QueuedQueryOrigin::InitialCloudMode),
                ctx,
            );
        });
        panel.read(&app, |panel, ctx| {
            assert!(!panel.enter_hint_shown_for_test(ctx));
        });
    });
}

#[test]
fn multi_cycle_queue_keeps_each_rows_attachments_independent() {
    // attach -> queue -> attach -> queue: each row owns its own attachments, and draining one
    // never disturbs the other's.
    with_singleton(|mut app, model, conv| {
        let first_id = model.update(&mut app, |m, ctx| {
            m.append(
                conv,
                query_with_attachments("first", vec![image_attachment("first.png")]),
                ctx,
            )
        });
        let second_id = model.update(&mut app, |m, ctx| {
            m.append(
                conv,
                query_with_attachments("second", vec![image_attachment("second.png")]),
                ctx,
            )
        });

        model.read(&app, |m, _| {
            assert_eq!(
                m.attachments_for(conv, first_id)[0].file_name(),
                "first.png"
            );
            assert_eq!(
                m.attachments_for(conv, second_id)[0].file_name(),
                "second.png"
            );
        });

        // Drain the first row; the second row's attachments are untouched.
        let action = drain_one(&model, &mut app, conv);
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }
        model.read(&app, |m, _| {
            assert!(m.attachments_for(conv, first_id).is_empty());
            assert_eq!(m.attachments_for(conv, second_id).len(), 1);
            assert_eq!(
                m.attachments_for(conv, second_id)[0].file_name(),
                "second.png"
            );
        });
    });
}

#[test]
fn finish_reason_is_scoped_to_the_finished_conversation() {
    // An orchestration pane hosts the lead and local child conversations in one view, so the
    // most recent block in the pane can belong to a sibling conversation that is still
    // mid-turn. The per-conversation lookup must report the finished conversation's own block
    // as Complete (so its queued prompts drain) and the streaming sibling's as unfinished.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let finished_block =
                view.insert_dummy_ai_block("review".to_owned(), "done".to_owned(), ctx);
            let finished_conversation = finished_block.as_ref(ctx).conversation_id();
            // Inserted after the finished block, so it is the last block in the pane and
            // masks the pane-global `active_ai_block` / `last_ai_block` lookups.
            let streaming_block = view.insert_dummy_streaming_ai_block("working".to_owned(), ctx);
            let streaming_conversation = streaming_block.as_ref(ctx).conversation_id();
            assert_ne!(finished_conversation, streaming_conversation);

            assert_eq!(
                view.finish_reason_for_conversation(finished_conversation, ctx),
                Some(FinishReason::Complete)
            );
            assert_eq!(
                view.finish_reason_for_conversation(streaming_conversation, ctx),
                None
            );
            // A conversation with no blocks in this pane has no finish reason.
            assert_eq!(
                view.finish_reason_for_conversation(AIConversationId::new(), ctx),
                None
            );
        });
    });
}

#[test]
fn finished_receiving_output_drains_queue_when_sibling_block_masks_turn_end() {
    // End-to-end through the controller-event path: `FinishedReceivingOutput` for a finished
    // conversation must drain that conversation's queue even when a sibling conversation's
    // still-streaming block is the most recent block in the pane (orchestration panes host the
    // lead and local child conversations in one view).
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let finished_block =
                view.insert_dummy_ai_block("review".to_owned(), "done".to_owned(), ctx);
            let finished_conversation = finished_block.as_ref(ctx).conversation_id();
            // Inserted after the finished block, so it is the last block in the pane and
            // masks the pane-global `active_ai_block` / `last_ai_block` lookups.
            let streaming_block = view.insert_dummy_streaming_ai_block("working".to_owned(), ctx);
            let streaming_conversation = streaming_block.as_ref(ctx).conversation_id();
            assert_ne!(finished_conversation, streaming_conversation);

            QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                model.append(finished_conversation, user_query("queued follow up"), ctx);
                model.append(
                    streaming_conversation,
                    user_query("sibling stays queued"),
                    ctx,
                );
            });

            view.handle_ai_controller_event(
                view.ai_controller.clone(),
                &BlocklistAIControllerEvent::FinishedReceivingOutput {
                    stream_id: ResponseStreamId::new_for_test(),
                    conversation_id: finished_conversation,
                },
                ctx,
            );

            // The finished conversation's queued prompt fired; the still-streaming sibling's
            // queue is untouched.
            let model = QueuedQueryModel::as_ref(ctx);
            assert!(model.queue(finished_conversation).is_empty());
            assert_eq!(model.queue(streaming_conversation).len(), 1);
            assert_eq!(
                model.queue(streaming_conversation)[0].text(),
                "sibling stays queued"
            );
        });
    });
}
