use chrono::{DateTime, Duration, Utc};
use instant::Instant;
use persistence::model::AgentConversationData;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use warp_core::features::FeatureFlag;
use warpui::{App, EntityId};

use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::task::{TaskCreatorInfo, TaskStatusMessage};
use crate::ai::ambient_agents::AgentConfigSnapshot;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskState};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::auth::AuthStateProvider;
use crate::test_util::ai_agent_tasks::{create_api_task, create_message};

use super::{
    AgentConversationsModel, AgentConversationsModelEvent, AgentManagementFilters,
    AgentRunDisplayStatus, ArtifactFilter, ConversationMetadata, ConversationOrTask,
    EnvironmentFilter, HarnessFilter, OwnerFilter, StatusFilter, TaskFetchState,
    MAX_PERSONAL_TASKS, MAX_TEAM_TASKS,
};
use crate::ai::ambient_agents::task::HarnessConfig;
use warp_cli::agent::Harness;

/// Creates a test task with specified creator UID and updated_at time
fn create_test_task(
    task_id: &str,
    creator_uid: &str,
    updated_at: DateTime<Utc>,
) -> AmbientAgentTask {
    AmbientAgentTask {
        task_id: task_id.parse().unwrap(),
        parent_run_id: None,
        title: format!("Task {task_id}"),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: updated_at,
        started_at: Some(updated_at),
        updated_at,
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskCreatorInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: Some(format!("User {creator_uid}")),
        }),
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
fn test_conversation_status_update_emits_conversation_updated() {
    App::test((), |mut app| async move {
        let _interactive_management_guard =
            FeatureFlag::InteractiveConversationManagementView.override_enabled(true);
        let agent_model = app.add_singleton_model(|_| create_test_model());
        let saw_conversation_updated = Arc::new(AtomicBool::new(false));

        app.update(|ctx| {
            let saw_conversation_updated = saw_conversation_updated.clone();
            ctx.subscribe_to_model(&agent_model, move |_, event, _| {
                if matches!(event, AgentConversationsModelEvent::ConversationUpdated) {
                    saw_conversation_updated.store(true, Ordering::SeqCst);
                }
            });
        });

        agent_model.update(&mut app, |model, ctx| {
            model.handle_history_event(
                &BlocklistAIHistoryEvent::UpdatedConversationStatus {
                    conversation_id: AIConversationId::new(),
                    terminal_view_id: EntityId::new(),
                    is_restored: false,
                },
                ctx,
            );
        });

        assert!(saw_conversation_updated.load(Ordering::SeqCst));
    });
}

#[test]
fn test_display_status_uses_setup_task_states() {
    App::test((), |mut app| async move {
        let now = Utc::now();
        let test_cases = [
            (
                AmbientAgentTaskState::Queued,
                AgentRunDisplayStatus::TaskQueued,
            ),
            (
                AmbientAgentTaskState::Pending,
                AgentRunDisplayStatus::TaskPending,
            ),
            (
                AmbientAgentTaskState::Claimed,
                AgentRunDisplayStatus::TaskClaimed,
            ),
        ];

        app.update(|ctx| {
            for (index, (task_state, expected_status)) in test_cases.into_iter().enumerate() {
                let mut task = create_test_task(&make_uuid(index + 4000), "user-a", now);
                task.state = task_state;
                assert_eq!(
                    AgentRunDisplayStatus::from_task(&task, ctx),
                    expected_status
                );
            }
        });
    });
}

#[test]
fn test_display_status_uses_matching_conversation_for_in_progress_task() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        let task_id = make_uuid(4003);

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: Some(task_id.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        let mut task = create_test_task(&task_id, "user-a", now);
        task.state = AmbientAgentTaskState::InProgress;

        app.update(|ctx| {
            let display_status = AgentRunDisplayStatus::from_task(&task, ctx);
            assert_eq!(display_status, AgentRunDisplayStatus::ConversationSucceeded);
            assert_eq!(display_status.status_filter(), StatusFilter::Done);
            assert!(!display_status.is_cancellable());
            assert!(!display_status.is_working());
        });
    });
}

