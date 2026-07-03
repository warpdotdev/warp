use std::{env, fmt, path::Path};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use url::Url;

use crate::agent::OutputFormat;
use warp_core::channel::ChannelState;

#[cfg(windows)]
mod process_handle;

pub mod scope;
pub mod skill;

pub mod agent;
pub mod completions;
pub mod config_file;
pub mod json_filter;
pub mod mcp;
pub mod model;
pub mod provider;
pub mod secret;

mod environment;

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

/// Global options that apply to all CLI commands.
#[derive(Debug, Default, Clone, clap::Args)]
pub struct GlobalOptions {
    /// API key for local AI provider requests.
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

/// Command-line argument parser for the main Warp binary. This is used across all channels.
#[derive(Debug, Default, Parser, Clone)]
#[command(
    name = "warp",
    display_name = "Warp",
    about = r#"Warp terminal and local AI tooling

Use the CLI to:
* Start local agent sessions
* Manage local MCP servers
* Inspect available AI models"#
)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[clap(flatten)]
    global_options: GlobalOptions,

    /// Enable debug mode.
    #[arg(long = "debug", global = true, help = "Enable debug logging")]
    debug: bool,

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
        use clap::FromArgMatches as _;

        // Warp-managed secrets are hosted infrastructure and are not part of Warper.
        let args: Vec<String> = env::args().collect();
        if args.len() > 1 && args[1] == "secret" {
            eprintln!("error: unrecognized subcommand 'secret'\n");
            eprintln!("For more information, try '--help'");
            std::process::exit(2);
        }

        let command = Self::clap_command();

        command
            .try_get_matches()
            .and_then(|matches| Self::from_arg_matches(&matches))
            .unwrap_or_else(|err| {
                // We attach a console to ensure help and error messages are printed
                // when using the CLI.
                #[cfg(windows)]
                warp_util::windows::attach_to_parent_console();
                err.exit()
            })
    }

    /// Construct the [`clap::Command`] that backs `Args`.
    ///
    /// IMPORTANT: use this instead of [`CommandFactory::command`], since we customize the command at runtime.
    pub fn clap_command() -> clap::Command {
        let mut command = <Args as CommandFactory>::command();

        // Hide hosted Warp-managed secrets from help text.
        command = command.mut_subcommand("secret", |c| c.hide(true));

        // Substitute the actual binary name into help output. Ideally clap would do this for us.
        let bin_name =
            binary_name().unwrap_or_else(|| ChannelState::channel().cli_command_name().to_string());
        command = command.after_help(color_print::cformat!(
            r#"<bold><underline>Examples:</underline></bold>

  <dim>$</dim> <bold>{bin_name} agent run --prompt "Build anything"</bold>

  <dim>$</dim> <bold>{bin_name} mcp list</bold>

<bold><underline>Learn more:</underline></bold>
* Use <bold>{bin_name} help</bold> to learn more about each command
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

    /// Run a headless ripgrep search worker.
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
    /// Run local agent tasks.
    #[command(subcommand)]
    Agent(crate::agent::AgentCommand),

    /// Manage MCP servers.
    #[command(subcommand)]
    MCP(crate::mcp::MCPCommand),

    /// Manage available models.
    #[command(subcommand)]
    Model(crate::model::ModelCommand),

    /// Manage providers.
    #[command(subcommand)]
    Provider(crate::provider::ProviderCommand),

    /// Manage secrets.
    #[command(subcommand)]
    Secret(Box<crate::secret::SecretCommand>),
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

    /// Spawn and tear down a local terminal session for WARPER-001 smoke tests.
    #[clap(long_flag = "warper-local-terminal-smoke", hide = true)]
    WarperLocalTerminalSmoke,
}

impl Command {
    /// Whether or not the Command should print to stdout.
    pub fn prints_to_stdout(&self) -> bool {
        match self {
            Command::Worker(_) => false,
            Command::CommandLine(_)
            | Command::DumpDebugInfo
            | Command::WarperLocalTerminalSmoke => true,
            Command::Completions { .. } => true,
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
    #[cfg(target_os = "linux")]
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
