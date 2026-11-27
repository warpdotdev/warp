#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, fmt};

use clap::{CommandFactory, FromArgMatches as _, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use url::Url;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;

use crate::agent::OutputFormat;

#[cfg(windows)]
mod process_handle;

pub mod artifact;
pub mod scope;
pub mod skill;
mod sort_order;
pub use sort_order::SortOrderArg;

pub mod agent;
pub mod api_key;
pub mod completions;
pub mod computer_use_mcp;
pub mod config_file;
mod date_time;
pub mod environment;
pub mod environment_checkout;
pub mod federate;
pub mod harness_support;
pub mod integration;
pub mod json_filter;
pub mod local_control;
pub mod mcp;
pub mod memory_store;
pub mod model;
pub mod provider;
pub mod runner;
pub mod schedule;
pub mod secret;
pub mod share;
pub mod task;
// Each of these variables is injected under both its `OZ_` and its `WARP_` name, carrying the
// identical value. Read sites still use the `OZ_` names.
pub const OZ_RUN_ID_ENV: &str = "OZ_RUN_ID";
pub const WARP_RUN_ID_ENV: &str = "WARP_RUN_ID";
pub const OZ_PARENT_RUN_ID_ENV: &str = "OZ_PARENT_RUN_ID";
pub const WARP_PARENT_RUN_ID_ENV: &str = "WARP_PARENT_RUN_ID";
pub const OZ_CLI_ENV: &str = "OZ_CLI";
pub const WARP_CLI_ENV: &str = "WARP_CLI";
pub const OZ_HARNESS_ENV: &str = "OZ_HARNESS";
pub const WARP_HARNESS_ENV: &str = "WARP_HARNESS";
pub const SERVER_ROOT_URL_OVERRIDE_ENV: &str = "WARP_SERVER_ROOT_URL";
pub const WS_SERVER_URL_OVERRIDE_ENV: &str = "WARP_WS_SERVER_URL";
pub const SESSION_SHARING_SERVER_URL_OVERRIDE_ENV: &str = "WARP_SESSION_SHARING_SERVER_URL";
pub const PREWARMED_SANDBOX_CONTROL_URL_OVERRIDE_ENV: &str = "WARP_PREWARMED_SANDBOX_CONTROL_URL";
pub const WARP_PREWARMED_SANDBOX_ID_ENV: &str = "WARP_PREWARMED_SANDBOX_ID";

/// Options related to the parent process that spawned this Warp instance.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct ParentOpts {
    /// The ID of the Warp process that spawned this one.
    ///
    /// Used by codepaths that attempt to detect when the parent Warp process
    /// has terminated. Guaranteed to be [`None`] when this is the initial
    /// Warp process, but may also be [`None`] for Warp child processes if the
    /// child process doesn't need to keep track of its parent.
    #[arg(long = "parent-pid", hide = true)]
    pub pid: Option<u32>,

    /// A handle to our parent process.
    ///
    /// Used on Windows for crash recovery instead of parent_pid, as process
    /// IDs can be reused, so a process handle is more robust.
    #[cfg(windows)]
    #[arg(long = "parent-handle", hide = true)]
    pub handle: Option<process_handle::ProcessHandle>,
}

/// Returns whether an argument requests one of Warp's hidden worker modes.
pub fn is_worker_invocation(arg: &str) -> bool {
    let command = WorkerCommand::augment_subcommands(clap::Command::new("worker"));
    command.find_subcommand(arg).is_some()
        || arg.strip_prefix("--").is_some_and(|long_flag| {
            command
                .get_subcommands()
                .any(|subcommand| subcommand.get_long_flag() == Some(long_flag))
        })
}

/// Hidden worker args used to scope remote-server proxy/daemon sockets by
/// Warp identity without exposing credentials.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct RemoteServerIdentityArgs {
    /// Non-secret identity partition key for the remote-server daemon.
    #[arg(long = "identity-key", hide = true)]
    pub identity_key: String,
}

/// Global options that apply to all CLI commands.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct GlobalOptions {
    /// API key for server authentication.
    #[arg(
        long = "api-key",
        global = true,
        env = "WARP_API_KEY",
        hide_env_values = true
    )]
    pub api_key: Option<String>,

    /// Set the output format.
    #[arg(
        long = "output-format",
        global = true,
        value_enum,
        default_value_t = OutputFormat::Pretty,
        env = "WARP_OUTPUT_FORMAT"
    )]
    pub output_format: OutputFormat,
}

/// Normal argument parser for the shared Warp executable across all channels.
///
/// Oz commands are subcommands of this parser, so invoking an `oz` symlink does
/// not require a mode flag. Warp Control uses its separate [`local_control::ControlArgs`]
/// parser, selected before this parser sees the arguments.
#[derive(Debug, Default, Parser, Clone)]
#[command(
    name = "oz",
    display_name = "Oz",
    about = r#"The orchestration platform for cloud agents

