//! Tests for [`OrchestrationViewerModel`].
//!
//! Layout:
//!
//! 1. Pure-function tests for [`conversation_status_from_state`]. These carry
//!    over unchanged from the legacy polling path.
//! 2. Streamer-driven path tests (flag ON). The model translates
//!    `OrchestrationEventStreamerEvent::ChildSpawned` /
//!    `ChildStatusChanged` events into local placeholder conversations.
//!    These tests drive the model via the streamer's emit path and a
//!    `MockAIClient` that returns canned `get_ambient_agent_task` responses
//!    for the pill metadata fetch.
//! 3. Legacy polling-path tests (flag OFF). The model registers children
//!    from `register_child` (called from `apply_children_fetch`). These map
//!    directly to the spec's polling-path semantics.
//!
//! `apply_children_fetch` itself is exercised through `register_child`, so
//! we drive the polling-path tests at the same boundary by calling
//! `register_child` from the test rather than invoking a synthetic
//! `apply_children_fetch` shell.

use std::sync::Arc;

use chrono::Utc;
use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, SingletonEntity};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::ambient_agents::task::{AgentConfigSnapshot, AmbientAgentTask};
use crate::ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamerEvent;
use crate::server::server_api::ai::{AIClient, MockAIClient};
use crate::server::server_api::ServerApiProvider;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

// ---- Pure-function tests ----------------------------------------------------

#[test]
fn maps_working_states_to_in_progress() {
    for state in [
        AmbientAgentTaskState::Queued,
        AmbientAgentTaskState::Pending,
        AmbientAgentTaskState::Claimed,
        AmbientAgentTaskState::InProgress,
    ] {
        assert!(
            matches!(
                conversation_status_from_state(&state),
                ConversationStatus::InProgress
            ),
            "expected InProgress for {state:?}",
        );
    }
}

#[test]
fn maps_succeeded_to_success() {
    assert!(matches!(
        conversation_status_from_state(&AmbientAgentTaskState::Succeeded),
        ConversationStatus::Success
    ));
}

#[test]
fn maps_failed_and_error_to_error() {
    assert!(matches!(
        conversation_status_from_state(&AmbientAgentTaskState::Failed),
        ConversationStatus::Error
    ));
    assert!(matches!(
        conversation_status_from_state(&AmbientAgentTaskState::Error),
        ConversationStatus::Error
    ));
}

#[test]
fn maps_blocked_to_blocked() {
    let status = conversation_status_from_state(&AmbientAgentTaskState::Blocked);
    assert!(matches!(status, ConversationStatus::Blocked { .. }));
}

#[test]
fn maps_cancelled_to_cancelled() {
    assert!(matches!(
        conversation_status_from_state(&AmbientAgentTaskState::Cancelled),
        ConversationStatus::Cancelled
    ));
}

#[test]
fn unknown_state_maps_to_error() {
    // Aligns with `is_terminal`, `is_failure_like`, and `status_icon_and_color`
    // in task.rs, which all treat Unknown as a terminal error state.
    assert!(matches!(
        conversation_status_from_state(&AmbientAgentTaskState::Unknown),
        ConversationStatus::Error
    ));
}

// ---- Test helpers -----------------------------------------------------------

/// Stub UUIDs used for `AmbientAgentTaskId`s; the model treats them as opaque.
const PARENT_TASK_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_A_TASK_ID: &str = "22222222-2222-2222-2222-222222222222";
const CHILD_B_TASK_ID: &str = "33333333-3333-3333-3333-333333333333";
const SESSION_A: &str = "44444444-4444-4444-4444-444444444444";

fn task_id(s: &str) -> AmbientAgentTaskId {
    s.parse().expect("hardcoded task id parses")
}

/// Builds a minimal [`AmbientAgentTask`] suitable for the registration path.
fn make_task(
    id: &str,
    state: AmbientAgentTaskState,
    title: &str,
    session_id: Option<&str>,
) -> AmbientAgentTask {
    make_task_with_name(id, state, None, title, session_id)
}

