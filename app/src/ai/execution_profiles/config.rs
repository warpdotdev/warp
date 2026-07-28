//! Stable IDs and file-backed serialization for the execution-profile collection.
//!
//! The setting is a map from [`ExecutionProfileId`] to
//! [`AIExecutionProfile`]. The map key is the stable identity and the
//! `default` key determines which profile is the default. File I/O passes
//! through [`ExecutionProfileFile`] so unsupported wire values, invalid
//! regular expressions, and invalid UUIDs cannot enter the active collection.

use std::collections::HashMap;
use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    AIExecutionProfile, ActionPermission, AskUserQuestionPermission, ComputerUsePermission,
    RunAgentsPermission, WriteToPtyPermission,
};
use crate::ai::llms::LLMId;
use crate::server::ids::ServerId;
use crate::settings::AgentModeCommandExecutionPredicate;

/// Reserved key for the one required default profile.
const DEFAULT_PROFILE_KEY: &str = "default";
/// Prefix for opaque IDs assigned to newly created profiles.
const GENERATED_PROFILE_PREFIX: &str = "profile-";
/// Prefix for deterministic IDs assigned during legacy cloud import.
const LEGACY_PROFILE_PREFIX: &str = "legacy-";

/// Stable identifier used as a profile's key in `settings.toml`.
///
/// Keys are non-empty and contain only ASCII letters, digits, underscores,
/// and hyphens. The reserved `default` key identifies the default profile.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct ExecutionProfileId(String);

impl ExecutionProfileId {
    /// Generates a new opaque profile ID.
    pub fn new() -> Self {
        Self::generated()
    }

    /// Returns the reserved ID for the collection's default profile.
    pub fn default_profile() -> Self {
        Self(DEFAULT_PROFILE_KEY.to_string())
    }

    /// Generates a fresh opaque `profile-<uuid>` ID.
    pub fn generated() -> Self {
        Self(format!(
            "{GENERATED_PROFILE_PREFIX}{}",
            uuid::Uuid::new_v4()
        ))
    }

    /// Derives a deterministic file-safe ID from a legacy cloud object.
    ///
    /// Independent devices therefore map the same cloud profile to the same
    /// collection entry.
    pub fn from_legacy_server_id(server_id: ServerId) -> Self {
        Self(format!(
            "{LEGACY_PROFILE_PREFIX}{}",
            hex::encode(server_id.to_string())
        ))
    }

    /// Parses and validates a profile key.
    pub fn parse(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        Self::is_valid_key(&value).then_some(Self(value))
    }

    /// Returns whether this is the reserved default-profile ID.
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_PROFILE_KEY
    }

    /// Returns the key as it appears in the serialized collection.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_valid_key(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }
}

impl<'de> Deserialize<'de> for ExecutionProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).ok_or_else(|| serde::de::Error::custom("invalid execution profile key"))
    }
}

impl fmt::Display for ExecutionProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Complete execution-profile collection persisted as one setting value.
///
/// The collection always contains [`ExecutionProfileId::default_profile`].
/// Each profile's `is_default_profile` field is derived from its map key
/// instead of trusted from serialized data.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExecutionProfilesConfig(IndexMap<ExecutionProfileId, AIExecutionProfile>);

impl ExecutionProfilesConfig {
    /// Returns the profile with the given stable ID.
    pub fn profile(&self, id: &ExecutionProfileId) -> Option<&AIExecutionProfile> {
        self.0.get(id)
    }

    /// Returns the mutable profile with the given stable ID.
    pub fn profile_mut(&mut self, id: &ExecutionProfileId) -> Option<&mut AIExecutionProfile> {
        self.0.get_mut(id)
    }

    /// Iterates over stable IDs in persisted order.
    pub fn profile_ids(&self) -> impl Iterator<Item = &ExecutionProfileId> {
        self.0.keys()
    }

    /// Iterates over IDs and profiles in persisted order.
    pub fn profiles(&self) -> impl Iterator<Item = (&ExecutionProfileId, &AIExecutionProfile)> {
        self.0.iter()
    }