The Oz CLI is a tool for running, managing, and orchestrating coding agents at scale.
Use the CLI to:
* Launch and inspect cloud agents
* Schedule cloud agents to run in the future
* Manage the environments that cloud agents run in
* Upload secrets to Oz's secure storage"#
)]
#[clap(subcommand_precedence_over_arg = true)]
pub struct Args {
    #[clap(flatten)]
    global_options: GlobalOptions,

    /// Enable debug mode.
    #[arg(long = "debug", global = true, help = "Enable debug logging")]
    debug: bool,

    /// Override the server root URL.
    #[arg(
        long = "server-root-url",
        global = true,
        hide = true,
        env = "WARP_SERVER_ROOT_URL"
    )]
    server_root_url: Option<String>,

    /// Override the websocket server URL.
    #[arg(
        long = "ws-server-url",
        global = true,
        hide = true,
        env = "WARP_WS_SERVER_URL"
    )]
    ws_server_url: Option<String>,

    /// Override the session sharing server URL.
    #[arg(
        long = "session-sharing-server-url",
        global = true,
        hide = true,
        env = "WARP_SESSION_SHARING_SERVER_URL"
    )]
    session_sharing_server_url: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    #[clap(flatten)]
    args: AppArgs,
}

/// Command and arguments to execute in an externally launched terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalCommandSpec {
    /// Program and arguments passed after `-e` or `--command`.
    Argv(Vec<String>),
    /// Raw `cmd.exe` command string passed after `/c` on Windows.
    WindowsCmdLine(String),
}

/// Options for opening a local terminal session and optionally executing an initial command.
#[derive(Debug, Default, clap::Args, Clone, PartialEq, Eq)]
pub struct ExternalSessionArgs {
    /// Initial working directory for the terminal session.
    #[arg(long = "working-directory", visible_alias = "cwd", value_name = "DIR")]
    pub working_directory: Option<PathBuf>,

    /// Open the session in a new window instead of a new tab in an existing window.
    #[arg(long = "new-window")]
    pub new_window: bool,

    /// Custom title for the opened tab.
    #[arg(
        short = 'T',
        long = "title",
        allow_hyphen_values = true,
        value_name = "TITLE"
    )]
    pub title: Option<String>,

    /// Keep the terminal session open after the command finishes.
    #[arg(
        long = "wait-after-command",
        num_args = 0..=1,
        default_missing_value = "true",
        value_name = "BOOL"
    )]
    pub wait_after_command: Option<bool>,

    /// Execute a command with arguments in the opened terminal session.
    #[arg(
        short = 'e',
        long = "command",
        num_args = 1..,
        allow_hyphen_values = true,
        value_name = "COMMAND"
    )]
    pub command: Vec<String>,

    /// Parsed command specification populated from `-e` / `--command` or Windows `/c`.
    #[clap(skip)]
    pub command_spec: Option<ExternalCommandSpec>,
}

impl ExternalSessionArgs {
    /// Returns `true` if any external session launch option or command was specified.
    pub fn has_session_options(&self) -> bool {
        self.command_spec.is_some()
            || !self.command.is_empty()
            || self.working_directory.is_some()
            || self.new_window
            || self.title.is_some()
            || self.wait_after_command.is_some()
    }

    /// Returns the resolved command specification, if a command was requested.
    pub fn resolved_command_spec(&self) -> Option<ExternalCommandSpec> {
        self.command_spec.clone().or_else(|| {
            (!self.command.is_empty()).then(|| ExternalCommandSpec::Argv(self.command.clone()))
        })
    }
}

/// Flags for the Warp application. Additional binaries, like test runners, may use this type
/// along with their own flags, or convert their flags into an `AppArgs` value.
#[derive(Debug, Default, clap::Args, Clone)]
pub struct AppArgs {
    /// True if this instance of Warp was launched at the end of the auto-update process.
    #[arg(long = "finish-update", hide = true)]
    pub finish_update: bool,

    /// Crash recovery mechanism to use if we detect the parent process terminated.
    #[cfg(enable_crash_recovery)]
    #[arg(long = "crash-recovery-mechanism", value_enum, requires = "ParentOpts")]
    pub crash_recovery_mechanism: Option<RecoveryMechanism>,

    /// Options related to the parent process that spawned this Warp instance.
    #[clap(flatten)]
    pub parent: ParentOpts,

    /// Options for launching an external terminal session/command.
    #[clap(flatten)]
    pub external_session: ExternalSessionArgs,

    /// URLs to open in Warp.
    #[arg(hide = true)]
    pub urls: Vec<Url>,
}