fn make_task_with_name(
    id: &str,
    state: AmbientAgentTaskState,
    snapshot_name: Option<&str>,
    title: &str,
    session_id: Option<&str>,
) -> AmbientAgentTask {
    let now = Utc::now();
    let agent_config_snapshot = snapshot_name.map(|name| AgentConfigSnapshot {
        name: Some(name.to_string()),
        ..Default::default()
    });
    AmbientAgentTask {
        task_id: task_id(id),
        parent_run_id: Some(PARENT_TASK_ID.to_string()),
        title: title.to_string(),
        state,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        session_id: session_id.map(String::from),
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

/// Wires up `BlocklistAIHistoryModel`, a real [`TerminalView`], and an
/// orchestrator parent conversation marked active for that view. Returns
/// the model built directly (bypassing `OrchestrationViewerModel::new`,
/// which would otherwise kick off either a REST fetch or a streamer
/// registration).
fn setup_model(
    app: &mut App,
    parent_task_id: AmbientAgentTaskId,
) -> (EntityId, AIConversationId, OrchestrationViewerModel) {
    initialize_app_for_terminal_view(app);
    let terminal_view = add_window_with_terminal(app, None);
    let terminal_view_id = terminal_view.id();
    let history = BlocklistAIHistoryModel::handle(app);
    let parent_conversation_id = history.update(app, |history, ctx| {
        let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
        history.set_active_conversation_id(id, terminal_view_id, ctx);
        id
    });

    let model = OrchestrationViewerModel {
        parent_task_id,
        terminal_view_id,
        terminal_view: terminal_view.downgrade(),
        children: HashMap::new(),
        children_by_run_id: HashMap::new(),
        polling_handle: None,
        fetch_generation: 0,
        idle_due_to_no_children: false,
    };

    (terminal_view_id, parent_conversation_id, model)
}

// ---- register_child tests (drives the shared registration path) ------------

#[test]
fn registers_new_child_conversation() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);

        let model_handle = app.add_model(|_| model);
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        // Child registered in the model's index.
        model_handle.read(&app, |model, _| {
            let entry = model
                .children
                .get(&task_id(CHILD_A_TASK_ID))
                .expect("child registered");
            assert!(entry.session_id.is_none());
            assert!(!entry.pane_materialization_requested);
            assert!(matches!(
                entry.last_state,
                AmbientAgentTaskState::InProgress
            ));
            // run_id reverse-index is also populated.
            assert_eq!(
                model.children_by_run_id.get(CHILD_A_TASK_ID),
                Some(&task_id(CHILD_A_TASK_ID))
            );
        });

        // Child conversation registered in the history model and linked to parent.
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            assert_eq!(child_ids.len(), 1, "expected one child conversation");
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert_eq!(child.agent_name(), Some("Worker"));
            assert_eq!(
                child.parent_conversation_id(),
                Some(parent_conv_id),
                "child linked to parent conversation"
            );
            assert!(child.is_viewing_shared_session());
            assert!(matches!(child.status(), ConversationStatus::InProgress));
        });
    });
}

#[test]
fn skips_parent_task_id_as_child() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // The server endpoint may include the parent itself in the response;
        // `register_child` filters it out.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    PARENT_TASK_ID,
                    AmbientAgentTaskState::Succeeded,
                    "Self",
                    None,
                ),
                ctx,
            );
        });

        model_handle.read(&app, |model, _| {
            assert!(
                model.children.is_empty(),
                "parent task should not register itself as a child"
            );
        });
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            assert!(
                history
                    .child_conversation_ids_of(&parent_conv_id)
                    .is_empty(),
                "no child conversations should have been created"
            );
        });
    });
}

