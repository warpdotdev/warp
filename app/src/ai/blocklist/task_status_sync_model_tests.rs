use super::{classify_renderable_error, map_cli_session_status};
use crate::ai::agent::RenderableAIError;
use crate::server::server_api::ai::TaskStatusUpdate;
use crate::terminal::cli_agent_sessions::CLIAgentSessionStatus;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};

/// Helper to assert a (state, Option<TaskStatusUpdate>) tuple.
fn assert_update(
    (state, update): (AgentTaskState, Option<TaskStatusUpdate>),
    expected_state: AgentTaskState,
    expected_code: Option<PlatformErrorCode>,
    message_contains: Option<&str>,
) {
    assert_eq!(state, expected_state, "unexpected AgentTaskState");
    match (update, expected_code, message_contains) {
        (Some(u), code, msg) => {
            assert_eq!(u.error_code, code, "unexpected PlatformErrorCode");
            if let Some(substr) = msg {
                assert!(
                    u.message.contains(substr),
                    "message {:?} does not contain {:?}",
                    u.message,
                    substr
                );
            }
        }
        (None, None, None) => {}
        (None, _, _) => panic!("expected a TaskStatusUpdate, got None"),
    }
}

// --- classify_renderable_error ---

#[test]
fn quota_limit_is_failed_with_insufficient_credits() {
    assert_update(
        classify_renderable_error(&RenderableAIError::QuotaLimit {
            user_display_message: None,
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::InsufficientCredits),
        Some("credits"),
    );
}

#[test]
fn server_overloaded_is_error_with_resource_unavailable() {
    assert_update(
        classify_renderable_error(&RenderableAIError::ServerOverloaded),
        AgentTaskState::Error,
        Some(PlatformErrorCode::ResourceUnavailable),
        Some("overloaded"),
    );
}

#[test]
fn internal_warp_error_is_error() {
    assert_update(
        classify_renderable_error(&RenderableAIError::InternalWarpError),
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
        Some("internal error"),
    );
}

#[test]
fn context_window_exceeded_is_failed() {
    assert_update(
        classify_renderable_error(&RenderableAIError::ContextWindowExceeded("too big".into())),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::InternalError),
        Some("Context window exceeded"),
    );
}

#[test]
fn invalid_api_key_is_failed_with_auth_required() {
    assert_update(
        classify_renderable_error(&RenderableAIError::InvalidApiKey {
            provider: "OpenAI".into(),
            model_name: "gpt-4".into(),
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::AuthenticationRequired),
        Some("OpenAI"),
    );
}

#[test]
fn aws_bedrock_credentials_is_failed_with_auth_required() {
    assert_update(
        classify_renderable_error(&RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid {
            model_name: "claude-v2".into(),
        }),
        AgentTaskState::Failed,
        Some(PlatformErrorCode::AuthenticationRequired),
        Some("claude-v2"),
    );
}

#[test]
fn other_error_is_error_with_internal() {
    assert_update(
        classify_renderable_error(&RenderableAIError::Other {
            error_message: "something broke".into(),
            will_attempt_resume: false,
            waiting_for_network: false,
        }),
        AgentTaskState::Error,
        Some(PlatformErrorCode::InternalError),
        Some("something broke"),
    );
}

// --- map_cli_session_status ---

#[test]
fn cli_in_progress_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::InProgress);
    assert_eq!(state, AgentTaskState::InProgress);
    assert!(update.is_none());
}

#[test]
fn cli_success_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Success);
    assert_eq!(state, AgentTaskState::Succeeded);
    assert!(update.is_none());
}

#[test]
fn cli_blocked_maps_correctly() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Blocked {
        message: Some("needs approval".into()),
    });
    assert_eq!(state, AgentTaskState::Blocked);
    let update = update.expect("should have status update");
    assert!(update.message.contains("needs approval"));
}

#[test]
fn cli_blocked_without_message() {
    let (state, update) = map_cli_session_status(&CLIAgentSessionStatus::Blocked { message: None });
    assert_eq!(state, AgentTaskState::Blocked);
    assert!(update.is_none());
}
