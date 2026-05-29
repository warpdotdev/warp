use std::fmt;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::SortOrderArg;
use crate::config_file::ConfigFileArgs;
use crate::environment::EnvironmentCreateArgs;
use crate::json_filter::JsonOutput;
use crate::mcp::MCPSpec;
use crate::model::ModelArgs;
use crate::scope::ObjectScope;
use crate::share::ShareArgs;
use crate::skill::SkillSpec;

/// Output format for agent results.
#[derive(Debug, Copy, Clone, ValueEnum, Eq, PartialEq, Default)]
pub enum OutputFormat {
    /// Output as JSON.
    #[value(name = "json")]
    Json,
    /// Output as newline-delimited JSON.
    #[value(name = "ndjson")]
    Ndjson,
    /// Output as human-readable text.
    #[default]
    #[value(name = "pretty")]
    Pretty,
    /// Output as plain text.
    #[value(name = "text")]
    Text,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no values are skipped");
        f.write_str(value.get_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    PlainText(String),
    SavedPrompt(String),
}

impl fmt::Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Prompt::PlainText(text) => write!(f, "Prompt: {text}"),
            Prompt::SavedPrompt(id) => write!(f, "Saved Prompt ID: {id}"),
        }
    }
}

/// Prompt arguments - mutually exclusive prompt or saved-prompt.
/// The required constraint is enforced at the command level via ArgGroup.
#[derive(Debug, Clone, Args)]
#[group(multiple = false)]
pub struct PromptArg {
    /// Prompt for the agent to carry out.
    #[arg(long = "prompt", short = 'p')]
    pub prompt: Option<String>,
    /// The saved AI prompt to run, identified by id.
    #[arg(long = "saved-prompt")]
    pub saved_prompt: Option<String>,
}

impl PromptArg {
    pub fn to_prompt(&self) -> Option<Prompt> {
        match (self.prompt.as_ref(), self.saved_prompt.as_ref()) {
            (Some(prompt), None) => Some(Prompt::PlainText(prompt.clone())),
            (None, Some(saved_prompt)) => Some(Prompt::SavedPrompt(saved_prompt.clone())),
            _ => None,
        }
    }
}

/// Shared CLI args for controlling computer use capabilities.
#[derive(Debug, Clone, Args, Default)]
pub struct ComputerUseArgs {
    /// Enable computer use capabilities for this agent run.
    #[arg(long = "computer-use", conflicts_with = "no_computer_use")]
    pub computer_use: bool,

    /// Disable computer use capabilities for this agent run.
    #[arg(long = "no-computer-use", conflicts_with = "computer_use")]
    pub no_computer_use: bool,
}

impl ComputerUseArgs {
    /// Returns the computer use override based on CLI flags.
    /// - `Some(true)` if `--computer-use` was specified
    /// - `Some(false)` if `--no-computer-use` was specified
    /// - `None` if neither was specified (use default behavior)
    pub fn computer_use_override(&self) -> Option<bool> {
        match (self.computer_use, self.no_computer_use) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    }
}

/// Hidden variant of [`ComputerUseArgs`] for commands where computer use flags
/// should be accepted but not shown in help output.
#[derive(Debug, Clone, Args, Default)]
pub struct HiddenComputerUseArgs {
    /// Enable computer use capabilities for this agent run.
    #[arg(long = "computer-use", conflicts_with = "no_computer_use", hide = true)]
    pub computer_use: bool,

    /// Disable computer use capabilities for this agent run.
    #[arg(long = "no-computer-use", conflicts_with = "computer_use", hide = true)]
    pub no_computer_use: bool,
}

impl HiddenComputerUseArgs {
    pub fn computer_use_override(&self) -> Option<bool> {
        match (self.computer_use, self.no_computer_use) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    }
}
/// The execution harness for an agent run.
#[derive(Debug, Copy, Clone, ValueEnum, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    /// Use Warp's built-in MAA infrastructure (default).
    #[default]
    #[value(name = "oz")]
    Oz,
    /// Delegate to the `claude` CLI.
    #[value(name = "claude", alias = "claude-code")]
    Claude,
    /// Delegate to the `opencode` CLI.
    #[value(name = "opencode", alias = "open-code")]
    OpenCode,
    /// Delegate to the `gemini` CLI.
    #[value(name = "gemini")]
    Gemini,
    /// Delegate to the `codex` CLI.
    #[value(name = "codex")]
    Codex,
    /// A harness produced by a newer client/server that this client doesn't
    /// recognize. Surfaced via deserialization fallbacks (e.g. unknown GraphQL
    /// enum values, unknown `harness_type` strings); never selectable from the
    /// CLI or harness dropdown.
    #[serde(other)]
    #[value(skip)]
    Unknown,
}

