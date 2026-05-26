//! Command-line interface for controlling a running local Warp app.
mod auth_commands;
mod commands;
mod completions;
mod output;
mod selectors;

use std::path::PathBuf;
use std::process::ExitCode;

use crate::agent::OutputFormat;
use auth_commands::{AuthCommand, run_auth_command};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::aot::Shell;

use commands::{
    run_action_command, run_app_command, run_appearance_command, run_block_command,
    run_capability_command, run_drive_command, run_file_command, run_history_command,
    run_input_command, run_instance_command, run_keybinding_command, run_pane_command,
    run_project_command, run_session_command, run_setting_command, run_surface_command,
    run_tab_command, run_theme_command, run_window_command,
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

  <dim>$</dim> <bold>{bin_name} pane split --direction right</bold>

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
    /// Inspect local-control capabilities exposed by the selected app.
    #[command(subcommand)]
    Capability(CapabilityCommand),
    /// Control local Warp windows.
    #[command(subcommand)]
    Window(WindowCommand),
    /// Control local Warp tabs.
    #[command(subcommand)]
    Tab(TabCommand),
    /// Control local Warp panes.
    #[command(subcommand)]
    Pane(PaneCommand),
    /// Inspect and control sessions in a selected pane.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Inspect terminal blocks in a selected session.
    #[command(subcommand)]
    Block(BlockCommand),
    /// Stage text in a selected Warp input buffer.
    #[command(subcommand)]
    Input(InputCommand),
    /// Inspect command history for a selected session.
    #[command(subcommand)]
    History(HistoryCommand),
    /// Inspect and update Warp themes.
    #[command(subcommand)]
    Theme(ThemeCommand),
    /// Inspect and update appearance controls.
    #[command(subcommand)]
    Appearance(AppearanceCommand),
    /// Inspect and update allowlisted settings.
    #[command(subcommand)]
    Setting(SettingCommand),
    /// Inspect keybinding metadata.
    #[command(subcommand)]
    Keybinding(KeybindingCommand),
    /// Inspect local-control action metadata.
    #[command(subcommand)]
    Action(ActionCommand),
    /// Operate Warp's visible file/editor state.
    #[command(subcommand)]
    File(FileCommand),
    /// Operate Warp projects and workspaces.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Operate Warp Drive app surfaces and objects.
    #[command(subcommand)]
    Drive(DriveCommand),
    /// Open and toggle Warp surfaces.
    #[command(subcommand)]
    Surface(SurfaceCommand),
    /// Manage authenticated scripting status and API-key setup.
    #[command(subcommand)]
    Auth(AuthCommand),
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
    /// Inspect one locally discoverable Warp instance.
    Inspect(TargetArgs),
}

/// Commands that inspect the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum AppCommand {
    /// Check that the selected local Warp app responds.
    Ping(TargetArgs),
    /// Print protocol and app version metadata for the selected local Warp app.
    Version(TargetArgs),
    /// Print the active window, tab, pane, and session chain.
    Active(TargetArgs),
    /// Focus the selected local Warp app.
    Focus(TargetArgs),
}

/// Commands that inspect local-control capabilities.
#[derive(Debug, Clone, Subcommand)]
pub enum CapabilityCommand {
    /// List advertised local-control capabilities.
    List(TargetArgs),
    /// Inspect one advertised action capability.
    Inspect(NamedActionArgs),
}

/// Commands that control windows in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum WindowCommand {
    /// List windows.
    List(TargetArgs),
    /// Inspect a selected window.
    Inspect(TargetArgs),
    /// Create a new window.
    Create(WindowCreateArgs),
    /// Focus a selected window.
    Focus(TargetArgs),
    /// Close a selected window.
    Close(TargetArgs),
}

/// Commands that control tabs in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// List tabs.
    List(TargetArgs),
    /// Inspect a selected tab.
    Inspect(TargetArgs),
    /// Create a new tab in the active window.
    Create(TabCreateArgs),
    /// Activate a target, previous, next, or last tab.
    Activate(TabActivateArgs),
    /// Move a tab left or right.
    Move(TabMoveArgs),
    /// Rename a tab.
    Rename(TabRenameArgs),
    /// Reset a tab title.
    ResetName(TargetArgs),
    /// Set or clear a tab color.
    #[command(subcommand)]
    Color(TabColorCommand),
    /// Close the active tab, selected tab, other tabs, or tabs to the right.
    Close(TabCloseArgs),
}

