#![cfg_attr(target_family = "wasm", allow(dead_code))]

use std::path::Path;
use std::{env, fmt};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
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
pub mod config_file;
mod date_time;
pub mod environment;
pub mod federate;
pub mod harness_support;
pub mod integration;
pub mod json_filter;
pub mod local_control;
pub mod mcp;
pub mod memory_store;
pub mod model;
pub mod provider;
pub mod schedule;
pub mod secret;
pub mod share;
pub mod task;
pub const OZ_RUN_ID_ENV: &str = "OZ_RUN_ID";
pub const OZ_PARENT_RUN_ID_ENV: &str = "OZ_PARENT_RUN_ID";
pub const OZ_CLI_ENV: &str = "OZ_CLI";
pub const OZ_HARNESS_ENV: &str = "OZ_HARNESS";
pub const SERVER_ROOT_URL_OVERRIDE_ENV: &str = "WARP_SERVER_ROOT_URL";
pub const WS_SERVER_URL_OVERRIDE_ENV: &str = "WARP_WS_SERVER_URL";
pub const SESSION_SHARING_SERVER_URL_OVERRIDE_ENV: &str = "WARP_SESSION_SHARING_SERVER_URL";

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
    #[arg(long = "api-key", global = true, env = "WARP_API_KEY")]
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
#[clap(args_conflicts_with_subcommands = true)]
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
                use clap::FromArgMatches as _;

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

                let command = Self::clap_command();

                command.try_get_matches()
                    .and_then(|matches| Self::from_arg_matches(&matches))
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

    /// Run this process as the plugin host rather than the main app.
    #[cfg(feature = "plugin_host")]
    #[clap(long_flag = "plugin-host")]
    PluginHost {
        #[clap(flatten)]
        parent: ParentOpts,
    },

    /// Run the minidump server.
    #[clap(hide = true)]
    MinidumpServer {
        /// Socket name for the minidump server.
        socket_name: std::path::PathBuf,
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
        paths: Vec<std::path::PathBuf>,
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
}

impl CliCommand {
    /// Returns the command path used to identify this invocation in tracing.
    pub fn as_str_for_tracing(&self) -> &'static str {
        match self {
            CliCommand::Agent(command) => command.as_str_for_tracing(),
            CliCommand::Environment(command) => command.as_str_for_tracing(),
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
