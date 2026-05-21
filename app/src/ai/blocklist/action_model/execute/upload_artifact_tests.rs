use std::fs;
use std::path::{Path, PathBuf};

use async_channel::unbounded;
use warpui::{App, EntityId, ModelHandle};

use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType,
    UploadArtifactRequest, UploadArtifactResult,
};
use crate::ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use crate::ai::execution_profiles::{profiles::AIExecutionProfilesModel, ActionPermission};
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::{cloud_objects::update_manager::UpdateManager, sync_queue::SyncQueue};
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{SessionId, SessionInfo, Sessions};
use crate::terminal::model::terminal_model::BlockIndex;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::ShellType;
use crate::terminal::ShellLaunchData;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::{team_tester::TeamTesterStatus, user_workspaces::UserWorkspaces};
use crate::LaunchMode;

use super::*;

fn build_upload_artifact_action(file_path: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("upload-artifact-action".to_string()),
        action: AIAgentActionType::UploadArtifact(UploadArtifactRequest {
            file_path: file_path.to_string(),
            description: Some("Upload the generated report".to_string()),
        }),
        task_id: TaskId::new("upload-artifact-task".to_string()),
        requires_result: false,
    }
}

fn initialize_upload_artifact_test(
    app: &mut App,
    terminal_view_id: EntityId,
    current_working_directory: &Path,
) -> (
    ModelHandle<BlocklistAIHistoryModel>,
    ModelHandle<ActiveSession>,
) {
    initialize_settings_for_tests(app);

    let history = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(UserWorkspaces::default_mock);
    let profiles = app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
    });
    app.add_singleton_model(BlocklistAIPermissions::new);

    profiles.update(app, |profiles, ctx| {
        if let Some(profile_id) = profiles.create_profile(ctx) {
            profiles.set_read_files(profile_id, &ActionPermission::AlwaysAsk, ctx);
            profiles.set_active_profile(terminal_view_id, profile_id, ctx);
        }
    });

    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_, model_events_rx) = unbounded();
    let model_event_dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session = app
        .add_model(|ctx| ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx));

    let session_id = SessionId::from(7);
    sessions.update(app, |sessions, _ctx| {
        let mut session_info = SessionInfo::new_for_test().with_id(session_id);
        session_info.launch_data = Some(test_shell_launch_data());
        sessions.register_session_for_test(session_info);
    });
    model_event_dispatcher.update(app, |model_event_dispatcher, ctx| {
        model_event_dispatcher.set_active_session_id(session_id);
        ctx.emit(ModelEvent::BlockMetadataReceived(
            BlockMetadataReceivedEvent {
                block_metadata: BlockMetadata::new(
                    Some(session_id),
                    Some(current_working_directory.display().to_string()),
                ),
                block_index: BlockIndex::zero(),
                is_after_in_band_command: false,
                is_done_bootstrapping: true,
            },
        ));
    });

    (history, active_session)
}

fn test_shell_launch_data() -> ShellLaunchData {
    #[cfg(unix)]
    {
        ShellLaunchData::Executable {
            executable_path: PathBuf::from("/bin/bash"),
            shell_type: ShellType::Bash,
        }
    }

    #[cfg(windows)]
    {
        ShellLaunchData::Executable {
            executable_path: PathBuf::from(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            ),
            shell_type: ShellType::PowerShell,
        }
    }
}

#[test]
fn should_autoexecute_honors_file_read_permissions_for_resolved_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let artifact_path = cwd.join("reports/report.txt");
    fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    fs::write(&artifact_path, "artifact contents").unwrap();

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, active_session) =
            initialize_upload_artifact_test(&mut app, terminal_view_id, &cwd);
        let executor =
            app.add_model(|_| UploadArtifactExecutor::new(active_session, terminal_view_id));
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_upload_artifact_action("reports/report.txt");

        let should_autoexecute_before = executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            )
        });
        assert!(!should_autoexecute_before);

        app.update(|ctx| {
            BlocklistAIPermissions::handle(ctx).update(ctx, |permissions, _ctx| {
                permissions
                    .add_temporary_file_read_permissions(conversation_id, [artifact_path.clone()]);
            });
        });

        let should_autoexecute_after = executor.update(&mut app, |executor, ctx| {
            executor.should_autoexecute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            )
        });
        assert!(should_autoexecute_after);
    });
}

#[test]
fn execute_returns_error_when_conversation_has_not_synced_to_server() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();
    let artifact_path = cwd.join("report.txt");
    fs::write(&artifact_path, "artifact contents").unwrap();

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (history, active_session) =
            initialize_upload_artifact_test(&mut app, terminal_view_id, &cwd);
        let executor =
            app.add_model(|_| UploadArtifactExecutor::new(active_session, terminal_view_id));
        let conversation_id = history.update(&mut app, |history, ctx| {
            history.start_new_conversation(terminal_view_id, false, false, false, ctx)
        });
        let action = build_upload_artifact_action(&artifact_path.display().to_string());

        let execution = executor.update(&mut app, |executor, ctx| {
            executor.execute(
                ExecuteActionInput {
                    action: &action,
                    conversation_id,
                },
                ctx,
            )
        });

        assert!(matches!(
            execution,
            AnyActionExecution::Sync(AIAgentActionResultType::UploadArtifact(
                UploadArtifactResult::Error(message),
            )) if message == "Current conversation has not been synced to the server yet"
        ));
    });
}

#[test]
fn resolve_path_uses_active_session_working_directory_for_relative_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cwd = temp_dir.path().join("workspace");
    fs::create_dir_all(&cwd).unwrap();

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let (_, active_session) = initialize_upload_artifact_test(&mut app, terminal_view_id, &cwd);
        let executor =
            app.add_model(|_| UploadArtifactExecutor::new(active_session, terminal_view_id));

        let resolved_path = executor.update(&mut app, |executor, ctx| {
            executor.resolve_path("reports/out.txt", ctx)
        });

        assert_eq!(resolved_path, cwd.join("reports/out.txt"));
    });
}
