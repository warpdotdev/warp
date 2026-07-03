// Suppress warnings about rustdoc style.
#![allow(clippy::doc_lazy_continuation)]

mod ai;
mod alloc;
mod antivirus;
#[cfg(target_os = "macos")]
mod app_menus;
mod app_services;
mod app_state;
mod auth;
mod banner;
mod chip_configurator;
mod code;
mod code_review;
mod coding_entrypoints;
mod coding_panel_enablement_state;
mod command_palette;
mod completer;
#[allow(dead_code)]
mod context_chips;
#[cfg(enable_crash_recovery)]
mod crash_recovery;
mod debounce;
mod debug_dump;
mod default_terminal;
#[cfg(windows)]
mod dynamic_libraries;
mod env_vars;
mod experiments;
mod external_secrets;
mod global_resource_handles;
mod gpu_state;
mod input_classifier;
mod interval_timer;
mod linear;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod login_item;
mod menu;
mod modal;
mod network;
mod notebooks;
mod notification;
mod palette;
mod persistence;
mod platform;
#[cfg(feature = "plugin_host")]
mod plugin;
mod prefix;
#[cfg(target_os = "macos")]
mod preview_config_migration;
mod profiling;
mod projects;
mod prompt;
mod quit_warning;
mod resource_limits;
mod safe_triangle;
mod search_bar;
mod server;
mod session_management;
mod shell_indicator;
mod suggestions;
mod system;
mod tab;
#[cfg(test)]
mod test_util;
mod throttle;
mod tips;
mod tracing;
mod ui_components;
mod undo_close;
mod uri;
mod user_config;
pub mod util;
mod view_components;
mod vim_registers;
mod voice;
mod voltron;
mod warp_managed_paths_watcher;
mod window_settings;
mod workspaces;

// PLEASE DO NOT ADD MORE PUBLIC MODULES!
//
// Any modules which we make public outside of the `warp` crate lose dead code
// checking support, as the compiler cannot make any assumptions about whether
// or not the function/type is used by another crate that pulls in this one as
// a dependency.
//
// If you feel the need to export a module so that a type or function within it
// can be used by an integration test, you should define a new assertion function
// in the warp::integration_testing::assertions module (or a sub-module).  These
// functions will allow us to keep types internal to this crate and expose a
// simpler API for integration tests to consume.
pub mod appearance;
pub mod channel;
pub mod editor;
pub mod features;
pub mod input_suggestions;
#[cfg(feature = "integration_tests")]
pub mod integration_testing;
pub mod keyboard;
pub mod launch_configs;
pub mod pane_group;
pub mod resource_center;
pub mod root_view;
pub mod search;
pub mod settings;
pub mod settings_view;
pub mod tab_configs;
pub mod terminal;
pub mod themes;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::aws_credentials::AwsCredentialRefresher as _;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::mcp::FileBasedMCPManager;
use crate::ai::mcp::FileMCPWatcher;
use ::ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use ::ai::index::full_source_code_embedding::store_client::MockStoreClient;
use ::ai::index::full_source_code_embedding::SyncTask;
use ::ai::index::DEFAULT_SYNC_REQUESTS_PER_MIN;
use ::ai::project_context::model::ProjectContextModel;
pub use ai::agent::{todos::AIAgentTodoList, AIAgentActionResultType, FileEdit, TodoOperation};
use ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use ai::execution_profiles::editor::ExecutionProfileEditorManager;
use ai::execution_profiles::profiles::AIExecutionProfilesModel;
use ai::persisted_workspace::PersistedWorkspace;
use auth::{LocalActorIdentity, LocalActorIdentityProvider};
use code::editor_management::CodeManager;
use code::opened_files::OpenedFilesModel;
use code_review::GlobalCodeReviewModel;
use quit_warning::UnsavedStateSummary;
#[cfg(feature = "local_fs")]
use settings::import::model::ImportedConfigModel;
use warp_cli::GlobalOptions;
use warp_cli::{agent::AgentCommand, CliCommand};

#[cfg(feature = "local_fs")]
use repo_metadata::{
    repositories::DetectedRepositories, watcher::DirectoryWatcher, RepoMetadataModel,
};
#[cfg(feature = "local_fs")]
use watcher::HomeDirectoryWatcher;

use settings_view::pane_manager::SettingsPaneManager;
use terminal::general_settings::GeneralSettings;
use terminal::keys_settings::KeysSettings;
#[cfg(feature = "local_tty")]
use terminal::local_shell::LocalShellState;
pub use util::bindings::cmd_or_ctrl_shift;
pub mod workflows;
pub mod workspace;

#[cfg(feature = "integration_tests")]
pub use persistence::testing as sqlite_testing;

use ::settings::{Setting, ToggleableSetting};
pub use warp_core::errors::{report_error, report_if_error};

#[cfg(feature = "plugin_host")]
pub use plugin::{run_plugin_host, PLUGIN_HOST_FLAG};
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::platform::app::ApproveTerminateResult;
use window_settings::WindowSettings;

use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::outline::RepoOutlines;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::code::global_buffer_model::GlobalBufferModel;
#[cfg(feature = "local_fs")]
use crate::code::language_server_shutdown_manager::LanguageServerShutdownManager;
use crate::context_chips::prompt::Prompt;
use crate::default_terminal::DefaultTerminal;
use crate::gpu_state::GPUState;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::palette::PaletteMode;
use crate::persistence::PersistenceWriter;
use crate::projects::ProjectManagementModel;
use crate::session_management::{RunningSessionSummary, SessionNavigationData};
use crate::settings::manager::SettingsManager;
use crate::settings::{AccessibilitySettings, ScrollSettings, SelectionSettings};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::DisplayCount;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::SystemStats;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::view::inline_banner::ByoLlmAuthBannerSessionState;
use crate::terminal::{AudibleBell, History};
use crate::undo_close::UndoCloseStack;
use crate::user_config::WarpConfig;
use crate::vim_registers::VimRegisters;
use crate::warp_managed_paths_watcher::{ensure_warp_watch_roots_exist, WarpManagedPathsWatcher};
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspace::{ActiveSession, ToastStack};
#[cfg(feature = "local_tty")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use appearance::{Appearance, AppearanceManager};
use channel::ChannelState;
use interval_timer::IntervalTimer;
use itertools::Itertools;
use rust_embed::RustEmbed;
use settings::{ExtraMetaKeys, PrivacySettings};
use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;
use terminal::input;
use terminal::session_settings::SessionSettings;
use url::Url;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
use workspace::sync_inputs::SyncedInputState;

use warpui::{integration::TestDriver, App, AssetProvider, Event};

use self::features::FeatureFlag;
pub use crate::ai::blocklist::metadata::{AgentModeEntrypoint, AgentModeEntrypointSelectionType};
use crate::app_state::AppState;
use crate::experiments::ImprovedPaletteSearch;
pub use crate::global_resource_handles::{GlobalResourceHandles, GlobalResourceHandlesProvider};
use crate::notification::NotificationContext;
use crate::root_view::{
    quake_mode_window_id, quake_mode_window_is_open, OpenFromRestoredArg, OpenPath,
};
use crate::terminal::CustomSecretRegexUpdater;
use crate::util::bindings::is_binding_cross_platform;
use crate::workspace::metadata::PaletteSource;
use crate::workspace::{PaneViewLocator, Workspace, WorkspaceAction};
use crate::workspaces::user_workspaces::UserWorkspaces;
use warp_logging::LogDestination;

// Re-export the safe logging macros at the crate root level for backwards compatibility
pub use warp_core::{safe_debug, safe_error, safe_info, safe_warn};

use crate::antivirus::AntivirusInfo;
#[cfg(feature = "local_fs")]
use warp_files::FileModel;
use warpui::platform::TerminationMode;
use warpui::windowing::state::ApplicationStage;
use warpui::{AppContext, SingletonEntity, WindowId};

#[derive(Clone, Copy, RustEmbed)]
#[folder = "assets"]
#[include = "bundled/**"] // Should be kept in sync with BUNDLED_ASSETS_DIR.
#[include = "async/**"]
// Should be kept in sync with ASYNC_ASSETS_DIR.
// Excludes take precedence.
// Standalone CLI builds are headless and never render the
// theme imagery in `async/`, so we exclude those bytes from the
// embedded asset set to keep the CLI binary small — mirroring the carve-out
#[cfg_attr(feature = "standalone", exclude = "async/**")]
pub struct Assets;

pub static ASSETS: Assets = Assets;

/// Launch mode for how to start up Warp.
#[allow(clippy::large_enum_variant)]
pub enum LaunchMode {
    /// Run the regular GUI application.
    App { args: warp_cli::AppArgs },

    /// Run the Warp command-line SDK.
    CommandLine {
        command: warp_cli::CliCommand,
        global_options: GlobalOptions,
        debug: bool,
        /// Whether this CLI invocation is running in a sandboxed environment.
        is_sandboxed: bool,
        /// Override for computer use permission from CLI flags. If None, uses default behavior.
        computer_use_override: Option<bool>,
    },
    /// Run a test - this may be an integration test or an eval.
    Test {
        driver: Box<Option<TestDriver>>,
        is_integration_test: bool,
    },
}