#[test]
fn test_display_status_updates_when_blocked_conversation_resumes() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        let task_id = make_uuid(4006);

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: Some(task_id.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Blocked {
                    blocked_action: "waiting for approval".to_string(),
                },
                ctx,
            );
        });

        let mut task = create_test_task(&task_id, "user-a", now);
        task.state = AmbientAgentTaskState::InProgress;

        app.update(|ctx| {
            let display_status = AgentRunDisplayStatus::from_task(&task, ctx);
            assert!(matches!(
                display_status,
                AgentRunDisplayStatus::ConversationBlocked { .. }
            ));
            assert_eq!(display_status.status_filter(), StatusFilter::Failed);
            assert!(!display_status.is_cancellable());
        });

        history_model.update(&mut app, |model, ctx| {
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });

        app.update(|ctx| {
            let display_status = AgentRunDisplayStatus::from_task(&task, ctx);
            assert_eq!(
                display_status,
                AgentRunDisplayStatus::ConversationInProgress
            );
            assert_eq!(display_status.status_filter(), StatusFilter::Working);
            assert!(display_status.is_cancellable());
            assert!(display_status.is_working());
        });
    });
}

#[test]
fn test_display_status_terminal_task_state_overrides_matching_conversation() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        let task_id = make_uuid(4004);

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: Some(task_id.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Error,
                ctx,
            );
        });

        let mut task = create_test_task(&task_id, "user-a", now);
        task.state = AmbientAgentTaskState::Succeeded;

        app.update(|ctx| {
            assert_eq!(
                AgentRunDisplayStatus::from_task(&task, ctx),
                AgentRunDisplayStatus::TaskSucceeded
            );
        });
    });
}

#[test]
fn test_status_filter_uses_display_status_for_task_backed_conversations() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let terminal_view_id = EntityId::new();
        let task_id = make_uuid(4005);

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: Some(task_id.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::Success,
                ctx,
            );
        });

        let mut model = create_test_model();
        let mut task = create_test_task(&task_id, "user-a", now);
        task.state = AmbientAgentTaskState::InProgress;
        model.tasks.insert(task.task_id, task.clone());
        model.conversations.insert(
            conversation_id,
            create_test_conversation_metadata(conversation_id, "Conversation"),
        );

        app.update(|ctx| {
            let done_items: Vec<_> = model
                .get_tasks_and_conversations(
                    &AgentManagementFilters {
                        owners: OwnerFilter::All,
                        status: StatusFilter::Done,
                        ..Default::default()
                    },
                    ctx,
                )
                .collect();
            assert_eq!(done_items.len(), 1);
            assert!(matches!(
                done_items.first(),
                Some(ConversationOrTask::Task(_))
            ));

            let working_items: Vec<_> = model
                .get_tasks_and_conversations(
                    &AgentManagementFilters {
                        owners: OwnerFilter::All,
                        status: StatusFilter::Working,
                        ..Default::default()
                    },
                    ctx,
                )
                .collect();
            assert!(working_items.is_empty());
        });
    });
}

/// Helper to generate a unique UUID for task IDs
fn make_uuid(index: usize) -> String {
    format!("550e8400-e29b-41d4-a716-{:012}", index)
}

fn create_test_model() -> AgentConversationsModel {
    AgentConversationsModel {
        tasks: HashMap::new(),
        conversations: HashMap::new(),
        in_flight_poll_abort_handle: None,
        next_poll_abort_handle: None,
        active_data_consumers_per_window: HashMap::new(),
        has_finished_initial_load: false,
        task_fetch_state: Default::default(),
    }
}

fn create_test_conversation_metadata(
    conversation_id: AIConversationId,
    title: &str,
) -> ConversationMetadata {
    ConversationMetadata {
        nav_data: ConversationNavigationData {
            id: conversation_id,
            title: title.to_string(),
            initial_query: None,
            last_updated: chrono::Local::now(),
            terminal_view_id: None,
            window_id: None,
            pane_view_locator: None,
            initial_working_directory: None,
            latest_working_directory: None,
            is_selected: false,
            is_in_active_pane: false,
            is_closed: false,
            server_conversation_token: None,
        },
    }
}

fn create_restored_conversation(
    conversation_id: AIConversationId,
    root_task_id: &str,
    conversation_data: AgentConversationData,
) -> AIConversation {
    let task = create_api_task(
        root_task_id,
        vec![create_message(
            &format!("{root_task_id}-message"),
            root_task_id,
        )],
    );

    AIConversation::new_restored(conversation_id, vec![task], Some(conversation_data))
        .expect("restored conversation should build")
}

fn all_owner_filters() -> AgentManagementFilters {
    AgentManagementFilters {
        owners: OwnerFilter::All,
        ..Default::default()
    }
}

