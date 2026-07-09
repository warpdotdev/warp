#![allow(clippy::doc_lazy_continuation)]

mod ai;
mod alloc;
mod antivirus;
#[cfg(target_os = "macos")]
mod app_menus;
mod app_services;
mod app_state;
mod auth;
mod autoupdate;
mod banner;
mod billing;
mod changelog_model;
mod chip_configurator;
mod cloud_object;
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
#[cfg(feature = "crash_reporting")]
mod crash_reporting;
mod debug_dump;
mod default_terminal;
mod download_method;
mod drive;
#[cfg(windows)]
mod dynamic_libraries;
mod env_vars;
mod experiments;
mod external_secrets;
#[cfg(target_family = "wasm")]
mod font_fallback;
mod global_resource_handles;
mod gpu_state;
mod input_classifier;
mod interval_timer;
mod linear;
#[cfg(feature = "local_fs")]
mod local_control;
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
mod pricing;
mod profiling;
mod projects;
mod prompt;
mod quit_warning;
mod referral_theme_status;
#[allow(dead_code)]
mod remote_server;
mod resource_limits;
mod reward_view;
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
#[cfg(feature = "tui")]
mod tui;
#[cfg(feature = "tui")]
pub mod tui_export;
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
#[cfg(target_family = "wasm")]
mod wasm_nux_dialog;
mod window_settings;
mod word_block_editor;
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
pub mod ai_assistant;
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
use ::ai::index::full_source_code_embedding::manager::{
    CodebaseIndexManager, CodebaseIndexManagerConfig,
};
#[cfg(feature = "local_fs")]
use ::ai::index::full_source_code_embedding::SnapshotStorage;
use ::ai::index::full_source_code_embedding::SyncTask;
use ::ai::index::DEFAULT_SYNC_REQUESTS_PER_MIN;
use ::ai::project_context::model::ProjectContextModel;
pub use ai::agent::todos::AIAgentTodoList;
pub use ai::agent::{AIAgentActionResultType, FileEdit, TodoOperation};
use ai::agent_conversations_model::AgentConversationsModel;
use ai::agent_management::AgentNotificationsModel;
use ai::ambient_agents::scheduled::ScheduledAgentManager;
use ai::blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions};
use ai::execution_profiles::editor::ExecutionProfileEditorManager;
use ai::execution_profiles::profiles::AIExecutionProfilesModel;
use ai::metadata_project_rules::read_project_rule_contents;
use ai::persisted_workspace::PersistedWorkspace;
use auth::auth_manager::AuthManager;
use auth::auth_state::{AuthState, AuthStateProvider};
use code::editor_management::CodeManager;
use code::opened_files::OpenedFilesModel;
use code_review::git_repo_model::GitRepoModels;
use code_review::GlobalCodeReviewModel;
use quit_warning::UnsavedStateSummary;
#[cfg(feature = "local_fs")]
use repo_metadata::{
    repositories::DetectedRepositories, watcher::DirectoryWatcher, RepoMetadataModel,
};
use server::network_log_pane_manager::NetworkLogPaneManager;
use server::telemetry::context_provider::AppTelemetryContextProvider;
use server::voice_transcriber::ServerVoiceTranscriber;
#[cfg(feature = "local_fs")]
use settings::import::model::ImportedConfigModel;
use settings_view::pane_manager::SettingsPaneManager;
use terminal::general_settings::GeneralSettings;
use terminal::keys_settings::KeysSettings;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use terminal::local_shell::LocalShellState;
pub use util::bindings::cmd_or_ctrl_shift;
use voice::transcriber::VoiceTranscriber;
use warp_cli::agent::AgentCommand;
use warp_cli::{CliCommand, GlobalOptions};
#[cfg(feature = "local_fs")]
use watcher::HomeDirectoryWatcher;

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::aws_credentials::AwsCredentialRefresher as _;
#[cfg(not(target_family = "wasm"))]
use crate::ai::geap_credentials::GeapCredentialRefresher as _;
use crate::ai::mcp::{FileBasedMCPManager, FileMCPWatcher};
use crate::uri::web_intent_parser::maybe_rewrite_web_url_to_intent;
use crate::view_components::DismissibleToast;
pub mod workflows;
pub mod workspace;

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Deref;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::sync::Arc;

use ::settings::{Setting, ToggleableSetting};
#[cfg(feature = "local_tty")]
use anyhow::Context;
use anyhow::{anyhow, Result};
use appearance::{Appearance, AppearanceManager};
use channel::ChannelState;
use interval_timer::IntervalTimer;
use itertools::Itertools;
#[cfg(feature = "integration_tests")]
pub use persistence::testing as sqlite_testing;
#[cfg(feature = "plugin_host")]
pub use plugin::{run_plugin_host, PLUGIN_HOST_FLAG};
use referral_theme_status::ReferralThemeStatus;
use server::server_api::ServerApiProvider;
use settings::{ExtraMetaKeys, PrivacySettings};
#[cfg(feature = "local_fs")]
use shellexpand::tilde;
use terminal::input;
use terminal::session_settings::SessionSettings;
use url::Url;
use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
// Re-export the debounce function to simplify imports.
pub use warp_core::r#async::debounce;
// Re-export the send_telemetry_from_ctx macro at the crate root level
pub use warp_core::send_telemetry_from_app_ctx;
pub use warp_core::send_telemetry_from_ctx;
// Re-export the safe logging macros at the crate root level for backwards compatibility
pub use warp_core::{safe_debug, safe_error, safe_info, safe_warn};
use warp_errors::{report_error, report_if_error};
#[cfg(feature = "local_fs")]
use warp_files::FileModel;
use warp_logging::LogDestination;
use warp_managed_secrets::ManagedSecretManager;
use warp_server_client::iap::{IapManager, IapManagerEvent, IapState};
use warp_server_client::network_logging::NetworkLogModel;
use warpui::integration::TestDriver;
use warpui::modals::{AlertDialogWithCallbacks, AppModalCallback};
use warpui::platform::app::{ApproveTerminateResult, TerminationRequestSource};
use warpui::platform::TerminationMode;
use warpui::windowing::state::ApplicationStage;
use warpui::{App, AppContext, Event, SingletonEntity, WindowId};
use window_settings::WindowSettings;
use workflows::manager::WorkflowManager;
use workspace::sync_inputs::SyncedInputState;

use self::features::FeatureFlag;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::blocklist::RecordingController;
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::facts::manager::AIFactManager;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::{MCPGalleryManager, TemplatableMCPServerManager};
use crate::ai::outline::RepoOutlines;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::ai::AIRequestUsageModel;
use crate::antivirus::AntivirusInfo;
use crate::app_state::AppState;
use crate::autoupdate::{AutoupdateState, RelaunchModel};
use crate::changelog_model::ChangelogModel;
use crate::cloud_object::model::actions::{ObjectAction, ObjectActions};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::code::global_buffer_model::GlobalBufferModel;
#[cfg(feature = "local_fs")]
use crate::code::language_server_shutdown_manager::LanguageServerShutdownManager;
use crate::context_chips::prompt::Prompt;
use crate::default_terminal::DefaultTerminal;
use crate::drive::export::ExportManager;
use crate::drive::CloudObjectTypeAndId;
use crate::env_vars::manager::EnvVarCollectionManager;
use crate::experiments::ImprovedPaletteSearch;
pub use crate::global_resource_handles::{GlobalResourceHandles, GlobalResourceHandlesProvider};
use crate::gpu_state::GPUState;
use crate::network::NetworkStatus;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::manager::NotebookManager;
use crate::notebooks::CloudNotebook;
use crate::notification::NotificationContext;
use crate::palette::PaletteMode;
use crate::persistence::model::AgentConversationData;
use crate::persistence::PersistenceWriter;
use crate::projects::ProjectManagementModel;
use crate::root_view::{
    quake_mode_window_id, quake_mode_window_is_open, OpenFromRestoredArg, OpenPath,
};
use crate::server::cloud_objects::listener::Listener;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::experiments::ServerExperiments;
use crate::server::sync_queue::{QueueItem, SyncQueue};
pub use crate::server::telemetry::{
    AgentModeEntrypoint, AgentModeEntrypointSelectionType, TelemetryEvent,
};
use crate::server::telemetry::{AppStartupInfo, CloseTarget, PaletteSource, TelemetryCollector};
use crate::session_management::{RunningSessionSummary, SessionNavigationData};
use crate::settings::cloud_preferences_syncer::initialize_cloud_preferences_syncer;
use crate::settings::manager::SettingsManager;
use crate::settings::{AISettings, AccessibilitySettings, ScrollSettings, SelectionSettings};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::DisplayCount;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::SystemStats;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::view::inline_banner::ByoLlmAuthBannerSessionState;
use crate::terminal::{AudibleBell, CustomSecretRegexUpdater, History};
#[cfg(feature = "tui")]
pub use crate::tui::{TuiLoginModel, TuiLoginPhase};
use crate::undo_close::UndoCloseStack;
use crate::user_config::WarpConfig;
use crate::util::bindings::is_binding_cross_platform;
use crate::vim_registers::VimRegisters;
use crate::warp_managed_paths_watcher::{ensure_warp_watch_roots_exist, WarpManagedPathsWatcher};
use crate::workflows::aliases::WorkflowAliases;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspace::{
    ActiveSession, OneTimeModalModel, PaneViewLocator, ToastStack, Workspace, WorkspaceAction,
};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Our embedded application assets.
pub static ASSETS: warp_assets::Assets = warp_assets::Assets;

fn determine_agent_source(
    launch_mode: &LaunchMode,
) -> Option<crate::ai::ambient_agents::AgentSource> {
    match launch_mode {
        LaunchMode::CommandLine { .. } => {
            if std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true") {
                Some(crate::ai::ambient_agents::AgentSource::GitHubAction)
            } else {
                Some(crate::ai::ambient_agents::AgentSource::Cli)
            }
        }
        LaunchMode::App { .. } | LaunchMode::Test { .. } => {
            Some(crate::ai::ambient_agents::AgentSource::CloudMode)
        }
        // RemoteServerProxy and RemoteServerDaemon are headless server
        // processes that don't use the agent subsystem.
        // TODO: the TUI front-end has no agent harness wired up yet; give it an
        // appropriate `AgentSource` once that lands.
        LaunchMode::RemoteServerProxy
        | LaunchMode::RemoteServerDaemon { .. }
        | LaunchMode::Tui { .. } => None,
    }
}

