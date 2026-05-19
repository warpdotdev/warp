//! This module defines the schema for DCS hooks sent from the shell to the Rust
//! app -- for example, the payloads sent from shell precmd and preexec.
use crate::terminal::model::block::BlockId;
use crate::terminal::model::session::SessionId;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use warp_core::command::ExitCode;

/// Indicates that the following JSON-encoded message is hex-encoded for Warp's lifecycle hooks.
/// In DCS, it is used as the final char in the DCS start sequence.
/// In OSC, it is used as the first parameter.
pub(super) const HEX_ENCODED_JSON_MARKER: char = 'd';

/// Indicates that the following JSON-encoded message is unencoded for Warp's lifecycle hooks.
/// In DCS, it is used as the final char in the DCS start sequence.
/// In OSC, it is used as the first parameter.
pub(super) const UNENCODED_JSON_MARKER: char = 'f';

/// Indicates that the following message is a ANSI-C quoted message for receiving Warp's lifecycle
/// hooks via key-value pairs.
/// In OSC< it is used as the first parameter.
pub(super) const UNENCODED_KV_MARKER: char = 'k';

/// Enum representing all possible JSON payloads for Warp's DCS's.
#[derive(Serialize, Debug, Deserialize)]
#[allow(clippy::upper_case_acronyms)]
#[serde(tag = "hook")]
pub(super) enum DProtoHook {
    CommandFinished {
        value: CommandFinishedValue,
    },
    Precmd {
        value: PrecmdValue,
    },
    Preexec {
        value: PreexecValue,
    },
    Bootstrapped {
        // This is wrapped in an `Box` to suppress clippy's large-enum-variant warning, not because it
        // functionally needs to be wrapped in an `Box`.
        value: Box<BootstrappedValue>,
    },
    PreInteractiveSSHSession {
        value: PreInteractiveSSHSessionValue,
    },
    SSH {
        value: SSHValue,
    },
    InitShell {
        value: InitShellValue,
    },
    InputBuffer {
        value: InputBufferValue,
    },
    Clear {
        value: ClearValue,
    },
    InitSubshell {
        value: InitSubshellValue,
    },
    SourcedRcFileForWarp {
        value: SourcedRcFileForWarpValue,
    },
    InitSsh {
        value: InitSshValue,
    },
    FinishUpdate {
        value: FinishUpdateValue,
    },
    RemoteWarpificationIsUnavailable {
        // If a value is provided, it's suggesting a way to install TMUX on the remote.
        value: WarpificationUnavailableReason,
    },
    SshTmuxInstaller {
        value: String,
    },
    TmuxInstallFailed {
        value: TmuxInstallFailedInfo,
    },
    ExitShell {
        value: ExitShellValue,
    },
}

