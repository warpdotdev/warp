use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use session_sharing_protocol::common::SessionId;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};
use warpui::App;

use super::super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use super::{
    classify_renderable_error, map_cli_session_status, map_conversation_status,
    LocalAgentTaskSyncModel,
};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::RenderableAIError;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, MockAIClient, TaskStatusUpdate};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};

/// Helper to assert a (state, Option<TaskStatusUpdate>) tuple.
fn assert_update(
    (state, update): (AgentTaskState, Option<TaskStatusUpdate>),
    expected_state: AgentTaskState,
    expected_code: Option<PlatformErrorCode>,
    message_contains: Option<&str>,
) {
    assert_eq!(state, expected_state, "unexpected AgentTaskState");
    match (update, expected_code, message_contains) {
        (Some(u), code, msg) => {
            assert_eq!(u.error_code, code, "unexpected PlatformErrorCode");
            if let Some(substr) = msg {
                assert!(
                    u.message.contains(substr),
                    "message {:?} does not contain {:?}",
                    u.message,
                    substr
                );
            }
        }
        (None, None, None) => {}
        (None, _, _) => panic!("expected a TaskStatusUpdate, got None"),
    }
}

// --- classify_renderable_error ---

#[test]
fn quota_limit_is_failed_with_insufficient_credits() {
    assert_update(
        classify_renderable_error(&RenderableAIError::QuotaLimit {
            user_display_message: None,
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::InsufficientCredits),
        Some("credits"),
    );
}

#[test]
fn server_overloaded_is_error_with_resource_unavailable() {
    assert_update(
        classify_renderable_error(&RenderableAIError::ServerOverloaded),
        AgentTaskState::Error,
        Some(PlatformErrorCode::ResourceUnavailable),
        Some("overloaded"),
    );
}

#[test]
fn internal_warp_error_is_error() {
    assert_update(
        classify_renderable_error(&RenderableAIError::InternalWarpError),
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
        Some("internal error"),
    );
}

#[test]
fn context_window_exceeded_is_failed() {
    assert_update(
        classify_renderable_error(&RenderableAIError::ContextWindowExceeded("too big".into())),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::InternalError),
        Some("Context window exceeded"),
    );
}

#[test]
fn invalid_api_key_is_failed_with_auth_required() {
    assert_update(
        classify_renderable_error(&RenderableAIError::InvalidApiKey {
            provider: "OpenAI".into(),
            model_name: "gpt-4".into(),
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::AuthenticationRequired),
        Some("OpenAI"),
    );
}

#[test]
fn aws_bedrock_credentials_is_failed_with_auth_required() {
    assert_update(
        classify_renderable_error(&RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid {
            model_name: "claude-v2".into(),
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::AuthenticationRequired),
        Some("claude-v2"),
    );
}

#[test]
fn other_error_is_error_with_internal() {
    assert_update(
        classify_renderable_error(&RenderableAIError::Other {
            error_message: "something broke".into(),
            will_attempt_resume: false,
            waiting_for_network: false,
        }),
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
        Some("something broke"),
    );
}

// --- map_conversation_status ---

/// A yielded conversation must report `IN_PROGRESS` to the task service
/// so the task row stays active across the yield. No status message is
/// attached because the yield is an internal state.
#[test]
fn map_conversation_status_waiting_for_events_reports_in_progress_with_no_message() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation = AIConversation::new(false, false);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });
        history_model.update(&mut app, |model, ctx| {
            let conv = model
                .conversation_mut(&conversation_id)
                .expect("conversation was just restored");
            conv.update_status(ConversationStatus::WaitingForEvents, terminal_view_id, ctx);
        });

        history_model.read(&app, |model, _| {
            let conv = model.conversation(&conversation_id).unwrap();
            assert_eq!(conv.status(), &ConversationStatus::WaitingForEvents);
            let (state, update) = map_conversation_status(conv);
            assert_eq!(state, AgentTaskState::InProgress);
            assert!(
                update.is_none(),
                "WaitingForEvents must not attach a status message"
            );
        });
    });
}