impl Harness {
    pub fn parse_orchestration_harness(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        <Self as ValueEnum>::from_str(&normalized, true).ok()
    }

    pub fn parse_local_child_harness(value: &str) -> Option<Self> {
        match Self::parse_orchestration_harness(value) {
            Some(harness @ (Self::Claude | Self::OpenCode | Self::Codex)) => Some(harness),
            Some(Self::Oz) | Some(Self::Gemini) | Some(Self::Unknown) | None => None,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Oz => "Oz",
            Self::Claude => "Claude Code",
            Self::OpenCode => "OpenCode",
            Self::Gemini => "Gemini CLI",
            Self::Codex => "Codex",
            Self::Unknown => "Unknown",
        }
    }

    /// Parses a harness config-name string (the lowercase name written into
    /// `HarnessConfig::harness_type` by the spawner, e.g. `"claude"`, `"gemini"`, `"oz"`)
    /// into a [`Harness`] variant. Inverse of [`Harness::config_name`]. Returns `None` for
    /// unrecognized names so callers can distinguish a future-server harness from a
    /// round-tripped [`Harness::Unknown`]; callers that want to fall back to `Unknown`
    /// should `.unwrap_or(Harness::Unknown)`. UI surfaces should treat `Unknown` as a
    /// non-Oz, non-runnable harness.
    pub fn from_config_name(name: &str) -> Option<Self> {
        match name {
            "oz" => Some(Harness::Oz),
            "claude" => Some(Harness::Claude),
            "opencode" => Some(Harness::OpenCode),
            "gemini" => Some(Harness::Gemini),
            "codex" => Some(Harness::Codex),
            "unknown" => Some(Harness::Unknown),
            _ => None,
        }
    }

    /// Canonical config name for this harness (the lowercase string written into
    /// `HarnessConfig::harness_type`). Inverse of [`Harness::from_config_name`].
    /// The exhaustive match here forces every new [`Harness`] variant to declare a
    /// canonical name, which prevents `from_config_name` from silently falling back to
    /// `Unknown` when a new variant is added.
    pub fn config_name(self) -> &'static str {
        match self {
            Harness::Oz => "oz",
            Harness::Claude => "claude",
            Harness::OpenCode => "opencode",
            Harness::Gemini => "gemini",
            Harness::Codex => "codex",
            Harness::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.config_name())
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

/// Profile subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum AgentProfileCommand {
    /// List available agent profiles.
    List,
}

/// Agent-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum AgentCommand {
    /// Run a new Oz agent.
    Run(RunAgentArgs),
    /// Dispatch an Oz agent that runs remotely.
    RunCloud(RunCloudArgs),
    /// Manage agent profiles.
    #[command(subcommand)]
    Profile(AgentProfileCommand),
    /// List all available agents.
    List(AgentListArgs),
    /// Get details of an agent.
    Get(AgentGetArgs),
    /// Create a new agent.
    Create(AgentCreateArgs),
    /// Update an existing agent.
    Update(AgentUpdateArgs),
    /// Delete an agent.
    Delete(AgentDeleteArgs),
    /// List available agent skills.
    Skills(ListAgentSkillsArgs),
}

#[derive(Debug, Clone, Args)]
#[command(
    visible_alias = "r",
    group(
        clap::ArgGroup::new("prompt_group")
            .required(true)
            .multiple(true)
            .args(["prompt", "saved_prompt", "task_id", "skill"])
    )
)]
pub struct RunAgentArgs {
    #[command(flatten)]
    pub prompt_arg: PromptArg,

    #[command(flatten)]
    pub model: ModelArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    /// Use a skill as the base prompt for the agent.
    ///
    /// Format: `skill_name`, `repo:skill_name`, or `org/repo:skill_name`
    ///
    /// Skills are searched in `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, and `.codex/skills/` directories.
    /// If a repo is specified, searches only that repo. If org is also specified,
    /// validates the repo's git remote matches the expected org.
    ///
    /// When used with --prompt, the first skill provides the base context and the prompt is the task.
    /// Repeat this flag to make additional skills available to the agent.
    ///
    /// To automate a skill on a schedule, use `oz schedule create --skill <SKILL>`.
    #[arg(long = "skill", value_name = "SKILL")]
    pub skill: Vec<SkillSpec>,

