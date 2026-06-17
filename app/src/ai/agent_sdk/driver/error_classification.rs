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
                "An internal error occurred. Please try running your task again. If the issue persists, contact support.",
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::BootstrapFailed { error } => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                format!("Terminal session failed to start: {error}"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::ShareSessionFailed { error: share_err } => {
            let message = match share_err {
                ShareSessionError::Internal(_) => {
                    "Failed to share agent session due to an internal error. Please try running your task again.".to_string()
                }
                ShareSessionError::Failed(reason) => {
                    // The reason string comes from the session-sharing layer and is aimed at
                    // interactive users (e.g. "try sharing again"). Provide a cloud-agent-
                    // appropriate message instead of wrapping it, which would produce
                    // repetitive "try again" text.
                    format!("Failed to share agent session: {reason}")
                }
                ShareSessionError::Disabled => {
                    "Session sharing is not enabled for your account. This is likely because \
                     an administrator has disabled session sharing for your team. Please \
                     verify that session sharing is enabled in your team settings, or try \
                     running without the --share flag."
                    .to_string()
                }
                ShareSessionError::Timeout => {
                    "Failed to share agent session: timed out waiting for the session sharing \
                     server to respond. Please check your network connection and try again."
                    .to_string()
                }
                ShareSessionError::Interrupted => {
                    "Session sharing was interrupted before it could complete. Please try running your task again.".to_string()
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
                "Warp Drive failed to sync. Please check your network connection and try again.",
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::NotLoggedIn => {
            let bin = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());
            (
                AgentTaskState::Error,
                TaskStatusUpdate::with_error_code(
                    format!(
                        "Authentication required. Log in via '{bin} login', provide an API key via '--api-key', or set the WARP_API_KEY environment variable."
                    ),
                    PlatformErrorCode::AuthenticationRequired,
                ),
            )
        }
        AgentDriverError::CloudProviderSetupFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                format!("Error configuring cloud access: {err:#}"),
                PlatformErrorCode::InternalError,
            ),
        ),

        // --- User-side errors (task → FAILED) ---
        AgentDriverError::MCPServerNotFound(uuid) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "MCP server {uuid} was not found. Verify the server exists in your Warp Drive and the UUID is correct."
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPStartupFailed { details } => {
            let server_lines = details
                .iter()
                .map(|detail| format!("- {detail}"))
                .collect::<Vec<_>>()
                .join("\n");
            (
                AgentTaskState::Failed,
                TaskStatusUpdate::with_error_code(
                    format!(
                        "One or more MCP servers failed to start:\n\n{server_lines}\n\nCheck that each server's configuration is valid and that it is reachable from the agent's environment."
                    ),
                    PlatformErrorCode::EnvironmentSetupFailed,
                ),
            )
        }
        AgentDriverError::MCPJsonParseError(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Failed to parse MCP server JSON configuration: {msg}"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::MCPMissingVariables => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                "MCP server configuration is missing required variables. Provide all required environment variables or template values.",
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ProfileError(name) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "Agent profile \"{name}\" not found. Check the profile ID and ensure it exists in your team's Warp Drive."
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::AIWorkflowNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "Saved prompt not found for ID {id}. Verify the prompt exists in your Warp Drive."
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentNotFound(id) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "Environment '{id}' not found. Verify the environment ID and ensure it exists in your team settings."
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::EnvironmentSetupFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "Environment setup failed: {msg}. Check your repository URLs and setup commands."
                ),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::InvalidWorkingDirectory { path, .. } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!(
                    "Working directory '{}' does not exist or is not a directory. Verify the path in your environment configuration.",
                    path.display()
                ),
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
            TaskStatusUpdate::message("Task cancelled."),
        ),
        AgentDriverError::ConversationBlocked { blocked_action } => (
            AgentTaskState::Blocked,
            TaskStatusUpdate::message(format!(
                "The agent got stuck waiting for user confirmation on the action: {blocked_action}"
            )),
        ),

        // --- Setup errors ---
        AgentDriverError::TeamMetadataRefreshTimeout => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                "Timed out refreshing team metadata. Please check your network connection and try again.",
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SkillResolutionFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Skill resolution failed: {msg}"),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::ConfigBuildFailed(err) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Failed to build agent configuration: {err}"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::PromptResolutionFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                format!("Failed to resolve prompt for the run: {err}"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::SecretsFetchFailed(err) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                format!("Failed to fetch task secrets: {err}"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::AwsBedrockCredentialsFailed(msg) => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Failed to initialize AWS Bedrock credentials: {msg}"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::ConversationLoadFailed(msg) => (
            AgentTaskState::Error,
            TaskStatusUpdate::with_error_code(
                format!("Failed to load conversation: {msg}"),
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
                format!(
                    "Conversation {conversation_id} was produced by the {expected} harness, but --harness {got} was requested. \
                     Re-run with --harness {expected} (or omit --harness) to continue this conversation."
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
                format!(
                    "Task {task_id} was created with the {expected} harness, but --harness {got} was requested. \
                     Re-run with --harness {expected} (or omit --harness) to continue this task."
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
                format!(
                    "Conversation {conversation_id} has no stored transcript for the {harness} harness. \
                     The prior run may have crashed before saving any state."
                ),
                PlatformErrorCode::ResourceNotFound,
            ),
        ),
        AgentDriverError::HarnessCommandFailed { exit_code } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Harness command exited with code {exit_code}"),
                PlatformErrorCode::InternalError,
            ),
        ),
        AgentDriverError::HarnessSetupFailed { harness, reason } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Harness '{harness}' validation failed: {reason}"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessConfigSetupFailed { harness, error } => (
            AgentTaskState::Failed,
            TaskStatusUpdate::with_error_code(
                format!("Harness '{harness}' config setup failed: {error}"),
                PlatformErrorCode::EnvironmentSetupFailed,
            ),
        ),
        AgentDriverError::HarnessAuthCheckFailed { harness, detail } => {
            let message = format!(
                "Harness '{harness}' authentication check failed: login credentials \
                 are invalid or expired. Verify that the authentication secret \
                 configured for this harness is correct."
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
            let message = format!(
                "Harness '{harness}' could not make a successful API request. \
                 Matched failure pattern '{pattern}' in harness output: \"{excerpt}\". \
                 This usually means the API key is invalid, out of credits, or the \
                 account is misconfigured."
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