#[test]
fn map_conversation_status_in_progress_reports_in_progress_with_no_message() {
    let conversation = AIConversation::new(false, false);
    assert_eq!(
        conversation.status(),
        &ConversationStatus::InProgress,
        "freshly constructed conversation should start in InProgress"
    );

    let (state, update) = map_conversation_status(&conversation);

    assert_eq!(state, AgentTaskState::InProgress);
    assert!(update.is_none());
}

// --- map_cli_session_status ---

#[test]
fn cli_in_progress_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::InProgress);
    assert_eq!(state, AgentTaskState::InProgress);
    assert!(update.is_none());
}

#[test]
fn cli_success_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Success);
    assert_eq!(state, AgentTaskState::Succeeded);
    assert!(update.is_none());
}

#[test]
fn cli_blocked_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Blocked {
        message: Some("needs approval".into()),
    });
    assert_eq!(state, AgentTaskState::Blocked);
    let update = update.expect("should have status update");
    assert!(update.message.contains("needs approval"));
}

#[test]
fn cli_blocked_without_message() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Blocked { message: None });
    assert_eq!(state, AgentTaskState::Blocked);
    assert!(update.is_none());
}

// --- Model-level tests ---

/// Parses a fixed UUID into an `AmbientAgentTaskId`. Using a constant uuid
/// makes test failures easier to read than `Uuid::new_v4()`.
fn fixed_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440a00"
        .parse()
        .expect("valid task id")
}

fn fixed_session_id() -> SessionId {
    "550e8400-e29b-41d4-a716-446655440a01"
        .parse()
        .expect("valid session id")
}

/// Yields back to the executor a few times so any `ctx.spawn`-scheduled
/// fire-and-forget tasks can drive their underlying mock RPC. A short
/// timer (smaller than the test budget) is enough; we just need the
/// background poll to happen at least once.
async fn pump_spawned_tasks() {
    for _ in 0..5 {
        warpui::r#async::Timer::after(Duration::from_millis(2)).await;
    }
}

fn install_model_with_call_counter(
    app: &mut App,
) -> (
    warpui::ModelHandle<LocalAgentTaskSyncModel>,
    Arc<AtomicUsize>,
) {
    register_cli_agent_sessions_model(app);
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_for_mock = counter.clone();
    let mut mock = MockAIClient::new();
    mock.expect_update_agent_task()
        .returning(move |_, _, _, _, _| {
            counter_for_mock.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
    let ai_client: Arc<dyn AIClient> = Arc::new(mock);
    let model = app.add_singleton_model(|ctx| {
        LocalAgentTaskSyncModel::new_with_ai_client_for_test(ai_client, ctx)
    });
    (model, counter)
}

/// Registers the `CLIAgentSessionsModel` singleton. The merged
/// `LocalAgentTaskSyncModel` subscribes to it during construction, so tests
/// that instantiate the model must register it first.
fn register_cli_agent_sessions_model(app: &mut App) {
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
}

#[test]
fn shared_session_link_fires_update_agent_task_with_session_id() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // A local orchestrator conversation owned by this client: not a
        // viewer, not a remote-child placeholder, and has a `task_id`.
        let mut conversation = AIConversation::new(false, false);
        let task_id = fixed_task_id();
        conversation.set_run_id(task_id.to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);
        let session_id = fixed_session_id();

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id,
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "update_agent_task must be invoked exactly once for the new (task_id, session_id) pair"
        );
    });
}

#[test]
fn shared_session_link_uses_correct_argument_order() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        let task_id = fixed_task_id();
        conversation.set_run_id(task_id.to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        register_cli_agent_sessions_model(&mut app);
        // Verify the exact argument shape we send to the server:
        //   update_agent_task(task_id, None, Some(session_id), None, None)
        let session_id = fixed_session_id();
        let mut mock = MockAIClient::new();
        mock.expect_update_agent_task()
            .withf(
                move |arg_task_id, task_state, arg_session_id, conv_id, status_msg| {
                    *arg_task_id == task_id
                        && task_state.is_none()
                        && *arg_session_id == Some(session_id)
                        && conv_id.is_none()
                        && status_msg.is_none()
                },
            )
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let _model = app.add_singleton_model(|ctx| {
            LocalAgentTaskSyncModel::new_with_ai_client_for_test(ai_client, ctx)
        });

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id,
            });
        });

        pump_spawned_tasks().await;
        // Mock drop verifies `.times(1)` and `.withf` predicate.
    });
}

