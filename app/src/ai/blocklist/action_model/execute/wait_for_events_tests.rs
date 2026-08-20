//! Unit tests for the pure helpers in `wait_for_events`, plus an App-based
//! test of the executor's parent-registration wiring.

use std::sync::Arc;
use std::time::Duration;

use warp_core::features::FeatureFlag;
use warpui::{App, EntityId};

use super::{
    AnyActionExecution, CLIENT_WATCHDOG_SAFETY_MARGIN, DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS,
    ExecuteActionInput, HARD_FLOOR, WaitForEventsExecutor, WaitForEventsExecutorEvent,
    watchdog_timeout_for_stamped_seconds,
};
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, MockAIClient};

#[test]
fn watchdog_timeout_constants_match_documented_values() {
    // The behavioural tests below assert the contract; this trips if
    // someone moves a constant without updating the documented intent.
    assert_eq!(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS, 30 * 60);
    assert_eq!(CLIENT_WATCHDOG_SAFETY_MARGIN, Duration::from_secs(30));
    assert_eq!(HARD_FLOOR, Duration::from_secs(5));
}

#[test]
fn watchdog_timeout_subtracts_margin_for_stamped_minute() {
    // A 60s stamped timeout has 30s of headroom after subtracting the
    // safety margin — that's the canonical "happy path" the safety
    // margin is designed for.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(60),
        Duration::from_secs(30)
    );
}

#[test]
fn watchdog_timeout_clamps_to_hard_floor_when_stamped_value_is_too_small() {
    // A 10s stamped timeout would become negative after subtracting the
    // 30s safety margin — the hard floor kicks in so the watchdog still
    // fires after a finite delay.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(10),
        HARD_FLOOR,
        "stamped 10s should clamp to HARD_FLOOR after subtracting the safety margin"
    );
}

#[test]
fn watchdog_timeout_falls_back_to_default_minus_margin_when_unset() {
    // Prost flattens scalars, so the proto's "unset" looks like `0` on
    // the Rust side; treat that as "use the default minus margin".
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(0), expected);
}

#[test]
fn watchdog_timeout_clamps_negative_value_to_default_minus_margin() {
    // Defense against a buggy or malicious payload. `Duration::from_secs`
    // takes a `u64`; a negative value would underflow without the clamp.
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(-42), expected);
}

#[test]
fn watchdog_timeout_preserves_large_stamped_value() {
    // Server-supplied values well above the margin pass through as
    // (stamped - margin). 15 minutes stays at 14m30s after the
    // subtraction.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(900),
        Duration::from_secs(900) - CLIENT_WATCHDOG_SAFETY_MARGIN
    );
}

#[test]
fn execute_invokes_parent_registration_for_child_conversations() {
    // `execute()` must route into the orchestration streamer behind the flag.
    // A child conversation is eligible for wait-time parent registration —
    // with multi-level orchestration a mid-tree node may have children of
    // its own — so the streamer issues a `get_ambient_agent_task` fetch,
    // and the wait still flips the conversation into WaitingForEvents.
    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::WaitForEventsParentRegistration.override_enabled(true);

        let terminal_view_id = EntityId::new();
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));

        // The registration fetch must be issued for the child (the old code
        // short-circuited children entirely, i.e. zero fetches). At least
        // once, not exactly once: the Err return below can also be refetched
        // by the streamer's restore-retry loop, and whether a retry lands
        // before test teardown is a platform timing race.
        let mut mock = MockAIClient::new();
        mock.expect_get_ambient_agent_task()
            .times(1..)
            .returning(|_| Err(anyhow::anyhow!("fetch observed")));
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();
        // Held for the lifetime of the test so the mock's times(1..)
        // expectation is verified on drop; resolved internally by `execute()`
        // via `OrchestrationEventStreamer::handle`.
        let _streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });

        let executor = app.add_model(|ctx| WaitForEventsExecutor::new(terminal_view_id, ctx));

        // Child conversation: own run_id plus a parent_agent_id.
        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440530".to_string());
        conversation.set_parent_agent_id("550e8400-e29b-41d4-a716-4466554405fc".to_string());
        let conversation_id = conversation.id();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });

        let action = AIAgentAction {
            id: AIAgentActionId::from("wait-action".to_string()),
            action: AIAgentActionType::WaitForEvents {
                tool_call_id: "tool-call-1".to_string(),
                idle_timeout_seconds: 600,
            },
            task_id: TaskId::new("wait-task".to_string()),
            requires_result: false,
        };

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });
        assert!(
            matches!(execution, AnyActionExecution::Async { .. }),
            "WaitForEvents should yield an async execution"
        );

        // Drive the spawned registration fetch so the mock observes it; the
        // times(1..) expectation is verified when `_streamer` drops at test
        // teardown.
        for _ in 0..3 {
            futures_lite::future::yield_now().await;
        }

        history_model.read(&app, |model, _| {
            assert!(
                matches!(
                    model.conversation(&conversation_id).map(|c| c.status()),
                    Some(ConversationStatus::WaitingForEvents)
                ),
                "execute() must flip the conversation into WaitingForEvents"
            );
        });
    });
}