    /// Name for this agent task.
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,
    /// Working directory for the agent
    #[arg(short = 'C', long = "cwd")]
    pub cwd: Option<PathBuf>,
    /// Display agent progress in the Warp interface.
    #[arg(long = "gui", hide = true)]
    pub gui: bool,
    #[command(flatten)]
    pub share: ShareArgs,
    /// MCP servers to start before executing the agent.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,
    /// LEGACY: MCP servers to start before executing the agent, identified by UUID.
    #[arg(long = "mcp-server", value_name = "UUID", hide = true)]
    pub mcp_servers: Vec<uuid::Uuid>,
    /// Cloud environment to use, identified by ID.
    #[arg(long = "environment", short = 'e', value_name = "ID")]
    pub environment: Option<String>,

    /// Keep the agent's session open after the conversation completes.
    ///
    /// This is useful when you want to keep the session alive for follow-up interactions.
    ///
    /// You can optionally provide a duration (e.g. `--idle-on-complete 10m`).
    #[arg(
        long = "idle-on-complete",
        value_name = "DURATION",
        num_args = 0..=1,
        default_missing_value = "45m",
        hide = true
    )]
    pub idle_on_complete: Option<humantime::Duration>,

    #[command(flatten)]
    pub snapshot: SnapshotArgs,
    /// Identifier for the task that spawned this agent, used to report progress.
    ///
    /// When `--conversation` is omitted, the conversation id is read off the server-side
    /// task metadata. Some worker follow-up call sites still pass both flags, so keep
    /// accepting the compatibility shape until all producers have been updated.
    #[arg(long = "task-id", hide = true, conflicts_with_all = ["prompt", "saved_prompt", "file"])]
    pub task_id: Option<String>,

    /// Whether we are running the agent in a sandboxed environment.
    #[arg(long = "sandboxed", hide = true)]
    pub sandboxed: bool,
    /// IAM role ARN to use for federated AWS Bedrock credentials for this run.
    #[arg(
        long = "bedrock-inference-role",
        value_name = "ROLE_ARN",
        requires = "bedrock_role_region",
        hide = true
    )]
    pub bedrock_inference_role: Option<String>,

    /// AWS region to use for the STS `AssumeRoleWithWebIdentity` call that
    /// mints federated Bedrock credentials. Required together with
    /// `--bedrock-inference-role`.
    #[arg(
        long = "bedrock-role-region",
        value_name = "REGION",
        requires = "bedrock_inference_role",
        hide = true
    )]
    pub bedrock_role_region: Option<String>,

    #[command(flatten)]
    pub computer_use: HiddenComputerUseArgs,

    /// Continue an existing cloud conversation by ID.
    #[arg(long = "conversation", value_name = "ID")]
    pub conversation: Option<String>,

    /// Agent profile to configure the terminal session.
    #[arg(long = "profile", value_name = "ID")]
    pub profile: Option<String>,

    /// Execution harness for the agent run.
    ///
    /// "oz" (default) uses Warp's built-in agent infrastructure.
    /// "claude" delegates to the `claude` CLI.
    #[arg(long = "harness", value_name = "HARNESS", default_value_t = Harness::Oz, hide = true)]
    pub harness: Harness,
}

impl RunAgentArgs {
    /// Combine `mcp_specs` with legacy `mcp_servers` (UUIDs) into a single list.
    pub fn all_mcp_specs(&self) -> Vec<MCPSpec> {
        let mut specs = self.mcp_specs.clone();
        specs.extend(self.mcp_servers.iter().cloned().map(MCPSpec::Uuid));
        specs
    }

    /// Return the first `--skill` value, which is the skill invoked for this run.
    pub fn invoked_skill(&self) -> Option<&SkillSpec> {
        self.skill.first()
    }
}

#[derive(Debug, Clone, Args)]
pub struct SnapshotArgs {
    /// Disable the end-of-run workspace snapshot upload.
    #[arg(long = "no-snapshot")]
    pub no_snapshot: bool,

    /// Maximum time to wait for the end-of-run snapshot upload.
    #[arg(long = "snapshot-upload-timeout", value_name = "DURATION")]
    pub snapshot_upload_timeout: Option<humantime::Duration>,