    /// Inserts a profile and derives its default marker from the ID.
    pub fn insert(
        &mut self,
        id: ExecutionProfileId,
        mut profile: AIExecutionProfile,
    ) -> Option<AIExecutionProfile> {
        profile.is_default_profile = id.is_default();
        self.0.insert(id, profile)
    }

    /// Removes a non-default profile.
    ///
    /// The reserved default profile cannot be removed.
    pub fn remove(&mut self, id: &ExecutionProfileId) -> Option<AIExecutionProfile> {
        if id.is_default() {
            return None;
        }
        self.0.shift_remove(id)
    }

    /// Builds a validated collection and derives every default marker.
    pub(crate) fn from_profiles(
        profiles: IndexMap<ExecutionProfileId, AIExecutionProfile>,
    ) -> Option<Self> {
        if !profiles.contains_key(&ExecutionProfileId::default_profile()) {
            return None;
        }
        let mut config = Self(profiles);
        for (id, profile) in &mut config.0 {
            profile.is_default_profile = id.is_default();
        }
        Some(config)
    }
}

impl Default for ExecutionProfilesConfig {
    fn default() -> Self {
        let id = ExecutionProfileId::default_profile();
        let mut profile = AIExecutionProfile {
            name: "Default".to_string(),
            ..Default::default()
        };
        profile.is_default_profile = true;
        Self(IndexMap::from([(id, profile)]))
    }
}

impl<'de> Deserialize<'de> for ExecutionProfilesConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let profiles =
            IndexMap::<ExecutionProfileId, AIExecutionProfile>::deserialize(deserializer)?;
        Self::from_profiles(profiles)
            .ok_or_else(|| serde::de::Error::custom("execution profiles must include `default`"))
    }
}

impl settings_value::SettingsValue for ExecutionProfilesConfig {
    fn to_file_value(&self) -> serde_json::Value {
        let profiles = self
            .profiles()
            .map(|(id, profile)| {
                (
                    id.as_str().to_string(),
                    serde_json::to_value(ExecutionProfileFile::from(profile))
                        .expect("execution profile file value should serialize"),
                )
            })
            .collect();
        serde_json::Value::Object(profiles)
    }

    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        // Collect through Option so one invalid key or profile rejects the
        // complete permission-bearing collection.
        let profiles = object
            .iter()
            .map(|(key, value)| {
                let id = ExecutionProfileId::parse(key.clone())?;
                let file_profile =
                    serde_json::from_value::<ExecutionProfileFile>(value.clone()).ok()?;
                let profile = AIExecutionProfile::try_from(file_profile).ok()?;
                Some((id, profile))
            })
            .collect::<Option<IndexMap<_, _>>>()?;
        Self::from_profiles(profiles)
    }

    fn file_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema
    where
        Self: schemars::JsonSchema,
    {
        generator.subschema_for::<HashMap<String, ExecutionProfileFile>>()
    }
}

impl schemars::JsonSchema for ExecutionProfilesConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ExecutionProfiles")
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<HashMap<String, ExecutionProfileFile>>()
    }
}

// Domain-only `Unknown` values are written conservatively as `always_ask`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "File-safe representation of general agent action permissions.",
    rename_all = "snake_case"
)]
enum FileActionPermission {
    #[schemars(description = "The agent decides whether explicit approval is required.")]
    AgentDecides,
    #[schemars(description = "The action may run without approval.")]
    AlwaysAllow,
    #[schemars(description = "The action always requires approval.")]
    #[default]
    AlwaysAsk,
}

impl From<ActionPermission> for FileActionPermission {
    fn from(value: ActionPermission) -> Self {
        match value {
            ActionPermission::AgentDecides => Self::AgentDecides,
            ActionPermission::AlwaysAllow => Self::AlwaysAllow,
            ActionPermission::AlwaysAsk | ActionPermission::Unknown => Self::AlwaysAsk,
        }
    }
}