#[test]
fn shared_session_link_skips_viewer_conversations() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // A viewer-side conversation: even if it carries a task_id, this
        // client does not own the task and must not link.
        let mut conversation =
            AIConversation::new(/* is_viewing_shared_session */ true, false);
        conversation.set_run_id(fixed_task_id().to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id: fixed_session_id(),
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "viewer guard must skip the RPC"
        );
    });
}

#[test]
fn shared_session_link_skips_remote_child_conversations() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id(fixed_task_id().to_string());
        conversation.mark_as_remote_child();
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id: fixed_session_id(),
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "remote-child guard must skip the RPC"
        );
    });
}

#[test]
fn shared_session_link_skips_when_task_id_missing() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // No set_run_id call: the conversation has no task_id yet.
        let conversation = AIConversation::new(false, false);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id,
                session_id: fixed_session_id(),
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "missing task_id must skip the RPC"
        );
    });
}

#[test]
fn shared_session_link_skips_unknown_conversation() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let (_model, counter) = install_model_with_call_counter(&mut app);

        // Emit for a conversation that was never registered: the subscriber
        // must early-return without firing an RPC.
        let bogus_conversation_id = AIConversationId::new();
        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
                conversation_id: bogus_conversation_id,
                session_id: fixed_session_id(),
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "unknown conversation must skip the RPC"
        );
    });
}

#[test]
fn conversation_server_token_assigned_fires_update_with_conversation_id() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        let task_id = fixed_task_id();
        conversation.set_run_id(task_id.to_string());
        conversation.set_server_conversation_token("server-conversation-id".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        register_cli_agent_sessions_model(&mut app);
        // Verify the exact argument shape we send to the server:
        //   update_agent_task(task_id, Some(state), None, Some(token), None)
        let mut mock = MockAIClient::new();
        mock.expect_update_agent_task()
            .withf(
                move |arg_task_id, task_state, arg_session_id, conv_id, status_msg| {
                    *arg_task_id == task_id
                        && task_state.is_some()
                        && arg_session_id.is_none()
                        && conv_id.as_deref() == Some("server-conversation-id")
                        && status_msg.is_none()
                },
            )
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let _model = app.add_singleton_model(|ctx| {
            LocalAgentTaskSyncModel::new_with_ai_client_for_test(ai_client, ctx)
        });

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_view_id,
            });
        });

        pump_spawned_tasks().await;
        // Mock drop verifies `.times(1)` and the predicate.
    });
}

#[test]
fn conversation_server_token_assigned_skips_viewer_conversations() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation =
            AIConversation::new(/* is_viewing_shared_session */ true, false);
        conversation.set_run_id(fixed_task_id().to_string());
        conversation.set_server_conversation_token("server-conversation-id".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_view_id,
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "viewer guard must skip the RPC for token-assigned events"
        );
    });
}

#[test]
fn conversation_server_token_assigned_skips_remote_child_conversations() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id(fixed_task_id().to_string());
        conversation.set_server_conversation_token("server-conversation-id".to_string());
        conversation.mark_as_remote_child();
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_view_id,
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "remote-child guard must skip the RPC for token-assigned events"
        );
    });
}

#[test]
fn conversation_server_token_assigned_skips_without_task_id() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false, false);
        conversation.set_server_conversation_token("server-conversation-id".to_string());
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let (_model, counter) = install_model_with_call_counter(&mut app);

        history_model.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_view_id,
            });
        });

        pump_spawned_tasks().await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "missing task_id must skip the RPC for token-assigned events"
        );
    });
}
