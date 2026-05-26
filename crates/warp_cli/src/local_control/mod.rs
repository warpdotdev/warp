//! Command-line interface for controlling a running local Warp app.
mod commands;
mod completions;
mod output;
mod selectors;

use std::process::ExitCode;

use crate::agent::OutputFormat;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::aot::Shell;

use commands::{
    run_app_command, run_appearance_command, run_instance_command, run_pane_command,
    run_setting_command, run_tab_command, run_theme_command,
};
use completions::generate_completions_to_stdout;
use output::write_control_error;

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
        let matches = Self::clap_command().get_matches();
        Self::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
    }

    pub fn clap_command() -> clap::Command {
        let bin_name = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
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

    /// Control local Warp panes.
    #[command(subcommand)]
    Pane(PaneCommand),

    /// Control Warp theme settings.
    #[command(subcommand)]
    Theme(ThemeCommand),

    /// Control Warp appearance settings.
    #[command(subcommand)]
    Appearance(AppearanceCommand),

    /// Control allowlisted Warp settings.
    #[command(subcommand)]
    Setting(SettingCommand),

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

    /// Print protocol and app version metadata for the selected local Warp app.
    Version(TargetArgs),
}

/// Commands that control tabs in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// Create a new terminal tab in the active window.
    Create(TargetArgs),

    /// Rename a tab.
    Rename(RenameArgs),

    /// Reset a tab name.
    ResetName(TargetArgs),

    /// Set or clear a tab color.
    #[command(subcommand)]
    Color(TabColorCommand),
}

/// Commands that control tab colors.
#[derive(Debug, Clone, Subcommand)]
pub enum TabColorCommand {
    /// Set a tab color.
    Set(ColorSetArgs),

    /// Clear a tab color.
    Clear(TargetArgs),
}

/// Commands that control panes in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum PaneCommand {
    /// Rename a pane.
    Rename(RenameArgs),

    /// Reset a pane name.
    ResetName(TargetArgs),
}

/// Commands that control Warp themes.
#[derive(Debug, Clone, Subcommand)]
pub enum ThemeCommand {
    /// Set the current theme.
    Set(ThemeSetArgs),

    /// Set whether Warp follows the system theme.
    SystemSet(ThemeSystemSetArgs),

    /// Set the light theme used when following the system theme.
    LightSet(ThemeSetArgs),

    /// Set the dark theme used when following the system theme.
    DarkSet(ThemeSetArgs),
}

/// Commands that control appearance settings.
#[derive(Debug, Clone, Subcommand)]
pub enum AppearanceCommand {
    /// Increase terminal font size.
    FontSizeIncrease(TargetArgs),

    /// Decrease terminal font size.
    FontSizeDecrease(TargetArgs),

    /// Reset terminal font size.
    FontSizeReset(TargetArgs),

    /// Increase UI zoom.
    ZoomIncrease(TargetArgs),

    /// Decrease UI zoom.
    ZoomDecrease(TargetArgs),

    /// Reset UI zoom.
    ZoomReset(TargetArgs),
}

/// Commands that control allowlisted settings.
#[derive(Debug, Clone, Subcommand)]
pub enum SettingCommand {
    /// Set one allowlisted setting.
    Set(SettingSetArgs),

    /// Toggle one allowlisted boolean setting.
    Toggle(SettingToggleArgs),
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

    /// Target a window by id, or pass `active` for the active window.
    #[arg(long = "window")]
    pub window: Option<String>,

    /// Target a tab by id, or pass `active` for the active tab.
    #[arg(long = "tab", conflicts_with = "tab_index")]
    pub tab: Option<String>,

    /// Target a tab by zero-based index.
    #[arg(long = "tab-index")]
    pub tab_index: Option<u32>,

    /// Target a pane by id, or pass `active` for the active pane.
    #[arg(long = "pane", conflicts_with = "pane_index")]
    pub pane: Option<String>,

    /// Target a pane by zero-based index.
    #[arg(long = "pane-index")]
    pub pane_index: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct RenameArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    pub title: String,
}

#[derive(Debug, Clone, Args)]
pub struct ColorSetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    pub color: String,
}

#[derive(Debug, Clone, Args)]
pub struct ThemeSetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    pub name: String,
}

#[derive(Debug, Clone, Args)]
pub struct ThemeSystemSetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(action = clap::ArgAction::Set)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Args)]
pub struct SettingSetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    pub key: String,

    pub value: String,
}

#[derive(Debug, Clone, Args)]
pub struct SettingToggleArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    pub key: String,
}

pub fn run(args: ControlArgs) -> ExitCode {
    let output_format = args.output_format;
    match run_inner(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if let Err(write_error) = write_control_error(&error, output_format) {
                eprintln!(
                    "error: failed to render local-control error: {}",
                    write_error.message
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: ControlArgs) -> Result<(), local_control::protocol::ControlError> {
    let output_format = args.output_format;
    match args.command {
        ControlCommand::Instance(command) => run_instance_command(command, output_format),
        ControlCommand::App(command) => run_app_command(command, output_format),
        ControlCommand::Tab(command) => run_tab_command(command, output_format),
        ControlCommand::Pane(command) => run_pane_command(command, output_format),
        ControlCommand::Theme(command) => run_theme_command(command, output_format),
        ControlCommand::Appearance(command) => run_appearance_command(command, output_format),
        ControlCommand::Setting(command) => run_setting_command(command, output_format),
        ControlCommand::Completions { shell } => generate_completions_to_stdout(shell),
    }
}

#[cfg(test)]
pub(crate) use completions::generate_completion_string;
#[cfg(test)]
pub(crate) use output::ErrorSummary;

#[cfg(test)]
#[path = "../local_control_tests.rs"]
mod tests;
