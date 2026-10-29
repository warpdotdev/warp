use std::collections::HashSet;

use warp_multi_agent_api as api;

use super::action::{RunAgentsAgentRunConfig, RunAgentsRequest};

/// Client-side representation of the orchestration config attached to a
/// conversation via `OrchestrationConfigSnapshot`.
///
/// Mirrors the proto `OrchestrationConfig` but uses Rust-native types
/// to keep view / model code free of proto imports.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrchestrationConfig {
    pub model_id: String,
    pub harness_type: String,
    pub execution_mode: OrchestrationExecutionMode,
    /// Complete batch roster, including children inheriting the batch model.
    /// Empty denotes a legacy config without per-child approval.
    pub agent_configs: Vec<OrchestrationAgentConfig>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrchestrationAgentConfig {
    /// Batch-local identity; compared after trimming, ignoring ASCII case.
    pub name: String,
    /// Empty inherits the batch model, or the adapter default when that is also empty.
    pub model_id: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum OrchestrationExecutionMode {
    Local,
    Remote {
        environment_id: String,
        worker_host: String,
        /// Runner UID pre-declared on the config; empty means unset.
        runner_id: String,
    },
}

impl OrchestrationExecutionMode {
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }
}

/// User's approval state for orchestration on the active config.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum OrchestrationConfigStatus {
    /// No `OrchestrationConfigSnapshot` has been seen yet.
    #[default]
    None,
    Approved,
    Disapproved,
}

impl OrchestrationConfigStatus {
    pub fn is_approved(&self) -> bool {
        matches!(self, Self::Approved)
    }

    pub fn is_disapproved(&self) -> bool {
        matches!(self, Self::Disapproved)
    }
}

// ---------------------------------------------------------------------------
// Match check — determines whether a `run_agents` call auto-launches.
// ---------------------------------------------------------------------------

