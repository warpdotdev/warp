//! Command-line interface for controlling a running local Warp app.
mod commands;
mod completions;
mod output;
mod selectors;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::aot::Shell;
use commands::{
    run_action_catalog_command, run_app_command, run_appearance_command, run_capability_command,
    run_file_command, run_input_command, run_instance_command, run_keybinding_command,
    run_pane_command, run_session_command, run_setting_command, run_surface_command,
    run_tab_command, run_theme_command, run_window_command,
};
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

/// Commands that inspect the public action catalog.
#[derive(Debug, Clone, Subcommand)]
pub enum ActionCatalogCommand {
    /// List allowlisted catalog actions.
    List,

    /// Inspect a single allowlisted catalog action.
    Inspect {
        /// Canonical action name, such as `tab.create` or `surface.settings.open`.
        action: String,
    },
}

impl ControlArgs {
    pub fn from_env() -> Self {
        let bin_name = crate::binary_name().unwrap_or_else(|| "warpctrl".to_owned());
        Self::try_parse_from_args(std::env::args_os(), bin_name).unwrap_or_else(|err| err.exit())
    }

    /// Parse Warp Control arguments only when the wrapper-injected mode flag is present.
    ///
    /// Startup calls this before the normal Warp/Oz parser. Arguments through
    /// `--warpctrl` are removed, and the remaining arguments are parsed as if
    /// the standalone command name were `warpctrl`.
    pub fn from_control_mode_env() -> Option<Self> {
        Self::try_parse_control_mode_from(std::env::args_os())
            .map(|result| result.unwrap_or_else(|err| err.exit()))
    }

    /// Testable implementation of [`Self::from_control_mode_env`].
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
  <dim>$</dim> <bold>{bin_name} action list</bold>

  <dim>$</dim> <bold>{bin_name} action inspect surface.settings.open</bold>

<bold><underline>Learn more:</underline></bold>
* Use <bold>{bin_name} help</bold> to learn more about each command
* Use <bold>{bin_name} action list</bold> to inspect allowlisted actions
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
    /// Inspect public action metadata and implementation status.
    #[command(subcommand)]
    Action(ActionCatalogCommand),

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

    /// Inspect terminal input state.
    #[command(subcommand)]
    Input(InputCommand),

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

    /// Open or toggle local Warp surfaces.
    #[command(subcommand)]
    Surface(SurfaceCommand),

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

    /// Print protocol and build identity metadata for the selected local Warp app.
    Version(TargetArgs),

    /// Print the active window/tab/pane/session chain.
    Active(TargetArgs),

    /// Focus the selected local Warp app.
    Focus(TargetArgs),
}

/// Commands that inspect public local-control capabilities.
#[derive(Debug, Clone, Subcommand)]
pub enum CapabilityCommand {
    /// List allowlisted local-control capabilities.
    List,

