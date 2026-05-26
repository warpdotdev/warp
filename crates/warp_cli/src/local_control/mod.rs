//! Command-line interface for controlling a running local Warp app.
mod commands;
mod completions;
mod output;
mod selectors;

use std::process::ExitCode;

use crate::agent::OutputFormat;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::aot::Shell;

use commands::{
    run_action_command, run_app_command, run_appearance_command, run_block_command,
    run_capability_command,
    run_drive_command, run_file_command, run_history_command, run_input_command,
    run_instance_command, run_keybinding_command, run_pane_command, run_project_command,
    run_session_command, run_setting_command, run_tab_command, run_theme_command,
    run_window_command,
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
    /// Inspect local-control capabilities.
    #[command(subcommand)]
    Capability(CapabilityCommand),
    /// Inspect the local-control action catalog.
    #[command(subcommand)]
    Action(ActionCommand),

    /// Inspect local Warp windows.
    #[command(subcommand)]
    Window(WindowCommand),

    /// Control local Warp tabs.
    #[command(subcommand)]
    Tab(TabCommand),
    /// Inspect local Warp panes.
    #[command(subcommand)]
    Pane(PaneCommand),

    /// Inspect local Warp sessions.
    #[command(subcommand)]
    Session(SessionCommand),

    /// Inspect terminal blocks.
    #[command(subcommand)]
    Block(BlockCommand),

    /// Inspect terminal input state.
    #[command(subcommand)]
    Input(InputCommand),

    /// Inspect terminal command history.
    #[command(subcommand)]
    History(HistoryCommand),
    /// Inspect Warp themes.
    #[command(subcommand)]
    Theme(ThemeCommand),

    /// Inspect appearance state.
    #[command(subcommand)]
    Appearance(AppearanceCommand),

    /// Inspect allowlisted settings.
    #[command(subcommand)]
    Setting(SettingCommand),

    /// Inspect keybinding metadata.
    #[command(subcommand)]
    Keybinding(KeybindingCommand),

    /// Inspect open file app-state metadata.
    #[command(subcommand)]
    File(FileCommand),

    /// Inspect project app-state metadata.
    #[command(subcommand)]
    Project(ProjectCommand),

    /// Inspect authenticated Warp Drive objects.
    #[command(subcommand)]
    Drive(DriveCommand),

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

    /// Print app, protocol, active target, and action metadata for the selected instance.
    Inspect(TargetArgs),
}