#[cfg(feature = "local_fs")]
fn daemon_codebase_index_snapshot_storage(launch_mode: &LaunchMode) -> Option<SnapshotStorage> {
    match launch_mode {
        LaunchMode::RemoteServerDaemon { identity_key } => {
            let data_dir = remote_server::setup::remote_server_daemon_data_dir(identity_key);
            let snapshot_dir = PathBuf::from(tilde(&data_dir).into_owned())
                .join("cache")
                .join("codebase_index_snapshots");
            SnapshotStorage::from_dir(snapshot_dir)
        }
        LaunchMode::App { .. }
        | LaunchMode::CommandLine { .. }
        | LaunchMode::RemoteServerProxy
        | LaunchMode::Test { .. }
        | LaunchMode::Tui { .. } => None,
    }
}

/// Launch mode for how to start up Warp.
#[allow(clippy::large_enum_variant)]
pub(crate) enum LaunchMode {
    /// Run the regular GUI application.
    App {
        args: warp_cli::AppArgs,
        /// API key for server authentication, if provided via `--api-key` or `WARP_API_KEY`.
        /// Only used on dogfood channels.
        api_key: Option<String>,
    },

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

    /// Remote server proxy — bridges SSH stdio to the daemon's Unix socket.
    /// This is a short-lived process that runs for the lifetime of an SSH session.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    RemoteServerProxy,

    /// Remote server daemon — long-lived headless process serving remote
    /// connections via a Unix domain socket.
    #[cfg_attr(not(unix), allow(dead_code))]
    RemoteServerDaemon {
        /// Stable identity key used to partition the daemon's socket/PID
        /// directory on the remote host.
        identity_key: String,
    },

    /// Run the headless TUI front-end (the `warp-tui` binary in the `warp_tui`
    /// crate). Boots the real headless app so auth/agent state can be reused,
    /// then renders an editor-backed input UI to the terminal (via `mount`)
    /// instead of opening a GUI window.
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    Tui {
        /// Builds the root TUI view and starts the TUI driver. Runs after
        /// `initialize_app`; supplied by [`run_tui`]. Carried in the variant
        /// (rather than as a `run_internal` parameter) so it stays scoped to
        /// this mode.
        mount: TuiMountFn,
    },
}

impl LaunchMode {
    fn args(&self) -> Cow<'_, warp_cli::AppArgs> {
        match self {
            LaunchMode::App { args, .. } => Cow::Borrowed(args),
            LaunchMode::CommandLine { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::Tui { .. } => Cow::Owned(warp_cli::AppArgs::default()),
        }
    }

