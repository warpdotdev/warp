//! Tests for [`OrchestrationChildTracker`]'s state machine.
//!
//! Each case drives `observe_child` inside a real
//! `ModelContext<OrchestrationEventStreamer>` so the pill-bar broadcasts have
//! somewhere to land, then asserts on the tracker's own bookkeeping. Metadata
//! fetches are counted rather than issued, so no history or network plumbing
//! is required.

use std::collections::HashSet;
use std::sync::Arc;

use warp_multi_agent_api as api;
use warpui::{App, ModelContext, ModelHandle};

use super::*;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, MockAIClient};

const PARENT_RUN_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_A_RUN_ID: &str = "22222222-2222-2222-2222-222222222222";
const SESSION_A: &str = "44444444-4444-4444-4444-444444444444";

fn task_id(s: &str) -> AmbientAgentTaskId {
    s.parse().expect("hardcoded task id parses")
}

fn child_task_id() -> AmbientAgentTaskId {
    task_id(CHILD_A_RUN_ID)
}

/// No run has been killed locally, so the tombstone gate never fires.
fn no_killed_runs() -> HashSet<String> {
    HashSet::new()
}

/// Installs the singletons the streamer depends on and returns its handle.
fn install_streamer(app: &mut App) -> ModelHandle<OrchestrationEventStreamer> {
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
    let server_api = ServerApiProvider::new_for_test().get();
    app.add_singleton_model(|ctx| {
        OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
    })
}

/// Runs `test` against a fresh tracker for `PARENT_RUN_ID` inside a streamer
/// model context.
fn with_tracker(
    test: impl FnOnce(&mut OrchestrationChildTracker, &mut ModelContext<OrchestrationEventStreamer>)
    + 'static,
) {
    App::test((), |mut app| async move {
        let streamer = install_streamer(&mut app);
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = OrchestrationChildTracker::new(task_id(PARENT_RUN_ID));
            test(&mut tracker, ctx);
        });
    });
}

/// Builds a child task row for `ChildSignal::Seeded`, parented under
/// `PARENT_RUN_ID` so it is treated as a child rather than the parent's own
/// row.
fn seed_row(task_id: AmbientAgentTaskId) -> Box<AmbientAgentTask> {
    use chrono::Utc;
    Box::new(AmbientAgentTask {
        task_id,
        parent_run_id: Some(PARENT_RUN_ID.to_string()),
        title: "child".to_string(),
        state: AmbientAgentTaskState::InProgress,
        prompt: "prompt".to_string(),
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        updated_at: Utc::now(),
        run_time: None,
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        agent_config_snapshot: None,
        artifacts: vec![],
        is_sandbox_running: false,
        last_event_sequence: None,
        children: vec![],
        debug_agent_available: false,
        scope: None,
    })
}

#[test]
fn started_tracks_child_before_metadata_arrives() {
    // The placeholder must exist as soon as discovery fires so signals that
    // race ahead of the metadata fetch find an entry to update.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("discovery tracks the child immediately");
        assert!(child.is_remote_child);
        assert_eq!(child.session_id, None);
        assert!(tracker.children_awaiting_metadata.contains(CHILD_A_RUN_ID));
    });
}

#[test]
fn repeated_started_signals_share_one_metadata_fetch() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        assert_eq!(tracker.children.len(), 1);
        assert_eq!(
            tracker.metadata_fetch_dispatch_count, 1,
            "a repeat discovery must not dispatch another fetch"
        );
    });
}

#[test]
fn lifecycle_discovers_a_child_that_was_never_announced() {
    // Lifecycle is the backstop for a missed or reordered discovery event, so
    // it must produce the same placeholder discovery would have.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Lifecycle(api::LifecycleEventType::InProgress),
            &no_killed_runs(),
            ctx,
        );

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("lifecycle backfills an unannounced child");
        assert!(child.is_remote_child);
    });
}

#[test]
fn lifecycle_then_started_does_not_duplicate_discovery() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Lifecycle(api::LifecycleEventType::InProgress),
            &no_killed_runs(),
            ctx,
        );
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        assert_eq!(tracker.children.len(), 1);
        assert_eq!(
            tracker.metadata_fetch_dispatch_count, 1,
            "reordered lifecycle and discovery signals share one fetch"
        );
    });
}

#[test]
fn signals_for_a_killed_run_are_dropped() {
    // A run killed locally must not be resurrected by server events still in
    // flight.
    with_tracker(|tracker, ctx| {
        let killed = HashSet::from([CHILD_A_RUN_ID.to_string()]);

        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Lifecycle(api::LifecycleEventType::InProgress),
            &killed,
            ctx,
        );

        assert!(tracker.children.is_empty());
        assert!(tracker.children_awaiting_metadata.is_empty());
        assert_eq!(tracker.metadata_fetch_dispatch_count, 0);
    });
}

