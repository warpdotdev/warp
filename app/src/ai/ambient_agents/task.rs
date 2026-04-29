//! Ambient agent task types and utilities.

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use warp_cli::agent::Harness;
use warp_core::report_error;
use warp_core::ui::theme::WarpTheme;
use warpui::color::ColorU;

use crate::ai::artifacts::{deserialize_artifacts, Artifact};
use crate::server::server_api::ServerApiProvider;
use crate::ui_components::icons::Icon;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use warpui::{SingletonEntity, View, ViewContext};

use super::AmbientAgentTaskId;

/// Runtime configuration snapshot for agent execution.
///
/// This is the merged/resolved config used when spawning or running an agent.
/// It combines settings from config files and CLI args.
/// Unlike `AgentConfig` (the cloud model), field names here use the runtime format
/// (e.g. `model_id` instead of `base_model_id`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentConfigSnapshot {
    /// Config name for searchability/traceability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_prompt: Option<String>,
    /// MCP server configuration map (unwrapped; no `mcpServers` wrapper).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<serde_json::Map<String, serde_json::Value>>,
    /// Profile ID for local agent runs. This configures the terminal session
    /// with the specified execution profile. Only used for local runs, not cloud runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Self-hosted worker ID that should execute this task.
    /// If None or Some("warp"), the task will be dispatched to Warp-hosted (Namespace) workers.
    /// Otherwise, the task will only be assigned to a connected self-hosted worker with matching ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_host: Option<String>,
    /// Skill spec to use as the base prompt for the agent.
    /// Format: "skill_name", "repo:skill_name", or "org/repo:skill_name".
    /// The skill is resolved at runtime in the agent environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_spec: Option<String>,
    /// Whether computer use is enabled for this agent run.
    /// If None, the default behavior is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computer_use_enabled: Option<bool>,
    /// Execution harness for the agent run.
    /// If None, we use Warp's default ("oz").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessConfig>,
    /// Authentication secrets for third-party harnesses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_auth_secrets: Option<HarnessAuthSecretsConfig>,
}

/// Configuration for a third-party execution harness.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HarnessConfig {
    /// The harness type, e.g. [`Harness::Claude`].
    #[serde(
        rename = "type",
        serialize_with = "serialize_harness",
        deserialize_with = "deserialize_harness"
    )]
    pub harness_type: Harness,
}

impl HarnessConfig {
    /// Builds a harness config from just the harness type.
    pub fn from_harness_type(harness_type: Harness) -> Self {
        Self { harness_type }
    }
}

/// Parses a harness type name (e.g. `"claude"`) into a [`Harness`] variant.
/// Unknown values fall back to [`Harness::Unknown`] so we don't
/// misrepresent a future-server harness as Oz; UI surfaces should treat
/// `Unknown` as a non-Oz, non-runnable harness.
pub(crate) fn harness_from_name(name: &str) -> Harness {
    match name {
        "claude" => Harness::Claude,
        "opencode" => Harness::OpenCode,
        "gemini" => Harness::Gemini,
        "oz" => Harness::Oz,
        other => {
            log::warn!("Unknown harness config name: {other:?}; treating as Unknown");
            Harness::Unknown
        }
    }
}

fn serialize_harness<S: Serializer>(harness: &Harness, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&harness.to_string())
}

fn deserialize_harness<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Harness, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(harness_from_name(&name))
}

/// Authentication secrets for third-party harnesses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarnessAuthSecretsConfig {
    /// Name of a managed secret for Claude Code harness authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_auth_secret_name: Option<String>,
}

impl AgentConfigSnapshot {
    /// Returns true if this config is empty (no options are set).
    pub fn is_empty(&self) -> bool {
        let Self {
            name,
            environment_id,
            model_id,
            base_prompt,
            mcp_servers,
            profile_id,
            worker_host,
            skill_spec,
            computer_use_enabled,
            harness,
            harness_auth_secrets,
        } = self;

        name.is_none()
            && environment_id.is_none()
            && model_id.is_none()
            && base_prompt.is_none()
            && mcp_servers.is_none()
            && profile_id.is_none()
            && worker_host.is_none()
            && skill_spec.is_none()
            && computer_use_enabled.is_none()
            && harness.is_none()
            && harness_auth_secrets.is_none()
    }
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
            AgentSource::Interactive => "Warp (local agent)",
            AgentSource::WebApp => "Oz Web",
            AgentSource::GitHubAction => "GitHub Action",
            AgentSource::CloudMode => "Warp (cloud agent)",
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
    pub status_message: Option<TaskStatusMessage>,
    #[serde(default, deserialize_with = "deserialize_ambient_agent_source")]
    pub source: Option<AgentSource>,
    pub session_id: Option<String>,
    pub session_link: Option<String>,
    pub creator: Option<TaskCreatorInfo>,
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

impl AmbientAgentTask {
    /// Total credits used (inference + compute).
    pub fn credits_used(&self) -> Option<f32> {
        self.request_usage
            .as_ref()
            .map(|u| (u.inference_cost.unwrap_or(0.0) + u.compute_cost.unwrap_or(0.0)) as f32)
    }

    /// Duration from started_at to updated_at.
    pub fn run_time(&self) -> Option<chrono::Duration> {
        let started = self.started_at?;
        let duration = self.updated_at.signed_duration_since(started);
        (duration.num_seconds() >= 0).then_some(duration)
    }

    /// Creator's display name, if available.
    pub fn creator_display_name(&self) -> Option<String> {
        self.creator.as_ref().and_then(|c| c.display_name.clone())
    }

    /// Returns true if the underlying session for the ambient agent is no longer running.
    pub fn is_no_longer_running(&self) -> bool {
        !self.is_sandbox_running && !self.state.is_working()
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
pub struct TaskCreatorInfo {
    #[serde(rename = "type")]
    pub creator_type: String,
    pub uid: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct TaskStatusMessage {
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct RequestUsage {
    pub inference_cost: Option<f64>,
    pub compute_cost: Option<f64>,
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
