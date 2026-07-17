use warp::tui_export::{
    register_tui_session_view_test_singletons, AIConversationId, BlocklistAIHistoryModel,
    StartAgentExecutionMode, StartAgentExecutor, StartAgentExecutorEvent, StartAgentOutcome,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle, ReadModel, SingletonEntity as _, UpdateModel};
use warpui_core::{App, WindowId};

use super::TuiOrchestrationModel;
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};

struct OrchestrationFixture {
    sessions: ModelHandle<TuiSessions>,
    window_id: WindowId,
}

/// Boots the container + root + orchestration model wiring (no live PTYs).
fn orchestration_fixture(app: &mut App) -> OrchestrationFixture {
    register_tui_session_view_test_singletons(app);
    add_test_semantic_selection(app);
    app.update(crate::autoupdate::TuiAutoupdater::register);
    let (window_id, root) = app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| RootTuiView::new(),
        )
    });
    let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
    root.update(app, |_, ctx| {
        ctx.subscribe_to_model(&sessions, |_, _, _, ctx| ctx.notify());
    });
    let orchestration = app.update(TuiOrchestrationModel::register);
    app.update(|ctx| TuiSessions::wire_orchestration(&sessions, &orchestration, ctx));
    OrchestrationFixture {
        sessions,
        window_id,
    }
}

/// Registers a session with a live active conversation.
fn add_dispatching_session(
    app: &mut App,
    fixture: &OrchestrationFixture,
    focus: bool,
) -> TuiSessionId {
    let (session, manager) = add_test_terminal_session(app, fixture.window_id);
    app.update(|ctx| TuiSessions::register_session(&fixture.sessions, session, manager, focus, ctx))
}

/// Creates a standalone executor and relays its frontend materialization
/// events into the coordinator.
fn add_relayed_executor(
    app: &mut App,
    parent_session_id: TuiSessionId,
) -> ModelHandle<StartAgentExecutor> {
    let executor = app.add_model(StartAgentExecutor::new);
    app.update(|ctx| {
        let orchestration = TuiOrchestrationModel::handle(ctx);
        ctx.subscribe_to_model(&executor, move |_, event, ctx| {
            orchestration.update(ctx, |orchestration, ctx| match event {
                StartAgentExecutorEvent::CreateAgent(request) => {
                    orchestration.dispatch_create_agent(
                        parent_session_id,
                        (**request).clone(),
                        None,
                        ctx,
                    );
                }
                StartAgentExecutorEvent::CleanupFailedChildLaunch { conversation_id } => {
                    orchestration.cleanup_failed_child(conversation_id, ctx);
                }
            });
        });
    });
    executor
}

/// Dispatches a StartAgent request through the session's executor and
/// returns the resolved outcome (the orchestration model resolves
/// unsupported modes synchronously within the same effect flush).
fn dispatch_and_recv(
    app: &mut App,
    session_id: TuiSessionId,
    executor: &ModelHandle<StartAgentExecutor>,
    execution_mode: StartAgentExecutionMode,
) -> (AIConversationId, StartAgentOutcome) {
    let parent_conversation_id = app.read(|ctx| {
        warp::tui_export::BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(session_id.surface_id())
            .expect("fixture registered an active conversation")
            .id()
    });
    let receiver = app.update_model(executor, |executor, ctx| {
        executor.dispatch(
            "researcher".to_string(),
            "research the codebase".to_string(),
            execution_mode,
            None,
            parent_conversation_id,
            Some("parent-run-1".to_string()),
            ctx,
        )
    });
    (
        parent_conversation_id,
        receiver
            .try_recv()
            .expect("unsupported-mode dispatches resolve before the update returns"),
    )
}

fn assert_error_containing(outcome: StartAgentOutcome, needle: &str) {
    match outcome {
        StartAgentOutcome::Error(message) => {
            assert!(message.contains(needle), "unexpected error: {message}");
        }
        StartAgentOutcome::Started { agent_id } => {
            panic!("expected an error outcome, got Started({agent_id})");
        }
    }
}

fn assert_failed_launch_cleaned_up(
    app: &App,
    fixture: &OrchestrationFixture,
    parent_conversation_id: AIConversationId,
    expected_session_count: usize,
) {
    app.read(|ctx| {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        assert!(history
            .child_conversation_ids_of(&parent_conversation_id)
            .is_empty());
        assert!(TuiOrchestrationModel::as_ref(ctx)
            .event_consumers_by_session
            .is_empty());
    });
    assert_eq!(
        app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
        expected_session_count,
    );
}
#[test]
fn local_harness_children_fail_cleanly() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let session_id = add_dispatching_session(&mut app, &fixture, true);
        let executor = add_relayed_executor(&mut app, session_id);

        let (parent_conversation_id, outcome) = dispatch_and_recv(
            &mut app,
            session_id,
            &executor,
            StartAgentExecutionMode::Local {
                harness_type: Some("claude".to_string()),
                model_id: None,
            },
        );
        assert_error_containing(outcome, "aren't supported in the Warp TUI yet");
        assert_failed_launch_cleaned_up(&app, &fixture, parent_conversation_id, 1);
    });
}

#[test]
fn remote_children_fail_cleanly() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let session_id = add_dispatching_session(&mut app, &fixture, true);
        let executor = add_relayed_executor(&mut app, session_id);

        let (parent_conversation_id, outcome) = dispatch_and_recv(
            &mut app,
            session_id,
            &executor,
            StartAgentExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                skill_references: Vec::new(),
                model_id: "auto".to_string(),
                computer_use_enabled: false,
                worker_host: "warp".to_string(),
                harness_type: "oz".to_string(),
                title: "Researcher".to_string(),
                auth_secret_name: None,
                agent_identity_uid: None,
            },
        );
        assert_error_containing(outcome, "Cloud child agents aren't supported");
        assert_failed_launch_cleaned_up(&app, &fixture, parent_conversation_id, 1);
    });
}

#[test]
fn failed_launch_cleanup_preserves_other_sessions() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let _ = add_dispatching_session(&mut app, &fixture, true);
        let background_session_id = add_dispatching_session(&mut app, &fixture, false);
        let executor = add_relayed_executor(&mut app, background_session_id);

        let (parent_conversation_id, outcome) = dispatch_and_recv(
            &mut app,
            background_session_id,
            &executor,
            StartAgentExecutionMode::Local {
                harness_type: Some("codex".to_string()),
                model_id: None,
            },
        );
        assert_error_containing(outcome, "aren't supported in the Warp TUI yet");
        assert_failed_launch_cleaned_up(&app, &fixture, parent_conversation_id, 2);
    });
}
