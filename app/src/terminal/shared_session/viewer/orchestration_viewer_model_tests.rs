//! Tests for [`OrchestrationViewerModel`]: task-state mapping, child
//! registration, streamer-driven discovery, and the pending-metadata poll.

use chrono::Utc;
use warpui::{App, ModelHandle, ViewHandle};

use super::*;
use crate::ai::agent::conversation::AIConversation;
use crate::ai::ambient_agents::task::AgentConfigSnapshot;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

// ---- Task-state mapping -----------------------------------------------------

#[test]
fn working_states_map_to_in_progress() {
    for state in [
        AmbientAgentTaskState::Queued,
        AmbientAgentTaskState::Pending,
        AmbientAgentTaskState::Claimed,
        AmbientAgentTaskState::InProgress,
    ] {
        assert_eq!(
            conversation_status_from_state(&state),
            ConversationStatus::InProgress,
            "state={state:?}",
        );
    }
}

#[test]
fn settled_states_map_to_matching_conversation_status() {
    // `Unknown` is a forward-compat catch-all for server states the client
    // doesn't recognize; the rest of the client treats it as a failure.
    let cases = [
        (
            AmbientAgentTaskState::Succeeded,
            ConversationStatus::Success,
        ),
        (AmbientAgentTaskState::Failed, ConversationStatus::Error),
        (AmbientAgentTaskState::Error, ConversationStatus::Error),
        (AmbientAgentTaskState::Unknown, ConversationStatus::Error),
        (
            AmbientAgentTaskState::Cancelled,
            ConversationStatus::Cancelled,
        ),
        (
            AmbientAgentTaskState::Blocked,
            ConversationStatus::Blocked {
                blocked_action: String::new(),
            },
        ),
    ];
    for (state, expected) in cases {
        assert_eq!(
            conversation_status_from_state(&state),
            expected,
            "state={state:?}",
        );
    }
}

// ---- Stub UUIDs used throughout these tests ---------------------------------

const PARENT_TASK_ID: &str = "11111111-1111-1111-1111-111111111111";
const CHILD_A_TASK_ID: &str = "22222222-2222-2222-2222-222222222222";
const CHILD_B_TASK_ID: &str = "33333333-3333-3333-3333-333333333333";
const SESSION_A: &str = "44444444-4444-4444-4444-444444444444";

fn task_id(id: &str) -> AmbientAgentTaskId {
    id.parse().expect("hardcoded task id parses")
}

// ---- Child registration -----------------------------------------------------

#[test]
fn registers_child_as_remote_child_conversation() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            assert!(entry.session_id.is_none());
            assert!(!entry.pane_materialization_requested);
            assert_eq!(entry.last_state, AmbientAgentTaskState::Queued);
        });
        fixture.model.read(&app, |model, _| {
            assert_eq!(
                model.children_by_run_id.get(CHILD_A_TASK_ID),
                Some(&task_id(CHILD_A_TASK_ID)),
                "run_id index must resolve back to the child task",
            );
        });

        let child = only_child_conversation(&app, &fixture);
        assert_eq!(child.agent_name(), Some("Worker"));
        assert_eq!(child.parent_conversation_id(), Some(fixture.parent_conv_id));
        assert!(child.is_remote_child());
        assert!(!child.is_viewing_shared_session());
        assert_eq!(child.status(), &ConversationStatus::InProgress);
    });
}

#[test]
fn registers_child_as_shared_session_viewer_when_unified_stack_is_disabled() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(false);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        let child = only_child_conversation(&app, &fixture);
        assert!(child.is_viewing_shared_session());
        assert!(!child.is_remote_child());
    });
}

#[test]
fn skips_registration_for_the_parent_task() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // The server's ancestor response includes the parent itself.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(PARENT_TASK_ID, "Self"));

        fixture
            .model
            .read(&app, |model, _| assert!(model.children.is_empty()));
        assert!(child_conversations(&app, &fixture).is_empty());
    });
}