impl LaunchMode {
    fn args(&self) -> Cow<'_, warp_cli::AppArgs> {
        match self {
            LaunchMode::App { args, .. } => Cow::Borrowed(args),
            _ => Cow::Owned(warp_cli::AppArgs::default()),
        }
    }

    /// Returns `true` if this process is running an integration test.
    fn is_integration_test(&self) -> bool {
        match self {
            LaunchMode::Test {
                is_integration_test,
                ..
            } => *is_integration_test,
            _ => false,
        }
    }

    fn take_test_driver(&mut self) -> Option<TestDriver> {
        match self {
            LaunchMode::Test { driver, .. } => driver.take(),
            _ => None,
        }
    }

    /// Add an URL to open. Only supported for [`LaunchMode::App`]
    #[allow(dead_code)]
    fn add_url(&mut self, url: Url) {
        if let LaunchMode::App { ref mut args, .. } = self {
            args.urls.push(url);
        }
    }

    fn execution_mode(&self) -> ExecutionMode {
        match self {
            LaunchMode::App { .. } => ExecutionMode::App,
            LaunchMode::CommandLine { .. } => ExecutionMode::Sdk,
            LaunchMode::Test { .. } => ExecutionMode::App,
        }
    }

    fn is_sandboxed(&self) -> bool {
        match self {
            LaunchMode::CommandLine { is_sandboxed, .. } => *is_sandboxed,
            _ => false,
        }
    }

    /// Returns `true` if Warp should run headlessly, without a visible UI.
    fn is_headless(&self) -> bool {
        match self {
            LaunchMode::CommandLine { command, .. } => match command {
                CliCommand::Agent(AgentCommand::Run(args)) => !args.gui,
                _ => true,
            },
            _ => false,
        }
    }

    /// Whether or not to start a crash recovery process (on platforms that support it).
    #[cfg(enable_crash_recovery)]
    pub(crate) fn crash_recovery_enabled(&self) -> bool {
        match self {
            LaunchMode::App { .. } => true,
            LaunchMode::CommandLine { .. } => false,
            LaunchMode::Test { .. } => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_unit_test() -> Self {
        LaunchMode::Test {
            driver: Box::new(None),
            is_integration_test: false,
        }
    }
}

impl AssetProvider for Assets {
    fn get(&self, path: &str) -> Result<Cow<'_, [u8]>> {
        <Assets as RustEmbed>::get(path)
            .map(|f| f.data)
            .ok_or_else(|| anyhow!("no asset exists at path {}", path))
    }
}

/// If the given event is a key down event containing alt modifiers, and those
/// alt modifiers should be treated as meta keys, then remove the alts and
/// prefix the keys with an escape. See WAR-472.
fn apply_extra_meta_keys(event: &mut Event, extra_metas: ExtraMetaKeys) {
    if let Event::KeyDown {
        keystroke, details, ..
    } = event
    {
        let left_as_meta = extra_metas.left_alt && details.left_alt;
        let right_as_meta = extra_metas.right_alt && details.right_alt;
        if left_as_meta || right_as_meta {
            let side = match (left_as_meta, right_as_meta) {
                (true, true) => "left+right alt",
                (true, false) => "left alt",
                (false, true) => "right alt",
                (false, false) => unreachable!(),
            };
            log::info!("Treating {side} as meta");
            keystroke.alt = false;
            keystroke.meta = true;
        }
    }
}

fn apply_scroll_multiplier(event: &mut Event, app: &AppContext) {
    if let Event::ScrollWheel { delta, precise, .. } = event {
        if !*precise {
            let scroll_multiplier = *ScrollSettings::as_ref(app).mouse_scroll_multiplier.value();
            *delta *= scroll_multiplier;
        }
    }
}

/// Runs the app. If a subcommand was requested, it'll be run instead of the main application.
pub fn run() -> Result<()> {
    // Perform any necessary platform-specific initialization.
    platform::init();

    // Ensure feature flags are initialized before parsing command-line arguments.
    init_feature_flags();

    // Parse command-line arguments.
    let args = warp_cli::Args::from_env();

    if let Some(command) = args.command() {
        #[cfg(windows)]
        if command.prints_to_stdout() {
            // We attach a console to ensure that all standard output gets printed correctly.
            warp_util::windows::attach_to_parent_console();
        }
        match command {
            #[cfg(all(feature = "local_tty", unix))]
            warp_cli::Command::Worker(warp_cli::WorkerCommand::TerminalServer(args)) => {
                // If we were asked to run as a terminal server (as opposed to the main
                // GUI application), do so immediately.  Ideally, the terminal server would
                // be a separate binary, but it's much easier to distribute a single binary,
                // so starting the terminal server event loop immediately is the closest
                // approximation we can get to running a separate binary.
                crate::terminal::local_tty::server::run_terminal_server(args);
                return Ok(());
            }
            #[cfg(feature = "plugin_host")]
            warp_cli::Command::Worker(warp_cli::WorkerCommand::PluginHost { .. }) => {
                return crate::run_plugin_host();
            }
            #[cfg(feature = "local_tty")]
            warp_cli::Command::Worker(warp_cli::WorkerCommand::MinidumpServer { socket_name }) => {
                let _ = socket_name;
                panic!("The minidump server is not supported in Warper");
            }
            warp_cli::Command::Worker(warp_cli::WorkerCommand::RipgrepSearch {
                parent,
                ignore_case,
                multiline,
                pattern,
                paths,
            }) => {
                warp_ripgrep::search::run_search_subprocess(
                    std::slice::from_ref(pattern),
                    paths.clone(),
                    *ignore_case,
                    *multiline,
                    parent.pid,
                )
                .map_err(|err| anyhow!(err.to_string()))?;
                return Ok(());
            }
            warp_cli::Command::Completions { shell } => {
                return warp_cli::completions::generate_to_stdout(*shell);
            }
            warp_cli::Command::CommandLine(cmd) => {
                let (is_sandboxed, computer_use_override) = match cmd.as_ref() {
                    warp_cli::CliCommand::Agent(warp_cli::agent::AgentCommand::Run(run_args)) => {
                        (false, run_args.computer_use.computer_use_override())
                    }
                    _ => (false, None),
                };

                return run_internal(LaunchMode::CommandLine {
                    command: cmd.as_ref().clone(),
                    global_options: GlobalOptions {
                        output_format: args.output_format(),
                        api_key: args.api_key().cloned(),
                    },
                    debug: args.debug(),
                    is_sandboxed,
                    computer_use_override,
                });
            }
            warp_cli::Command::DumpDebugInfo => {
                return debug_dump::run();
            }
            #[cfg(all(feature = "local_tty", unix))]
            warp_cli::Command::WarperLocalTerminalSmoke => {
                return terminal::local_tty::spawner::run_local_terminal_smoke();
            }
            #[cfg(not(all(feature = "local_tty", unix)))]
            warp_cli::Command::WarperLocalTerminalSmoke => {
                anyhow::bail!("local terminal smoke is only supported on Unix local_tty builds");
            }
        }
    }

    // If running as a standalone CLI binary, print help
    // instead of launching the GUI app.
    let is_cli_binary = cfg!(feature = "standalone") || std::env::var_os("WARP_CLI_MODE").is_some();
    if is_cli_binary {
        warp_cli::Args::clap_command().print_help()?;
        return Ok(());
    }

    run_internal(LaunchMode::App {
        args: args.into_app_args(),
    })
}

/// Runs an integration test using the provided test driver.
pub fn run_integration_test(driver: TestDriver) -> Result<()> {
    let is_integration_test = std::env::var("WARP_INTEGRATION").is_ok();
    let launch = LaunchMode::Test {
        driver: Box::new(Some(driver)),
        is_integration_test,
    };
    run_internal(launch)
}

