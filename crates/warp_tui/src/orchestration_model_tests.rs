use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryModel, CloudAgentStartupBlocker, CloudAgentStartupFailure,
    CloudAgentStartupIssue, ConversationStatus, Harness, OrchestrationEventStreamerEvent,
    RenderableAIError, StartAgentExecutionMode, StartAgentExecutor, StartAgentExecutorEvent,
    StartAgentOutcome, StartAgentRequest, register_tui_session_view_test_singletons,
};
use warp_core::features::FeatureFlag;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle, ReadModel, SingletonEntity as _, UpdateModel};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect, text_width};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _, TypedActionView as _, WindowId};

use super::{ORCHESTRATOR_TAB_LABEL, TuiOrchestrationModel};
use crate::cloud_run::TuiCloudRunStartup;
use crate::cloud_run_view::{TuiCloudRunAction, TuiCloudRunView};
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessionView, TuiSessions};
use crate::tab_bar::TuiTabBarNavigationDirection;
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};

struct OrchestrationFixture {
    sessions: ModelHandle<TuiSessions>,
    window_id: WindowId,
}

fn remote_request(parent_conversation_id: AIConversationId) -> StartAgentRequest {
    StartAgentRequest {
        id: Default::default(),
        name: "cloud-researcher".to_string(),
        prompt: "research the codebase".to_string(),
        execution_mode: StartAgentExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            skill_references: Vec::new(),
            model_id: "auto".to_string(),
            computer_use_enabled: false,
            worker_host: "warp".to_string(),
            harness_type: "oz".to_string(),
            title: "Researcher".to_string(),
            auth_secret_name: None,
            runner_id: String::new(),
            agent_identity_uid: None,
        },
        lifecycle_subscription: None,
        parent_conversation_id,
        parent_run_id: Some("parent-run-1".to_string()),
    }
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

fn add_child_session(
    app: &mut App,
    fixture: &OrchestrationFixture,
    parent_conversation_id: AIConversationId,
    name: &str,
) -> (TuiSessionId, AIConversationId) {
    let (session, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, session, manager, false, ctx)
    });
    let conversation_id = app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                session_id.surface_id(),
                name.to_owned(),
                parent_conversation_id,
                Some(Harness::Oz),
                false,
                ctx,
            );
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
            conversation_id
        })
    });
    (session_id, conversation_id)
}

fn add_remote_child_session(
    app: &mut App,
    fixture: &OrchestrationFixture,
    parent_session_id: TuiSessionId,
    request: &StartAgentRequest,
    display_name: String,
    orchestration_harness: Harness,
) -> (
    AIConversationId,
    warpui::EntityId,
    ModelHandle<crate::cloud_run::TuiCloudRunState>,
) {
    let child = app.update(|ctx| {
        TuiSessions::create_remote_child_session(&fixture.sessions, parent_session_id, ctx)
    });
    let surface_id = child.session_id.surface_id();
    let cloud_run_state = child.cloud_run_state.clone();
    let conversation_id = app.update(|ctx| {
        TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.initialize_remote_child_session(
                &child,
                request,
                display_name,
                orchestration_harness,
                ctx,
            )
        })
    });
    (conversation_id, surface_id, cloud_run_state)
}

fn cloud_view(
    surface_id: warpui::EntityId,
    ctx: &warpui::AppContext,
) -> warpui::ViewHandle<TuiCloudRunView> {
    let session_id = TuiSessions::as_ref(ctx)
        .session_id_for_surface(surface_id)
        .expect("cloud session is retained");
    match TuiSessions::as_ref(ctx)
        .session(session_id)
        .expect("cloud session is registered")
        .view()
    {
        TuiSessionView::Cloud(view) => view.clone(),
        TuiSessionView::Terminal(_) => panic!("expected a lightweight cloud session"),
    }
}

/// Registers a session with a live active conversation.
fn add_dispatching_session(
    app: &mut App,
    fixture: &OrchestrationFixture,
    focus: bool,
) -> TuiSessionId {
    let (session, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, session, manager, focus, ctx)
    });
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(session_id.surface_id(), false, false, false, ctx);
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
        });
    });
    session_id
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
                    orchestration.cleanup_child(conversation_id, ctx);
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
        assert!(
            history
                .child_conversation_ids_of(&parent_conversation_id)
                .is_empty()
        );
        assert!(
            TuiOrchestrationModel::as_ref(ctx)
                .event_consumers_by_session
                .is_empty()
        );
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
        assert_error_containing(outcome, "aren't supported in Warp Agent CLI yet");
        assert_failed_launch_cleaned_up(&app, &fixture, parent_conversation_id, 1);
    });
}