impl From<FileActionPermission> for ActionPermission {
    fn from(value: FileActionPermission) -> Self {
        match value {
            FileActionPermission::AgentDecides => Self::AgentDecides,
            FileActionPermission::AlwaysAllow => Self::AlwaysAllow,
            FileActionPermission::AlwaysAsk => Self::AlwaysAsk,
        }
    }
}

// Domain-only `Unknown` values are written conservatively as `always_ask`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "File-safe representation of terminal-write permissions.",
    rename_all = "snake_case"
)]
enum FileWriteToPtyPermission {
    #[schemars(description = "Terminal writes may proceed without approval.")]
    AlwaysAllow,
    #[schemars(description = "Every terminal write requires approval.")]
    #[default]
    AlwaysAsk,
    #[schemars(description = "Only the first write to a process requires approval.")]
    AskOnFirstWrite,
}

impl From<WriteToPtyPermission> for FileWriteToPtyPermission {
    fn from(value: WriteToPtyPermission) -> Self {
        match value {
            WriteToPtyPermission::AlwaysAllow => Self::AlwaysAllow,
            WriteToPtyPermission::AlwaysAsk | WriteToPtyPermission::Unknown => Self::AlwaysAsk,
            WriteToPtyPermission::AskOnFirstWrite => Self::AskOnFirstWrite,
        }
    }
}

impl From<FileWriteToPtyPermission> for WriteToPtyPermission {
    fn from(value: FileWriteToPtyPermission) -> Self {
        match value {
            FileWriteToPtyPermission::AlwaysAllow => Self::AlwaysAllow,
            FileWriteToPtyPermission::AlwaysAsk => Self::AlwaysAsk,
            FileWriteToPtyPermission::AskOnFirstWrite => Self::AskOnFirstWrite,
        }
    }
}

// Domain-only `Unknown` values are written conservatively as `always_ask`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "File-safe representation of permission to ask the user questions.",
    rename_all = "snake_case"
)]
enum FileAskUserQuestionPermission {
    #[schemars(description = "The agent may not ask the user questions.")]
    Never,
    #[schemars(description = "Questions are suppressed only during auto-approval.")]
    AskExceptInAutoApprove,
    #[schemars(description = "Questions are always available to the agent.")]
    #[default]
    AlwaysAsk,
}

impl From<AskUserQuestionPermission> for FileAskUserQuestionPermission {
    fn from(value: AskUserQuestionPermission) -> Self {
        match value {
            AskUserQuestionPermission::Never => Self::Never,
            AskUserQuestionPermission::AskExceptInAutoApprove => Self::AskExceptInAutoApprove,
            AskUserQuestionPermission::AlwaysAsk | AskUserQuestionPermission::Unknown => {
                Self::AlwaysAsk
            }
        }
    }
}

impl From<FileAskUserQuestionPermission> for AskUserQuestionPermission {
    fn from(value: FileAskUserQuestionPermission) -> Self {
        match value {
            FileAskUserQuestionPermission::Never => Self::Never,
            FileAskUserQuestionPermission::AskExceptInAutoApprove => Self::AskExceptInAutoApprove,
            FileAskUserQuestionPermission::AlwaysAsk => Self::AlwaysAsk,
        }
    }
}

// Domain-only `Unknown` values fail closed to `never_allow`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "File-safe representation of child-agent launch permissions.",
    rename_all = "snake_case"
)]
enum FileRunAgentsPermission {
    #[schemars(description = "Child agents may not be launched.")]
    NeverAllow,
    #[schemars(description = "Child agents may be launched without approval.")]
    AlwaysAllow,
    #[schemars(description = "Child-agent launches require approval.")]
    #[default]
    AlwaysAsk,
}

impl From<RunAgentsPermission> for FileRunAgentsPermission {
    fn from(value: RunAgentsPermission) -> Self {
        match value {
            RunAgentsPermission::NeverAllow | RunAgentsPermission::Unknown => Self::NeverAllow,
            RunAgentsPermission::AlwaysAllow => Self::AlwaysAllow,
            RunAgentsPermission::AlwaysAsk => Self::AlwaysAsk,
        }
    }
}