impl DProtoHook {
    pub fn name(&self) -> &'static str {
        match self {
            DProtoHook::CommandFinished { .. } => "CommandFinished",
            DProtoHook::Precmd { .. } => "Precmd",
            DProtoHook::Preexec { .. } => "Preexec",
            DProtoHook::Bootstrapped { .. } => "Bootstrapped",
            DProtoHook::PreInteractiveSSHSession { .. } => "PreInteractiveSSHSession",
            DProtoHook::SSH { .. } => "SSH",
            DProtoHook::InitShell { .. } => "InitShell",
            DProtoHook::InputBuffer { .. } => "InputBuffer",
            DProtoHook::Clear { .. } => "Clear",
            DProtoHook::InitSubshell { .. } => "InitSubshell",
            DProtoHook::SourcedRcFileForWarp { .. } => "SourcedRcFileForWarp",
            DProtoHook::InitSsh { .. } => "InitSsh",
            DProtoHook::FinishUpdate { .. } => "FinishUpdate",
            DProtoHook::RemoteWarpificationIsUnavailable { .. } => {
                "RemoteWarpificationIsUnavailable"
            }
            DProtoHook::SshTmuxInstaller { .. } => "SshTmuxInstaller",
            DProtoHook::TmuxInstallFailed { .. } => "TmuxInstallFailed",
            DProtoHook::ExitShell { .. } => "ExitShell",
        }
    }

    /// This function exists because there doesn't yet exist meaningful defaults for all shell
    /// hooks.
    pub fn default_from_name(hook: &str) -> Option<Self> {
        match hook {
            "CommandFinished" => Some(DProtoHook::CommandFinished {
                value: Default::default(),
            }),
            "Precmd" => Some(DProtoHook::Precmd {
                value: Default::default(),
            }),
            "Preexec" => Some(DProtoHook::Preexec {
                value: Default::default(),
            }),
            "Bootstrapped" => Some(DProtoHook::Bootstrapped {
                value: Default::default(),
            }),
            "PreInteractiveSSHSession" => Some(DProtoHook::PreInteractiveSSHSession {
                value: Default::default(),
            }),
            "SSH" => Some(DProtoHook::SSH {
                value: Default::default(),
            }),
            "InitShell" => Some(DProtoHook::InitShell {
                value: Default::default(),
            }),
            "InputBuffer" => Some(DProtoHook::InputBuffer {
                value: Default::default(),
            }),
            "Clear" => Some(DProtoHook::Clear {
                value: Default::default(),
            }),
            "InitSubshell" => Some(DProtoHook::InitSubshell {
                value: Default::default(),
            }),
            "SourcedRcFileForWarp" => Some(DProtoHook::SourcedRcFileForWarp {
                value: Default::default(),
            }),
            "InitSsh" => Some(DProtoHook::InitSsh {
                value: Default::default(),
            }),
            "FinishUpdate" => Some(DProtoHook::FinishUpdate {
                value: Default::default(),
            }),
            "SshTmuxInstaller" => Some(DProtoHook::SshTmuxInstaller {
                value: Default::default(),
            }),
            "TmuxInstallFailed" => Some(DProtoHook::TmuxInstallFailed {
                value: Default::default(),
            }),
            "ExitShell" => Some(DProtoHook::ExitShell {
                value: Default::default(),
            }),
            _ => {
                debug_assert!(
                    false,
                    "We do not yet support receiving the {hook} hook via key-value pairs"
                );
                None
            }
        }
    }

    /// Populates a field of the hook's `value` with the given key-value pair.
    pub fn populate_field(&mut self, key: String, v: String) {
        let map_empty_to_none = |s: String| {
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        };
        match self {
            DProtoHook::CommandFinished { value } => match key.as_ref() {
                "exit_code" => value.exit_code = v.parse::<i32>().unwrap_or_default().into(),
                "next_block_id" => {
                    value.next_block_id = v.to_string().into();
                }
                _ => {
                    log::warn!("Tried to add unknown field to CommandFinished");
                }
            },
            DProtoHook::Precmd { value } => match key.as_ref() {
                "pwd" => {
                    value.pwd = map_empty_to_none(v);
                }
                "ps1" => {
                    value.ps1 = map_empty_to_none(v);
                }
                "ps1_is_encoded" => value.ps1_is_encoded = v.parse::<bool>().ok(),
                "honor_ps1" => value.honor_ps1 = v.parse::<bool>().ok(),
                "rprompt" => {
                    value.rprompt = map_empty_to_none(v);
                }
                "git_head" => {
                    value.git_head = map_empty_to_none(v);
                }
                "git_branch" => {
                    value.git_branch = map_empty_to_none(v);
                }
                "virtual_env" => {
                    value.virtual_env = map_empty_to_none(v);
                }
                "conda_env" => {
                    value.conda_env = map_empty_to_none(v);
                }
                "kube_config" => {
                    value.kube_config = map_empty_to_none(v);
                }
                "session_id" => value.session_id = v.parse::<u64>().ok(),
                _ => {
                    log::warn!("Tried to add unknown field {key} to Precmd");
                }
            },
            DProtoHook::InitShell { value } => match key.as_ref() {
                "session_id" => {
                    value.session_id = v
                        .parse::<u64>()
                        .ok()
                        .map(|id| id.into())
                        .unwrap_or_default()
                }
                "shell" => {
                    value.shell = v;
                }
                "is_subshell" => {
                    value.is_subshell = v.parse::<bool>().ok().unwrap_or_default();
                }
                "user" => {
                    value.user = trim_null_byte(v);
                }
                "hostname" => {
                    value.hostname = trim_null_byte(v);
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to InitShell");
                }
            },
            DProtoHook::Bootstrapped { value } => match key.as_ref() {
                "histfile" => {
                    value.histfile = map_empty_to_none(v);
                }
                "shell" => value.shell = trim_null_byte(v),
                "home_dir" => value.home_dir = map_empty_to_none(v),
                "path" => {
                    value.path = map_empty_to_none(v);
                }
                "cdpath" => {
                    value.cdpath = map_empty_to_none(v);
                }
                "editor" => {
                    value.editor = map_empty_to_none(v);
                }
                "aliases" => {
                    value.aliases = map_empty_to_none(v);
                }
                "abbreviations" => {
                    value.abbreviations = map_empty_to_none(v);
                }
                "function_names" => {
                    value.function_names = map_empty_to_none(v);
                }
                "env_var_names" => {
                    value.env_var_names = map_empty_to_none(v);
                }
                "builtins" => {
                    value.builtins = map_empty_to_none(v);
                }
                "keywords" => {
                    value.keywords = map_empty_to_none(v);
                }
                "shell_version" => {
                    value.shell_version = map_empty_to_none(v);
                }
                "shell_options" => {
                    value.shell_options = Some(parse_shell_options_list(v));
                }
                "rcfiles_start_time" => {
                    value.rcfiles_start_time = parse_float_from_string(v);
                }
                "rcfiles_end_time" => {
                    value.rcfiles_end_time = parse_float_from_string(v);
                }
                "shell_plugins" => {
                    value.shell_plugins = Some(parse_shell_options_list(v));
                }
                "vi_mode_enabled" => {
                    value.vi_mode_enabled = map_empty_to_none(v);
                }
                "os_category" => {
                    value.os_category = map_empty_to_none(v);
                }
                "linux_distribution" => {
                    value.linux_distribution = map_empty_to_none(v);
                }
                "wsl_name" => {
                    value.wsl_name = map_empty_to_none(v);
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to Bootstrapped hook");
                }
            },
            DProtoHook::Preexec { value } => match key.as_ref() {
                "command" => {
                    value.command = v;
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to Preexec hook");
                }
            },
            DProtoHook::Clear { .. } => {
                log::warn!("Tried to add unknown field {key} to Clear hook");
            }
            DProtoHook::FinishUpdate { value } => match key.as_ref() {
                "update_id" => {
                    value.update_id = v;
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to FinishUpdate hook");
                }
            },
            DProtoHook::InputBuffer { value } => match key.as_ref() {
                "buffer" => {
                    value.buffer = v;
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to InputBuffer hook");
                }
            },
            DProtoHook::ExitShell { value } => match key.as_ref() {
                "session_id" => {
                    value.session_id = v.parse::<u64>().ok().map(Into::into).unwrap_or_default()
                }
                _ => {
                    log::warn!("Tried to add unknown field {key} to ExitShell hook");
                }
            },
            _ => {
                debug_assert!(
                    false,
                    "Populating fields of the {} hook is not yet supported via key-value pairs",
                    self.name()
                );
            }
        }
    }
}