    /// Inspect a single local-control capability by canonical action name.
    Inspect {
        /// Canonical action name, such as `tab.create` or `surface.settings.open`.
        action: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowCommand {
    /// List windows in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one window in the selected local Warp app.
    Inspect(TargetArgs),

    /// Create a new window.
    Create(TabCreateArgs),

    /// Focus a window.
    Focus(TargetArgs),

    /// Close a window.
    Close(TargetArgs),
}

/// Commands that control tabs in the selected Warp app instance.
#[derive(Debug, Clone, Subcommand)]
pub enum TabCommand {
    /// List tabs in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one tab in the selected local Warp app.
    Inspect(TargetArgs),

    /// Create a new terminal tab in the active window.
    Create(TabCreateArgs),

    /// Activate a tab.
    Activate(TabActivateArgs),

    /// Move the active tab.
    Move(TabMoveArgs),

    /// Close tabs.
    Close(TabCloseArgs),

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

/// Commands that inspect local Warp panes.
#[derive(Debug, Clone, Subcommand)]
pub enum PaneCommand {
    /// List panes in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one pane in the selected local Warp app.
    Inspect(TargetArgs),

    /// Split the active pane.
    Split(PaneSplitArgs),

    /// Focus a pane.
    Focus(TargetArgs),

    /// Navigate between panes.
    Navigate(PaneNavigateArgs),

    /// Resize the active pane.
    Resize(PaneResizeArgs),

    /// Maximize the active pane.
    Maximize(TargetArgs),

    /// Unmaximize the active pane.
    Unmaximize(TargetArgs),

    /// Close the active pane.
    Close(TargetArgs),

    /// Rename a pane.
    Rename(RenameArgs),

    /// Reset a pane name.
    ResetName(TargetArgs),
}

/// Commands that inspect local Warp sessions.
#[derive(Debug, Clone, Subcommand)]
pub enum SessionCommand {
    /// List sessions in the selected local Warp app.
    List(TargetArgs),

    /// Inspect one session in the selected local Warp app.
    Inspect(TargetArgs),

    /// Activate a session.
    Activate(TargetArgs),

    /// Activate the previous session.
    Previous(TargetArgs),

    /// Activate the next session.
    Next(TargetArgs),

    /// Reopen the most recently closed session.
    ReopenClosed(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputCommand {
    /// Insert text into the input buffer without submitting it.
    Insert(TextTargetArgs),

    /// Replace the input buffer without submitting it.
    Replace(TextTargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceCommand {
    /// List available and unavailable tour surfaces.
    List(TargetArgs),
    /// Open settings surfaces.
    #[command(subcommand)]
    Settings(SurfaceSettingsCommand),

    /// Open the command palette.
    #[command(subcommand)]
    CommandPalette(SurfaceQueryCommand),

    /// Open command search.
    #[command(subcommand)]
    CommandSearch(SurfaceQueryCommand),
    /// Open the theme picker.
    #[command(subcommand)]
    ThemePicker(SurfaceOpenCommand),

    /// Open keybinding settings.
    #[command(subcommand)]
    Keybindings(SurfaceOpenCommand),

    /// Open or toggle Warp Drive.
    #[command(subcommand)]
    WarpDrive(SurfaceOpenToggleCommand),

    /// Toggle the resource center.
    #[command(subcommand)]
    ResourceCenter(SurfaceToggleCommand),

    /// Toggle the AI assistant.
    #[command(subcommand)]
    AiAssistant(SurfaceToggleCommand),

    /// Open or toggle code review.
    #[command(subcommand)]
    CodeReview(SurfaceOpenToggleCommand),

    /// Open the project explorer.
    #[command(subcommand)]
    ProjectExplorer(SurfaceOpenCommand),

    /// Open global search.
    #[command(subcommand)]
    GlobalSearch(SurfaceOpenCommand),

    /// Open the conversation list.
    #[command(subcommand)]
    ConversationList(SurfaceOpenCommand),

    /// Toggle the left panel.
    #[command(subcommand)]
    LeftPanel(SurfaceToggleCommand),

    /// Toggle the right panel.
    #[command(subcommand)]
    RightPanel(SurfaceToggleCommand),

    /// Open or toggle vertical tabs.
    #[command(subcommand)]
    VerticalTabs(SurfaceOpenToggleCommand),

    /// Open agent management.
    #[command(subcommand)]
    AgentManagement(SurfaceOpenCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceSettingsCommand {
    /// Open Settings, optionally scoped to a page or query.
    Open(PageQueryArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceQueryCommand {
    /// Open the surface with an optional seeded query.
    Open(QueryArgs),
}
#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceOpenCommand {
    /// Open the surface.
    Open(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceOpenToggleCommand {
    /// Open the surface.
    Open(TargetArgs),

    /// Toggle the surface.
    Toggle(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SurfaceToggleCommand {
    /// Toggle the surface.
    Toggle(TargetArgs),
}

/// Commands that inspect Warp themes.
#[derive(Debug, Clone, Subcommand)]
pub enum ThemeCommand {
    /// List available themes.
    List(TargetArgs),

    /// Read current theme state.
    Get(TargetArgs),

    /// Set the current theme.
    Set(ThemeSetArgs),

    /// Set whether Warp follows the system theme.
    SystemSet(ThemeSystemSetArgs),

    /// Set the light theme used when following the system theme.
    LightSet(ThemeSetArgs),

    /// Set the dark theme used when following the system theme.
    DarkSet(ThemeSetArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AppearanceCommand {
    /// Read appearance state.
    Get(TargetArgs),

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

#[derive(Debug, Clone, Subcommand)]
pub enum SettingCommand {
    /// List allowlisted settings.
    List(NamespaceTargetArgs),

    /// Read one allowlisted setting.
    Get(SettingGetArgs),

    /// Set one allowlisted setting.
    Set(SettingSetArgs),

    /// Toggle one allowlisted boolean setting.
    Toggle(SettingToggleArgs),
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
    /// Open a file in Warp.
    Open(FileOpenArgs),
}

/// Exact selectors for a target within the selected Warp instance.
#[derive(Debug, Clone, Args, Default)]
pub struct TargetArgs {
    /// Target a specific local Warp instance id from `warpctrl instance list`.
    #[arg(long = "instance", conflicts_with = "pid")]
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
pub struct TabCreateArgs {
    #[arg(long = "type", value_enum)]
    pub tab_type: Option<CliTabType>,

    #[arg(long = "shell")]
    pub shell: Option<String>,

    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct TabActivateArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "previous", conflicts_with_all = ["next", "last"])]
    pub previous: bool,

    #[arg(long = "next", conflicts_with_all = ["previous", "last"])]
    pub next: bool,

    #[arg(long = "last", conflicts_with_all = ["previous", "next"])]
    pub last: bool,
}

#[derive(Debug, Clone, Args)]
pub struct TabMoveArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "direction", value_enum)]
    pub direction: CliTabMoveDirection,
}

#[derive(Debug, Clone, Args)]
pub struct TabCloseArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "active", conflicts_with_all = ["others", "right_of"])]
    pub active: bool,

    #[arg(long = "others", conflicts_with_all = ["active", "right_of"])]
    pub others: bool,

    #[arg(long = "right-of", conflicts_with_all = ["active", "others"])]
    pub right_of: bool,
}

#[derive(Debug, Clone, Args)]
pub struct PaneSplitArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "direction", value_enum)]
    pub direction: CliCardinalDirection,
}

#[derive(Debug, Clone, Args)]
pub struct PaneNavigateArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "direction", value_enum)]
    pub direction: CliDirection,
}

#[derive(Debug, Clone, Args)]
pub struct PaneResizeArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "direction", value_enum)]
    pub direction: CliCardinalDirection,

    #[arg(long = "amount")]
    pub amount: Option<u32>,
}

#[derive(Debug, Clone, Args)]
pub struct TextTargetArgs {
    pub text: String,

    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PageQueryArgs {
    #[arg(long = "page")]
    pub page: Option<String>,

    #[arg(long = "query")]
    pub query: Option<String>,

    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct QueryArgs {
    #[arg(long = "query")]
    pub query: Option<String>,

    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Debug, Clone, Args)]
pub struct FileOpenArgs {
    pub path: String,

