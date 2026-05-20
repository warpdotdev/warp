use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
            persistence::CloudModel,
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision,
    },
    server::{ids::SyncId, server_api::ai::AgentConfigSnapshot, sync_queue::QueueItem},
};
use warpui::{AppContext, SingletonEntity as _};

/// A CloudAgentConfig represents a saved agent configuration that can be referenced
/// when running agents via `--agent-id`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct AgentConfig {
    /// Configuration name
    pub name: String,
    /// Base model ID to use for the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model_id: Option<String>,
    /// Base prompt to prepend to user prompts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_prompt: Option<String>,
    /// MCP servers configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<HashMap<String, serde_json::Value>>,
}

pub type CloudAgentConfig = GenericCloudObject<GenericStringObjectId, CloudAgentConfigModel>;
pub type CloudAgentConfigModel = GenericStringModel<AgentConfig, JsonSerializer>;

impl AgentConfig {
    /// Convert to AgentConfigSnapshot for use in agent execution.
    ///
    /// Note: `AgentConfig` matches the server's JSON format (e.g. `base_model_id`),
    /// while `AgentConfigSnapshot` is the runtime config format (e.g. `model_id`).
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

impl CloudAgentConfig {
    pub fn get_all(app: &AppContext) -> Vec<CloudAgentConfig> {
        CloudModel::as_ref(app)
            .get_all_objects_of_type::<GenericStringObjectId, CloudAgentConfigModel>()
            .cloned()
            .collect()
    }

    pub fn get_by_id<'a>(sync_id: &'a SyncId, app: &'a AppContext) -> Option<&'a CloudAgentConfig> {
        CloudModel::as_ref(app)
            .get_object_of_type::<GenericStringObjectId, CloudAgentConfigModel>(sync_id)
    }
}

impl StringModel for AgentConfig {
    type CloudObjectType = CloudAgentConfig;

    fn model_type_name(&self) -> &'static str {
        "Cloud agent config"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::CloudAgentConfig)
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &CloudAgentConfig,
    ) -> QueueItem {
        QueueItem::UpdateCloudAgentConfig {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        None
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }
}

impl JsonModel for AgentConfig {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::CloudAgentConfig
    }
}