/// Commands that control panes in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum PaneCommand {
    /// List panes.
    List(TargetArgs),
    /// Inspect a selected pane.
    Inspect(TargetArgs),
    /// Split a selected pane.
    Split(PaneSplitArgs),
    /// Focus a selected pane.
    Focus(TargetArgs),
    /// Navigate pane focus.
    Navigate(PaneNavigateArgs),
    /// Resize a selected pane.
    Resize(PaneResizeArgs),
    /// Maximize a selected pane.
    Maximize(TargetArgs),
    /// Unmaximize the current pane.
    Unmaximize(TargetArgs),
    /// Close a selected pane.
    Close(TargetArgs),
    /// Rename a pane.
    Rename(PaneRenameArgs),
    /// Reset a pane title.
    ResetName(TargetArgs),
}

/// Commands that inspect and control sessions.
#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// List sessions.
    List(TargetArgs),
    /// Inspect a selected session.
    Inspect(TargetArgs),
    /// Activate a selected session.
    Activate(TargetArgs),
    /// Activate the previous session.
    Previous(TargetArgs),
    /// Activate the next session.
    Next(TargetArgs),
    /// Reopen a closed session.
    ReopenClosed(TargetArgs),
}

/// Commands that inspect terminal blocks.
#[derive(Debug, Clone, Subcommand)]
pub enum BlockCommand {
    /// List terminal blocks.
    List(BlockListArgs),
    /// Inspect a selected block.
    Inspect(TargetArgs),
    /// Print block output.
    Output(BlockOutputArgs),
}

/// Commands that stage text in Warp input.
#[derive(Debug, Clone, Subcommand)]
pub enum InputCommand {
    /// Print the staged input buffer.
    Get(TargetArgs),
    /// Insert text without submitting it.
    Insert(InputTextArgs),
    /// Replace the staged input buffer without submitting it.
    Replace(InputTextArgs),
    /// Clear the staged input buffer.
    Clear(TargetArgs),
    /// Submit a command to the selected input.
    Run(InputRunArgs),
    /// Set terminal or agent input mode.
    #[command(subcommand)]
    Mode(InputModeCommand),
}

/// Commands that inspect command history.
#[derive(Debug, Clone, Subcommand)]
pub enum HistoryCommand {
    /// List command history entries.
    List(HistoryListArgs),
}

/// Commands that inspect and update theme settings.
#[derive(Debug, Clone, Subcommand)]
pub enum ThemeCommand {
    /// List available themes.
    List(TargetArgs),
    /// Print the current theme configuration.
    Get(TargetArgs),
    /// Set a fixed theme.
    Set(ThemeNameArgs),
    /// Set follow-system-theme behavior.
    #[command(subcommand)]
    System(ThemeSystemCommand),
    /// Set the light theme used when following the system theme.
    #[command(subcommand)]
    Light(ThemeNameCommand),
    /// Set the dark theme used when following the system theme.
    #[command(subcommand)]
    Dark(ThemeNameCommand),
}

/// Commands that inspect and update appearance controls.
#[derive(Debug, Clone, Subcommand)]
pub enum AppearanceCommand {
    /// Print appearance metadata.
    Get(TargetArgs),
    /// Control font size.
    #[command(subcommand)]
    FontSize(AppearanceStepCommand),
    /// Control UI zoom.
    #[command(subcommand)]
    Zoom(AppearanceStepCommand),
}

/// Commands that inspect and update allowlisted settings.
#[derive(Debug, Clone, Subcommand)]
pub enum SettingCommand {
    /// List allowlisted settings.
    List(SettingListArgs),
    /// Print one allowlisted setting.
    Get(SettingKeyArgs),
    /// Set one allowlisted setting.
    Set(SettingSetArgs),
    /// Toggle one allowlisted boolean setting.
    Toggle(SettingKeyArgs),
}

/// Commands that inspect keybinding metadata.
#[derive(Debug, Clone, Subcommand)]
pub enum KeybindingCommand {
    /// List keybindings.
    List(TargetArgs),
    /// Print one keybinding.
    Get(KeybindingGetArgs),
}