#[test]
fn a_killed_run_forgets_state_recorded_before_the_kill() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);
        let killed = HashSet::from([CHILD_A_RUN_ID.to_string()]);

        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &killed, ctx);

        assert!(tracker.children.is_empty());
        assert!(tracker.children_by_run_id.is_empty());
        assert!(tracker.children_awaiting_metadata.is_empty());
    });
}

#[test]
fn a_malformed_run_id_is_dropped() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            "not-a-task-id",
            ChildSignal::Started,
            &no_killed_runs(),
            ctx,
        );

        assert!(tracker.children.is_empty());
        assert!(tracker.children_awaiting_metadata.is_empty());
    });
}

#[test]
fn a_registered_child_is_tracked_without_a_fetch() {
    // An in-band child already owns a local conversation and is hydrated by
    // its executor, so the tracker observes it for status only.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Registered,
            &no_killed_runs(),
            ctx,
        );

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("a registered child is tracked immediately");
        assert!(!child.is_remote_child);
        assert!(tracker.in_band_children.contains(&child_task_id()));
        assert_eq!(tracker.metadata_fetch_dispatch_count, 0);
    });
}

#[test]
fn discovery_of_a_registered_child_does_not_fetch() {
    // The server announces in-band children too; that echo must not turn into
    // a redundant hydration round-trip.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Registered,
            &no_killed_runs(),
            ctx,
        );

        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        assert_eq!(tracker.metadata_fetch_dispatch_count, 0);
        assert!(
            !tracker
                .children
                .get(&child_task_id())
                .expect("the registered entry survives discovery")
                .is_remote_child
        );
    });
}

#[test]
fn session_linked_fills_the_session_id_and_materializes_the_pane() {
    // The session UUID arrives on the wire, so the live pane can open without
    // waiting for a metadata round-trip.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);
        let fetches_before_link = tracker.metadata_fetch_dispatch_count;

        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::SessionLinked {
                session_uuid: SESSION_A.to_string(),
            },
            &no_killed_runs(),
            ctx,
        );

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("the child is tracked");
        assert_eq!(child.session_id, Some(SESSION_A.parse().unwrap()));
        assert_eq!(
            tracker.metadata_fetch_dispatch_count, fetches_before_link,
            "a linked session needs no metadata fetch"
        );
    });
}

#[test]
fn a_session_link_that_precedes_discovery_is_applied_on_insert() {
    // `run_session_linked` can beat `child_agent_started` on the wire; the
    // session id must survive until the placeholder exists.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::SessionLinked {
                session_uuid: SESSION_A.to_string(),
            },
            &no_killed_runs(),
            ctx,
        );
        assert!(tracker.children.is_empty());

        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("discovery creates the child");
        assert_eq!(child.session_id, Some(SESSION_A.parse().unwrap()));
        assert!(tracker.pending_session_ids.is_empty());
    });
}

#[test]
fn a_malformed_session_uuid_is_dropped() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);

        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::SessionLinked {
                session_uuid: "not-a-session".to_string(),
            },
            &no_killed_runs(),
            ctx,
        );

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("the child is tracked");
        assert_eq!(child.session_id, None);
    });
}

#[test]
fn a_seeded_child_is_tracked_as_a_remote_placeholder() {
    // Seed rows describe runs owned by another process, so they materialize
    // the same remote placeholder discovery produces.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Seeded(seed_row(child_task_id())),
            &no_killed_runs(),
            ctx,
        );

        let child = tracker
            .children
            .get(&child_task_id())
            .expect("a seeded child is tracked immediately");
        assert!(child.is_remote_child);
        assert_eq!(child.last_state, Some(AmbientAgentTaskState::InProgress));
    });
}

#[test]
fn a_seed_row_clears_an_outstanding_fetch() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_A_RUN_ID, ChildSignal::Started, &no_killed_runs(), ctx);
        assert!(tracker.children_awaiting_metadata.contains(CHILD_A_RUN_ID));

        tracker.observe_child(
            CHILD_A_RUN_ID,
            ChildSignal::Seeded(seed_row(child_task_id())),
            &no_killed_runs(),
            ctx,
        );

        assert!(
            tracker.children_awaiting_metadata.is_empty(),
            "seed data resolves the hydration the fetch was after"
        );
    });
}

#[test]
fn the_parents_own_seed_row_is_not_tracked_as_a_child() {
    // The ancestor endpoint returns the parent alongside its children.
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            PARENT_RUN_ID,
            ChildSignal::Seeded(seed_row(task_id(PARENT_RUN_ID))),
            &no_killed_runs(),
            ctx,
        );

        assert!(tracker.children.is_empty());
    });
}