/// Returns `true` when the `run_agents` call's run-wide fields match
/// the active `OrchestrationConfig`.
///
/// Empty/unset fields on the call are treated as inheriting from the
/// config (and therefore matching). Fields not in the config
/// (`computer_use_enabled`, `skills`, `base_prompt`, per-agent `title`) are excluded.
/// This check alone does not authorize auto-launch: roster identity and model
/// availability must also be validated before reusing a per-child approval.
pub fn matches_active_config(request: &RunAgentsRequest, config: &OrchestrationConfig) -> bool {
    // model_id — empty on the call means "inherit from config" → matches.
    if !request.model_id.is_empty() && request.model_id != config.model_id {
        return false;
    }

    // harness_type
    if !request.harness_type.is_empty() && request.harness_type != config.harness_type {
        return false;
    }

    // execution_mode variant must agree.
    match (&request.execution_mode, &config.execution_mode) {
        (super::action::RunAgentsExecutionMode::Local, OrchestrationExecutionMode::Local) => true,
        (
            super::action::RunAgentsExecutionMode::Remote {
                environment_id,
                worker_host,
                runner_id,
                ..
            },
            OrchestrationExecutionMode::Remote {
                environment_id: cfg_env,
                worker_host: cfg_host,
                runner_id: cfg_runner,
            },
        ) => {
            let env_matches = environment_id.is_empty() || environment_id == cfg_env;
            let host_matches = worker_host.is_empty() || worker_host == cfg_host;
            // Empty runner_id on the call inherits from the config → matches.
            let runner_matches = runner_id.is_empty() || runner_id == cfg_runner;
            env_matches && host_matches && runner_matches
        }
        // Variant mismatch (Local vs Remote).
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Proto ↔ native conversions
// ---------------------------------------------------------------------------

impl OrchestrationConfig {
    /// Checks complete roster identity, not model equality. Order is irrelevant; duplicate,
    /// empty, added, removed, or renamed identities prevent approval reuse.
    pub fn has_matching_agent_roster(&self, agents: &[RunAgentsAgentRunConfig]) -> bool {
        if self.agent_configs.is_empty() || self.agent_configs.len() != agents.len() {
            return false;
        }
        let mut names = HashSet::new();
        agents.iter().all(|agent| {
            let name = agent.name.trim().to_ascii_lowercase();
            !name.is_empty()
                && names.insert(name.clone())
                && self
                    .agent_configs
                    .iter()
                    .filter(|approved| approved.name.trim().eq_ignore_ascii_case(&name))
                    .count()
                    == 1
        })
    }

    /// Substitutes configured models by normalized name. Approval reuse additionally requires
    /// complete roster identity and model validation for the selected harness and location.
    pub fn resolve_agent_configs(&self, agents: &mut [RunAgentsAgentRunConfig]) {
        for agent in agents {
            if let Some(approved) = self
                .agent_configs
                .iter()
                .find(|approved| approved.name.trim().eq_ignore_ascii_case(agent.name.trim()))
            {
                agent.model_id = approved.model_id.clone();
            }
        }
    }

    /// Converts from the proto `OrchestrationConfig` message.
    pub fn from_proto(proto: &api::OrchestrationConfig) -> Self {
        let execution_mode = match &proto.execution_mode {
            Some(api::orchestration_config::ExecutionMode::Remote(remote)) => {
                OrchestrationExecutionMode::Remote {
                    environment_id: remote.environment_id.clone(),
                    worker_host: remote.worker_host.clone(),
                    runner_id: remote.runner_id.clone(),
                }
            }
            Some(api::orchestration_config::ExecutionMode::Local(_)) | None => {
                OrchestrationExecutionMode::Local
            }
        };

        let harness_type = harness_proto_to_string(proto.harness.as_ref()).unwrap_or_default();

        Self {
            model_id: proto.model_id.clone(),
            harness_type,
            execution_mode,
            agent_configs: proto
                .agent_configs
                .iter()
                .map(|agent| OrchestrationAgentConfig {
                    name: agent.name.clone(),
                    model_id: agent.model_id.clone(),
                })
                .collect(),
        }
    }

    /// Converts to the proto `OrchestrationConfig` message.
    pub fn to_proto(&self) -> api::OrchestrationConfig {
        let execution_mode = match &self.execution_mode {
            OrchestrationExecutionMode::Local => {
                Some(api::orchestration_config::ExecutionMode::Local(
                    api::orchestration_config::Local {},
                ))
            }
            OrchestrationExecutionMode::Remote {
                environment_id,
                worker_host,
                runner_id,
            } => Some(api::orchestration_config::ExecutionMode::Remote(
                api::orchestration_config::Remote {
                    environment_id: environment_id.clone(),
                    worker_host: worker_host.clone(),
                    runner_id: runner_id.clone(),
                },
            )),
        };

        api::OrchestrationConfig {
            model_id: self.model_id.clone(),
            harness: harness_type_to_proto(&self.harness_type),
            execution_mode,
            agent_configs: self
                .agent_configs
                .iter()
                .map(|agent| api::orchestration_config::AgentConfig {
                    name: agent.name.clone(),
                    model_id: agent.model_id.clone(),
                })
                .collect(),
        }
    }
}

impl OrchestrationConfigStatus {
    /// Converts from the proto `OrchestrationStatus` message.
    pub fn from_proto(proto: Option<&api::OrchestrationStatus>) -> Self {
        let Some(status) = proto else {
            return Self::None;
        };
        match &status.status {
            Some(api::orchestration_status::Status::Approved(_)) => Self::Approved,
            Some(api::orchestration_status::Status::Disapproved(_)) => Self::Disapproved,
            None => Self::None,
        }
    }

    /// Converts to the proto `OrchestrationStatus` message.
    pub fn to_proto(&self) -> Option<api::OrchestrationStatus> {
        match self {
            Self::None => None,
            Self::Approved => Some(api::OrchestrationStatus {
                status: Some(api::orchestration_status::Status::Approved(
                    api::orchestration_status::Approved {},
                )),
            }),
            Self::Disapproved => Some(api::OrchestrationStatus {
                status: Some(api::orchestration_status::Status::Disapproved(
                    api::orchestration_status::Disapproved {},
                )),
            }),
        }
    }
}

/// Maps the proto `Harness` oneof to a client-side string identifier.
/// Returns `None` for an unset variant.
fn harness_proto_to_string(harness: Option<&api::Harness>) -> Option<String> {
    let variant = harness?.variant.as_ref()?;
    Some(
        match variant {
            api::harness::Variant::Oz(_) => "oz",
            api::harness::Variant::ClaudeCode(_) => "claude",
            api::harness::Variant::OpenCode(_) => "opencode",
            api::harness::Variant::Gemini(_) => "gemini",
            api::harness::Variant::Codex(_) => "codex",
            api::harness::Variant::Pi(_) => "pi",
        }
        .to_string(),
    )
}

/// Converts a client-side harness string identifier to the proto `Harness`
/// oneof variant. Returns `None` for empty or unknown strings.
fn harness_type_to_proto(harness_type: &str) -> Option<api::Harness> {
    let variant = match harness_type {
        "oz" => api::harness::Variant::Oz(api::harness::Oz {}),
        "claude" => api::harness::Variant::ClaudeCode(api::harness::ClaudeCode {}),
        "opencode" => api::harness::Variant::OpenCode(api::harness::OpenCode {}),
        "gemini" => api::harness::Variant::Gemini(api::harness::Gemini {}),
        "codex" => api::harness::Variant::Codex(api::harness::Codex {}),
        "pi" => api::harness::Variant::Pi(api::harness::Pi {}),
        _ => return None,
    };
    Some(api::Harness {
        variant: Some(variant),
    })
}

#[cfg(test)]
#[path = "orchestration_config_tests.rs"]
mod tests;