impl Args {
    /// Parses command-line arguments from the operating environment. May exit early if arguments
    /// are incorrectly specified.
    pub fn from_env() -> Self {
        cfg_if::cfg_if! {
            // wasm doesn't have any concept of an environment, so skip parsing and return defaults
            if #[cfg(target_family = "wasm")] {
                Args::default()
            } else {
                // Check for disabled commands before parsing to prevent help from showing (e.g.
                // `warp environment` should not return help text)
                if !FeatureFlag::CloudEnvironments.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "environment" {
                        eprintln!("error: unrecognized subcommand 'environment'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::ProviderCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "provider" {
                        eprintln!("error: unrecognized subcommand 'provider'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::IntegrationCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "integration" {
                        eprintln!("error: unrecognized subcommand 'integration'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::ScheduledAmbientAgents.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "schedule" {
                        eprintln!("error: unrecognized subcommand 'schedule'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::WarpManagedSecrets.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "secret" {
                        eprintln!("error: unrecognized subcommand 'secret'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::OzIdentityFederation.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "federate" {
                        eprintln!("error: unrecognized subcommand 'federate'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::ArtifactCommand.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "artifact" {
                        eprintln!("error: unrecognized subcommand 'artifact'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::APIKeyManagement.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "api-key" {
                        eprintln!("error: unrecognized subcommand 'api-key'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                if !FeatureFlag::CloudAgentRunners.is_enabled() {
                    let args: Vec<String> = env::args().collect();
                    if args.len() > 1 && args[1] == "runner" {
                        eprintln!("error: unrecognized subcommand 'runner'\n");
                        eprintln!("For more information, try '--help'");
                        std::process::exit(2);
                    }
                }

                #[cfg(windows)]
                let raw_windows_cmdline = current_win32_command_line();
                #[cfg(not(windows))]
                let raw_windows_cmdline: Option<String> = None;

                Self::try_parse_from_iter_with_raw_cmdline(
                    env::args_os(),
                    cfg!(windows),
                    raw_windows_cmdline.as_deref(),
                )
                .unwrap_or_else(|err| {
                    // We attach a console to ensure help and error messages are printed
                    // when using the CLI.
                    #[cfg(windows)]
                    warp_util::windows::attach_to_parent_console();
                    err.exit()
                })
            }
        }
    }

    /// Parses command-line arguments from an iterator while preserving child command arguments
    /// passed after `-e` / `--command` (and `/c` on Windows) verbatim.
    pub fn try_parse_app_from<I, T>(iter: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        Self::try_parse_from_iter_with_options(iter, cfg!(windows))
    }

    fn try_parse_from_iter_with_options<I, T>(
        iter: I,
        allow_windows_slash_c: bool,
    ) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        Self::try_parse_from_iter_with_raw_cmdline(iter, allow_windows_slash_c, None)
    }

    fn try_parse_from_iter_with_raw_cmdline<I, T>(
        iter: I,
        allow_windows_slash_c: bool,
        raw_windows_cmdline: Option<&str>,
    ) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let raw_args: Vec<OsString> = iter.into_iter().map(Into::into).collect();
        let mut command = Self::clap_command();

        if let Some((prefix_end, split_kind, inline_first_arg)) =
            find_external_command_split(&raw_args, &command, allow_windows_slash_c)
        {
            let mut trailing_args = Vec::new();
            if let Some(first_arg) = inline_first_arg {
                trailing_args.push(first_arg);
            }
            for arg in &raw_args[prefix_end + 1..] {
                trailing_args.push(arg.to_string_lossy().into_owned());
            }

            if trailing_args.is_empty() {
                let flag = match split_kind {
                    ExternalCommandSplitKind::Argv => "`-e` / `--command`",
                    ExternalCommandSplitKind::WindowsCmdLine => "`/c`",
                };
                return Err(command.error(
                    clap::error::ErrorKind::MissingRequiredArgument,
                    format!("expected a command after {flag}"),
                ));
            }

            let matches = command.try_get_matches_from(&raw_args[..prefix_end])?;
            let mut parsed = Self::from_arg_matches(&matches)?;
            match split_kind {
                ExternalCommandSplitKind::Argv => {
                    parsed.args.external_session.command = trailing_args.clone();
                    parsed.args.external_session.command_spec =
                        Some(ExternalCommandSpec::Argv(trailing_args));
                }
                ExternalCommandSplitKind::WindowsCmdLine => {
                    let cmdline = resolve_windows_cmdline_after_slash_c(
                        prefix_end,
                        &trailing_args,
                        raw_windows_cmdline,
                    );
                    parsed.args.external_session.command_spec =
                        Some(ExternalCommandSpec::WindowsCmdLine(cmdline));
                }
            }
            return Ok(parsed);
        }

        let matches = command.try_get_matches_from(raw_args)?;
        let mut parsed = Self::from_arg_matches(&matches)?;
        if parsed.args.external_session.command_spec.is_none()
            && !parsed.args.external_session.command.is_empty()
        {
            parsed.args.external_session.command_spec = Some(ExternalCommandSpec::Argv(
                parsed.args.external_session.command.clone(),
            ));
        }
        Ok(parsed)
    }

    /// Construct the [`clap::Command`] that backs `Args`.
    ///
    /// IMPORTANT: use this instead of [`CommandFactory::command`], since we customize the command at runtime.
    pub fn clap_command() -> clap::Command {
        let mut command = <Args as CommandFactory>::command();

        // Hide the environment subcommands and --environment flags from help text
        if !FeatureFlag::CloudEnvironments.is_enabled() {
            command = command.mut_subcommand("environment", |c| c.hide(true));
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd
                    .mut_subcommand("run", |run_cmd| {
                        run_cmd.mut_arg("environment", |arg| arg.hide(true))
                    })
                    .mut_subcommand("run-cloud", |cloud_cmd| {
                        cloud_cmd.mut_arg("environment", |arg| arg.hide(true))
                    })
            });
        }

        // Hide the --conversation flag from help text
        if !FeatureFlag::CloudConversations.is_enabled() {
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd
                    .mut_subcommand("run", |run_cmd| {
                        run_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
                    .mut_subcommand("run-cloud", |cloud_cmd| {
                        cloud_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
            });
        }

        if !FeatureFlag::AmbientAgentsCommandLine.is_enabled() {
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd.mut_subcommand("run-cloud", |c| c.hide(true))
            });
        }

        // Hide the third-party harness flags on `run-cloud` when the harness
        // feature is off, so `--help` matches the runtime gating (a non-oz
        // `--harness` is rejected unless AgentHarness is enabled).
        if !FeatureFlag::AgentHarness.is_enabled() {
            command = command.mut_subcommand("agent", |agent_cmd| {
                agent_cmd.mut_subcommand("run-cloud", |cloud_cmd| {
                    cloud_cmd
                        .mut_arg("harness", |arg| arg.hide(true))
                        .mut_arg("claude_auth_secret", |arg| arg.hide(true))
                        .mut_arg("codex_auth_secret", |arg| arg.hide(true))
                })
            });
        }

        // Hide the provider subcommand from help text
        if !FeatureFlag::ProviderCommand.is_enabled() {
            command = command.mut_subcommand("provider", |c| c.hide(true));
        }

        // Hide the integration subcommand from help text
        if !FeatureFlag::IntegrationCommand.is_enabled() {
            command = command.mut_subcommand("integration", |c| c.hide(true));
        }

        // Hide the schedule subcommand from help text.
        if !FeatureFlag::ScheduledAmbientAgents.is_enabled() {
            command = command.mut_subcommand("schedule", |c| c.hide(true));
        }

        // Hide the secret subcommand from help text.
        if !FeatureFlag::WarpManagedSecrets.is_enabled() {
            command = command.mut_subcommand("secret", |c| c.hide(true));
        }

        // Hide the federate subcommand from help text.
        if !FeatureFlag::OzIdentityFederation.is_enabled() {
            command = command.mut_subcommand("federate", |c| c.hide(true));
        }

        // Hide the harness-support subcommand from help text.
        if !FeatureFlag::AgentHarness.is_enabled() {
            command = command.mut_subcommand("harness-support", |c| c.hide(true));
        }

        // Hide the conversation subcommand and --conversation flag from help text.
        if !FeatureFlag::ConversationApi.is_enabled() {
            command = command.mut_subcommand("run", |run_cmd| {
                run_cmd
                    .mut_subcommand("conversation", |c| c.hide(true))
                    .mut_subcommand("get", |get_cmd| {
                        get_cmd.mut_arg("conversation", |arg| arg.hide(true))
                    })
            });
        }

        // Hide the artifact subcommand from help text.
        if !FeatureFlag::ArtifactCommand.is_enabled() {
            command = command.mut_subcommand("artifact", |c| c.hide(true));
        }

        // Hide the api-key subcommand from help text.
        if !FeatureFlag::APIKeyManagement.is_enabled() {
            command = command.mut_subcommand("api-key", |c| c.hide(true));
        }

        // Hide the runner subcommand from help text.
        if !FeatureFlag::CloudAgentRunners.is_enabled() {
            command = command.mut_subcommand("runner", |c| c.hide(true));
        }

        #[allow(unexpected_cfgs)]
        let is_oz_cli = cfg!(feature = "standalone")
            || binary_name().is_some_and(|name| name.starts_with("oz"))
            || env::var_os("WARP_CLI_MODE").is_some();
        if is_oz_cli {
            for arg_id in [
                "working_directory",
                "new_window",
                "title",
                "wait_after_command",
                "command",
            ] {
                command = command.mut_arg(arg_id, |arg| arg.hide(true));
            }
        }

        // Wire up `--version` / `-V` using the same version metadata used elsewhere in the
        // app, so the CLI reports the build's release tag.
        command = command.version(version_string());

        // Substitute the actual binary name into help output. Ideally clap would do this for us.
        let bin_name =
            binary_name().unwrap_or_else(|| ChannelState::channel().cli_command_name().to_string());
        command = command.after_help(color_print::cformat!(
            r#"<bold><underline>Examples:</underline></bold>

  <dim>$</dim> <bold>{bin_name} agent run --prompt "Build anything"</bold>

  <dim>$</dim> <bold>{bin_name} mcp list</bold>

<bold><underline>Learn more:</underline></bold>
* Use <bold>{bin_name} help</bold> to learn more about each command
* Read the documentation at https://docs.warp.dev/reference/cli
"#
        ));

        command
    }

    /// The requested subcommand, if any.
    pub fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }

    /// Args for the main Warp application, if not running a subcommand.
    pub fn app_args(&self) -> &AppArgs {
        &self.args
    }

    /// Extract the main Warp application args.
    pub fn into_app_args(self) -> AppArgs {
        self.args
    }

    /// Returns the global options.
    pub fn global_options(&self) -> &GlobalOptions {
        &self.global_options
    }

    /// Returns the API key if provided.
    pub fn api_key(&self) -> Option<&String> {
        self.global_options.api_key.as_ref()
    }

    /// Returns the output format.
    pub fn output_format(&self) -> OutputFormat {
        self.global_options.output_format
    }

    /// Returns true if debug logging is enabled.
    pub fn debug(&self) -> bool {
        self.debug
    }

    pub fn server_root_url(&self) -> Option<&str> {
        self.server_root_url.as_deref()
    }

    pub fn ws_server_url(&self) -> Option<&str> {
        self.ws_server_url.as_deref()
    }

    pub fn session_sharing_server_url(&self) -> Option<&str> {
        self.session_sharing_server_url.as_deref()
    }
}

/// Warp may spawn several worker processes - mostly servers that support the main application.
///
/// These subcommands run those worker processes, which are bundled into the Warp binary.
#[derive(Debug, Clone, Subcommand)]
pub enum WorkerCommand {
    /// Run the terminal server.
    #[clap(hide = true)]
    #[cfg(unix)]
    TerminalServer(TerminalServerArgs),

    /// Run the minidump server.
    #[clap(hide = true)]
    MinidumpServer {
        /// Socket name for the minidump server.
        socket_name: PathBuf,
    },

    /// Run the remote development server proxy over SSH stdio.
    /// Ensures the daemon is running, then bridges its stdin/stdout
    /// to the daemon via a Unix domain socket.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RemoteServerProxy(RemoteServerIdentityArgs),

    /// Run the long-lived remote development server daemon.
    /// Listens on a Unix domain socket and accepts multiple concurrent
    /// connections from proxy processes.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RemoteServerDaemon(RemoteServerIdentityArgs),

    /// Connect an Agent Client Protocol agent's stdin/stdout to the loopback socket the agent
    /// driver is listening on. Run from the driver's terminal session so the agent inherits the
    /// shell state established by environment setup commands.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    AcpBridge {
        /// File the driver wrote with the address to connect to, the token to present, and the
        /// agent command line.
        #[clap(long = "launch-file")]
        launch_file: PathBuf,
    },

    /// Serve computer-use tools to an external agent harness over MCP on stdin/stdout.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    ComputerUseMcp(computer_use_mcp::ComputerUseMcpArgs),

    /// Run a headless ripgrep search worker.
    #[cfg(not(target_family = "wasm"))]
    #[clap(hide = true)]
    RipgrepSearch {
        #[clap(flatten)]
        parent: ParentOpts,
        #[clap(long = "ignore-case")]
        ignore_case: bool,
        #[clap(long = "multiline")]
        multiline: bool,
        /// Search pattern.
        pattern: String,
        /// Paths to search.
        paths: Vec<PathBuf>,
    },
}

/// CLI-related subcommands. The command-line interface to Warp isn't a full SDK (e.g. with language bindings),
/// but it allows scripting some Warp functionality.
#[derive(Debug, Clone, Subcommand)]
pub enum CliCommand {
    /// Interact with Oz.
    #[command(subcommand)]
    Agent(crate::agent::AgentCommand),

