use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};

use super::terminal::ShareSessionError;
use super::AgentDriverError;
use crate::ai::blocklist::local_agent_task_sync_model::classify_renderable_error;
use crate::server::server_api::ai::TaskStatusUpdate;

/// Classify an `AgentDriverError` into a task state and a `TaskStatusUpdate`
/// suitable for reporting via `update_agent_task`.
pub fn classify_driver_error(error: &AgentDriverError) -> (AgentTaskState, TaskStatusUpdate) {
    match error {
        // --- Warp-side errors (task → ERROR) ---
        AgentDriverError::TerminalUnavailable | AgentDriverError::InvalidRuntimeState => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.internal_error"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::BootstrapFailed => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.bootstrap_failed"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::ShareSessionFailed { error: share_err } => {
            let message = match share_err {
                ShareSessionError::Internal(_) => {
                    i18n::t("ai.agent_sdk.driver.error_classification.share_session_internal")
                }
                ShareSessionError::Failed(reason) => {
                    // The reason string comes from the session-sharing layer and is aimed at
                    // interactive users (e.g. "try sharing again"). Provide a cloud-agent-
                    // appropriate message instead of wrapping it, which would produce
                    // repetitive "try again" text.
                    i18n::t("ai.agent_sdk.driver.error_classification.share_session_failed")
                        .replace("{reason}", reason)
                }
                ShareSessionError::Disabled => {
                    i18n::t("ai.agent_sdk.driver.error_classification.share_session_disabled")
                }
                ShareSessionError::Timeout => {
                    i18n::t("ai.agent_sdk.driver.error_classification.share_session_timeout")
                }
                ShareSessionError::Interrupted => {
                    i18n::t("ai.agent_sdk.driver.error_classification.share_session_interrupted")
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
                i18n::t("ai.agent_sdk.driver.error_classification.warp_drive_sync_failed"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::NotLoggedIn => {
            let bin = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());
            (
                AgentTaskState::Error,
                TaskStatusUpdate::with_error_code(
                    i18n::t("ai.agent_sdk.driver.error_classification.not_logged_in")
                        .replace("{bin}", &bin),
                    PlatformErrorCode::AuthenticationRequired,
                ),
            )
        }
        AgentDriverError::CloudProviderSetupFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.cloud_access_failed")
                    .replace("{error}", &format!("{err:#}")),
                PlatformErrorCode::InternalError,
            ),
        ),

        // --- User-side errors (task → FAILED) ---
        AgentDriverError::MCPServerNotFound(uuid) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.mcp_server_not_found")
                    .replace("{uuid}", &uuid.to_string()),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPStartupFailed => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.mcp_startup_failed"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPJsonParseError(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.mcp_json_parse_failed")
                    .replace("{message}", msg),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPMissingVariables => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.mcp_missing_variables"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ProfileError(name) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.profile_not_found")
                    .replace("{name}", name),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::AIWorkflowNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.saved_prompt_not_found")
                    .replace("{id}", id),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.environment_not_found")
                    .replace("{id}", id),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentSetupFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.environment_setup_failed")
                    .replace("{message}", msg),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::InvalidWorkingDirectory { path, .. } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.invalid_working_directory")
                    .replace("{path}", &path.display().to_string()),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),

        // --- Conversation errors ---
        // Delegate to classify_renderable_error for proper ERROR vs FAILED
        // distinction and PlatformErrorCode. This is a belt-and-suspenders
        // fallback — LocalAgentTaskSyncModel handles most conversation errors,
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
            TaskStatusUpdate::message(i18n::t(
                "ai.agent_sdk.driver.error_classification.task_cancelled",
            )),
        ),
        AgentDriverError::ConversationBlocked { blocked_action } => (
            AgentTaskState::Blocked,
            TaskStatusUpdate::message(
                i18n::t("ai.agent_sdk.driver.error_classification.conversation_blocked")
                    .replace("{blocked_action}", blocked_action),
            ),
        ),

        // --- Setup errors ---
        AgentDriverError::TeamMetadataRefreshTimeout => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.team_metadata_timeout"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SkillResolutionFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.skill_resolution_failed")
                    .replace("{message}", msg),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::ConfigBuildFailed(err) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.config_build_failed")
                    .replace("{error}", &err.to_string()),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::PromptResolutionFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.prompt_resolution_failed")
                    .replace("{error}", &err.to_string()),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SecretsFetchFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.secrets_fetch_failed")
                    .replace("{error}", &err.to_string()),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::AwsBedrockCredentialsFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.aws_bedrock_failed")
                    .replace("{message}", msg),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ConversationLoadFailed(msg) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.conversation_load_failed")
                    .replace("{message}", msg),
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
                i18n::t("ai.agent_sdk.driver.error_classification.conversation_harness_mismatch")
                    .replace("{conversation_id}", conversation_id)
                    .replace("{expected}", expected)
                    .replace("{got}", got),
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
                i18n::t("ai.agent_sdk.driver.error_classification.task_harness_mismatch")
                    .replace("{task_id}", task_id)
                    .replace("{expected}", expected)
                    .replace("{got}", got),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ConversationResumeStateMissing {
            harness,
            conversation_id,
        } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.resume_state_missing")
                    .replace("{conversation_id}", conversation_id)
                    .replace("{harness}", harness),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::HarnessCommandFailed { exit_code } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.harness_command_failed")
                    .replace("{exit_code}", &exit_code.to_string()),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::HarnessSetupFailed { harness, reason } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.harness_setup_failed")
                    .replace("{harness}", harness)
                    .replace("{reason}", reason),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessConfigSetupFailed { harness, error } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                i18n::t("ai.agent_sdk.driver.error_classification.harness_config_failed")
                    .replace("{harness}", harness)
                    .replace("{error}", &error.to_string()),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessAuthCheckFailed { harness, detail } => {
            let message =
                i18n::t("ai.agent_sdk.driver.error_classification.harness_auth_check_failed")
                    .replace("{harness}", harness);
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
            let message =
                i18n::t("ai.agent_sdk.driver.error_classification.harness_runtime_failure")
                    .replace("{harness}", harness)
                    .replace("{pattern}", pattern)
                    .replace("{excerpt}", excerpt);
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