    #[arg(long = "line")]
    pub line: Option<u32>,

    #[arg(long = "column")]
    pub column: Option<u32>,

    #[arg(long = "new-tab")]
    pub new_tab: bool,

    #[command(flatten)]
    pub target: TargetArgs,
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

#[derive(Debug, Clone, Args)]
pub struct NamespaceTargetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[arg(long = "namespace")]
    pub namespace: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliTabType {
    Terminal,
    Agent,
    CloudAgent,
    Default,
}

impl From<CliTabType> for local_control::protocol::TabType {
    fn from(value: CliTabType) -> Self {
        match value {
            CliTabType::Terminal => Self::Terminal,
            CliTabType::Agent => Self::Agent,
            CliTabType::CloudAgent => Self::CloudAgent,
            CliTabType::Default => Self::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliCardinalDirection {
    Left,
    Right,
    Up,
    Down,
}

impl From<CliCardinalDirection> for local_control::protocol::Direction {
    fn from(value: CliCardinalDirection) -> Self {
        match value {
            CliCardinalDirection::Left => Self::Left,
            CliCardinalDirection::Right => Self::Right,
            CliCardinalDirection::Up => Self::Up,
            CliCardinalDirection::Down => Self::Down,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliDirection {
    Left,
    Right,
    Up,
    Down,
    Previous,
    Next,
}

impl From<CliDirection> for local_control::protocol::Direction {
    fn from(value: CliDirection) -> Self {
        match value {
            CliDirection::Left => Self::Left,
            CliDirection::Right => Self::Right,
            CliDirection::Up => Self::Up,
            CliDirection::Down => Self::Down,
            CliDirection::Previous => Self::Previous,
            CliDirection::Next => Self::Next,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliTabMoveDirection {
    Left,
    Right,
    Previous,
    Next,
}

impl From<CliTabMoveDirection> for local_control::protocol::Direction {
    fn from(value: CliTabMoveDirection) -> Self {
        match value {
            CliTabMoveDirection::Left => Self::Left,
            CliTabMoveDirection::Right => Self::Right,
            CliTabMoveDirection::Previous => Self::Previous,
            CliTabMoveDirection::Next => Self::Next,
        }
    }
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
        ControlCommand::Capability(command) => run_capability_command(command, output_format),
        ControlCommand::Action(command) => run_action_catalog_command(command, output_format),
        ControlCommand::Window(command) => run_window_command(command, output_format),
        ControlCommand::Tab(command) => run_tab_command(command, output_format),
        ControlCommand::Pane(command) => run_pane_command(command, output_format),
        ControlCommand::Session(command) => run_session_command(command, output_format),
        ControlCommand::Input(command) => run_input_command(command, output_format),
        ControlCommand::Theme(command) => run_theme_command(command, output_format),
        ControlCommand::Appearance(command) => run_appearance_command(command, output_format),
        ControlCommand::Setting(command) => run_setting_command(command, output_format),
        ControlCommand::Keybinding(command) => run_keybinding_command(command, output_format),
        ControlCommand::File(command) => run_file_command(command, output_format),
        ControlCommand::Surface(command) => run_surface_command(command, output_format),
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
