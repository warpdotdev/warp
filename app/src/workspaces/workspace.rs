use crate::ai::execution_profiles::{
    ActionPermission, ComputerUsePermission, WriteToPtyPermission,
};
use crate::ai::llms::LLMModelHost;
use crate::settings::AgentModeCommandExecutionPredicate;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Hash, Debug, PartialEq, Eq)]
pub struct WorkspaceUid(String);

impl From<String> for WorkspaceUid {
    fn from(uid: String) -> Self {
        WorkspaceUid(uid)
    }
}

impl From<WorkspaceUid> for String {
    fn from(workspace_uid: WorkspaceUid) -> String {
        workspace_uid.0
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub uid: WorkspaceUid,
    pub name: String,
    pub settings: WorkspaceSettings,
}

impl Workspace {
    pub fn from_local_cache(uid: WorkspaceUid, name: String) -> Self {
        Self {
            uid,
            name,
            settings: Default::default(), // TODO: persistence wrapper instead of default
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum HostEnablementSetting {
    Enforce,
    #[default]
    RespectUserSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LlmHostSettings {
    pub enabled: bool,
    pub enablement_setting: HostEnablementSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LlmSettings {
    pub enabled: bool,
    #[serde(default)]
    pub host_configs: std::collections::HashMap<LLMModelHost, LlmHostSettings>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum UgcCollectionEnablementSetting {
    Disable,
    Enable,
    #[default]
    RespectUserSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UgcCollectionSettings {
    pub setting: UgcCollectionEnablementSetting,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum LocalPolicySetting {
    Disable,
    Enable,
    #[default]
    RespectUserSetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiPermissionsSettings {
    pub allow_ai_in_remote_sessions: bool,
    #[serde(with = "serde_regex")]
    pub remote_session_regex_list: Vec<Regex>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AiAutonomySettings {
    pub apply_code_diffs_setting: Option<ActionPermission>,
    pub read_files_setting: Option<ActionPermission>,
    pub read_files_allowlist: Option<Vec<PathBuf>>,
    pub execute_commands_setting: Option<ActionPermission>,
    pub execute_commands_allowlist: Option<Vec<AgentModeCommandExecutionPredicate>>,
    pub execute_commands_denylist: Option<Vec<AgentModeCommandExecutionPredicate>>,
    pub write_to_pty_setting: Option<WriteToPtyPermission>,
    pub computer_use_setting: Option<ComputerUsePermission>,
}

impl AiAutonomySettings {
    pub fn has_any_overrides(&self) -> bool {
        self.apply_code_diffs_setting.is_some()
            || self.read_files_setting.is_some()
            || self.read_files_allowlist.is_some()
            || self.execute_commands_setting.is_some()
            || self.execute_commands_allowlist.is_some()
            || self.execute_commands_denylist.is_some()
            || self.write_to_pty_setting.is_some()
            || self.computer_use_setting.is_some()
    }

    pub fn has_override_for_code_diffs(&self) -> bool {
        self.apply_code_diffs_setting.is_some()
    }

    pub fn has_override_for_read_files(&self) -> bool {
        self.read_files_setting.is_some()
    }

    pub fn has_override_for_read_files_allowlist(&self) -> bool {
        self.read_files_allowlist.is_some()
    }

    pub fn has_override_for_execute_commands(&self) -> bool {
        self.execute_commands_setting.is_some()
    }

    pub fn has_override_for_execute_commands_allowlist(&self) -> bool {
        self.execute_commands_allowlist.is_some()
    }

    pub fn has_override_for_execute_commands_denylist(&self) -> bool {
        self.execute_commands_denylist.is_some()
    }

    pub fn has_override_for_write_to_pty(&self) -> bool {
        self.write_to_pty_setting.is_some()
    }

    pub fn has_override_for_computer_use(&self) -> bool {
        self.computer_use_setting.is_some()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnterpriseSecretRegex {
    pub pattern: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SecretRedactionSettings {
    pub enabled: bool,
    pub regexes: Vec<EnterpriseSecretRegex>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CodebaseContextSettings {
    pub setting: LocalPolicySetting,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SandboxedAgentSettings {
    pub execute_commands_denylist: Option<Vec<AgentModeCommandExecutionPredicate>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    pub llm_settings: LlmSettings,
    pub ugc_collection_settings: UgcCollectionSettings,
    pub secret_redaction_settings: SecretRedactionSettings,
    pub ai_permissions_settings: AiPermissionsSettings,
    pub ai_autonomy_settings: AiAutonomySettings,
    pub codebase_context_settings: CodebaseContextSettings,
    pub sandboxed_agent_settings: Option<SandboxedAgentSettings>,
    /// Local agent attribution policy. When `Enable` or `Disable`, the user
    /// toggle is locked. When `RespectUserSetting`, the user can choose.
    #[serde(alias = "enable_warp_attribution")]
    #[serde(default)]
    pub agent_attribution_policy: LocalPolicySetting,
}
