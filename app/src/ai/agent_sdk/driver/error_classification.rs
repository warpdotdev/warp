use crate::ai::blocklist::task_status_sync_model::classify_renderable_error;
use crate::localization;
use crate::server::server_api::ai::TaskStatusUpdate;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};
use warp_localization::{LocaleId, replace_placeholders};

use super::AgentDriverError;
use super::terminal::ShareSessionError;

fn text(key: &str) -> String {
    localization::text_for_locale(LocaleId::EnUs, key)
}

fn text_with_args(key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&text(key), args)
        .expect("localized text template arguments must match the catalog")
}

/// Classify an `AgentDriverError` into a task state and a `TaskStatusUpdate`
/// suitable for reporting via `update_agent_task`.
pub fn classify_driver_error(error: &AgentDriverError) -> (AgentTaskState, TaskStatusUpdate) {
    match error {
        // --- Warp-side errors (task → ERROR) ---
        AgentDriverError::TerminalUnavailable | AgentDriverError::InvalidRuntimeState => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.internal_error"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::BootstrapFailed => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.bootstrap_failed"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::ShareSessionFailed { error: share_err } => {
            let message = match share_err {
                ShareSessionError::Internal(_) => {
                    text("agent_sdk.driver.error_classification.share_internal")
                }
                ShareSessionError::Failed(reason) => {
                    // The reason string comes from the session-sharing layer and is aimed at
                    // interactive users (e.g. "try sharing again"). Provide a cloud-agent-
                    // appropriate message instead of wrapping it, which would produce
                    // repetitive "try again" text.
                    text_with_args(
                        "agent_sdk.driver.error_classification.share_failed",
                        &[("reason", reason)],
                    )
                }
                ShareSessionError::Disabled => {
                    text("agent_sdk.driver.error_classification.share_disabled")
                }
                ShareSessionError::Timeout => {
                    text("agent_sdk.driver.error_classification.share_timeout")
                }
                ShareSessionError::Interrupted => {
                    text("agent_sdk.driver.error_classification.share_interrupted")
                }
            };
            (
                AgentTaskState::Error,
                TaskStatusUpdate::with_error_code(
                    message,
                    match share_err {
                        ShareSessionError::Disabled => PlatformErrorCode::FeatureNotAvailable,
                        _ => PlatformErrorCode::InternalError,
                    },
                ),
            )
        }
        AgentDriverError::WarpDriveSyncFailed => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.warp_drive_sync_failed"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::NotLoggedIn => {
            let bin = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());
            (
                AgentTaskState::Error,
                TaskStatusUpdate::with_error_code(
                    text_with_args(
                        "agent_sdk.driver.error_classification.not_logged_in",
                        &[("bin", &bin)],
                    ),
                    PlatformErrorCode::AuthenticationRequired,
                ),
            )
        }
        AgentDriverError::CloudProviderSetupFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.cloud_provider_setup_failed",
                    &[("error", &format!("{err:#}"))],
                ),
                PlatformErrorCode::InternalError,
            ),
        ),

        // --- User-side errors (task → FAILED) ---
        AgentDriverError::MCPServerNotFound(uuid) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.mcp_server_not_found",
                    &[("uuid", &uuid.to_string())],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPStartupFailed => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.mcp_startup_failed"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPJsonParseError(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.mcp_json_parse_error",
                    &[("message", msg)],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPMissingVariables => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.mcp_missing_variables"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ProfileError(name) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.profile_not_found",
                    &[("name", name)],
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::AIWorkflowNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.saved_prompt_not_found",
                    &[("id", id)],
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.environment_not_found",
                    &[("id", id)],
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentSetupFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.environment_setup_failed",
                    &[("message", msg)],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::InvalidWorkingDirectory { path, .. } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.invalid_working_directory",
                    &[("path", &path.display().to_string())],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),

        // --- Conversation errors ---
        // Delegate to classify_renderable_error for proper ERROR vs FAILED
        // distinction and PlatformErrorCode. This is a belt-and-suspenders
        // fallback — TaskStatusSyncModel handles most conversation errors,
        // but the driver catches them too if the conversation ends with an error.
        AgentDriverError::ConversationError { error } => {
            let (state, update) = classify_renderable_error(error);
            (
                state,
                update.unwrap_or_else(|| {
                    TaskStatusUpdate::with_error_code(
                        error.to_string(),
                        PlatformErrorCode::InternalError,
                    )
                }),
            )
        }

        // --- Cancellation / Blocked (no error code) ---
        AgentDriverError::ConversationCancelled { .. } => (
            AgentTaskState::Cancelled,
            TaskStatusUpdate::message(text(
                "agent_sdk.driver.error_classification.conversation_cancelled",
            )),
        ),
        AgentDriverError::ConversationBlocked { blocked_action } => (
            AgentTaskState::Blocked,
            TaskStatusUpdate::message(text_with_args(
                "agent_sdk.driver.error_classification.conversation_blocked",
                &[("blocked_action", blocked_action)],
            )),
        ),

        // --- Setup errors ---
        AgentDriverError::TeamMetadataRefreshTimeout => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text("agent_sdk.driver.error_classification.team_metadata_refresh_timeout"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SkillResolutionFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.skill_resolution_failed",
                    &[("message", msg)],
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::ConfigBuildFailed(err) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.config_build_failed",
                    &[("error", &err.to_string())],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::PromptResolutionFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.prompt_resolution_failed",
                    &[("error", &err.to_string())],
                ),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SecretsFetchFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.secrets_fetch_failed",
                    &[("error", &err.to_string())],
                ),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::AwsBedrockCredentialsFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.aws_bedrock_credentials_failed",
                    &[("message", msg)],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ConversationLoadFailed(msg) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.conversation_load_failed",
                    &[("message", msg)],
                ),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::ConversationHarnessMismatch {
            conversation_id,
            expected,
            got,
        } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.conversation_harness_mismatch",
                    &[
                        ("conversation_id", conversation_id),
                        ("expected", expected),
                        ("got", got),
                    ],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::TaskHarnessMismatch {
            task_id,
            expected,
            got,
        } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.task_harness_mismatch",
                    &[("task_id", task_id), ("expected", expected), ("got", got)],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ConversationResumeStateMissing {
            harness,
            conversation_id,
        } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.conversation_resume_state_missing",
                    &[("conversation_id", conversation_id), ("harness", harness)],
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::HarnessCommandFailed { exit_code } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.harness_command_failed",
                    &[("exit_code", &exit_code.to_string())],
                ),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::HarnessSetupFailed { harness, reason } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.harness_setup_failed",
                    &[("harness", harness), ("reason", reason)],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessConfigSetupFailed { harness, error } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                text_with_args(
                    "agent_sdk.driver.error_classification.harness_config_setup_failed",
                    &[("harness", harness), ("error", &error.to_string())],
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessAuthCheckFailed { harness, detail } => {
            let message = text_with_args(
                "agent_sdk.driver.error_classification.harness_auth_check_failed",
                &[("harness", harness)],
            );
            log::error!("Preflight detail for {harness}: {detail}");
            (
                AgentTaskState::Failed,
                TaskStatusUpdate::with_error_code(
                    message,
                    PlatformErrorCode::AuthenticationRequired,
                ),
            )
        }
        AgentDriverError::HarnessRuntimeFailureDetected {
            harness,
            pattern,
            excerpt,
        } => {
            let message = text_with_args(
                "agent_sdk.driver.error_classification.harness_runtime_failure_detected",
                &[
                    ("harness", harness),
                    ("pattern", pattern),
                    ("excerpt", excerpt),
                ],
            );
            log::error!("Runtime failure for {harness}: pattern={pattern}, excerpt={excerpt}");
            (
                AgentTaskState::Failed,
                TaskStatusUpdate::with_error_code(
                    message,
                    PlatformErrorCode::AuthenticationRequired,
                ),
            )
        }
    }
}

#[cfg(test)]
#[path = "error_classification_tests.rs"]
mod tests;