/// Details that help us determine which, if any, of our TMUX install scripts
/// we should suggest to the user.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq, Eq)]
pub struct SystemDetails {
    #[serde(alias = "os")]
    pub operating_system: String,
    #[serde(alias = "pkg")]
    pub package_manager: String,
    pub shell: String,
    /// Is the user's home directory writable? This is None if we haven't gathered that
    /// information.
    pub writable_home: Option<bool>,
}

/// The reason that warpification was not available when the user tried
/// to warpify.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all(serialize = "snake_case"))]
pub enum WarpificationUnavailableReason {
    TmuxFailed,
    UnsupportedTmuxVersion {
        #[serde(flatten)]
        system_details: SystemDetails,
    },
    TmuxNotInstalled {
        #[serde(flatten)]
        system_details: SystemDetails,
        root_access: String,
    },
    UnsupportedShell {
        shell_name: String,
    },
    Timeout {
        is_tmux_install: bool,
        is_shell_detection: bool,
        #[serde(flatten)]
        system_details: Option<SystemDetails>,
    },
    TmuxInstallFailed {
        #[serde(flatten)]
        system_details: Option<SystemDetails>,
        line: Option<String>,
        command: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TmuxInstallFailedInfo {
    pub line: String,
    pub command: String,
}
/// Received from the pty when a command has finished executing.
#[derive(Debug, Deserialize, Default, Serialize, PartialEq, Eq)]
pub struct CommandFinishedValue {
    pub exit_code: ExitCode,
    pub next_block_id: BlockId,
}

/// Received from the pty at precmd.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrecmdValue {
    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub pwd: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub ps1: Option<String>,

    pub ps1_is_encoded: Option<bool>,

    pub honor_ps1: Option<bool>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub rprompt: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub git_head: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub git_branch: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub virtual_env: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub conda_env: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub node_version: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub kube_config: Option<String>,

    pub session_id: Option<u64>,

    /// Whether this PrecmdValue was emitted after the completion of an in-band command.
    #[serde(default)]
    pub is_after_in_band_command: bool,
}

impl PrecmdValue {
    /// Returns `true` if this PrecmdValue was emitted after the completion of an in-band command.
    ///
    /// This relies on the assumption that the warp_precmd shell function (responsible for writing
    /// this to the PTY from the shell) does not populate `pwd` or `ps1` when the previous command
    /// was an in-band command; for all other cases these fields should always be populated.
    pub fn was_sent_after_in_band_command(&self) -> bool {
        self.is_after_in_band_command || (self.pwd.is_none() && self.ps1.is_none())
    }
}

impl Default for PrecmdValue {
    fn default() -> Self {
        Self {
            pwd: Default::default(),
            ps1: Default::default(),
            // By default, we assume that we are using hex-encoding.
            ps1_is_encoded: Some(true),
            honor_ps1: Default::default(),
            rprompt: Default::default(),
            git_head: Default::default(),
            git_branch: Default::default(),
            virtual_env: Default::default(),
            conda_env: Default::default(),
            node_version: Default::default(),
            kube_config: Default::default(),
            session_id: Default::default(),
            is_after_in_band_command: Default::default(),
        }
    }
}

/// Received from the pty at preexec.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PreexecValue {
    /// The command for which this preexec hook is emitted.
    ///
    /// For Bash specifically, this command may not be the entire command string (it will only
    /// include up to the first job control indicator, e.g. '|', '&&'). This is due to a
    /// shortcoming of the bash_preexec library we use to simulate preexec hooks in bash.
    pub command: String,
}