    /// Manage cloud environments.
    #[command(subcommand)]
    Environment(crate::environment::EnvironmentCommand),

    #[command(hide = true)]
    EnvironmentCheckout(crate::environment_checkout::EnvironmentCheckoutArgs),

    /// Manage MCP servers.
    #[command(subcommand)]
    MCP(crate::mcp::MCPCommand),

    /// Manage runs.
    #[command(subcommand, alias = "task")]
    Run(crate::task::TaskCommand),

    /// Manage available models.
    #[command(subcommand)]
    Model(crate::model::ModelCommand),
    /// Manage memory stores.
    #[command(subcommand, alias = "memory-stores")]
    MemoryStore(crate::memory_store::MemoryStoreCommand),
    /// Manage memories.
    #[command(subcommand)]
    Memory(crate::memory_store::MemoryCommand),

    /// Log in to Warp.
    Login,
    /// Log out of Warp.
    Logout,
    /// Print information about the logged-in user.
    Whoami,

    /// Manage providers.
    #[command(subcommand)]
    Provider(crate::provider::ProviderCommand),

    /// Manage integrations.
    #[command(subcommand)]
    Integration(crate::integration::IntegrationCommand),

    /// Create and manage scheduled Oz agents. Scheduled agents run a user-defined task periodically, according to a cron schedule.
    ///
    /// As a shorthand, the `schedule` command behaves identically to `schedule create`.
    Schedule(crate::schedule::ScheduleCommand),

