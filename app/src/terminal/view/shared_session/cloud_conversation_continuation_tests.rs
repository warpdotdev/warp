use std::sync::Arc;

use chrono::Utc;
use persistence::model::ConversationUsageMetadata;
use warp_cli::agent::Harness;
use warp_graphql::object_permissions::AccessLevel;
use warpui::{App, EntityId, SingletonEntity};

use super::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::task::{
    AgentConfigSnapshot, HarnessConfig, TaskPrincipalInfo, TaskStatusErrorCode, TaskStatusMessage,
};
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::auth::user::TEST_USER_UID;
use crate::auth::{AuthStateProvider, UserUid};
use crate::cloud_object::{
    Owner, Revision, ServerGuestSubject, ServerMetadata, ServerObjectGuest, ServerPermissions,
};
use crate::server::ids::ServerId;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::workspaces::team::Team;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;
use crate::FeatureFlag;

const CONVERSATION_TOKEN: &str = "server-conversation-token";

#[derive(Clone, Copy)]
enum AuthFixture {
    LoggedIn,
    LoggedOut,
}

#[derive(Clone, Copy)]
enum ConversationPermissionFixture {
    CurrentUserOwner,
    OtherUserOwner,
    CurrentTeamOwner,
    CurrentTeamEditorGuest,
}

struct TestHandles {
    terminal_view_id: EntityId,
    task_id: AmbientAgentTaskId,
}

fn setup_app(
    app: &mut App,
    auth_fixture: AuthFixture,
    harness: AIAgentHarness,
    permissions_fixture: ConversationPermissionFixture,
) -> TestHandles {
    setup_app_with_creator(app, auth_fixture, harness, permissions_fixture, None)
}

fn setup_app_with_creator(
    app: &mut App,
    auth_fixture: AuthFixture,
    harness: AIAgentHarness,
    permissions_fixture: ConversationPermissionFixture,
    creator_uid: Option<String>,
) -> TestHandles {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    match auth_fixture {
        AuthFixture::LoggedIn => {
            app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        }
        AuthFixture::LoggedOut => {
            app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        }
    }
    let workspaces = workspaces_for_permission_fixture(permissions_fixture);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(AgentConversationsModel::new);

    let terminal_view_id = EntityId::new();
    let task_id = ambient_task_id(1);
    let task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::Succeeded,
    );

    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
    BlocklistAIHistoryModel::handle(app).update(app, |model, ctx| {
        let conversation_id =
            model.start_new_conversation(terminal_view_id, false, false, false, ctx);
        model.set_server_conversation_token_for_conversation(
            conversation_id,
            CONVERSATION_TOKEN.to_string(),
        );
        model.set_server_metadata_for_conversation(
            conversation_id,
            server_conversation_metadata(harness, permissions_fixture, Some(task_id), creator_uid),
            ctx,
        );
    });

    TestHandles {
        terminal_view_id,
        task_id,
    }
}

fn setup_task_without_server_metadata(app: &mut App) -> TestHandles {
    setup_task_without_server_metadata_for_creator(app, "other-user")
}

fn setup_owned_task_without_server_metadata(app: &mut App) -> TestHandles {
    setup_task_without_server_metadata_for_creator(app, TEST_USER_UID)
}

fn setup_task_without_server_metadata_for_creator(app: &mut App, creator_uid: &str) -> TestHandles {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(AgentConversationsModel::new);

    let terminal_view_id = EntityId::new();
    let task_id = ambient_task_id(1);
    let task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::Succeeded,
    )
    .with_creator(creator_uid);
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });

    TestHandles {
        terminal_view_id,
        task_id,
    }
}

fn ambient_task_id(index: usize) -> AmbientAgentTaskId {
    format!("550e8400-e29b-41d4-a716-{index:012}")
        .parse()
        .unwrap()
}

fn ambient_agent_task(
    task_id: AmbientAgentTaskId,
    conversation_token: &str,
    state: AmbientAgentTaskState,
) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Task".to_string(),
        state,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: TEST_USER_UID.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: Some(conversation_token.to_string()),
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn active_ambient_agent_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    let mut task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::InProgress,
    );
    task.session_link = Some("https://example.com/session/active".to_string());
    task.is_sandbox_running = true;
    task
}

trait AmbientAgentTaskTestExt {
    fn with_creator(self, creator_uid: &str) -> Self;
    fn with_harness(self, harness: Harness) -> Self;
}

impl AmbientAgentTaskTestExt for AmbientAgentTask {
    fn with_creator(mut self, creator_uid: &str) -> Self {
        self.creator = Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        });
        self
    }

    fn with_harness(mut self, harness: Harness) -> Self {
        self.agent_config_snapshot = Some(AgentConfigSnapshot {
            harness: (harness != Harness::Oz).then_some(HarnessConfig {
                harness_type: harness,
                model_id: None,
                reasoning_level: None,
            }),
            ..Default::default()
        });
        self
    }
}

fn current_team_uid() -> ServerId {
    ServerId::from(123)
}

fn workspaces_for_permission_fixture(
    permissions_fixture: ConversationPermissionFixture,
) -> Vec<Workspace> {
    match permissions_fixture {
        ConversationPermissionFixture::CurrentTeamOwner
        | ConversationPermissionFixture::CurrentTeamEditorGuest => {
            vec![Workspace::from_local_cache(
                ServerId::from(456).into(),
                "Test Workspace".to_string(),
                Some(vec![Team::from_local_cache(
                    current_team_uid(),
                    "Test Team".to_string(),
                    None,
                    None,
                    None,
                )]),
            )]
        }
        ConversationPermissionFixture::CurrentUserOwner
        | ConversationPermissionFixture::OtherUserOwner => vec![],
    }
}

