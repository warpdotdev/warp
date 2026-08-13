//! Unit tests for the [`OrchestrationChildTracker`] state machine.
//!
//! Every test drives `observe_child` against a real
//! `ModelContext<OrchestrationEventStreamer>` so the pill-bar broadcasts have
//! somewhere to go, then asserts on the tracker's own bookkeeping: the
//! model-backed side effects (history writes, network fetches) are compiled
//! out of test builds.

use std::collections::HashSet;
use std::sync::Arc;

use warp_multi_agent_api as api;
use warpui::App;

use super::*;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, MockAIClient};

const PARENT_RUN_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_RUN_ID: &str = "22222222-2222-2222-2222-222222222222";
const SESSION_UUID: &str = "44444444-4444-4444-4444-444444444444";

fn task_id(run_id: &str) -> AmbientAgentTaskId {
    run_id.parse().expect("hardcoded task id parses")
}

/// No runs have been killed locally.
fn no_kills() -> HashSet<String> {
    HashSet::new()
}

/// Runs `body` against a fresh tracker for `PARENT_RUN_ID`, with the
/// singletons `OrchestrationEventStreamer` depends on installed.
fn with_tracker<F>(body: F)
where
    F: FnOnce(&mut OrchestrationChildTracker, &mut ModelContext<OrchestrationEventStreamer>)
        + 'static,
{
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
        let ai_client: Arc<dyn AIClient> = Arc::new(MockAIClient::new());
        let server_api = ServerApiProvider::new_for_test().get();
        let streamer = app.add_singleton_model(|ctx| {
            OrchestrationEventStreamer::new_with_clients_for_test(ai_client, server_api, ctx)
        });
        streamer.update(&mut app, |_streamer, ctx| {
            let mut tracker = OrchestrationChildTracker::new(task_id(PARENT_RUN_ID));
            body(&mut tracker, ctx);
        });
    });
}

/// A child task row as a REST seed delivers it, parented under
/// `PARENT_RUN_ID` so it is not mistaken for the parent's own row.
fn child_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    use chrono::Utc;
    AmbientAgentTask {
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
    }
}

#[test]
fn started_dispatches_metadata_fetch_before_creating_placeholder() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Started, &no_kills(), ctx);

        assert!(
            tracker.is_awaiting_metadata(CHILD_RUN_ID),
            "discovery must mark the run as awaiting metadata"
        );
        assert!(
            tracker.children.is_empty(),
            "the placeholder is created only once the fetch resolves"
        );
    });
}

#[test]
fn repeated_started_signals_dispatch_one_metadata_fetch() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Started, &no_kills(), ctx);
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Started, &no_kills(), ctx);

        assert_eq!(
            tracker.metadata_fetch_dispatch_count(),
            1,
            "a repeat discovery signal must not dispatch a second fetch"
        );
    });
}

#[test]
fn signal_for_killed_run_has_no_effect() {
    with_tracker(|tracker, ctx| {
        let killed = HashSet::from([CHILD_RUN_ID.to_string()]);

        tracker.observe_child(
            CHILD_RUN_ID,
            ChildSignal::Lifecycle(api::LifecycleEventType::InProgress),
            &killed,
            ctx,
        );

        assert!(
            tracker.children.is_empty(),
            "a killed run must not be given a placeholder"
        );
        assert_eq!(
            tracker.metadata_fetch_dispatch_count(),
            0,
            "a killed run must not be fetched"
        );
    });
}

#[test]
fn signal_for_killed_run_forgets_an_existing_child() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Registered, &no_kills(), ctx);
        let killed = HashSet::from([CHILD_RUN_ID.to_string()]);

        tracker.observe_child(
            CHILD_RUN_ID,
            ChildSignal::Lifecycle(api::LifecycleEventType::Cancelled),
            &killed,
            ctx,
        );

        assert!(
            tracker.children.is_empty(),
            "a killed run must be dropped from the tracker"
        );
    });
}

#[test]
fn registered_child_is_tracked_in_band_without_a_discovery_fetch() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Registered, &no_kills(), ctx);

        let child = tracker
            .children
            .get(&task_id(CHILD_RUN_ID))
            .expect("a registered child is tracked immediately");
        assert!(
            !child.is_remote_child,
            "a child created in this process owns a real local conversation"
        );
        assert!(
            tracker.in_band_children.contains(&task_id(CHILD_RUN_ID)),
            "a registered child is marked in-band"
        );
        assert_eq!(
            tracker.metadata_fetch_dispatch_count(),
            0,
            "an in-band child needs no discovery fetch"
        );
    });
}

#[test]
fn started_after_registered_keeps_the_child_and_skips_discovery() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Registered, &no_kills(), ctx);

        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Started, &no_kills(), ctx);

        assert!(
            tracker.children.contains_key(&task_id(CHILD_RUN_ID)),
            "the registered child survives a later discovery signal"
        );
        assert_eq!(
            tracker.metadata_fetch_dispatch_count(),
            0,
            "an already-represented child must not be fetched"
        );
    });
}

#[test]
fn session_linked_records_the_session_id_without_a_metadata_fetch() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Registered, &no_kills(), ctx);

        tracker.observe_child(
            CHILD_RUN_ID,
            ChildSignal::SessionLinked {
                session_uuid: SESSION_UUID.to_string(),
            },
            &no_kills(),
            ctx,
        );

        let child = tracker
            .children
            .get(&task_id(CHILD_RUN_ID))
            .expect("child is tracked");
        assert_eq!(
            child.session_id,
            Some(SESSION_UUID.parse().expect("hardcoded session id parses")),
            "session id is recorded immediately from the session-linked event"
        );
        assert_eq!(
            tracker.metadata_fetch_dispatch_count(),
            0,
            "the session link carries everything the tracker needs"
        );
    });
}

#[test]
fn session_linked_before_the_child_exists_is_applied_on_creation() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_RUN_ID,
            ChildSignal::SessionLinked {
                session_uuid: SESSION_UUID.to_string(),
            },
            &no_kills(),
            ctx,
        );

        tracker.observe_child(CHILD_RUN_ID, ChildSignal::Registered, &no_kills(), ctx);

        let child = tracker
            .children
            .get(&task_id(CHILD_RUN_ID))
            .expect("child is tracked");
        assert_eq!(
            child.session_id,
            Some(SESSION_UUID.parse().expect("hardcoded session id parses")),
            "an early session link is stashed and applied when the child appears"
        );
    });
}

#[test]
fn seeded_child_is_marked_as_a_remote_child() {
    with_tracker(|tracker, ctx| {
        tracker.observe_child(
            CHILD_RUN_ID,
            ChildSignal::Seeded(Box::new(child_task(task_id(CHILD_RUN_ID)))),
            &no_kills(),
            ctx,
        );

        let child = tracker
            .children
            .get(&task_id(CHILD_RUN_ID))
            .expect("a seeded child is tracked immediately");
        assert!(
            child.is_remote_child,
            "a placeholder for a run hosted elsewhere is a remote child"
        );
    });
}