#[test]
fn test_eviction_protects_personal_from_team_overflow() {
    // Add 50 old personal tasks + 600 new team tasks
    // After eviction: all 50 personal remain, only 300 team remain
    let current_user = "user-personal";
    let team_user = "user-team";
    let now = Utc::now();

    let mut model = create_test_model();

    // Add 50 old personal tasks
    for i in 0..50 {
        let task = create_test_task(&make_uuid(i), current_user, now - Duration::days(30));
        model.tasks.insert(task.task_id, task);
    }

    // Add 600 new team tasks
    for i in 50..650 {
        let task = create_test_task(&make_uuid(i), team_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    model.enforce_task_cap(current_user);

    // Count personal vs team
    let personal_count = model
        .tasks
        .values()
        .filter(|t| t.creator.as_ref().is_some_and(|c| c.uid == current_user))
        .count();
    let team_count = model.tasks.len() - personal_count;

    // All 50 personal tasks should remain
    assert_eq!(personal_count, 50, "all personal tasks should remain");
    // Team tasks should be capped at MAX_TEAM_TASKS
    assert_eq!(team_count, MAX_TEAM_TASKS, "team tasks should be capped");
}

#[test]
fn test_eviction_caps_each_group_independently() {
    // Add 250 personal + 350 team
    // After eviction: 200 personal + 300 team
    let current_user = "user-personal";
    let team_user = "user-team";
    let now = Utc::now();

    let mut model = create_test_model();

    // Add 250 personal tasks
    for i in 0..250 {
        let task = create_test_task(&make_uuid(i), current_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    // Add 350 team tasks
    for i in 250..600 {
        let task = create_test_task(&make_uuid(i), team_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    model.enforce_task_cap(current_user);

    // Count personal vs team
    let personal_count = model
        .tasks
        .values()
        .filter(|t| t.creator.as_ref().is_some_and(|c| c.uid == current_user))
        .count();
    let team_count = model.tasks.len() - personal_count;

    // Personal capped at MAX_PERSONAL_TASKS
    assert_eq!(
        personal_count, MAX_PERSONAL_TASKS,
        "personal tasks should be capped"
    );
    // Team capped at MAX_TEAM_TASKS
    assert_eq!(team_count, MAX_TEAM_TASKS, "team tasks should be capped");
}

#[test]
fn test_eviction_removes_oldest_within_group() {
    let current_user = "user-personal";
    let now = Utc::now();

    let mut model = create_test_model();

    // Add 250 personal tasks with different timestamps
    // Newer tasks have lower index (i.e., index 0 is newest)
    for i in 0..250 {
        let task = create_test_task(&make_uuid(i), current_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    // Add 350 team tasks (to trigger eviction)
    let team_user = "user-team";
    for i in 250..600 {
        let task = create_test_task(&make_uuid(i), team_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    model.enforce_task_cap(current_user);

    // The 200 newest personal tasks should remain (indices 0-199)
    for i in 0..MAX_PERSONAL_TASKS {
        let task_id: AmbientAgentTaskId = make_uuid(i).parse().unwrap();
        assert!(
            model.tasks.contains_key(&task_id),
            "newest personal task {i} should remain"
        );
    }

    // The oldest personal tasks should be evicted (indices 200-249)
    for i in MAX_PERSONAL_TASKS..250 {
        let task_id: AmbientAgentTaskId = make_uuid(i).parse().unwrap();
        assert!(
            !model.tasks.contains_key(&task_id),
            "oldest personal task {i} should be evicted"
        );
    }
}

#[test]
fn test_eviction_noop_when_under_cap() {
    let current_user = "user-personal";
    let team_user = "user-team";
    let now = Utc::now();

    let mut model = create_test_model();

    // Add 100 personal + 100 team (well under cap)
    for i in 0..100 {
        let task = create_test_task(&make_uuid(i), current_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }
    for i in 100..200 {
        let task = create_test_task(&make_uuid(i), team_user, now - Duration::hours(i as i64));
        model.tasks.insert(task.task_id, task);
    }

    let original_count = model.tasks.len();
    model.enforce_task_cap(current_user);

    // No tasks should be evicted
    assert_eq!(
        model.tasks.len(),
        original_count,
        "no tasks should be evicted when under cap"
    );
}

#[test]
fn test_environment_none_filter_includes_conversations() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();

        let mut model = create_test_model();

        // Task with no environment.
        let task_no_env = create_test_task(&make_uuid(1), "user-a", now);
        model.tasks.insert(task_no_env.task_id, task_no_env.clone());

        // Task with an environment (should be excluded when filtering for None).
        let mut task_with_env = create_test_task(&make_uuid(2), "user-b", now);
        task_with_env.agent_config_snapshot = Some(AgentConfigSnapshot {
            environment_id: Some("env_123".to_string()),
            ..Default::default()
        });
        model
            .tasks
            .insert(task_with_env.task_id, task_with_env.clone());

        // Local conversation (environment_id is always None) should be included.
        let conversation_id = AIConversationId::new();
        model.conversations.insert(
            conversation_id,
            ConversationMetadata {
                nav_data: ConversationNavigationData {
                    id: conversation_id,
                    title: "Test conversation".to_string(),
                    initial_query: None,
                    last_updated: chrono::Local::now(),
                    terminal_view_id: None,
                    window_id: None,
                    pane_view_locator: None,
                    initial_working_directory: None,
                    latest_working_directory: None,
                    is_selected: false,
                    is_in_active_pane: false,
                    is_closed: false,
                    server_conversation_token: None,
                },
            },
        );

        let filters = AgentManagementFilters {
            owners: OwnerFilter::All,
            environment: EnvironmentFilter::NoEnvironment,
            ..Default::default()
        };

        app.update(|ctx| {
            let mut saw_conversation = false;
            let mut saw_task_no_env = false;
            let mut saw_task_with_env = false;

            for item in model.get_tasks_and_conversations(&filters, ctx) {
                match item {
                    ConversationOrTask::Conversation(_) => saw_conversation = true,
                    ConversationOrTask::Task(task) if task.task_id == task_no_env.task_id => {
                        saw_task_no_env = true
                    }
                    ConversationOrTask::Task(task) if task.task_id == task_with_env.task_id => {
                        saw_task_with_env = true
                    }
                    ConversationOrTask::Task(_) => {}
                }
            }

            assert!(
                saw_conversation,
                "expected Environment=None filter to include conversations"
            );
            assert!(
                saw_task_no_env,
                "expected Environment=None filter to include tasks without an environment"
            );
            assert!(
                !saw_task_with_env,
                "expected Environment=None filter to exclude tasks with an environment"
            );
        });
    })
}

#[test]
fn test_file_artifact_filter_matches_only_items_with_file_artifacts() {
    let artifacts_with_file = vec![Artifact::File {
        artifact_uid: "artifact-file-1".to_string(),
        filepath: "outputs/report.txt".to_string(),
        filename: "report.txt".to_string(),
        mime_type: "text/plain".to_string(),
        description: Some("Daily summary".to_string()),
        size_bytes: Some(42),
    }];
    let artifacts_with_pr = vec![Artifact::PullRequest {
        url: "https://github.com/org/repo/pull/1".to_string(),
        branch: "main".to_string(),
        repo: Some("repo".to_string()),
        number: Some(1),
    }];

    assert!(super::artifacts_match_filter(
        &artifacts_with_file,
        &ArtifactFilter::File,
    ));
    assert!(!super::artifacts_match_filter(
        &artifacts_with_pr,
        &ArtifactFilter::File,
    ));
    assert!(super::artifacts_match_filter(
        &artifacts_with_file,
        &ArtifactFilter::All,
    ));
}

#[test]
fn test_task_status_maps_blocked_state_to_blocked() {
    App::test((), |mut app| async move {
        let now = Utc::now();
        let mut task = create_test_task(&make_uuid(999), "user-a", now);
        task.state = AmbientAgentTaskState::Blocked;
        task.status_message = Some(TaskStatusMessage {
            message: "Needs clarification".to_string(),
        });

        app.update(|ctx| {
            let status = ConversationOrTask::Task(&task).status(ctx);
            match status {
                ConversationStatus::Blocked { blocked_action } => {
                    assert_eq!(blocked_action, "Needs clarification");
                }
                other => panic!("expected blocked status, got {other:?}"),
            }
        });
    });
}

#[test]
fn test_get_tasks_and_conversations_prefers_task_when_task_id_matches_conversation_run_id() {
    App::test((), |mut app| async move {
        let _orchestration_v2_guard = FeatureFlag::OrchestrationV2.override_enabled(true);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let task_id = make_uuid(3000);

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: None,
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: Some(task_id.clone()),
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let mut model = create_test_model();
        let mut task = create_test_task(&task_id, "user-a", now);
        task.conversation_id = None;
        model.tasks.insert(task.task_id, task.clone());
        model.conversations.insert(
            conversation_id,
            create_test_conversation_metadata(conversation_id, "Conversation"),
        );

        app.update(|ctx| {
            let items: Vec<String> = model
                .get_tasks_and_conversations(&all_owner_filters(), ctx)
                .map(|item| match item {
                    ConversationOrTask::Task(task) => format!("task:{}", task.task_id),
                    ConversationOrTask::Conversation(conversation) => {
                        format!("conversation:{}", conversation.nav_data.id)
                    }
                })
                .collect();

            assert_eq!(items, vec![format!("task:{}", task.task_id)]);
        });
    });
}

#[test]
fn test_get_tasks_and_conversations_prefers_task_when_server_token_matches() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();
        let server_token = "server-token-123";

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: Some(server_token.to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let mut model = create_test_model();
        let mut task = create_test_task(&make_uuid(3001), "user-a", now);
        task.conversation_id = Some(server_token.to_string());
        model.tasks.insert(task.task_id, task.clone());
        model.conversations.insert(
            conversation_id,
            create_test_conversation_metadata(conversation_id, "Conversation"),
        );

        app.update(|ctx| {
            let items: Vec<String> = model
                .get_tasks_and_conversations(&all_owner_filters(), ctx)
                .map(|item| match item {
                    ConversationOrTask::Task(task) => format!("task:{}", task.task_id),
                    ConversationOrTask::Conversation(conversation) => {
                        format!("conversation:{}", conversation.nav_data.id)
                    }
                })
                .collect();

            assert_eq!(items, vec![format!("task:{}", task.task_id)]);
        });
    });
}

#[test]
fn test_get_tasks_and_conversations_keeps_unrelated_tasks_and_conversations() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let now = Utc::now();
        let conversation_id = AIConversationId::new();

        let conversation = create_restored_conversation(
            conversation_id,
            "root-task",
            AgentConversationData {
                server_conversation_token: Some("server-token-123".to_string()),
                conversation_usage_metadata: None,
                reverted_action_ids: None,
                forked_from_server_conversation_token: None,
                artifacts_json: None,
                parent_agent_id: None,
                agent_name: None,
                parent_conversation_id: None,
                run_id: None,
                autoexecute_override: None,
                last_event_sequence: None,
            },
        );

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let mut model = create_test_model();
        let mut task = create_test_task(&make_uuid(3002), "user-a", now);
        task.conversation_id = Some("different-token".to_string());
        model.tasks.insert(task.task_id, task.clone());
        model.conversations.insert(
            conversation_id,
            create_test_conversation_metadata(conversation_id, "Conversation"),
        );

        app.update(|ctx| {
            let items: Vec<String> = model
                .get_tasks_and_conversations(&all_owner_filters(), ctx)
                .map(|item| match item {
                    ConversationOrTask::Task(task) => format!("task:{}", task.task_id),
                    ConversationOrTask::Conversation(conversation) => {
                        format!("conversation:{}", conversation.nav_data.id)
                    }
                })
                .collect();

            assert_eq!(items.len(), 2);
            assert!(items.contains(&format!("task:{}", task.task_id)));
            assert!(items.contains(&format!("conversation:{conversation_id}")));
        });
    });
}

/// Helper: build a task with the given harness on its config snapshot.
///
/// `harness` semantics:
/// - `None`            → leaves `agent_config_snapshot = None` (stub task).
/// - `Some(None)`      → `agent_config_snapshot = Some { harness: None }`.
/// - `Some(Some(h))`   → `agent_config_snapshot = Some { harness: Some(h) }`.
fn task_with_harness(
    task_id_index: usize,
    creator_uid: &str,
    harness: Option<Option<Harness>>,
) -> AmbientAgentTask {
    let mut task = create_test_task(&make_uuid(task_id_index), creator_uid, Utc::now());
    match harness {
        None => task.agent_config_snapshot = None,
        Some(None) => {
            task.agent_config_snapshot = Some(AgentConfigSnapshot {
                harness: None,
                ..Default::default()
            });
        }
        Some(Some(h)) => {
            task.agent_config_snapshot = Some(AgentConfigSnapshot {
                harness: Some(HarnessConfig::from_harness_type(h)),
                ..Default::default()
            });
        }
    }
    task
}

#[test]
fn test_harness_filter_matches_only_selected_harness() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut model = create_test_model();

        let task_claude = task_with_harness(5100, "user-a", Some(Some(Harness::Claude)));
        let task_gemini = task_with_harness(5101, "user-a", Some(Some(Harness::Gemini)));
        // Snapshot present but no harness set → Some(Oz), matches Warp Agent.
        let task_oz_default = task_with_harness(5102, "user-a", Some(None));
        // No snapshot at all → None, matches only `All`.
        let task_no_snapshot = task_with_harness(5103, "user-a", None);

        for t in [
            &task_claude,
            &task_gemini,
            &task_oz_default,
            &task_no_snapshot,
        ] {
            model.tasks.insert(t.task_id, t.clone());
        }

        // Local conversation: effectively Warp Agent.
        let conv_id = AIConversationId::new();
        model.conversations.insert(
            conv_id,
            create_test_conversation_metadata(conv_id, "Local conv"),
        );

        app.update(|ctx| {
            let items_for = |filter: HarnessFilter| -> Vec<String> {
                model
                    .get_tasks_and_conversations(
                        &AgentManagementFilters {
                            owners: OwnerFilter::All,
                            harness: filter,
                            ..Default::default()
                        },
                        ctx,
                    )
                    .map(|item| match item {
                        ConversationOrTask::Task(t) => format!("task:{}", t.task_id),
                        ConversationOrTask::Conversation(c) => {
                            format!("conversation:{}", c.nav_data.id)
                        }
                    })
                    .collect()
            };

            // All → everything (incl. the unknown-harness stub task).
            assert_eq!(items_for(HarnessFilter::All).len(), 5);

            // Claude → only the claude task.
            let claude_items = items_for(HarnessFilter::Specific(Harness::Claude));
            assert_eq!(claude_items, vec![format!("task:{}", task_claude.task_id)]);

            // Gemini → only the gemini task.
            let gemini_items = items_for(HarnessFilter::Specific(Harness::Gemini));
            assert_eq!(gemini_items, vec![format!("task:{}", task_gemini.task_id)]);

            // Warp Agent / Oz → default-snapshot task and local conversation.
            // The stub task with no snapshot resolves to `harness() == None` and
            // is deliberately excluded from any specific-harness filter.
            let oz_items = items_for(HarnessFilter::Specific(Harness::Oz));
            assert_eq!(
                oz_items.len(),
                2,
                "expected 2 Warp Agent matches, got {oz_items:?}"
            );
            assert!(oz_items.contains(&format!("task:{}", task_oz_default.task_id)));
            assert!(oz_items.contains(&format!("conversation:{conv_id}")));
            assert!(
                !oz_items.contains(&format!("task:{}", task_no_snapshot.task_id)),
                "stub task with no snapshot should not match the Warp Agent filter"
            );
        });
    });
}