/// Runs the app.
fn run_internal(mut launch_mode: LaunchMode) -> Result<()> {
    #[cfg(windows)]
    dynamic_libraries::configure_library_loading();

    profiling::init();

    // The `run` function already initializes feature flags, but ensure they're initialized here
    // for other entrypoints.
    init_feature_flags();

    let mut timer = IntervalTimer::new();

    tracing::init()?;

    let is_cli = matches!(launch_mode, LaunchMode::CommandLine { .. });

    // Log to a file for CLI commands to not obscure the command output.
    let log_destination = match &launch_mode {
        LaunchMode::CommandLine { debug, .. } => {
            if *debug {
                Some(LogDestination::Stderr)
            } else {
                Some(LogDestination::File)
            }
        }
        _ => None,
    };

    cfg_if::cfg_if! {
        if #[cfg(enable_crash_recovery)] {
            if crash_recovery::is_crash_recovery_process(launch_mode.args().as_ref()) {
                warp_logging::init_for_crash_recovery_process()?;
            } else {
                warp_logging::init(warp_logging::LogConfig { is_cli, log_destination })?;
            }
        } else {
            warp_logging::init(warp_logging::LogConfig { is_cli, log_destination })?;
        }
    }

    timer.mark_interval_end("LOG_FILE_SETUP_COMPLETE");

    // Adjust resource limits early, before doing other work, to ensure that
    // any children we spawn (like the terminal server) inherit our adjusted
    // rlimits.
    resource_limits::adjust_resource_limits();

    #[cfg(all(feature = "release_bundle", target_os = "linux"))]
    if let LaunchMode::App { .. } = launch_mode {
        match app_services::linux::pass_startup_args_to_existing_instance(
            launch_mode.args().as_ref(),
        ) {
            // If we were able to contact an existing application instance, quit -
            // we only want to run a single instance of Warp at a time.
            Ok(_) => std::process::exit(0),
            // If Warp isn't already running, we're good to go.
            Err(app_services::linux::StartupArgsForwardingError::NoExistingInstance) => {}
            // If we were unable to perform the forwarding for an unknown reason,
            // it's better to run a second instance than potentially end up in a
            // state where Warp refuses to run even a first instance.
            Err(err) => {
                let err = anyhow::Error::from(err).context("Failed to forward startup args");
                log::error!("{err:#}");
            }
        }
    }

    #[cfg(all(feature = "release_bundle", windows))]
    if let LaunchMode::App { .. } = launch_mode {
        match app_services::windows::pass_startup_args_to_existing_instance(
            launch_mode.args().as_ref(),
        ) {
            // If we were able to contact an existing application instance, quit -
            // we only want to run a single instance of Warp at a time.
            Ok(_) => std::process::exit(0),
            // If Warp isn't already running, we're good to go.
            Err(app_services::windows::StartupArgsForwardingError::NoExistingInstance) => {}
            // If we were unable to perform the forwarding for an unknown reason,
            // it's better to run a second instance than potentially end up in a
            // state where Warp refuses to run even a first instance.
            Err(err) => {
                let err = anyhow::Error::from(err).context("Failed to forward startup args");
                log::error!("{err:#}");
            }
        }
    }

    // Sets up a Job Object that we associate with the Warp process to handle
    // shared fate with its child processes. This should be called before we
    // start spawning any child processes.
    #[cfg(windows)]
    command::windows::init();

    // Configure rustls to use its default crypto provider.  This MUST be called
    // before making any network requests that use TLS, otherwise rustls will
    // panic.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("must be able to initialize crypto provider for TLS support");

    let private_preferences = settings::init_private_user_preferences();
    let (public_preferences, startup_toml_parse_error) = settings::init_public_user_preferences();

    // When the SettingsFile feature flag is enabled, public settings live in
    // the TOML-backed store. When disabled, they live in the platform-native
    // store (same backend as private). Use the correct one for pre-app reads.
    #[cfg_attr(not(any(enable_crash_recovery, target_os = "linux")), expect(unused))]
    let prefs_for_public_settings: &dyn warpui_extras::user_preferences::UserPreferences =
        if FeatureFlag::SettingsFile.is_enabled() {
            public_preferences.as_ref()
        } else {
            private_preferences.deref()
        };

    #[cfg(enable_crash_recovery)]
    let crash_recovery =
        crash_recovery::CrashRecovery::new(&launch_mode, prefs_for_public_settings);

    // Set up the pty spawner before doing any meaningful work. We want to
    // ensure that the process is in the cleanest possible state (minimal opened
    // files, modified signal handlers, etc.) to avoid unexpected effects on
    // spawned ptys.
    #[cfg(feature = "local_tty")]
    let pty_spawner =
        terminal::local_tty::spawner::PtySpawner::new().context("Failed to create pty spawner")?;

    let mut app_builder = if launch_mode.is_headless() {
        warpui::platform::AppBuilder::new_headless(
            app_callbacks(launch_mode.is_integration_test()),
            Box::new(ASSETS),
            launch_mode.take_test_driver(),
        )
    } else {
        warpui::platform::AppBuilder::new(
            app_callbacks(launch_mode.is_integration_test()),
            Box::new(ASSETS),
            launch_mode.take_test_driver(),
        )
    };

    #[cfg(target_os = "macos")]
    {
        use warpui::platform::mac::AppExt;

        let activate_on_launch = !launch_mode.is_integration_test()
            || std::env::var("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS").is_ok();
        app_builder.set_activate_on_launch(activate_on_launch);

        let dev_icon = ASSETS.get("bundled/png/local.png")?;
        app_builder.set_dev_icon(dev_icon);

        app_builder.set_menu_bar_builder(app_menus::menu_bar);
        app_builder.set_dock_menu_builder(|_| app_menus::dock_menu());
    }

    #[cfg(target_os = "linux")]
    {
        use crate::settings::ForceX11;
        use warpui::platform::linux::{self, AppBuilderExt};

        app_builder.set_window_class(ChannelState::app_id().to_string());

        let force_x11 = ForceX11::read_from_preferences(prefs_for_public_settings)
            .unwrap_or(ForceX11::default_value());
        // Force use of wayland if the user has passed the `WARP_ENABLE_WAYLAND` env var.
        let allow_wayland = linux::is_wayland_env_var_set() || !force_x11;
        app_builder.force_x11(!allow_wayland);
    }

    #[cfg(target_os = "windows")]
    {
        use warpui::platform::windows::AppBuilderExt;
        app_builder.set_app_user_model_id(ChannelState::app_id().to_string());

        // Only use DXC for DirectX shader compilation if we're not running in a Parallels VM
        // Parallels VMs can have issues with DXC shader compilation
        let is_parallels_vm = crate::util::vm_detection::is_running_in_windows_parallels_vm();
        if !is_parallels_vm {
            log::info!("Using DXC for DirectX shader compilation");
            use warpui::platform::windows::DXCPath;

            app_builder.use_dxc_for_directx_shader_compilation(DXCPath {
                dxc_path: "dxcompiler.dll".to_string(),
                dxil_path: "dxil.dll".to_string(),
            });
        } else {
            log::info!("Skipping DXC for DirectX shader compilation; running in a Parallels VM");
        }
    }

    // Override any bindings that have a `Custom` trigger to a `Keystroke`-based trigger. In theory,
    // this should be a noop on Mac (since the keystrokes registered via the  Mac menus first
    // intercept the binding), but just to be safe we only enable this in cases where we don't
    // include mac menus.
    #[cfg(not(target_os = "macos"))]
    app_builder.convert_custom_triggers_to_keystroke_triggers(
        crate::util::bindings::custom_tag_to_keystroke,
    );

    #[cfg(target_os = "macos")]
    app_builder.register_default_keystroke_triggers_for_custom_actions(
        crate::util::bindings::custom_tag_to_keystroke,
    );

    app_builder.run(move |ctx| {
        // Rotate the log files in the background.
        ctx.background_executor()
            .spawn(warp_logging::rotate_log_files())
            .detach();

        ctx.add_singleton_model(|ctx| {
            AppExecutionMode::new(
                launch_mode.execution_mode(),
                launch_mode.is_sandboxed(),
                ctx,
            )
        });
        // Add the terminal server singleton to the application.
        #[cfg(feature = "local_tty")]
        ctx.add_singleton_model(move |_ctx| pty_spawner);

        // Register user preferences.  This must be done before initializing
        // feature flags or experiments, both of which check user preferences for
        // overrides.
        ctx.add_singleton_model(move |_ctx| ::settings::PublicPreferences::new(public_preferences));
        ctx.add_singleton_model(move |_ctx| private_preferences);
        let startup_toml_parse_error = startup_toml_parse_error;

        // Tell the settings crate whether the TOML settings file is active.
        // This must happen after preferences are registered but before settings
        // are initialized, so the routing logic picks the correct backend.
        ::settings::set_settings_file_enabled(FeatureFlag::SettingsFile.is_enabled());

        #[cfg(enable_crash_recovery)]
        ctx.add_singleton_model(move |_ctx| crash_recovery);

        #[cfg(feature = "plugin_host")]
        ctx.add_singleton_model(move |ctx| {
            plugin::PluginHost::new(ctx).expect("Could not instantiate PluginHost")
        });
        let app_state = initialize_app(&launch_mode, timer, startup_toml_parse_error, ctx);

        if ImprovedPaletteSearch::improved_search_enabled(ctx) {
            FeatureFlag::UseTantivySearch.set_enabled(true);
        }

        launch(ctx, app_state, launch_mode);
    })
}

pub struct UpdateQuakeModeEventArg {
    active_window_id: Option<WindowId>,
}