/// Received from the pty after the shell has finished executing Warp's
/// bootstrap script.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct BootstrappedValue {
    #[serde(deserialize_with = "empty_string_is_none")]
    pub histfile: Option<String>,

    #[serde(deserialize_with = "trim_null_byte_deserializer", default)]
    pub shell: String,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub home_dir: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub path: Option<String>,

    /// `CDPATH` from the shell, colon-separated.
    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub cdpath: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub editor: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub aliases: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub abbreviations: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub function_names: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub env_var_names: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub builtins: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub keywords: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none")]
    pub shell_version: Option<String>,

    /// A list of options enabled for the shell by the end of bootstrap.  Will
    /// be None if the shell doesn't support listing options via builtin.
    #[serde(deserialize_with = "parse_shell_options_list_deserializer", default)]
    pub shell_options: Option<HashSet<String>>,

    /// The time at which we started sourcing the user rcfiles, measured in
    /// seconds since epoch.
    #[serde(deserialize_with = "parse_float_from_string_deserializer", default)]
    pub rcfiles_start_time: Option<OrderedFloat<f64>>,

    /// The time at which we finished sourcing the user rcfiles, measured in
    /// seconds since epoch.
    #[serde(deserialize_with = "parse_float_from_string_deserializer", default)]
    pub rcfiles_end_time: Option<OrderedFloat<f64>>,

    /// Tags for known shell configurations/plugins, especially ones that are
    /// incompatible with Warp.
    #[serde(deserialize_with = "parse_shell_options_list_deserializer", default)]
    pub shell_plugins: Option<HashSet<String>>,

    /// Whether the shell's native vi mode implementation is on.
    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub vi_mode_enabled: Option<String>,

    /// The operating system category (e.g. MacOS, Linux, Windows).
    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub os_category: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub linux_distribution: Option<String>,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub wsl_name: Option<String>,

    /// The full path to the running shell binary (e.g. "/usr/bin/zsh").
    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub shell_path: Option<String>,
}

/// Custom serde deserializer that parses a float from a string.
fn parse_float_from_string_deserializer<'de, D>(
    deserializer: D,
) -> Result<Option<OrderedFloat<f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        s.parse::<f64>().map_err(serde::de::Error::custom)?.into(),
    ))
}

fn parse_float_from_string(s: String) -> Option<OrderedFloat<f64>> {
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().map(|f| f.into()).ok()
}

