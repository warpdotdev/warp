use warp::tui_export::{
    register_tui_session_view_test_singletons, StartAgentExecutionMode, StartAgentExecutor,
    StartAgentOutcome,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle, ReadModel, SingletonEntity as _, UpdateModel};
use warpui_core::{App, WindowId};

use super::TuiOrchestrationModel;
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::test_fixtures::{
    add_active_test_conversation, add_test_semantic_selection, add_test_terminal_session,
};

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
    app.update(TuiOrchestrationModel::register);
    OrchestrationFixture {
        sessions,
        window_id,
    }
}

/// Registers a session (with a live active conversation) and returns its id
/// plus its surface's `StartAgentExecutor`.
fn add_dispatching_session(
    app: &mut App,
    fixture: &OrchestrationFixture,
    focus: bool,
) -> (TuiSessionId, ModelHandle<StartAgentExecutor>) {
    let (session, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update_model(&fixture.sessions, |sessions, ctx| {
        sessions.add_session(session.clone(), manager, focus, ctx)
    });
    add_active_test_conversation(app, session_id.surface_id());
    let executor = app.read(|ctx| {
        let action_model = session.as_ref(ctx).ai_action_model().clone();
        ctx.read_model(&action_model, |model, app| model.start_agent_executor(app))
    });
    (session_id, executor)
}

/// Dispatches a StartAgent request through the session's executor and
/// returns the resolved outcome (the orchestration model resolves
/// unsupported modes synchronously within the same effect flush).
fn dispatch_and_recv(
    app: &mut App,
    fixture: &OrchestrationFixture,
    session_id: TuiSessionId,
    executor: &ModelHandle<StartAgentExecutor>,
    execution_mode: StartAgentExecutionMode,
) -> StartAgentOutcome {
    let parent_conversation_id = app.read(|ctx| {
        warp::tui_export::BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(session_id.surface_id())
            .expect("fixture registered an active conversation")
            .id()
    });
    let _ = fixture;
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
    receiver
        .try_recv()
        .expect("unsupported-mode dispatches resolve before the update returns")
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

#[test]
fn local_harness_children_fail_cleanly() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let (session_id, executor) = add_dispatching_session(&mut app, &fixture, true);

        let outcome = dispatch_and_recv(
            &mut app,
            &fixture,
            session_id,
            &executor,
            StartAgentExecutionMode::Local {
                harness_type: Some("claude".to_string()),
                model_id: None,
            },
        );
        assert_error_containing(outcome, "aren't supported in the Warp TUI yet");
    });
}

#[test]
fn remote_children_fail_cleanly() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let (session_id, executor) = add_dispatching_session(&mut app, &fixture, true);

        let outcome = dispatch_and_recv(
            &mut app,
            &fixture,
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
            },
        );
        assert_error_containing(outcome, "Cloud child agents aren't supported");
    });
}

#[test]
fn sessions_registered_after_init_are_wired_for_orchestration() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        // First session exists before the dispatching one, mirroring a child
        // session dispatching grandchildren later in an app's lifetime.
        let _ = add_dispatching_session(&mut app, &fixture, true);
        let (late_session_id, late_executor) = add_dispatching_session(&mut app, &fixture, false);

        let outcome = dispatch_and_recv(
            &mut app,
            &fixture,
            late_session_id,
            &late_executor,
            StartAgentExecutionMode::Local {
                harness_type: Some("codex".to_string()),
                model_id: None,
            },
        );
        // A resolved outcome proves the late session's executor is wired to
        // the orchestration model (an unwired executor would never resolve).
        assert_error_containing(outcome, "aren't supported in the Warp TUI yet");
    });
}
