use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};

use super::classify_driver_error;
use crate::ai::agent_sdk::driver::terminal::ShareSessionError;
use crate::ai::agent_sdk::driver::AgentDriverError;

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
fn bootstrap_failed_is_error_with_internal() {
    assert_state_and_code(
        AgentDriverError::BootstrapFailed,
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
    );
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
        AgentDriverError::MCPServerNotFound(uuid::Uuid::nil()),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::EnvironmentSetupFailed),
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
