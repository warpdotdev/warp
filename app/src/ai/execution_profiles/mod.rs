use std::path::PathBuf;

use crate::cloud_object::UniquePer;
use crate::server::sync_queue::QueueItem;
use crate::settings::AISettings;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    cloud_object::{
        model::{
            generic_string_model::{GenericStringModel, GenericStringObjectId, StringModel},
            json_model::{JsonModel, JsonSerializer},
        },
        GenericCloudObject, GenericStringObjectFormat, GenericStringObjectUniqueKey,
        JsonObjectType, Revision, ServerCloudObject,
    },
    settings::{
        AgentModeCommandExecutionPredicate, DEFAULT_COMMAND_EXECUTION_ALLOWLIST,
        DEFAULT_COMMAND_EXECUTION_DENYLIST,
    },
};
use serde::{Deserialize, Serialize};
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, SingletonEntity};

use super::llms::{LLMContextWindow, LLMId, LLMPreferences};

pub const PROFILE_NAME_MAX_LENGTH: usize = 50;

pub mod editor;
pub mod model_menu_items;
pub mod profiles;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionPermission {
    AgentDecides,
    AlwaysAllow,
    AlwaysAsk,

    // This is intended to catch deserialization errors whenever we add new variants to this enum. Say we
    // want to add a "Never" variant. Without this catch-all, old clients wouldn't be able to deserialize
    // a "Never" into one of the existing options.
    #[serde(other)]
    Unknown,
}

