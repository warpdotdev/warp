use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};

use super::classify_driver_error;
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::agent_sdk::driver::terminal::{BootstrapError, ShareSessionError};

fn managed_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str("019f0016-9e6a-762e-9034-dcb7aec284b0").unwrap()
}

fn assert_state_and_code(
    error: AgentDriverError,
    expected_state: AgentTaskState,
    expected_code: Option<PlatformErrorCode>,
) {
    let (state, update) = classify_driver_error(&error);
    assert_eq!(state, expected_state, "unexpected state for {error}");
    assert_eq!(
        update.error_code, expected_code,
        "unexpected error_code for {error}"
    );
}

// --- Infrastructure errors → ERROR ---

#[test]
fn bootstrap_pty_spawn_failed_with_reason_includes_reason_in_message() {
    let (state, update) = classify_driver_error(&AgentDriverError::BootstrapFailed {
        error: BootstrapError::PtySpawnFailed {
            reason: Some("Argument list too long (os error 7)".to_string()),
        },
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
    assert!(
        update.message.contains("Argument list too long"),
        "message should include the specific failure reason: {:?}",
        update.message
    );
}

#[test]
fn bootstrap_pty_spawn_failed_without_reason_is_generic() {
    let (state, update) = classify_driver_error(&AgentDriverError::BootstrapFailed {
        error: BootstrapError::PtySpawnFailed { reason: None },
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
    assert!(
        update.message.contains("Shell spawn failed"),
        "message should describe the spawn failure: {:?}",
        update.message
    );
}

#[test]
fn bootstrap_timed_out_is_error_with_internal() {
    let (state, update) = classify_driver_error(&AgentDriverError::BootstrapFailed {
        error: BootstrapError::TimedOut,
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
    assert!(
        update.message.contains("did not start within"),
        "message should describe the timeout: {:?}",
        update.message
    );
}

#[test]
fn bootstrap_internal_error_is_error_with_internal() {
    let (state, update) = classify_driver_error(&AgentDriverError::BootstrapFailed {
        error: BootstrapError::InternalError,
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
}

#[test]
fn terminal_unavailable_is_error_with_internal() {
    assert_state_and_code(
        AgentDriverError::TerminalUnavailable,
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
    );
}

#[test]
fn not_logged_in_is_error_with_auth_required() {
    let (state, update) = classify_driver_error(&AgentDriverError::NotLoggedIn);
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::AuthenticationRequired)
    );
    assert!(
        update.message.contains("WARP_API_KEY"),
        "message should mention WARP_API_KEY: {:?}",
        update.message
    );
}

#[test]
fn warp_drive_sync_failed_is_error() {
    assert_state_and_code(
        AgentDriverError::WarpDriveSyncFailed,
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
    );
}

// --- Config/user errors → FAILED ---

#[test]
fn mcp_server_not_found_is_failed_with_env_setup() {
    assert_state_and_code(
        AgentDriverError::MCPServerNotFound {
            uuid: uuid::Uuid::nil(),
            name: None,
        },
        AgentTaskState::Failed,
        Some(PlatformErrorCode::EnvironmentSetupFailed),
    );
}

#[test]
fn mcp_server_not_found_names_the_configured_server() {
    let uuid = managed_uuid();
    let (_, update) = classify_driver_error(&AgentDriverError::MCPServerNotFound {
        uuid,
        name: Some("sentry".to_string()),
    });

    assert_eq!(
        update.message,
        format!(
            "MCP server 'sentry' ({uuid}) was not found. Verify the server exists in your Warp Drive and the UUID is correct."
        )
    );
}

#[test]
fn managed_mcp_resolution_failed_is_failed_with_env_setup() {
    assert_state_and_code(
        AgentDriverError::ManagedMcpResolutionFailed {
            uuid: uuid::Uuid::nil(),
            name: None,
            message: "not active".into(),
        },
        AgentTaskState::Failed,
        Some(PlatformErrorCode::EnvironmentSetupFailed),
    );
}

#[test]
fn managed_mcp_resolution_failed_names_the_configured_server() {
    let uuid = managed_uuid();
    let (_, update) = classify_driver_error(&AgentDriverError::ManagedMcpResolutionFailed {
        uuid,
        name: Some("sentry".to_string()),
        message: "received non-OK response code 429 Too Many Requests".to_string(),
    });

    assert_eq!(
        update.message,
        format!(
            "Managed MCP server 'sentry' ({uuid}) could not be resolved: received non-OK response code 429 Too Many Requests"
        )
    );
}

/// Every reference that carries no usable name must fall back to today's
/// UID-only wording instead of rendering an empty or doubled identifier.
#[test]
fn managed_mcp_resolution_failed_falls_back_to_the_uid_alone() {
    let uuid = managed_uuid();
    let unnamed = [
        // `oz agent run --mcp <uuid>` configures no name at all.
        None,
        // That reference also makes the UUID its own `mcp_servers` key.
        Some(uuid.to_string()),
        // Non-canonical spellings of the same UUID are accepted as a `warp_id`.
        Some(uuid.to_string().to_uppercase()),
        Some(uuid.simple().to_string()),
        // Nothing rejects a blank `mcp_servers` key.
        Some(String::new()),
        Some("   ".to_string()),
    ];

    for name in unnamed {
        let (_, update) = classify_driver_error(&AgentDriverError::ManagedMcpResolutionFailed {
            uuid,
            name: name.clone(),
            message: "not active".to_string(),
        });

        assert_eq!(
            update.message,
            format!("Managed MCP server {uuid} could not be resolved: not active"),
            "name {name:?} must not be rendered"
        );
    }
}

#[test]
fn managed_mcp_resolution_failed_trims_a_padded_name() {
    let uuid = managed_uuid();
    let (_, update) = classify_driver_error(&AgentDriverError::ManagedMcpResolutionFailed {
        uuid,
        name: Some("  sentry ".to_string()),
        message: "not active".to_string(),
    });

    assert_eq!(
        update.message,
        format!("Managed MCP server 'sentry' ({uuid}) could not be resolved: not active")
    );
}

#[test]
fn mcp_startup_failed_is_failed_with_env_setup_and_per_server_details() {
    let (state, update) = classify_driver_error(&AgentDriverError::MCPStartupFailed {
        details: vec![
            "'devin' failed to start: connection refused".to_string(),
            "'datadog' did not start within 20s".to_string(),
        ],
    });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::EnvironmentSetupFailed)
    );
    // Each unavailable server is rendered as its own bullet line.
    assert!(
        update
            .message
            .contains("- 'devin' failed to start: connection refused")
    );
    assert!(
        update
            .message
            .contains("- 'datadog' did not start within 20s")
    );
}

#[test]
fn environment_setup_failed_is_failed() {
    assert_state_and_code(
        AgentDriverError::EnvironmentSetupFailed("bad repo".into()),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::EnvironmentSetupFailed),
    );
}

#[test]
fn setup_command_exited_shell_is_failed_with_env_setup_and_names_command() {
    let (state, update) = classify_driver_error(&AgentDriverError::SetupCommandExitedShell {
        command: "./setup.sh".into(),
    });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::EnvironmentSetupFailed)
    );
    // The message must name the setup command that exited the shell and
    // point the user at the environment's setup commands.
    assert!(update.message.contains("./setup.sh"), "{}", update.message);
    assert!(
        update
            .message
            .contains("Check the setup commands for this environment"),
        "{}",
        update.message
    );
}

#[test]
fn profile_error_is_failed_with_resource_not_found() {
    assert_state_and_code(
        AgentDriverError::ProfileError("my-profile".into()),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::ResourceNotFound),
    );
}