#[test]
fn github_auth_blocker_keeps_the_remote_session_and_actionable_url() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let request = remote_request(parent_conversation_id);
        let (conversation_id, surface_id, cloud_run_state) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &request,
            "cloud-researcher".to_string(),
            Harness::Oz,
        );
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.finish_remote_child_launch(
                    conversation_id,
                    surface_id,
                    cloud_run_state.clone(),
                    Err(CloudAgentStartupIssue::Blocked(
                        CloudAgentStartupBlocker::GitHubAuthRequired {
                            message: "GitHub authentication required".to_string(),
                            auth_url: "https://example.com/auth".to_string(),
                        },
                    )),
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            assert!(
                TuiSessions::as_ref(ctx)
                    .session_id_for_surface(surface_id)
                    .is_some()
            );
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .unwrap()
                    .status(),
                &ConversationStatus::Blocked {
                    blocked_action: "GitHub authentication required".to_string(),
                }
            );
            let TuiCloudRunStartup::Blocked(blocker) = cloud_run_state.as_ref(ctx).startup() else {
                panic!("expected blocked cloud startup state");
            };
            assert_eq!(blocker.primary_url(), "https://example.com/auth");
        });
    });
}

#[test]
fn failed_remote_launch_records_cloud_startup_error_for_tui_rendering() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let request = remote_request(parent_conversation_id);
        let (conversation_id, surface_id, cloud_run_state) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &request,
            "cloud-researcher".to_string(),
            Harness::Oz,
        );
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.finish_remote_child_launch(
                    conversation_id,
                    surface_id,
                    cloud_run_state,
                    Err(CloudAgentStartupIssue::Failed(
                        CloudAgentStartupFailure::Other {
                            message: "Environment failed to start".to_string(),
                        },
                    )),
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("remote child conversation");
            assert_eq!(conversation.status(), &ConversationStatus::Error);
            assert!(matches!(
                conversation.status_error(),
                Some(RenderableAIError::CloudStartupFailed(message))
                    if message == "Environment failed to start"
            ));
        });
    });
}

#[test]
fn snapshot_is_shared_across_tree_and_filters_conversations_without_sessions() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .expect("parent conversation")
                .id()
        });
        let (first_session_id, first_child_id) =
            add_child_session(&mut app, &fixture, parent_conversation_id, "first-child");
        let (second_session_id, second_child_id) =
            add_child_session(&mut app, &fixture, parent_conversation_id, "second-child");
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "missing-session".to_owned(),
                    parent_conversation_id,
                    Some(Harness::Oz),
                    false,
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let model = TuiOrchestrationModel::as_ref(ctx);
            let parent = model
                .snapshot(parent_conversation_id, ctx)
                .expect("parent has navigable children");
            let child = model
                .snapshot(first_child_id, ctx)
                .expect("child resolves the same tree");
            assert_eq!(parent.root_conversation_id, parent_conversation_id);
            assert_eq!(child.root_conversation_id, parent_conversation_id);
            assert_eq!(
                parent
                    .children
                    .iter()
                    .map(|child| child.conversation_id)
                    .collect::<Vec<_>>(),
                vec![first_child_id, second_child_id]
            );
            assert_eq!(
                parent
                    .children
                    .iter()
                    .map(|child| child.spawn_index)
                    .collect::<Vec<_>>(),
                vec![0, 1]
            );
        });
        app.update(|ctx| {
            let selected = TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.focus_conversation_session(second_child_id, ctx)
            });
            assert_eq!(selected, Some(second_session_id));
        });
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(second_child_id, ctx)
                .expect("tab snapshot");
            assert_eq!(snapshot.page_anchor, Some(first_child_id));
            assert!(snapshot.reveal_selected);
        });
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.set_explicit_page(parent_conversation_id, second_child_id, ctx);
            });
        });
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(parent_conversation_id, ctx)
                .expect("tab snapshot");
            assert_eq!(snapshot.page_anchor, Some(second_child_id));
            assert!(!snapshot.reveal_selected);
        });

        app.update(|ctx| {
            let selected = TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.focus_conversation_session(first_child_id, ctx)
            });
            assert_eq!(selected, Some(first_session_id));
        });
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(first_child_id, ctx)
                .expect("tab snapshot");
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(first_session_id)
            );
            assert_eq!(snapshot.page_anchor, Some(first_child_id));
            assert!(snapshot.reveal_selected);
        });
    });
}