    /// Returns `true` if this process is running an integration test.
    fn is_integration_test(&self) -> bool {
        match self {
            LaunchMode::Test {
                is_integration_test,
                ..
            } => *is_integration_test,
            LaunchMode::App { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::Tui { .. } => false,
        }
    }

    /// The settings surface for this launch mode. The TUI front-end gets its
    /// own settings file and local-only (non-cloud-synced) config; every other
    /// mode uses the standard GUI settings surface.
    fn settings_mode(&self) -> ::settings::SettingsMode {
        match self {
            LaunchMode::Tui { .. } => ::settings::SettingsMode::Tui,
            LaunchMode::App { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. } => ::settings::SettingsMode::Gui,
        }
    }

    fn take_test_driver(&mut self) -> Option<TestDriver> {
        match self {
            LaunchMode::Test { driver, .. } => driver.take(),
            LaunchMode::App { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::Tui { .. } => None,
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
            // The TUI front-end is an app-style client, not the SDK.
            LaunchMode::Tui { .. } => ExecutionMode::App,
            // RemoteServerProxy is a thin byte bridge; Sdk is the closest match.
            LaunchMode::RemoteServerProxy => ExecutionMode::Sdk,
            // RemoteServerDaemon gets its own mode for distinct Sentry tagging.
            LaunchMode::RemoteServerDaemon { .. } => ExecutionMode::RemoteServerDaemon,
        }
    }

    fn is_sandboxed(&self) -> bool {
        match self {
            LaunchMode::CommandLine { is_sandboxed, .. } => *is_sandboxed,
            LaunchMode::App { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::Tui { .. } => false,
        }
    }

    /// Returns `true` if Warp should run headlessly, without a visible UI.
    fn is_headless(&self) -> bool {
        match self {
            LaunchMode::CommandLine { command, .. } => match command {
                CliCommand::Agent(AgentCommand::Run(args)) => !args.gui,
                _ => true,
            },
            LaunchMode::RemoteServerProxy | LaunchMode::RemoteServerDaemon { .. } => true,
            // The TUI front-end renders to the terminal, with no GUI window.
            LaunchMode::Tui { .. } => true,
            LaunchMode::App { .. } | LaunchMode::Test { .. } => false,
        }
    }

    /// Returns `true` if this process can build and sync codebase indices.
    fn supports_indexing(&self) -> bool {
        match self {
            LaunchMode::CommandLine { command, .. } => {
                matches!(command, CliCommand::Agent(AgentCommand::Run { .. }))
            }
            LaunchMode::RemoteServerDaemon { .. } => {
                FeatureFlag::RemoteCodebaseIndexing.is_enabled()
            }
            LaunchMode::App { .. } | LaunchMode::Test { .. } => true,
            LaunchMode::RemoteServerProxy => false,
            // Codebase indexing stays off for the TUI until it has deferred
            // persisted-index restore and multi-process-safe snapshot writes
            // (the GUI may run concurrently against the same data dir).
            // Project rules/skills discovery does not depend on this; see
            // `PersistedWorkspace::new`.
            LaunchMode::Tui { .. } => false,
        }
    }

    /// Whether or not to start a crash recovery process (on platforms that support it).
    #[cfg(enable_crash_recovery)]
    pub(crate) fn crash_recovery_enabled(&self) -> bool {
        match self {
            LaunchMode::App { .. } => true,
            LaunchMode::CommandLine { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::Tui { .. } => false,
        }
    }

    /// Whether Sentry / crash reporting should be initialized.
    #[cfg_attr(not(feature = "crash_reporting"), allow(dead_code))]
    pub(crate) fn needs_crash_reporting(&self) -> bool {
        match self {
            LaunchMode::App { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::Tui { .. } => true,
        }
    }

    /// Whether profiling and tracing should be initialized.
    pub(crate) fn needs_profiling(&self) -> bool {
        match self {
            LaunchMode::App { .. }
            | LaunchMode::CommandLine { .. }
            | LaunchMode::Test { .. }
            | LaunchMode::RemoteServerDaemon { .. }
            | LaunchMode::RemoteServerProxy
            | LaunchMode::Tui { .. } => true,
        }
    }

    /// Log destination for this mode.
    fn log_destination(&self) -> Option<LogDestination> {
        match self {
            LaunchMode::CommandLine { debug, .. } => {
                if *debug {
                    Some(LogDestination::Stderr)
                } else {
                    Some(LogDestination::File)
                }
            }
            // Proxy must log to stderr because stdout is the protocol channel.
            LaunchMode::RemoteServerProxy => Some(LogDestination::Stderr),
            LaunchMode::RemoteServerDaemon { .. } => Some(LogDestination::File),
            // A TUI owns the terminal, so logs go to a file; stdout/stderr would
            // corrupt the rendered output and the device-code prompt.
            LaunchMode::Tui { .. } => Some(LogDestination::File),
            LaunchMode::App { .. } | LaunchMode::Test { .. } => None,
        }
    }

    fn as_str_for_tracing(&self) -> &'static str {
        match self {
            LaunchMode::App { .. } => "app",
            LaunchMode::CommandLine { command, .. } => command.as_str_for_tracing(),
            LaunchMode::Test { .. } => "test",
            LaunchMode::RemoteServerDaemon { .. } => "remote_server_daemon",
            LaunchMode::RemoteServerProxy => "remote_server_proxy",
            LaunchMode::Tui { .. } => "tui",
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

/// Runs the shared Warp executable as the app or as one of its command-line modes.
///
/// The bundled Warp Control wrapper injects `--warpctrl`, which is dispatched
/// before the normal Warp/Oz parser. Oz subcommands are part of that normal
/// parser and therefore do not require a separate mode flag.
#[::tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
pub fn run() -> Result<()> {
    // Perform any necessary platform-specific initialization.
    platform::init();

    // Ensure feature flags are initialized before parsing command-line arguments.
    features::init_feature_flags();
    if let Some(args) = warp_cli::local_control::ControlArgs::from_control_mode_env() {
        #[cfg(windows)]
        warp_util::windows::attach_to_parent_console();
        warp_cli::local_control::run_and_exit(args);
    }

    // Parse command-line arguments.
    let args = warp_cli::Args::from_env();

    // Server URL overrides are only honored on internal dev channels. Release channels silently
    // ignore `--server-root-url` / `--ws-server-url` / `--session-sharing-server-url` (and their
    // `WARP_*` env-var equivalents) so shipped builds can't be redirected away from their
    // baked-in server URLs. See `Channel::allows_server_url_overrides`.
    if ChannelState::channel().allows_server_url_overrides() {
        if let Some(url) = args.server_root_url() {
            if let Err(e) = ChannelState::override_server_root_url(url.to_owned()) {
                eprintln!("Error: Invalid server root URL: {e:#}");
            }
        }

        if let Some(url) = args.ws_server_url() {
            if let Err(e) = ChannelState::override_ws_server_url(url.to_owned()) {
                eprintln!("Error: Invalid websocket server URL: {e:#}");
            }
        }

        if let Some(url) = args.session_sharing_server_url() {
            if let Err(e) = ChannelState::override_session_sharing_server_url(url.to_owned()) {
                eprintln!("Error: Invalid session sharing server URL: {e:#}");
            }
        }
    }

    if let Some(command) = args.command() {
        #[cfg(windows)]
        if command.prints_to_stdout() {
            // We attach a console to ensure that all standard output gets printed correctly.
            warp_util::windows::attach_to_parent_console();
        }
        match command {
            warp_cli::Command::Worker(worker) => return run_worker_command(worker),
            warp_cli::Command::Completions { shell } => {
                return warp_cli::completions::generate_to_stdout(*shell);
            }
            warp_cli::Command::CommandLine(cmd) => {
                let (is_sandboxed, computer_use_override) = match cmd.as_ref() {
                    warp_cli::CliCommand::Agent(warp_cli::agent::AgentCommand::Run(run_args)) => (
                        run_args.sandboxed,
                        run_args.computer_use.computer_use_override(),
                    ),
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
            #[cfg(not(target_family = "wasm"))]
            warp_cli::Command::PrintTelemetryEvents => {
                return TelemetryEvent::print_telemetry_events_json();
            }
        }
    }

    // If running as a standalone CLI binary or invoked as "oz", print help
    // instead of launching the GUI app.
    let is_cli_binary = cfg!(feature = "standalone")
        || warp_cli::binary_name().is_some_and(|name| name.starts_with("oz"))
        || std::env::var_os("WARP_CLI_MODE").is_some();
    if is_cli_binary {
        warp_cli::Args::clap_command().print_help()?;
        return Ok(());
    }

    let api_key = args.api_key().cloned();
    run_internal(LaunchMode::App {
        args: args.into_app_args(),
        api_key,
    })
}

/// Runs a parsed Warp worker command.
fn run_worker_command(worker: &warp_cli::WorkerCommand) -> Result<()> {
    match worker {
        #[cfg(all(feature = "local_tty", unix))]
        warp_cli::WorkerCommand::TerminalServer(args) => {
            crate::terminal::local_tty::server::run_terminal_server(args);
            Ok(())
        }
        #[cfg(feature = "plugin_host")]
        warp_cli::WorkerCommand::PluginHost { .. } => crate::run_plugin_host(),
        #[cfg(feature = "local_tty")]
        warp_cli::WorkerCommand::MinidumpServer { socket_name } => {
            cfg_if::cfg_if! {
                if #[cfg(all(linux_or_windows, feature = "crash_reporting"))] {
                    crate::crash_reporting::run_minidump_server(socket_name)
                } else {
                    let _ = socket_name;
                    panic!("The minidump server is not supported on this platform");
                }
            }
        }
        #[cfg(not(target_family = "wasm"))]
        warp_cli::WorkerCommand::RemoteServerProxy(args) => {
            // Proxy is a thin byte bridge (stdin/stdout ↔ Unix socket).
            // It only needs logging to stderr since stdout is the protocol
            // channel. No crash reporting, no initialize_app.
            let launch_mode = LaunchMode::RemoteServerProxy;
            let mut tracing_initialization = tracing::init()?;
            warp_logging::init(warp_logging::LogConfig {
                is_cli: true,
                log_destination: launch_mode.log_destination(),
                ..Default::default()
            })?;
            tracing_initialization.log_initialization_warning();
            crate::remote_server::run_proxy(args.identity_key.clone())
        }
        #[cfg(not(target_family = "wasm"))]
        warp_cli::WorkerCommand::RemoteServerDaemon(args) => {
            // Daemon handles its own full initialization (including
            // initialize_app and crash reporting) inside run_daemon_app.
            crate::remote_server::run_daemon(args.identity_key.clone())
        }
        #[cfg(not(target_family = "wasm"))]
        warp_cli::WorkerCommand::RipgrepSearch {
            parent,
            ignore_case,
            multiline,
            pattern,
            paths,
        } => {
            warp_ripgrep::search::run_search_subprocess(
                std::slice::from_ref(pattern),
                paths.clone(),
                *ignore_case,
                *multiline,
                parent.pid,
            )
            .map_err(|err| anyhow!(err.to_string()))?;
            Ok(())
        }
        #[cfg(not(any(
            feature = "local_tty",
            feature = "plugin_host",
            not(target_family = "wasm")
        )))]
        worker => {
            // On wasm, specifically, we should fail spectacularly if we get here.
            #[cfg(target_family = "wasm")]
            panic!("Worker process not supported on WASM: {worker:?}")
        }
    }
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

/// Runs the headless TUI front-end (the `warp-tui` binary in the `warp_tui`
/// crate). Bootstraps the real (headless) app and then runs `mount`, which
/// builds the root TUI view and starts the non-blocking TUI driver.
///
/// `mount` is supplied by the `warp_tui` crate (which owns the concrete root
/// view plus the window/driver bootstrap), so `warp` never has to depend on
/// `warp_tui`.
#[cfg(feature = "tui")]
pub fn run_tui(mount: TuiMountFn) -> Result<()> {
    run_internal(LaunchMode::Tui { mount })
}

/// Dispatches a worker command when the current executable was re-invoked for one.
#[cfg(feature = "tui")]
pub fn run_tui_worker_if_requested() -> Option<Result<()>> {
    // Worker spawners always put the worker mode in argv[1]. Do not scan later
    // arguments because a TUI prompt value may legitimately match a worker name.
    let is_worker = std::env::args()
        .nth(1)
        .is_some_and(|arg| warp_cli::is_worker_invocation(&arg));
    if !is_worker {
        return None;
    }

    features::init_feature_flags();
    let args = warp_cli::Args::from_env();
    let Some(warp_cli::Command::Worker(worker)) = args.command() else {
        return Some(Err(anyhow!(
            "Recognized a Warp worker invocation, but failed to parse its worker command"
        )));
    };
    Some(run_worker_command(worker))
}

/// The headless TUI front-end's mount callback, carried by [`LaunchMode::Tui`].
/// Supplied to [`run_tui`] by the `warp_tui` crate; it runs after
/// `initialize_app` to build the root TUI view and start the TUI driver.
pub type TuiMountFn = Box<dyn FnOnce(&mut warpui::AppContext)>;

/// Runs the app (or CLI / daemon). For [`LaunchMode::Tui`] it runs the mount
/// carried by the variant after `initialize_app` (building the root TUI view and
/// starting the driver) in place of the GUI/CLI `launch()` path.
fn run_internal(mut launch_mode: LaunchMode) -> Result<()> {
    let mut timer = IntervalTimer::new();

    // ── Early initialization (pre-AppBuilder) ──────────────────────
    // These steps run before the platform event loop is started.
    // They must not depend on AppContext.

    #[cfg(windows)]
    dynamic_libraries::configure_library_loading();

    if launch_mode.needs_profiling() {
        profiling::init();
    }

    // The `run` function already initializes feature flags, but ensure they're initialized here
    // for other entrypoints.
    features::init_feature_flags();

    #[cfg(feature = "crash_reporting")]
    if launch_mode.needs_crash_reporting() {
        // Ensure that the main/root Sentry hub is initialized on the main
        // thread.  PtySpawner creates a background thread to receive logs from
        // the terminal server process, and we don't want it to be the host of
        // the primary sentry::Hub.
        sentry::Hub::main();
    }

    let mut tracing_initialization = launch_mode
        .needs_profiling()
        .then(tracing::init)
        .transpose()?;

    // Start the `run_internal` span here - we can't do it before this point
    // because we need the tracing initialization to be complete first.
    let span = ::tracing::info_span!(
        "run_internal",
        tags.cloud_agent = true,
        launch_mode = launch_mode.as_str_for_tracing()
    );
    let _enter = span.enter();

    let log_destination = launch_mode.log_destination();
    let is_cli = log_destination.is_some();

    cfg_if::cfg_if! {
        if #[cfg(enable_crash_recovery)] {
            if crash_recovery::is_crash_recovery_process(launch_mode.args().as_ref()) {
                warp_logging::init_for_crash_recovery_process()?;
            } else {
                warp_logging::init(warp_logging::LogConfig {
                    is_cli,
                    log_destination,
                    ..Default::default()
                })?;
            }
        } else {
            warp_logging::init(warp_logging::LogConfig {
                is_cli,
                log_destination,
                ..Default::default()
            })?;
        }
    }

    if let Some(initialization) = tracing_initialization.as_mut() {
        initialization.log_initialization_warning();
    }
    timer.mark_interval_end("LOG_FILE_SETUP_COMPLETE");

    #[cfg(windows)]
    platform::windows::check_redirection_guard();

    // Adjust resource limits early, before doing other work, to ensure that
    // any children we spawn (like the terminal server) inherit our adjusted
    // rlimits.
    resource_limits::adjust_resource_limits();

    // For wasm builds we have this special case to parse out the intent
    // from the url that is used to visite the app on web.
    #[cfg(target_family = "wasm")]
    {
        use uri::web_intent_parser;
        if let Some(intent) = web_intent_parser::parse_web_intent_from_current_url() {
            launch_mode.add_url(intent);
        }
        web_intent_parser::set_context_flags_from_current_url();
    }

    // Collect errors that occur in run_internal() before the Sentry client is initialized,
    // so they can be replayed to Sentry once it's ready.
    #[cfg_attr(
        not(all(
            feature = "release_bundle",
            any(windows, any(target_os = "linux", target_os = "freebsd"))
        )),
        expect(unused_mut)
    )]
    let mut pre_sentry_errors: Vec<anyhow::Error> = Vec::new();

    #[cfg(all(
        feature = "release_bundle",
        any(target_os = "linux", target_os = "freebsd")
    ))]
    if let LaunchMode::App { .. } = launch_mode {
        match app_services::linux::pass_startup_args_to_existing_instance(
            launch_mode.args().as_ref(),
        ) {
            // If we were able to contact an existing application instance, quit -
            // we only want to run a single instance of Warp at a time.
            Ok(_) => std::process::exit(0),
            // If Warp isn't already running, we're good to go.
            Err(app_services::linux::StartupArgsForwardingError::NoExistingInstance) => {}
            // If we just finished an auto-update, we should continue running.
            Err(app_services::linux::StartupArgsForwardingError::IgnoredAfterAutoUpdate) => {}
            // If we were unable to perform the forwarding for an unknown reason,
            // it's better to run a second instance than potentially end up in a
            // state where Warp refuses to run even a first instance.
            Err(err) => {
                let err = anyhow::Error::from(err).context("Failed to forward startup args");
                report_error!(&err);
                pre_sentry_errors.push(err);
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
            // If we just finished an auto-update, we should continue running.
            Err(app_services::windows::StartupArgsForwardingError::IgnoredAfterAutoUpdate) => {}
            // If we were unable to perform the forwarding for an unknown reason,
            // it's better to run a second instance than potentially end up in a
            // state where Warp refuses to run even a first instance.
            Err(err) => {
                let err = anyhow::Error::from(err).context("Failed to forward startup args");
                report_error!(&err);
                pre_sentry_errors.push(err);
            }
        }
    }

    // Sets up a Job Object that we associate with the Warp process to handle
    // shared fate with its child processes. This should be called before we
    // start spawning any child processes.
    #[cfg(windows)]
    command::windows::init();

    // Establish the settings surface (GUI vs TUI) before initializing
    // preferences so the settings infra selects the right file name and
    // cloud-sync behavior for this launch mode.
    ::settings::set_settings_mode(launch_mode.settings_mode());

    let private_preferences = settings::init_private_user_preferences();
    let (public_preferences, startup_toml_parse_error) = settings::init_public_user_preferences();

    // When the SettingsFile feature flag is enabled, public settings live in
    // the TOML-backed store. When disabled, they live in the platform-native
    // store (same backend as private). Use the correct one for pre-app reads.
    #[cfg_attr(
        not(any(
            enable_crash_recovery,
            target_os = "linux",
            target_os = "freebsd",
            target_os = "macos"
        )),
        expect(unused)
    )]
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
    //
    #[cfg(feature = "local_tty")]
    let pty_spawner =
        terminal::local_tty::spawner::PtySpawner::new().context("Failed to create pty spawner")?;

    // The TUI front-end skips the GUI lifecycle callbacks (which reach for
    // singletons/windows it never creates), so it uses empty callbacks.
    let callbacks = if matches!(launch_mode, LaunchMode::Tui { .. }) {
        warpui::platform::AppCallbacks::default()
    } else {
        app_callbacks(
            launch_mode.is_integration_test(),
            tracing_initialization.take(),
        )
    };
    let mut app_builder = if launch_mode.is_headless() {
        warpui::platform::AppBuilder::new_headless(
            callbacks,
            Box::new(ASSETS),
            launch_mode.take_test_driver(),
        )
    } else {
        warpui::platform::AppBuilder::new(
            callbacks,
            Box::new(ASSETS),
            launch_mode.take_test_driver(),
        )
    };

    #[cfg(target_os = "macos")]
    {
        use warpui::platform::mac::AppExt;
        use warpui::AssetProvider as _;

        let activate_on_launch = !launch_mode.is_integration_test()
            || std::env::var("WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS").is_ok();
        app_builder.set_activate_on_launch(activate_on_launch);

        let dev_icon = ASSETS.get("bundled/png/local.png")?;
        app_builder.set_dev_icon(dev_icon);

        let show_dock_icon = crate::settings::app_icon::ShowDockIconState::read_from_preferences(
            prefs_for_public_settings,
        )
        .unwrap_or_else(crate::settings::app_icon::ShowDockIconState::default_value);
        app_builder.set_show_dock_icon_on_launch(show_dock_icon);
        app_builder.set_menu_bar_builder(app_menus::menu_bar);
        app_builder.set_dock_menu_builder(|_| app_menus::dock_menu());
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use warpui::platform::linux::{self, AppBuilderExt};

        use crate::settings::ForceX11;

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
        #[cfg(not(target_family = "wasm"))]
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
        #[cfg(feature = "crash_reporting")]
        crate::crash_reporting::set_client_type_tag(launch_mode.execution_mode().client_id());

        // Add the terminal server singleton to the application.
        #[cfg(feature = "local_tty")]
        ctx.add_singleton_model(move |_ctx| pty_spawner);

        // Register user preferences.  This must be done before initializing
        // feature flags or experiments, both of which check user preferences for
        // overrides.
        ctx.add_singleton_model(move |_ctx| ::settings::PublicPreferences::new(public_preferences));
        ctx.add_singleton_model(move |_ctx| private_preferences);
        let startup_toml_parse_error = startup_toml_parse_error;

        #[cfg(enable_crash_recovery)]
        ctx.add_singleton_model(move |_ctx| crash_recovery);

        #[cfg(feature = "plugin_host")]
        ctx.add_singleton_model(move |ctx| {
            plugin::PluginHost::new(ctx).expect("Could not instantiate PluginHost")
        });
        let app_state = initialize_app(
            &launch_mode,
            timer,
            startup_toml_parse_error,
            ctx,
            pre_sentry_errors,
        );

        if ImprovedPaletteSearch::improved_search_enabled(ctx) {
            FeatureFlag::UseTantivySearch.set_enabled(true);
        }

        // The TUI front-end reuses the full `initialize_app` bootstrap above (so
        // auth, `Appearance`, settings, etc. exist), then runs the device-login
        // flow and mounts the TUI (via `crate::tui::init`) instead of the
        // GUI/CLI `launch()` path.
        match launch_mode {
            #[cfg(feature = "tui")]
            LaunchMode::Tui { mount } => crate::tui::init(mount, ctx),
            #[cfg(not(feature = "tui"))]
            LaunchMode::Tui { .. } => {
                unreachable!("the `tui` launch mode requires the `tui` feature")
            }
            other => launch(ctx, app_state, other),
        }
    })
}

