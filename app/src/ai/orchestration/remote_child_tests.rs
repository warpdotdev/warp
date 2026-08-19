use std::path::PathBuf;

use ai::skills::parse_skill;
use anyhow::anyhow;
use tempfile::TempDir;
use warpui::{App, SingletonEntity as _};

use super::{
    CloudAgentStartupAuthFlow, CloudAgentStartupBlocker, CloudAgentStartupFailure,
    CloudAgentStartupIssue, CloudAgentStartupPresentation, RemoteChildLaunchConfig,
    classify_cloud_agent_startup_error, prepare_remote_child_launch,
};
use crate::ai::agent::{StartAgentExecutionMode, UserQueryMode};
use crate::ai::blocklist::StartAgentRequest;
use crate::ai::skills::{BundledSkillActivation, SkillManager, SkillReference};
use crate::server::server_api::{AIApiError, ClientError, CloudAgentCapacityError};

fn config(harness_type: &str) -> RemoteChildLaunchConfig {
    RemoteChildLaunchConfig {
        environment_id: String::new(),
        skill_references: Vec::new(),
        working_dir: PathBuf::new(),
        model_id: String::new(),
        computer_use_enabled: false,
        worker_host: String::new(),
        harness_type: harness_type.to_string(),
        title: String::new(),
        auth_secret_name: None,
        runner_id: String::new(),
        agent_identity_uid: None,
    }
}

#[test]
fn orchestration_harness_defaults_to_oz_and_parses_known_harnesses() {
    assert_eq!(
        config("").orchestration_harness(),
        warp_cli::agent::Harness::Oz
    );
    assert_eq!(
        config("claude").orchestration_harness(),
        warp_cli::agent::Harness::Claude
    );
}

#[test]
fn prepared_remote_request_matches_gui_wire_semantics() {
    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        let request = StartAgentRequest {
            id: Default::default(),
            name: "  researcher  ".to_string(),
            prompt: "Inspect the code".to_string(),
            execution_mode: StartAgentExecutionMode::Remote {
                environment_id: "env-1".to_string(),
                skill_references: Vec::new(),
                model_id: "auto".to_string(),
                computer_use_enabled: true,
                worker_host: "warp".to_string(),
                harness_type: "oz".to_string(),
                title: "Research".to_string(),
                auth_secret_name: None,
                runner_id: String::new(),
                agent_identity_uid: None,
            },
            lifecycle_subscription: None,
            parent_conversation_id: crate::ai::agent::conversation::AIConversationId::new(),
            parent_run_id: Some("parent-run".to_string()),
        };
        app.read(|ctx| {
            let prepared = prepare_remote_child_launch(
                &request,
                RemoteChildLaunchConfig {
                    environment_id: "env-1".to_string(),
                    skill_references: Vec::new(),
                    working_dir: PathBuf::new(),
                    model_id: "auto".to_string(),
                    computer_use_enabled: true,
                    worker_host: "warp".to_string(),
                    harness_type: "oz".to_string(),
                    title: "Research".to_string(),
                    auth_secret_name: None,
                    runner_id: "runner-1".to_string(),
                    agent_identity_uid: Some("researcher-agent".to_string()),
                },
                ctx,
            )
            .unwrap();
            assert_eq!(prepared.display_name, "researcher");
            assert_eq!(
                prepared.spawn_request.prompt.as_deref(),
                Some("Inspect the code")
            );
            assert_eq!(prepared.spawn_request.mode, UserQueryMode::Normal);
            assert_eq!(
                prepared.spawn_request.parent_run_id.as_deref(),
                Some("parent-run")
            );
            assert_eq!(
                prepared.spawn_request.agent_identity_uid.as_deref(),
                Some("researcher-agent")
            );
            let config = prepared.spawn_request.config.unwrap();
            assert_eq!(config.environment_id.as_deref(), Some("env-1"));
            assert_eq!(config.runner_id.as_deref(), Some("runner-1"));
            assert_eq!(config.model_id.as_deref(), Some("auto"));
            assert_eq!(config.worker_host.as_deref(), Some("warp"));
            assert_eq!(config.computer_use_enabled, Some(true));
        });
    });
}