fn initialize_app(
    launch_mode: &LaunchMode,
    mut timer: IntervalTimer,
    startup_toml_parse_error: Option<warpui_extras::user_preferences::Error>,
    ctx: &mut warpui::AppContext,
) -> Option<AppState> {
    let data_domain = ChannelState::data_domain();

    // Register an implementation of the secure storage service.
    cfg_if::cfg_if! {
        if #[cfg(feature = "integration_tests")] {
            warpui_extras::secure_storage::register_noop(&data_domain, ctx);
        } else if #[cfg(target_os = "linux")] {
            warpui_extras::secure_storage::register_with_fallback(&data_domain, warp_core::paths::state_dir(), ctx)
        } else if #[cfg(target_os = "windows")] {
            warpui_extras::secure_storage::register_with_dir(&data_domain, warp_core::paths::state_dir(), ctx)
        } else {
            warpui_extras::secure_storage::register(&data_domain, ctx);
        }
    }

    // One-time migration: give Preview its own config directory by
    // symlinking contents from the shared ~/.warp location. Must run
    // before ensure_warp_watch_roots_exist() creates the new directory.
    #[cfg(target_os = "macos")]
    preview_config_migration::migrate_preview_config_dir_if_needed();

    ensure_warp_watch_roots_exist();
    ctx.add_singleton_model(WarpManagedPathsWatcher::new);

    ctx.add_singleton_model(WarpConfig::new);
    ctx.add_singleton_model(|_ctx| SettingsManager::default());

    let user_defaults_on_startup = settings::init(startup_toml_parse_error, ctx);
    timer.mark_interval_end("READ_USER_DEFAULTS_AND_INITIALIZE_SETTINGS");

    if FeatureFlag::UIZoom.is_enabled() {
        ctx.set_zoom_factor(WindowSettings::as_ref(ctx).zoom_level.as_zoom_factor());
    }

    let local_identity = LocalActorIdentity::initialize(ctx);
    timer.mark_interval_end("LOCAL_IDENTITY_INITIALIZED");

    ctx.add_singleton_model(|_ctx| LocalActorIdentityProvider::new(local_identity));

    ctx.add_singleton_model(|_ctx| GPUState::new());

    PrivacySettings::register_singleton(ctx);

    // If any part of sqlite initialization fails, we just don't do session restoration (i.e.
    // feature degradation).
    let (sqlite_data, writer_handles) = persistence::initialize(ctx);
    timer.mark_interval_end("SQLITE_INITIALIZED");

    let persistence_writer = PersistenceWriter::new(writer_handles);

    let model_event_sender = persistence_writer.sender();

    let tips_handle = ctx.add_model(|_| user_defaults_on_startup.tips_data);
    let user_default_shell_unsupported_banner_model_handle =
        ctx.add_model(|_| user_defaults_on_startup.user_default_shell_unsupported_banner_state);
    let settings_file_error = user_defaults_on_startup.settings_file_error;
    ctx.add_singleton_model(move |_ctx| {
        GlobalResourceHandlesProvider::new(GlobalResourceHandles {
            model_event_sender,
            tips_completed: tips_handle,
            user_default_shell_unsupported_banner_model_handle,
            settings_file_error,
        })
    });

    let (
        cached_workspaces,
        current_workspace_uid,
        app_state,
        command_history,
        ai_queries,
        persisted_workspaces,
        workspace_language_servers,
        multi_agent_conversations,
        persisted_projects,
        persisted_project_rules,
        persisted_ignored_suggestions,
        persisted_mcp_server_installations,
        mcp_servers_to_restore,
    ) = sqlite_data
        .map(|sqlite_data| {
            (
                sqlite_data.workspaces,
                sqlite_data.current_workspace_uid,
                Some(sqlite_data.app_state),
                sqlite_data.command_history,
                sqlite_data.ai_queries,
                sqlite_data.codebase_indices,
                sqlite_data.workspace_language_servers,
                sqlite_data.multi_agent_conversations,
                sqlite_data.projects,
                sqlite_data.project_rules,
                sqlite_data.ignored_suggestions,
                sqlite_data.mcp_server_installations,
                sqlite_data.mcp_servers_to_restore,
            )
        })
        .unwrap_or_else(|| {
            (
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            )
        });

    let _ = (cached_workspaces, current_workspace_uid);
    ctx.add_singleton_model(UserWorkspaces::local_only);

    // Initialize ApiKeyManager after UserWorkspaces so it can subscribe to workspace/settings changes
    ctx.add_singleton_model(|ctx| {
        let mut manager = ::ai::api_keys::ApiKeyManager::new(ctx);
        manager.subscribe_to_settings_changes(ctx);
        manager
    });

    ctx.add_singleton_model(AntivirusInfo::new);

    timer.mark_interval_end("INIT_CRASH_REPORTING");

    ctx.set_fallback_font_source_provider(|url| ::asset_cache::url_source(url));

    ctx.set_default_binding_validator(is_binding_cross_platform);

    // Initialize timestamp for session id and last active event
    App::record_last_active_timestamp();

    ctx.add_singleton_model(|_| SettingsPaneManager::new());
    ctx.add_singleton_model(|_| ExecutionProfileEditorManager::default());
    #[cfg(target_os = "macos")]
    if !launch_mode.is_headless() {
        AppearanceManager::as_ref(ctx).set_app_icon(ctx);
    }

    #[cfg(feature = "local_tty")]
    terminal::available_shells::register(ctx);

    // Add truly global actions that don't depend on the existence of any view here
    ctx.add_global_action("app:toggle_user_ps1", move |_args: &(), ctx| {
        SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
            report_if_error!(session_settings.honor_ps1.toggle_and_save_value(ctx));
        });
    });
    ctx.add_global_action("app:toggle_copy_on_select", move |_args: &(), ctx| {
        SelectionSettings::handle(ctx).update(ctx, |selection_settings, ctx| {
            report_if_error!(selection_settings.copy_on_select.toggle_and_save_value(ctx));
        });
    });

    ctx.add_singleton_model(|_ctx| SyncedInputState::new());

    log::info!(
        "Starting warp with channel state {} and version {:?}",
        ChannelState::debug_str(),
        ChannelState::app_version()
    );

    // Teach our app that sometimes option means meta.
    ctx.set_event_munger(move |event, ctx| {
        let extra_meta_keys = *KeysSettings::as_ref(ctx).extra_meta_keys;
        apply_extra_meta_keys(event, extra_meta_keys);
        apply_scroll_multiplier(event, ctx);
    });

    ctx.set_a11y_verbosity(*AccessibilitySettings::as_ref(ctx).a11y_verbosity);

    #[cfg(enable_crash_recovery)]
    ctx.on_draw_frame_error(|ctx, window_id| {
        crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, _ctx| {
            crash_recovery.on_draw_frame_error(window_id);
        });
    });

    ctx.on_first_frame_drawn(move |ctx| {
        IntervalTimer::handle(ctx).update(ctx, |timer, _| {
            timer.mark_interval_end("FIRST_FRAME_DRAWN");
        });

        GPUState::handle(ctx).update(ctx, |gpu_state, ctx| {
            gpu_state.set_has_lower_power_gpu(warpui::rendering::is_low_power_gpu_available(), ctx);
        });

        for window_id in ctx.window_ids().collect_vec() {
            SettingsPaneManager::handle(ctx)
                .read(ctx, |model, _| model.settings_view(window_id))
                .update(ctx, |settings, ctx| {
                    settings.refresh_preferred_graphics_backend_dropdown(ctx);
                })
        }
    });

    #[cfg(enable_crash_recovery)]
    ctx.on_frame_drawn(|ctx, window_id| {
        crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, ctx| {
            crash_recovery.on_frame_drawn(window_id, ctx);
        });
    });
    {
        ctx.add_singleton_model(DirectoryWatcher::new);
        ctx.add_singleton_model(|_| DetectedRepositories::default());
        if let Some(home_dir) = dirs::home_dir() {
            ctx.add_singleton_model(|ctx| HomeDirectoryWatcher::new(home_dir, ctx));
        } else {
            log::info!("Home directory not found; skipping HomeDirectoryWatcher registration");
        }
    }

    #[cfg(feature = "local_fs")]
    {
        let imported_config_model = ctx.add_singleton_model(ImportedConfigModel::new);

        if ChannelState::channel() != warp_core::channel::Channel::Oss
            && FeatureFlag::SettingsImport.is_enabled()
            && ChannelState::channel() != warp_core::channel::Channel::Integration
        {
            imported_config_model.update(ctx, |model, ctx| {
                model.search_for_settings_to_import(ctx);
            });
        }

        ctx.add_singleton_model(RepoMetadataModel::new);
    }

    {
        use code_review::git_status_update::GitStatusUpdateModel;
        ctx.add_singleton_model(|_| GitStatusUpdateModel::new());
    }

    ctx.add_singleton_model(|ctx| {
        ProjectManagementModel::new(persisted_projects, persistence_writer.sender(), ctx)
    });

    ctx.add_singleton_model(move |_| History::new(command_history));

    ctx.add_singleton_model(CustomSecretRegexUpdater::new);

    timer.mark_interval_end("INITIALIZE_TELEMETRY_COLLECTION");

    // Register initial keybindings prior to creating menus
    ai::init(ctx);
    app_services::init(ctx);
    code::editor::find::view::init(ctx);
    workspace::init(ctx);
    pane_group::init(ctx);
    terminal::init(ctx);
    input::init(ctx);
    editor::init(ctx);
    menu::init(ctx);
    tips::tip_view::init(ctx);
    launch_configs::init(ctx);
    workflows::init(ctx);
    themes::theme_chooser::init(ctx);
    themes::theme_creator_modal::init(ctx);
    themes::theme_deletion_modal::init(ctx);
    root_view::init(ctx);
    voltron::init(ctx);
    crate::view_components::find::init(ctx);
    prompt::editor_modal::init(ctx);
    ai::blocklist::agent_view::editor::init(ctx);
    undo_close::init(ctx);
    tab_configs::new_worktree_modal::init(ctx);
    tab_configs::params_modal::init(ctx);
    ai::blocklist::init(ctx);
    ai::blocklist::block::status_bar::init(ctx);
    env_vars::env_var_collection_block::init(ctx);
    terminal::ssh::install_tmux::init(ctx);
    terminal::ssh::warpify::init(ctx);
    terminal::ssh::error::init(ctx);
    context_chips::display_menu::init(ctx);
    context_chips::node_version_popup::init(ctx);
    ai::agent::todos::popup::init(ctx);
    coding_entrypoints::project_buttons::init(ctx);
    if FeatureFlag::CodeReviewSaveChanges.is_enabled() {
        code_review::init(ctx);
    }

    let display_count = ctx.windows().display_count();
    ctx.add_singleton_model(|_| DisplayCount(display_count));

    ctx.add_singleton_model(|_| NetworkStatus::new());
    ctx.add_singleton_model(|_| SystemStats::new());
    ctx.add_singleton_model(|_| KeybindingChangedNotifier::new());
    ctx.add_singleton_model(|_| search::command_palette::SelectedItems::new());
    ctx.add_singleton_model(search::files::model::FileSearchModel::new);
    ctx.add_singleton_model(|_| VimRegisters::new());
    ctx.add_singleton_model(UndoCloseStack::new);
    ctx.add_singleton_model(|_| ToastStack);
    ctx.add_singleton_model(|_| GlobalCodeReviewModel);
    #[cfg(feature = "local_fs")]
    ctx.add_singleton_model(FileModel::new);
    ctx.add_singleton_model(GlobalBufferModel::new);
    #[cfg(windows)]
    ctx.add_singleton_model(util::traffic_lights::windows::RendererState::new);
    #[cfg(feature = "local_fs")]
    ctx.add_singleton_model(|_| LanguageServerShutdownManager::new());

    #[cfg(feature = "voice_input")]
    ctx.add_singleton_model(voice_input::VoiceInput::new);

    {
        let conversations = &multi_agent_conversations;
        ctx.add_singleton_model(move |_| BlocklistAIHistoryModel::new(ai_queries, conversations));
    }
    ctx.add_singleton_model(AIDocumentModel::new);
    ctx.add_singleton_model(move |_| RestoredAgentConversations::new(multi_agent_conversations));
    ctx.add_singleton_model(|_| CLIAgentSessionsModel::new());
    // ActiveAgentViewsModel is used to track active agent conversations and notify listeners when they change.
    ctx.add_singleton_model(|_| ActiveAgentViewsModel::new());
    ctx.add_singleton_model(BlocklistAIPermissions::new);
    ctx.add_singleton_model(ai::blocklist::orchestration_events::OrchestrationEventService::new);

    ctx.add_singleton_model(RepoOutlines::new);
    ctx.add_singleton_model(|ctx| {
        warp_core::sync_queue::SyncQueue::<SyncTask>::new_with_rate_limit(
            &ctx.background_executor(),
            Some(DEFAULT_SYNC_REQUESTS_PER_MIN),
        )
    });

    ctx.add_singleton_model(|_| AudibleBell::new());

    // LogManager must be registered before any subsystem (e.g. MCP, LSP) that creates file-based loggers.
    ctx.add_singleton_model(|_| simple_logger::manager::LogManager::new());

    let running_mcp_servers = app_state
        .as_ref()
        .map(|app_state| app_state.running_mcp_servers.as_slice())
        .unwrap_or(&[]);

    // FileMCPWatcher must be registered before FileBasedMCPManager, which subscribes to it.
    ctx.add_singleton_model(FileMCPWatcher::new);
    ctx.add_singleton_model(FileBasedMCPManager::new);

    // TemplatableMCPServerManager must be registered after FileBasedMCPManager so it can receive file-based server updates.
    ctx.add_singleton_model(|ctx| {
        TemplatableMCPServerManager::new(
            persisted_mcp_server_installations,
            mcp_servers_to_restore,
            running_mcp_servers,
            ctx,
        )
    });

    // SkillManager is used to cache SKILL.md files for all active terminal views and their working directories
    ctx.add_singleton_model(SkillManager::new);

    // ByoLlmAuthBannerSessionState tracks dismissal of the BYO LLM auth banner (e.g., AWS Bedrock login).
    ctx.add_singleton_model(ByoLlmAuthBannerSessionState::new);

    ctx.add_singleton_model(|_| CodeManager::default());
    ctx.add_singleton_model(|_| OpenedFilesModel::new());
    ctx.add_singleton_model(NotebookKeybindings::new);
    ctx.add_singleton_model(TerminalKeybindings::new);
    ctx.add_singleton_model(|_| ActiveSession::default());
    #[cfg(feature = "local_tty")]
    {
        ctx.add_singleton_model(LocalShellState::new);
        ctx.add_singleton_model(system::SystemInfo::new);
    }

    // Add a singleton model that holds the current prompt configuration.
    ctx.add_singleton_model(Prompt::new);

    // Add a singleton model for resizable modals whose size should be persisted through restarts.
    ctx.add_singleton_model(|_| ResizableData::default());

    ctx.add_singleton_model(LocalWorkflows::new);

    ctx.add_singleton_model(LLMPreferences::new);

    ctx.add_singleton_model(|ctx| {
        ai::agent_tips::AITipModel::<ai::AgentTip>::new_for_agent_tips(ctx)
    });

    timer.mark_interval_end("SINGLETON_MODELS_REGISTERED");

    ctx.add_singleton_model(move |_| timer);

    let is_ssh_tmux_wrapper_enabled = ctx
        .private_user_preferences()
        .read_value("SshTmuxWrapperOverride")
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok());

    if let Some(is_ssh_tmux_wrapper_enabled) = is_ssh_tmux_wrapper_enabled {
        FeatureFlag::SSHTmuxWrapper.set_user_preference(is_ssh_tmux_wrapper_enabled);
    }

    ctx.add_singleton_model(|ctx| AIExecutionProfilesModel::new(launch_mode, ctx));

    ctx.add_singleton_model(DefaultTerminal::new);

    ctx.add_singleton_model(|ctx| {
        let (indices_to_restore, max_indices_allowed, max_files_per_repo, embedding_batch_size) =
            (vec![], None, 0, 100);

        let store_client: Arc<
            dyn ::ai::index::full_source_code_embedding::store_client::StoreClient,
        > = Arc::new(MockStoreClient);

        CodebaseIndexManager::new(
            indices_to_restore,
            max_indices_allowed,
            max_files_per_repo,
            embedding_batch_size,
            store_client,
            ctx,
        )
    });

    ctx.add_singleton_model(|ctx| {
        ProjectContextModel::new_from_persisted(persisted_project_rules, ctx)
    });
    ctx.add_singleton_model(|ctx| {
        PersistedWorkspace::new(
            persisted_workspaces,
            workspace_language_servers,
            persistence_writer.sender(),
            ctx,
        )
    });
    ctx.add_singleton_model(move |_| persistence_writer);

    ctx.add_singleton_model(input_classifier::InputClassifierModel::new);

    ctx.add_singleton_model(move |_| IgnoredSuggestionsModel::new(persisted_ignored_suggestions));

    // When running natively, add the http server singleton to the application.
    ctx.add_singleton_model(move |ctx| {
        let routers = vec![
            app_installation_detection::make_router(),
            profiling::make_router(),
        ];
        http_server::HttpServer::new(routers, ctx)
    });

    app_state
}

