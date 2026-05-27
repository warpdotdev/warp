use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::ChipValue;
use crate::terminal::model::block::{Block, BlockMetadata};
use crate::terminal::model::session::{Session, SessionId};
use crate::terminal::shell::ShellType;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellCommandGenerator {
    command: ShellCommand,
    dependencies: Vec<String>,
}

/// Representation of a shell command. The command may or may not be supported on all shells.
///
/// In YAML (as a hypothetical example), this should work with a variable format like:
/// ```yaml
/// # This parses to ShellCommand::Portable
/// - "this is a portable command"
/// # This parses to ShellCommand::ShellSpecific
/// - bash: "this works on bash"
///   zsh: "this works on zsh"
///   # this command does not support Fish
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ShellCommand {
    /// A shell command that works on all shells.
    Portable(String),
    /// A shell command that only works on specific shells.
    ShellSpecific(HashMap<ShellType, String>),
}

impl ShellCommandGenerator {
    pub fn command(&self) -> &ShellCommand {
        &self.command
    }

    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    pub fn new(command: ShellCommand, dependencies: Option<Vec<String>>) -> Self {
        Self {
            command,
            dependencies: dependencies.unwrap_or_default(),
        }
    }
}

impl ShellCommand {
    /// Construct a new portable shell command.
    pub fn portable(command: impl Into<String>) -> Self {
        Self::Portable(command.into())
    }

    /// Construct a set of shell-specific commands.
    pub fn shell_specific(commands: impl Into<HashMap<ShellType, String>>) -> Self {
        Self::ShellSpecific(commands.into())
    }

    /// Gets the variant of this command that works on the given shell. If this command does not
    /// support the shell, returns `None`.
    pub fn for_shell(&self, shell_type: ShellType) -> Option<&str> {
        match self {
            Self::Portable(command) => Some(command.as_str()),
            Self::ShellSpecific(commands) => commands.get(&shell_type).map(String::as_str),
        }
    }
}

/// Tracks whether the set of external commands (executables on `$PATH`) has been loaded for a
/// session, and if so, which required commands are present.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ExternalCommandsAvailability {
    #[default]
    Unknown,
    Known {
        command_count: usize,
        required_command_presence: HashMap<String, bool>,
    },
}

impl ExternalCommandsAvailability {
    pub fn contains(&self, command: &str) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Known {
                required_command_presence,
                ..
            } => required_command_presence.get(command).copied(),
        }
    }

    pub fn command_count(&self) -> Option<usize> {
        match self {
            Self::Unknown => None,
            Self::Known { command_count, .. } => Some(*command_count),
        }
    }
}

/// A snapshot of session-level capabilities that a chip's runtime policy uses to determine
/// availability (e.g. whether the session is local, which executables are present).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ChipRuntimeCapabilities {
    pub session_id: Option<SessionId>,
    pub session_is_local: Option<bool>,
    pub external_commands: ExternalCommandsAvailability,
}

impl ChipRuntimeCapabilities {
    pub fn from_session(session: &Session) -> Self {
        Self::from_session_with_external_command_queries(session, std::iter::empty::<&str>(), false)
    }