#[test]
fn remote_child_session_is_navigable_and_projects_lifecycle() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let request = remote_request(parent_conversation_id);
        let (conversation_id, surface_id, cloud_run_state) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &request,
            "cloud-researcher".to_string(),
            Harness::Oz,
        );
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let conversation = history.conversation(&conversation_id).unwrap();
            assert!(conversation.is_remote_child());
            assert_eq!(
                history.resolved_parent_conversation_id_for_conversation(conversation),
                Some(parent_conversation_id)
            );
            assert!(
                TuiSessions::as_ref(ctx)
                    .session_id_for_surface(surface_id)
                    .is_some()
            );
            assert!(matches!(
                cloud_run_state.as_ref(ctx).startup(),
                TuiCloudRunStartup::Dispatching
            ));
            assert_eq!(
                cloud_run_state.as_ref(ctx).conversation_id(),
                Some(conversation_id)
            );
            let view = cloud_view(surface_id, ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 80, 12),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            let status_line = lines
                .iter()
                .find(|line| line.contains("Starting cloud run…"))
                .expect("cloud status is visible");
            let status_content = status_line.trim();
            assert_eq!(
                status_line.find(status_content),
                Some(usize::from((80 - text_width(status_content)).div_ceil(2)))
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Shift + ↑ sub-agents"))
            );
        });
        app.update(|ctx| {
            let view = cloud_view(surface_id, ctx);
            view.update(ctx, |view, ctx| {
                view.refresh_orchestration_tab_state(ctx);
                view.handle_action(&TuiCloudRunAction::FocusOrchestrationTabs, ctx);
            });
        });
        app.read(|ctx| {
            let view = cloud_view(surface_id, ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 112, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            assert_eq!(
                lines.last().map(|line| line.trim()),
                Some(
                    "Tab or ← → to navigate | Shift + ← → to go to start/end | ↓ to send a \
                     message  Ctrl+C to kill sub-agent"
                )
            );
        });

        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.assign_run_id_for_conversation(
                    conversation_id,
                    "00000000-0000-0000-0000-000000000004".to_string(),
                    None,
                    surface_id,
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation_id_for_agent_id("00000000-0000-0000-0000-000000000004"),
                Some(conversation_id)
            );
        });
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.handle_streamer_event(
                    &OrchestrationEventStreamerEvent::WatchedRunStatusChanged {
                        owner_conversation_id: parent_conversation_id,
                        run_id: "00000000-0000-0000-0000-000000000004".to_string(),
                        status: ConversationStatus::Success,
                    },
                    ctx,
                );
            });
        });
        app.read(|ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .unwrap()
                    .status(),
                &ConversationStatus::Success
            );
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(conversation_id, ctx)
                .expect("remote child remains navigable");
            let child = snapshot
                .children
                .iter()
                .find(|child| child.conversation_id == conversation_id)
                .expect("remote child has an orchestration tab");
            assert_eq!(child.status, ConversationStatus::Success);
        });
    });
}

#[test]
fn kill_child_agent_removes_session_and_conversation_from_map() {
    // AC 6 / TuiSessions removal: `kill_child_agent` must delete the child
    // conversation from history and remove the retained TUI session from the
    // session registry.
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let request = remote_request(parent_conversation_id);
        // Register a remote child with full orchestration-model wiring so the
        // child is in `child_session_by_conversation` and its session is in
        // `TuiSessions`.
        let (child_conversation_id, child_surface_id, _cloud_run_state) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &request,
            "researcher".to_string(),
            Harness::Oz,
        );

        // Confirm the child is registered before the kill.
        let initial_session_count = app.read_model(&fixture.sessions, |sessions, _| sessions.len());
        assert_eq!(
            initial_session_count, 2,
            "parent + child sessions before kill"
        );
        app.read(|ctx| {
            assert!(
                TuiSessions::as_ref(ctx)
                    .session_id_for_surface(child_surface_id)
                    .is_some(),
                "child session must be registered before kill"
            );
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_some(),
                "child conversation must exist before kill"
            );
        });

        // Kill the child through the orchestration model.
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.kill_child_agent(child_conversation_id, ctx);
            });
        });

        // After kill: conversation is gone from history, session is removed.
        app.read(|ctx| {
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_conversation_id)
                    .is_none(),
                "child conversation must be deleted from history after kill"
            );
            assert!(
                TuiSessions::as_ref(ctx)
                    .session_id_for_surface(child_surface_id)
                    .is_none(),
                "child session must be removed from TuiSessions after kill"
            );
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .is_some(),
                "parent conversation must survive child kill"
            );
        });
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1,
            "only the parent session should remain after kill"
        );
    });
}