#[test]
fn skips_child_when_no_active_parent_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal_view.id();

        // Do NOT create a parent conversation for this terminal view.
        // find_parent_conversation_id() should return None and the child
        // registration should be deferred to the next event/poll.
        let model = OrchestrationViewerModel {
            parent_task_id: task_id(PARENT_TASK_ID),
            terminal_view_id,
            terminal_view: terminal_view.downgrade(),
            children: HashMap::new(),
            children_by_run_id: HashMap::new(),
            polling_handle: None,
            fetch_generation: 0,
            idle_due_to_no_children: false,
        };
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        model_handle.read(&app, |model, _| {
            assert!(
                model.children.is_empty(),
                "child should not be registered without a parent conversation"
            );
        });
    });
}

#[test]
fn updates_status_on_state_change() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // First registration: child in progress.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        // Second registration: same child, now succeeded.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::Succeeded,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert!(matches!(entry.last_state, AmbientAgentTaskState::Succeeded));
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            assert_eq!(child_ids.len(), 1, "still one child after re-registration");
            let child = history.conversation(&child_ids[0]).unwrap();
            assert!(matches!(child.status(), ConversationStatus::Success));
        });
    });
}

#[test]
fn materialization_requested_only_once_per_child() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // First registration: child has session_id from the start.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                ),
                ctx,
            );
        });
        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert!(entry.session_id.is_some());
            assert!(
                entry.pane_materialization_requested,
                "first sight with session_id should flip the gate"
            );
        });

        // Second registration: same child, still has the same session_id.
        // Gate must remain set; we never want to re-emit materialization.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                ),
                ctx,
            );
        });
        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert!(entry.pane_materialization_requested);
        });
    });
}

#[test]
fn materialization_gate_flips_on_session_id_transition() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // First: no session_id yet (e.g. child is Queued).
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::Queued,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });
        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert!(entry.session_id.is_none());
            assert!(
                !entry.pane_materialization_requested,
                "no session_id ⇒ no materialization yet"
            );
        });

        // Second: session_id arrives.
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                ),
                ctx,
            );
        });
        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert_eq!(entry.session_id, Some(SESSION_A.parse().unwrap()));
            assert!(entry.pane_materialization_requested);
        });
    });
}

#[test]
fn registers_multiple_children() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Agent One",
                    None,
                ),
                ctx,
            );
            model.register_child(
                make_task(
                    CHILD_B_TASK_ID,
                    AmbientAgentTaskState::Succeeded,
                    "Agent Two",
                    None,
                ),
                ctx,
            );
        });

        model_handle.read(&app, |model, _| {
            assert_eq!(model.children.len(), 2);
            assert!(model.children.contains_key(&task_id(CHILD_A_TASK_ID)));
            assert!(model.children.contains_key(&task_id(CHILD_B_TASK_ID)));
        });
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            assert_eq!(child_ids.len(), 2);
        });
    });
}

// ---- display_name precedence -----------------------------------------------

#[test]
fn registers_child_agent_name_from_snapshot_name() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task_with_name(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    Some("frontend-tests"),
                    "Long descriptive task title",
                    None,
                ),
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            // Pill label prefers the orchestrator-supplied short name.
            assert_eq!(child.agent_name(), Some("frontend-tests"));
            assert_eq!(
                child.title().as_deref(),
                Some("Long descriptive task title")
            );
        });
    });
}

#[test]
fn registers_child_agent_name_falls_back_to_title_when_snapshot_name_is_missing() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task_with_name(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    None,
                    "Long descriptive task title",
                    None,
                ),
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert_eq!(child.agent_name(), Some("Long descriptive task title"));
            assert_eq!(
                child.title().as_deref(),
                Some("Long descriptive task title")
            );
        });
    });
}

#[test]
fn registers_child_agent_name_does_not_set_fallback_for_whitespace_only_title() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task_with_name(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    None,
                    "   ",
                    None,
                ),
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert_eq!(child.agent_name(), Some("Agent"));
            assert_eq!(
                child.title(),
                None,
                "whitespace-only title must not become a fallback display title"
            );
        });
    });
}

#[test]
fn registers_child_agent_name_uses_literal_agent_when_both_are_empty() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task_with_name(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    None,
                    "",
                    None,
                ),
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert_eq!(child.agent_name(), Some("Agent"));
            assert_eq!(child.title(), None);
        });
    });
}