#[test]
fn environment_not_found_is_failed_with_resource_not_found() {
    assert_state_and_code(
        AgentDriverError::EnvironmentNotFound("env-123".into()),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::ResourceNotFound),
    );
}

#[test]
fn conversation_harness_mismatch_is_failed_with_env_setup() {
    let (state, update) = classify_driver_error(&AgentDriverError::ConversationHarnessMismatch {
        conversation_id: "conv-123".into(),
        expected: "claude".into(),
        got: "oz".into(),
    });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::EnvironmentSetupFailed)
    );
    assert!(update.message.contains("conv-123"));
    assert!(update.message.contains("--harness claude"));
}

#[test]
fn conversation_resume_state_missing_is_failed_with_resource_not_found() {
    let (state, update) =
        classify_driver_error(&AgentDriverError::ConversationResumeStateMissing {
            harness: "claude".into(),
            conversation_id: "conv-123".into(),
        });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(update.error_code, Some(PlatformErrorCode::ResourceNotFound));
    assert!(update.message.contains("conv-123"));
    assert!(update.message.contains("claude"));
}

// --- ShareSessionFailed variants ---

#[test]
fn share_session_disabled_gets_feature_not_available() {
    let (state, update) = classify_driver_error(&AgentDriverError::ShareSessionFailed {
        error: ShareSessionError::Disabled,
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::FeatureNotAvailable)
    );
    assert!(update.message.contains("not enabled"));
    assert!(update.message.contains("--share flag"));
}

