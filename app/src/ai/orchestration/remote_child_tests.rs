use anyhow::anyhow;
use warpui::App;

use super::{
    CloudAgentStartupBlocker, CloudAgentStartupFailure, CloudAgentStartupIssue,
    RemoteChildLaunchConfig, classify_cloud_agent_startup_error, prepare_remote_child_launch,
};
use crate::ai::agent::{StartAgentExecutionMode, UserQueryMode};
use crate::ai::blocklist::StartAgentRequest;
use crate::server::server_api::{AIApiError, ClientError, CloudAgentCapacityError};

fn config(harness_type: &str) -> RemoteChildLaunchConfig {
    RemoteChildLaunchConfig {
        environment_id: String::new(),
        skill_references: Vec::new(),
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
