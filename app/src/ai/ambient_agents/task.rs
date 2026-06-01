//! Ambient agent task types and utilities.

use anyhow::anyhow;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
#[cfg(not(target_family = "wasm"))]
pub use cloud_object_models::HarnessModelConfig;
pub use cloud_object_models::{AgentConfigSnapshot, HarnessAuthSecretsConfig, HarnessConfig};
use iso8601_duration::Duration as Iso8601Duration;
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::SessionId;
use url::Url;
use warp_core::report_error;
use warp_core::ui::theme::WarpTheme;
use warpui::color::ColorU;
use warpui::{SingletonEntity, View, ViewContext};

use super::AmbientAgentTaskId;
use crate::ai::artifacts::{deserialize_artifacts, Artifact};
use crate::server::server_api::ServerApiProvider;
use crate::ui_components::icons::Icon;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;

fn parse_session_id_from_link(session_link: &str) -> Option<SessionId> {
    Url::parse(session_link).ok().and_then(|url| {
        url.path_segments()
            .into_iter()
            .flatten()
            .last()
            .and_then(|segment| segment.parse().ok())
    })
}

fn parse_execution_session_id(execution: RunExecution<'_>) -> Option<SessionId> {
    execution
        .session_id
        .and_then(|id| id.parse().ok())
        .or_else(|| execution.session_link.and_then(parse_session_id_from_link))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSource {
    Linear,
    AgentWebhook,
    Slack,
    Cli,
    ScheduledAgent,
    Interactive,
    WebApp,
    GitHubAction,
    CloudMode,
}

impl AgentSource {
    pub fn as_str(&self) -> &str {
        match self {
            AgentSource::Linear => "LINEAR",
            AgentSource::AgentWebhook => "API",
            AgentSource::Slack => "SLACK",
            AgentSource::Cli => "CLI",
            AgentSource::ScheduledAgent => "SCHEDULED_AGENT",
            // The public API's run source for local interactive tasks is named
            // `LOCAL`.
            AgentSource::Interactive => "LOCAL",
            AgentSource::WebApp => "WEB_APP",
            AgentSource::GitHubAction => "GITHUB_ACTION",
            AgentSource::CloudMode => "CLOUD_MODE",
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            AgentSource::Linear => "Linear",
            AgentSource::AgentWebhook => "API",
            AgentSource::Slack => "Slack",
            AgentSource::Cli => "CLI",
            AgentSource::ScheduledAgent => "Scheduled",
            AgentSource::Interactive | AgentSource::CloudMode => "Warp App",
            AgentSource::WebApp => "Oz Web",
            AgentSource::GitHubAction => "GitHub Action",
        }
    }

    /// Returns true when tasks from this source must not accept user-triggered cloud follow-ups.
    pub fn blocks_cloud_followups(&self) -> bool {
        match self {
            AgentSource::GitHubAction => true,
            AgentSource::Linear
            | AgentSource::AgentWebhook
            | AgentSource::Slack
            | AgentSource::Cli
            | AgentSource::ScheduledAgent
            | AgentSource::Interactive
            | AgentSource::WebApp
            | AgentSource::CloudMode => false,
        }
    }

    /// Returns true if this source represents a user-initiated conversation
    /// (as opposed to automated/programmatic sources like CLI or scheduled runs).
    pub fn is_user_initiated(&self) -> bool {
        match self {
            AgentSource::Linear
            | AgentSource::Slack
            | AgentSource::Interactive
            | AgentSource::WebApp
            | AgentSource::CloudMode => true,
            AgentSource::Cli
            | AgentSource::ScheduledAgent
            | AgentSource::AgentWebhook
            | AgentSource::GitHubAction => false,
        }
    }
}

fn deserialize_ambient_agent_source<'de, D>(
    deserializer: D,
) -> Result<Option<AgentSource>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = serde::Deserialize::deserialize(deserializer)?;
    Ok(match s {
        Some(s) => match s.as_str() {
            "LINEAR" => Some(AgentSource::Linear),
            "AGENT_WEBHOOK" | "API" => Some(AgentSource::AgentWebhook),
            "SLACK" => Some(AgentSource::Slack),
            "LOCAL" => Some(AgentSource::Interactive),
            "CLI" => Some(AgentSource::Cli),
            "SCHEDULED_AGENT" => Some(AgentSource::ScheduledAgent),
            "WEB_APP" => Some(AgentSource::WebApp),
            "GITHUB_ACTION" => Some(AgentSource::GitHubAction),
            "CLOUD_MODE" => Some(AgentSource::CloudMode),
            _ => {
                report_error!(anyhow!("Unknown AmbientAgentSource: {}", s));
                None
            }
        },
        None => None,
    })
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct AmbientAgentTask {
    pub task_id: AmbientAgentTaskId,
    #[serde(default)]
    pub parent_run_id: Option<String>,
    pub title: String,
    pub state: AmbientAgentTaskState,
    pub prompt: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub run_time: Option<Iso8601Duration>,
    pub status_message: Option<TaskStatusMessage>,
    #[serde(default, deserialize_with = "deserialize_ambient_agent_source")]
    pub source: Option<AgentSource>,
    pub session_id: Option<String>,
    pub session_link: Option<String>,
    pub creator: Option<TaskPrincipalInfo>,
    #[serde(default)]
    pub executor: Option<TaskPrincipalInfo>,
    pub conversation_id: Option<String>,
    pub request_usage: Option<RequestUsage>,
    pub is_sandbox_running: bool,

    /// Snapshot of the agent config used to create the task.
    #[serde(default, alias = "agent_config")]
    pub agent_config_snapshot: Option<AgentConfigSnapshot>,
    #[serde(default, deserialize_with = "deserialize_artifacts")]
    pub artifacts: Vec<Artifact>,

    /// The last event sequence number recorded for this run by the server.
    /// Used by orchestration event delivery to resume from the correct
    /// cursor on restart. Populated by `GET /agent/runs/{run_id}` when the
    /// server supports it; `None` on older servers.
    #[serde(default)]
    pub last_event_sequence: Option<i64>,

    /// The server-recorded `run_id`s of direct children of this run. Used
    /// by orchestration event-delivery restore to discover children whose
    /// records may not exist locally (e.g. remote-worker children in the
    /// driver case). Empty on older servers.
    #[serde(default)]
    pub children: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunExecution<'a> {
    pub session_id: Option<&'a str>,
    pub session_link: Option<&'a str>,
    pub request_usage: Option<&'a RequestUsage>,
    pub is_sandbox_running: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientAgentLiveSessionState {
    /// The task does not currently have a running execution with a joinable session signal.
    Inactive,
    /// The task has a running execution, but this client does not have a parsed
    /// shared-session id it can attach to.
    ActiveUnattachable,
    /// The task has a running execution and this client can attach to its shared session.
    Attachable { session_id: SessionId },
}

impl RunExecution<'_> {
    pub fn has_joinable_session(&self) -> bool {
        self.session_id.is_some() || self.session_link.is_some()
    }

    pub fn is_active(&self) -> bool {
        self.is_sandbox_running && self.has_joinable_session()
    }
}