#[test]
fn test_harness_filter_is_filtering_and_reset() {
    // Default is All → not filtering, and after toggling reset_all_but_owner returns to default.
    let mut filters = AgentManagementFilters::default();
    assert!(!filters.is_filtering());

    filters.harness = HarnessFilter::Specific(Harness::Claude);
    assert!(
        filters.is_filtering(),
        "harness != All should report filtering"
    );

    filters.reset_all_but_owner();
    assert_eq!(filters.harness, HarnessFilter::default());
    assert!(!filters.is_filtering());
}

#[test]
fn test_get_or_async_fetch_task_data_returns_cached_task_without_fetching() {
    // If the task is already in `tasks`, return it directly and don't touch the fetch-state
    // map — even if a stale `PermanentlyFailedAt` entry exists (which shouldn't normally happen,
    // but proves the success path takes precedence).
    App::test((), |mut app| async move {
        let now = Utc::now();
        let task = create_test_task(&make_uuid(7000), "user-a", now);
        let task_id = task.task_id;

        let model_handle = app.add_singleton_model(|_| {
            let mut model = create_test_model();
            model.tasks.insert(task_id, task.clone());
            // Sentinel: even if a permanent-failure entry is present, the cached task wins.
            model
                .task_fetch_state
                .insert(task_id, TaskFetchState::PermanentlyFailedAt(Instant::now()));
            model
        });

        let result = model_handle.update(&mut app, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        assert!(result.is_some(), "cached task should be returned");
        model_handle.update(&mut app, |model, _| {
            // The cached-hit fast path doesn't touch `task_fetch_state`, so the sentinel
            // entry is left as-is and (importantly) no `InFlight` entry was added.
            assert!(matches!(
                model.task_fetch_state.get(&task_id),
                Some(TaskFetchState::PermanentlyFailedAt(_))
            ));
        });
    });
}

#[test]
fn test_get_or_async_fetch_task_data_skips_when_permanently_failed() {
    // A task id marked as `PermanentlyFailedAt` within its cooldown (e.g. very recent 403) must
    // not spawn a new fetch.
    App::test((), |mut app| async move {
        let task_id: AmbientAgentTaskId = make_uuid(7001).parse().unwrap();

        let model_handle = app.add_singleton_model(|_| {
            let mut model = create_test_model();
            model
                .task_fetch_state
                .insert(task_id, TaskFetchState::PermanentlyFailedAt(Instant::now()));
            model
        });

        let result = model_handle.update(&mut app, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        assert!(result.is_none());
        model_handle.update(&mut app, |model, _| {
            // The state is unchanged — still permanently failed, no in-flight upgrade.
            assert!(matches!(
                model.task_fetch_state.get(&task_id),
                Some(TaskFetchState::PermanentlyFailedAt(_))
            ));
        });
    });
}

#[test]
fn test_get_or_async_fetch_task_data_skips_when_in_flight() {
    // A task id already marked as `InFlight` must not spawn a duplicate fetch.
    App::test((), |mut app| async move {
        let task_id: AmbientAgentTaskId = make_uuid(7002).parse().unwrap();

        let model_handle = app.add_singleton_model(|_| {
            let mut model = create_test_model();
            model
                .task_fetch_state
                .insert(task_id, TaskFetchState::InFlight);
            model
        });

        let result = model_handle.update(&mut app, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        assert!(result.is_none());
        model_handle.update(&mut app, |model, _| {
            // Still exactly the one in-flight entry we pre-seeded.
            assert_eq!(model.task_fetch_state.len(), 1);
            assert!(matches!(
                model.task_fetch_state.get(&task_id),
                Some(TaskFetchState::InFlight)
            ));
        });
    });
}

#[test]
fn test_get_or_async_fetch_task_data_skips_within_transient_cooldown() {
    // A recent transient failure (timestamp younger than the cooldown) must short-circuit.
    App::test((), |mut app| async move {
        let task_id: AmbientAgentTaskId = make_uuid(7003).parse().unwrap();

        let model_handle = app.add_singleton_model(|_| {
            let mut model = create_test_model();
            model
                .task_fetch_state
                .insert(task_id, TaskFetchState::TransientlyFailedAt(Instant::now()));
            model
        });

        let result = model_handle.update(&mut app, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        assert!(result.is_none());
        model_handle.update(&mut app, |model, _| {
            // The transient entry is preserved (no upgrade to in-flight).
            assert!(matches!(
                model.task_fetch_state.get(&task_id),
                Some(TaskFetchState::TransientlyFailedAt(_))
            ));
        });
    });
}

#[test]
fn test_agent_management_filters_serde_backwards_compat() {
    // Persisted state from older clients has no `harness` key → deserializes to All.
    let legacy = r#"{
        "owners": "PersonalOnly",
        "status": "All",
        "source": "All",
        "created_on": "All",
        "creator": "All",
        "artifact": "All"
    }"#;
    let decoded: AgentManagementFilters =
        serde_json::from_str(legacy).expect("legacy payload without harness must deserialize");
    assert_eq!(decoded.harness, HarnessFilter::All);

    // Round trip a Specific(Claude) value.
    let original = AgentManagementFilters {
        harness: HarnessFilter::Specific(Harness::Claude),
        ..Default::default()
    };
    let encoded = serde_json::to_string(&original).unwrap();
    assert!(
        encoded.contains("\"harness\":\"claude\""),
        "expected serialized form to contain \"harness\":\"claude\", got {encoded}"
    );
    let decoded: AgentManagementFilters = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, original);

    // Unknown harness strings deserialize to All (forward compat).
    let forward = r#"{
        "owners": "PersonalOnly",
        "status": "All",
        "source": "All",
        "created_on": "All",
        "creator": "All",
        "artifact": "All",
        "harness": "some-future-harness"
    }"#;
    let decoded: AgentManagementFilters = serde_json::from_str(forward).unwrap();
    assert_eq!(decoded.harness, HarnessFilter::All);
}
