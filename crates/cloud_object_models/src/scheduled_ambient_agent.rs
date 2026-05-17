use std::collections::HashMap;

use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use warp_cli::agent::Harness;

use crate::{JsonModel, JsonSerializer};

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
    /// The model to use with this harness. None means use the harness default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Optional reasoning level for harnesses that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessModelConfig {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
}

impl HarnessConfig {
    /// Builds a harness config from just the harness type.
    pub fn from_harness_type(harness_type: Harness) -> Self {
        Self {
            harness_type,
            model_id: None,
            reasoning_level: None,
        }
    }

    pub fn model_config(&self) -> Option<HarnessModelConfig> {
        self.model_id
            .as_ref()
            .filter(|id| !id.is_empty())
            .map(|model_id| HarnessModelConfig {
                model_id: model_id.clone(),
                reasoning_level: self.reasoning_level.clone(),
            })
    }
}

fn serialize_harness<S: Serializer>(harness: &Harness, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(harness.config_name())
}

fn deserialize_harness<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Harness, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(Harness::from_config_name(&name).unwrap_or_else(|| {
        log::warn!("Unknown harness config name: {name:?}; treating as Unknown");
        Harness::Unknown
    }))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HarnessAuthSecretsConfig {
    /// Name of a managed secret for Claude Code harness authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_auth_secret_name: Option<String>,
    /// Name of a managed secret for Codex harness authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_auth_secret_name: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScheduledAmbientAgent {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cron_schedule: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_spawn_error: Option<String>,
    #[serde(default, skip_serializing_if = "AgentConfigSnapshot::is_empty")]
    pub agent_config: AgentConfigSnapshot,
}

impl ScheduledAmbientAgent {
    pub fn new(name: String, cron_schedule: String, enabled: bool, prompt: String) -> Self {
        Self {
            name,
            cron_schedule,
            enabled,
            prompt,
            last_spawn_error: None,
            agent_config: Default::default(),
        }
    }
}

impl JsonModel for ScheduledAmbientAgent {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::ScheduledAmbientAgent
    }
}

pub type CloudScheduledAmbientAgent =
    GenericCloudObject<GenericStringObjectId, CloudScheduledAmbientAgentModel>;
pub type CloudScheduledAmbientAgentModel =
    GenericStringModel<ScheduledAmbientAgent, JsonSerializer>;
pub type ServerScheduledAmbientAgent =
    GenericServerObject<GenericStringObjectId, CloudScheduledAmbientAgentModel>;

pub type AgentConfigMap = HashMap<String, serde_json::Value>;