#[test]
fn skips_registration_without_an_active_parent_conversation() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Registering without a parent conversation would lose the child's
        // parent linkage, so the child is dropped until one exists.
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let model = app.add_model(|_| test_model(task_id(PARENT_TASK_ID), &terminal_view));
        let fixture = Fixture {
            terminal_view_id: terminal_view.id(),
            parent_conv_id: AIConversationId::new(),
            model,
        };

        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        fixture
            .model
            .read(&app, |model, _| assert!(model.children.is_empty()));
    });
}

#[test]
fn updates_child_status_when_task_state_changes() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        register_child(
            &mut app,
            &fixture,
            task(CHILD_A_TASK_ID, AmbientAgentTaskState::Succeeded, "Worker"),
        );

        read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            assert_eq!(entry.last_state, AmbientAgentTaskState::Succeeded);
        });
        let child = only_child_conversation(&app, &fixture);
        assert_eq!(child.status(), &ConversationStatus::Success);
    });
}

#[test]
fn maps_child_run_id_to_its_local_conversation() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Sibling references in transcript bodies resolve display names
        // through this mapping.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        let child_conversation_id = read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            entry.conversation_id
        });
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation_id_for_agent_id(CHILD_A_TASK_ID),
                Some(child_conversation_id),
            );
        });
    });
}

#[test]
fn child_display_name_prefers_snapshot_name_over_title() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let long_title = "Long descriptive task title";
        // (snapshot name, title) -> (agent name, fallback display title)
        let cases = [
            (
                Some("frontend-tests"),
                long_title,
                "frontend-tests",
                Some(long_title),
            ),
            (
                Some("  frontend-tests  "),
                long_title,
                "frontend-tests",
                Some(long_title),
            ),
            (None, long_title, long_title, Some(long_title)),
            (None, "   ", "Agent", None),
            (None, "", "Agent", None),
        ];

        let fixture = setup(&mut app);
        for (index, (snapshot_name, title, expected_name, expected_title)) in
            cases.into_iter().enumerate()
        {
            let child_task_id = nth_child_task_id(index);
            register_child(
                &mut app,
                &fixture,
                task_with_agent_name(&child_task_id, snapshot_name, title),
            );

            let conversation_id = read_child(&app, &fixture, &child_task_id, |entry| {
                entry.conversation_id
            });
            let child = conversation(&app, conversation_id);
            assert_eq!(child.agent_name(), Some(expected_name), "case {index}");
            assert_eq!(child.title().as_deref(), expected_title, "case {index}");
        }
    });
}

// ---- Pane materialization ---------------------------------------------------

#[test]
fn requests_materialization_once_when_the_child_becomes_attachable() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            assert!(
                !entry.pane_materialization_requested,
                "a child with no attachable session is not materializable yet",
            );
        });

        register_child(
            &mut app,
            &fixture,
            live_task(CHILD_A_TASK_ID, "Worker", SESSION_A),
        );
        read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            assert_eq!(entry.session_id, Some(session_id(SESSION_A)));
            assert!(entry.pane_materialization_requested);
        });

        // Re-registration must not re-request materialization.
        register_child(
            &mut app,
            &fixture,
            live_task(CHILD_A_TASK_ID, "Worker", SESSION_A),
        );
        fixture.model.read(&app, |model, _| {
            assert!(!model.has_pending_session_id_children());
        });
    });
}

#[test]
fn requests_materialization_for_a_completed_child_with_a_stale_session() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // A completed run can still carry the session id of its finished
        // execution; the transcript decides materialization, not that id.
        let fixture = setup(&mut app);
        let mut completed = task(CHILD_A_TASK_ID, AmbientAgentTaskState::Succeeded, "Worker");
        completed.session_id = Some(SESSION_A.to_string());
        completed.conversation_id = Some("completed-child-token".to_string());
        register_child(&mut app, &fixture, completed);

        read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            assert!(entry.pane_materialization_requested);
            assert_eq!(entry.session_id, Some(session_id(SESSION_A)));
        });
        fixture.model.read(&app, |model, _| {
            assert!(
                !model.has_pending_session_id_children(),
                "a materialized child must not keep polling for metadata",
            );
        });
    });
}