    /// Manage secrets.
    #[command(subcommand)]
    Secret(crate::secret::SecretCommand),

    /// Issue and manage federated identity tokens.
    #[command(subcommand)]
    Federate(crate::federate::FederateCommand),

    /// Support commands for agent harnesses to integrate with Oz.
    #[command(hide = true)]
    HarnessSupport(crate::harness_support::HarnessSupportArgs),

    /// Manage artifacts.
    #[command(subcommand)]
    Artifact(crate::artifact::ArtifactCommand),

    /// Manage API keys.
    #[command(subcommand)]
    ApiKey(crate::api_key::ApiKeyCommand),

    /// Manage cloud agent runners.
    #[command(subcommand)]
    Runner(crate::runner::RunnerCommand),
}

impl CliCommand {
    /// Returns the command path used to identify this invocation in tracing.
    pub fn as_str_for_tracing(&self) -> &'static str {
        match self {
            CliCommand::Agent(command) => command.as_str_for_tracing(),
            CliCommand::Environment(command) => command.as_str_for_tracing(),
            CliCommand::EnvironmentCheckout(_) => "environment_checkout",
            CliCommand::MCP(command) => command.as_str_for_tracing(),
            CliCommand::Run(command) => command.as_str_for_tracing(),
            CliCommand::Model(command) => command.as_str_for_tracing(),
            CliCommand::Login => "login",
            CliCommand::Logout => "logout",
            CliCommand::Whoami => "whoami",
            CliCommand::Provider(command) => command.as_str_for_tracing(),
            CliCommand::Integration(command) => command.as_str_for_tracing(),
            CliCommand::Schedule(command) => command.as_str_for_tracing(),
            CliCommand::Secret(command) => command.as_str_for_tracing(),
            CliCommand::Federate(command) => command.as_str_for_tracing(),
            CliCommand::HarnessSupport(args) => args.command.as_str_for_tracing(),
            CliCommand::Artifact(command) => command.as_str_for_tracing(),
            CliCommand::ApiKey(command) => command.as_str_for_tracing(),
            CliCommand::MemoryStore(command) => command.as_str_for_tracing(),
            CliCommand::Memory(command) => command.as_str_for_tracing(),
            CliCommand::Runner(command) => command.as_str_for_tracing(),
        }
    }
}