#[test]
fn kill_descendant_agents_removes_nested_sessions_and_conversations() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let child_request = remote_request(parent_conversation_id);
        let (child_conversation_id, child_surface_id, _) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &child_request,
            "researcher".to_string(),
            Harness::Oz,
        );
        let child_session_id = app.read(|ctx| {
            TuiSessions::as_ref(ctx)
                .session_id_for_surface(child_surface_id)
                .expect("child session should be retained")
        });
        let grandchild_request = remote_request(child_conversation_id);
        let (grandchild_conversation_id, grandchild_surface_id, _) = add_remote_child_session(
            &mut app,
            &fixture,
            child_session_id,
            &grandchild_request,
            "nested-researcher".to_string(),
            Harness::Oz,
        );

        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.kill_descendant_agents(parent_conversation_id, ctx);
            });
        });

        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(history.conversation(&child_conversation_id).is_none());
            assert!(history.conversation(&grandchild_conversation_id).is_none());
            let sessions = TuiSessions::as_ref(ctx);
            assert!(sessions.session_id_for_surface(child_surface_id).is_none());
            assert!(
                sessions
                    .session_id_for_surface(grandchild_surface_id)
                    .is_none()
            );
            assert_eq!(sessions.focused_session_id(), Some(parent_session_id));
        });
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1
        );
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
        assert_error_containing(outcome, "aren't supported in Warp Agent CLI yet");
        assert_failed_launch_cleaned_up(&app, &fixture, parent_conversation_id, 2);
    });
}

// ---- Child-agent restoration (APP-5038) ------------------------------------

fn read_active_conversation_id(app: &App, session_id: TuiSessionId) -> AIConversationId {
    app.read(|ctx| {
        BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(session_id.surface_id())
            .expect("session has an active conversation")
            .id()
    })
}

/// Seeds a hydrated remote-child conversation under `parent_conversation_id`
/// with a stable run identity but no retained session, mimicking a
/// startup-hydrated orchestration child that has not yet been materialized.
fn seed_remote_child(
    app: &mut App,
    parent_conversation_id: AIConversationId,
    name: &str,
    run_id: &str,
) -> AIConversationId {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let child_id = history.start_new_child_conversation(
                warpui::EntityId::new(),
                name.to_owned(),
                parent_conversation_id,
                Some(Harness::Oz),
                true,
                ctx,
            );
            if let Some(conversation) = history.conversation_mut(&child_id) {
                conversation.mark_as_remote_child();
                conversation.set_run_id(run_id.to_owned());
            }
            child_id
        })
    })
}
fn restore_descendants(
    app: &mut App,
    parent_conversation_id: AIConversationId,
    root_session_id: TuiSessionId,
) {
    app.update(|ctx| {
        TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.restore_descendant_sessions(parent_conversation_id, root_session_id, ctx);
        });
    });
}

fn snapshot_child_ids(app: &App, selected: AIConversationId) -> Option<Vec<AIConversationId>> {
    app.read(|ctx| {
        TuiOrchestrationModel::as_ref(ctx)
            .snapshot(selected, ctx)
            .map(|snapshot| {
                snapshot
                    .children
                    .iter()
                    .map(|child| child.conversation_id)
                    .collect()
            })
    })
}

#[test]
fn restoring_parent_materializes_supported_descendant_sessions() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);

        let child_id = seed_remote_child(
            &mut app,
            parent_conversation_id,
            "cloud-child",
            "00000000-0000-0000-0000-000000000001",
        );
        let grandchild_id = seed_remote_child(
            &mut app,
            child_id,
            "cloud-grandchild",
            "00000000-0000-0000-0000-000000000002",
        );

        // Before restore: the descendants have no sessions, so the shared
        // snapshot filters them out and there is nothing navigable.
        assert_eq!(snapshot_child_ids(&app, parent_conversation_id), None);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1
        );

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);

        // After restore: parent + both descendants have sessions. The root
        // level shows only the direct child, which carries a subtree rollup
        // for the restored grandchild.
        assert_eq!(
            snapshot_child_ids(&app, parent_conversation_id),
            Some(vec![child_id])
        );
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            3
        );
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(parent_conversation_id, ctx)
                .expect("root level snapshot");
            assert_eq!(snapshot.anchor_conversation_id, parent_conversation_id);
            assert_eq!(snapshot.anchor_label, ORCHESTRATOR_TAB_LABEL);
            assert!(snapshot.anchor_status.is_some());
            assert!(snapshot.breadcrumbs.is_empty());
            let child = &snapshot.children[0];
            assert_eq!(
                child
                    .subtree_rollup
                    .as_ref()
                    .map(|rollup| rollup.descendant_count),
                Some(1),
                "the restored grandchild must roll up into the child's badge"
            );
        });

        // Selecting the group child re-anchors the bar to its level: the
        // grandchild becomes the row and a root breadcrumb leads back up.
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(child_id, ctx)
                .expect("drilled-in level snapshot");
            assert_eq!(snapshot.anchor_conversation_id, child_id);
            assert_eq!(snapshot.anchor_label, "cloud-child");
            assert_eq!(
                snapshot
                    .children
                    .iter()
                    .map(|child| child.conversation_id)
                    .collect::<Vec<_>>(),
                vec![grandchild_id]
            );
            assert_eq!(
                snapshot
                    .breadcrumbs
                    .iter()
                    .map(|breadcrumb| (breadcrumb.conversation_id, breadcrumb.label.as_str()))
                    .collect::<Vec<_>>(),
                vec![(parent_conversation_id, ORCHESTRATOR_TAB_LABEL)]
            );
        });

        // A grandchild leaf anchors its parent's level (same row).
        assert_eq!(
            snapshot_child_ids(&app, grandchild_id),
            Some(vec![grandchild_id])
        );

        // A nested grandchild keeps its own parent linkage in history.
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let grandchild = history.conversation(&grandchild_id).expect("grandchild");
            assert_eq!(
                history.resolved_parent_conversation_id_for_conversation(grandchild),
                Some(child_id)
            );
        });

        // Tab navigation resolves each restored session.
        app.update(|ctx| {
            let selected = TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.focus_conversation_session(grandchild_id, ctx)
            });
            assert!(selected.is_some());
        });
    });
}

