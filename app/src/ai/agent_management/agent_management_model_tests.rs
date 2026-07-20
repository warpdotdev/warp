use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_errors::report_if_error;
use warpui::{App, EntityId, ModelHandle, SingletonEntity};

use super::AgentNotificationsModel;
use crate::BlocklistAIHistoryModel;
use crate::ai::active_agent_views_model::{ActiveAgentViewsEvent, ActiveAgentViewsModel};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent_management::notifications::{
    NotificationCategory, NotificationFilter, NotificationOrigin, NotificationSourceAgent,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::settings::AISettings;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::event::{
    CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::WorkspaceRegistry;

fn setup_app(
    app: &mut App,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    ModelHandle<AgentNotificationsModel>,
) {
    initialize_settings_for_tests(app);
    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));
    // Registered after the history model since it subscribes to history events; the
    // notifications model reads it to suppress completion notifications when a prompt is queued.
    app.add_singleton_model(crate::ai::blocklist::QueuedQueryModel::new);
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    let notifications = app.add_singleton_model(AgentNotificationsModel::new);
    (history, notifications)
}

fn make_pr_artifact(url: &str, branch: &str) -> Artifact {
    Artifact::PullRequest {
        url: url.to_string(),
        branch: branch.to_string(),
        repo: None,
        number: None,
    }
}

fn make_plan_artifact(doc_uid: &str, title: &str) -> Artifact {
    Artifact::Plan {
        document_uid: doc_uid.to_string(),
        notebook_uid: None,
        title: Some(title.to_string()),
    }
}

#[test]
fn artifact_event_accumulates_into_pending() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                artifact: make_pr_artifact("https://github.com/org/repo/pull/42", "feature-branch"),
            });
        });

        notifications.read(&app, |model, _| {
            let pending = model.pending_artifacts.get(&conversation_id).unwrap();
            assert_eq!(pending.len(), 1);
            assert!(matches!(&pending[0], Artifact::PullRequest { branch, .. } if branch == "feature-branch"));
        });
    });
}

#[test]
fn multiple_artifacts_accumulated_across_turns() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                artifact: make_plan_artifact("doc-1", "My Plan"),
            });
        });
        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                artifact: make_pr_artifact("https://github.com/org/repo/pull/1", "main"),
            });
        });

        notifications.read(&app, |model, _| {
            let pending = model.pending_artifacts.get(&conversation_id).unwrap();
            assert_eq!(pending.len(), 2);
            assert!(matches!(&pending[0], Artifact::Plan { title: Some(t), .. } if t == "My Plan"));
            assert!(matches!(&pending[1], Artifact::PullRequest { .. }));
        });
    });
}

#[test]
fn add_notification_tracks_unread_activity_when_in_app_notifications_are_hidden() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            report_if_error!(settings.show_agent_notifications.set_value(false, ctx));
        });

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Agent task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::Oz { is_ambient: false },
                NotificationOrigin::Conversation(conversation_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
        });

        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                1
            );
            assert!(
                model
                    .notifications()
                    .has_unread_for_terminal_view(terminal_view_id)
            );
        });
        assert_eq!(app.dock_badge_count(), 1);
    });
}

