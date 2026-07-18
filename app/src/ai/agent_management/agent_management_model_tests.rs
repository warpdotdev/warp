use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_errors::report_if_error;
use warpui::{App, EntityId, ModelHandle, SingletonEntity};

use super::AgentNotificationsModel;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent_management::notifications::{
    NotificationCategory, NotificationFilter, NotificationOrigin, NotificationSourceAgent,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::BlocklistAIHistoryModel;

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
            assert!(model
                .notifications()
                .has_unread_for_terminal_view(terminal_view_id));
        });
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
    assert!(ConversationStatus::Blocked {
        blocked_action: "approve diff".to_owned(),
    }
    .should_trigger_notification());
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