#[test]
fn snapshot_keeps_flat_projection_with_multi_level_disabled() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(false);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);
        let child_id = seed_remote_child(
            &mut app,
            parent_conversation_id,
            "cloud-child",
            "00000000-0000-0000-0000-000000000001",
        );
        let grandchild_id = seed_remote_child(
            &mut app,
            child_id,
            "cloud-grandchild",
            "00000000-0000-0000-0000-000000000002",
        );
        restore_descendants(&mut app, parent_conversation_id, parent_session_id);

        // Flag off: every descendant renders as a flat sibling of the root
        // level, with no breadcrumbs, no anchor glyph, and no rollups.
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(grandchild_id, ctx)
                .expect("flat snapshot");
            assert_eq!(snapshot.anchor_conversation_id, parent_conversation_id);
            assert_eq!(snapshot.anchor_label, ORCHESTRATOR_TAB_LABEL);
            assert_eq!(snapshot.anchor_status, None);
            assert!(snapshot.breadcrumbs.is_empty());
            assert_eq!(
                snapshot
                    .children
                    .iter()
                    .map(|child| child.conversation_id)
                    .collect::<Vec<_>>(),
                vec![child_id, grandchild_id]
            );
            assert!(
                snapshot
                    .children
                    .iter()
                    .all(|child| child.subtree_rollup.is_none())
            );
        });
    });
}

#[test]
fn breadcrumbs_cap_at_root_plus_parent_at_depth_three() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let root_id = read_active_conversation_id(&app, parent_session_id);
        let alpha_id = seed_remote_child(
            &mut app,
            root_id,
            "alpha",
            "00000000-0000-0000-0000-000000000001",
        );
        let beta_id = seed_remote_child(
            &mut app,
            alpha_id,
            "beta",
            "00000000-0000-0000-0000-000000000002",
        );
        let gamma_id = seed_remote_child(
            &mut app,
            beta_id,
            "gamma",
            "00000000-0000-0000-0000-000000000003",
        );
        restore_descendants(&mut app, root_id, parent_session_id);

        // gamma is a leaf three levels down: the bar anchors its parent
        // (beta) and shows exactly two breadcrumbs — root plus parent's
        // parent — never the full ancestor chain.
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(gamma_id, ctx)
                .expect("depth-three snapshot");
            assert_eq!(snapshot.anchor_conversation_id, beta_id);
            assert_eq!(snapshot.anchor_label, "beta");
            assert_eq!(
                snapshot
                    .children
                    .iter()
                    .map(|child| child.conversation_id)
                    .collect::<Vec<_>>(),
                vec![gamma_id]
            );
            assert_eq!(
                snapshot
                    .breadcrumbs
                    .iter()
                    .map(|breadcrumb| (breadcrumb.conversation_id, breadcrumb.label.as_str()))
                    .collect::<Vec<_>>(),
                vec![(root_id, ORCHESTRATOR_TAB_LABEL), (alpha_id, "alpha"),]
            );
        });
    });
}