/// Received from the pty when Warp's SSH wrapper is executed, prior to
/// bootstrapping the SSH session.
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize, Clone)]
pub struct PreInteractiveSSHSessionValue {}

/// Received from the pty after establishing an SSH connection, prior to
/// bootstrapping the session.
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize, Clone)]
pub struct SSHValue {
    pub socket_path: PathBuf,
    pub remote_shell: String,
}

/// Received from the pty after the shell session has been initialized, marking
/// the shell ready to execute the bootstrap script.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InitShellValue {
    pub session_id: SessionId,

    pub shell: String,

    #[serde(default)]
    pub is_subshell: bool,

    #[serde(deserialize_with = "trim_null_byte_deserializer", default)]
    pub user: String,

    #[serde(deserialize_with = "trim_null_byte_deserializer", default)]
    pub hostname: String,

    #[serde(deserialize_with = "empty_string_is_none", default)]
    pub wsl_name: Option<String>,
}

/// Emitted as part of the new ssh session bootstrapping process.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InitSshValue {
    pub shell: String,
    pub uname: Option<String>,
}

/// Emitted as part of the tmux bootstrapping process.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InitSubshellValue {
    pub shell: String,
    pub uname: Option<String>,
}

/// Emitted by a snippet included in the user's RC file, which signals a new session is being
/// created; if the session is for a subshell, this triggers Warp's bootstrap process.
/// Otherwise, it's ignored.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourcedRcFileForWarpValue {
    pub shell: String,
    pub uname: Option<String>,
    pub tmux: Option<bool>,
}

/// Received from the pty via a shell line editor hook, whether readline (bash),
/// ZLE, or the fish [command line editor](https://fishshell.com/docs/current/interactive.html#command-line-editor).
/// The binding is triggered when Warp writes the `ESC-i` escape sequence to the pty.
/// Warp usually does this after a block completes, to collect any typeahead
/// that the user entered while the block was running (see
/// [`TerminalView::request_input_buffer`]).
#[derive(Debug, Default, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct InputBufferValue {
    pub buffer: String,
}

/// Received from the pty when the terminal screen should be cleared (e.g. via
/// the `clear` command or ctrl-l).
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ClearValue {}

/// Received from the pty when warp_finish_update is called at the end of an
/// assisted auto-update.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FinishUpdateValue {
    pub update_id: String,
}

/// Received from the pty right before the remote shell exits (via `exit`,
/// `logout`, Ctrl-D on an empty prompt, etc.). Lets the Warp client drop
/// per-session resources — in particular the `ssh … remote-server-proxy`
/// child process that holds a multiplexed channel on the foreground ssh
/// ControlMaster — before the user's outer ssh tunnel tries to close, so
/// the master can exit cleanly instead of hanging on orphaned slaves.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExitShellValue {
    pub session_id: SessionId,
}

/// Custom serde deserializer that trims trailing null bytes.
fn trim_null_byte_deserializer<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let trimmed = trim_null_byte(s);
    Ok(trimmed)
}

fn trim_null_byte(s: String) -> String {
    s.trim().trim_end_matches(char::from(0)).to_owned()
}

/// Custom serde deserializer that maps empty strings to none.
fn empty_string_is_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let trimmed = trim_null_byte(s);
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

fn parse_shell_options_list_deserializer<'de, D>(
    deserializer: D,
) -> Result<Option<HashSet<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let options: HashSet<String> = parse_shell_options_list(s);
    Ok(Some(options))
}

fn parse_shell_options_list(s: String) -> HashSet<String> {
    s.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(Into::into)
        .collect()
}

/// Represents a shell hook that will be constructed over time by receiving key-value pairs.
#[derive(Debug, Serialize)]
pub struct PendingHook {
    hook: DProtoHook,
}

impl PendingHook {
    pub fn create(hook_name: &str) -> Option<Self> {
        DProtoHook::default_from_name(hook_name).map(|hook| Self { hook })
    }

    /// Updates the field on the hook according to the given key-value pair.
    pub fn update(&mut self, key: String, mut value: String) {
        if super::is_ansi_c_quoted(&value) {
            value = super::parse_ansi_c_quoted_string(value);
        }
        self.hook.populate_field(key, value);
    }

    pub(super) fn finish(self) -> DProtoHook {
        self.hook
    }
}