#[test]
fn share_session_timeout_gets_internal_error() {
    let (state, update) = classify_driver_error(&AgentDriverError::ShareSessionFailed {
        error: ShareSessionError::Timeout,
    });
    assert_eq!(state, AgentTaskState::Error);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
    assert!(update.message.contains("timed out"));
}

#[test]
fn share_session_failed_includes_reason() {
    let (state, update) = classify_driver_error(&AgentDriverError::ShareSessionFailed {
        error: ShareSessionError::Failed("server rejected".into()),
    });
    assert_eq!(state, AgentTaskState::Error);
    assert!(update.message.contains("server rejected"));
}

// --- Conversation-level outcomes ---

#[test]
fn conversation_cancelled_is_cancelled() {
    let (state, update) = classify_driver_error(&AgentDriverError::ConversationCancelled {
        reason: crate::ai::agent::CancellationReason::ManuallyCancelled,
    });
    assert_eq!(state, AgentTaskState::Cancelled);
    assert!(update.error_code.is_none());
}

#[test]
fn conversation_blocked_is_blocked() {
    let (state, update) = classify_driver_error(&AgentDriverError::ConversationBlocked {
        blocked_action: "rm -rf /".into(),
    });
    assert_eq!(state, AgentTaskState::Blocked);
    assert!(update.message.contains("rm -rf /"));
}

// --- Harness auth preflight errors ---

#[test]
fn harness_auth_check_failed_is_failed_with_auth_required() {
    let (state, update) = classify_driver_error(&AgentDriverError::HarnessAuthCheckFailed {
        harness: "claude".into(),
        detail: "exit code 1".into(),
    });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::AuthenticationRequired)
    );
    assert!(update.message.contains("authentication check failed"));
    assert!(update.message.contains("claude"));
}

// --- Runtime failure detection ---

#[test]
fn harness_runtime_failure_detected_is_failed_with_auth_required() {
    let (state, update) = classify_driver_error(&AgentDriverError::HarnessRuntimeFailureDetected {
        harness: "claude".into(),
        pattern: "credit balance is too low".into(),
        excerpt: "Error: Your credit balance is too low to make this request.".into(),
    });
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(
        update.error_code,
        Some(PlatformErrorCode::AuthenticationRequired)
    );
    // The user-visible message must surface both the matched pattern and
    // the excerpt so on-call/users have actionable context.
    assert!(update.message.contains("claude"));
    assert!(update.message.contains("credit balance is too low"));
    assert!(update.message.contains("Your credit balance is too low"));
}