// ---- Streamer-driven discovery ----------------------------------------------

#[test]
fn parks_an_undiscovered_child_until_its_task_data_arrives() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // A lifecycle event can arrive before (or instead of) `ChildSpawned`;
        // either way the child waits on the shared task cache.
        let fixture = setup(&mut app);
        status_changed(
            &mut app,
            &fixture,
            CHILD_A_TASK_ID,
            ConversationStatus::InProgress,
        );

        fixture.model.read(&app, |model, _| {
            assert!(
                model
                    .pending_task_ids_for_discovery
                    .contains(&task_id(CHILD_A_TASK_ID)),
            );
            assert!(model.children.is_empty());
        });

        cache_task(&mut app, live_task(CHILD_A_TASK_ID, "Worker", SESSION_A));
        fixture.model.update(&mut app, |model, ctx| {
            model.drain_pending_task_discoveries(ctx);
        });

        fixture.model.read(&app, |model, _| {
            assert!(model.pending_task_ids_for_discovery.is_empty());
            assert!(model.children.contains_key(&task_id(CHILD_A_TASK_ID)));
        });
        assert_eq!(child_conversations(&app, &fixture).len(), 1);
    });
}

#[test]
fn malformed_run_ids_never_create_a_child() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        fixture.model.update(&mut app, |model, ctx| {
            model.handle_child_spawned("not-a-uuid".to_string(), ctx);
            model.handle_child_status_changed("not-a-uuid", ConversationStatus::Success, ctx);
        });

        fixture.model.read(&app, |model, _| {
            assert!(model.children.is_empty());
            assert!(model.children_by_run_id.is_empty());
            assert!(model.pending_task_ids_for_discovery.is_empty());
        });
        assert!(child_conversations(&app, &fixture).is_empty());
    });
}

#[test]
fn status_change_updates_a_registered_child() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        status_changed(
            &mut app,
            &fixture,
            CHILD_A_TASK_ID,
            ConversationStatus::Success,
        );

        let child = only_child_conversation(&app, &fixture);
        assert_eq!(child.status(), &ConversationStatus::Success);
    });
}

#[test]
fn ignores_streamer_events_for_other_parents() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Every viewer pane subscribes to the same shared streamer, so each
        // model must filter on its own parent.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        fixture.model.update(&mut app, |model, ctx| {
            model.handle_streamer_event(
                &OrchestrationEventStreamerEvent::ChildStatusChanged {
                    parent_task_id: task_id(CHILD_B_TASK_ID),
                    run_id: CHILD_A_TASK_ID.to_string(),
                    status: ConversationStatus::Cancelled,
                },
                ctx,
            );
        });

        let child = only_child_conversation(&app, &fixture);
        assert_eq!(child.status(), &ConversationStatus::InProgress);
    });
}

#[test]
fn status_change_refetches_metadata_while_the_child_is_unmaterialized() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // A child first seen pre-claim has no attachable session; lifecycle
        // events are the trigger to pick up fresher task data.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        status_changed(
            &mut app,
            &fixture,
            CHILD_A_TASK_ID,
            ConversationStatus::InProgress,
        );

        fixture.model.read(&app, |model, _| {
            assert_eq!(model.metadata_fetch_dispatch_count, 1);
        });
    });
}

#[test]
fn status_change_does_not_refetch_metadata_after_materialization() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Otherwise every status change on a long-running child would cost a
        // metadata fetch.
        let fixture = setup(&mut app);
        register_child(
            &mut app,
            &fixture,
            live_task(CHILD_A_TASK_ID, "Worker", SESSION_A),
        );

        for status in [
            ConversationStatus::InProgress,
            ConversationStatus::Success,
            ConversationStatus::Cancelled,
        ] {
            status_changed(&mut app, &fixture, CHILD_A_TASK_ID, status);
        }

        fixture.model.read(&app, |model, _| {
            assert_eq!(model.metadata_fetch_dispatch_count, 0);
        });
    });
}