impl From<FileRunAgentsPermission> for RunAgentsPermission {
    fn from(value: FileRunAgentsPermission) -> Self {
        match value {
            FileRunAgentsPermission::NeverAllow => Self::NeverAllow,
            FileRunAgentsPermission::AlwaysAllow => Self::AlwaysAllow,
            FileRunAgentsPermission::AlwaysAsk => Self::AlwaysAsk,
        }
    }
}

// Domain-only `Unknown` values fail closed to `never`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "File-safe representation of computer-use permissions.",
    rename_all = "snake_case"
)]
enum FileComputerUsePermission {
    #[schemars(description = "Computer use is disabled.")]
    #[default]
    Never,
    #[schemars(description = "Each computer-use request requires approval.")]
    AlwaysAsk,
    #[schemars(description = "Computer use may proceed without approval.")]
    AlwaysAllow,
}

impl From<ComputerUsePermission> for FileComputerUsePermission {
    fn from(value: ComputerUsePermission) -> Self {
        match value {
            ComputerUsePermission::Never | ComputerUsePermission::Unknown => Self::Never,
            ComputerUsePermission::AlwaysAsk => Self::AlwaysAsk,
            ComputerUsePermission::AlwaysAllow => Self::AlwaysAllow,
        }
    }
}

impl From<FileComputerUsePermission> for ComputerUsePermission {
    fn from(value: FileComputerUsePermission) -> Self {
        match value {
            FileComputerUsePermission::Never => Self::Never,
            FileComputerUsePermission::AlwaysAsk => Self::AlwaysAsk,
            FileComputerUsePermission::AlwaysAllow => Self::AlwaysAllow,
        }
    }
}

// `is_default_profile` is omitted because the containing map key owns that
// invariant. String-backed regex and UUID fields are validated while converting
// back to [`AIExecutionProfile`].
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
#[schemars(description = "User-editable representation of one execution profile.")]
struct ExecutionProfileFile {
    #[schemars(description = "User-facing profile name.")]
    name: String,
    #[schemars(description = "Permission to apply generated code changes.")]
    apply_code_diffs: FileActionPermission,
    #[schemars(description = "Permission to read files.")]
    read_files: FileActionPermission,
    #[schemars(description = "Permission to execute commands.")]
    execute_commands: FileActionPermission,
    #[schemars(description = "Permission to write to interactive terminal processes.")]
    write_to_pty: FileWriteToPtyPermission,
    #[schemars(description = "Permission to call MCP servers.")]
    mcp_permissions: FileActionPermission,
    #[schemars(description = "Permission to ask the user questions.")]
    ask_user_question: FileAskUserQuestionPermission,
    #[schemars(description = "Permission to launch child agents.")]
    run_agents: FileRunAgentsPermission,
    #[schemars(description = "Command patterns that must always require approval.")]
    command_denylist: Vec<String>,
    #[schemars(description = "Command patterns that may execute without approval.")]
    command_allowlist: Vec<String>,
    #[schemars(description = "Directories that may be read without approval.")]
    directory_allowlist: Vec<std::path::PathBuf>,
    #[schemars(description = "MCP server IDs that may be called without approval.")]
    mcp_allowlist: Vec<String>,
    #[schemars(description = "MCP server IDs that must require approval.")]
    mcp_denylist: Vec<String>,
    #[schemars(description = "Permission to use the computer-use tool.")]
    computer_use: FileComputerUsePermission,
    #[schemars(description = "Optional base-model override.")]
    base_model: Option<String>,
    #[schemars(description = "Optional coding-model override.")]
    coding_model: Option<String>,
    #[schemars(description = "Optional full-terminal-use model override.")]
    cli_agent_model: Option<String>,
    #[schemars(description = "Optional computer-use model override.")]
    computer_use_model: Option<String>,
    #[schemars(description = "Optional maximum context window in tokens.")]
    context_window_limit: Option<u32>,
    #[schemars(description = "Whether plans are automatically synced to Warp Drive.")]
    autosync_plans_to_warp_drive: bool,
    #[schemars(description = "Whether the web-search tool is available.")]
    web_search_enabled: bool,
}

