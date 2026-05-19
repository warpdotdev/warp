use std::collections::HashMap;

use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use serde::{Deserialize, Serialize};

use crate::{AgentConfigSnapshot, JsonModel, JsonSerializer};

/// A cloud agent config represents a saved agent configuration that can be referenced when running agents via `--agent-id`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct AgentConfig {
    /// Configuration name.
    pub name: String,
    /// Base model ID to use for the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model_id: Option<String>,
    /// Base prompt to prepend to user prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_prompt: Option<String>,
    /// MCP servers configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<HashMap<String, serde_json::Value>>,
}

impl AgentConfig {
    /// Converts to `AgentConfigSnapshot` for use in agent execution.
    ///
    /// `AgentConfig` matches the server's JSON format, such as `base_model_id`, while `AgentConfigSnapshot` is the runtime config format, such as `model_id`.
    pub fn to_ambient_config(&self) -> AgentConfigSnapshot {
        AgentConfigSnapshot {
            name: Some(self.name.clone()),
            environment_id: None,
            model_id: self.base_model_id.clone(),
            base_prompt: self.base_prompt.clone(),
            mcp_servers: self.mcp_servers.clone().map(|m| m.into_iter().collect()),
            profile_id: None,
            worker_host: None,
            skill_spec: None,
            computer_use_enabled: None,
            harness: None,
            harness_auth_secrets: None,
        }
    }
}

impl JsonModel for AgentConfig {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::CloudAgentConfig
    }
}

pub type CloudAgentConfig = GenericCloudObject<GenericStringObjectId, CloudAgentConfigModel>;
pub type CloudAgentConfigModel = GenericStringModel<AgentConfig, JsonSerializer>;
pub type ServerCloudAgentConfig = GenericServerObject<GenericStringObjectId, CloudAgentConfigModel>;
