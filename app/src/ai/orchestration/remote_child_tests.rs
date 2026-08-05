use anyhow::anyhow;
use warp_cli::agent::Harness;
use warpui::App;

use super::{
    CloudAgentStartupAuthFlow, CloudAgentStartupBlocker, CloudAgentStartupFailure,
    CloudAgentStartupIssue, CloudAgentStartupPresentation, RemoteChildLaunchConfig,
    classify_cloud_agent_startup_error, effective_computer_use_enabled,
    prepare_remote_child_launch, spawn_computer_use_enabled,
};
use crate::ai::agent::{StartAgentExecutionMode, UserQueryMode};
use crate::ai::blocklist::StartAgentRequest;
use crate::server::server_api::{AIApiError, ClientError, CloudAgentCapacityError};

fn config(harness_type: &str) -> RemoteChildLaunchConfig {
    RemoteChildLaunchConfig {
        environment_id: String::new(),
        skill_references: Vec::new(),
        model_id: String::new(),
        computer_use_enabled: None,
        worker_host: String::new(),
        harness_type: harness_type.to_string(),
        title: String::new(),
        auth_secret_name: None,
        runner_id: String::new(),
        agent_identity_uid: None,
    }
}

/// A minimal orchestrated child request with the parent run id set.
fn remote_child_request() -> StartAgentRequest {
    StartAgentRequest {
        id: Default::default(),
        name: "researcher".to_string(),
        prompt: "Inspect the code".to_string(),
        execution_mode: StartAgentExecutionMode::Remote {
            environment_id: String::new(),
            skill_references: Vec::new(),
            model_id: String::new(),
            computer_use_enabled: None,
            worker_host: String::new(),
            harness_type: "oz".to_string(),
            title: String::new(),
            auth_secret_name: None,
            runner_id: String::new(),
            agent_identity_uid: None,
        },
        lifecycle_subscription: None,
        parent_conversation_id: crate::ai::agent::conversation::AIConversationId::new(),
        parent_run_id: Some("parent-run".to_string()),
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
                computer_use_enabled: Some(true),
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
                    computer_use_enabled: Some(true),
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

/// Regression for REMOTE-2444: a `run_agents` call that left computer use
/// unspecified must not send an explicit `false` on the child spawn request,
/// which would override the server's Oz default and force computer use off.
#[test]
fn unspecified_computer_use_is_omitted_from_the_child_spawn_request() {
    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        let request = remote_child_request();
        app.read(|ctx| {
            let mut config = config("oz");
            config.computer_use_enabled = None;
            let prepared = prepare_remote_child_launch(&request, config, ctx).unwrap();
            assert_eq!(
                prepared.spawn_request.config.unwrap().computer_use_enabled,
                None
            );
        });
    });
}

#[test]
fn explicitly_disabled_computer_use_still_reaches_the_child_spawn_request() {
    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        let request = remote_child_request();
        app.read(|ctx| {
            let mut config = config("oz");
            config.computer_use_enabled = Some(false);
            let prepared = prepare_remote_child_launch(&request, config, ctx).unwrap();
            assert_eq!(
                prepared.spawn_request.config.unwrap().computer_use_enabled,
                Some(false)
            );
        });
    });
}

#[test]
fn computer_use_is_dropped_for_third_party_harness_children() {
    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);
        let request = remote_child_request();
        app.read(|ctx| {
            let mut config = config("claude");
            config.computer_use_enabled = Some(true);
            let prepared = prepare_remote_child_launch(&request, config, ctx).unwrap();
            assert_eq!(
                prepared.spawn_request.config.unwrap().computer_use_enabled,
                None
            );
        });
    });
}

#[test]
fn computer_use_resolution_mirrors_the_cloud_default() {
    // Unspecified defers to the server: Oz enables computer use, third-party
    // harnesses do not.
    assert_eq!(spawn_computer_use_enabled(None, Harness::Oz), None);
    assert!(effective_computer_use_enabled(None, Harness::Oz));
    assert_eq!(spawn_computer_use_enabled(None, Harness::Claude), None);
    assert!(!effective_computer_use_enabled(None, Harness::Claude));
    // An explicit choice wins for Oz children.
    assert_eq!(
        spawn_computer_use_enabled(Some(true), Harness::Oz),
        Some(true)
    );
    assert!(effective_computer_use_enabled(Some(true), Harness::Oz));
    assert_eq!(
        spawn_computer_use_enabled(Some(false), Harness::Oz),
        Some(false)
    );
    assert!(!effective_computer_use_enabled(Some(false), Harness::Oz));
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