/// Registers the singleton models `execute()` unconditionally reaches into
/// (`BlocklistAIHistoryModel`, `OrchestrationEventStreamer`), creates a
/// `WaitingForEvents` conversation, and returns its id plus the terminal
/// surface id. `WaitForEventsParentRegistration` is left disabled, so the
/// streamer singleton only needs to exist — it never issues a fetch.
fn setup_waiting_conversation(
    app: &mut App,
) -> (EntityId, crate::ai::agent::conversation::AIConversationId) {
    let terminal_view_id = EntityId::new();
    let history_model =
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
    let server_api = ServerApiProvider::new_for_test().get();
    app.add_singleton_model(|ctx| {
        OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
    });

    let conversation = AIConversation::new(false, false);
    let conversation_id = conversation.id();
    history_model.update(app, |model, ctx| {
        model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        model.update_conversation_status(
            terminal_view_id,
            conversation_id,
            ConversationStatus::WaitingForEvents,
            ctx,
        );
    });
    (terminal_view_id, conversation_id)
}

/// Sets up a `WaitForEventsExecutor` with one accepted wait for `conversation_id`,
/// returning the executor and its generation-1 tool_call_id so callers can invoke
/// `fire_watchdog_if_current` directly instead of waiting out the real timer.
fn executor_with_accepted_wait(
    app: &mut App,
    terminal_view_id: EntityId,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
    idle_timeout_seconds: i32,
) -> (warpui::ModelHandle<WaitForEventsExecutor>, String) {
    let tool_call_id = "tool-call-1".to_string();
    let executor = app.add_model(|ctx| WaitForEventsExecutor::new(terminal_view_id, ctx));
    let action = AIAgentAction {
        id: AIAgentActionId::from("wait-action".to_string()),
        action: AIAgentActionType::WaitForEvents {
            tool_call_id: tool_call_id.clone(),
            idle_timeout_seconds,
        },
        task_id: TaskId::new("wait-task".to_string()),
        requires_result: false,
    };
    executor.update(app, |executor, ctx| {
        let input = ExecuteActionInput {
            action: &action,
            conversation_id,
        };
        let _: AnyActionExecution = executor.execute(input, ctx).into();
    });
    (executor, tool_call_id)
}

#[test]
fn watchdog_expiry_emits_warm_wait_window_expired_and_preserves_pending_entry_when_flag_enabled() {
    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::HibernateOnFirstWaitTimeout.override_enabled(true);

        let (terminal_view_id, conversation_id) = setup_waiting_conversation(&mut app);
        let (executor, tool_call_id) =
            executor_with_accepted_wait(&mut app, terminal_view_id, conversation_id, 1800);

        let events: std::rc::Rc<std::cell::RefCell<Vec<WaitForEventsExecutorEvent>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_clone = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &executor,
                move |_, event: &WaitForEventsExecutorEvent, _| {
                    events_clone.borrow_mut().push(event.clone());
                },
            );
        });

        // Fire the watchdog directly for generation 1 (the only `execute()` call
        // above), instead of waiting out the real 1770s timer.
        executor.update(&mut app, |executor, ctx| {
            executor.fire_watchdog_if_current(conversation_id, &tool_call_id, 1, ctx);
        });

        let recorded = events.borrow();
        assert_eq!(recorded.len(), 1, "expected exactly one emitted event");
        match &recorded[0] {
            WaitForEventsExecutorEvent::WarmWaitWindowExpired {
                conversation_id: event_conversation_id,
                tool_call_id: event_tool_call_id,
                server_idle_timeout_seconds,
                used_fallback,
                resolved_watchdog,
            } => {
                assert_eq!(*event_conversation_id, conversation_id);
                assert_eq!(event_tool_call_id, &tool_call_id);
                assert_eq!(*server_idle_timeout_seconds, 1800);
                assert!(
                    !used_fallback,
                    "1800 is a valid server stamp, not a fallback"
                );
                assert_eq!(
                    *resolved_watchdog,
                    watchdog_timeout_for_stamped_seconds(1800)
                );
            }
        }

        // The pending wait must remain registered until the action model
        // yields it via `take_pending_wait`.
        executor.read(&app, |executor, _| {
            assert!(
                executor.pending.contains_key(&conversation_id),
                "pending entry must survive the watchdog firing while awaiting yield"
            );
        });
    });
}