    /// Maximum time to wait for the declarations script before uploading the snapshot.
    #[arg(long = "snapshot-script-timeout", value_name = "DURATION")]
    pub snapshot_script_timeout: Option<humantime::Duration>,
}

#[derive(Debug, Clone, Args)]
#[command(
    name = "run-cloud",
    visible_alias = "ra",
    alias = "run-ambient",
    group(
        clap::ArgGroup::new("prompt_group")
            .required(true)
            .multiple(true)
            .args(["prompt", "saved_prompt", "skill"])
    )
)]
pub struct RunCloudArgs {
    #[command(flatten)]
    pub prompt_arg: PromptArg,

    #[command(flatten)]
    pub model: ModelArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    /// Use a skill as the base prompt for the agent.
    ///
    /// Format: `skill_name`, `repo:skill_name`, or `org/repo:skill_name`
    ///
    /// Skills are searched in `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, and `.codex/skills/` directories.
    /// If a repo is specified, searches only that repo. If org is also specified,
    /// validates the repo's git remote matches the expected org.
    ///
    /// When used with --prompt, the skill provides the base context and the prompt is the task.
    ///
    /// To automate a skill on a schedule, use `oz schedule create --skill <SKILL>`.
    #[arg(long = "skill", value_name = "SKILL")]
    pub skill: Option<SkillSpec>,

    /// Name for this agent task.
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,

    /// MCP servers to start before executing the agent.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,

    /// The environment to run this ambient agent in.
    #[command(flatten)]
    pub environment: EnvironmentCreateArgs,
    /// Open the agent's session in Warp once it's available.
    #[arg(long = "open")]
    pub open: bool,

    /// Continue an existing cloud conversation by ID.
    #[arg(long = "conversation", value_name = "ID")]
    pub conversation: Option<String>,

    #[command(flatten)]
    pub scope: ObjectScope,

    /// UID of the agent to execute this run as.
    ///
    /// This will apply the agent's configuration, such
    /// as its skills and base model, and attribute
    /// credit usage back to the agent.
    #[arg(long = "agent", value_name = "UID")]
    pub agent_uid: Option<String>,

    /// Where this job should be hosted. Setting "warp" runs it on Warp's infrastructure. Any other
    /// value is treated is a self-hosted job and the value will be matched with the self-hosted
    /// worker's name.
    #[arg(long = "host", value_name = "WORKER_ID")]
    pub worker_host: Option<String>,

    /// Path to a file to attach to the agent query.
    ///
    /// Can be specified multiple times to attach multiple files (maximum 5).
    ///
    /// Example: --attach file1.png --attach file2.txt
    #[arg(
        long = "attach",
        value_name = "PATH",
        num_args = 1,
        action = clap::ArgAction::Append,
        value_parser = clap::value_parser!(PathBuf),
    )]
    pub attachment_paths: Vec<PathBuf>,

    #[command(flatten)]
    pub computer_use: ComputerUseArgs,
    #[command(flatten)]
    pub snapshot: SnapshotArgs,

    /// Execution harness for the agent run.
    ///
    /// "oz" (default) uses Warp's built-in agent infrastructure.
    /// "claude" delegates to the `claude` CLI.
    #[arg(long = "harness", value_name = "HARNESS", default_value_t = Harness::Oz, hide = true)]
    pub harness: Harness,

    /// Name of a managed secret for Claude Code harness authentication.
    ///
    /// Resolved server-side and injected into the agent container.
    /// Only valid when --harness is set to "claude".
    #[arg(long = "claude-auth-secret", value_name = "NAME", hide = true)]
    pub claude_auth_secret: Option<String>,
}

/// Sort field for named agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AgentSortByArg {
    #[value(name = "name")]
    Name,
    #[value(name = "created-at")]
    CreatedAt,
}

/// Arguments for listing named agents.
#[derive(Debug, Clone, Args)]
pub struct AgentListArgs {
    /// Sort field. Only supported for pretty, text, and ndjson output.
    #[arg(long = "sort-by", value_enum, value_name = "FIELD")]
    pub sort_by: Option<AgentSortByArg>,