/// A subcommand of the main Warp application. This includes all [`WorkerCommand`]s as well as app-specific debugging tools.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[clap(flatten)]
    Worker(WorkerCommand),

    /// Commands that make up the Warp CLI.
    #[clap(flatten)]
    CommandLine(Box<CliCommand>),

    /// Generate shell completions for your shell to stdout.
    ///
    ///
    /// For bash, add the following to ~/.bashrc:
    ///     source <(path/to/warp completions bash)
    ///
    /// For zsh, add the following to ~/.zshrc:
    ///     source <(path/to/warp completions zsh)
    ///
    /// For fish, add the following to ~/.config/fish/config.fish:
    ///     path/to/warp completions fish | source
    ///
    /// For Powershell, add the following to $PROFILE:
    ///     path\to\warp | Out-String | Invoke-Expression
    ///
    /// If no shell is provided, this defaults to the shell that Warp was run from.
    #[command(verbatim_doc_comment)]
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Option<clap_complete::aot::Shell>,
    },

    /// Print debugging information and exit.
    #[clap(long_flag = "dump-debug-info")]
    DumpDebugInfo,
    /// Print the JSON schema for the current Warp channel's settings and exit.
    #[cfg(not(target_family = "wasm"))]
    DumpSettingsSchema {
        /// Write the schema to this path instead of standard output.
        output_path: Option<PathBuf>,
    },

    /// Print telemetry events in production and exit.
    #[clap(long_flag = "print-telemetry-events", hide = true)]
    #[cfg(not(target_family = "wasm"))]
    PrintTelemetryEvents,
}

impl Command {
    /// Whether or not the Command should print to stdout.
    pub fn prints_to_stdout(&self) -> bool {
        match self {
            Command::Worker(_) => false,
            Command::CommandLine(_) | Command::DumpDebugInfo => true,
            Command::Completions { .. } => true,
            #[cfg(not(target_family = "wasm"))]
            Command::DumpSettingsSchema { output_path } => output_path.is_none(),
            #[cfg(not(target_family = "wasm"))]
            Command::PrintTelemetryEvents => true,
        }
    }
}