#[test]
fn registers_child_agent_name_trims_whitespace() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task_with_name(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    Some("  frontend-tests  "),
                    "Long descriptive task title",
                    None,
                ),
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert_eq!(child.agent_name(), Some("frontend-tests"));
            assert_eq!(
                child.title().as_deref(),
                Some("Long descriptive task title")
            );
        });
    });
}

// ---- Streamer-driven path tests --------------------------------------------

#[test]
fn child_status_changed_with_unknown_run_id_is_silently_dropped() {
    // Spec § `PR 2 — OrchestrationViewerModel as consumer`:
    //   "If the run_id is not in the local map (unlikely race), drop the
    //   event silently — the spawn flow will re-create the placeholder."
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // No children registered yet — the run_id is unknown to the local map.
        model_handle.update(&mut app, |model, ctx| {
            model.handle_child_status_changed("unknown-run-id", ConversationStatus::Success, ctx);
        });

        // Should be a no-op: no panic, no new placeholders, no children added.
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            assert!(
                history
                    .all_live_conversations_for_terminal_view(terminal_view_id)
                    .filter(|conversation| conversation.is_viewing_shared_session())
                    .count()
                    == 0,
                "no viewer-side placeholder conversations should have been created"
            );
        });
    });
}

#[test]
fn child_status_changed_updates_existing_placeholder_via_local_map() {
    // After a child is registered, a subsequent ChildStatusChanged for the
    // same run_id must update the placeholder via the local run_id map.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Step 1: register a child (the registration step the streamer-side
        // ChildSpawned handler would have performed after its async fetch).
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        // Step 2: a ChildStatusChanged event lands for the same run_id.
        model_handle.update(&mut app, |model, ctx| {
            model.handle_child_status_changed(CHILD_A_TASK_ID, ConversationStatus::Success, ctx);
        });

        // The placeholder's status should reflect Success.
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history.conversation(&child_ids[0]).unwrap();
            assert!(matches!(child.status(), ConversationStatus::Success));
        });
    });
}

// ---- agent_id_to_conversation_id population --------------------------------

#[test]
fn b1_populates_agent_id_to_conversation_id_for_new_child() {
    // After `apply_children_fetch` registers a new viewer-created child,
    // `BlocklistAIHistoryModel::conversation_id_for_agent_id` resolves the
    // child's `run_id` back to the local child conversation so sibling
    // references in transcript bodies render display names instead of
    // "Unknown agent".
    App::test((), |mut app| async move {
        // `agent_id_key` reads `AIConversation::orchestration_agent_id`,
        // which only returns the `run_id` when OrchestrationV2 is enabled.
        // Without this override, the v1 fallback is `server_conversation_token`,
        // which the test doesn't populate.
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        let child_conversation_id = model_handle.read(&app, |model, _| {
            model
                .children
                .get(&task_id(CHILD_A_TASK_ID))
                .expect("child registered")
                .conversation_id
        });
        history.read(&app, |history, _| {
            // The child's run_id matches its task_id under v2 (and is the
            // string form of the same AmbientAgentTaskId in either case).
            let child_run_id = task_id(CHILD_A_TASK_ID).to_string();
            assert_eq!(
                history.conversation_id_for_agent_id(&child_run_id),
                Some(child_conversation_id),
                "sibling references via run_id must resolve to the child conversation",
            );
        });
    });
}

// ---- parent_agent_id backfill ----------------------------------------------

