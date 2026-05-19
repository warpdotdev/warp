use std::path::PathBuf;

use ai::LLMId;
use cloud_objects::{
    cloud_object::{GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType},
    ids::GenericStringObjectId,
};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use warp_core::{channel::ChannelState, features::FeatureFlag};

use crate::{JsonModel, JsonSerializer};

pub const PROFILE_NAME_MAX_LENGTH: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionPermission {
    AgentDecides,
    AlwaysAllow,
    AlwaysAsk,
    // This is intended to catch deserialization errors whenever we add new variants to this enum. For example, if we add a "Never" variant, old clients can still deserialize it into an existing option.
    #[serde(other)]
    Unknown,
}

impl ActionPermission {
    pub fn description(&self) -> &'static str {
        match self {
            ActionPermission::AgentDecides | ActionPermission::Unknown => {
                "The Agent chooses the safest path: acting on its own when confident, and asking for approval when uncertain."
            }
            ActionPermission::AlwaysAllow => {
                "Give the Agent full autonomy  — no manual approval ever required."
            }
            ActionPermission::AlwaysAsk => {
                "Require explicit approval before the Agent takes any action."
            }
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
            WriteToPtyPermission::AlwaysAsk => {
                "The agent will always ask for permission to interact with a running command."
            }
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
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskUserQuestionPermission {
    Never,
    AskExceptInAutoApprove,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
enum AgentModeCommandExecutionPredicateType {
    #[serde(with = "serde_regex")]
    AnchoredRegex(Regex),
}

impl AgentModeCommandExecutionPredicateType {
    fn new_regex(regex: &str) -> Result<Self, regex::Error> {
        let anchored_regex = Regex::new(&format!("^{regex}$"))?;
        Ok(Self::AnchoredRegex(anchored_regex))
    }

    fn matches(&self, cmd: &str) -> bool {
        match self {
            Self::AnchoredRegex(regex) => regex.is_match(cmd),
        }
    }
}

impl PartialEq for AgentModeCommandExecutionPredicateType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::AnchoredRegex(a), Self::AnchoredRegex(b)) => {
                let a_unanchored = &a.as_str()[1..a.as_str().len() - 1];
                let b_unanchored = &b.as_str()[1..b.as_str().len() - 1];
                a_unanchored == b_unanchored
            }
        }
    }
}

impl std::fmt::Display for AgentModeCommandExecutionPredicateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AnchoredRegex(regex) => {
                write!(f, "{}", &regex.as_str()[1..regex.as_str().len() - 1])
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(transparent)]
pub struct AgentModeCommandExecutionPredicate(AgentModeCommandExecutionPredicateType);

impl schemars::JsonSchema for AgentModeCommandExecutionPredicate {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("AgentModeCommandExecutionPredicate")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<String>()
    }
}

impl AgentModeCommandExecutionPredicate {
    pub fn new_regex(regex: &str) -> Result<Self, regex::Error> {
        Ok(Self(AgentModeCommandExecutionPredicateType::new_regex(
            regex,
        )?))
    }

    pub fn matches(&self, cmd: &str) -> bool {
        self.0.matches(cmd)
    }
}

impl std::fmt::Display for AgentModeCommandExecutionPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl settings_value::SettingsValue for AgentModeCommandExecutionPredicate {
    fn to_file_value(&self) -> serde_json::Value {
        serde_json::Value::String(self.to_string())
    }

    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
        value.as_str().and_then(|s| Self::new_regex(s).ok())
    }
}

lazy_static! {
    static ref OPTIONAL_ARGS_REGEX: Regex =
        Regex::new(r"(\s.*)?").expect("Can parse optional args regex");
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        lazy_static! {
            pub static ref DEFAULT_COMMAND_EXECUTION_ALLOWLIST: Vec<AgentModeCommandExecutionPredicate> = vec![];
            pub static ref DEFAULT_COMMAND_EXECUTION_DENYLIST: Vec<AgentModeCommandExecutionPredicate> = vec![];
        }
    } else {
        lazy_static! {
            pub static ref DEFAULT_COMMAND_EXECUTION_ALLOWLIST: Vec<AgentModeCommandExecutionPredicate> = vec![
                AgentModeCommandExecutionPredicate::new_regex(&format!("cat{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default cat rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("echo{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default echo rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex("find .*").expect("Can parse default find rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("grep{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default grep rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("ls{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default ls rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex("which .*").expect("Can parse default which rule into regex"),
            ];

            pub static ref DEFAULT_COMMAND_EXECUTION_DENYLIST: Vec<AgentModeCommandExecutionPredicate> = vec![
                AgentModeCommandExecutionPredicate::new_regex(&format!("bash{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default bash rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("fish{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default fish rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("pwsh{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default pwsh rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("sh{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default sh rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("zsh{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default zsh rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("curl{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default curl rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("eval{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default eval rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("exec{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default exec rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("source{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default source rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("wget{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default wget rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("dig{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default dig rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("nslookup{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default nslookup rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("host{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default host rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("ssh{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default ssh rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("scp{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default scp rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("rsync{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default rsync rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("telnet{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default telnet rule into regex"),
                AgentModeCommandExecutionPredicate::new_regex(&format!("rm{}", OPTIONAL_ARGS_REGEX.as_str())).expect("Can parse default rm rule into regex"),
            ];
        }
    }
}

/// Core data structure representing an AI execution profile, which includes model configuration, behavior settings, and permissions.
///
/// NOTE: `planning_model` was removed after planning via subagent was deprecated; serialized legacy profiles may include a `planning_model` field and this field name should remain reserved indefinitely.
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
    /// Always ask for permission for these commands.
    pub command_denylist: Vec<AgentModeCommandExecutionPredicate>,
    /// When `execute_commands` is set to `AlwaysAsk`, autoexecute these commands.
    pub command_allowlist: Vec<AgentModeCommandExecutionPredicate>,
    /// When `read_files` is set to `AlwaysAsk`, autoread from these directories.
    pub directory_allowlist: Vec<PathBuf>,
    pub mcp_allowlist: Vec<uuid::Uuid>,
    pub mcp_denylist: Vec<uuid::Uuid>,
    pub computer_use: ComputerUsePermission,
    pub base_model: Option<LLMId>,
    pub coding_model: Option<LLMId>,
    pub cli_agent_model: Option<LLMId>,
    pub computer_use_model: Option<LLMId>,
    pub context_window_limit: Option<u32>,
    /// Whether plans created by the agent should be automatically synced to Warp Drive.
    pub autosync_plans_to_warp_drive: bool,
    /// Whether the agent may use web search when helpful for completing tasks.
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

    /// This creates a CLI-specific profile that will never ask the user for permission, since we cannot do so in a non-interactive setting.
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

impl JsonModel for AIExecutionProfile {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::AIExecutionProfile
    }
}

pub type CloudAIExecutionProfile =
    GenericCloudObject<GenericStringObjectId, CloudAIExecutionProfileModel>;
pub type CloudAIExecutionProfileModel = GenericStringModel<AIExecutionProfile, JsonSerializer>;
pub type ServerAIExecutionProfile =
    GenericServerObject<GenericStringObjectId, CloudAIExecutionProfileModel>;