#[test]
fn watchdog_expiry_completes_directly_when_flag_disabled() {
    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::HibernateOnFirstWaitTimeout.override_enabled(false);

        let (terminal_view_id, conversation_id) = setup_waiting_conversation(&mut app);
        let (executor, tool_call_id) =
            executor_with_accepted_wait(&mut app, terminal_view_id, conversation_id, 1800);

        let events: std::rc::Rc<std::cell::RefCell<Vec<WaitForEventsExecutorEvent>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_clone = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &executor,
                move |_, event: &WaitForEventsExecutorEvent, _| {
                    events_clone.borrow_mut().push(event.clone());
                },
            );
        });

        executor.update(&mut app, |executor, ctx| {
            executor.fire_watchdog_if_current(conversation_id, &tool_call_id, 1, ctx);
        });

        assert!(
            events.borrow().is_empty(),
            "the old rewait path must not emit WarmWaitWindowExpired"
        );
        executor.read(&app, |executor, _| {
            assert!(
                !executor.pending.contains_key(&conversation_id),
                "the old rewait path completes and removes the pending entry immediately"
            );
        });
    });
}

#[test]
fn take_pending_wait_removes_the_entry_after_warm_expiry() {
    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::HibernateOnFirstWaitTimeout.override_enabled(true);

        let (terminal_view_id, conversation_id) = setup_waiting_conversation(&mut app);
        let (executor, tool_call_id) =
            executor_with_accepted_wait(&mut app, terminal_view_id, conversation_id, 0);

        executor.update(&mut app, |executor, ctx| {
            executor.fire_watchdog_if_current(conversation_id, &tool_call_id, 1, ctx);
        });
        executor.read(&app, |executor, _| {
            assert!(executor.pending.contains_key(&conversation_id));
        });

        executor.update(&mut app, |executor, _| {
            executor.take_pending_wait(conversation_id);
        });
        executor.read(&app, |executor, _| {
            assert!(
                !executor.pending.contains_key(&conversation_id),
                "take_pending_wait must remove the pending entry"
            );
        });
    });
}

#[test]
fn warm_wait_window_expired_reports_used_fallback_when_server_stamp_is_absent() {
    // Fallback resolves to the same duration as an explicit 1800s stamp, but
    // telemetry must still distinguish the two so a regression that clears the
    // server value on the wire doesn't silently look identical.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(0),
        watchdog_timeout_for_stamped_seconds(1800),
    );

    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::HibernateOnFirstWaitTimeout.override_enabled(true);

        let (terminal_view_id, conversation_id) = setup_waiting_conversation(&mut app);

        // 0 means "unset" (prost flat-scalar convention), so the executor
        // must resolve this via the fallback path.
        let (executor, tool_call_id) =
            executor_with_accepted_wait(&mut app, terminal_view_id, conversation_id, 0);

        let events: std::rc::Rc<std::cell::RefCell<Vec<WaitForEventsExecutorEvent>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events_clone = events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(
                &executor,
                move |_, event: &WaitForEventsExecutorEvent, _| {
                    events_clone.borrow_mut().push(event.clone());
                },
            );
        });

        executor.update(&mut app, |executor, ctx| {
            executor.fire_watchdog_if_current(conversation_id, &tool_call_id, 1, ctx);
        });

        let recorded = events.borrow();
        match recorded.as_slice() {
            [
                WaitForEventsExecutorEvent::WarmWaitWindowExpired {
                    server_idle_timeout_seconds,
                    used_fallback,
                    ..
                },
            ] => {
                assert_eq!(*server_idle_timeout_seconds, 0);
                assert!(
                    *used_fallback,
                    "an unset stamp must report used_fallback=true"
                );
            }
            other => panic!("expected exactly one WarmWaitWindowExpired event, got {other:?}"),
        }
    });
}