    /// Sort direction. Only supported for pretty, text, and ndjson output.
    #[arg(long = "sort-order", value_enum, value_name = "DIR")]
    pub sort_order: Option<SortOrderArg>,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// Arguments for getting a named agent.
#[derive(Debug, Clone, Args)]
pub struct AgentGetArgs {
    /// UID of the agent to get.
    pub uid: String,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// Arguments for creating a named agent.
#[derive(Debug, Clone, Args)]
pub struct AgentCreateArgs {
    /// Name of the agent.
    #[arg(long = "name", short = 'n')]
    pub name: String,

    /// Description of the agent.
    #[arg(long = "description")]
    pub description: Option<String>,

    /// Attach a secret to the agent. Repeat the flag for multiple secrets.
    #[arg(long = "secret", value_name = "NAME")]
    pub secrets: Vec<String>,

    /// Attach a skill to the agent. Repeat the flag for multiple skills.
    #[arg(long = "skill", value_name = "SKILL")]
    pub skills: Vec<String>,

    /// Base model for runs of this agent.
    #[arg(long = "base-model", value_name = "MODEL_ID")]
    pub base_model: Option<String>,

    /// Default cloud environment for runs of this agent.
    #[arg(long = "environment", short = 'e', value_name = "ENVIRONMENT_ID")]
    pub environment: Option<String>,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// Arguments for updating a named agent.
#[derive(Debug, Clone, Args)]
pub struct AgentUpdateArgs {
    /// UID of the agent to update.
    pub uid: String,

    /// New name for the agent.
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,

    /// Replacement description for the agent.
    #[arg(long = "description", conflicts_with = "remove_description")]
    pub description: Option<String>,

    /// Remove the agent description.
    #[arg(long = "remove-description", conflicts_with = "description")]
    pub remove_description: bool,

    /// Add a secret to the agent. Repeat the flag for multiple secrets.
    #[arg(
        long = "add-secret",
        value_name = "NAME",
        conflicts_with = "remove_all_secrets"
    )]
    pub add_secrets: Vec<String>,

    /// Remove a secret from the agent. Repeat the flag for multiple secrets.
    #[arg(
        long = "remove-secret",
        value_name = "NAME",
        conflicts_with = "remove_all_secrets"
    )]
    pub remove_secrets: Vec<String>,

    /// Remove all secrets from the agent.
    #[arg(
        long = "remove-all-secrets",
        conflicts_with_all = ["add_secrets", "remove_secrets"]
    )]
    pub remove_all_secrets: bool,

    /// Add a skill to the agent. Repeat the flag for multiple skills.
    #[arg(
        long = "add-skill",
        value_name = "SKILL",
        conflicts_with = "remove_all_skills"
    )]
    pub add_skills: Vec<String>,

    /// Remove a skill from the agent. Repeat the flag for multiple skills.
    #[arg(
        long = "remove-skill",
        value_name = "SKILL",
        conflicts_with = "remove_all_skills"
    )]
    pub remove_skills: Vec<String>,

    /// Remove all skills from the agent.
    #[arg(
        long = "remove-all-skills",
        conflicts_with_all = ["add_skills", "remove_skills"]
    )]
    pub remove_all_skills: bool,

    /// Replacement base model for runs executed by this agent.
    #[arg(
        long = "base-model",
        value_name = "MODEL_ID",
        conflicts_with = "remove_base_model"
    )]
    pub base_model: Option<String>,

    /// Remove the agent base model.
    #[arg(long = "remove-base-model", conflicts_with = "base_model")]
    pub remove_base_model: bool,

    /// Replacement default cloud environment for runs executed by this agent.
    #[arg(
        long = "environment",
        short = 'e',
        value_name = "ENVIRONMENT_ID",
        conflicts_with = "remove_environment"
    )]
    pub environment: Option<String>,

    /// Remove the agent default environment.
    #[arg(long = "remove-environment", conflicts_with = "environment")]
    pub remove_environment: bool,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

/// Arguments for deleting a named agent.
#[derive(Debug, Clone, Args)]
pub struct AgentDeleteArgs {
    /// UID of the agent to delete.
    pub uid: String,
}

/// Arguments for listing available agent skills.
#[derive(Debug, Clone, Args)]
pub struct ListAgentSkillsArgs {
    /// List skills from a specific GitHub repository.
    ///
    /// Format: `owner/repo` or `https://github.com/owner/repo`
    ///
    /// When provided, lists skills from this repo instead of from your environments.
    /// Any environments that include this repo will still be shown in the results.
    #[arg(long = "repo", short = 'r', value_name = "REPO")]
    pub repo: Option<String>,
}
