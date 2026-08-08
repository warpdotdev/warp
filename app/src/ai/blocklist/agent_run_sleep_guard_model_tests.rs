use std::time::Duration;

use instant::Instant;
use warpui::r#async::Timer;
use warpui::{App, EntityId, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn held_guard_count(app: &App) -> usize {
    AgentRunSleepGuardModel::handle(app).read(app, |model, _| model.held_guard_count())
}

#[test]
fn agent_run_sleep_guard_model_lifecycle_uses_history_events() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_surface_id = EntityId::new();
        let history = BlocklistAIHistoryModel::handle(&app);
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_surface_id, false, false, false, ctx)
        });

        // Starting a conversation and every subsequent status transition use the
        // production history event path consumed by AgentRunSleepGuardModel.
        history.update(&mut app, |history, ctx| {
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });
        assert_eq!(held_guard_count(&app), 1);

        history.update(&mut app, |history, ctx| {
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::TransientError,
                ctx,
            );
        });
        assert_eq!(held_guard_count(&app), 1);

        for status in [
            ConversationStatus::Success,
            ConversationStatus::Error,
            ConversationStatus::Cancelled,
            ConversationStatus::WaitingForEvents,
            ConversationStatus::Blocked {
                blocked_action: "approval".to_string(),
            },
        ] {
            history.update(&mut app, |history, ctx| {
                history.update_conversation_status(
                    terminal_surface_id,
                    conversation_id,
                    status.clone(),
                    ctx,
                );
            });
            assert_eq!(held_guard_count(&app), 0);

            history.update(&mut app, |history, ctx| {
                history.update_conversation_status(
                    terminal_surface_id,
                    conversation_id,
                    ConversationStatus::InProgress,
                    ctx,
                );
            });
            assert_eq!(held_guard_count(&app), 1);
        }
    });
}

#[test]
fn agent_run_sleep_guard_model_releases_on_history_cleanup_events() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_surface_id = EntityId::new();
        let history = BlocklistAIHistoryModel::handle(&app);

        let cleared_conversation_id = history.update(&mut app, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            conversation_id
        });
        assert_eq!(held_guard_count(&app), 1);
        history.update(&mut app, |history, ctx| {
            history.clear_conversations_for_terminal_surface(terminal_surface_id, ctx);
        });
        assert_eq!(held_guard_count(&app), 0);

        let removed_conversation_id = history.update(&mut app, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            conversation_id
        });
        assert_eq!(held_guard_count(&app), 1);
        history.update(&mut app, |history, ctx| {
            history.remove_conversation(removed_conversation_id, terminal_surface_id, ctx);
        });
        assert_eq!(held_guard_count(&app), 0);

        let deleted_conversation_id = history.update(&mut app, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            conversation_id
        });
        assert_eq!(held_guard_count(&app), 1);
        history.update(&mut app, |history, ctx| {
            history.delete_conversation(deleted_conversation_id, Some(terminal_surface_id), ctx);
        });
        assert_eq!(held_guard_count(&app), 0);

        // Keep the IDs live in the test body so each cleanup assertion remains
        // tied to a concrete production event path.
        assert_ne!(cleared_conversation_id, removed_conversation_id);
        assert_ne!(removed_conversation_id, deleted_conversation_id);
    });
}

#[test]
fn agent_run_sleep_guard_model_cap_expiry_records_telemetry_and_reacquires() {
    warpui::telemetry::flush_events();

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let start = Instant::now();
        AgentRunSleepGuardModel::handle(&app).update(&mut app, |model, _| {
            model.set_now_for_test(start);
        });
        let terminal_surface_id = EntityId::new();
        let history = BlocklistAIHistoryModel::handle(&app);
        let conversation_id = history.update(&mut app, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            conversation_id
        });
        assert_eq!(held_guard_count(&app), 1);

        // Advance the test clock beyond the production deadline without any refresh.
        let expiry_time = start + AGENT_RUN_SLEEP_GUARD_CAP + Duration::from_secs(1);
        AgentRunSleepGuardModel::handle(&app).update(&mut app, |model, ctx| {
            model.set_now_for_test(expiry_time);
            model.expire_for_test(expiry_time, ctx);
        });
        assert_eq!(held_guard_count(&app), 0);

        // `send_telemetry_from_ctx!` records asynchronously on the app executor. Poll the
        // queue with a bounded wait so this remains reliable when the executor is under load.
        let expiry_event = {
            let mut expiry_event = None;
            for _ in 0..20 {
                expiry_event = warpui::telemetry::flush_events().into_iter().find(|event| {
                    match &event.payload {
                        warpui::telemetry::EventPayload::NamedEvent { name, .. } => {
                            name == "AgentMode.SleepGuardCapExpired"
                        }
                        _ => false,
                    }
                });
                if expiry_event.is_some() {
                    break;
                }
                Timer::after(Duration::from_millis(10)).await;
            }
            expiry_event.expect("cap expiry should record telemetry")
        };
        assert!(matches!(
            expiry_event.payload,
            warpui::telemetry::EventPayload::NamedEvent { .. }
        ));

        // A subsequent real history status event re-acquires protection while
        // the conversation remains InProgress.
        history.update(&mut app, |history, ctx| {
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });
        assert_eq!(held_guard_count(&app), 1);
    });
}

#[test]
fn agent_run_sleep_guard_model_refresh_keeps_active_conversation_awake_past_cap() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let start = Instant::now();
        AgentRunSleepGuardModel::handle(&app).update(&mut app, |model, _| {
            model.set_now_for_test(start);
        });
        let terminal_surface_id = EntityId::new();
        let history = BlocklistAIHistoryModel::handle(&app);
        let conversation_id = history.update(&mut app, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(terminal_surface_id, false, false, false, ctx);
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            conversation_id
        });
        assert_eq!(held_guard_count(&app), 1);

        // Invoke the production refresh entry point at intervals shorter than the cap. Each
        // refresh occurs after the original deadline but before the previous refreshed deadline.
        let refresh_interval = AGENT_RUN_SLEEP_GUARD_CAP - Duration::from_secs(1);
        let mut now = start;
        for _ in 0..3 {
            now += refresh_interval;
            AgentRunSleepGuardModel::handle(&app).update(&mut app, |model, ctx| {
                model.set_now_for_test(now);
                model.refresh(conversation_id, ctx);
                model.expire_for_test(now, ctx);
            });
            assert_eq!(held_guard_count(&app), 1);
        }
        assert!(now > start + AGENT_RUN_SLEEP_GUARD_CAP);
        assert!(history.read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .is_some_and(|conversation| {
                    matches!(conversation.status(), ConversationStatus::InProgress)
                })
        }));
    });
}