#[test]
fn sessionless_parents_are_filtered_from_chips_and_marked_non_navigable() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let root_session_id = add_dispatching_session(&mut app, &fixture, true);
        let root_id = read_active_conversation_id(&app, root_session_id);

        // A loaded but sessionless intermediate (e.g. a restored non-Oz local
        // child the TUI cannot materialize).
        let sessionless_parent_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "claude-mid".to_owned(),
                    root_id,
                    Some(Harness::Claude),
                    false,
                    ctx,
                )
            })
        });
        let (_, session_backed_child_id) = add_child_session(
            &mut app,
            &fixture,
            sessionless_parent_id,
            "session-backed-child",
        );
        let (_, leaf_id) = add_child_session(&mut app, &fixture, session_backed_child_id, "leaf");

        app.read(|ctx| {
            let model = TuiOrchestrationModel::as_ref(ctx);
            // The leaf anchors its session-backed parent; the sessionless
            // grandparent contributes no breadcrumb chip — only the root
            // remains, so ascent stays reachable.
            let snapshot = model.snapshot(leaf_id, ctx).expect("leaf level snapshot");
            assert_eq!(snapshot.anchor_conversation_id, session_backed_child_id);
            assert!(snapshot.anchor_navigable);
            assert_eq!(
                snapshot
                    .breadcrumbs
                    .iter()
                    .map(|breadcrumb| breadcrumb.conversation_id)
                    .collect::<Vec<_>>(),
                vec![root_id],
                "a sessionless parent must not become a breadcrumb chip"
            );

            // A leaf directly under the sessionless parent still frames that
            // level, but the anchor is marked non-navigable so keyboard
            // navigation and clicks skip it.
            let snapshot = model
                .snapshot(session_backed_child_id, ctx)
                .expect("mid level snapshot");
            assert_eq!(snapshot.anchor_conversation_id, session_backed_child_id);
        });

        // Remove the leaf so session-backed-child becomes a leaf whose parent
        // is the sessionless conversation: the bar anchors the sessionless
        // parent and marks it non-navigable.
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.kill_child_agent(leaf_id, ctx);
            });
        });
        app.read(|ctx| {
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(session_backed_child_id, ctx)
                .expect("sessionless-anchor snapshot");
            assert_eq!(snapshot.anchor_conversation_id, sessionless_parent_id);
            assert!(!snapshot.anchor_navigable);
            assert_eq!(
                snapshot
                    .breadcrumbs
                    .iter()
                    .map(|breadcrumb| breadcrumb.conversation_id)
                    .collect::<Vec<_>>(),
                vec![root_id]
            );
        });
    });
}

#[test]
fn adjacent_tree_conversation_walks_the_whole_tree_and_wraps() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let root_id = read_active_conversation_id(&app, parent_session_id);
        let child_id = seed_remote_child(
            &mut app,
            root_id,
            "cloud-child",
            "00000000-0000-0000-0000-000000000001",
        );
        let grandchild_id = seed_remote_child(
            &mut app,
            child_id,
            "cloud-grandchild",
            "00000000-0000-0000-0000-000000000002",
        );
        restore_descendants(&mut app, root_id, parent_session_id);

        app.read(|ctx| {
            let model = TuiOrchestrationModel::as_ref(ctx);
            // Tree order is root → child → grandchild, wrapping at the ends,
            // so Tab alone still reaches every agent at any depth.
            assert_eq!(
                model.adjacent_tree_conversation(root_id, TuiTabBarNavigationDirection::Next, ctx),
                Some(child_id)
            );
            assert_eq!(
                model.adjacent_tree_conversation(child_id, TuiTabBarNavigationDirection::Next, ctx),
                Some(grandchild_id)
            );
            assert_eq!(
                model.adjacent_tree_conversation(
                    grandchild_id,
                    TuiTabBarNavigationDirection::Next,
                    ctx
                ),
                Some(root_id)
            );
            assert_eq!(
                model.adjacent_tree_conversation(
                    root_id,
                    TuiTabBarNavigationDirection::Previous,
                    ctx
                ),
                Some(grandchild_id)
            );
        });
    });
}

#[test]
fn explicit_paging_is_tracked_per_level() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let root_id = read_active_conversation_id(&app, parent_session_id);
        let first_child_id = seed_remote_child(
            &mut app,
            root_id,
            "first-child",
            "00000000-0000-0000-0000-000000000001",
        );
        let second_child_id = seed_remote_child(
            &mut app,
            root_id,
            "second-child",
            "00000000-0000-0000-0000-000000000002",
        );
        let grandchild_id = seed_remote_child(
            &mut app,
            first_child_id,
            "grandchild",
            "00000000-0000-0000-0000-000000000003",
        );
        restore_descendants(&mut app, root_id, parent_session_id);

        // Page explicitly within the root level.
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.set_explicit_page(root_id, second_child_id, ctx);
            });
        });
        app.read(|ctx| {
            let root_level = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(root_id, ctx)
                .expect("root level snapshot");
            assert_eq!(root_level.page_anchor, Some(second_child_id));
            assert!(!root_level.reveal_selected);
            // The drilled-in level under first-child is unaffected: it keeps
            // automatic reveal with its own first tab as the page anchor.
            let drilled = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(first_child_id, ctx)
                .expect("drilled level snapshot");
            assert_eq!(drilled.anchor_conversation_id, first_child_id);
            assert_eq!(drilled.page_anchor, Some(grandchild_id));
            assert!(drilled.reveal_selected);
        });
    });
}