/// Arguments for the terminal server.
#[cfg(not(windows))]
#[derive(Debug, Clone, Default, clap::Args)]
pub struct TerminalServerArgs {
    #[clap(flatten)]
    pub parent: ParentOpts,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum RecoveryMechanism {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[value(name = "force-x11")]
    X11,
    #[value(name = "force-dedicated-gpu")]
    DedicatedGpu,
    #[value(name = "disable-opengl")]
    DisableOpenGL,
    #[value(name = "force-vulkan")]
    ForceVulkan,
}

impl fmt::Display for RecoveryMechanism {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().expect("no values are skipped");
        f.write_str(value.get_name())
    }
}

/// Returns the subcommand name to use for starting the terminal server.
pub fn terminal_server_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("terminal-server")
        .expect("terminal-server subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the subcommand name to use for starting the installation detection server.
pub fn installation_detection_server_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("installation-detection-server")
        .expect("installation-detection-server subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the subcommand name to use for starting the ripgrep search worker.
#[cfg(not(target_family = "wasm"))]
pub fn ripgrep_search_subcommand() -> String {
    <Args as CommandFactory>::command()
        .find_subcommand("ripgrep-search")
        .expect("ripgrep-search subcommand not found")
        .get_name()
        .to_string()
}

/// Returns the flag to use when finishing the auto-update process.
pub fn finish_update_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .get_arguments()
        .find(|arg| arg.get_long() == Some("finish-update"))
        .expect("finish-update flag not found")
        .get_long()
        .unwrap();
    format!("--{flag}")
}

/// Returns the flag to use for the dump-debug-info subcommand.
pub fn dump_debug_info_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .find_subcommand("dump-debug-info")
        .expect("dump-debug-info subcommand not found")
        .get_long_flag()
        .expect("dump-debug-info flag not found");
    format!("--{flag}")
}

/// Returns a flag that sets the current process as the parent of a Warp subcommand to spawn.
pub fn parent_flag() -> String {
    let command = <Args as CommandFactory>::command();
    let flag = command
        .get_arguments()
        .find(|arg| arg.get_long() == Some("parent-pid"))
        .expect("parent-pid flag not found")
        .get_long()
        .unwrap();
    format!("--{flag}={}", std::process::id())
}

/// The name that this binary was invoked as.
pub fn binary_name() -> Option<String> {
    // Adapted from https://github.com/clap-rs/clap/blob/2c04acd3607e5c4676477ca14948419bb31c73a1/clap_builder/src/builder/command.rs#L888-L902
    // Unfortunately, we can't use Command::get_bin_name because it's not populated until args are parsed.
    let arg0 = env::args().next()?;
    Path::new(&arg0).file_name()?.to_str().map(|s| s.to_owned())
}

/// The version string shown for `--version` / `-V`.
///
/// Sourced from [`ChannelState::app_version`], which is populated from the
/// `GIT_RELEASE_TAG` env var at compile time. Falls back to a placeholder for
/// untagged builds (e.g. local `cargo run`).
pub fn version_string() -> &'static str {
    ChannelState::app_version().unwrap_or("<unknown>")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalCommandSplitKind {
    Argv,
    WindowsCmdLine,
}

fn flag_takes_separate_value(command: &clap::Command, token: &str) -> bool {
    let long = token
        .strip_prefix("--")
        .filter(|s| !s.is_empty() && !s.contains('='));
    let short = if long.is_none() {
        token
            .strip_prefix('-')
            .filter(|s| s.chars().count() == 1)
            .and_then(|s| s.chars().next())
            .filter(|&ch| ch != '-')
    } else {
        None
    };

    if long.is_none() && short.is_none() {
        return false;
    }

    command.get_arguments().any(|arg| {
        if matches!(arg.get_id().as_str(), "wait_after_command" | "command") {
            return false;
        }
        let matches_flag = long.is_some_and(|long| {
            arg.get_long() == Some(long)
                || arg
                    .get_all_aliases()
                    .into_iter()
                    .flatten()
                    .any(|a| a == long)
        }) || short.is_some_and(|ch| {
            arg.get_short() == Some(ch)
                || arg
                    .get_all_short_aliases()
                    .into_iter()
                    .flatten()
                    .any(|a| a == ch)
        });
        matches_flag
            && arg.get_action().takes_values()
            && arg
                .get_num_args()
                .map(|r| r.min_values() >= 1)
                .unwrap_or(true)
    })
}