fn app_callbacks(is_integration_test: bool) -> warpui::platform::AppCallbacks {
    warpui::platform::AppCallbacks {
        on_internet_reachability_changed: Some(Box::new(move |reachable, ctx| {
            NetworkStatus::handle(ctx)
                .update(ctx, move |me, ctx| me.reachability_changed(reachable, ctx));
        })),
        on_become_active: None,
        on_screen_changed: Some(Box::new(move |ctx| {
            ctx.dispatch_global_action(
                "root_view:move_quake_mode_window_from_screen_change",
                &KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .value()
                    .clone(),
            );

            let new_display_count = ctx.windows().display_count();
            DisplayCount::handle(ctx).update(ctx, |display_count, ctx| {
                display_count.0 = new_display_count;
                ctx.notify();
            });
        })),
        on_cpu_awakened: Some(Box::new(move |ctx| {
            SystemStats::handle(ctx).update(ctx, move |system, ctx| {
                log::info!("System has returned from sleep");
                system.dispatch_cpu_was_awakened(ctx);
            });
        })),
        on_cpu_will_sleep: Some(Box::new(move |ctx| {
            SystemStats::handle(ctx).update(ctx, move |system, ctx| {
                log::info!("System is going to sleep...");
                system.dispatch_cpu_will_sleep(ctx);
            });
        })),
        on_resigned_active: Some(Box::new(move |ctx| {
            let active_window_id = ctx.windows().active_window();
            let update_quake_mode_arg = UpdateQuakeModeEventArg { active_window_id };

            #[cfg(feature = "voice_input")]
            {
                if let voice_input::VoiceInputState::Listening { enabled_from, .. } =
                    voice_input::VoiceInput::as_ref(ctx).state()
                {
                    // Abort the voice input if it's toggled from a key press, as we cannot listen to key events
                    // if the user is focused on a different app - we could miss the release of the key.
                    if matches!(
                        *enabled_from,
                        voice_input::VoiceInputToggledFrom::Key { .. }
                    ) {
                        ctx.dispatch_global_action("root_view:abort_voice_input", &());
                    }
                }
            }
            ctx.dispatch_global_action("root_view:update_quake_mode_state", &update_quake_mode_arg);
        })),
        on_will_terminate: Some(Box::new(move |ctx| {
            PersistenceWriter::handle(ctx).update(ctx, |writer, _ctx| {
                writer.terminate();
            });

            // Shutdown all LSP servers gracefully before app termination
            lsp::LspManagerModel::handle(ctx).update(ctx, |manager, ctx| {
                manager.terminate(ctx);
            });

            // We want to tear down the terminal server after terminating the persistence writer,
            // so we don't keep track of the fact that the shell sessions terminated.
            #[cfg(feature = "local_tty")]
            terminal::local_tty::spawner::PtySpawner::handle(ctx).update(ctx, |pty_spawner, _| {
                pty_spawner.prepare_for_app_termination();
            });

            #[cfg(all(feature = "local_tty", windows))]
            terminal::local_tty::shutdown_all_pty_event_loops(ctx);

            // Tear down app services before spawning the new process, to
            // ensure that the new process doesn't find the old process while
            // attempting to enforce our single-instance policy on Linux.
            app_services::teardown(ctx);

            // Tear down any application profilers that are running, writing
            // results to disk.
            profiling::teardown();

            #[cfg(enable_crash_recovery)]
            crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, _ctx| {
                crash_recovery.teardown();
            });
        })),
        on_should_close_window: Some(Box::new(move |window_id, ctx| {
            let general_settings = GeneralSettings::as_ref(ctx);
            // On Linux or Windows, if we're about to close the final window, we should quit the app instead.
            // On Mac, we do this conditionally based on a user setting.
            let quit_on_last_window_closed = cfg!(any(target_os = "linux", windows))
                || *general_settings.quit_on_last_window_closed;
            if ctx.window_ids().count() == 1 && quit_on_last_window_closed {
                log::info!("No windows left, terminating app");
                ctx.terminate_app(TerminationMode::Cancellable, None);
                return ApproveTerminateResult::Cancel;
            }

            let summary = UnsavedStateSummary::for_window(window_id, ctx);

            // Don't show dialog on integration test. Machine can't press buttons.
            if !is_integration_test && summary.should_display_warning(ctx) {
                let shown = summary
                    .dialog()
                    .on_confirm(move |ctx| {
                        ctx.windows()
                            .close_window(window_id, TerminationMode::ForceTerminate);
                    })
                    .on_cancel(move |ctx| {
                        on_close_window_cancelled(window_id, false, ctx);
                    })
                    .on_show_processes(move |ctx| {
                        on_close_window_cancelled(window_id, true, ctx);
                    })
                    .show(ctx);
                if shown {
                    ApproveTerminateResult::Cancel
                } else {
                    ApproveTerminateResult::Terminate
                }
            } else {
                ApproveTerminateResult::Terminate
            }
        })),
        on_should_terminate_app: Some(Box::new(move |ctx| {
            let summary = UnsavedStateSummary::for_app(ctx);
            // Don't show dialog on integration test. Machine can't press buttons.
            if !is_integration_test && summary.should_display_warning(ctx) {
                let shown = summary
                    .dialog()
                    .on_confirm(|ctx| ctx.terminate_app(TerminationMode::ForceTerminate, None))
                    .on_show_processes(|ctx| on_close_app_cancelled(true, ctx))
                    .on_cancel(|ctx| on_close_app_cancelled(false, ctx))
                    .show(ctx);
                if shown {
                    return ApproveTerminateResult::Cancel;
                }
            }

            ApproveTerminateResult::Terminate
        })),
        on_disable_warning_modal: Some(Box::new(move |ctx| {
            GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
                report_if_error!(general_settings
                    .show_warning_before_quitting
                    .toggle_and_save_value(ctx));
            });
        })),
        on_notification_clicked: Some(Box::new(move |notification_response, ctx| {
            if let Some(notification_data) = notification_response.data() {
                let context: serde_json::Result<NotificationContext> =
                    serde_json::from_str(notification_data);
                if let Ok(NotificationContext::BlockOrigin {
                    window_id,
                    pane_group_id,
                    pane_id,
                }) = context
                {
                    // Ensure the window ID exists, if so dispatch an action to focus
                    // the correct pane.
                    if ctx.window_ids().contains(&window_id) {
                        if let Some(root_view_id) = ctx.root_view_id(window_id) {
                            ctx.dispatch_action(
                                window_id,
                                &[root_view_id],
                                "root_view:handle_notification_click",
                                &PaneViewLocator {
                                    pane_group_id,
                                    pane_id,
                                },
                                log::Level::Info,
                            );
                        }
                    }
                }
            }
        })),
        on_new_window_requested: Some(Box::new(move |ctx| {
            // This one is called when the app is requested to open a new window,
            // e.g. clicking on the Dock icon. It is NOT called from the New Window
            // menu item.
            App::record_last_active_timestamp();
            ctx.dispatch_global_action("root_view:open_new", &());
            ctx.dispatch_global_action("workspace:save_app", &());
        })),
        on_open_urls: Some(Box::new(move |urls, ctx| {
            for url in &urls {
                let parsed_url = Url::parse(url);
                match parsed_url {
                    Ok(url) => uri::handle_incoming_uri(&url, ctx),
                    Err(e) => log::warn!("Unable to parse received url: {e}"),
                }
            }
        })),
        on_os_appearance_changed: Some(Box::new(move |ctx| {
            AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
                appearance_manager.refresh_theme_state(ctx);
            });
        })),
        on_active_window_changed: Some(Box::new(move |ctx| {
            let windowing_model = ctx.windows();
            let active_window_id = windowing_model.active_window();
            let key_window_is_modal_panel = windowing_model.key_window_is_modal_panel();

            if !key_window_is_modal_panel {
                let update_quake_mode_arg = UpdateQuakeModeEventArg { active_window_id };
                ctx.dispatch_global_action(
                    "root_view:update_quake_mode_state",
                    &update_quake_mode_arg,
                );
            }

            ctx.dispatch_global_action("workspace:save_app", &());
        })),
        on_window_will_close: Some(Box::new(move |closed_window_data, ctx| {
            if ctx.windows().stage() == ApplicationStage::Terminating {
                return;
            }

            if let Some(window_data) = closed_window_data {
                UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
                    stack.handle_window_closed(window_data, ctx);
                });
            }
            ctx.dispatch_global_action("workspace:save_app", &());
        })),
        on_window_moved: Some(Box::new(move |ctx| {
            ctx.dispatch_global_action("workspace:save_app", &());
        })),
        on_window_resized: Some(Box::new(move |ctx| {
            ctx.dispatch_global_action("workspace:save_app", &());
        })),
        ..Default::default()
    }
}