impl Default for ExecutionProfileFile {
    fn default() -> Self {
        Self::from(&AIExecutionProfile::default())
    }
}

impl From<&AIExecutionProfile> for ExecutionProfileFile {
    fn from(profile: &AIExecutionProfile) -> Self {
        Self {
            name: profile.name.clone(),
            apply_code_diffs: profile.apply_code_diffs.into(),
            read_files: profile.read_files.into(),
            execute_commands: profile.execute_commands.into(),
            write_to_pty: profile.write_to_pty.into(),
            mcp_permissions: profile.mcp_permissions.into(),
            ask_user_question: profile.ask_user_question.into(),
            run_agents: profile.run_agents.into(),
            command_denylist: profile
                .command_denylist
                .iter()
                .map(ToString::to_string)
                .collect(),
            command_allowlist: profile
                .command_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            directory_allowlist: profile.directory_allowlist.clone(),
            mcp_allowlist: profile
                .mcp_allowlist
                .iter()
                .map(ToString::to_string)
                .collect(),
            mcp_denylist: profile
                .mcp_denylist
                .iter()
                .map(ToString::to_string)
                .collect(),
            computer_use: profile.computer_use.into(),
            base_model: profile.base_model.clone().map(Into::into),
            coding_model: profile.coding_model.clone().map(Into::into),
            cli_agent_model: profile.cli_agent_model.clone().map(Into::into),
            computer_use_model: profile.computer_use_model.clone().map(Into::into),
            context_window_limit: profile.context_window_limit,
            autosync_plans_to_warp_drive: profile.autosync_plans_to_warp_drive,
            web_search_enabled: profile.web_search_enabled,
        }
    }
}

impl TryFrom<ExecutionProfileFile> for AIExecutionProfile {
    type Error = ();

    fn try_from(file: ExecutionProfileFile) -> Result<Self, Self::Error> {
        // Regex and UUID validation happens at the file boundary. Returning an
        // error for any entry prevents partial recovery of a malformed profile.
        fn parse_commands(
            commands: Vec<String>,
        ) -> Result<Vec<AgentModeCommandExecutionPredicate>, ()> {
            commands
                .into_iter()
                .map(|command| {
                    AgentModeCommandExecutionPredicate::new_regex(&command).map_err(|_| ())
                })
                .collect()
        }

        fn parse_uuids(ids: Vec<String>) -> Result<Vec<uuid::Uuid>, ()> {
            ids.into_iter()
                .map(|id| uuid::Uuid::parse_str(&id).map_err(|_| ()))
                .collect()
        }

        Ok(AIExecutionProfile {
            name: file.name,
            // The containing collection derives this from the stable map key.
            is_default_profile: false,
            apply_code_diffs: file.apply_code_diffs.into(),
            read_files: file.read_files.into(),
            execute_commands: file.execute_commands.into(),
            write_to_pty: file.write_to_pty.into(),
            mcp_permissions: file.mcp_permissions.into(),
            ask_user_question: file.ask_user_question.into(),
            run_agents: file.run_agents.into(),
            command_denylist: parse_commands(file.command_denylist)?,
            command_allowlist: parse_commands(file.command_allowlist)?,
            directory_allowlist: file.directory_allowlist,
            mcp_allowlist: parse_uuids(file.mcp_allowlist)?,
            mcp_denylist: parse_uuids(file.mcp_denylist)?,
            computer_use: file.computer_use.into(),
            base_model: file.base_model.map(LLMId::from),
            coding_model: file.coding_model.map(LLMId::from),
            cli_agent_model: file.cli_agent_model.map(LLMId::from),
            computer_use_model: file.computer_use_model.map(LLMId::from),
            context_window_limit: file.context_window_limit,
            autosync_plans_to_warp_drive: file.autosync_plans_to_warp_drive,
            web_search_enabled: file.web_search_enabled,
        })
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