fn find_external_command_split(
    raw_args: &[OsString],
    command: &clap::Command,
    allow_windows_slash_c: bool,
) -> Option<(usize, ExternalCommandSplitKind, Option<String>)> {
    let mut idx = 1;
    while idx < raw_args.len() {
        let token = raw_args[idx].to_string_lossy();

        if token == "--" {
            return None;
        }
        if token == "-e" || token == "--command" {
            return Some((idx, ExternalCommandSplitKind::Argv, None));
        }
        if let Some(first_arg) = token.strip_prefix("--command=") {
            let inline_first_arg = (!first_arg.is_empty()).then(|| first_arg.to_owned());
            return Some((idx, ExternalCommandSplitKind::Argv, inline_first_arg));
        }
        if !token.starts_with("--")
            && let Some(first_arg) = token.strip_prefix("-e")
            && !first_arg.is_empty()
        {
            return Some((
                idx,
                ExternalCommandSplitKind::Argv,
                Some(first_arg.to_owned()),
            ));
        }
        if allow_windows_slash_c && (token == "/c" || token == "/C") {
            return Some((idx, ExternalCommandSplitKind::WindowsCmdLine, None));
        }
        if token == "--wait-after-command" {
            if raw_args
                .get(idx + 1)
                .is_some_and(|next| matches!(next.to_string_lossy().as_ref(), "true" | "false"))
            {
                idx += 2;
            } else {
                idx += 1;
            }
            continue;
        }
        if flag_takes_separate_value(command, &token) {
            idx += 2;
            continue;
        }
        let is_long_flag_subcommand = token.strip_prefix("--").is_some_and(|long_flag| {
            command
                .get_subcommands()
                .any(|sub| sub.get_long_flag() == Some(long_flag))
        });
        if command.find_subcommand(&*token).is_some()
            || is_long_flag_subcommand
            || is_worker_invocation(&token)
        {
            return None;
        }
        if token.starts_with('-') {
            idx += 1;
            continue;
        }
        idx += 1;
    }

    None
}

#[cfg(windows)]
fn current_win32_command_line() -> Option<String> {
    unsafe {
        let ptr = windows::Win32::System::Environment::GetCommandLineW();
        ptr.to_string().ok()
    }
}

fn resolve_windows_cmdline_after_slash_c(
    prefix_end: usize,
    trailing_args: &[String],
    raw_windows_cmdline: Option<&str>,
) -> String {
    if let Some(cmdline) = raw_windows_cmdline
        && let Some(raw_tail) = extract_raw_windows_cmdline_after_token(cmdline, prefix_end)
    {
        return raw_tail;
    }

    join_windows_cmdline_args(trailing_args)
}

fn join_windows_cmdline_args(trailing_args: &[String]) -> String {
    if trailing_args.len() == 1 {
        return trailing_args[0].clone();
    }

    let mut result = String::new();
    for (idx, arg) in trailing_args.iter().enumerate() {
        if idx > 0 {
            result.push(' ');
        }
        let already_quoted = arg.len() >= 2 && arg.starts_with('"') && arg.ends_with('"');
        if !already_quoted && (arg.is_empty() || arg.chars().any(char::is_whitespace)) {
            result.push('"');
            result.push_str(&arg.replace('"', "\\\""));
            result.push('"');
        } else {
            result.push_str(arg);
        }
    }
    result
}

fn extract_raw_windows_cmdline_after_token(cmdline: &str, token_index: usize) -> Option<String> {
    let bytes = cmdline.as_bytes();
    let mut i = 0;

    // `argv[0]` follows CommandLineToArgvW rules without backslash quote escaping.
    let mut in_quotes = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            b' ' | b'\t' if !in_quotes => break,
            _ => i += 1,
        }
    }

    for _ in 1..=token_index {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }

        let mut in_quotes = false;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                let mut backslashes = 0;
                while i < bytes.len() && bytes[i] == b'\\' {
                    backslashes += 1;
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'"' {
                    if backslashes % 2 == 0 {
                        if in_quotes && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                            i += 2;
                        } else {
                            in_quotes = !in_quotes;
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            } else if bytes[i] == b'"' {
                if in_quotes && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    i += 2;
                } else {
                    in_quotes = !in_quotes;
                    i += 1;
                }
            } else if !in_quotes && matches!(bytes[i], b' ' | b'\t') {
                break;
            } else {
                i += 1;
            }
        }
    }

    let rest = cmdline[i..].trim_matches(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n'));
    if rest.is_empty() {
        return None;
    }

    Some(unwrap_redundant_windows_cmdline_quotes(rest).to_owned())
}

fn unwrap_redundant_windows_cmdline_quotes(cmdline: &str) -> &str {
    // VS Code's non-wt fallback wraps the entire quoted command string in outer quotes
    // (`exec /c "\"prog\" \"arg\" & pause"`), producing a leading `""` and trailing `"`.
    if cmdline.len() >= 3 && cmdline.starts_with("\"\"") && cmdline.ends_with('"') {
        &cmdline[1..cmdline.len() - 1]
    } else {
        cmdline
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
