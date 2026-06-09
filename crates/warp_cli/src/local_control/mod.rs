//! Command-line interface for controlling a running local Warp app.
mod commands;
mod completions;
mod output;
mod selectors;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::aot::Shell;
use commands::{run_app_command, run_instance_command, run_tab_command};
use completions::generate_completions_to_stdout;
use output::write_control_error;

use crate::agent::OutputFormat;
/// Hidden flag used by the channel-specific Warp app binary to enter `warpctrl` mode.
pub const CONTROL_MODE_FLAG: &str = "--warpctrl";

/// Parsed top-level arguments for `warpctrl`.
#[derive(Debug, Parser)]
#[command(
    name = "warpctrl",
    display_name = "warpctrl",
    about = "Control a running local Warp app instance"
)]
pub struct ControlArgs {
    /// Set the output format.
    #[arg(
        long = "output-format",
        global = true,
        value_enum,
        default_value_t = OutputFormat::Pretty,
        env = "WARP_OUTPUT_FORMAT"
    )]
    pub output_format: OutputFormat,

    #[command(subcommand)]
    pub command: ControlCommand,
}

impl ControlArgs {
    pub fn from_env() -> Self {
        let bin_name = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
        Self::try_parse_from_args(std::env::args_os(), bin_name).unwrap_or_else(|err| err.exit())
    }

    pub fn from_control_mode_env() -> Option<Self> {
        Self::try_parse_control_mode_from(std::env::args_os())
            .map(|result| result.unwrap_or_else(|err| err.exit()))
    }

    pub fn try_parse_control_mode_from<I, T>(args: I) -> Option<Result<Self, clap::Error>>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        let mut stripped_args = vec![OsString::from("warpctrl")];
        let mut found_control_mode = false;

        for arg in args {
            let arg = arg.into();
            if !found_control_mode {
                if arg.to_str() == Some(CONTROL_MODE_FLAG) {
                    found_control_mode = true;
                }
                continue;
            }
            stripped_args.push(arg);
        }

        found_control_mode.then(|| Self::try_parse_from_args(stripped_args, "warpctrl"))
    }

    pub fn clap_command() -> clap::Command {
        let bin_name = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
        Self::clap_command_for_bin_name(bin_name)
    }

    fn try_parse_from_args<I, T>(args: I, bin_name: impl Into<String>) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::clap_command_for_bin_name(bin_name).try_get_matches_from(args)?;
        Self::from_arg_matches(&matches)
    }

    fn clap_command_for_bin_name(bin_name: impl Into<String>) -> clap::Command {
        let bin_name = bin_name.into();
        <Self as CommandFactory>::command()
            .version(crate::version_string())
            .bin_name(bin_name.clone())
            .after_help(color_print::cformat!(
                r#"<bold><underline>Examples:</underline></bold>

  <dim>$</dim> <bold>{bin_name} instance list</bold>

  <dim>$</dim> <bold>{bin_name} tab create</bold>

<bold><underline>Learn more:</underline></bold>
* Use <bold>{bin_name} help</bold> to learn more about each command
"#
            ))
    }
}

/// Top-level `warpctrl` command groups.
#[derive(Debug, Clone, Subcommand)]
pub enum ControlCommand {
    /// Inspect local Warp app instances.
    #[command(subcommand)]
    Instance(InstanceCommand),
    /// Inspect a selected local Warp app.
    #[command(subcommand)]
    App(AppCommand),

    /// Control local Warp tabs.
    #[command(subcommand)]
    Tab(TabCommand),

    /// Generate shell completions for your shell to stdout.
    ///
    /// For bash, add the following to ~/.bashrc:
    ///     source <(path/to/warpctrl completions bash)
    ///
    /// For zsh, add the following to ~/.zshrc:
    ///     source <(path/to/warpctrl completions zsh)
    ///
    /// For fish, add the following to ~/.config/fish/config.fish:
    ///     path/to/warpctrl completions fish | source
    ///
    /// For Powershell, add the following to $PROFILE:
    ///     path\to\warpctrl completions powershell | Out-String | Invoke-Expression
    ///
    /// If no shell is provided, this defaults to the shell that Warp was run from.
    #[command(verbatim_doc_comment)]
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Option<Shell>,
    },
}

/// Commands that inspect locally discoverable Warp instances.
#[derive(Debug, Clone, Subcommand)]
pub enum InstanceCommand {
    /// List locally discoverable Warp instances.
    List,
}

/// Commands that inspect the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum AppCommand {
    /// Check that the selected local Warp app responds.
    Ping(TargetArgs),

    /// Print protocol and build identity metadata for the selected local Warp app.
    Version(TargetArgs),
}

/// Commands that control tabs in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// Create a new terminal tab in the active window.
    Create(TargetArgs),
}

/// Common flags for selecting which running Warp instance receives a command.
#[derive(Debug, Clone, Args, Default)]
pub struct TargetArgs {
    /// Target a specific local Warp instance id from `warp instance list`.
    #[arg(long = "instance")]
    pub instance: Option<String>,

    /// Target a specific local Warp process id.
    #[arg(long = "pid", conflicts_with = "instance")]
    pub pid: Option<u32>,
}

pub fn run(args: ControlArgs) -> ExitCode {
    ExitCode::from(run_exit_code(args))
}

pub fn run_and_exit(args: ControlArgs) -> ! {
    std::process::exit(i32::from(run_exit_code(args)))
}

fn run_exit_code(args: ControlArgs) -> u8 {
    let output_format = args.output_format;
    match run_inner(args) {
        Ok(()) => 0,
        Err(error) => {
            if let Err(write_error) = write_control_error(&error, output_format) {
                eprintln!(
                    "error: failed to render local-control error: {}",
                    write_error.message
                );
            }
            1
        }
    }
}

fn run_inner(args: ControlArgs) -> Result<(), local_control::protocol::ControlError> {
    let output_format = args.output_format;
    match args.command {
        ControlCommand::Instance(command) => run_instance_command(command, output_format),
        ControlCommand::App(command) => run_app_command(command, output_format),
        ControlCommand::Tab(command) => run_tab_command(command, output_format),
        ControlCommand::Completions { shell } => generate_completions_to_stdout(shell),
    }
}

#[cfg(test)]
pub(crate) use commands::render_human_readable_for_test;
#[cfg(test)]
pub(crate) use completions::generate_completion_string;
#[cfg(test)]
pub(crate) use output::ErrorSummary;

#[cfg(test)]
#[path = "../local_control_tests.rs"]
mod tests;