pub struct UpdateQuakeModeEventArg {
    active_window_id: Option<WindowId>,
}

#[::tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
pub(crate) fn initialize_app(
    launch_mode: &LaunchMode,
    mut timer: IntervalTimer,
    startup_toml_parse_error: Option<warpui_extras::user_preferences::Error>,
    ctx: &mut warpui::AppContext,
    _pre_sentry_errors: impl IntoIterator<Item = anyhow::Error>,
) -> Option<AppState> {
    // WARNING: Errors that happen here before crash_reporting::init will not be collected in
    // Sentry. Only the dependencies of crash_reporting should be initialized here. Avoid adding
    // any other stuff here, as failures will be silent. Push them to pre_sentry_errors instead.
    let data_domain = ChannelState::data_domain();

    // Daemon auth arrives through the client handshake, so avoid platform keychains that may
    // require an interactive unlock prompt. Other headless modes still use secure storage for
    // persisted login and BYO provider credentials.
    if matches!(launch_mode, LaunchMode::RemoteServerDaemon { .. }) {
        warpui_extras::secure_storage::register_unavailable(ctx);
    } else {
        // Register an implementation of the secure storage service.
        cfg_if::cfg_if! {
            if #[cfg(feature = "integration_tests")] {
                warpui_extras::secure_storage::register_noop(&data_domain, ctx);
            } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
                warpui_extras::secure_storage::register_with_fallback(&data_domain, warp_core::paths::state_dir(), ctx)
            } else if #[cfg(target_os = "windows")] {
                warpui_extras::secure_storage::register_with_dir(&data_domain, warp_core::paths::state_dir(), ctx)
            } else {
                warpui_extras::secure_storage::register(&data_domain, ctx);
            }
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

    // Extract API key from command line options, if applicable.
    let api_key = match launch_mode {
        LaunchMode::CommandLine { global_options, .. } => global_options.api_key.clone(),
        LaunchMode::App { api_key, .. } if ChannelState::channel().is_dogfood() => api_key.clone(),
        _ => None,
    };
    let api_key = if FeatureFlag::APIKeyAuthentication.is_enabled() {
        api_key
    } else {
        None
    };

    // A key supplied to an App launch but dropped here means Warp will start logged out
    // (non-dogfood channel or feature disabled). Surface this loudly so it isn't silent.
    if api_key.is_none()
        && matches!(
            launch_mode,
            LaunchMode::App {
                api_key: Some(_),
                ..
            }
        )
    {
        let channel = ChannelState::channel();
        let warning = format!(
            "WARNING: --api-key/WARP_API_KEY was provided but IGNORED on the '{channel}' channel — Warp is starting LOGGED OUT. API-key auth is only available on internal (dogfood) builds."
        );
        eprintln!("{warning}");
    }

    let auth_state = Arc::new(AuthState::initialize(ctx, api_key));
    timer.mark_interval_end("AUTH_MANAGER_SET_USER");

    let agent_source = determine_agent_source(launch_mode);

    // NetworkLogModel must be registered before ServerApiProvider so that
    // `NetworkLogModel::install_on_clients` can reach it when forwarding items
    // captured by the HTTP client hooks.
    ctx.add_singleton_model(|_ctx| NetworkLogModel::default());

    // Create a shared IAP state for staging builds. The same `Arc<IapState>`
    // is handed to both `ServerApi` (for sync reads on the request path) and
    // `IapManager` (which owns refresh logic on the main thread).
    #[cfg(not(target_family = "wasm"))]
    let iap_state = ChannelState::iap_config().map(|cfg| Arc::new(IapState::new(&cfg)));
    #[cfg(target_family = "wasm")]
    let iap_state: Option<Arc<IapState>> = None;

    let server_api_provider = ctx.add_singleton_model({
        let auth_state = auth_state.clone();
        let iap_state = iap_state.clone();
        move |ctx| ServerApiProvider::new(auth_state, agent_source, iap_state, ctx)
    });

    let server_api = server_api_provider.as_ref(ctx).get();
    #[cfg(not(target_family = "wasm"))]
    if let Ok(run_id) = std::env::var(warp_cli::OZ_RUN_ID_ENV) {
        match run_id.parse() {
            Ok(task_id) => server_api.set_ambient_agent_task_id(Some(task_id)),
            Err(err) => log::warn!("Ignoring invalid {}: {err}", warp_cli::OZ_RUN_ID_ENV),
        }
    }
    let ai_client = server_api_provider.as_ref(ctx).get_ai_client();
    #[cfg(not(target_family = "wasm"))]
    // Refresh starts only after the authenticated server client exists; tracing initialization
    // remains responsible for deciding whether this process opted in to cloud-agent export.
    tracing::start_auth_refresh(
        server_api_provider.as_ref(ctx).get_managed_secrets_client(),
        ctx,
    );

    ctx.add_singleton_model(|_ctx| AuthStateProvider::new(auth_state.clone()));

    ctx.add_singleton_model(AppTelemetryContextProvider::new_context_provider);

    ctx.add_singleton_model(|ctx| {
        AuthManager::new(
            server_api.clone(),
            server_api_provider.as_ref(ctx).get_auth_client(),
            ctx,
        )
    });

    ctx.add_singleton_model(|_ctx| GPUState::new());

    PrivacySettings::register_singleton(ctx);

    // If any part of sqlite initialization fails, we just don't do session restoration (i.e.
    // feature degradation).
    let persistence_scope = match launch_mode {
        LaunchMode::RemoteServerDaemon { identity_key } => {
            persistence::PersistenceScope::RemoteServerDaemon {
                identity_key: identity_key.clone(),
            }
        }
        // The TUI keeps its own database so GUI/TUI version skew can never
        // migrate a shared database out from under the older binary.
        LaunchMode::Tui { .. } => persistence::PersistenceScope::Tui,
        LaunchMode::App { .. }
        | LaunchMode::CommandLine { .. }
        | LaunchMode::RemoteServerProxy
        | LaunchMode::Test { .. } => persistence::PersistenceScope::App,
    };
    // Only read the subsets of persisted data this launch mode actually
    // consumes; loading everything is expensive on large databases.
    let persisted_data_scope = match launch_mode {
        LaunchMode::Tui { .. } => persistence::PersistedDataScope::TuiFrontend,
        LaunchMode::RemoteServerDaemon { .. } => {
            persistence::PersistedDataScope::CodebaseIndicesOnly
        }
        LaunchMode::App { .. }
        | LaunchMode::CommandLine { .. }
        | LaunchMode::RemoteServerProxy
        | LaunchMode::Test { .. } => persistence::PersistedDataScope::Full,
    };
    let (sqlite_data, writer_handles) =
        persistence::initialize(ctx, persistence_scope, persisted_data_scope);
    timer.mark_interval_end("SQLITE_INITIALIZED");

    let persistence_writer = PersistenceWriter::new(writer_handles);

    let model_event_sender = persistence_writer.sender();

    let referral_theme_status = ctx.add_model(ReferralThemeStatus::new);
    let tips_handle = ctx.add_model(|_| user_defaults_on_startup.tips_data);
    let user_default_shell_unsupported_banner_model_handle =
        ctx.add_model(|_| user_defaults_on_startup.user_default_shell_unsupported_banner_state);
    // Extract the full-file parse error (if any) before the settings_file_error
    // value is moved below. Only FileParseFailed gates the broken-file guard
    // in `initialize_cloud_preferences_syncer`; InvalidSettings means TOML
    // parsed but individual values were wrong, which doesn't mean local
    // state is unusable.
    let startup_toml_parse_error_for_syncer = user_defaults_on_startup
        .settings_file_error
        .as_ref()
        .and_then(|err| match err {
            settings::SettingsFileError::FileParseFailed(msg) => Some(msg.clone()),
            settings::SettingsFileError::InvalidSettings(_) => None,
        });
    let settings_file_error = user_defaults_on_startup.settings_file_error;
    ctx.add_singleton_model(move |_ctx| {
        GlobalResourceHandlesProvider::new(GlobalResourceHandles {
            model_event_sender,
            tips_completed: tips_handle,
            referral_theme_status,
            user_default_shell_unsupported_banner_model_handle,
            settings_file_error,
        })
    });

    let (
        cloud_objects,
        cached_workspaces,
        current_workspace_uid,
        app_state,
        command_history,
        restored_user_profiles,
        time_of_next_force_object_refresh,
        object_actions,
        experiments,
        ai_queries,
        nld_prompts,
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
                sqlite_data.cloud_objects,
                sqlite_data.workspaces,
                sqlite_data.current_workspace_uid,
                sqlite_data.app_state,
                sqlite_data.command_history,
                sqlite_data.user_profiles,
                sqlite_data.time_of_next_force_object_refresh,
                sqlite_data.object_actions,
                sqlite_data.experiments,
                sqlite_data.ai_queries,
                sqlite_data.nld_prompts,
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
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            )
        });

    // The daemon's `PersistedDataScope::CodebaseIndicesOnly` read already
    // skips everything except codebase index metadata.
    if matches!(launch_mode, LaunchMode::RemoteServerDaemon { .. }) {
        let codebase_index_count = persisted_workspaces.len();
        log::debug!(
            "[Remote codebase indexing] Restored daemon codebase index metadata: metadata_count={codebase_index_count}"
        );
    }

    // Initialize a global model to track server-side experiment state.
    // This depends on the [`GlobalResourceHandlesProvider`] and so it must
    // be initialized after it.
    ctx.add_singleton_model(|ctx| ServerExperiments::new_from_cache(experiments, ctx));

    ctx.add_singleton_model(|ctx| AIRequestUsageModel::new(ai_client, ctx));

    ctx.add_singleton_model(|ctx| {
        UserWorkspaces::new(
            server_api_provider.as_ref(ctx).get_team_client(),
            server_api_provider.as_ref(ctx).get_workspace_client(),
            cached_workspaces,
            current_workspace_uid,
            ctx,
        )
    });

    // Initialize ApiKeyManager after UserWorkspaces so it can subscribe to workspace/settings changes
    ctx.add_singleton_model(|ctx| {
        #[cfg_attr(target_family = "wasm", allow(unused_mut))]
        let mut manager = ::ai::api_keys::ApiKeyManager::new(ctx);
        #[cfg(not(target_family = "wasm"))]
        manager.subscribe_to_settings_changes(ctx);
        // Gemini Enterprise (GEAP) credential refresh triggers: workspace
        // settings saves / team changes and the member's enablement toggle.
        #[cfg(not(target_family = "wasm"))]
        if FeatureFlag::GeminiEnterprise.is_enabled() {
            manager.subscribe_to_geap_settings_changes(ctx);
        }
        // The Grok subscription refresher (`ai::grok_subscription`) has no
        // visibility into workspace policy, so wire the BYO API key policy in
        // here. The initial value resumes proactive refresh of any tokens
        // restored from secure storage; TeamsChanged keeps the policy aligned
        // as team data loads or the workspace changes.
        #[cfg(not(target_family = "wasm"))]
        if FeatureFlag::SuperGrok.is_enabled() {
            use crate::workspaces::user_workspaces::UserWorkspacesEvent;
            ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, _, event, ctx| {
                if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                    let allowed = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx);
                    manager.set_grok_refresh_allowed(allowed, ctx);
                }
            });
            let allowed = UserWorkspaces::as_ref(ctx).is_byo_api_key_enabled(ctx);
            manager.set_grok_refresh_allowed(allowed, ctx);
        }
        manager
    });

    ctx.add_singleton_model(AntivirusInfo::new);

    cfg_if::cfg_if! {
        if #[cfg(feature = "crash_reporting")] {
            let is_crash_reporting_enabled = crash_reporting::init(ctx);
        } else {
            let is_crash_reporting_enabled = false;
        }
    }
    // Send buffered pre-init errors to Sentry now that the client is ready.
    #[cfg(feature = "crash_reporting")]
    for err in _pre_sentry_errors {
        sentry::integrations::anyhow::capture_anyhow(&err);
    }
    timer.mark_interval_end("INIT_CRASH_REPORTING");

    if let LaunchMode::App { .. } = launch_mode {
        autoupdate::check_and_report_update_errors(ctx);
    }

    ctx.set_fallback_font_source_provider(|url| ::asset_cache::url_source(url));

    ctx.set_default_binding_validator(is_binding_cross_platform);

    if FeatureFlag::Autoupdate.is_enabled() {
        // Attempt to clean up any old executable, whether or not we were
        // explicitly launched as part of the auto-update process.  We may have
        // failed to remove the executable on a previous launch of the app and
        // should try again.
        if let Err(e) = autoupdate::remove_old_executable() {
            report_error!(e.context("Failed to remove old executable"));
        }
    }

    experiments::init(ctx);

    // Initialize timestamp for session id and last active event
    App::record_last_active_timestamp();

    ctx.add_singleton_model(|_| SettingsPaneManager::new());
    ctx.add_singleton_model(|_| AIFactManager::new());
    ctx.add_singleton_model(|_| RecordingController::new());
    ctx.add_singleton_model(|_| ExecutionProfileEditorManager::default());
    ctx.add_singleton_model(|_| NetworkLogPaneManager::default());
    ctx.add_singleton_model(|_| pricing::PricingInfoModel::new());
    ctx.add_singleton_model(|ctx| {
        // Not using the *Provider types isn't ideal, but it's worth it for the ability to move managed secrets to a separate crate.
        ManagedSecretManager::new(
            server_api_provider.as_ref(ctx).get_managed_secrets_client(),
            auth_state.clone(),
        )
    });

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

    ctx.add_singleton_model(remote_server::manager::RemoteServerManager::new);
    #[cfg(not(target_family = "wasm"))]
    ctx.add_singleton_model(remote_server::codebase_index_model::RemoteCodebaseIndexModel::new);
    #[cfg(not(target_family = "wasm"))]
    remote_server::wire_auth_token_rotation(ctx);

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

    // Rewrite recognized Warp web URLs (sessions, Drive, settings, home) into local
    // intent URLs when possible so they open directly in the desktop app.
    ctx.set_before_open_url(|url_str, _ctx| {
        if let Ok(url) = Url::parse(url_str) {
            if let Some(intent) = maybe_rewrite_web_url_to_intent(&url) {
                return intent.to_string();
            }
        }
        url_str.to_owned()
    });

    ctx.set_a11y_verbosity(*AccessibilitySettings::as_ref(ctx).a11y_verbosity);

    #[cfg(enable_crash_recovery)]
    ctx.on_draw_frame_error(|ctx, window_id| {
        crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, _ctx| {
            crash_recovery.on_draw_frame_error(window_id);
        });
    });

    let user_is_logged_in = auth_state.is_logged_in();

    if user_is_logged_in {
        // Skip refresh_user for CLI mode — the CLI handles auth refresh in
        // ensure_auth_state so it can detect invalid credentials before running
        // a command.
        if !matches!(launch_mode, LaunchMode::CommandLine { .. }) {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.refresh_user(ctx);
            });
        }

        // Set the first frame callback to record the app's startup time.
        // This is only sent for logged-in users so that new users don't skew performance metrics.
        let is_screen_reader_enabled = ctx.is_screen_reader_enabled();
        let from_relaunch = launch_mode.args().finish_update;
        ctx.on_first_frame_drawn(move |ctx| {
            let timing_data = IntervalTimer::handle(ctx).update(ctx, |timer, _| {
                timer.mark_interval_end("FIRST_FRAME_DRAWN");
                timer.compute_stats()
            });
            let event = TelemetryEvent::AppStartup(AppStartupInfo {
                is_session_restoration_on: user_defaults_on_startup.should_restore_session,
                is_screen_reader_enabled,
                from_relaunch,
                is_crash_reporting_enabled,
                timing_data,
            });

            GPUState::handle(ctx).update(ctx, |gpu_state, ctx| {
                gpu_state
                    .set_has_lower_power_gpu(warpui::rendering::is_low_power_gpu_available(), ctx);
            });

            for window_id in ctx.window_ids().collect_vec() {
                SettingsPaneManager::handle(ctx)
                    .read(ctx, |model, _| model.settings_view(window_id))
                    .update(ctx, |settings, ctx| {
                        settings.refresh_preferred_graphics_backend_dropdown(ctx);
                    })
            }

            send_telemetry_from_app_ctx!(event, ctx);
        });

        #[cfg(enable_crash_recovery)]
        ctx.on_frame_drawn(|ctx, window_id| {
            crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, ctx| {
                crash_recovery.on_frame_drawn(window_id, ctx);
            });
        })
    } else {
        // If the app was opened while logged out, record an event for measuring new users.
        // This is sent immediately in case they quit the app on the signup screen.
        send_telemetry_sync_from_app_ctx!(TelemetryEvent::LoggedOutStartup, ctx);
        download_method::determine_and_report(
            auth_state.clone(),
            ctx.background_executor().clone(),
        );
    }

    #[cfg(not(target_family = "wasm"))]
    {
        ctx.add_singleton_model(DirectoryWatcher::new);
        // Register the skill provider directories as force-included paths so
        // the gitignore-pruning watch descend filter still watches gitignored
        // skill directories (e.g. `.agents/skills`) for `Repository`
        // subscribers (LSP, MCP). Registered before any repository begins
        // watching so it gates descent on the very first registration.
        DirectoryWatcher::handle(ctx).update(ctx, |watcher, _| {
            watcher.register_force_included_paths(
                ::ai::skills::SKILL_PROVIDER_DEFINITIONS
                    .iter()
                    .map(|provider| provider.skills_path.clone()),
            );
        });
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

        if ChannelState::channel() != warp_core::channel::Channel::Integration {
            imported_config_model.update(ctx, |model, ctx| {
                model.search_for_settings_to_import(ctx);
            });
        }

        let emit_incremental_updates = matches!(launch_mode, LaunchMode::RemoteServerDaemon { .. });
        ctx.add_singleton_model(|ctx| {
            let model = if emit_incremental_updates {
                RepoMetadataModel::new_with_incremental_updates(ctx)
            } else {
                RepoMetadataModel::new(ctx)
            };
            model.register_force_included_paths(
                ::ai::skills::SKILL_PROVIDER_DEFINITIONS
                    .iter()
                    .map(|provider| provider.skills_path.clone()),
                ctx,
            );
            model.set_project_skill_provider_paths(
                ::ai::skills::SKILL_PROVIDER_DEFINITIONS
                    .iter()
                    .map(|provider| provider.skills_path.clone()),
                ctx,
            );

            // Subscribe to RemoteServerManager push events so that remote repo
            // metadata snapshots and incremental updates populate the remote
            // sub-model and trigger RepoMetadataEvent emissions.
            {
                use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
                let mgr = RemoteServerManager::handle(ctx);
                ctx.subscribe_to_model(&mgr, |me, _, event, ctx| match event {
                    RemoteServerManagerEvent::RepoMetadataSnapshot { host_id, update } => {
                        me.insert_remote_snapshot(host_id.clone(), update, ctx);
                    }
                    RemoteServerManagerEvent::RepoMetadataUpdated { host_id, update }
                    | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { host_id, update } => {
                        me.apply_remote_incremental_update(host_id, update, ctx);
                    }
                    RemoteServerManagerEvent::HostDisconnected { host_id } => {
                        me.remove_remote_repositories_for_host(host_id, ctx);
                    }
                    _ => {}
                });
            }

            model
        });
    }

    ctx.add_singleton_model(|_| GitRepoModels::new());

    ctx.add_singleton_model(|ctx| {
        ProjectManagementModel::new(persisted_projects, persistence_writer.sender(), ctx)
    });

    ctx.add_singleton_model(move |_| History::new(command_history));

    ctx.add_singleton_model(CustomSecretRegexUpdater::new);

    // Register the `TelemetryCollection` singleton model.
    let server_api_clone = server_api.clone();
    ctx.add_singleton_model(|ctx| {
        let telemetry_collector = TelemetryCollector::new(server_api_clone);
        telemetry_collector.initialize_telemetry_collection(ctx);
        telemetry_collector
    });
    timer.mark_interval_end("INITIALIZE_TELEMETRY_COLLECTION");

    // Register initial keybindings prior to creating menus
    ai::init(ctx);
    app_services::init(ctx);
    // // TODO: Temporarily disabling keybindings for WASM builds. Will be implemented in future WASM support.
    #[cfg(not(target_family = "wasm"))]
    code::editor::find::view::init(ctx);
    workspace::init(ctx);
    pane_group::init(ctx);
    terminal::init(ctx);
    input::init(ctx);
    editor::init(ctx);
    onboarding::init(ctx);
    menu::init(ctx);
    tips::tip_view::init(ctx);
    launch_configs::init(ctx);
    workflows::init(ctx);
    themes::theme_chooser::init(ctx);
    themes::theme_creator_modal::init(ctx);
    themes::theme_deletion_modal::init(ctx);
    root_view::init(ctx);
    voltron::init(ctx);
    auth::init(ctx);
    reward_view::init(ctx);
    crate::view_components::find::init(ctx);
    prompt::editor_modal::init(ctx);
    ai::blocklist::agent_view::editor::init(ctx);
    undo_close::init(ctx);
    billing::shared_objects_creation_denied_modal::init(ctx);
    tab_configs::new_worktree_modal::init(ctx);
    tab_configs::params_modal::init(ctx);
    ai::blocklist::init(ctx);
    ai::blocklist::block::status_bar::init(ctx);
    drive::index::init(ctx);
    drive::sharing::dialog::init(ctx);
    ai_assistant::panel::init(ctx);
    settings_view::update_environment_form::init(ctx);
    env_vars::env_var_collection_block::init(ctx);
    context_chips::display_menu::init(ctx);
    context_chips::node_version_popup::init(ctx);
    env_vars::view::env_var_collection::init(ctx);
    ai::agent::todos::popup::init(ctx);
    terminal::view::init_environment::mode_selector::init(ctx);
    coding_entrypoints::project_buttons::init(ctx);
    if FeatureFlag::CodeReviewSaveChanges.is_enabled() {
        code_review::init(ctx);
    }

    let display_count = ctx.windows().display_count();
    ctx.add_singleton_model(|_| DisplayCount(display_count));

    ctx.add_singleton_model(|_| RelaunchModel::new());
    ctx.add_singleton_model(|_| ChangelogModel::new(server_api.clone()));
    ctx.add_singleton_model(|_| GitHubAuthNotifier::new());
    ctx.add_singleton_model(|_| NetworkStatus::new());
    ctx.add_singleton_model(|_| SystemStats::new());
    workspace::auto_handoff::init(ctx);
    ctx.add_singleton_model(|_| KeybindingChangedNotifier::new());
    ctx.add_singleton_model(|_| search::command_palette::SelectedItems::new());
    ctx.add_singleton_model(search::files::model::FileSearchModel::new);
    ctx.add_singleton_model(|_| VimRegisters::new());
    ctx.add_singleton_model(UndoCloseStack::new);
    ctx.add_singleton_model(|_| ToastStack);
    ctx.add_singleton_model(|_| GlobalCodeReviewModel);
    ctx.add_singleton_model(workspace::OneTimeModalModel::new);
    ctx.add_singleton_model(
        workspace::bonus_grant_notification_model::BonusGrantNotificationModel::new,
    );
    #[cfg(feature = "local_fs")]
    ctx.add_singleton_model(FileModel::new);
    ctx.add_singleton_model(GlobalBufferModel::new);
    #[cfg(windows)]
    ctx.add_singleton_model(util::traffic_lights::windows::RendererState::new);
    #[cfg(feature = "local_fs")]
    ctx.add_singleton_model(|_| LanguageServerShutdownManager::new());

    #[cfg(feature = "voice_input")]
    ctx.add_singleton_model(voice_input::VoiceInput::new);
    ctx.add_singleton_model(|_| {
        VoiceTranscriber::new(Arc::new(ServerVoiceTranscriber::new(server_api.clone())))
    });

    let notebooks = cloud_objects
        .iter()
        .filter_map(|object| {
            let notebook: Option<&CloudNotebook> = object.into();
            notebook
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut all_queue_items = Vec::new();
    let objects_with_pending_changes = cloud_objects
        .iter()
        .filter(|object| object.metadata().has_pending_content_changes())
        .cloned()
        .collect::<Vec<_>>();
    all_queue_items.extend(QueueItem::from_cached_objects(
        objects_with_pending_changes.into_iter(),
    ));

    let cloud_model = ctx.add_singleton_model(|_ctx| {
        CloudModel::new(
            persistence_writer.sender(),
            cloud_objects,
            time_of_next_force_object_refresh,
        )
    });

    let unsynced_actions: Vec<(CloudObjectTypeAndId, ObjectAction)> = object_actions
        .iter()
        .filter(|action| action.is_pending())
        .filter_map(|action| {
            cloud_model.read(ctx, |model, _| {
                let object = model.get_by_uid(&action.uid);
                object.map(|o| (o.cloud_object_type_and_id(), action.clone()))
            })
        })
        .collect::<Vec<_>>();

    all_queue_items.extend(QueueItem::from_unsynced_actions(
        unsynced_actions.into_iter(),
    ));

    ctx.add_singleton_model(|ctx| {
        SyncQueue::new(
            all_queue_items,
            server_api_provider.as_ref(ctx).get_cloud_objects_client(),
            ctx,
        )
    });

    // Seed the orchestration pin set from persisted conversation data
    // before the conversations vec is consumed by the singletons below.
    // Each conversation's `AgentConversationData.pinned` is the source of
    // truth; the singleton mirrors them in memory for fast cross-pane lookups.
    let initial_pinned_conversations: HashSet<AIConversationId> = multi_agent_conversations
        .iter()
        .filter_map(|conv| {
            let data =
                serde_json::from_str::<AgentConversationData>(&conv.conversation.conversation_data)
                    .ok()?;
            if !data.pinned {
                return None;
            }
            AIConversationId::try_from(conv.conversation.conversation_id.clone()).ok()
        })
        .collect();
    {
        let conversations = &multi_agent_conversations;
        ctx.add_singleton_model(move |_| {
            // Only wire NLD prompt history when the feature is enabled; disabled
            // (stable/preview) builds skip this so they don't retain the prompt snapshot.
            let nld_prompts = if FeatureFlag::NldPromptHistoryMatch.is_enabled() {
                nld_prompts
            } else {
                Vec::new()
            };
            BlocklistAIHistoryModel::new(ai_queries, nld_prompts, conversations)
        });
    }
    // Per-conversation queued prompts. Registered after the history model
    // since it subscribes to history events for cleanup.
    ctx.add_singleton_model(ai::blocklist::QueuedQueryModel::new);
    // Cross-pane UI state for the orchestration pill bar. Registered
    // after the history model since it subscribes to history events.
    ctx.add_singleton_model(move |ctx| {
        ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel::new(
            initial_pinned_conversations,
            ctx,
        )
    });
    // Conversations restore lazily from the local DB on demand; startup only
    // loads metadata.
    ctx.add_singleton_model(|_| RestoredAgentConversations::new());
    ctx.add_singleton_model(|_| CLIAgentSessionsModel::new());
    // ActiveAgentViewsModel is used to track active agent conversations and notify listeners when they change.
    ctx.add_singleton_model(|_| ActiveAgentViewsModel::new());
    ctx.add_singleton_model(AgentNotificationsModel::new);
    ctx.add_singleton_model(BlocklistAIPermissions::new);
    ctx.add_singleton_model(ai::blocklist::orchestration_events::OrchestrationEventService::new);
    ctx.add_singleton_model(
        ai::blocklist::local_agent_task_sync_model::LocalAgentTaskSyncModel::new,
    );
    ctx.add_singleton_model(
        ai::blocklist::orchestration_event_streamer::OrchestrationEventStreamer::new,
    );

    if launch_mode.supports_indexing() {
        ctx.add_singleton_model(RepoOutlines::new);
    } else {
        ctx.add_singleton_model(|ctx| RepoOutlines::new_with_indexing_enabled(false, ctx));
    }
    ctx.add_singleton_model(|ctx| {
        warp_core::sync_queue::SyncQueue::<SyncTask>::new_with_rate_limit(
            &ctx.background_executor(),
            Some(DEFAULT_SYNC_REQUESTS_PER_MIN),
        )
    });

    ctx.add_singleton_model(|_| UserProfiles::new(restored_user_profiles));

    ctx.add_singleton_model(|_| ObjectActions::new(object_actions));

    ctx.add_singleton_model(|_| AudibleBell::new());

    // This model has to be registered after the user workspaces model because it relies on it,
    // and before the UpdateManager models because they rely on the TeamTester model.
    ctx.add_singleton_model(TeamTesterStatus::new);

    ctx.add_singleton_model(|ctx| {
        TeamUpdateManager::new(
            server_api_provider.as_ref(ctx).get_team_client(),
            persistence_writer.sender(),
            ctx,
        )
    });

    ctx.add_singleton_model(|ctx| {
        UpdateManager::new(
            persistence_writer.sender(),
            server_api_provider.as_ref(ctx).get_cloud_objects_client(),
            ctx,
        )
    });

    let toml_file_path = settings::user_preferences_toml_file_path();
    ctx.add_singleton_model(move |ctx| {
        initialize_cloud_preferences_syncer(
            toml_file_path,
            startup_toml_parse_error_for_syncer.as_deref(),
            ctx,
        )
    });

    // LogManager must be registered before any subsystem (e.g. MCP, LSP) that creates file-based loggers.
    ctx.add_singleton_model(|_| simple_logger::manager::LogManager::new());

    let running_mcp_servers = app_state
        .as_ref()
        .map(|app_state| app_state.running_mcp_servers.as_slice())
        .unwrap_or(&[]);

    // FileMCPWatcher must be registered before FileBasedMCPManager, which subscribes to it.
    ctx.add_singleton_model(FileMCPWatcher::new);
    ctx.add_singleton_model(FileBasedMCPManager::new);

    // TemplatableMCPServerManager must be registered after UpdateManager and MCPServerManager so it can migrate legacy MCPs on start up
    // It should also be registered after FileBasedMCPManager so it can receive file-based server updates.
    ctx.add_singleton_model(|ctx| {
        TemplatableMCPServerManager::new(
            persisted_mcp_server_installations,
            mcp_servers_to_restore,
            running_mcp_servers,
            ctx,
        )
    });

    // MCPGalleryManager subscribes to UpdateManager so that it can be notified when gallery items are updated.
    // The registration of this singleton must be after UpdateManager is registered.
    ctx.add_singleton_model(MCPGalleryManager::new);

    // SkillManager is used to cache SKILL.md files for all active terminal views and their working directories
    ctx.add_singleton_model(SkillManager::new);

    // CloudViewModel subscribes to UpdateManager so that it can be notified when objects are
    // created on the server.
    ctx.add_singleton_model(CloudViewModel::new);

    // AIDocumentModel subscribes to UpdateManager so that it can be notified when notebooks are created on the server.
    ctx.add_singleton_model(AIDocumentModel::new);

    // AgentConversationsModel subscribes to UpdateManager for RTC task updates.
    ctx.add_singleton_model(AgentConversationsModel::new);

    // ByoLlmAuthBannerSessionState tracks dismissal of the BYO LLM auth banner (e.g., AWS Bedrock login).
    ctx.add_singleton_model(ByoLlmAuthBannerSessionState::new);

    ctx.add_singleton_model(ExportManager::new);
    ctx.add_singleton_model(|ctx| NotebookManager::new(notebooks, ctx));
    ctx.add_singleton_model(|_| CodeManager::default());
    ctx.add_singleton_model(|_| OpenedFilesModel::new());
    ctx.add_singleton_model(NotebookKeybindings::new);
    ctx.add_singleton_model(TerminalKeybindings::new);
    ctx.add_singleton_model(|_| ActiveSession::default());
    ctx.add_singleton_model(|ctx| {
        Listener::new(
            server_api_provider.as_ref(ctx).get_cloud_objects_client(),
            ctx,
        )
    });

    #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
    {
        ctx.add_singleton_model(LocalShellState::new);
        ctx.add_singleton_model(system::SystemInfo::new);
    }

    // `IapManager` drives gcloud-based IAP token refresh for staging builds.
    // Register it after `LocalShellState`: the Manager needs to know where the gcloud
    // cli lives & thus needs PATH config set by ~/.zshrc et al.
    //
    // Registered on all targets (including wasm) so consumers such as the
    // shared-session viewer network — which compiles and runs on wasm — can
    // read the singleton without panicking. On wasm `iap_state` is always
    // `None`, making this an inert no-op: `IapManager::new` early-returns from
    // its refresh loop and `iap_state()` yields no proxy-auth header.
    ctx.add_singleton_model(move |ctx| {
        let path_resolver = Box::new(|_ctx: &mut AppContext| {
            #[cfg(feature = "local_tty")]
            let path_future = LocalShellState::handle(_ctx).update(_ctx, |shell_state, ctx| {
                shell_state.get_interactive_path_env_var(ctx)
            });

            #[cfg(not(feature = "local_tty"))]
            let path_future =
                futures::FutureExt::boxed_local(futures::future::ready(None::<String>));

            path_future
        });
        IapManager::new(iap_state, path_resolver, ctx)
    });
    // Subscribe to IAP manager events to show toasts when refresh fails.
    ctx.subscribe_to_model(&IapManager::handle(ctx), |_, e, ctx| {
        match e {
            IapManagerEvent::RefreshFailed {
                message,
                is_first_failure_of_streak,
            } if *is_first_failure_of_streak => {
                let window_id = ctx
                    .windows()
                    .active_window()
                    .or_else(|| ctx.windows().ordered_window_ids().first().copied());
                let Some(window_id) = window_id else {
                    return;
                };
                let toast: DismissibleToast<WorkspaceAction> =
                    DismissibleToast::error(format!("IAP credential refresh failed: {message}"));
                ToastStack::handle(ctx).update(ctx, |stack, ctx| {
                    stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            _ => {}
        };
    });

    // Add a singleton model that holds the current prompt configuration.
    ctx.add_singleton_model(Prompt::new);

    // Add a singleton model for resizable modals whose size should be persisted through restarts.
    ctx.add_singleton_model(|_| ResizableData::default());

    // Add a singleton model to maintain state of shared session across all windows.
    ctx.add_singleton_model(terminal::shared_session::manager::Manager::new);

    ctx.add_singleton_model(
        terminal::shared_session::permissions_manager::SessionPermissionsManager::new,
    );

    ctx.add_singleton_model(EnvVarCollectionManager::new);
    ctx.add_singleton_model(WorkflowManager::new);

    if FeatureFlag::ScheduledAmbientAgents.is_enabled() {
        ctx.add_singleton_model(ScheduledAgentManager::new);
    }

    AutoupdateState::register(ctx, server_api.clone());

    ctx.add_singleton_model(LocalWorkflows::new);

    ctx.add_singleton_model(LLMPreferences::new);
    ctx.add_singleton_model(HarnessAvailabilityModel::new);
    ctx.add_singleton_model(ConnectedSelfHostedWorkersModel::new);

    let tip_model_handle = ctx.add_singleton_model(|ctx| {
        ai::agent_tips::AITipModel::<ai::AgentTip>::new_for_agent_tips(ctx)
    });
    {
        // Rebuild the tip pool when AI settings change so tips whose applicability
        // depends on AI settings appear/disappear without waiting for the next cooldown cycle.
        let tip_model_handle_for_ai = tip_model_handle.clone();
        ctx.subscribe_to_model(&AISettings::handle(ctx), move |_, _, ctx| {
            tip_model_handle_for_ai.update(ctx, |model, ctx| {
                model.revalidate_tips(ctx);
            });
        });
        // Also revalidate when workspace/team data changes (e.g. voice toggled at
        // the org level). Billing metadata — including `warp_ai_policy.is_voice_enabled`
        // — lives inside the team data, so `TeamsChanged` covers all policy updates.
        let tip_model_handle_for_teams = tip_model_handle.clone();
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), move |_, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                tip_model_handle_for_teams.update(ctx, |model, ctx| {
                    model.revalidate_tips(ctx);
                });
            }
        });
        // Revalidate when any keybinding changes so tips with `<keybinding>`
        // placeholders are hidden/shown when the referenced binding is cleared
        // or reassigned.
        ctx.subscribe_to_model(&KeybindingChangedNotifier::handle(ctx), move |_, _, ctx| {
            tip_model_handle.update(ctx, |model, ctx| {
                model.revalidate_tips(ctx);
            });
        });
    }

    timer.mark_interval_end("SINGLETON_MODELS_REGISTERED");

    ctx.add_singleton_model(move |_| timer);

    ctx.add_singleton_model(|ctx| AIExecutionProfilesModel::new(launch_mode, ctx));

    ctx.add_singleton_model(DefaultTerminal::new);

    ctx.add_singleton_model(|ctx| {
        let should_restore_indices = launch_mode.supports_indexing()
            && (matches!(launch_mode, LaunchMode::RemoteServerDaemon { .. })
                || UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx));
        let indices_to_restore = if should_restore_indices {
            persisted_workspaces.clone()
        } else {
            vec![]
        };

        let codebase_limits = AIRequestUsageModel::as_ref(ctx).codebase_context_limits();
        let mut codebase_index_config = CodebaseIndexManagerConfig::new(
            indices_to_restore,
            codebase_limits.max_indices_allowed,
            codebase_limits.max_files_per_repo,
            codebase_limits.embedding_generation_batch_size,
            server_api_provider.as_ref(ctx).get(),
            launch_mode.supports_indexing(),
        );
        if matches!(launch_mode, LaunchMode::RemoteServerDaemon { .. }) {
            codebase_index_config = codebase_index_config.defer_persisted_index_restore();
        }
        #[cfg(feature = "local_fs")]
        if let Some(snapshot_storage) = daemon_codebase_index_snapshot_storage(launch_mode) {
            return CodebaseIndexManager::new_with_snapshot_storage(
                codebase_index_config,
                Some(snapshot_storage),
                ctx,
            );
        }

        CodebaseIndexManager::new_with_config(codebase_index_config, ctx)
    });

    ctx.add_singleton_model(|ctx| {
        ProjectContextModel::new_from_persisted(
            persisted_project_rules,
            read_project_rule_contents,
            ctx,
        )
    });

    // Index global rules (e.g. ~/.agents/AGENTS.md) on a background task so
    // they are available to subsequent agent queries.
    ProjectContextModel::handle(ctx).update(ctx, |me, ctx| me.index_global_rules(ctx));
    #[cfg(all(not(target_family = "wasm"), feature = "local_fs"))]
    {
        ctx.add_singleton_model(ai::remote_agent_context::RemoteAgentContext::new);
    }

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

    // Subscribe WorkflowAliases to the UpdateManager so that it can be notified when objects are
    // trashed.
    WorkflowAliases::handle(ctx).update(ctx, |aliases, ctx| {
        aliases.connect(ctx);
    });

    // When running natively, add the http server singleton to the application.
    #[cfg(not(target_family = "wasm"))]
    ctx.add_singleton_model(move |ctx| {
        let routers = vec![
            app_installation_detection::make_router(),
            profiling::make_router(),
        ];
        http_server::HttpServer::new(routers, ctx)
    });
    #[cfg(feature = "local_fs")]
    if matches!(
        launch_mode,
        LaunchMode::App { .. } | LaunchMode::Test { .. }
    ) && FeatureFlag::WarpControlCli.is_enabled()
    {
        ctx.add_singleton_model(local_control::LocalControlBridge::new);
        ctx.add_singleton_model(local_control::LocalControlServer::new);
    }

    app_state
}