#[test]
fn repo_qualified_skill_spec_resolves_into_runtime_skills() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path().join("myrepo");
    let skill_path = repo_root.join(".agents/skills/my-skill/SKILL.md");
    std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    std::fs::write(
        &skill_path,
        "---\nname: my-skill\ndescription: Test skill\n---\nUse this skill.\n",
    )
    .unwrap();
    let parsed_skill = parse_skill(&skill_path).unwrap();
    let skill_spec = format!(
        "someorg/myrepo:{}",
        skill_path.strip_prefix(&repo_root).unwrap().display()
    );

    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        SkillManager::handle(&app).update(&mut app, |manager, _| {
            manager.handle_skills_added(vec![parsed_skill.clone()]);
            manager.add_bundled_skill_for_testing(
                "bundled-test",
                parsed_skill,
                BundledSkillActivation::Always,
            );
        });
        let skill_references = vec![
            SkillReference::Path(warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                PathBuf::from(&skill_spec),
            )),
            SkillReference::Path(warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                PathBuf::from("myrepo:my-skill"),
            )),
            SkillReference::Path(warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                skill_path.clone(),
            )),
            SkillReference::BundledSkillId("bundled-test".to_string()),
        ];
        let request = StartAgentRequest {
            id: Default::default(),
            name: "child".to_string(),
            prompt: "Run".to_string(),
            execution_mode: StartAgentExecutionMode::Remote {
                environment_id: String::new(),
                skill_references: skill_references.clone(),
                model_id: String::new(),
                computer_use_enabled: false,
                worker_host: String::new(),
                harness_type: String::new(),
                title: String::new(),
                auth_secret_name: None,
                runner_id: String::new(),
                agent_identity_uid: None,
            },
            lifecycle_subscription: None,
            parent_conversation_id: crate::ai::agent::conversation::AIConversationId::new(),
            parent_run_id: Some("parent-run".to_string()),
        };
        app.read(|ctx| {
            let prepared = prepare_remote_child_launch(
                &request,
                RemoteChildLaunchConfig {
                    environment_id: String::new(),
                    skill_references,
                    working_dir: temp.path().to_path_buf(),
                    model_id: String::new(),
                    computer_use_enabled: false,
                    worker_host: String::new(),
                    harness_type: String::new(),
                    title: String::new(),
                    auth_secret_name: None,
                    runner_id: String::new(),
                    agent_identity_uid: None,
                },
                ctx,
            )
            .unwrap();
            assert_eq!(prepared.spawn_request.runtime_skills.len(), 4);
        });
    });
}

#[test]
fn missing_repo_qualified_skill_reports_repository_and_reason() {
    let temp = TempDir::new().unwrap();

    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        let request = StartAgentRequest {
            id: Default::default(),
            name: "child".to_string(),
            prompt: "Run".to_string(),
            execution_mode: StartAgentExecutionMode::Remote {
                environment_id: String::new(),
                skill_references: Vec::new(),
                model_id: String::new(),
                computer_use_enabled: false,
                worker_host: String::new(),
                harness_type: String::new(),
                title: String::new(),
                auth_secret_name: None,
                runner_id: String::new(),
                agent_identity_uid: None,
            },
            lifecycle_subscription: None,
            parent_conversation_id: crate::ai::agent::conversation::AIConversationId::new(),
            parent_run_id: Some("parent-run".to_string()),
        };
        let error = app.read(|ctx| {
            prepare_remote_child_launch(
                &request,
                RemoteChildLaunchConfig {
                    skill_references: vec![SkillReference::Path(
                        warp_util::local_or_remote_path::LocalOrRemotePath::Local(
                            "missing-repo:missing-skill".into(),
                        ),
                    )],
                    working_dir: temp.path().to_path_buf(),
                    ..config("")
                },
                ctx,
            )
            .unwrap_err()
        });
        let message = error.user_message();
        assert!(message.contains("missing-repo"));
        assert!(message.contains("Repository 'missing-repo' not found"));
    });
}
#[test]
fn github_auth_error_is_a_shared_blocker_with_cloud_callback_url() {
    let error = anyhow::Error::new(ClientError {
        error: "GitHub authentication required".to_string(),
        auth_url: Some("https://example.com/auth?scheme=warpdev".to_string()),
    });
    let CloudAgentStartupIssue::Blocked(CloudAgentStartupBlocker::GitHubAuthRequired {
        message,
        auth_url,
    }) = classify_cloud_agent_startup_error(&error)
    else {
        panic!("expected GitHub auth blocker");
    };
    assert_eq!(message, "GitHub authentication required");
    assert!(auth_url.starts_with("https://example.com/auth?"));
    assert!(auth_url.contains("next="));
}

#[test]
fn cloud_startup_presentations_preserve_gui_copy_and_child_retry_semantics() {
    assert_eq!(
        CloudAgentStartupPresentation::failure("Server error"),
        CloudAgentStartupPresentation {
            title: "Failed to start environment",
            detail: "Server error".to_string(),
            action_label: None,
            primary_url: None,
        }
    );
    assert_eq!(
        CloudAgentStartupPresentation::github_auth(
            "https://example.com/auth",
            CloudAgentStartupAuthFlow::RetryRetainedRequest,
        ),
        CloudAgentStartupPresentation {
            title: "GitHub Authentication Required",
            detail: "Please authenticate with GitHub to continue".to_string(),
            action_label: Some("Authenticate with GitHub"),
            primary_url: Some("https://example.com/auth".to_string()),
        }
    );
    assert_eq!(
        CloudAgentStartupPresentation::github_auth(
            "https://example.com/auth",
            CloudAgentStartupAuthFlow::RerunOrchestrationRequest,
        )
        .detail,
        "Authenticate with GitHub, then run the orchestration request again."
    );
}

#[test]
fn capacity_quota_and_fallback_errors_keep_their_semantics() {
    let capacity = anyhow::Error::new(CloudAgentCapacityError {
        error: "Too many agents".to_string(),
        running_agents: 4,
    });
    assert_eq!(
        classify_cloud_agent_startup_error(&capacity),
        CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Capacity {
            message: "Too many agents".to_string(),
        })
    );

    let quota = anyhow::Error::new(AIApiError::QuotaLimit {
        user_display_message: Some("Buy more credits".to_string()),
    });
    assert_eq!(
        classify_cloud_agent_startup_error(&quota),
        CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::OutOfCredits {
            message: "Buy more credits".to_string(),
        })
    );

    let fallback = anyhow!("network unavailable");
    assert_eq!(
        classify_cloud_agent_startup_error(&fallback),
        CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Other {
            message: "network unavailable".to_string(),
        })
    );
}