#[test]
fn dock_badge_is_seeded_when_notifications_model_starts() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        setup_app(&mut app);

        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn dock_badge_updates_after_add_and_item_read() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Claude task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
                NotificationOrigin::CLISession(terminal_view_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 1);

        let notification_id = notifications.read(&app, |model, _| {
            model
                .notifications()
                .items_filtered(NotificationFilter::All)
                .next()
                .expect("notification was just added")
                .id
        });
        notifications.update(&mut app, |model, ctx| {
            model.mark_item_read(notification_id, ctx);
        });

        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn dock_badge_counts_local_warp_agent_completion_and_clears_when_read() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Agent task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::Oz { is_ambient: false },
                NotificationOrigin::Conversation(conversation_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 1);

        let notification_id = notifications.read(&app, |model, _| {
            let item = model
                .notifications()
                .items_filtered(NotificationFilter::All)
                .next()
                .expect("local Warp agent notification was just added");
            assert!(!item.is_read);
            item.id
        });
        notifications.update(&mut app, |model, ctx| {
            model.mark_item_read(notification_id, ctx);
        });

        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn dock_badge_updates_after_terminal_view_read_and_mark_all_read() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let terminal_a = EntityId::new();
        let terminal_b = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Codex task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Codex,
                    is_ambient: false,
                },
                NotificationOrigin::CLISession(terminal_a),
                terminal_a,
                vec![],
                None,
                ctx,
            );
            model.add_notification(
                "OpenCode task".to_owned(),
                "Waiting for input.".to_owned(),
                NotificationCategory::Request,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::OpenCode,
                    is_ambient: false,
                },
                NotificationOrigin::CLISession(terminal_b),
                terminal_b,
                vec![],
                None,
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 2);

        notifications.update(&mut app, |model, ctx| {
            model.mark_items_from_terminal_view_read(terminal_a, ctx);
        });
        assert_eq!(app.dock_badge_count(), 1);

        notifications.update(&mut app, |model, ctx| {
            model.mark_all_items_read(ctx);
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn dock_badge_updates_after_source_removal_and_active_view_close_cleanup() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Auggie task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Auggie,
                    is_ambient: false,
                },
                NotificationOrigin::CLISession(terminal_view_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 1);

        notifications.update(&mut app, |model, ctx| {
            model.remove_notification_by_source(
                NotificationOrigin::CLISession(terminal_view_id),
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 0);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Claude task".to_owned(),
                "Waiting for input.".to_owned(),
                NotificationCategory::Request,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
                NotificationOrigin::Conversation(conversation_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 1);

        notifications.update(&mut app, |model, ctx| {
            model.handle_active_agent_views_changed(
                &ActiveAgentViewsEvent::ConversationClosed { conversation_id },
                ctx,
            );
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn flush_drains_pending_artifacts() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                artifact: make_pr_artifact("https://github.com/org/repo/pull/1", "branch-1"),
            });
        });

        notifications.update(&mut app, |model, _| {
            let artifacts = model.flush_pending_artifacts(conversation_id);
            assert_eq!(artifacts.len(), 1);
            assert!(matches!(&artifacts[0], Artifact::PullRequest { branch, .. } if branch == "branch-1"));
        });

        notifications.read(&app, |model, _| {
            assert!(!model.pending_artifacts.contains_key(&conversation_id));
        });
    });
}

#[test]
fn flush_returns_empty_vec_when_no_artifacts() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);

        let conversation_id = AIConversationId::new();

        notifications.update(&mut app, |model, _| {
            let artifacts = model.flush_pending_artifacts(conversation_id);
            assert!(artifacts.is_empty());
        });
    });
}

#[test]
fn deletion_cleans_up_pending_artifacts() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);

        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                artifact: make_pr_artifact("https://github.com/org/repo/pull/1", "branch-1"),
            });
        });

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::DeletedConversation {
                terminal_surface_id: terminal_view_id,
                conversation_id,
                conversation_title: None,
                run_id: None,
            });
        });

        notifications.read(&app, |model, _| {
            assert!(!model.pending_artifacts.contains_key(&conversation_id));
        });
    });
}

#[test]
fn separate_conversations_have_independent_pending_artifacts() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);

        let conv_a = AIConversationId::new();
        let conv_b = AIConversationId::new();
        let terminal_view_id = EntityId::new();

        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id: conv_a,
                artifact: make_pr_artifact("https://github.com/org/repo/pull/1", "branch-a"),
            });
        });
        history.update(&mut app, |_: &mut BlocklistAIHistoryModel, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                terminal_surface_id: terminal_view_id,
                conversation_id: conv_b,
                artifact: make_plan_artifact("doc-b", "Plan B"),
            });
        });

        notifications.update(&mut app, |model, _| {
            let a = model.flush_pending_artifacts(conv_a);
            assert_eq!(a.len(), 1);
            assert!(matches!(&a[0], Artifact::PullRequest { branch, .. } if branch == "branch-a"));

            let b = model.flush_pending_artifacts(conv_b);
            assert_eq!(b.len(), 1);
            assert!(matches!(&b[0], Artifact::Plan { title: Some(t), .. } if t == "Plan B"));
        });
    });
}

// should_trigger_notification: pure-function tests pinning which statuses
// fire user-facing notifications. Terminal-error and blocked surface;
// in-progress, waiting-for-events, and user-cancelled do not.