/// Commands that inspect the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum AppCommand {
    /// Check that the selected local Warp app responds.
    Ping(TargetArgs),

    /// Print protocol and app version metadata for the selected local Warp app.
    Version(TargetArgs),

    /// Print the active window/tab/pane/session chain.
    Active(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum CapabilityCommand {
    /// List implemented local-control capabilities.
    List(TargetArgs),

    /// Inspect one local-control capability.
    #[command(alias = "get")]
    Inspect(ActionInspectArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ActionCommand {
    /// List allowlisted local-control actions.
    List(TargetArgs),

    /// Inspect one allowlisted local-control action.
    #[command(alias = "get")]
    Inspect(ActionInspectArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowCommand {
    /// List windows in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one window in the selected local Warp app.
    Inspect(TargetArgs),
}

/// Commands that control tabs in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// List tabs in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one tab in the selected local Warp app.
    Inspect(TargetArgs),

    /// Create a new terminal tab in the active window.
    Create(TargetArgs),
}

/// Commands that inspect local Warp panes.
#[derive(Debug, Clone, Subcommand)]
pub enum PaneCommand {
    /// List panes in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one pane in the selected local Warp app.
    Inspect(TargetArgs),
}
/// Commands that inspect local Warp sessions.

#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// List sessions in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one session in the selected local Warp app.
    Inspect(TargetArgs),
}
/// Commands that inspect terminal blocks.

#[derive(Debug, Clone, Subcommand)]
pub enum BlockCommand {
    /// List terminal blocks.
    List(LimitTargetArgs),

    /// Inspect one terminal block.
    #[command(alias = "get")]
    Inspect(BlockInspectArgs),

    /// Read one terminal block's output.
    Output(BlockInspectArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputCommand {
    /// Read the current input buffer.
    Get(TargetArgs),
}

/// Commands that inspect Warp themes.

#[derive(Debug, Clone, Subcommand)]
pub enum ThemeCommand {
    /// List available themes.
    List(TargetArgs),

    /// Read current theme state.
    Get(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AppearanceCommand {
    /// Read appearance state.
    Get(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum HistoryCommand {
    /// List command history entries.
    List(LimitTargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SettingCommand {
    /// List allowlisted settings.
    List(TargetArgs),

    /// Read one allowlisted setting.
    Get(SettingGetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum KeybindingCommand {
    /// List keybindings.
    List(TargetArgs),

    /// Read one keybinding by name.
    Get(KeybindingGetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum FileCommand {
    /// List files currently open in Warp editor state.
    List(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProjectCommand {
    /// Read the active project.
    Active(TargetArgs),

    /// List known projects.
    List(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum DriveCommand {
    /// List authenticated Warp Drive objects.
    List(DriveListArgs),

    /// Read one authenticated Warp Drive object by id.
    #[command(alias = "get")]
    Inspect(DriveInspectArgs),
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

    /// Target the active window or an opaque window id.
    #[arg(long = "window", conflicts_with_all = ["window_index", "window_title"])]
    pub window: Option<String>,

    /// Target a window by scoped index when the handler supports it.
    #[arg(long = "window-index", conflicts_with_all = ["window", "window_title"])]
    pub window_index: Option<u32>,

    /// Target a window by exact title when the handler supports it.
    #[arg(long = "window-title", conflicts_with_all = ["window", "window_index"])]
    pub window_title: Option<String>,

    /// Target the active tab or an opaque tab id.
    #[arg(long = "tab", conflicts_with_all = ["tab_index", "tab_title"])]
    pub tab: Option<String>,

    /// Target a tab by scoped index when the handler supports it.
    #[arg(long = "tab-index", conflicts_with_all = ["tab", "tab_title"])]
    pub tab_index: Option<u32>,

    /// Target a tab by exact title when the handler supports it.
    #[arg(long = "tab-title", conflicts_with_all = ["tab", "tab_index"])]
    pub tab_title: Option<String>,

    /// Target the active pane or an opaque pane id.
    #[arg(long = "pane", conflicts_with = "pane_index")]
    pub pane: Option<String>,

    /// Target a pane by scoped index when the handler supports it.
    #[arg(long = "pane-index", conflicts_with = "pane")]
    pub pane_index: Option<u32>,

    /// Target the active session or an opaque session id.
    #[arg(long = "session")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct ActionInspectArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Action name, such as tab.create or window.list.
    pub action: String,
}

#[derive(Debug, Clone, Args)]
pub struct LimitTargetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Maximum number of items to return.
    #[arg(long = "limit")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct BlockInspectArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Opaque block id returned by block list.
    pub block_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct SettingGetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Allowlisted setting key.
    pub key: String,
}

#[derive(Debug, Clone, Args)]
pub struct KeybindingGetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Keybinding action name.
    pub name: String,
}

#[derive(Debug, Clone, Args)]
pub struct DriveListArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Filter by Drive object type.
    #[arg(long = "type", value_enum)]
    pub object_type: Option<CliDriveObjectType>,
}

#[derive(Debug, Clone, Args)]
pub struct DriveInspectArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Opaque Drive object id returned by drive list.
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliDriveObjectType {
    Workflow,
    Notebook,
    EnvVarCollection,
    Prompt,
    Folder,
    AiFact,
    McpServer,
    Space,
    Trash,
}

impl From<CliDriveObjectType> for local_control::DriveObjectType {
    fn from(value: CliDriveObjectType) -> Self {
        match value {
            CliDriveObjectType::Workflow => Self::Workflow,
            CliDriveObjectType::Notebook => Self::Notebook,
            CliDriveObjectType::EnvVarCollection => Self::EnvVarCollection,
            CliDriveObjectType::Prompt => Self::Prompt,
            CliDriveObjectType::Folder => Self::Folder,
            CliDriveObjectType::AiFact => Self::AiFact,
            CliDriveObjectType::McpServer => Self::McpServer,
            CliDriveObjectType::Space => Self::Space,
            CliDriveObjectType::Trash => Self::Trash,
        }
    }
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
        ControlCommand::Capability(command) => run_capability_command(command, output_format),
        ControlCommand::Action(command) => run_action_command(command, output_format),
        ControlCommand::Window(command) => run_window_command(command, output_format),
        ControlCommand::Tab(command) => run_tab_command(command, output_format),
        ControlCommand::Pane(command) => run_pane_command(command, output_format),
        ControlCommand::Session(command) => run_session_command(command, output_format),
        ControlCommand::Block(command) => run_block_command(command, output_format),
        ControlCommand::Input(command) => run_input_command(command, output_format),
        ControlCommand::History(command) => run_history_command(command, output_format),
        ControlCommand::Theme(command) => run_theme_command(command, output_format),
        ControlCommand::Appearance(command) => run_appearance_command(command, output_format),
        ControlCommand::Setting(command) => run_setting_command(command, output_format),
        ControlCommand::Keybinding(command) => run_keybinding_command(command, output_format),
        ControlCommand::File(command) => run_file_command(command, output_format),
        ControlCommand::Project(command) => run_project_command(command, output_format),
        ControlCommand::Drive(command) => run_drive_command(command, output_format),
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