pub(crate) fn app_callbacks(
    is_integration_test: bool,
    mut tracing_initialization: Option<tracing::Initialization>,
) -> warpui::platform::AppCallbacks {
    warpui::platform::AppCallbacks {
        on_internet_reachability_changed: Some(Box::new(move |reachable, ctx| {
            NetworkStatus::handle(ctx)
                .update(ctx, move |me, ctx| me.reachability_changed(reachable, ctx));
        })),
        on_become_active: Some(Box::new(move |ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            ctx.record_app_focus(
                auth_state.user_id().map(|uid| uid.as_string()),
                auth_state.anonymous_id(),
            );
        })),
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

            let auth_state = AuthStateProvider::as_ref(ctx).get();
            ctx.record_app_blur(
                auth_state.user_id().map(|uid| uid.as_string()),
                auth_state.anonymous_id(),
            );
        })),
        on_will_terminate: Some(Box::new(move |ctx| {
            NotebookManager::handle(ctx).update(ctx, |manager, ctx| {
                // Notebooks are only saved periodically, so ensure that any pending changes have
                // been sent to the writer thread before terminating.
                manager.close_notebooks(ctx);
            });

            PersistenceWriter::handle(ctx).update(ctx, |writer, _ctx| {
                writer.terminate();
            });

            let auth_state = AuthStateProvider::as_ref(ctx).get();
            ctx.try_record_daily_app_focus_duration(
                auth_state.user_id().map(|uid| uid.as_string()),
                auth_state.anonymous_id(),
            );
            TelemetryCollector::handle(ctx).update(ctx, |telemetry_collector, ctx| {
                telemetry_collector.flush_telemetry_events_for_shutdown(ctx);
            });

            // Shutdown all LSP servers gracefully before app termination
            lsp::LspManagerModel::handle(ctx).update(ctx, |manager, ctx| {
                manager.terminate(ctx);
            });

            // We want to tear down the terminal server before relaunching for
            // autoupdate, to ensure we're not running any extra Warp processes
            // when we bring up the new process.  Additionally, this must occur
            // after terminating the persistence writer, so we don't keep track
            // of the fact that the shell sessions terminated.
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
            autoupdate::spawn_child_if_necessary(ctx);

            // Tear down any application profilers that are running, writing
            // results to disk.
            profiling::teardown();

            #[cfg(enable_crash_recovery)]
            crash_recovery::CrashRecovery::handle(ctx).update(ctx, |crash_recovery, _ctx| {
                crash_recovery.teardown();
            });
            if let Some(initialization) = tracing_initialization.as_mut() {
                initialization.shutdown();
            }

            // Tear down crash reporting as the last thing we do before the application
            // terminates.
            #[cfg(feature = "crash_reporting")]
            crash_reporting::uninit_sentry();
        })),
        on_should_close_window: Some(Box::new(move |window_id, ctx| {
            let general_settings = GeneralSettings::as_ref(ctx);
            // On Linux or Windows, if we're about to close the final window, we should quit the app instead.
            // On Mac, we do this conditionally based on a user setting.
            let quit_on_last_window_closed =
                cfg!(any(target_os = "linux", target_os = "freebsd", windows))
                    || *general_settings.quit_on_last_window_closed;
            if ctx.window_ids().count() == 1 && quit_on_last_window_closed {
                log::info!("No windows left, terminating app");
                ctx.terminate_app(TerminationMode::Cancellable, None);
                return ApproveTerminateResult::Cancel;
            }

            let summary = UnsavedStateSummary::for_window(window_id, ctx);

            send_telemetry_from_app_ctx!(
                TelemetryEvent::UserInitiatedClose {
                    initiated_on: CloseTarget::Window,
                },
                ctx
            );

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
        on_should_terminate_app: Some(Box::new(move |source, ctx| {
            // Never interrupt a system-initiated termination (logout / restart /
            // scheduled OS update): both cancel paths below return
            // `ApproveTerminateResult::Cancel`, which macOS interprets as Warp
            // refusing to quit. That can abort a scheduled OS update while the
            // quit-warning modal has no visible window to attach to, leaving
            // Warp waiting on a prompt nobody can see (#12441). Skipping
            // `apply_pending_update` here doesn't lose the update: the next
            // update check re-detects it (autoupdate state isn't persisted
            // across restarts, so the artifact may be re-downloaded).
            if source == TerminationRequestSource::System {
                return ApproveTerminateResult::Terminate;
            }

            send_telemetry_from_app_ctx!(
                TelemetryEvent::UserInitiatedClose {
                    initiated_on: CloseTarget::App,
                },
                ctx
            );

            // If there's a pending autoupdate, apply that before showing the unsaved changes
            // dialog. We apply the update first so that the dialog can force-terminate.
            let applying_update = autoupdate::apply_pending_update(ctx, |ctx| {
                // Once the deferred update is applied, re-terminate the app. This termination is
                // cancellable so that we still show the unsaved changes dialog.
                log::info!("Deferred autoupdate applied, terminating app");
                ctx.terminate_app(TerminationMode::Cancellable, None);
            });
            if applying_update {
                return ApproveTerminateResult::Cancel;
            }

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
            send_telemetry_from_app_ctx!(TelemetryEvent::QuitModalDisabled, ctx);
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

            if let Some(active_window_id) = active_window_id {
                OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                    model.update_target_window_id(active_window_id, ctx);
                });
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

/// Focuses the active window or if there isn't one then a window with a running process
/// and then shows the native modal.
fn focus_running_window_and_show_native_modal(
    sessions_summary: RunningSessionSummary,
    dialog_with_callbacks: AlertDialogWithCallbacks<AppModalCallback>,
    ctx: &mut AppContext,
) {
    let windowing_model = ctx.windows();
    let active_window_id = windowing_model.active_window();
    // Show the nav palette in the active window. If there is no active window,
    // arbitrarily pick one of the windows having a running process.
    let window_id_to_focus = active_window_id.unwrap_or_else(|| {
        *sessions_summary
            .windows_running()
            .iter()
            .next()
            .expect("already checked len > 0")
    });
    ctx.windows().show_window_and_focus_app(window_id_to_focus);
    if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id_to_focus) {
        if let Some(handle) = workspaces.first() {
            handle.update(ctx, |view, ctx| {
                view.show_native_modal(dialog_with_callbacks, ctx);
            });
        }
    }
}

fn on_close_app_cancelled(open_navigation_palette: bool, ctx: &mut AppContext) {
    autoupdate::cancel_relaunch(ctx);

    send_telemetry_from_app_ctx!(
        TelemetryEvent::QuitModalCancel {
            nav_palette: open_navigation_palette,
            modal_for: CloseTarget::App,
        },
        ctx
    );

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
    send_telemetry_from_app_ctx!(
        TelemetryEvent::QuitModalCancel {
            nav_palette: open_navigation_palette,
            modal_for: CloseTarget::Window,
        },
        ctx
    );

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

fn is_cloud_agent_web_home_launch_url(url: &Url) -> bool {
    url.scheme() == ChannelState::url_scheme()
        && url.host_str() == Some("action")
        && url.path() == "/new_cloud_agent_conversation"
        && url
            .query_pairs()
            .any(|(key, value)| key == "source" && value == "web_home")
}

#[::tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
fn launch(ctx: &mut warpui::AppContext, app_state: Option<AppState>, launch_mode: LaunchMode) {
    IntervalTimer::handle(ctx).update(ctx, |timer, _ctx| {
        timer.mark_interval_end("APP_LAUNCHED");
    });

    keyboard::load_custom_keybindings(ctx);

    IntervalTimer::handle(ctx).update(ctx, |timer, _ctx| {
        timer.mark_interval_end("KEYBINDINGS_LOADED");
    });

    // For now, we only specify application-level fallback fonts on web.
    #[cfg(target_family = "wasm")]
    ctx.set_fallback_font_fn(font_fallback::fallback_font_fn);

    match launch_mode {
        // The TUI front-end runs its own mount in the run closure and returns
        // before reaching launch().
        LaunchMode::Tui { .. } => unreachable!("LaunchMode::Tui is handled before launch()"),
        LaunchMode::App { .. } | LaunchMode::Test { .. } => {
            let should_skip_restore = launch_mode
                .args()
                .urls
                .iter()
                .any(is_cloud_agent_web_home_launch_url);
            let app_state = if should_skip_restore { None } else { app_state };
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
        #[cfg_attr(target_family = "wasm", allow(unused_variables))]
        LaunchMode::CommandLine {
            command,
            global_options,
            ..
        } => {
            cfg_if::cfg_if! {
                if #[cfg(target_family = "wasm")] {
                    panic!("Cannot execute CLI command {command:?} on the web");
                } else {
                    if let Err(err) = crate::ai::agent_sdk::run(ctx, command.clone(), global_options.clone()) {
                        eprintln!("{err:#}");
                        report_error!(err);
                        std::process::exit(1);
                    }
                }
            }
        }
        // Proxy should never reach launch() — it's a thin byte bridge.
        LaunchMode::RemoteServerProxy => {
            report_error!("Proxy mode should not use the launch() path");
            std::process::exit(1);
        }
        // Daemon: bind the Unix socket and register the ServerModel.
        // initialize_app already set up everything else including crash
        // reporting.
        #[cfg(unix)]
        LaunchMode::RemoteServerDaemon { identity_key } => {
            remote_server::unix::launch_daemon(&identity_key, ctx);
        }
        #[cfg(not(unix))]
        LaunchMode::RemoteServerDaemon { .. } => {
            report_error!("RemoteServerDaemon is not supported on this platform");
            std::process::exit(1);
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