#[test]
fn should_trigger_notification_returns_true_for_success() {
    assert!(ConversationStatus::Success.should_trigger_notification());
}

#[test]
fn should_trigger_notification_returns_true_for_blocked() {
    assert!(
        ConversationStatus::Blocked {
            blocked_action: "approve diff".to_owned(),
        }
        .should_trigger_notification()
    );
}

#[test]
fn should_trigger_notification_returns_true_for_error() {
    assert!(ConversationStatus::Error.should_trigger_notification());
}

#[test]
fn should_trigger_notification_returns_false_for_in_progress() {
    assert!(!ConversationStatus::InProgress.should_trigger_notification());
}

#[test]
fn should_trigger_notification_returns_false_for_waiting_for_events() {
    assert!(!ConversationStatus::WaitingForEvents.should_trigger_notification());
}

#[test]
fn should_trigger_notification_returns_false_for_cancelled() {
    assert!(!ConversationStatus::Cancelled.should_trigger_notification());
}

// Mailbox suppression for non-terminal status updates. In App::test the
// `is_conversation_open` gate always returns false, so the
// WaitingForEvents and InProgress arms both clear stale notifications
// regardless of status; this still pins the user-visible contract that
// no stale "Task completed" toast survives a non-terminal transition.

/// Disables `show_agent_notifications` so subsequent `add_notification`
/// calls skip the `send_telemetry_from_ctx!` branch — the test app does
/// not register a `TelemetryContextProvider` singleton and the macro
/// would otherwise panic.
fn disable_telemetry_path(app: &mut App) {
    AISettings::handle(app).update(app, |settings, ctx| {
        report_if_error!(settings.show_agent_notifications.set_value(false, ctx));
    });
}

fn start_cli_agent_session(app: &mut App, terminal_view_id: EntityId, agent: CLIAgent) {
    CLIAgentSessionsModel::handle(app).update(app, |sessions, ctx| {
        sessions.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext::default(),
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input: false,
                listener: None,
                plugin_version: None,
                remote_host: None,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
            },
            ctx,
        );
    });
}

fn emit_cli_agent_event(
    app: &mut App,
    terminal_view_id: EntityId,
    agent: CLIAgent,
    event_type: CLIAgentEventType,
    payload: CLIAgentEventPayload,
) {
    let event = CLIAgentEvent {
        v: 1,
        agent,
        event: event_type,
        session_id: Some("test-session".to_owned()),
        cwd: None,
        project: None,
        payload,
        source: CLIAgentEventSource::RichPlugin,
    };

    CLIAgentSessionsModel::handle(app).update(app, |sessions, ctx| {
        sessions.update_from_event(terminal_view_id, &event, ctx);
    });
}

fn assert_cli_notification(
    notifications: &ModelHandle<AgentNotificationsModel>,
    app: &App,
    terminal_view_id: EntityId,
    expected_agent: CLIAgent,
    expected_category: NotificationCategory,
    expected_title: &str,
    expected_message: &str,
) {
    notifications.read(app, |model, _| {
        let item = model
            .notifications()
            .items_filtered(NotificationFilter::All)
            .find(|item| item.terminal_view_id == terminal_view_id)
            .expect("notification should exist for terminal view");
        assert_eq!(
            item.origin,
            NotificationOrigin::CLISession(terminal_view_id)
        );
        assert_eq!(item.category, expected_category);
        assert_eq!(item.title, expected_title);
        assert_eq!(item.message, expected_message);
        assert!(!item.is_read);

        let NotificationSourceAgent::CLI { agent, is_ambient } = item.agent else {
            panic!("expected CLI notification source");
        };
        assert_eq!(agent, expected_agent);
        assert!(!is_ambient);
    });
}

/// Pre-populates a `Complete` notification for `conversation_id` so that a
/// subsequent non-terminal status update has something to clear.
fn seed_stale_notification(
    notifications: &ModelHandle<AgentNotificationsModel>,
    app: &mut App,
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
) {
    notifications.update(app, |model, ctx| {
        model.add_notification(
            "Agent task".to_owned(),
            "Task completed.".to_owned(),
            NotificationCategory::Complete,
            NotificationSourceAgent::Oz { is_ambient: false },
            NotificationOrigin::Conversation(conversation_id),
            terminal_view_id,
            vec![],
            None,
            ctx,
        );
    });
}