#[test]
fn kill_child_agent_subtree_removes_nested_descendants_with_the_child() {
    let _flag = FeatureFlag::MultiLevelOrchestration.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .active_conversation(parent_session_id.surface_id())
                .unwrap()
                .id()
        });
        let child_request = remote_request(parent_conversation_id);
        let (child_conversation_id, child_surface_id, _) = add_remote_child_session(
            &mut app,
            &fixture,
            parent_session_id,
            &child_request,
            "researcher".to_string(),
            Harness::Oz,
        );
        let child_session_id = app.read(|ctx| {
            TuiSessions::as_ref(ctx)
                .session_id_for_surface(child_surface_id)
                .expect("child session should be retained")
        });
        let grandchild_request = remote_request(child_conversation_id);
        let (grandchild_conversation_id, grandchild_surface_id, _) = add_remote_child_session(
            &mut app,
            &fixture,
            child_session_id,
            &grandchild_request,
            "nested-researcher".to_string(),
            Harness::Oz,
        );

        // Killing the group child tears down the whole subtree deepest-first:
        // no orphaned grandchild session or conversation remains.
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.kill_child_agent_subtree(child_conversation_id, ctx);
            });
        });

        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(history.conversation(&child_conversation_id).is_none());
            assert!(history.conversation(&grandchild_conversation_id).is_none());
            assert!(
                history.conversation(&parent_conversation_id).is_some(),
                "the parent must survive the subtree kill"
            );
            let sessions = TuiSessions::as_ref(ctx);
            assert!(sessions.session_id_for_surface(child_surface_id).is_none());
            assert!(
                sessions
                    .session_id_for_surface(grandchild_surface_id)
                    .is_none()
            );
        });
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1
        );
    });
}

#[test]
fn restored_remote_child_uses_authoritative_task_status() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);
        let run_id = "00000000-0000-0000-0000-000000000001";
        let child_id = seed_remote_child(&mut app, parent_conversation_id, "cloud-child", run_id);
        let task_id = run_id.parse().expect("hardcoded task ID parses");

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);
        app.update(|ctx| {
            let session_id = {
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                TuiSessions::as_ref(ctx)
                    .session_ids_by_conversation(history)
                    .get(&child_id)
                    .copied()
                    .expect("restored remote child has a session")
            };
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.apply_restored_remote_child_status(
                    session_id,
                    child_id,
                    task_id,
                    ConversationStatus::Success,
                    ctx,
                );
            });
        });

        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert_eq!(
                history
                    .conversation(&child_id)
                    .expect("restored remote child")
                    .status(),
                &ConversationStatus::Success
            );
            let session_id = TuiSessions::as_ref(ctx)
                .session_ids_by_conversation(history)
                .get(&child_id)
                .copied()
                .expect("restored remote child has a session");
            let TuiSessionView::Cloud(view) = TuiSessions::as_ref(ctx)
                .session(session_id)
                .expect("restored remote child session")
                .view()
            else {
                panic!("restored remote child uses a cloud view");
            };
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                view.as_ref(ctx).render(ctx),
                TuiRect::new(0, 0, 112, 24),
                ctx,
            );
            let lines = frame.buffer.to_lines();
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Cloud run succeeded"))
            );
            assert!(
                lines
                    .iter()
                    .all(|line| !line.contains("Cloud run in progress"))
            );
        });
    });
}
#[test]
fn restoring_parent_without_children_is_noop() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);

        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1
        );
        assert_eq!(snapshot_child_ids(&app, parent_conversation_id), None);
    });
}

#[test]
fn restoring_parent_twice_does_not_duplicate_child_sessions() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);
        let child_id = seed_remote_child(
            &mut app,
            parent_conversation_id,
            "cloud-child",
            "00000000-0000-0000-0000-000000000001",
        );

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            2
        );

        // A second restore of the same tree is idempotent: the child already
        // has a session, so no duplicate session or tab is created.
        restore_descendants(&mut app, parent_conversation_id, parent_session_id);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            2
        );
        assert_eq!(
            snapshot_child_ids(&app, parent_conversation_id),
            Some(vec![child_id])
        );
    });
}

