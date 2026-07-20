//! Prepares orchestrated remote-child launches and classifies their startup errors.
//!
//! This module owns frontend-neutral request construction and startup issue semantics;
//! frontend-specific callers remain responsible for lifecycle state and presentation.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use prost::Message as _;
use warp_cli::agent::Harness;
use warp_multi_agent_api as multi_agent_api;
use warpui::{AppContext, SingletonEntity as _};

use crate::ChannelState;
use crate::ai::agent::UserQueryMode;
use crate::ai::ambient_agents::task::{
    HarnessAuthSecretsConfig, HarnessConfig, normalize_orchestrator_agent_name,
};
use crate::ai::ambient_agents::{
    OUT_OF_CREDITS_TASK_FAILURE_MESSAGE, SERVER_OVERLOADED_TASK_FAILURE_MESSAGE, github_auth_url,
};
use crate::ai::blocklist::StartAgentRequest;
use crate::ai::skills::{SkillManager, SkillReference};
use crate::server::server_api::ai::{AgentConfigSnapshot, SpawnAgentRequest};
use crate::server::server_api::{AIApiError, ClientError, CloudAgentCapacityError};
use crate::settings::PrivacySettings;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::AdminEnablementSetting;

/// Remote execution fields carried by [`crate::ai::agent::StartAgentExecutionMode`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteChildLaunchConfig {
    pub environment_id: String,
    pub skill_references: Vec<SkillReference>,
    pub model_id: String,
    pub computer_use_enabled: bool,
    pub worker_host: String,
    pub harness_type: String,
    pub title: String,
    pub auth_secret_name: Option<String>,
    pub runner_id: String,
    pub agent_identity_uid: Option<String>,
}

impl RemoteChildLaunchConfig {
    pub fn orchestration_harness(&self) -> Harness {
        if self.harness_type.trim().is_empty() {
            Harness::Oz
        } else {
            Harness::parse_orchestration_harness(&self.harness_type).unwrap_or(Harness::Unknown)
        }
    }
}

/// Frontend-neutral output used to launch one remote child.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
#[derive(Clone, Debug)]
pub struct PreparedRemoteChildLaunch {
    pub display_name: String,
    pub orchestration_harness: Harness,
    pub spawn_request: SpawnAgentRequest,
}

/// Failure while constructing the remote child request, before calling the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareRemoteChildLaunchError {
    MissingParentRunId,
    UnresolvedSkills { references: Vec<String> },
}

impl PrepareRemoteChildLaunchError {
    pub fn user_message(&self) -> String {
        match self {
            Self::MissingParentRunId => {
                "Remote child agents require the parent run_id to be available.".to_string()
            }
            Self::UnresolvedSkills { references } => {
                format!(
                    "Failed to resolve child agent skills: {}",
                    references.join(", ")
                )
            }
        }
    }
}

/// A recoverable startup condition that requires user action.
///
/// The GUI represents this as `ambient_agent::Status::NeedsGithubAuth`.
/// Orchestrated children retain their surface so the user can follow the
/// remediation link, but the original child launch still resolves as failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupBlocker {
    GitHubAuthRequired { message: String, auth_url: String },
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
impl CloudAgentStartupBlocker {
    pub fn message(&self) -> &str {
        match self {
            Self::GitHubAuthRequired { message, .. } => message,
        }
    }

    pub fn primary_url(&self) -> &str {
        match self {
            Self::GitHubAuthRequired { auth_url, .. } => auth_url,
        }
    }
}

/// A terminal cloud-agent startup failure.
///
/// The GUI represents these as `ambient_agent::Status::Failed`. Unlike a
/// blocker, a failure has no remediation action that requires retaining an
/// optimistic orchestrated-child surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupFailure {
    Capacity { message: String },
    OutOfCredits { message: String },
    ServerOverloaded { message: String },
    Other { message: String },
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
impl CloudAgentStartupFailure {
    pub fn message(&self) -> &str {
        match self {
            Self::Capacity { message }
            | Self::OutOfCredits { message }
            | Self::ServerOverloaded { message }
            | Self::Other { message } => message,
        }
    }
}

/// Shared interpretation of an error returned while starting a cloud agent.
///
/// This distinction preserves the existing orchestrated-child contract:
/// blockers remain visible for user action, while terminal failures are
/// eligible for failed-launch cleanup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupIssue {
    Blocked(CloudAgentStartupBlocker),
    Failed(CloudAgentStartupFailure),
}