impl ActionPermission {
    pub fn description(&self) -> &'static str {
        match self {
            ActionPermission::AgentDecides | ActionPermission::Unknown => "The Agent chooses the safest path: acting on its own when confident, and asking for approval when uncertain.",
            ActionPermission::AlwaysAllow => "Give the Agent full autonomy  — no manual approval ever required.",
            ActionPermission::AlwaysAsk => "Require explicit approval before the Agent takes any action.",
        }
    }

    pub fn is_always_ask(&self) -> bool {
        matches!(self, Self::AlwaysAsk)
    }

    pub fn is_always_allow(&self) -> bool {
        matches!(self, Self::AlwaysAllow)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteToPtyPermission {
    // This is for backwards compatibility with the old "Never" value.
    #[serde(alias = "Never")]
    AlwaysAllow,
    #[default]
    AlwaysAsk,
    AskOnFirstWrite,

    // This is intended to catch deserialization errors whenever we add new variants to this enum.
    #[serde(other)]
    Unknown,
}

impl WriteToPtyPermission {
    pub fn description(&self) -> &'static str {
        match self {
            WriteToPtyPermission::AlwaysAllow => ActionPermission::AlwaysAllow.description(),
            WriteToPtyPermission::AskOnFirstWrite => {
                "The agent will ask for permission the first time it needs to interact with a running command. After that, it will continue automatically for the rest of that command."
            }
            WriteToPtyPermission::AlwaysAsk => "The agent will always ask for permission to interact with a running command.",
            WriteToPtyPermission::Unknown => ActionPermission::Unknown.description(),
        }
    }

    pub fn is_always_allow(&self) -> bool {
        matches!(self, Self::AlwaysAllow)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerUsePermission {
    #[default]
    Never,
    AlwaysAsk,
    AlwaysAllow,

    // This is intended to catch deserialization errors whenever we add new variants to this enum.
    #[serde(other)]
    Unknown,
}

/// Result of resolving the cloud agent computer use setting.
/// Contains both the effective value and whether it's forced by organization policy.
pub struct CloudAgentComputerUseState {
    /// Whether computer use is enabled for cloud agents.
    pub enabled: bool,
    /// Whether this value is forced by organization settings (true = user cannot change it).
    pub is_forced_by_org: bool,
}

impl ComputerUsePermission {
    pub fn description(&self) -> &'static str {
        match self {
            ComputerUsePermission::Never => {
                "Computer use tools are disabled and will not be available to the Agent."
            }
            ComputerUsePermission::AlwaysAsk => {
                "Require explicit approval before the Agent uses computer use tools."
            }
            ComputerUsePermission::AlwaysAllow => {
                "Give the Agent full autonomy to use computer use tools without approval."
            }
            ComputerUsePermission::Unknown => "Unknown setting.",
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Never | Self::Unknown)
    }

    pub fn is_always_allow(&self) -> bool {
        matches!(self, Self::AlwaysAllow)
    }

    /// Resolves the effective cloud agent computer use state by reading the workspace
    /// autonomy setting and user's local preference from their respective singletons.
    pub fn resolve_cloud_agent_state(ctx: &AppContext) -> CloudAgentComputerUseState {
        if !FeatureFlag::AgentModeComputerUse.is_enabled() {
            return CloudAgentComputerUseState {
                enabled: false,
                is_forced_by_org: false,
            };
        }

        let autonomy_setting = UserWorkspaces::as_ref(ctx)
            .ai_autonomy_settings()
            .computer_use_setting;
        let user_preference = *AISettings::as_ref(ctx).cloud_agent_computer_use_enabled;

        match autonomy_setting {
            Some(ComputerUsePermission::Never) => CloudAgentComputerUseState {
                enabled: false,
                is_forced_by_org: true,
            },
            Some(ComputerUsePermission::AlwaysAllow) => CloudAgentComputerUseState {
                enabled: true,
                is_forced_by_org: true,
            },
            // TODO(QUALITY-297): Currently this case should never be hit because the
            // AlwaysAsk variant isn't accessible in the admin console. We need to figure
            // out how to handle it when it eventually becomes available. For now, I'm
            // treating this conservatively and marking computer use as disabled.
            Some(ComputerUsePermission::AlwaysAsk) => CloudAgentComputerUseState {
                enabled: false,
                is_forced_by_org: true,
            },
            Some(ComputerUsePermission::Unknown) | None => CloudAgentComputerUseState {
                enabled: user_preference,
                is_forced_by_org: false,
            },
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskUserQuestionPermission {
    /// Never pause; skip questions and continue with best judgment.
    Never,
    /// Pause and wait for the user, unless auto-approve mode is enabled.
    AskExceptInAutoApprove,
    /// Always pause and wait for the user to answer before continuing, even in auto-approve mode.
    #[default]
    AlwaysAsk,

    // This is intended to catch deserialization errors whenever we add new variants to this enum.
    #[serde(other)]
    Unknown,
}

impl AskUserQuestionPermission {
    pub fn label(&self) -> &'static str {
        match self {
            AskUserQuestionPermission::Never => "Never ask",
            AskUserQuestionPermission::AskExceptInAutoApprove => "Ask unless auto-approve",
            AskUserQuestionPermission::AlwaysAsk | AskUserQuestionPermission::Unknown => {
                "Always ask"
            }
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            AskUserQuestionPermission::AskExceptInAutoApprove
            | AskUserQuestionPermission::Unknown => {
                "The Agent may ask a question and pause for your response, but will continue automatically when auto-approve is on."
            }
            AskUserQuestionPermission::Never => {
                "The Agent will not ask questions and will continue with its best judgment."
            }
            AskUserQuestionPermission::AlwaysAsk => {
                "The Agent may ask a question and will pause for your response even when auto-approve is on."
            }
        }
    }
}

/// Core data structure representing an AI execution profile, which includes model configuration,
/// behavior settings, and permissions.
///
/// NOTE: `planning_model` was removed after planning via subagent was deprecated; serialized legacy
/// profiles may include a `planning_model` field and this field name should remain reserved
/// indefinitely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AIExecutionProfile {
    pub name: String,
    pub is_default_profile: bool,
    pub apply_code_diffs: ActionPermission,
    pub read_files: ActionPermission,

    pub execute_commands: ActionPermission,
    pub write_to_pty: WriteToPtyPermission,
    pub mcp_permissions: ActionPermission,
    pub ask_user_question: AskUserQuestionPermission,

    /// Always ask for permission for these commands
    pub command_denylist: Vec<AgentModeCommandExecutionPredicate>,

    /// When the execute_commands is set to AlwaysAsk, autoexecute these commands
    pub command_allowlist: Vec<AgentModeCommandExecutionPredicate>,

    /// When the read_files is set to AlwaysAsk, autoread from these directories
    pub directory_allowlist: Vec<PathBuf>,

    pub mcp_allowlist: Vec<uuid::Uuid>,
    pub mcp_denylist: Vec<uuid::Uuid>,

    pub computer_use: ComputerUsePermission,

    pub base_model: Option<LLMId>,
    pub coding_model: Option<LLMId>,
    pub cli_agent_model: Option<LLMId>,
    pub computer_use_model: Option<LLMId>,

    pub context_window_limit: Option<u32>,

    /// Whether plans created by the agent should be automatically synced to Warp Drive
    pub autosync_plans_to_warp_drive: bool,

    /// Whether the agent may use web search when helpful for completing tasks
    pub web_search_enabled: bool,
}

impl Default for AIExecutionProfile {
    fn default() -> Self {
        Self {
            name: Default::default(),
            is_default_profile: false,
            apply_code_diffs: ActionPermission::AgentDecides,
            read_files: ActionPermission::AgentDecides,
            execute_commands: ActionPermission::AlwaysAsk,
            write_to_pty: WriteToPtyPermission::AlwaysAsk,
            mcp_permissions: ActionPermission::AgentDecides,
            ask_user_question: AskUserQuestionPermission::AlwaysAsk,
            command_denylist: DEFAULT_COMMAND_EXECUTION_DENYLIST.clone(),
            command_allowlist: Vec::new(),
            directory_allowlist: Vec::new(),
            mcp_allowlist: Vec::new(),
            mcp_denylist: Vec::new(),
            computer_use: ComputerUsePermission::Never,
            base_model: None,
            coding_model: None,
            cli_agent_model: None,
            computer_use_model: None,
            context_window_limit: None,
            autosync_plans_to_warp_drive: true,
            web_search_enabled: true,
        }
    }
}

impl AIExecutionProfile {
    pub fn create_default_from_legacy_settings(app: &AppContext) -> Self {
        // Note that the legacy "Autonomy" and "Code Access" settings are not imported here.
        // The "Code Access" setting defaulted to "Always Ask", which is the most restrictive, so
        // it's impossible for us to infer some hesitancy about autonomy from the setting and we should
        // ignore it. The same applies to "Autonomy".
        let ai_settings = AISettings::as_ref(app);
        Self {
            name: "Default".to_string(),
            is_default_profile: true,
            command_denylist: ai_settings.agent_mode_command_execution_denylist.clone(),
            // We initialize the command allowlist to be anything the user added, excluding all
            // the pre-populated defaults.
            command_allowlist: ai_settings
                .agent_mode_command_execution_allowlist
                .iter()
                .filter(|cmd| !DEFAULT_COMMAND_EXECUTION_ALLOWLIST.contains(cmd))
                .cloned()
                .collect(),
            directory_allowlist: ai_settings.agent_mode_coding_file_read_allowlist.clone(),
            ..Default::default()
        }
    }

    #[cfg(feature = "agent_mode_evals")]
    pub fn create_agent_mode_eval_profile() -> Self {
        Self {
            name: "Agent Mode Eval".to_string(),
            is_default_profile: false,
            apply_code_diffs: ActionPermission::AlwaysAllow,
            read_files: ActionPermission::AlwaysAllow,
            execute_commands: ActionPermission::AlwaysAllow,
            write_to_pty: WriteToPtyPermission::AlwaysAllow,
            mcp_permissions: ActionPermission::AlwaysAllow,
            ask_user_question: AskUserQuestionPermission::Never,
            command_denylist: Vec::new(),
            command_allowlist: Vec::new(),
            directory_allowlist: Vec::new(),
            mcp_allowlist: Vec::new(),
            mcp_denylist: Vec::new(),
            computer_use: ComputerUsePermission::Never,
            base_model: None,
            coding_model: None,
            cli_agent_model: None,
            computer_use_model: None,
            context_window_limit: None,
            autosync_plans_to_warp_drive: false,
            web_search_enabled: true,
        }
    }

    /// This creates a CLI-specific profile that will never ask the user for permission,
    /// since we cannot do so in a non-interactive setting.
    pub fn create_default_cli_profile(
        is_sandboxed: bool,
        computer_use_override: Option<bool>,
    ) -> Self {
        let command_denylist = if is_sandboxed {
            Vec::new()
        } else {
            DEFAULT_COMMAND_EXECUTION_DENYLIST.to_vec()
        };

        let computer_use_permission = match computer_use_override {
            Some(true) => {
                if is_sandboxed || FeatureFlag::LocalComputerUse.is_enabled() {
                    ComputerUsePermission::AlwaysAllow
                } else {
                    ComputerUsePermission::Never
                }
            }
            Some(false) => ComputerUsePermission::Never,
            None => {
                if is_sandboxed && ChannelState::channel().is_dogfood() {
                    ComputerUsePermission::AlwaysAllow
                } else {
                    ComputerUsePermission::Never
                }
            }
        };

        Self {
            name: "Default (CLI)".to_owned(),
            is_default_profile: true,
            apply_code_diffs: ActionPermission::AlwaysAllow,
            read_files: ActionPermission::AlwaysAllow,
            execute_commands: ActionPermission::AlwaysAllow,
            mcp_permissions: ActionPermission::AlwaysAllow,
            write_to_pty: WriteToPtyPermission::AlwaysAllow,
            ask_user_question: AskUserQuestionPermission::Never,
            command_denylist,
            command_allowlist: DEFAULT_COMMAND_EXECUTION_ALLOWLIST.to_vec(),
            directory_allowlist: Vec::new(),
            mcp_allowlist: Vec::new(),
            mcp_denylist: Vec::new(),
            computer_use: computer_use_permission,
            base_model: None,
            coding_model: None,
            cli_agent_model: None,
            computer_use_model: None,
            context_window_limit: None,
            autosync_plans_to_warp_drive: FeatureFlag::SyncAmbientPlans.is_enabled(),
            web_search_enabled: true,
        }
    }
}

impl AIExecutionProfile {
    pub fn configurable_context_window(&self, app: &AppContext) -> Option<LLMContextWindow> {
        let prefs = LLMPreferences::as_ref(app);
        let cw = self
            .base_model
            .as_ref()
            .and_then(|id| prefs.get_llm_info(id))
            .map(|info| info.context_window.clone())
            .unwrap_or_else(|| prefs.get_default_base_model().context_window.clone());
        if cw.is_configurable && cw.max > 0 {
            Some(cw)
        } else {
            None
        }
    }

    pub fn context_window_display_value(&self, app: &AppContext) -> Option<u32> {
        let cw = self.configurable_context_window(app)?;
        Some(self.context_window_limit.unwrap_or(cw.default_max))
    }
}

pub type CloudAIExecutionProfile =
    GenericCloudObject<GenericStringObjectId, CloudAIExecutionProfileModel>;
pub type CloudAIExecutionProfileModel = GenericStringModel<AIExecutionProfile, JsonSerializer>;

impl StringModel for AIExecutionProfile {
    type CloudObjectType = CloudAIExecutionProfile;

    fn model_type_name(&self) -> &'static str {
        "AIExecutionProfile"
    }

    fn should_enforce_revisions() -> bool {
        true
    }

    fn model_format() -> GenericStringObjectFormat {
        GenericStringObjectFormat::Json(JsonObjectType::AIExecutionProfile)
    }

    fn should_show_activity_toasts() -> bool {
        false
    }

    fn warn_if_unsaved_at_quit() -> bool {
        true
    }

    fn display_name(&self) -> String {
        // Handles case where default profile was previously created and named "Untitled"
        if self.is_default_profile {
            "Default".to_string()
        } else if self.name.trim().is_empty() {
            "Untitled".to_string()
        } else {
            self.name.clone()
        }
    }

    fn update_object_queue_item(
        &self,
        revision_ts: Option<Revision>,
        object: &Self::CloudObjectType,
    ) -> QueueItem {
        QueueItem::UpdateAIExecutionProfile {
            model: object.model().clone().into(),
            id: object.id,
            revision: revision_ts.or_else(|| object.metadata.revision.clone()),
        }
    }

    fn new_from_server_update(&self, server_cloud_object: &ServerCloudObject) -> Option<Self> {
        if let ServerCloudObject::AIExecutionProfile(server_ai_execution_profile) =
            server_cloud_object
        {
            return Some(server_ai_execution_profile.model.clone().string_model);
        }
        None
    }

    fn should_clear_on_unique_key_conflict(&self) -> bool {
        true
    }

    fn uniqueness_key(&self) -> Option<GenericStringObjectUniqueKey> {
        // We want to prevent the creation of several default profiles per user. If it's not the default
        // profile, then there can be many.
        self.is_default_profile
            .then_some(GenericStringObjectUniqueKey {
                key: "default".to_string(),
                unique_per: UniquePer::User,
            })
    }

    fn renders_in_warp_drive(&self) -> bool {
        false
    }
}

impl JsonModel for AIExecutionProfile {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::AIExecutionProfile
    }
}