// ---- Pending-metadata poll --------------------------------------------------

#[test]
fn polls_for_task_metadata_until_the_child_is_materialized() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Without lifecycle events, the poll is the only thing that picks up
        // the claim-time session id for a child first seen pre-claim.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        fixture.model.read(&app, |model, _| {
            assert!(model.pending_session_id_poll_handle.is_some());
            assert_eq!(model.metadata_fetch_dispatch_count, 0);
        });

        run_poll_tick(&mut app, &fixture);
        fixture.model.read(&app, |model, _| {
            assert_eq!(model.metadata_fetch_dispatch_count, 1);
            assert!(
                model.pending_session_id_poll_handle.is_some(),
                "the poll reschedules while children remain pending",
            );
        });

        register_child(
            &mut app,
            &fixture,
            live_task(CHILD_A_TASK_ID, "Worker", SESSION_A),
        );
        run_poll_tick(&mut app, &fixture);
        fixture.model.read(&app, |model, _| {
            assert_eq!(
                model.metadata_fetch_dispatch_count, 1,
                "a materialized child must not be refetched",
            );
            assert!(
                model.pending_session_id_poll_handle.is_none(),
                "the poll stops once no child is pending",
            );
        });
    });
}

#[test]
fn does_not_poll_when_the_child_is_already_materialized() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(
            &mut app,
            &fixture,
            live_task(CHILD_A_TASK_ID, "Worker", SESSION_A),
        );

        fixture.model.read(&app, |model, _| {
            assert!(!model.has_pending_session_id_children());
            assert!(model.pending_session_id_poll_handle.is_none());
        });
    });
}

#[test]
fn poll_dispatches_one_fetch_per_pending_child() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        register_child(&mut app, &fixture, queued_task(CHILD_B_TASK_ID, "Worker"));

        run_poll_tick(&mut app, &fixture);

        fixture.model.read(&app, |model, _| {
            assert_eq!(model.children.len(), 2);
            assert_eq!(model.metadata_fetch_dispatch_count, 2);
        });
    });
}

// ---- parent_agent_id backfill -----------------------------------------------

#[test]
fn backfills_parent_agent_id_when_the_orchestrator_run_id_arrives() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Children registered before the orchestrator has a run id would
        // otherwise never resolve back to their parent conversation.
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));
        let child_conversation_id = read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            entry.conversation_id
        });
        assert!(
            conversation(&app, child_conversation_id)
                .parent_agent_id()
                .is_none(),
        );

        assign_parent_run_id(&mut app, &fixture);

        assert_eq!(
            conversation(&app, child_conversation_id).parent_agent_id(),
            Some(PARENT_TASK_ID),
        );
    });
}

#[test]
fn backfill_preserves_an_existing_parent_agent_id() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        assign_parent_run_id(&mut app, &fixture);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        assign_parent_run_id(&mut app, &fixture);

        let child_conversation_id = read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            entry.conversation_id
        });
        assert_eq!(
            conversation(&app, child_conversation_id).parent_agent_id(),
            Some(PARENT_TASK_ID),
        );
    });
}

#[test]
fn backfill_ignores_conversations_other_than_the_orchestrator() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = setup(&mut app);
        register_child(&mut app, &fixture, queued_task(CHILD_A_TASK_ID, "Worker"));

        // An unrelated conversation's token, then the orchestrator's own
        // token before it has an agent id: neither may stamp children.
        for conversation_id in [AIConversationId::new(), fixture.parent_conv_id] {
            let event = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                conversation_id,
                terminal_surface_id: fixture.terminal_view_id,
            };
            fixture.model.update(&mut app, |model, ctx| {
                model.maybe_backfill_parent_agent_ids(&event, ctx);
            });
        }

        let child_conversation_id = read_child(&app, &fixture, CHILD_A_TASK_ID, |entry| {
            entry.conversation_id
        });
        assert!(
            conversation(&app, child_conversation_id)
                .parent_agent_id()
                .is_none(),
        );
    });
}