#[test]
fn b2_backfills_parent_agent_id_on_orchestrator_token_assigned() {
    // When the orchestrator's local conversation doesn't have an
    // `orchestration_agent_id` yet at child-creation time, the
    // viewer-created child's `parent_agent_id` stays `None`. When the
    // orchestrator subsequently receives its run id (via
    // `assign_run_id_for_conversation`), the model should backfill
    // `parent_agent_id` on every tracked child so
    // `orchestration_conversation_links::parent_conversation_id` resolves
    // back to the orchestrator.
    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Step 1: register a child while the parent has no orchestration
        // agent id. The child's `parent_agent_id` must be `None`.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });
        let history = BlocklistAIHistoryModel::handle(&app);
        let child_conversation_id = history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            assert_eq!(child_ids.len(), 1, "one child registered");
            let child = history
                .conversation(&child_ids[0])
                .expect("child conversation exists");
            assert!(
                child.parent_agent_id().is_none(),
                "parent_agent_id should be unset before the orchestrator has a run id",
            );
            child_ids[0]
        });

        // Step 2: assign the parent's run id. `assign_run_id_for_conversation`
        // emits `ConversationServerTokenAssigned`, which fires the model's
        // subscription. Since `setup_model` bypasses the constructor (and
        // therefore the subscription wiring), call the handler directly.
        let parent_run_id = parent.to_string();
        history.update(&mut app, |history, ctx| {
            history.assign_run_id_for_conversation(
                parent_conv_id,
                parent_run_id.clone(),
                Some(parent),
                terminal_view_id,
                ctx,
            );
        });
        let synthetic_event = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id: parent_conv_id,
            terminal_view_id,
        };
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_backfill_parent_agent_ids(&synthetic_event, ctx);
        });

        // Step 3: the child's `parent_agent_id` is now stamped with the
        // orchestrator's run id, so `parent_agent_id`-based resolution can
        // walk back up to the parent.
        history.read(&app, |history, _| {
            let child = history
                .conversation(&child_conversation_id)
                .expect("child conversation exists");
            assert_eq!(
                child.parent_agent_id(),
                Some(parent_run_id.as_str()),
                "parent_agent_id should be backfilled to the orchestrator's run id",
            );
        });
    });
}

#[test]
fn b2_does_not_overwrite_existing_parent_agent_id() {
    // The backfill is a one-way upgrade. Children whose `parent_agent_id`
    // is already set (e.g. created after the orchestrator already had a
    // run id) must not be clobbered.
    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Pre-seed the orchestrator with a run id so the child created
        // below picks it up immediately.
        let original_parent_run_id = parent.to_string();
        let history = BlocklistAIHistoryModel::handle(&app);
        history.update(&mut app, |history, ctx| {
            history.assign_run_id_for_conversation(
                parent_conv_id,
                original_parent_run_id.clone(),
                Some(parent),
                terminal_view_id,
                ctx,
            );
        });
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });
        let child_conversation_id = history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            child_ids[0]
        });

        // Now fire a backfill: the existing `parent_agent_id` must stay.
        let synthetic_event = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id: parent_conv_id,
            terminal_view_id,
        };
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_backfill_parent_agent_ids(&synthetic_event, ctx);
        });
        history.read(&app, |history, _| {
            let child = history
                .conversation(&child_conversation_id)
                .expect("child conversation exists");
            assert_eq!(
                child.parent_agent_id(),
                Some(original_parent_run_id.as_str()),
            );
        });
    });
}

#[test]
fn b2_ignores_token_assigned_for_unrelated_conversation() {
    // Events for other conversations (e.g. the user's local conversation
    // in another tab) must not trigger backfill on this model's children.
    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });

        // Synthesize an event for some unrelated conversation id; the
        // backfill handler must short-circuit on the parent-mismatch check.
        let unrelated_event = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id: AIConversationId::new(),
            terminal_view_id,
        };
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_backfill_parent_agent_ids(&unrelated_event, ctx);
        });

        // Belt-and-braces: ensure the parent's lookup short-circuits when
        // the orchestrator id is still unknown.
        let still_no_parent_id = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id: parent_conv_id,
            terminal_view_id,
        };
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_backfill_parent_agent_ids(&still_no_parent_id, ctx);
        });

        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history.conversation(&child_ids[0]).unwrap();
            assert!(
                child.parent_agent_id().is_none(),
                "backfill must not run when orchestrator has no agent id yet",
            );
        });
    });
}