/// Commands that inspect action metadata.
#[derive(Debug, Clone, Subcommand)]
pub enum ActionCommand {
    /// List local-control actions.
    List(TargetArgs),
    /// Inspect one local-control action.
    Inspect(NamedActionArgs),
}

/// Commands that operate Warp's visible file/editor state.
#[derive(Debug, Clone, Subcommand)]
pub enum FileCommand {
    /// List files currently open in Warp editor state.
    List(TargetArgs),
    /// Open a path in Warp.
    Open(FileOpenArgs),
}

/// Commands that operate Warp projects and workspaces.
#[derive(Debug, Clone, Subcommand)]
pub enum ProjectCommand {
    /// Print the active project.
    Active(TargetArgs),
    /// List known projects in Warp app state.
    List(TargetArgs),
    /// Open a project path in Warp.
    Open(ProjectOpenArgs),
}

/// Commands that operate Warp Drive app surfaces and objects.
#[derive(Debug, Clone, Subcommand)]
pub enum DriveCommand {
    /// List Warp Drive objects by type.
    List(DriveListArgs),
    /// Inspect a Warp Drive object.
    Inspect(DriveObjectIdArgs),
    /// Open a Warp Drive object or surface.
    Open(DriveObjectIdArgs),
    /// Operate Warp Drive notebooks.
    #[command(subcommand)]
    Notebook(DriveObjectOpenCommand),
    /// Operate Warp Drive environment variable collections.
    #[command(name = "env-var-collection", subcommand)]
    EnvVarCollection(DriveObjectOpenCommand),
    /// Operate Warp Drive objects.
    #[command(subcommand)]
    Object(DriveObjectCommand),
    /// Operate Warp Drive workflows.
    #[command(subcommand)]
    Workflow(DriveWorkflowCommand),
}

/// Commands that open and toggle Warp surfaces.
#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceCommand {
    /// Open settings.
    #[command(subcommand)]
    Settings(SurfaceSettingsCommand),
    /// Open the command palette.
    #[command(name = "command-palette", subcommand)]
    CommandPalette(SurfaceQueryOpenCommand),
    /// Open command search.
    #[command(name = "command-search", subcommand)]
    CommandSearch(SurfaceQueryOpenCommand),
    /// Open or toggle Warp Drive.
    #[command(name = "warp-drive", subcommand)]
    WarpDrive(SurfaceOpenToggleCommand),
    /// Toggle the resource center.
    #[command(name = "resource-center", subcommand)]
    ResourceCenter(SurfaceToggleCommand),
    /// Toggle the AI assistant panel.
    #[command(name = "ai-assistant", subcommand)]
    AiAssistant(SurfaceToggleCommand),
    /// Toggle the code review panel.
    #[command(name = "code-review", subcommand)]
    CodeReview(SurfaceToggleCommand),
    /// Toggle the left panel.
    #[command(name = "left-panel", subcommand)]
    LeftPanel(SurfaceToggleCommand),
    /// Toggle the right panel.
    #[command(name = "right-panel", subcommand)]
    RightPanel(SurfaceToggleCommand),
    /// Toggle the vertical tabs panel.
    #[command(name = "vertical-tabs", subcommand)]
    VerticalTabs(SurfaceToggleCommand),
}