// ---- Streamer consumer registration -----------------------------------------

#[test]
fn registers_streamer_consumer_when_the_parent_placeholder_becomes_active() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // The parent placeholder is usually marked active after the model is
        // constructed, so registration has to retry on history events or the
        // pill bar stays empty for the model's lifetime.
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal_view.id();
        let parent = task_id(PARENT_TASK_ID);
        let _model = app.add_model(|ctx| {
            OrchestrationViewerModel::new(parent, terminal_view_id, terminal_view.downgrade(), ctx)
        });

        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |streamer, _| {
            assert_eq!(streamer.viewer_mode_consumer_count_for_test(parent), 0);
        });

        // Shape produced when a viewer joins a shared session: viewing a
        // shared session, no parent conversation, no run id stamped.
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            let id = history.start_new_conversation(terminal_view_id, false, true, false, ctx);
            history.set_viewing_shared_session_for_conversation(id, true);
            history.set_active_conversation_id(id, terminal_view_id, ctx);
        });

        streamer.read(&app, |streamer, _| {
            assert_eq!(streamer.viewer_mode_consumer_count_for_test(parent), 1);
        });
    });
}

#[test]
fn does_not_register_streamer_consumer_for_a_child_placeholder() {
    let _unified_stack = FeatureFlag::OrchestrationUnifiedStack.override_enabled(true);
    App::test((), |mut app| async move {
        // Registering against a child would persist the orchestration cursor
        // on the wrong conversation.
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal_view.id();
        let parent = task_id(PARENT_TASK_ID);
        let _model = app.add_model(|ctx| {
            OrchestrationViewerModel::new(parent, terminal_view_id, terminal_view.downgrade(), ctx)
        });

        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
            let parent_conv_id =
                history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            let child_id = history.start_new_child_conversation(
                terminal_view_id,
                "child".to_string(),
                parent_conv_id,
                None,
                false,
                ctx,
            );
            history.set_viewing_shared_session_for_conversation(child_id, true);
            history.set_active_conversation_id(child_id, terminal_view_id, ctx);
        });

        let streamer = OrchestrationEventStreamer::handle(&app);
        streamer.read(&app, |streamer, _| {
            assert_eq!(streamer.viewer_mode_consumer_count_for_test(parent), 0);
        });
    });
}

// ---- Test helpers for Fixture-based tests ------------------------------------

/// A viewer model wired to a terminal view whose active conversation is the
/// orchestrator placeholder.
struct Fixture {
    terminal_view_id: EntityId,
    parent_conv_id: AIConversationId,
    model: ModelHandle<OrchestrationViewerModel>,
}

fn setup(app: &mut App) -> Fixture {
    initialize_app_for_terminal_view(app);
    let terminal_view = add_window_with_terminal(app, None);
    let terminal_view_id = terminal_view.id();
    let parent_conv_id = BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        let id = history.start_new_conversation(terminal_view_id, false, false, false, ctx);
        history.set_active_conversation_id(id, terminal_view_id, ctx);
        id
    });
    let model = app.add_model(|_| test_model(task_id(PARENT_TASK_ID), &terminal_view));
    Fixture {
        terminal_view_id,
        parent_conv_id,
        model,
    }
}

/// Builds the model directly, bypassing the streamer registration that
/// [`OrchestrationViewerModel::new`] performs.
fn test_model(
    parent_task_id: AmbientAgentTaskId,
    terminal_view: &ViewHandle<TerminalView>,
) -> OrchestrationViewerModel {
    OrchestrationViewerModel {
        parent_task_id,
        terminal_view_id: terminal_view.id(),
        terminal_view: terminal_view.downgrade(),
        children: HashMap::new(),
        children_by_run_id: HashMap::new(),
        metadata_fetches: HashSet::new(),
        pending_task_ids_for_discovery: HashSet::new(),
        pending_session_id_poll_handle: None,
        metadata_fetch_dispatch_count: 0,
    }
}

