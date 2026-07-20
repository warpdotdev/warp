use std::fs;
use std::io::Write;
use std::path::PathBuf;

use ai::skills::{ParsedSkill, SkillProvider, SkillReference, SkillScope, parse_skill};
use async_channel::unbounded;
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use tempfile::TempDir;
use warp_core::HostId;
use warp_core::features::FeatureFlag;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, ModelHandle};
use watcher::HomeDirectoryWatcher;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionId, AIAgentActionResultType, AIAgentActionType, ReadSkillRequest,
    ReadSkillResult,
};
use crate::ai::blocklist::action_model::AIConversationId;
use crate::ai::skills::{BundledSkillActivation, SkillManager};
use crate::settings::AISettings;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{BootstrapSessionType, SessionId, SessionInfo, Sessions};
use crate::terminal::model_events::ModelEventDispatcher;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;

fn initialize_app(app: &mut App) {
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(AISettings::new_with_defaults);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(SkillManager::new);
}
fn add_test_read_skill_executor(app: &mut App) -> ModelHandle<ReadSkillExecutor> {
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_model_events_tx, model_events_rx) = unbounded();
    let model_event_dispatcher =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let active_session = app
        .add_model(|ctx| ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx));
    app.add_model(|_| ReadSkillExecutor::new(active_session))
}

fn bundled_skill(name: &str) -> ParsedSkill {
    ParsedSkill {
        name: name.to_string(),
        description: format!("{name} bundled skill"),
        path: LocalOrRemotePath::Local(PathBuf::from(format!("/bundled/skills/{name}/SKILL.md"))),
        content: format!("# {name}"),
        line_range: None,
        provider: SkillProvider::Warp,
        scope: SkillScope::Bundled,
    }
}

fn create_test_skill_file(dir: &TempDir, name: &str, description: &str) -> std::path::PathBuf {
    let skill_content = format!(
        r#"---
name: {}
description: {}
---

# {}

## Instructions
Test instructions for this skill.

## Examples
Example usage of the skill.
"#,
        name, description, name
    );

    let skill_dir = dir.path().join(format!(".claude/skills/{}", name));
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    let mut file = fs::File::create(&skill_path).unwrap();
    file.write_all(skill_content.as_bytes()).unwrap();
    file.flush().unwrap();

    skill_path
}

#[test]
fn test_read_skill_executor_success() {
    let temp_dir = TempDir::new().unwrap();
    let skill_path = create_test_skill_file(&temp_dir, "test-skill", "A test skill");

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Populate SkillManager cache with the test skill
        let parsed_skill = parse_skill(&skill_path).expect("Failed to parse test skill");
        SkillManager::handle(&app).update(&mut app, |manager, _ctx| {
            manager.add_skill_for_testing(parsed_skill);
        });

        let executor_handle = add_test_read_skill_executor(&mut app);

        let action = AIAgentAction {
            id: AIAgentActionId::from("test-action-id".to_string()),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::Path(LocalOrRemotePath::Local(skill_path.clone())),
            }),
            task_id: TaskId::new("test-task-id".to_string()),
            requires_result: false,
        };

        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();

            match result {
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Success { content },
                )) => {
                    assert_eq!(content.file_name, skill_path.to_string_lossy().to_string());
                }
                _ => panic!("Successfully read skill file; should return ReadSkillResult::Success"),
            }
        });
    });
}

#[test]
fn disconnected_remote_session_does_not_fall_back_to_client_global_bundled_skill() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _bundled_skills = FeatureFlag::BundledSkills.override_enabled(true);
        SkillManager::handle(&app).update(&mut app, |manager, _ctx| {
            manager.add_bundled_skill_for_testing(
                "remote-only",
                bundled_skill("remote-only"),
                BundledSkillActivation::Always,
            );
        });

        let session_id = SessionId::from(42);
        let sessions = app.add_model(|_| Sessions::new_for_test());
        sessions.update(&mut app, |sessions, _ctx| {
            sessions.register_session_for_test(
                SessionInfo::new_for_test()
                    .with_id(session_id)
                    .with_session_type(BootstrapSessionType::WarpifiedRemote),
            );
        });
        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        model_event_dispatcher.update(&mut app, |dispatcher, _ctx| {
            dispatcher.set_active_session_id(session_id);
        });
        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let executor_handle = app.add_model(|_| ReadSkillExecutor::new(active_session));

        let action = AIAgentAction {
            id: AIAgentActionId::from("test-action-id".to_string()),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::BundledSkillId("remote-only".to_string()),
            }),
            task_id: TaskId::new("test-task-id".to_string()),
            requires_result: false,
        };
        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            assert!(matches!(
                result,
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Error(error)
                )) if error == "Bundled skills are not available on this remote session"
            ));
        });
    });
}