/// Builds the public API request for one remote child without owning frontend lifecycle state.
pub fn prepare_remote_child_launch(
    request: &StartAgentRequest,
    config: RemoteChildLaunchConfig,
    ctx: &AppContext,
) -> Result<PreparedRemoteChildLaunch, PrepareRemoteChildLaunchError> {
    let orchestration_harness = config.orchestration_harness();
    let RemoteChildLaunchConfig {
        environment_id,
        skill_references,
        model_id,
        computer_use_enabled,
        worker_host,
        harness_type,
        title,
        auth_secret_name,
        runner_id,
        agent_identity_uid,
    } = config;
    let Some(parent_run_id) = request.parent_run_id.clone() else {
        return Err(PrepareRemoteChildLaunchError::MissingParentRunId);
    };
    let runtime_skills = resolve_runtime_skills(&skill_references, ctx)?;
    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let display_name = agent_name.clone().unwrap_or_default();
    let environment_id = Some(environment_id).filter(|id| !id.trim().is_empty());
    let harness_override = if harness_type.is_empty() {
        None
    } else {
        match <Harness as clap::ValueEnum>::from_str(&harness_type, true) {
            Ok(harness) => Some(HarnessConfig::from_harness_type(harness)),
            Err(_) => {
                log::warn!(
                    "Unknown harness type from StartAgentV2 proto: {harness_type:?}; omitting harness override so the server picks its default"
                );
                None
            }
        }
    };
    let computer_use_enabled =
        (orchestration_harness == Harness::Oz).then_some(computer_use_enabled);
    let harness_auth_secrets = auth_secret_name
        .filter(|name| !name.trim().is_empty())
        .and_then(|name| match orchestration_harness {
            Harness::Claude => Some(HarnessAuthSecretsConfig {
                claude_auth_secret_name: Some(name),
                codex_auth_secret_name: None,
            }),
            Harness::Codex => Some(HarnessAuthSecretsConfig {
                claude_auth_secret_name: None,
                codex_auth_secret_name: Some(name),
            }),
            Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => None,
        });
    let spawn_request = SpawnAgentRequest {
        prompt: Some(request.prompt.clone()),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            name: agent_name,
            environment_id,
            runner_id: (!runner_id.is_empty()).then_some(runner_id),
            model_id: (!model_id.is_empty()).then_some(model_id),
            worker_host: (!worker_host.is_empty()).then_some(worker_host),
            computer_use_enabled,
            harness: harness_override,
            harness_auth_secrets,
            ..Default::default()
        }),
        title: (!title.is_empty()).then_some(title),
        team: None,
        skill: None,
        attachments: Vec::new(),
        interactive: Some(true),
        parent_run_id: Some(parent_run_id),
        runtime_skills,
        referenced_attachments: Vec::new(),
        conversation_id: None,
        initial_snapshot_token: None,
        agent_identity_uid: agent_identity_uid.filter(|uid| !uid.trim().is_empty()),
        snapshot_disabled: should_disable_snapshot(ctx).then_some(true),
        orchestration_handoff: None,
    };
    Ok(PreparedRemoteChildLaunch {
        display_name,
        orchestration_harness,
        spawn_request,
    })
}

/// Maps server/client launch failures into shared startup presentation.
pub fn classify_cloud_agent_startup_error(error: &anyhow::Error) -> CloudAgentStartupIssue {
    if let Some(client_error) = error.downcast_ref::<ClientError>()
        && let Some(auth_url) = &client_error.auth_url
    {
        return CloudAgentStartupIssue::Blocked(CloudAgentStartupBlocker::GitHubAuthRequired {
            message: client_error.error.clone(),
            auth_url: github_auth_url::cloud_setup_auth_url_with_next(auth_url),
        });
    }
    if let Some(capacity_error) = error.downcast_ref::<CloudAgentCapacityError>() {
        return CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Capacity {
            message: capacity_error.error.clone(),
        });
    }
    if let Some(ai_api_error) = error.downcast_ref::<AIApiError>() {
        match ai_api_error {
            AIApiError::QuotaLimit {
                user_display_message,
            } => {
                return CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::OutOfCredits {
                    message: user_display_message
                        .clone()
                        .unwrap_or_else(|| OUT_OF_CREDITS_TASK_FAILURE_MESSAGE.to_string()),
                });
            }
            AIApiError::ServerOverloaded => {
                return CloudAgentStartupIssue::Failed(
                    CloudAgentStartupFailure::ServerOverloaded {
                        message: SERVER_OVERLOADED_TASK_FAILURE_MESSAGE.to_string(),
                    },
                );
            }
            AIApiError::Transport(_)
            | AIApiError::Deserialization(_)
            | AIApiError::NoContextFound
            | AIApiError::ErrorStatus(_, _)
            | AIApiError::Other(_)
            | AIApiError::Stream { .. }
            | AIApiError::UnexpectedEof
            | AIApiError::GrokSubscriptionTokenRefreshFailed => {}
        }
    }
    CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Other {
        message: error.to_string(),
    })
}

pub(crate) fn should_disable_snapshot(ctx: &AppContext) -> bool {
    let privacy = PrivacySettings::as_ref(ctx);
    if !privacy.is_cloud_conversation_storage_enabled {
        return true;
    }
    matches!(
        UserWorkspaces::as_ref(ctx).get_cloud_conversation_storage_enablement_setting(),
        AdminEnablementSetting::Disable
    )
}

/// Builds the Oz web URL for a server-assigned agent run ID.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn oz_run_url(run_id: &str) -> String {
    format!("{}/runs/{run_id}", ChannelState::oz_root_url())
}

fn resolve_runtime_skills(
    skill_references: &[SkillReference],
    ctx: &AppContext,
) -> Result<Vec<String>, PrepareRemoteChildLaunchError> {
    let skill_manager = SkillManager::as_ref(ctx);
    let mut runtime_skills = Vec::with_capacity(skill_references.len());
    let mut unresolved_references = Vec::new();
    for reference in skill_references {
        let Some(skill) = skill_manager.active_skill_by_reference(reference, ctx) else {
            unresolved_references.push(reference.to_string());
            continue;
        };
        runtime_skills.push(
            BASE64_STANDARD.encode(multi_agent_api::Skill::from(skill.clone()).encode_to_vec()),
        );
    }
    if unresolved_references.is_empty() {
        Ok(runtime_skills)
    } else {
        Err(PrepareRemoteChildLaunchError::UnresolvedSkills {
            references: unresolved_references,
        })
    }
}

#[cfg(test)]
#[path = "remote_child_tests.rs"]
mod tests;
