//! Integration test for the orchestration usage rollup (QUALITY-1703).
//!
//! Verifies that the usage summary footer aggregates "Diffs applied"
//! (along with "Files changed" and "Commands executed") across an
//! orchestrator and its locally-loaded child agents, with a per-agent
//! disclosure for "Diffs applied" that expands independently of the
//! existing credits breakdown.
//!
//! The orchestrator and its children are seeded directly via
//! [`TerminalView::seed_orchestration_rollup_usage_for_test`] rather than
//! driven through a live multi-agent run, since that would require a real
//! authenticated backend. This lets the test construct the rollup state
//! deterministically and capture it visually.

use std::collections::HashMap;
use std::time::Duration;

use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::terminal::view::DummyConversationUsage;
use warp_multi_agent_api as api;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::new_builder;
use crate::Builder;

const ORCHESTRATOR_TASK_ID: &str = "usage-rollup-orchestrator-task";
const ORCHESTRATOR_REQUEST_ID: &str = "usage-rollup-orchestrator-request";

fn orchestrator_usage() -> DummyConversationUsage {
    DummyConversationUsage {
        credits_spent: 4.0,
        files_changed: 1,
        lines_added: 5,
        lines_removed: 2,
        commands_executed: 3,
    }
}

/// Two children with distinct, non-zero usage on every rolled-up metric, so
/// the resulting per-agent diffs breakdown has more than one contributing
/// row (mirroring the reported bug: an orchestrator whose children did all
/// the editing).
fn child_agents() -> Vec<(String, DummyConversationUsage)> {
    vec![
        (
            "DesignBot".to_string(),
            DummyConversationUsage {
                credits_spent: 12.0,
                files_changed: 4,
                lines_added: 120,
                lines_removed: 34,
                commands_executed: 9,
            },
        ),
        (
            "ReviewBot".to_string(),
            DummyConversationUsage {
                credits_spent: 3.0,
                files_changed: 1,
                lines_added: 10,
                lines_removed: 1,
                commands_executed: 2,
            },
        ),
    ]
}

/// A minimal restored conversation (one user query, one agent text
/// response) whose AI block resolves a real `conversation_id` (unlike
/// `insert_dummy_ai_block`'s `FakeAIBlockModel`), so the usage button
/// actually renders.
fn orchestrator_conversation_data() -> api::ConversationData {
    api::ConversationData {
        tasks: vec![api::Task {
            id: ORCHESTRATOR_TASK_ID.to_string(),
            messages: vec![
                api::Message {
                    id: "usage-rollup-user-query".to_string(),
                    task_id: ORCHESTRATOR_TASK_ID.to_string(),
                    server_message_data: String::new(),
                    citations: vec![],
                    message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                        query: "Coordinate the child agents and report back.".to_string(),
                        context: None,
                        referenced_attachments: HashMap::new(),
                        mode: None,
                        intended_agent: Default::default(),
                    })),
                    request_id: ORCHESTRATOR_REQUEST_ID.to_string(),
                    timestamp: None,
                    fetched_memories: vec![],
                },
                api::Message {
                    id: "usage-rollup-agent-output".to_string(),
                    task_id: ORCHESTRATOR_TASK_ID.to_string(),
                    server_message_data: String::new(),
                    citations: vec![],
                    message: Some(api::message::Message::AgentOutput(
                        api::message::AgentOutput {
                            text: "Delegated the work to two child agents.".to_string(),
                        },
                    )),
                    request_id: ORCHESTRATOR_REQUEST_ID.to_string(),
                    timestamp: None,
                    fetched_memories: vec![],
                },
            ],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        ..Default::default()
    }
}

/// Manual-only: requires a real display to render, screenshot, and record
/// video of the usage footer. Run with:
/// `WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 cargo run -p integration --bin integration -- test_orchestration_usage_rollup_aggregates_diffs_applied`
///
/// Records the whole flow (expanding the "Diffs applied" per-agent
/// breakdown, then collapsing it again) to `recording.mp4` in the test's
/// artifacts directory. The collapsed and expanded stills are captured
/// immediately before recording starts and immediately after it stops,
/// respectively — an explicit frame-capture request racing the video
/// recorder's own concurrent capture loop is unreliable.
pub fn test_orchestration_usage_rollup_aggregates_diffs_applied() -> Builder {
    new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions(
                "Restore orchestrator AI block and seed child agent usage",
            )
            .with_action(|app, window_id, _| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.update(app, |view, ctx| {
                    view.load_conversation_from_tasks(orchestrator_conversation_data(), ctx);
                    view.seed_orchestration_rollup_usage_for_test(
                        orchestrator_usage(),
                        child_agents(),
                        ctx,
                    );
                });
            })
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    async_assert!(
                        view.last_ai_block().is_some(),
                        "Orchestrator AI block should exist"
                    )
                })
            }),
        )
        .with_step(
            // Captured before recording starts: an explicit frame-capture
            // request racing the video recorder's own concurrent capture
            // loop is unreliable (observed both a mistimed frame and an
            // outright capture timeout in earlier revisions of this test).
            TestStep::new("Open the usage summary footer and capture it collapsed")
                .with_click_on_saved_position("usage_footer:open_button")
                .set_timeout(Duration::from_secs(10))
                .set_post_step_pause(Duration::from_secs(1))
                .with_take_screenshot("usage_footer_rollup_collapsed.png"),
        )
        .with_step(
            TestStep::new("Start recording the usage footer rollup flow")
                .with_start_recording()
                .set_post_step_pause(Duration::from_millis(300)),
        )
        .with_step(
            TestStep::new("Expand the diffs-applied per-agent breakdown")
                .with_click_on_saved_position("usage_footer:diffs_details_toggle")
                .set_post_step_pause(Duration::from_millis(900)),
        )
        .with_step(
            TestStep::new("Collapse the diffs-applied per-agent breakdown")
                .with_click_on_saved_position("usage_footer:diffs_details_toggle")
                .set_post_step_pause(Duration::from_millis(900)),
        )
        .with_step(TestStep::new("Stop recording").with_stop_recording())
        .with_step(
            // Captured after recording stops, for the same reason the
            // collapsed capture happens before recording starts.
            TestStep::new("Re-expand the diffs-applied per-agent breakdown and capture it")
                .with_click_on_saved_position("usage_footer:diffs_details_toggle")
                .set_timeout(Duration::from_secs(10))
                .set_post_step_pause(Duration::from_secs(1))
                .with_take_screenshot("usage_footer_rollup_diffs_expanded.png"),
        )
}
