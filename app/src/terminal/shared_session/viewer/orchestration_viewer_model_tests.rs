//! Tests for [`OrchestrationViewerModel`].
//!
//! Split into two layers:
//!
//! 1. Pure-function tests for [`conversation_status_from_state`] — no app context needed.
//! 2. App-context tests for [`OrchestrationViewerModel::apply_children_fetch`] —
//!    exercises the children-discovery, status-update, and materialization-emission
//!    paths against a real [`BlocklistAIHistoryModel`] + [`TerminalView`].
//!
//! The model's `fetch_children` / `schedule_next_poll` paths (HTTP + timer)
//! are not directly tested — they're thin wrappers that funnel responses
//! through `apply_children_fetch`, which is what we cover here.

use super::*;

use chrono::Utc;
use warpui::{App, EntityId, SingletonEntity};

use crate::ai::ambient_agents::task::AmbientAgentTask;
use crate::test_util::{add_window_with_terminal, terminal::initialize_app_for_terminal_view};

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

/// Builds a minimal [`AmbientAgentTask`] suitable for `apply_children_fetch`.
fn make_task(
    id: &str,
    state: AmbientAgentTaskState,
    title: &str,
    session_id: Option<&str>,
) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id: task_id(id),
        parent_run_id: Some(PARENT_TASK_ID.to_string()),
        title: title.to_string(),
        state,
        prompt: String::new(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        status_message: None,
        source: None,
        session_id: session_id.map(String::from),
        session_link: None,
        creator: None,
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

/// Wires up `BlocklistAIHistoryModel`, a real [`TerminalView`], and an
/// orchestrator parent conversation marked active for that view. Returns
/// the model built directly (bypassing `OrchestrationViewerModel::new`,
/// which would otherwise kick off an immediate REST fetch).
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
        polling_handle: None,
        fetch_generation: 0,
    };

    (terminal_view_id, parent_conversation_id, model)
}

// ---- apply_children_fetch tests ---------------------------------------------

#[test]
fn registers_new_child_conversation() {
    App::test((), |mut app| async move {
        let parent = task_id(PARENT_TASK_ID);
        let (_, parent_conv_id, model) = setup_model(&mut app, parent);

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

        // Server endpoint returns descendants *and* the parent itself.
        // The parent should be filtered out.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    PARENT_TASK_ID,
                    AmbientAgentTaskState::Succeeded,
                    "Self",
                    None,
                )],
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
        // registration should be deferred to the next poll.
        let model = OrchestrationViewerModel {
            parent_task_id: task_id(PARENT_TASK_ID),
            terminal_view_id,
            terminal_view: terminal_view.downgrade(),
            children: HashMap::new(),
            polling_handle: None,
            fetch_generation: 0,
        };
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

        // First fetch: child in progress.
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

        // Second fetch: same child, now succeeded.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::Succeeded,
                    "Worker",
                    None,
                )],
                ctx,
            );
        });

        // Model's cached state reflects the new state.
        model_handle.read(&app, |model, _| {
            let entry = model.children.get(&task_id(CHILD_A_TASK_ID)).unwrap();
            assert!(matches!(entry.last_state, AmbientAgentTaskState::Succeeded));
        });

        // History model's conversation status reflects the new state.
        let history = BlocklistAIHistoryModel::handle(&app);
        history.read(&app, |history, _| {
            let child_ids = history.child_conversation_ids_of(&parent_conv_id);
            assert_eq!(child_ids.len(), 1, "still one child after re-fetch");
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

        // First fetch: child has session_id from the start. Materialization
        // gate should flip to true.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                )],
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

        // Second fetch: same child, still has the same session_id. Gate must
        // remain set; we never want to re-emit the materialization event.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                )],
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

        // First fetch: no session_id yet (e.g. child is still Queued).
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::Queued,
                    "Worker",
                    None,
                )],
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

        // Second fetch: child now has a session_id. Gate flips.
        model_handle.update(&mut app, |model, ctx| {
            model.apply_children_fetch(
                vec![make_task(
                    CHILD_A_TASK_ID,
                    AmbientAgentTaskState::InProgress,
                    "Worker",
                    Some(SESSION_A),
                )],
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
            model.apply_children_fetch(
                vec![
                    make_task(
                        CHILD_A_TASK_ID,
                        AmbientAgentTaskState::InProgress,
                        "Agent One",
                        None,
                    ),
                    make_task(
                        CHILD_B_TASK_ID,
                        AmbientAgentTaskState::Succeeded,
                        "Agent Two",
                        None,
                    ),
                ],
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