fn session_id(id: &str) -> SessionId {
    id.parse().expect("hardcoded session id parses")
}

fn nth_child_task_id(index: usize) -> String {
    format!("2222222{index}-2222-2222-2222-222222222222")
}

/// Minimal task with no execution and no server conversation token.
fn task(id: &str, state: AmbientAgentTaskState, title: &str) -> AmbientAgentTask {
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
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
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
        debug_agent_available: false,
        scope: None,
    }
}

/// Child that has not been claimed yet, so it has nothing to materialize.
fn queued_task(id: &str, title: &str) -> AmbientAgentTask {
    task(id, AmbientAgentTaskState::Queued, title)
}

/// Child running in a sandbox with a joinable session.
fn live_task(id: &str, title: &str, session_id: &str) -> AmbientAgentTask {
    let mut task = task(id, AmbientAgentTaskState::InProgress, title);
    task.session_id = Some(session_id.to_string());
    task.is_sandbox_running = true;
    task
}

fn task_with_agent_name(id: &str, snapshot_name: Option<&str>, title: &str) -> AmbientAgentTask {
    let mut task = task(id, AmbientAgentTaskState::InProgress, title);
    task.agent_config_snapshot = snapshot_name.map(|name| AgentConfigSnapshot {
        name: Some(name.to_string()),
        ..Default::default()
    });
    task
}

fn register_child(app: &mut App, fixture: &Fixture, task: AmbientAgentTask) {
    fixture.model.update(app, |model, ctx| {
        model.register_child(task, ctx);
    });
}

fn status_changed(app: &mut App, fixture: &Fixture, run_id: &str, status: ConversationStatus) {
    fixture.model.update(app, |model, ctx| {
        model.handle_child_status_changed(run_id, status, ctx);
    });
}

/// Runs the poll body the metadata timer would run, clearing the handle first
/// so rescheduling is observable.
fn run_poll_tick(app: &mut App, fixture: &Fixture) {
    fixture.model.update(app, |model, ctx| {
        model.pending_session_id_poll_handle = None;
        model.run_pending_session_id_poll(ctx);
    });
}

/// Assigns the orchestrator's run id and delivers the resulting token event,
/// which the model would otherwise receive through its subscription.
fn assign_parent_run_id(app: &mut App, fixture: &Fixture) {
    let parent = task_id(PARENT_TASK_ID);
    BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        history.assign_run_id_for_conversation(
            fixture.parent_conv_id,
            parent.to_string(),
            Some(parent),
            fixture.terminal_view_id,
            ctx,
        );
    });
    let event = BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
        conversation_id: fixture.parent_conv_id,
        terminal_surface_id: fixture.terminal_view_id,
    };
    fixture.model.update(app, |model, ctx| {
        model.maybe_backfill_parent_agent_ids(&event, ctx);
    });
}

fn cache_task(app: &mut App, task: AmbientAgentTask) {
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
}

fn read_child<S>(
    app: &App,
    fixture: &Fixture,
    child: &str,
    read: impl FnOnce(&ChildAgentEntry) -> S,
) -> S {
    fixture.model.read(app, |model, _| {
        read(
            model
                .children
                .get(&task_id(child))
                .expect("child is registered"),
        )
    })
}

fn conversation(app: &App, conversation_id: AIConversationId) -> AIConversation {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .conversation(&conversation_id)
            .expect("conversation exists")
            .clone()
    })
}

fn child_conversations(app: &App, fixture: &Fixture) -> Vec<AIConversation> {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        history
            .child_conversation_ids_of(&fixture.parent_conv_id)
            .iter()
            .map(|id| {
                history
                    .conversation(id)
                    .expect("child conversation exists")
                    .clone()
            })
            .collect()
    })
}

fn only_child_conversation(app: &App, fixture: &Fixture) -> AIConversation {
    let mut children = child_conversations(app, fixture);
    assert_eq!(children.len(), 1, "expected exactly one child conversation");
    children.remove(0)
}