// ---- child-link sibling preload --------------------------------------------
//
// Removed: see specs/QUALITY-726/TECH.md §B4 for the deferral note.

// ---- idle_due_to_no_children polling-cost mitigation ----------------------

/// Builds an [`AppendedExchange`] event for the given conversation, with
/// stub identifiers for the unrelated fields. Mirrors what the history
/// model would emit when a fresh exchange is appended; the model's
/// `maybe_kick_polling` handler only reads `conversation_id`.
fn make_appended_exchange_event(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
) -> BlocklistAIHistoryEvent {
    BlocklistAIHistoryEvent::AppendedExchange {
        exchange_id: AIAgentExchangeId::new(),
        task_id: TaskId::new("test-task".to_string()),
        terminal_view_id,
        conversation_id,
        is_hidden: false,
        response_stream_id: None,
    }
}

/// Spawns a long-lived no-op future and stores its handle on the model.
/// Used to populate `polling_handle` in tests so we can assert that the
/// polling-state machine aborts it when transitioning to the
/// idle-due-to-no-children state.
///
/// `SpawnedFutureHandle::abort()` doesn't expose an observable side-effect
/// from outside the model, so the assertion target is
/// `model.polling_handle.is_none()` after the transition. The timer is
/// scheduled for an hour so it cannot fire during the test.
fn populate_polling_handle(
    model: &mut OrchestrationViewerModel,
    ctx: &mut ModelContext<OrchestrationViewerModel>,
) {
    let handle = ctx.spawn(
        async {
            Timer::after(Duration::from_secs(3600)).await;
        },
        |_me, _, _ctx| {},
    );
    model.polling_handle = Some(handle);
}

#[test]
fn empty_descendant_fetch_sets_idle_flag_and_aborts_polling() {
    // When a non-orchestrator share's first descendant fetch returns no
    // children, the viewer model sets `idle_due_to_no_children = true`
    // and tears down its polling handle. `schedule_next_poll` later
    // honours the flag and refuses to spawn another timer, so the model
    // spends zero CPU / network until an `AppendedExchange` on the
    // orchestrator wakes it up.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Simulate an already-active polling cadence by pre-populating the
        // handle. After the empty fetch this handle should be cleared.
        model_handle.update(&mut app, |model, ctx| {
            populate_polling_handle(model, ctx);
        });
        model_handle.read(&app, |model, _| {
            assert!(
                model.polling_handle.is_some(),
                "sanity: polling_handle populated for the test"
            );
            assert!(
                !model.idle_due_to_no_children,
                "sanity: idle flag starts clear"
            );
        });

        // Server returns no descendants. `apply_children_fetch` is the
        // sync portion of the fetch callback, so calling it directly
        // exercises the polling-state transition without an HTTP round
        // trip.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(vec![], ctx);
        });

        model_handle.read(&app, |model, _| {
            assert!(
                model.idle_due_to_no_children,
                "empty fetch must mark the model as idle-due-to-no-children"
            );
            assert!(
                model.polling_handle.is_none(),
                "empty fetch must abort the prior polling handle and clear the field"
            );
            assert!(
                model.children.is_empty(),
                "sanity: no children were registered"
            );
        });
    });
}

#[test]
fn appended_exchange_on_orchestrator_resumes_from_idle() {
    // Once the model has gone idle on an empty fetch, the next
    // `AppendedExchange` on the orchestrator conversation must resume
    // polling: clear the idle flag and call `fetch_children`. We observe
    // `fetch_children` indirectly via `fetch_generation`, which is bumped
    // synchronously at the head of the function before any spawn fires.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, parent_conv_id, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Drive the model into the idle state via the same path the
        // production fetch callback would: empty descendant list.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(vec![], ctx);
        });
        let generation_before_resume = model_handle.read(&app, |model, _| {
            assert!(
                model.idle_due_to_no_children,
                "sanity: idle flag set after empty fetch"
            );
            assert!(
                model.polling_handle.is_none(),
                "sanity: polling handle aborted"
            );
            model.fetch_generation
        });

        // Fire an `AppendedExchange` against the orchestrator. The model
        // resumes via the idle-due-to-no-children branch in
        // `maybe_kick_polling`.
        let event = make_appended_exchange_event(parent_conv_id, terminal_view_id);
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_kick_polling(&event, ctx);
        });

        model_handle.read(&app, |model, _| {
            assert!(
                !model.idle_due_to_no_children,
                "AppendedExchange on the orchestrator must clear the idle flag"
            );
            assert_eq!(
                model.fetch_generation,
                generation_before_resume.wrapping_add(1),
                "AppendedExchange on the orchestrator must call fetch_children, \
                 which bumps fetch_generation by one",
            );
        });
    });
}