#[test]
fn remote_session_reads_remote_bundled_skill_catalog() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _bundled_skills = FeatureFlag::BundledSkills.override_enabled(true);
        let host_id = HostId::new("remote-host".to_string());
        let remote_skill = ParsedSkill {
            name: "host-specific".to_string(),
            description: "remote bundled skill".to_string(),
            path: LocalOrRemotePath::Remote(RemotePath::new(
                host_id.clone(),
                StandardizedPath::try_new(
                    "/opt/warp/resources/bundled/skills/host-specific/SKILL.md",
                )
                .unwrap(),
            )),
            content: "remote rendered content".to_string(),
            line_range: None,
            provider: SkillProvider::Warp,
            scope: SkillScope::Bundled,
        };
        SkillManager::handle(&app).update(&mut app, |manager, _ctx| {
            manager.add_bundled_skill_for_testing(
                "host-specific",
                bundled_skill("host-specific"),
                BundledSkillActivation::Always,
            );
            manager.add_remote_bundled_skill_for_testing(
                host_id.clone(),
                "host-specific",
                remote_skill,
                BundledSkillActivation::Always,
            );
        });

        let session_id = SessionId::from(42);
        let sessions = app.add_model(|_| Sessions::new_for_test());
        sessions.update(&mut app, |sessions, _ctx| {
            sessions.register_session_for_test(
                SessionInfo::new_for_test()
                    .with_id(session_id)
                    .with_session_type(BootstrapSessionType::WarpifiedRemote),
            );
        });
        let session = sessions
            .read(&app, |sessions, _ctx| sessions.get(session_id))
            .unwrap();
        session.set_remote_host_id(Some(host_id));

        let (_model_events_tx, model_events_rx) = unbounded();
        let model_event_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
        model_event_dispatcher.update(&mut app, |dispatcher, _ctx| {
            dispatcher.set_active_session_id(session_id);
        });
        let active_session = app.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_event_dispatcher.clone(), ctx)
        });
        let executor_handle = app.add_model(|_| ReadSkillExecutor::new(active_session));

        let action = AIAgentAction {
            id: AIAgentActionId::from("test-action-id".to_string()),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::BundledSkillId("host-specific".to_string()),
            }),
            task_id: TaskId::new("test-task-id".to_string()),
            requires_result: false,
        };
        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            match result {
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Success { content },
                )) => {
                    assert_eq!(
                        content.file_name,
                        "/opt/warp/resources/bundled/skills/host-specific/SKILL.md"
                    );
                    assert_eq!(
                        content.content,
                        AnyFileContent::StringContent("remote rendered content".to_string())
                    );
                }
                _ => panic!("Remote session should read its host-specific bundled skill"),
            }
        });
    });
}

#[test]
fn test_read_skill_executor_reads_enabled_bundled_skill() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _bundled_skills = FeatureFlag::BundledSkills.override_enabled(true);
        SkillManager::handle(&app).update(&mut app, |manager, _ctx| {
            manager.add_bundled_skill_for_testing(
                "pr-comments",
                bundled_skill("pr-comments"),
                BundledSkillActivation::Always,
            );
        });
        let executor_handle = add_test_read_skill_executor(&mut app);

        let action = AIAgentAction {
            id: AIAgentActionId::from("test-action-id".to_string()),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::BundledSkillId("pr-comments".to_string()),
            }),
            task_id: TaskId::new("test-task-id".to_string()),
            requires_result: false,
        };

        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();

            match result {
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Success { content },
                )) => {
                    assert_eq!(content.file_name, "/bundled/skills/pr-comments/SKILL.md");
                }
                _ => panic!("Enabled bundled skill should return ReadSkillResult::Success"),
            }
        });
    });
}

#[test]
fn test_read_skill_executor_rejects_warp_control_bundled_skills_when_disabled() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let _bundled_skills = FeatureFlag::BundledSkills.override_enabled(true);
        let _warp_control_cli = FeatureFlag::WarpControlCli.override_enabled(false);
        let skill_id = "warpctrl";
        SkillManager::handle(&app).update(&mut app, |manager, _ctx| {
            manager.add_bundled_skill_for_testing(
                skill_id,
                bundled_skill(skill_id),
                BundledSkillActivation::RequiresFeature(FeatureFlag::WarpControlCli),
            );
        });
        let executor_handle = add_test_read_skill_executor(&mut app);
        let action = AIAgentAction {
            id: AIAgentActionId::from(format!("test-action-id-{skill_id}")),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::BundledSkillId(skill_id.to_string()),
            }),
            task_id: TaskId::new(format!("test-task-id-{skill_id}")),
            requires_result: false,
        };

        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            assert!(matches!(
                result,
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Error(_)
                ))
            ));
        });
    });
}
#[test]
fn test_read_skill_executor_file_not_found() {
    let temp_dir = TempDir::new().unwrap();
    // Don't create the SKILL.md file
    let skill_path = temp_dir.path().join("SKILL.md");

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let executor_handle = add_test_read_skill_executor(&mut app);

        let action = AIAgentAction {
            id: AIAgentActionId::from("test-action-id".to_string()),
            action: AIAgentActionType::ReadSkill(ReadSkillRequest {
                skill: SkillReference::Path(LocalOrRemotePath::Local(skill_path)),
            }),
            task_id: TaskId::new("test-task-id".to_string()),
            requires_result: false,
        };

        let input = ExecuteActionInput {
            action: &action,
            conversation_id: AIConversationId::new(),
        };

        executor_handle.update(&mut app, |executor, ctx| {
            let result: AnyActionExecution = executor.execute(input, ctx).into();

            match result {
                AnyActionExecution::Sync(AIAgentActionResultType::ReadSkill(
                    ReadSkillResult::Error(error_msg),
                )) => {
                    // Should contain an error about file not found or I/O error
                    assert!(!error_msg.is_empty());
                }
                _ => panic!(
                    "Nonexistent SKILL.md file at given path; should return ReadSkillResult::Error"
                ),
            }
        });
    });
}