#[test]
fn restore_skips_unsupported_or_malformed_children() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);

        // A supported remote sibling that should restore normally.
        let supported_id = seed_remote_child(
            &mut app,
            parent_conversation_id,
            "supported",
            "00000000-0000-0000-0000-000000000001",
        );
        // A remote child without a stable task/run identity.
        let no_identity_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let id = history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "no-identity".to_owned(),
                    parent_conversation_id,
                    Some(Harness::Oz),
                    true,
                    ctx,
                );
                history
                    .conversation_mut(&id)
                    .expect("child exists")
                    .mark_as_remote_child();
                id
            })
        });
        // An explicit local non-Oz harness child the TUI cannot display.
        let non_oz_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "claude-child".to_owned(),
                    parent_conversation_id,
                    Some(Harness::Claude),
                    false,
                    ctx,
                )
            })
        });
        // A shared-session viewer child with no matching TUI view.
        let shared_viewer_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let id = history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "shared-viewer".to_owned(),
                    parent_conversation_id,
                    Some(Harness::Oz),
                    false,
                    ctx,
                );
                history
                    .conversation_mut(&id)
                    .expect("child exists")
                    .set_is_viewing_shared_session(true);
                id
            })
        });

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);

        // Only the supported sibling materializes; the parent restore still
        // succeeds and the unsupported children are skipped.
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            2
        );
        assert_eq!(
            snapshot_child_ids(&app, parent_conversation_id),
            Some(vec![supported_id])
        );

        // The skipped children keep their history records (nothing deleted).
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            assert!(history.conversation(&no_identity_id).is_some());
            assert!(history.conversation(&non_oz_id).is_some());
            assert!(history.conversation(&shared_viewer_id).is_some());
        });
    });
}

#[test]
fn discard_restored_descendant_sessions_removes_projections_without_deleting_records() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);
        let child_id = seed_remote_child(
            &mut app,
            parent_conversation_id,
            "cloud-child",
            "00000000-0000-0000-0000-000000000001",
        );

        restore_descendants(&mut app, parent_conversation_id, parent_session_id);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            2
        );

        // When a different parent replaces the tree, the prior tree's restored
        // child-session projections are dropped without cancelling or deleting
        // the underlying conversation.
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.discard_restored_descendant_sessions(
                    parent_conversation_id,
                    parent_session_id,
                    ctx,
                );
            });
        });

        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1
        );
        app.read(|ctx| {
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_id)
                    .is_some(),
                "the child conversation record must be preserved after discard"
            );
        });
    });
}

#[test]
fn restored_local_oz_child_materializes_terminal_session_without_relaunch() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_dispatching_session(&mut app, &fixture, true);
        let parent_conversation_id = read_active_conversation_id(&app, parent_session_id);

        // Seed a hydrated local Oz child (no retained session yet).
        let child_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_child_conversation(
                    warpui::EntityId::new(),
                    "local-child".to_owned(),
                    parent_conversation_id,
                    Some(Harness::Oz),
                    false,
                    ctx,
                )
            })
        });

        // Materialize the child via the restore-only pieces on a fresh terminal
        // session: restore its transcript, then register it. This deliberately
        // does not call any launch/start-agent path, so the child is not
        // relaunched and no prompt is resent.
        let (child_view, child_manager) = add_test_terminal_session(&mut app, fixture.window_id);
        let child_session_id = app.update(|ctx| {
            TuiSessions::register_session(
                &fixture.sessions,
                child_view.clone(),
                child_manager,
                false,
                ctx,
            )
        });
        let child_conversation = app.read(|ctx| {
            BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .cloned()
                .expect("child conversation is hydrated")
        });
        app.update(|ctx| {
            child_view.update(ctx, |view, ctx| {
                view.restore_orchestrated_child_conversation(child_conversation, ctx);
            });
        });
        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.register_restored_local_oz_child_session(child_session_id, child_id, ctx);
            });
        });

        app.read(|ctx| {
            // The child is a full terminal session (not a lightweight cloud one).
            let session = TuiSessions::as_ref(ctx)
                .session(child_session_id)
                .expect("child session registered");
            assert!(matches!(session.view(), TuiSessionView::Terminal(_)));

            // It appears in the parent's orchestration snapshot with its
            // preserved agent name and parent linkage.
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(parent_conversation_id, ctx)
                .expect("child is navigable");
            let child = snapshot
                .children
                .iter()
                .find(|child| child.conversation_id == child_id)
                .expect("child has an orchestration tab");
            assert_eq!(child.label, "local-child");

            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let conversation = history.conversation(&child_id).expect("child conversation");
            assert_eq!(
                history.resolved_parent_conversation_id_for_conversation(conversation),
                Some(parent_conversation_id)
            );
        });
    });
}