    pub fn from_session_with_external_command_queries<'a>(
        session: &Session,
        required_executables: impl IntoIterator<Item = &'a str>,
        include_external_command_count: bool,
    ) -> Self {
        let external_commands = if session.has_loaded_external_commands() {
            let mut required_command_presence = HashMap::new();
            let required_executables = required_executables.into_iter().collect::<HashSet<_>>();

            let should_scan_executables =
                include_external_command_count || !required_executables.is_empty();
            let mut command_count = 0;
            if should_scan_executables {
                for executable in session.executable_names() {
                    command_count += 1;
                    if required_executables.contains(executable) {
                        required_command_presence.insert(executable.to_string(), true);
                    }
                }
            }

            for required_executable in required_executables {
                required_command_presence
                    .entry(required_executable.to_string())
                    .or_insert(false);
            }

            ExternalCommandsAvailability::Known {
                command_count,
                required_command_presence,
            }
        } else {
            ExternalCommandsAvailability::Unknown
        };

        Self {
            session_id: Some(session.id()),
            session_is_local: Some(session.is_local()),
            external_commands,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChipDisabledReason {
    RequiresLocalSession,
    RequiresExecutable { command: String },
}

impl ChipDisabledReason {
    pub fn tooltip_text(&self) -> String {
        match self {
            Self::RequiresLocalSession => "Requires a local session".to_string(),
            Self::RequiresExecutable { command } if command == "gh" => {
                "Requires the GitHub CLI".to_string()
            }
            Self::RequiresExecutable { command } => format!("Requires the `{command}` command"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ChipAvailability {
    #[default]
    Enabled,
    Disabled(ChipDisabledReason),
    Hidden,
}

impl ChipAvailability {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    pub fn tooltip_override_text(&self) -> Option<String> {
        match self {
            Self::Disabled(reason) => Some(reason.tooltip_text()),
            Self::Enabled | Self::Hidden => None,
        }
    }
}

/// An input that contributes to a chip's fingerprint hash. When all fingerprint inputs match
/// the previously computed fingerprint, the chip can skip re-fetching its value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ChipFingerprintInput {
    SessionId,
    SessionIsLocal,
    WorkingDirectory,
    GitBranch,
    PythonVirtualenv,
    CondaEnvironment,
    NodeVersion,
    SessionUser,
    SessionHostname,
    ExternalCommandsState,
    RequiredExecutablesPresence,
    /// A per-chip monotonic counter that increments each time a user command matching
    /// the chip's `invalidate_on_commands` list completes, causing the fingerprint to change.
    InvalidatingCommandCount,
}

/// Configuration that governs how a chip interacts with the runtime environment: which
/// executables it requires, whether it's restricted to local sessions, its shell command
/// timeout, and which inputs form its cache fingerprint.
#[derive(Clone, Debug, Default)]
pub struct ChipRuntimePolicy {
    required_executables: Vec<String>,
    local_only: bool,
    shell_command_timeout: Option<Duration>,
    fingerprint_inputs: Vec<ChipFingerprintInput>,
    /// When true, if the chip's shell command fails (or times out), the chip records the current
    /// fingerprint and skips re-execution on future fetches — including periodic refreshes —
    /// until the fingerprint changes (e.g. branch or directory change).
    suppress_on_failure: bool,
    /// Top-level command names (e.g. `["git", "gh", "gt"]`) whose execution should
    /// invalidate this chip's fingerprint. Pair with `ChipFingerprintInput::InvalidatingCommandCount`.
    invalidate_on_commands: Vec<String>,
}

impl ChipRuntimePolicy {
    pub fn new(
        required_executables: impl IntoIterator<Item = impl Into<String>>,
        local_only: bool,
        shell_command_timeout: Option<Duration>,
        fingerprint_inputs: impl IntoIterator<Item = ChipFingerprintInput>,
    ) -> Self {
        Self {
            required_executables: required_executables.into_iter().map(Into::into).collect(),
            local_only,
            shell_command_timeout,
            fingerprint_inputs: fingerprint_inputs.into_iter().collect(),
            suppress_on_failure: false,
            invalidate_on_commands: Vec::new(),
        }
    }

    pub fn for_shell_generator(generator: &ShellCommandGenerator) -> Self {
        Self::new(
            generator.dependencies().to_vec(),
            false,
            None,
            std::iter::empty(),
        )
    }

    pub fn required_executables(&self) -> &[String] {
        &self.required_executables
    }

    pub fn shell_command_timeout(&self) -> Option<Duration> {
        self.shell_command_timeout
    }

    pub fn fingerprint_inputs(&self) -> &[ChipFingerprintInput] {
        &self.fingerprint_inputs
    }

    pub fn suppress_on_failure(&self) -> bool {
        self.suppress_on_failure
    }

    pub fn with_suppress_on_failure(mut self) -> Self {
        self.suppress_on_failure = true;
        self
    }

    pub fn invalidate_on_commands(&self) -> &[String] {
        &self.invalidate_on_commands
    }

    pub fn with_invalidate_on_commands(
        mut self,
        commands: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.invalidate_on_commands = commands.into_iter().map(Into::into).collect();
        self
    }

    pub fn availability(&self, capabilities: &ChipRuntimeCapabilities) -> ChipAvailability {
        if self.local_only && matches!(capabilities.session_is_local, Some(false)) {
            return ChipAvailability::Disabled(ChipDisabledReason::RequiresLocalSession);
        }

        for command in &self.required_executables {
            if matches!(
                capabilities.external_commands.contains(command),
                Some(false)
            ) {
                return ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
                    command: command.clone(),
                });
            }
        }

        ChipAvailability::Enabled
    }
}

/// Context for built-in contextual [`PromptGenerator`]s.
pub struct GeneratorContext<'a> {
    /// The latest block in the session. While the prompt is shown, this block should have precmd
    /// metadata but will not have executed yet.
    pub active_block_metadata: &'a BlockMetadata,
    /// The session that the active block is part of. This should always be available once the
    /// session is bootstrapped. However, it may be missing due to errors restoring previous
    /// sessions or extracting session info from the shell.
    pub active_session: Option<&'a Session>,
    /// The most-recently-available environment data for the terminal session. Unlike the username,
    /// hostname, and other session-level info, this can change over the lifetime of a session -
    /// users can activate/deactivate virtualenvs, change branches, and so on.
    pub current_environment: &'a Environment,
}

/// Environment information for a terminal session.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// The Git branch that's checked out.
    git_branch: Option<String>,
    /// The name of the active Python virtual environment.
    python_virtualenv: Option<String>,
    /// The Anaconda environment name.
    conda_environment: Option<String>,
    /// The Node.js version.
    node_version: Option<String>,
}

#[derive(Clone, Debug)]
pub enum PromptGenerator {
    ShellCommand(ShellCommandGenerator),
    Contextual {
        /// A function that extracts the chip value from the prompt-generation context.
        from_context_fn: fn(&GeneratorContext) -> Option<ChipValue>,
    },
}

#[derive(Clone, Debug, Default)]
pub enum RefreshConfig {
    #[default]
    OnDemandOnly,
    #[allow(dead_code)]
    Periodically { interval: Duration },
    #[allow(dead_code)]
    OnFileChanges { filepath: String },
}

#[derive(Clone, Debug)]
pub struct ContextChip {
    title: String,
    generator: PromptGenerator,
    on_click_generator: Option<PromptGenerator>,
    /// TODO: this likely needs to move to a config state.
    icon_path: Option<&'static str>,
    refresh_config: RefreshConfig,
    runtime_policy: ChipRuntimePolicy,
    /// When `true`, a shell command that succeeds with empty output produces `Some("")` instead of `None`.
    /// Useful for chips like `GitDiffStats` where empty output can still mean
    /// a valid and clean working tree rather than "not applicable".
    allow_empty_value: bool,
}

impl ContextChip {
    /// Create a new built-in context chip using the given generator function.
    pub fn builtin(
        title: impl Into<String>,
        generator: fn(&GeneratorContext) -> Option<ChipValue>,
        refresh_config: RefreshConfig,
    ) -> Self {
        Self::builtin_with_runtime_policy(
            title,
            generator,
            refresh_config,
            ChipRuntimePolicy::default(),
        )
    }

    pub fn builtin_with_runtime_policy(
        title: impl Into<String>,
        generator: fn(&GeneratorContext) -> Option<ChipValue>,
        refresh_config: RefreshConfig,
        runtime_policy: ChipRuntimePolicy,
    ) -> Self {
        Self {
            title: title.into(),
            generator: PromptGenerator::Contextual {
                from_context_fn: generator,
            },
            on_click_generator: None,
            icon_path: None,
            refresh_config,
            runtime_policy,
            allow_empty_value: false,
        }
    }

    pub fn shell_builtin(
        title: impl Into<String>,
        generator: ShellCommandGenerator,
        on_click_generator: Option<ShellCommandGenerator>,
        refresh_config: RefreshConfig,
    ) -> Self {
        let runtime_policy = ChipRuntimePolicy::for_shell_generator(&generator);
        Self::shell_builtin_with_runtime_policy(
            title,
            generator,
            on_click_generator,
            refresh_config,
            runtime_policy,
        )
    }

    pub fn shell_builtin_with_runtime_policy(
        title: impl Into<String>,
        generator: ShellCommandGenerator,
        on_click_generator: Option<ShellCommandGenerator>,
        refresh_config: RefreshConfig,
        runtime_policy: ChipRuntimePolicy,
    ) -> Self {
        Self {
            title: title.into(),
            generator: PromptGenerator::ShellCommand(generator),
            on_click_generator: on_click_generator.map(PromptGenerator::ShellCommand),
            icon_path: None,
            refresh_config,
            runtime_policy,
            allow_empty_value: false,
        }
    }

    pub fn new_custom_chip(title: String, shell_command_generator: ShellCommandGenerator) -> Self {
        let runtime_policy = ChipRuntimePolicy::for_shell_generator(&shell_command_generator);
        Self {
            title,
            generator: PromptGenerator::ShellCommand(shell_command_generator),
            on_click_generator: None,
            icon_path: None,
            refresh_config: Default::default(),
            runtime_policy,
            allow_empty_value: false,
        }
    }

    pub fn title(&self) -> &str {
        self.title.as_str()
    }

    pub fn generator(&self) -> &PromptGenerator {
        &self.generator
    }

    pub fn on_click_generator(&self) -> Option<&PromptGenerator> {
        self.on_click_generator.as_ref()
    }

    pub fn refresh_config(&self) -> &RefreshConfig {
        &self.refresh_config
    }

    pub fn runtime_policy(&self) -> &ChipRuntimePolicy {
        &self.runtime_policy
    }

    pub fn availability(&self, capabilities: &ChipRuntimeCapabilities) -> ChipAvailability {
        self.runtime_policy.availability(capabilities)
    }

    pub fn icon_path(&self) -> Option<&'static str> {
        self.icon_path
    }

    pub fn allow_empty_value(&self) -> bool {
        self.allow_empty_value
    }

    pub fn with_allow_empty_value(mut self) -> Self {
        self.allow_empty_value = true;
        self
    }
}

impl Environment {
    /// Create a new environment with the given values.
    pub fn new(
        python_virtualenv: Option<String>,
        conda_environment: Option<String>,
        node_version: Option<String>,
    ) -> Self {
        Self {
            git_branch: None,
            python_virtualenv,
            conda_environment,
            node_version,
        }
    }

    /// Snapshot the environment from a block.
    pub fn from_block(block: &Block) -> Self {
        Self {
            git_branch: block.git_branch().cloned(),
            python_virtualenv: block.virtual_env_short_name(),
            conda_environment: block.conda_env().cloned(),
            node_version: block.node_version().cloned(),
        }
    }

    pub fn git_branch(&self) -> Option<&String> {
        self.git_branch.as_ref()
    }

    pub fn python_virtualenv(&self) -> Option<&String> {
        self.python_virtualenv.as_ref()
    }

    pub fn conda_environment(&self) -> Option<&String> {
        self.conda_environment.as_ref()
    }

    pub fn node_version(&self) -> Option<&String> {
        self.node_version.as_ref()
    }
}