/// Represents a single attachment input from the client (e.g., file upload)
#[derive(Clone, Debug, Serialize)]
pub struct AttachmentInput {
    pub file_name: String,
    pub mime_type: String,
    pub data: String, // base64-encoded data
}

/// Information about a task attachment retrieved from the server
#[derive(Clone, Debug)]
pub struct TaskAttachment {
    pub file_id: String,
    pub filename: String,
    pub download_url: String,
    pub mime_type: String,
}

/// Returns the trimmed orchestrator agent name, or `None` when empty / whitespace-only.
pub fn normalize_orchestrator_agent_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl AmbientAgentTask {
    pub fn run_id(&self) -> AmbientAgentTaskId {
        self.task_id
    }

    /// Returns the short label for this task: trimmed `agent_config_snapshot.name`,
    /// trimmed `title`, or `"Agent"`.
    pub fn display_name(&self) -> &str {
        if let Some(name) = self
            .agent_config_snapshot
            .as_ref()
            .and_then(|c| c.name.as_deref())
        {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        let trimmed_title = self.title.trim();
        if !trimmed_title.is_empty() {
            return trimmed_title;
        }
        "Agent"
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    /// Returns true when this task's source must not accept user-triggered cloud follow-ups.
    pub fn blocks_cloud_followups(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(AgentSource::blocks_cloud_followups)
    }

    pub fn active_run_execution(&self) -> RunExecution<'_> {
        RunExecution {
            session_id: self.session_id.as_deref(),
            session_link: self.session_link.as_deref().filter(|link| !link.is_empty()),
            request_usage: self.request_usage.as_ref(),
            is_sandbox_running: self.is_sandbox_running,
        }
    }

    pub fn active_execution_session_id(&self) -> Option<&str> {
        let execution = self.active_run_execution();
        if self.state == AmbientAgentTaskState::InProgress && execution.is_active() {
            execution.session_id
        } else {
            None
        }
    }

    /// Returns the canonical live-session state for this task from the client's perspective.
    ///
    /// This separates task liveness from attachability: an in-progress task can have an active
    /// execution without a usable shared-session id, and callers should not treat that as a
    /// completed transcript/follow-up state.
    pub fn active_live_session_state(&self) -> AmbientAgentLiveSessionState {
        let execution = self.active_run_execution();
        if self.state != AmbientAgentTaskState::InProgress || !execution.is_active() {
            return AmbientAgentLiveSessionState::Inactive;
        }

        match parse_execution_session_id(execution) {
            Some(session_id) => AmbientAgentLiveSessionState::Attachable { session_id },
            None => AmbientAgentLiveSessionState::ActiveUnattachable,
        }
    }

    pub fn active_execution_conversation_id(&self) -> Option<&str> {
        if self.has_active_execution() {
            self.conversation_id()
        } else {
            None
        }
    }

    pub fn has_active_execution(&self) -> bool {
        self.state == AmbientAgentTaskState::InProgress && self.active_run_execution().is_active()
    }

    pub fn is_terminal_run_state(&self) -> bool {
        self.state.is_terminal()
    }

    pub fn can_submit_cloud_followup(&self) -> bool {
        self.is_terminal_run_state() && !self.has_active_execution()
    }

    /// Total credits used (inference + compute + platform).
    pub fn credits_used(&self) -> Option<f32> {
        self.active_run_execution().request_usage.map(|u| {
            (u.inference_cost.unwrap_or(0.0)
                + u.compute_cost.unwrap_or(0.0)
                + u.platform_cost.unwrap_or(0.0)) as f32
        })
    }

    /// Server-reported run duration.
    pub fn run_time(&self) -> Option<ChronoDuration> {
        self.run_time.and_then(|run_time| run_time.to_chrono())
    }

    /// Creator's display name, if available.
    pub fn creator_display_name(&self) -> Option<String> {
        self.creator.as_ref().and_then(|c| c.display_name.clone())
    }

    /// Principal the run executed as, formatted for user-facing surfaces.
    pub fn executor_display_name(&self) -> Option<String> {
        self.executor.as_ref().and_then(|e| e.display_name.clone())
    }

    /// Returns true if the underlying session for the ambient agent is no longer running.
    pub fn is_no_longer_running(&self) -> bool {
        !self.active_run_execution().is_sandbox_running && !self.state.is_working()
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AmbientAgentTaskState {
    Queued,
    Pending,
    Claimed,
    #[serde(alias = "IN_PROGRESS")]
    InProgress,
    Succeeded,
    Failed,
    Error,
    Blocked,
    Cancelled,
    #[serde(other)]
    Unknown,
}

impl AmbientAgentTaskState {
    /// Returns the query param value for the server API.
    pub fn as_query_param(&self) -> Option<&str> {
        match self {
            AmbientAgentTaskState::Queued => Some("QUEUED"),
            AmbientAgentTaskState::Pending => Some("PENDING"),
            AmbientAgentTaskState::Claimed => Some("CLAIMED"),
            AmbientAgentTaskState::InProgress => Some("INPROGRESS"),
            AmbientAgentTaskState::Succeeded => Some("SUCCEEDED"),
            AmbientAgentTaskState::Failed => Some("FAILED"),
            AmbientAgentTaskState::Error => Some("ERROR"),
            AmbientAgentTaskState::Blocked => Some("BLOCKED"),
            AmbientAgentTaskState::Cancelled => Some("CANCELLED"),
            // Unknown states are only for resilient deserialization and should not be
            // sent back as filter values.
            AmbientAgentTaskState::Unknown => None,
        }
    }

    pub fn is_working(&self) -> bool {
        match self {
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed
            | AmbientAgentTaskState::InProgress => true,
            AmbientAgentTaskState::Succeeded
            | AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Blocked
            | AmbientAgentTaskState::Cancelled
            | AmbientAgentTaskState::Unknown => false,
        }
    }

    pub fn is_cancellable(&self) -> bool {
        self.is_working()
    }

    pub fn is_failure_like(&self) -> bool {
        match self {
            AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Blocked
            | AmbientAgentTaskState::Unknown => true,
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed
            | AmbientAgentTaskState::InProgress
            | AmbientAgentTaskState::Succeeded
            | AmbientAgentTaskState::Cancelled => false,
        }
    }

    pub fn is_terminal(&self) -> bool {
        match self {
            AmbientAgentTaskState::Succeeded
            | AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Blocked
            | AmbientAgentTaskState::Cancelled
            | AmbientAgentTaskState::Unknown => true,
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed
            | AmbientAgentTaskState::InProgress => false,
        }
    }

    pub fn status_icon_and_color(&self, theme: &WarpTheme) -> (Icon, ColorU) {
        match self {
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed
            | AmbientAgentTaskState::InProgress => (Icon::ClockLoader, theme.ansi_fg_magenta()),
            AmbientAgentTaskState::Succeeded => (Icon::Check, theme.ansi_fg_green()),
            AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Unknown => (Icon::Triangle, theme.ansi_fg_red()),
            AmbientAgentTaskState::Blocked => (Icon::StopFilled, theme.ansi_fg_yellow()),
            AmbientAgentTaskState::Cancelled => (
                Icon::Cancelled,
                theme.disabled_text_color(theme.background()).into_solid(),
            ),
        }
    }
}

impl std::fmt::Display for AmbientAgentTaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AmbientAgentTaskState::Queued => write!(f, "Queued"),
            AmbientAgentTaskState::Pending => write!(f, "Pending"),
            AmbientAgentTaskState::Claimed => write!(f, "Claimed"),
            AmbientAgentTaskState::InProgress => write!(f, "In progress"),
            AmbientAgentTaskState::Succeeded => write!(f, "Done"),
            AmbientAgentTaskState::Failed => write!(f, "Failed"),
            AmbientAgentTaskState::Error => write!(f, "Error"),
            AmbientAgentTaskState::Blocked => write!(f, "Blocked"),
            AmbientAgentTaskState::Cancelled => write!(f, "Cancelled"),
            AmbientAgentTaskState::Unknown => write!(f, "Failed"),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TaskPrincipalInfo {
    #[serde(rename = "type")]
    pub creator_type: String,
    pub uid: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TaskStatusMessage {
    pub message: String,
    #[serde(default, alias = "errorCode")]
    pub error_code: Option<TaskStatusErrorCode>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatusErrorCode {
    #[serde(alias = "ENVIRONMENT_SETUP_FAILED")]
    EnvironmentSetupFailed,
    #[serde(other)]
    Unknown,
}

impl TaskStatusErrorCode {
    pub fn is_environment_setup_failure(&self) -> bool {
        matches!(self, TaskStatusErrorCode::EnvironmentSetupFailed)
    }
}

impl TaskStatusMessage {
    pub fn is_environment_setup_failure(&self) -> bool {
        self.error_code
            .as_ref()
            .is_some_and(TaskStatusErrorCode::is_environment_setup_failure)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct RequestUsage {
    pub inference_cost: Option<f64>,
    pub compute_cost: Option<f64>,
    pub platform_cost: Option<f64>,
}

/// Cancel an ambient agent task and show a toast with the result.
pub fn cancel_task_with_toast<V: View>(task_id: AmbientAgentTaskId, ctx: &mut ViewContext<V>) {
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
    let window_id = ctx.window_id();
    ctx.spawn(
        async move { ai_client.cancel_ambient_agent_task(&task_id).await },
        move |_view, result, ctx| {
            let message = match result {
                Ok(()) => "Task cancelled".to_string(),
                Err(e) => {
                    log::error!("Failed to cancel task: {e}");
                    format!("Failed to cancel task: {e}")
                }
            };
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast = DismissibleToast::default(message);
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
        },
    );
}

/// Cancel an ambient agent task without surfacing a toast to the user.
pub fn cancel_task_silently<V: View>(task_id: AmbientAgentTaskId, ctx: &mut ViewContext<V>) {
    let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();
    ctx.spawn(
        async move { ai_client.cancel_ambient_agent_task(&task_id).await },
        move |_view, result, _| {
            if let Err(e) = result {
                log::error!("Failed to cancel task: {e}");
            }
        },
    );
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