fn on_close_app_cancelled(open_navigation_palette: bool, ctx: &mut AppContext) {
    let sessions = SessionNavigationData::all_sessions(ctx).collect_vec();
    let sessions_summary = RunningSessionSummary::new(&sessions);

    // If open_navigation_palette is false, return early. Otherwise, we honor the open_navigation_palette
    // param which is true if the user clicked the modal button for that. However, if the running
    // processes in this window have finished since the modal popped, there is nothing to do now and we
    // can return early
    if !open_navigation_palette || sessions_summary.long_running_cmds.is_empty() {
        return;
    }

    let windowing_model = ctx.windows();
    let active_window_id = windowing_model.active_window();
    // show the nav palette in the active window. if there is no active window,
    // arbitrarily pick one of the windows having a running process
    let window_id_to_focus = active_window_id.unwrap_or_else(|| {
        *sessions_summary
            .windows_running()
            .iter()
            .next()
            .expect("already checked len > 0")
    });

    windowing_model.show_window_and_focus_app(window_id_to_focus);

    // open the nav palette in the selected window
    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id_to_focus) {
        if let Some(handle) = workspaces.first() {
            ctx.dispatch_typed_action_for_view(
                window_id_to_focus,
                handle.id(),
                &WorkspaceAction::OpenPalette {
                    mode: PaletteMode::Navigation,
                    source: PaletteSource::QuitModal,
                    query: Some("running".to_owned()),
                },
            );
        }
    }
}

fn on_close_window_cancelled(
    window_id: WindowId,
    open_navigation_palette: bool,
    ctx: &mut AppContext,
) {
    let sessions = SessionNavigationData::all_sessions(ctx).collect_vec();
    let sessions_summary = RunningSessionSummary::new(&sessions);
    let num_processes_in_window = sessions_summary.processes_in_window(&window_id).len();

    // If open_navigation_palette is false, return early. Otherwise, we honor the
    // open_navigation_palette param which is true if the user clicked the modal
    // button for that. However, if the running processes in this window have finished
    // since the modal popped, there is nothing to do now and we can return early
    if !open_navigation_palette || num_processes_in_window == 0 {
        return;
    }

    ctx.windows().show_window_and_focus_app(window_id);

    // if we haven't returned early, it means open_navigation_palette is true as the
    // user pressed the modal button for opening the navigation palette to show their
    // running processes
    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) {
        if let Some(handle) = workspaces.first() {
            ctx.dispatch_typed_action_for_view(
                window_id,
                handle.id(),
                &WorkspaceAction::OpenPalette {
                    mode: PaletteMode::Navigation,
                    source: PaletteSource::QuitModal,
                    query: Some("running".to_owned()),
                },
            );
        }
    }
}

fn launch(ctx: &mut warpui::AppContext, app_state: Option<AppState>, launch_mode: LaunchMode) {
    IntervalTimer::handle(ctx).update(ctx, |timer, _ctx| {
        timer.mark_interval_end("APP_LAUNCHED");
    });

    keyboard::load_custom_keybindings(ctx);

    IntervalTimer::handle(ctx).update(ctx, |timer, _ctx| {
        timer.mark_interval_end("KEYBINDINGS_LOADED");
    });

    match &launch_mode {
        LaunchMode::App { .. } | LaunchMode::Test { .. } => {
            // Attempt to restore windows from the persisted application state.
            let arg = OpenFromRestoredArg { app_state };
            ctx.dispatch_global_action("root_view:open_from_restored", &arg);

            // Process any URLs that were provided on the command line (which may be
            // file:// URLs or ones using our custom URL scheme).
            for url in launch_mode.args().urls.iter() {
                uri::handle_incoming_uri(url, ctx);
            }

            // If, after session restoration and command-line argument handling, we
            // haven't opened any windows, open a new window.
            if ctx.window_ids().count() == 0 {
                ctx.dispatch_global_action("root_view:open_new", &());
            }

            IntervalTimer::handle(ctx).update(ctx, |timer, _| {
                timer.mark_interval_end("WINDOWS_CREATED");
            });

            // TODO(ben): We should skip this for LaunchMode::Test.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                use crate::login_item::maybe_register_app_as_login_item;
                use crate::terminal::general_settings::GeneralSettingsChangedEvent;
                // Note that we put this here because it depends on settings already having been initialized.
                ctx.subscribe_to_model(&GeneralSettings::handle(ctx), |_, event, ctx| {
                    if matches!(event, GeneralSettingsChangedEvent::LoginItem { .. }) {
                        maybe_register_app_as_login_item(ctx);
                    }
                });
                maybe_register_app_as_login_item(ctx);
            }
        }
        LaunchMode::CommandLine {
            command,
            global_options,
            ..
        } => {
            if let Err(err) =
                crate::ai::agent_sdk::run(ctx, command.clone(), global_options.clone())
            {
                eprintln!("{err:#}");
                report_error!(err);
                std::process::exit(1);
            }
        }
    }
}

/// Initializes the logger before running tests.
///
/// The `ctor` attribute here means that this runs BEFORE main(), whenever the
/// binary is executed. For this reason, we need to ensure that this function
/// only exists within unit test code. Production bundles and integration tests
/// also initialize the logging system, and initializing it twice causes a panic.
///
/// Additionally, we must not write anything to stdout in this function, as it
/// can interfere with test harnesses collecting the set of tests to run. (This
/// is why we're not simply calling the init() function above.)
#[ctor::ctor]
#[cfg(test)]
fn init_logging_for_unit_tests_glue() {
    // Initialize terminal-friendly logging for tests from the shared logger crate.
    warp_logging::init_logging_for_unit_tests();
}