/// Common flags for selecting which running Warp instance and target receives a command.
#[derive(Debug, Clone, Args, Default)]
pub struct TargetArgs {
    /// Target a specific local Warp instance id from `warp instance list`.
    #[arg(long = "instance")]
    pub instance: Option<String>,
    /// Target a specific local Warp process id.
    #[arg(long = "pid", conflicts_with = "instance")]
    pub pid: Option<u32>,
    /// Select a window with `active`, `id:<id>`, `index:<n>`, or `title:<title>`.
    #[arg(
        long = "window",
        conflicts_with_all = ["window_id", "window_index", "window_title"]
    )]
    pub window: Option<String>,
    /// Select a window by opaque id.
    #[arg(long = "window-id", conflicts_with_all = ["window_index", "window_title"])]
    pub window_id: Option<String>,
    /// Select a window by scoped index.
    #[arg(long = "window-index", conflicts_with = "window_title")]
    pub window_index: Option<u32>,
    /// Select a window by exact title.
    #[arg(long = "window-title")]
    pub window_title: Option<String>,
    /// Select a tab with `active`, `id:<id>`, `index:<n>`, or `title:<title>`.
    #[arg(
        long = "tab",
        conflicts_with_all = ["tab_id", "tab_index", "tab_title"]
    )]
    pub tab: Option<String>,
    /// Select a tab by opaque id.
    #[arg(long = "tab-id", conflicts_with_all = ["tab_index", "tab_title"])]
    pub tab_id: Option<String>,
    /// Select a tab by scoped index.
    #[arg(long = "tab-index", conflicts_with = "tab_title")]
    pub tab_index: Option<u32>,
    /// Select a tab by exact title.
    #[arg(long = "tab-title")]
    pub tab_title: Option<String>,
    /// Select a pane with `active`, `id:<id>`, or `index:<n>`.
    #[arg(long = "pane", conflicts_with_all = ["pane_id", "pane_index"])]
    pub pane: Option<String>,
    /// Select a pane by opaque id.
    #[arg(long = "pane-id", conflicts_with = "pane_index")]
    pub pane_id: Option<String>,
    /// Select a pane by scoped index.
    #[arg(long = "pane-index")]
    pub pane_index: Option<u32>,
    /// Select a session with `active`, `id:<id>`, or `index:<n>`.
    #[arg(
        long = "session",
        conflicts_with_all = ["session_id", "session_index"]
    )]
    pub session: Option<String>,
    /// Select a session by opaque id.
    #[arg(long = "session-id", conflicts_with = "session_index")]
    pub session_id: Option<String>,
    /// Select a session by scoped index.
    #[arg(long = "session-index")]
    pub session_index: Option<u32>,
    /// Select a block with `active`, `id:<id>`, or `index:<n>`.
    #[arg(long = "block", conflicts_with_all = ["block_id", "block_index"])]
    pub block: Option<String>,
    /// Select a block by opaque id.
    #[arg(long = "block-id", conflicts_with = "block_index")]
    pub block_id: Option<String>,
    /// Select a block by scoped index.
    #[arg(long = "block-index")]
    pub block_index: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct NamedActionArgs {
    /// Action name from the local-control catalog.
    pub action: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct WindowCreateArgs {
    /// Optional shell/session profile name.
    #[arg(long = "shell")]
    pub shell: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct TabCreateArgs {
    /// Type of tab to create.
    #[arg(long = "type", value_enum, default_value_t = TabType::Terminal)]
    pub tab_type: TabType,
    /// Optional shell/session profile name.
    #[arg(long = "shell")]
    pub shell: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum TabType {
    #[value(name = "terminal")]
    Terminal,
    #[value(name = "agent")]
    Agent,
    #[value(name = "cloud-agent")]
    CloudAgent,
    #[value(name = "default")]
    Default,
}

#[derive(Debug, Clone, Args)]
pub struct TabActivateArgs {
    /// Activate the previous tab.
    #[arg(long = "previous", conflicts_with_all = ["next", "last"])]
    pub previous: bool,
    /// Activate the next tab.
    #[arg(long = "next", conflicts_with_all = ["previous", "last"])]
    pub next: bool,
    /// Activate the last active tab.
    #[arg(long = "last", conflicts_with_all = ["previous", "next"])]
    pub last: bool,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct TabMoveArgs {
    /// Direction to move the tab.
    #[arg(long = "direction", value_enum)]
    pub direction: HorizontalDirection,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct TabRenameArgs {
    /// New tab title.
    pub title: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TabColorCommand {
    /// Set a tab color.
    Set(TabColorSetArgs),
    /// Clear a tab color.
    Clear(TargetArgs),
}

#[derive(Debug, Clone, Args)]
pub struct TabColorSetArgs {
    /// Color name or value.
    pub color: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct TabCloseArgs {
    /// Close the active tab.
    #[arg(long = "active", conflicts_with_all = ["others", "right_of"])]
    pub active: bool,
    /// Close tabs other than the selected tab.
    #[arg(long = "others", conflicts_with_all = ["active", "right_of"])]
    pub others: bool,
    /// Close tabs to the right of the selected tab.
    #[arg(long = "right-of", conflicts_with_all = ["active", "others"])]
    pub right_of: bool,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PaneSplitArgs {
    /// Direction for the new pane.
    #[arg(long = "direction", value_enum)]
    pub direction: SplitDirection,
    /// Optional shell/session profile name.
    #[arg(long = "shell")]
    pub shell: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PaneNavigateArgs {
    /// Direction to navigate focus.
    #[arg(long = "direction", value_enum)]
    pub direction: NavigationDirection,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PaneResizeArgs {
    /// Direction to resize pane dividers.
    #[arg(long = "direction", value_enum)]
    pub direction: SplitDirection,
    /// Amount in terminal cells.
    #[arg(long = "amount")]
    pub amount: Option<u32>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PaneRenameArgs {
    /// New pane title.
    pub title: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct BlockListArgs {
    /// Maximum number of blocks to return.
    #[arg(long = "limit")]
    pub limit: Option<u32>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct BlockOutputArgs {
    /// Return plain text output.
    #[arg(long = "plain", conflicts_with_all = ["ansi", "json"])]
    pub plain: bool,
    /// Return ANSI-preserving output.
    #[arg(long = "ansi", conflicts_with_all = ["plain", "json"])]
    pub ansi: bool,
    /// Return structured block JSON.
    #[arg(long = "json", conflicts_with_all = ["plain", "ansi"])]
    pub json: bool,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct InputTextArgs {
    /// Text to stage.
    pub text: String,
    #[command(flatten)]
    pub target: TargetArgs,
}
#[derive(Debug, Clone, Args)]
pub struct InputRunArgs {
    /// Command to submit.
    pub command: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputModeCommand {
    /// Set the input mode.
    Set(InputModeSetArgs),
}

#[derive(Debug, Clone, Args)]
pub struct InputModeSetArgs {
    /// Input mode to set.
    #[arg(value_enum)]
    pub mode: InputMode,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum InputMode {
    #[value(name = "terminal")]
    Terminal,
    #[value(name = "agent")]
    Agent,
}

#[derive(Debug, Clone, Args)]
pub struct HistoryListArgs {
    /// Maximum number of history entries to return.
    #[arg(long = "limit")]
    pub limit: Option<u32>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct ThemeNameArgs {
    /// Theme name.
    pub theme_name: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ThemeSystemCommand {
    /// Set whether Warp follows the system theme.
    Set(ThemeSystemSetArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ThemeSystemSetArgs {
    /// Whether to follow the system theme.
    pub enabled: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ThemeNameCommand {
    /// Set this theme slot.
    Set(ThemeNameArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AppearanceStepCommand {
    /// Increase the value.
    Increase(TargetArgs),
    /// Decrease the value.
    Decrease(TargetArgs),
    /// Reset the value.
    Reset(TargetArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SettingListArgs {
    /// Optional setting namespace.
    #[arg(long = "namespace")]
    pub namespace: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct SettingKeyArgs {
    /// Setting key.
    pub key: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct SettingSetArgs {
    /// Setting key.
    pub key: String,
    /// Setting value.
    pub value: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct KeybindingGetArgs {
    /// Keybinding name.
    pub binding_name: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct FileOpenArgs {
    /// Path to open in Warp.
    pub path: PathBuf,
    /// Optional one-based line number.
    #[arg(long = "line")]
    pub line: Option<u32>,
    /// Optional one-based column number.
    #[arg(long = "column")]
    pub column: Option<u32>,
    /// Open in a new tab.
    #[arg(long = "new-tab")]
    pub new_tab: bool,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct ProjectOpenArgs {
    /// Project path to open in Warp.
    pub path: PathBuf,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct DriveListArgs {
    /// Warp Drive object type.
    #[arg(long = "type", value_enum)]
    pub object_type: DriveObjectType,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum DriveObjectType {
    #[value(name = "workflow")]
    Workflow,
    #[value(name = "notebook")]
    Notebook,
    #[value(name = "env-var-collection")]
    EnvVarCollection,
    #[value(name = "prompt")]
    Prompt,
    #[value(name = "folder")]
    Folder,
    #[value(name = "ai-fact")]
    AiFact,
    #[value(name = "mcp-server")]
    McpServer,
    #[value(name = "space")]
    Space,
    #[value(name = "trash")]
    Trash,
}

#[derive(Debug, Clone, Args)]
pub struct DriveObjectIdArgs {
    /// Warp Drive object id.
    pub id: String,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DriveObjectOpenCommand {
    /// Open an object.
    Open(DriveObjectIdArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum DriveObjectCommand {
    /// Open the sharing dialog for user review.
    #[command(subcommand)]
    Share(DriveObjectShareCommand),
    /// Create a Warp Drive object.
    Create(DriveObjectCreateArgs),
    /// Update a Warp Drive object.
    Update(DriveObjectContentArgs),
    /// Delete a Warp Drive object.
    Delete(DriveObjectIdArgs),
    /// Insert a Warp Drive object into a target surface.
    Insert(DriveObjectInsertArgs),
    /// Share a personal object to the current user's team.
    ShareToTeam(DriveObjectIdArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum DriveObjectShareCommand {
    /// Open the share dialog.
    Open(DriveObjectIdArgs),
}

#[derive(Debug, Clone, Args)]
pub struct DriveObjectCreateArgs {
    /// Warp Drive object type.
    #[arg(long = "type", value_enum)]
    pub object_type: DriveObjectType,
    /// Inline content.
    #[arg(long = "content", conflicts_with = "content_file")]
    pub content: Option<String>,
    /// Path to content that the app-side Drive mutation may read after auth.
    #[arg(long = "content-file")]
    pub content_file: Option<PathBuf>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct DriveObjectContentArgs {
    /// Warp Drive object id.
    pub id: String,
    /// Inline content.
    #[arg(long = "content", conflicts_with = "content_file")]
    pub content: Option<String>,
    /// Path to content that the app-side Drive mutation may read after auth.
    #[arg(long = "content-file")]
    pub content_file: Option<PathBuf>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct DriveObjectInsertArgs {
    /// Warp Drive object id.
    pub id: String,
    /// Target selector for insertion.
    #[arg(long = "target")]
    pub insert_target: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DriveWorkflowCommand {
    /// Run a typed workflow.
    Run(DriveWorkflowRunArgs),
}

#[derive(Debug, Clone, Args)]
pub struct DriveWorkflowRunArgs {
    /// Warp Drive workflow id.
    pub id: String,
    /// Workflow argument in name=value form.
    #[arg(long = "arg")]
    pub args: Vec<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceSettingsCommand {
    /// Open settings.
    Open(SurfaceSettingsOpenArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SurfaceSettingsOpenArgs {
    /// Optional settings page.
    #[arg(long = "page")]
    pub page: Option<String>,
    /// Optional settings search query.
    #[arg(long = "query")]
    pub query: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceQueryOpenCommand {
    /// Open this surface.
    Open(SurfaceQueryArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SurfaceQueryArgs {
    /// Optional initial query.
    #[arg(long = "query")]
    pub query: Option<String>,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceOpenToggleCommand {
    /// Open this surface.
    Open(TargetArgs),
    /// Toggle this surface.
    Toggle(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceToggleCommand {
    /// Toggle this surface.
    Toggle(TargetArgs),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum HorizontalDirection {
    #[value(name = "left")]
    Left,
    #[value(name = "right")]
    Right,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum SplitDirection {
    #[value(name = "left")]
    Left,
    #[value(name = "right")]
    Right,
    #[value(name = "up")]
    Up,
    #[value(name = "down")]
    Down,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
pub enum NavigationDirection {
    #[value(name = "left")]
    Left,
    #[value(name = "right")]
    Right,
    #[value(name = "up")]
    Up,
    #[value(name = "down")]
    Down,
    #[value(name = "previous")]
    Previous,
    #[value(name = "next")]
    Next,
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
        ControlCommand::Keybinding(command) => run_keybinding_command(command),
        ControlCommand::Action(command) => run_action_command(command, output_format),
        ControlCommand::File(command) => run_file_command(command, output_format),
        ControlCommand::Project(command) => run_project_command(command, output_format),
        ControlCommand::Drive(command) => run_drive_command(command, output_format),
        ControlCommand::Surface(command) => run_surface_command(command, output_format),
        ControlCommand::Auth(command) => run_auth_command(command, output_format),
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
