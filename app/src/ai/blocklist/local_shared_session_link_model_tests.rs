use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use session_sharing_protocol::common::SessionId;
use warpui::App;

use super::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel, LocalSharedSessionLinkModel};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::{AIClient, MockAIClient};

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
    warpui::ModelHandle<LocalSharedSessionLinkModel>,
    Arc<AtomicUsize>,
) {
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
        LocalSharedSessionLinkModel::new_with_ai_client_for_test(ai_client, ctx)
    });
    (model, counter)
}

#[test]
fn local_shared_session_established_fires_update_agent_task_with_session_id() {
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
fn local_shared_session_established_uses_correct_argument_order() {
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
            LocalSharedSessionLinkModel::new_with_ai_client_for_test(ai_client, ctx)
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
fn local_shared_session_established_skips_viewer_conversations() {
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
fn local_shared_session_established_skips_remote_child_conversations() {
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
fn local_shared_session_established_skips_when_task_id_missing() {
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
fn local_shared_session_established_skips_unknown_conversation() {
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