#[test]
fn cli_agent_stop_events_add_completed_notifications_and_dock_badge_for_supported_agents() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        for (idx, (agent, expected_title, expected_message)) in [
            (
                CLIAgent::Codex,
                "Codex completed",
                "Notification from Codex",
            ),
            (CLIAgent::Claude, "Claude Code completed", "Task completed."),
            (CLIAgent::OpenCode, "OpenCode completed", "Task completed."),
        ]
        .into_iter()
        .enumerate()
        {
            let terminal_view_id = EntityId::new();
            start_cli_agent_session(&mut app, terminal_view_id, agent);
            emit_cli_agent_event(
                &mut app,
                terminal_view_id,
                agent,
                CLIAgentEventType::Stop,
                CLIAgentEventPayload::default(),
            );

            assert_eq!(app.dock_badge_count(), idx + 1);
            assert_cli_notification(
                &notifications,
                &app,
                terminal_view_id,
                agent,
                NotificationCategory::Complete,
                expected_title,
                expected_message,
            );
        }

        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                3
            );
        });
        assert_eq!(app.dock_badge_count(), 3);
    });
}

#[test]
fn cli_agent_question_events_badge_needs_attention_for_supported_agents() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        for (idx, (agent, expected_title)) in [
            (CLIAgent::Codex, "Codex needs attention"),
            (CLIAgent::Claude, "Claude Code needs attention"),
            (CLIAgent::OpenCode, "OpenCode needs attention"),
        ]
        .into_iter()
        .enumerate()
        {
            let terminal_view_id = EntityId::new();
            start_cli_agent_session(&mut app, terminal_view_id, agent);
            emit_cli_agent_event(
                &mut app,
                terminal_view_id,
                agent,
                CLIAgentEventType::QuestionAsked,
                CLIAgentEventPayload::default(),
            );

            assert_eq!(app.dock_badge_count(), idx + 1);
            assert_cli_notification(
                &notifications,
                &app,
                terminal_view_id,
                agent,
                NotificationCategory::Request,
                expected_title,
                "Waiting for your answer",
            );
        }

        assert_eq!(app.dock_badge_count(), 3);
    });
}

#[test]
fn cli_agent_permission_request_summary_is_request_notification_and_counts_in_dock_badge() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        for (idx, agent) in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::OpenCode]
            .into_iter()
            .enumerate()
        {
            let terminal_view_id = EntityId::new();
            let summary = format!("{} wants approval", agent.display_name());
            start_cli_agent_session(&mut app, terminal_view_id, agent);
            emit_cli_agent_event(
                &mut app,
                terminal_view_id,
                agent,
                CLIAgentEventType::PermissionRequest,
                CLIAgentEventPayload {
                    summary: Some(summary.clone()),
                    ..Default::default()
                },
            );

            assert_eq!(app.dock_badge_count(), idx + 1);
            assert_cli_notification(
                &notifications,
                &app,
                terminal_view_id,
                agent,
                NotificationCategory::Request,
                &summary,
                &summary,
            );
        }

        assert_eq!(app.dock_badge_count(), 3);
    });
}

#[test]
fn cli_agent_in_progress_event_clears_stale_notification_and_dock_badge() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let terminal_view_id = EntityId::new();
        start_cli_agent_session(&mut app, terminal_view_id, CLIAgent::Codex);
        emit_cli_agent_event(
            &mut app,
            terminal_view_id,
            CLIAgent::Codex,
            CLIAgentEventType::Stop,
            CLIAgentEventPayload::default(),
        );
        assert_eq!(app.dock_badge_count(), 1);

        emit_cli_agent_event(
            &mut app,
            terminal_view_id,
            CLIAgent::Codex,
            CLIAgentEventType::PromptSubmit,
            CLIAgentEventPayload {
                query: Some("continue".to_owned()),
                ..Default::default()
            },
        );

        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                0
            );
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn waiting_for_events_clears_stale_notification_and_adds_none() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let conversation = AIConversation::new(false, false);
        let conversation_id = conversation.id();
        let terminal_view_id = EntityId::new();
        history.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        seed_stale_notification(&notifications, &mut app, conversation_id, terminal_view_id);
        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                1,
                "precondition: one stale notification queued"
            );
        });

        history.update(&mut app, |model, ctx| {
            let conv = model
                .conversation_mut(&conversation_id)
                .expect("conversation was just restored");
            conv.update_status(ConversationStatus::WaitingForEvents, terminal_view_id, ctx);
        });

        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                0,
                "WaitingForEvents must clear stale notifications and add no new toast"
            );
        });
    });
}

