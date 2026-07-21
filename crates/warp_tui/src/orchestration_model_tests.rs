use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryModel, CloudAgentStartupBlocker, CloudAgentStartupIssue,
    ConversationStatus, Harness, OrchestrationEventStreamerEvent, StartAgentExecutionMode,
    StartAgentExecutor, StartAgentExecutorEvent, StartAgentOutcome, StartAgentRequest,
    register_tui_session_view_test_singletons,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle, ReadModel, SingletonEntity as _, UpdateModel};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect, text_width};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, TuiView as _, TypedActionView as _, WindowId};

use super::TuiOrchestrationModel;
use crate::cloud_run::TuiCloudRunStartup;
use crate::cloud_run_view::{TuiCloudRunAction, TuiCloudRunView};
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessionView, TuiSessions};
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
        assert_error_containing(outcome, "aren't supported in the Warp TUI yet");
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
                model.set_explicit_page(second_child_id, ctx);
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