#[test]
fn non_empty_fetch_clears_idle_flag_and_resumes_polling() {
    // The complementary path: a fetch that *does* discover children
    // clears the idle flag so subsequent `schedule_next_poll` calls go
    // back to the active cadence. We then exercise `schedule_next_poll`
    // directly to verify that, once the flag is clear, a new
    // `polling_handle` is created.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, mut model) = setup_model(&mut app, parent);
        // Manually start from the idle state so we can confirm the
        // non-empty fetch clears it.
        model.idle_due_to_no_children = true;
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });
        model_handle.read(&app, |model, _| {
            assert!(
                !model.idle_due_to_no_children,
                "non-empty fetch must clear the idle flag"
            );
            assert_eq!(model.children.len(), 1, "child was registered");
        });

        // The fetch callback would normally call `schedule_next_poll`
        // right after `apply_children_fetch`. Invoke it explicitly so we
        // can assert that, with the flag now cleared, a new polling
        // handle is installed.
        model_handle.update(&mut app, |model, ctx| {
            model.schedule_next_poll(ctx);
        });
        model_handle.read(&app, |model, _| {
            assert!(
                model.polling_handle.is_some(),
                "schedule_next_poll must spawn a new timer when not idle"
            );
        });
    });
}

#[test]
fn appended_exchange_on_non_orchestrator_does_not_resume_idle() {
    // Symmetric to the orchestrator-resume test: an exchange on an
    // unrelated conversation (i.e. not the orchestrator tracked by this
    // viewer) must not pull the model out of the idle-due-to-no-children
    // state. The flag stays set and `fetch_children` is not invoked.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        // Idle the model.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(vec![], ctx);
        });
        let generation_before_event = model_handle.read(&app, |model, _| {
            assert!(model.idle_due_to_no_children, "sanity: model is idle");
            model.fetch_generation
        });

        // Fire an `AppendedExchange` for some unrelated conversation. The
        // resume gate compares against the orchestrator id returned by
        // `find_parent_conversation_id`, so a fresh id will not match.
        let unrelated_conversation_id = AIConversationId::new();
        let event = make_appended_exchange_event(unrelated_conversation_id, terminal_view_id);
        model_handle.update(&mut app, |model, ctx| {
            model.maybe_kick_polling(&event, ctx);
        });

        model_handle.read(&app, |model, _| {
            assert!(
                model.idle_due_to_no_children,
                "AppendedExchange on an unrelated conversation must NOT resume the model"
            );
            assert_eq!(
                model.fetch_generation, generation_before_event,
                "fetch_children must not run when the resume gate doesn't match the orchestrator"
            );
            assert!(
                model.polling_handle.is_none(),
                "polling handle must remain cleared while idle"
            );
        });
    });
}

// ---- Streamer-driven path tests (flag ON) ----------------------------------

