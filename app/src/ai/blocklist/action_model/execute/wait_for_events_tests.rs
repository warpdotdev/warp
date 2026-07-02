//! Unit tests for the pure helpers in `wait_for_events`, plus an App-based
//! test of the executor's parent-registration wiring.

use std::sync::Arc;
use std::time::Duration;

use warp_core::features::FeatureFlag;
use warpui::{App, EntityId};

use super::{
    watchdog_timeout_for_stamped_seconds, AnyActionExecution, ExecuteActionInput,
    WaitForEventsExecutor, CLIENT_WATCHDOG_SAFETY_MARGIN,
    DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS, HARD_FLOOR,
};
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ai::{AIClient, MockAIClient};
use crate::server::server_api::ServerApiProvider;

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
fn execute_invokes_parent_registration_and_honors_child_short_circuit() {
    // `execute()` must route into the orchestration streamer behind the flag.
    // For a child conversation (has_parent_agent), the streamer short-circuits
    // without a server fetch (asserted via the mock's times(0) expectation),
    // and the wait still flips the conversation into WaitingForEvents.
    App::test((), |mut app| async move {
        let _flag_guard = FeatureFlag::WaitForEventsParentRegistration.override_enabled(true);

        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // A streamer whose server fetch must never be called: the child
        // short-circuit precedes any `get_ambient_agent_task` call.
        let mut mock = MockAIClient::new();
        mock.expect_get_ambient_agent_task().times(0);
        let ai_client: Arc<dyn AIClient> = Arc::new(mock);
        let server_api = ServerApiProvider::new_for_test().get();
        // Held for the lifetime of the test so the mock's times(0) expectation
        // is verified on drop; resolved internally by `execute()` via
        // `OrchestrationEventStreamer::handle`.
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

        history_model.read(&app, |model, _| {
            assert!(
                matches!(
                    model.conversation(&conversation_id).map(|c| c.status()),
                    Some(ConversationStatus::WaitingForEvents)
                ),
                "execute() must flip the conversation into WaitingForEvents"
            );
        });
        // The child short-circuit is asserted by the mock's times(0)
        // expectation, verified when `_streamer` drops at test teardown:
        // a child conversation must never trigger a `get_ambient_agent_task`
        // fetch.
    });
}