fn server_conversation_metadata(
    harness: AIAgentHarness,
    permissions_fixture: ConversationPermissionFixture,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
    creator_uid: Option<String>,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Conversation".to_string(),
        working_directory: None,
        harness,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
        },
        metadata: server_metadata(creator_uid),
        permissions: server_permissions(permissions_fixture),
        ambient_agent_task_id,
        server_conversation_token: ServerConversationToken::new(CONVERSATION_TOKEN.to_string()),
        artifacts: vec![],
    }
}
fn server_metadata(creator_uid: Option<String>) -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn server_permissions(permissions_fixture: ConversationPermissionFixture) -> ServerPermissions {
    let space = match permissions_fixture {
        ConversationPermissionFixture::CurrentUserOwner => Owner::mock_current_user(),
        ConversationPermissionFixture::OtherUserOwner
        | ConversationPermissionFixture::CurrentTeamEditorGuest => Owner::User {
            user_uid: UserUid::new("other-user"),
        },
        ConversationPermissionFixture::CurrentTeamOwner => Owner::Team {
            team_uid: current_team_uid(),
        },
    };
    let guests = match permissions_fixture {
        ConversationPermissionFixture::CurrentTeamEditorGuest => vec![ServerObjectGuest {
            subject: ServerGuestSubject::Team {
                team_uid: current_team_uid(),
            },
            access_level: AccessLevel::Editor,
            source: None,
        }],
        ConversationPermissionFixture::CurrentUserOwner
        | ConversationPermissionFixture::OtherUserOwner
        | ConversationPermissionFixture::CurrentTeamOwner => vec![],
    };

    ServerPermissions {
        space,
        guests,
        anyone_link_sharing: None,
        permissions_last_updated_ts: Utc::now().into(),
    }
}

#[test]
fn missing_task_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id, ..
        } = setup_task_without_server_metadata(&mut app);
        let missing_task_id = ambient_task_id(2);

        app.update(|ctx| {
            let state = resolve_cloud_conversation_continuation_ui_state(
                terminal_view_id,
                missing_task_id,
                ctx,
            );
            assert_eq!(state, Err(CloudConversationContinuationError::MissingTask));
        });
    });
}

#[test]
fn github_action_source_shows_tombstone_without_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task = ambient_agent_task(
                task_id,
                CONVERSATION_TOKEN,
                AmbientAgentTaskState::Succeeded,
            )
            .with_creator(TEST_USER_UID);
            task.source = Some(AgentSource::GitHubAction);
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            assert_eq!(
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx),
                Ok(CloudConversationContinuationUiState::Tombstone { cta: None })
            );
        });
    });
}

#[test]
fn oz_conversation_with_edit_access_shows_inline_followup_input() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
            ConversationPermissionFixture::CurrentUserOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);
            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::FollowupInput)
            );
        });
    });
}

#[test]
fn oz_conversation_with_view_access_shows_continue_locally_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
            ConversationPermissionFixture::OtherUserOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx)
                    .unwrap();

            assert!(matches!(
                state,
                CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueLocally { .. })
                }
            ));
        });
    });
}

#[test]
fn third_party_conversation_with_edit_access_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentUserOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn environment_setup_failure_without_conversation_shows_tombstone_without_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentUserOwner,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task =
                ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
            task.conversation_id = None;
            task.status_message = Some(TaskStatusMessage {
                message: "Environment setup failed: Failed to run setup command: hi".to_string(),
                error_code: Some(TaskStatusErrorCode::EnvironmentSetupFailed),
            });
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone { cta: None })
            );
        });
    });
}

#[test]
fn environment_setup_failure_with_conversation_shows_continue_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentUserOwner,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task =
                ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
            task.status_message = Some(TaskStatusMessage {
                message: "Environment setup failed: Failed to run setup command: hi".to_string(),
                error_code: Some(TaskStatusErrorCode::EnvironmentSetupFailed),
            });
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn third_party_conversation_created_by_current_user_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app_with_creator(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::OtherUserOwner,
            Some(TEST_USER_UID.to_string()),
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn third_party_conversation_owned_by_current_team_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentTeamOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn third_party_conversation_shared_with_current_team_as_editor_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentTeamEditorGuest,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn third_party_conversation_with_view_access_shows_tombstone_without_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::OtherUserOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);
            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone { cta: None })
            );
        });
    });
}

#[test]
fn unknown_access_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedOut,
            AIAgentHarness::ClaudeCode,
            ConversationPermissionFixture::CurrentUserOwner,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::UnknownConversationAccess)
            );
        });
    });
}

#[test]
fn missing_metadata_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_task_without_server_metadata(&mut app);

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::MissingServerConversationMetadata)
            );
        });
    });
}

#[test]
fn owned_oz_task_without_metadata_shows_inline_followup_input() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::FollowupInput)
            );
        });
    });
}

#[test]
fn owned_third_party_task_without_metadata_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(
                ambient_agent_task(
                    task_id,
                    CONVERSATION_TOKEN,
                    AmbientAgentTaskState::Succeeded,
                )
                .with_creator(TEST_USER_UID)
                .with_harness(Harness::Claude),
            );
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn active_task_execution_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
            ConversationPermissionFixture::CurrentUserOwner,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(active_ambient_agent_task(task_id));
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::ActiveTaskExecution)
            );
        });
    });
}