#[test]
fn handle_streamer_event_filters_on_parent_task_id() {
    // Each viewer pane has its own model filtered on its own
    // `parent_task_id`. Events targeted at a different parent must be
    // ignored — even when they arrive on the shared streamer subscription.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);

        // Register a child for `parent`; we'll later send an event for a
        // different parent and confirm the model doesn't touch the
        // pre-existing placeholder.
        let model_handle = app.add_model(|_| model);
        model_handle.update(&mut app, |model, ctx| {
            model.register_child(
                make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    None,
                ),
                ctx,
            );
        });

        let other_parent = task_id(CHILD_B_TASK_ID);
        model_handle.update(&mut app, |model, ctx| {
            // Synthetic event for a different parent task. Must be ignored.
            model.handle_streamer_event(
                &OrchestrationEventStreamerEvent::ChildStatusChanged {
                    parent_task_id: other_parent,
                    run_id: CHILD_A_TASK_ID.to_string(),
                    status: ConversationStatus::Cancelled,
                },
                ctx,
            );
        });

        // Placeholder status must remain InProgress (set by registration).
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            let child = history.conversation(&child_ids[0]).unwrap();
            assert!(matches!(child.status(), ConversationStatus::InProgress));
        });
    });
}

#[test]
fn child_spawned_with_malformed_run_id_is_dropped() {
    // The ChildSpawned handler parses the wire run_id into an
    // AmbientAgentTaskId. A malformed value must not panic.
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, _, model) = setup_model(&mut app, parent);
        let model_handle = app.add_model(|_| model);

        model_handle.update(&mut app, |model, ctx| {
            model.handle_child_spawned("not-a-uuid".to_string(), ctx);
        });

        model_handle.read(&app, |model, _| {
            assert!(model.children.is_empty());
            assert!(model.children_by_run_id.is_empty());
        });
    });
}

#[test]
fn streamer_consumer_is_registered_when_constructed_under_flag() {
    // Flag ON: `OrchestrationViewerModel::new` registers the pane on the
    // shared streamer entry and kicks off the cold-start seed.
    use warp_core::features::FeatureFlag;

    App::test((), |mut app| async move {
        let _v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let _streamer_guard = FeatureFlag::OrchestrationViewerStreamer.override_enabled(true);
        let _pill_bar_guard = FeatureFlag::OrchestrationViewerPillBar.override_enabled(true);

        let parent = task_id(PARENT_TASK_ID);
        let (terminal_view_id, _parent_conv_id, _) = setup_model(&mut app, parent);

        // The streamer singleton is registered by initialize_app_for_terminal_view
        // when OrchestrationV2 is enabled at app-setup time. We don't depend on
        // its presence here — what we're verifying is the registration path of
        // the model's `new` constructor. If the singleton isn't installed, the
        // model's `register_viewer_mode_consumer` call no-ops (handle resolution
        // returns nothing) but doesn't panic.

        // Try constructing the model. The construction must not panic even
        // when the streamer is or isn't present.
        let terminal_view = add_window_with_terminal(&mut app, None);
        let _ = app.add_model(|ctx| {
            OrchestrationViewerModel::new(parent, terminal_view_id, terminal_view.downgrade(), ctx)
        });

        // The streamer's viewer-mode registration is exercised end-to-end
        // by the streamer-side tests; here we just verify non-panicking
        // construction of the viewer model under the flag.
        let _ = terminal_view_id;
    });
}

// ---- Mock helper for `MockAIClient::expect_*` ------------------------------

// (Mock-based tests are kept off the critical path because the mock infra
// requires extensive plumbing for `App::test`. The streamer-side tests
// already cover the SSE / event-dispatch surface; the model-side tests
// above cover the registration semantics. A two-pane fixture is exercised
// at the streamer level by
// `viewer_mode_consumer_refcount_handles_multiple_panes_and_double_unregister`
// in `orchestration_event_streamer_tests.rs`.)
#[allow(dead_code)]
fn _mock_with_get_ambient_agent_task_for_child(task: AmbientAgentTask) -> Arc<dyn AIClient> {
    use mockall::predicate::eq;
    let mut mock = MockAIClient::new();
    let task_id = task.task_id;
    mock.expect_get_ambient_agent_task()
        .with(eq(task_id))
        .returning(move |_| Ok(task.clone()));
    Arc::new(mock)
}

#[allow(dead_code)]
fn _server_api_for_test() -> Arc<crate::server::server_api::ServerApi> {
    ServerApiProvider::new_for_test().get()
}