/// Mark all features which should be enabled on the current channel as enabled.
/// This sets global feature flag state and should never be called in a unit test.
pub fn init_feature_flags() {
    for flag in enabled_features() {
        flag.set_enabled(true);
    }
    features::mark_initialized();
}

/// Returns all feature flags which should be enabled in the current channel.
pub fn enabled_features() -> HashSet<FeatureFlag> {
    // Enable features overridden for the given channel.
    let mut flags = ChannelState::additional_features();

    // Enable flags for release builds, if appropriate.
    if ChannelState::is_release_bundle() {
        flags.extend(features::RELEASE_FLAGS);
    }

    flags.extend([
        #[cfg(feature = "changelog")]
        FeatureFlag::Changelog,
        #[cfg(feature = "runtime_feature_flags")]
        FeatureFlag::RuntimeFeatureFlags,
        #[cfg(feature = "sequential_storage")]
        FeatureFlag::SequentialStorage,
        #[cfg(feature = "in_band_generators_ssh")]
        FeatureFlag::InBandGeneratorsForSSH,
        #[cfg(feature = "run_generators_with_cmd_exe")]
        FeatureFlag::RunGeneratorsWithCmdExe,
        #[cfg(feature = "ligatures")]
        FeatureFlag::Ligatures,
        #[cfg(feature = "selectable_prompt")]
        FeatureFlag::SelectablePrompt,
        #[cfg(feature = "agent_mode")]
        FeatureFlag::AgentMode,
        #[cfg(feature = "resize_fix")]
        FeatureFlag::ResizeFix,
        #[cfg(feature = "richtext_multiselect")]
        FeatureFlag::RichTextMultiselect,
        #[cfg(feature = "default_waterfall_mode")]
        FeatureFlag::DefaultWaterfallMode,
        #[cfg(feature = "settings_file")]
        FeatureFlag::SettingsFile,
        #[cfg(feature = "settings_import")]
        FeatureFlag::SettingsImport,
        #[cfg(feature = "rect_selection")]
        FeatureFlag::RectSelection,
        #[cfg(feature = "alacritty_settings_import")]
        FeatureFlag::AlacrittySettingsImport,
        #[cfg(feature = "dynamic_workflow_enums")]
        FeatureFlag::DynamicWorkflowEnums,
        #[cfg(feature = "am_workflows")]
        FeatureFlag::AgentModeWorkflows,
        #[cfg(feature = "ai_rules")]
        FeatureFlag::AIRules,
        #[cfg(feature = "ssh_tmux_wrapper")]
        FeatureFlag::SSHTmuxWrapper,
        #[cfg(feature = "less_horizontal_terminal_padding")]
        FeatureFlag::LessHorizontalTerminalPadding,
        #[cfg(feature = "shell_selector")]
        FeatureFlag::ShellSelector,
        #[cfg(feature = "block_toolbelt_save_as_workflow")]
        FeatureFlag::BlockToolbeltSaveAsWorkflow,
        #[cfg(all(feature = "simulate_github_unauthed", debug_assertions))]
        FeatureFlag::SimulateGithubUnauthed,
        #[cfg(feature = "full_screen_zen_mode")]
        FeatureFlag::FullScreenZenMode,
        #[cfg(feature = "minimalist_ui")]
        FeatureFlag::MinimalistUI,
        #[cfg(feature = "remove_alt_screen_padding")]
        FeatureFlag::RemoveAltScreenPadding,
        #[cfg(feature = "avatar_in_tab_bar")]
        FeatureFlag::AvatarInTabBar,
        #[cfg(feature = "workflow_aliases")]
        FeatureFlag::WorkflowAliases,
        #[cfg(feature = "ssh_drag_and_drop")]
        FeatureFlag::SshDragAndDrop,
        #[cfg(feature = "drag_tabs_to_windows")]
        FeatureFlag::DragTabsToWindows,
        #[cfg(feature = "multi_workspace")]
        FeatureFlag::MultiWorkspace,
        #[cfg(feature = "ime_marked_text")]
        FeatureFlag::ImeMarkedText,
        #[cfg(feature = "iterm_images")]
        FeatureFlag::ITermImages,
        #[cfg(feature = "validate_autosuggestions")]
        FeatureFlag::ValidateAutosuggestions,
        #[cfg(feature = "prompt_suggestions_via_maa")]
        FeatureFlag::PromptSuggestionsViaMAA,
        #[cfg(feature = "clear_autosuggestion_on_escape")]
        FeatureFlag::ClearAutosuggestionOnEscape,
        #[cfg(all(not(windows), feature = "kitty_images"))]
        FeatureFlag::KittyImages,
        #[cfg(feature = "warp_packs")]
        FeatureFlag::WarpPacks,
        #[cfg(feature = "default_adeberry_theme")]
        FeatureFlag::DefaultAdeberryTheme,
        #[cfg(feature = "agent_mode_primary_xml")]
        FeatureFlag::AgentModePrimaryXML,
        #[cfg(feature = "agent_mode_pre_plan_xml")]
        FeatureFlag::AgentModePrePlanXML,
        #[cfg(feature = "suggested_rules")]
        FeatureFlag::SuggestedRules,
        #[cfg(feature = "suggested_agent_mode_workflows")]
        FeatureFlag::SuggestedAgentModeWorkflows,
        #[cfg(feature = "command_correction_key")]
        FeatureFlag::CommandCorrectionKey,
        #[cfg(feature = "full_source_code_embedding")]
        FeatureFlag::FullSourceCodeEmbedding,
        #[cfg(feature = "use_tantivy_search")]
        FeatureFlag::UseTantivySearch,
        #[cfg(feature = "grep_tool")]
        FeatureFlag::GrepTool,
        #[cfg(feature = "mcp_server")]
        FeatureFlag::McpServer,
        #[cfg(feature = "mcp_debugging_ids")]
        FeatureFlag::McpDebuggingIds,
        #[cfg(feature = "markdown_tables")]
        FeatureFlag::MarkdownTables,
        #[cfg(feature = "blocklist_markdown_table_rendering")]
        FeatureFlag::BlocklistMarkdownTableRendering,
        #[cfg(feature = "blocklist_markdown_images")]
        FeatureFlag::BlocklistMarkdownImages,
        #[cfg(feature = "markdown_mermaid")]
        FeatureFlag::MarkdownMermaid,
        #[cfg(feature = "editable_markdown_mermaid")]
        FeatureFlag::EditableMarkdownMermaid,
        #[cfg(feature = "image_as_context")]
        FeatureFlag::ImageAsContext,
        #[cfg(feature = "msys2_shells")]
        FeatureFlag::MSYS2Shells,
        #[cfg(feature = "file_retrieval_tools")]
        FeatureFlag::FileRetrievalTools,
        #[cfg(feature = "reload_stale_conversation_files")]
        FeatureFlag::ReloadStaleConversationFiles,
        #[cfg(feature = "shared_block_title_generation")]
        FeatureFlag::SharedBlockTitleGeneration,
        #[cfg(feature = "retry_truncated_code_responses")]
        FeatureFlag::RetryTruncatedCodeResponses,
        #[cfg(feature = "read_image_files")]
        FeatureFlag::ReadImageFiles,
        #[cfg(feature = "cross_repo_context")]
        FeatureFlag::CrossRepoContext,
        #[cfg(feature = "codebase_index_persistence")]
        FeatureFlag::CodebaseIndexPersistence,
        #[cfg(feature = "ai_context_menu")]
        FeatureFlag::AIContextMenuEnabled,
        #[cfg(feature = "at_menu_outside_of_ai_mode")]
        FeatureFlag::AtMenuOutsideOfAIMode,
        #[cfg(feature = "ai_resume_button")]
        FeatureFlag::AIResumeButton,
        #[cfg(feature = "figma_detection")]
        FeatureFlag::FigmaDetection,
        #[cfg(feature = "agent_decides_command_execution")]
        FeatureFlag::AgentDecidesCommandExecution,
        #[cfg(feature = "codebase_index_speedbump")]
        FeatureFlag::CodebaseIndexSpeedbump,
        #[cfg(feature = "context_line_review_comments")]
        FeatureFlag::ContextLineReviewComments,
        #[cfg(feature = "nld_fasttext_model")]
        FeatureFlag::NLDClassifierModelEnabled,
        #[cfg(feature = "fast_forward_autoexecute_button")]
        FeatureFlag::FastForwardAutoexecuteButton,
        #[cfg(feature = "code_find_replace")]
        FeatureFlag::CodeFindReplace,
        #[cfg(feature = "command_palette_file_search")]
        FeatureFlag::CommandPaletteFileSearch,
        #[cfg(feature = "ai_context_menu_commands")]
        FeatureFlag::AIContextMenuCommands,
        #[cfg(feature = "ai_context_menu_code")]
        FeatureFlag::AIContextMenuCode,
        #[cfg(feature = "expand_edit_to_pane")]
        FeatureFlag::ExpandEditToPane,
        #[cfg(feature = "fallback_model_load_output_messaging")]
        FeatureFlag::FallbackModelLoadOutputMessaging,
        #[cfg(feature = "tab_close_button_on_left")]
        FeatureFlag::TabCloseButtonOnLeft,
        #[cfg(feature = "profiles_design_revamp")]
        FeatureFlag::ProfilesDesignRevamp,
        #[cfg(feature = "search_codebase_ui")]
        FeatureFlag::SearchCodebaseUI,
        #[cfg(feature = "changed_lines_only_apply_diff_result")]
        FeatureFlag::ChangedLinesOnlyApplyDiffResult,
        #[cfg(feature = "linked_code_blocks")]
        FeatureFlag::LinkedCodeBlocks,
        #[cfg(feature = "tabbed_editor_view")]
        FeatureFlag::TabbedEditorView,
        #[cfg(feature = "undo_closed_panes")]
        FeatureFlag::UndoClosedPanes,
        #[cfg(feature = "multi_profile")]
        FeatureFlag::MultiProfile,
        #[cfg(feature = "conversation_artifacts")]
        FeatureFlag::ConversationArtifacts,
        #[cfg(feature = "get_started_tab")]
        FeatureFlag::GetStartedTab,
        #[cfg(feature = "welcome_tab")]
        FeatureFlag::WelcomeTab,
        #[cfg(feature = "projects")]
        FeatureFlag::Projects,
        #[cfg(feature = "pr_comments_slash_command")]
        FeatureFlag::PRCommentsSlashCommand,
        #[cfg(feature = "pr_comments_v2")]
        FeatureFlag::PRCommentsV2,
        #[cfg(feature = "pr_comments_skill")]
        FeatureFlag::PRCommentsSkill,
        #[cfg(feature = "selection_as_context")]
        FeatureFlag::SelectionAsContext,
        #[cfg(feature = "code_mode_chip")]
        FeatureFlag::CodeModeChip,
        #[cfg(feature = "github_pr_prompt_chip")]
        FeatureFlag::GithubPrPromptChip,
        #[cfg(feature = "create_project_flow")]
        FeatureFlag::CreateProjectFlow,
        #[cfg(feature = "vim_code_editor")]
        FeatureFlag::VimCodeEditor,
        #[cfg(feature = "allow_opening_file_links_using_editor_env")]
        FeatureFlag::AllowOpeningFileLinksUsingEditorEnv,
        #[cfg(feature = "nld_improvements")]
        FeatureFlag::NldImprovements,
        #[cfg(feature = "revert_diff_hunk")]
        FeatureFlag::RevertDiffHunk,
        #[cfg(feature = "code_review_save_changes")]
        FeatureFlag::CodeReviewSaveChanges,
        #[cfg(feature = "file_tree")]
        FeatureFlag::FileTree,
        #[cfg(feature = "allow_ignoring_input_suggestions")]
        FeatureFlag::AllowIgnoringInputSuggestions,
        #[cfg(feature = "code_launch_modal")]
        FeatureFlag::CodeLaunchModal,
        #[cfg(feature = "mcp_oauth")]
        FeatureFlag::McpOauth,
        #[cfg(feature = "file_based_mcp")]
        FeatureFlag::FileBasedMcp,
        #[cfg(feature = "diff_set_as_context")]
        FeatureFlag::DiffSetAsContext,
        #[cfg(feature = "discard_per_file_and_all_changes")]
        FeatureFlag::DiscardPerFileAndAllChanges,
        #[cfg(feature = "summarization_cancellation_confirmation")]
        FeatureFlag::SummarizationCancellationConfirmation,
        #[cfg(feature = "code_review_find")]
        FeatureFlag::CodeReviewFind,
        #[cfg(feature = "ui_zoom")]
        FeatureFlag::UIZoom,
        #[cfg(feature = "auto_open_code_review_pane")]
        FeatureFlag::AutoOpenCodeReviewPane,
        #[cfg(feature = "inline_code_review")]
        FeatureFlag::InlineCodeReview,
        #[cfg(feature = "summarize_conversation_command")]
        FeatureFlag::SummarizationConversationCommand,
        #[cfg(feature = "mcp_grouped_server_context")]
        FeatureFlag::MCPGroupedServerContext,
        #[cfg(feature = "web_search_ui")]
        FeatureFlag::WebSearchUI,
        #[cfg(feature = "web_fetch_ui")]
        FeatureFlag::WebFetchUI,
        #[cfg(feature = "fork_from_command")]
        FeatureFlag::ForkFromCommand,
        #[cfg(feature = "context_window_usage_v2")]
        FeatureFlag::ContextWindowUsageV2,
        #[cfg(feature = "global_search")]
        FeatureFlag::GlobalSearch,
        #[cfg(feature = "embedded_code_review_comments")]
        FeatureFlag::EmbeddedCodeReviewComments,
        #[cfg(feature = "file_and_diff_set_comments")]
        FeatureFlag::FileAndDiffSetComments,
        #[cfg(feature = "revert_to_checkpoints")]
        FeatureFlag::RevertToCheckpoints,
        #[cfg(feature = "rewind_slash_command")]
        FeatureFlag::RewindSlashCommand,
        #[cfg(feature = "agent_view")]
        FeatureFlag::AgentView,
        #[cfg(feature = "agent_view_block_context")]
        FeatureFlag::AgentViewBlockContext,
        #[cfg(feature = "v4a_file_diffs")]
        FeatureFlag::V4AFileDiffs,
        #[cfg(feature = "interactive_conversation_management_view")]
        FeatureFlag::InteractiveConversationManagementView,
        #[cfg(feature = "agent_tips")]
        FeatureFlag::AgentTips,
        #[cfg(feature = "agent_mode_computer_use")]
        FeatureFlag::AgentModeComputerUse,
        #[cfg(feature = "local_computer_use")]
        FeatureFlag::LocalComputerUse,
        #[cfg(feature = "agent_toolbar_editor")]
        FeatureFlag::AgentToolbarEditor,
        #[cfg(feature = "configurable_toolbar")]
        FeatureFlag::ConfigurableToolbar,
        #[cfg(feature = "agent_view_prompt_chip")]
        FeatureFlag::AgentViewPromptChip,
        #[cfg(feature = "classic_completions")]
        FeatureFlag::ClassicCompletions,
        #[cfg(feature = "force_classic_completions")]
        FeatureFlag::ForceClassicCompletions,
        #[cfg(feature = "agent_view_conversation_list_view")]
        FeatureFlag::AgentViewConversationListView,
        #[cfg(feature = "inline_history_menu")]
        FeatureFlag::InlineHistoryMenu,
        #[cfg(feature = "inline_repo_menu")]
        FeatureFlag::InlineRepoMenu,
        #[cfg(feature = "summarization_via_message_replacement")]
        FeatureFlag::SummarizationViaMessageReplacement,
        #[cfg(feature = "pluggable_notifications")]
        FeatureFlag::PluggableNotifications,
        #[cfg(feature = "list_skills")]
        FeatureFlag::ListSkills,
        #[cfg(feature = "ask_user_question")]
        FeatureFlag::AskUserQuestion,
        #[cfg(feature = "lsp_as_a_tool")]
        FeatureFlag::LSPAsATool,
        #[cfg(feature = "inline_profile_selector")]
        FeatureFlag::InlineProfileSelector,
        #[cfg(feature = "bundled_skills")]
        FeatureFlag::BundledSkills,
        #[cfg(feature = "new_tab_styling")]
        FeatureFlag::NewTabStyling,
        #[cfg(feature = "skill_arguments")]
        FeatureFlag::SkillArguments,
        #[cfg(feature = "active_conversation_requires_interaction")]
        FeatureFlag::ActiveConversationRequiresInteraction,
        #[cfg(feature = "conversations_as_context")]
        FeatureFlag::ConversationsAsContext,
        #[cfg(feature = "incremental_auto_reload")]
        FeatureFlag::IncrementalAutoReload,
        #[cfg(feature = "orchestration")]
        FeatureFlag::Orchestration,
        #[cfg(feature = "orchestration_v2")]
        FeatureFlag::OrchestrationV2,
        #[cfg(feature = "orchestration_event_push")]
        FeatureFlag::OrchestrationEventPush,
        #[cfg(feature = "pending_user_query_indicator")]
        FeatureFlag::PendingUserQueryIndicator,
        #[cfg(feature = "queue_slash_command")]
        FeatureFlag::QueueSlashCommand,
        #[cfg(feature = "kitty_keyboard_protocol")]
        FeatureFlag::KittyKeyboardProtocol,
        #[cfg(feature = "inline_menu_headers")]
        FeatureFlag::InlineMenuHeaders,
        #[cfg(feature = "directory_tab_colors")]
        FeatureFlag::DirectoryTabColors,
        #[cfg(feature = "open_warp_new_settings_modes")]
        FeatureFlag::OpenWarpNewSettingsModes,
        #[cfg(feature = "hoa_code_review")]
        FeatureFlag::HoaCodeReview,
        #[cfg(feature = "vertical_tabs")]
        FeatureFlag::VerticalTabs,
        #[cfg(feature = "vertical_tabs_summary_mode")]
        FeatureFlag::VerticalTabsSummaryMode,
        #[cfg(feature = "tab_configs")]
        FeatureFlag::TabConfigs,
        #[cfg(feature = "agent_harness")]
        FeatureFlag::AgentHarness,
        #[cfg(feature = "hoa_notifications")]
        FeatureFlag::HOANotifications,
        #[cfg(feature = "open_code_notifications")]
        FeatureFlag::OpenCodeNotifications,
        #[cfg(feature = "cli_agent_rich_input")]
        FeatureFlag::CLIAgentRichInput,
        #[cfg(feature = "transfer_control_tool")]
        FeatureFlag::TransferControlTool,
        #[cfg(feature = "warpify_footer")]
        FeatureFlag::WarpifyFooter,
        #[cfg(feature = "solo_user_byok")]
        FeatureFlag::SoloUserByok,
        #[cfg(feature = "git_operations_in_code_review")]
        FeatureFlag::GitOperationsInCodeReview,
        #[cfg(feature = "hoa_remote_control")]
        FeatureFlag::HOARemoteControl,
        #[cfg(feature = "codex_notifications")]
        FeatureFlag::CodexNotifications,
        #[cfg(feature = "trim_trailing_blank_lines")]
        FeatureFlag::TrimTrailingBlankLines,
    ]);

    flags
}