#[test]
fn in_progress_resume_clears_stale_notification_and_adds_none() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let conversation = AIConversation::new(false, false);
        let conversation_id = conversation.id();
        let terminal_view_id = EntityId::new();
        history.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        // First move the conversation into WaitingForEvents, then back into
        // InProgress. The second transition is the resume signal that
        // PRODUCT.md (18) requires not to fire a notification.
        history.update(&mut app, |model, ctx| {
            let conv = model
                .conversation_mut(&conversation_id)
                .expect("conversation was just restored");
            conv.update_status(ConversationStatus::WaitingForEvents, terminal_view_id, ctx);
        });

        seed_stale_notification(&notifications, &mut app, conversation_id, terminal_view_id);
        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                1,
                "precondition: one stale notification queued before the resume transition"
            );
        });

        history.update(&mut app, |model, ctx| {
            let conv = model
                .conversation_mut(&conversation_id)
                .expect("conversation still exists");
            conv.update_status(ConversationStatus::InProgress, terminal_view_id, ctx);
        });

        notifications.read(&app, |model, _| {
            assert_eq!(
                model
                    .notifications()
                    .filtered_count(NotificationFilter::All),
                0,
                "WaitingForEvents → InProgress resume must not fire a notification \
                 (covers PRODUCT.md (18))"
            );
        });
    });
}

#[test]
fn dock_badge_counts_belled_terminal_once_and_clears_when_viewed() {
    App::test((), |mut app| async move {
        let (_history, notifications) = setup_app(&mut app);

        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.record_terminal_bell(terminal_view_id, ctx);
            model.record_terminal_bell(terminal_view_id, ctx);
        });
        assert_eq!(app.dock_badge_count(), 1);

        // Viewing the terminal clears its bell entry even with HOA notifications disabled.
        notifications.update(&mut app, |model, ctx| {
            model.mark_items_from_terminal_view_read(terminal_view_id, ctx);
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn dock_badge_dedupes_bell_and_notification_for_same_terminal() {
    App::test((), |mut app| async move {
        let _guard = FeatureFlag::HOANotifications.override_enabled(true);
        let (_history, notifications) = setup_app(&mut app);
        disable_telemetry_path(&mut app);

        let terminal_view_id = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.add_notification(
                "Claude task".to_owned(),
                "Task completed.".to_owned(),
                NotificationCategory::Complete,
                NotificationSourceAgent::CLI {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
                NotificationOrigin::CLISession(terminal_view_id),
                terminal_view_id,
                vec![],
                None,
                ctx,
            );
            model.record_terminal_bell(terminal_view_id, ctx);
        });
        // The same terminal has an unread notification and an unviewed bell: badge once.
        assert_eq!(app.dock_badge_count(), 1);

        let other_terminal = EntityId::new();
        notifications.update(&mut app, |model, ctx| {
            model.record_terminal_bell(other_terminal, ctx);
        });
        assert_eq!(app.dock_badge_count(), 2);

        // Viewing the first terminal clears both its bell and its unread notification.
        notifications.update(&mut app, |model, ctx| {
            model.mark_items_from_terminal_view_read(terminal_view_id, ctx);
        });
        assert_eq!(app.dock_badge_count(), 1);

        notifications.update(&mut app, |model, ctx| {
            model.clear_terminal_bell(other_terminal, ctx);
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}

#[test]
fn mark_all_items_read_clears_belled_terminals() {
    App::test((), |mut app| async move {
        let (_history, notifications) = setup_app(&mut app);

        notifications.update(&mut app, |model, ctx| {
            model.record_terminal_bell(EntityId::new(), ctx);
            model.record_terminal_bell(EntityId::new(), ctx);
        });
        assert_eq!(app.dock_badge_count(), 2);

        notifications.update(&mut app, |model, ctx| {
            model.mark_all_items_read(ctx);
        });
        assert_eq!(app.dock_badge_count(), 0);
    });
}
