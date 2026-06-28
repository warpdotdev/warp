use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use anyhow::Result;
use cfg_if::cfg_if;
use itertools::Itertools;
use lazy_static::lazy_static;
use onboarding::{
    AgentOnboardingEvent, AgentOnboardingView, OnboardingIntention, SelectedSettings,
};
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::SessionId;
use settings::Setting as _;
use url::Url;
use warp_core::context_flag::ContextFlag;
use warp_core::safe_error;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Border, ChildAnchor, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Stack,
};
use warpui::keymap::{EditableBinding, FixedBinding};
use warpui::platform::{WindowBounds, WindowStyle};
use warpui::presenter::ChildView;
use warpui::rendering::OnGPUDeviceSelected;
use warpui::windowing::WindowManager;
use warpui::{
    id, AddWindowOptions, AppContext, DisplayId, Element, Entity, EntityId, FocusContext,
    NextNewWindowsHasThisWindowsBoundsUponClose, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::blocklist::SerializedBlockListItem;
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::ai::onboarding::{build_onboarding_models, current_onboarding_auth_state};
use crate::app_state::{AppState, PaneUuid, WindowSnapshot};
use crate::appearance::Appearance;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::auth_override_warning_modal::{
    AuthOverrideWarningModal, AuthOverrideWarningModalEvent, AuthOverrideWarningModalVariant,
};
use crate::auth::auth_state::AuthState;
use crate::auth::auth_view_modal::{AuthRedirectPayload, AuthView, AuthViewVariant};
use crate::auth::login_slide::{LoginSlideEvent, LoginSlideSource, LoginSlideView};
use crate::auth::needs_sso_link_view::NeedsSsoLinkView;
use crate::auth::paste_auth_token_modal::{PasteAuthTokenModalEvent, PasteAuthTokenModalView};
#[cfg(target_family = "wasm")]
use crate::auth::web_handoff::{WebHandoffEvent, WebHandoffView};
use crate::auth::{AuthStateProvider, LoginFailureReason};
use crate::autoupdate::{AutoupdateState, AutoupdateStateEvent, RequestType, UpdateReady};
use crate::changelog_model::ChangelogRequestType;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{GenericStringObjectFormat, JsonObjectType, ObjectType};
use crate::drive::export::ExportManager;
use crate::drive::items::WarpDriveItemId;
use crate::drive::{CloudObjectTypeAndId, OpenWarpDriveObjectArgs, OpenWarpDriveObjectSettings};
use crate::experiments::{BlockOnboarding, Experiment};
use crate::features::FeatureFlag;
use crate::interval_timer::IntervalTimer;
use crate::launch_configs::launch_config;
use crate::linear::LinearIssueWork;
use crate::notebooks::manager::NotebookSource;
use crate::pane_group::{NewTerminalOptions, PanesLayout};
use crate::persistence::ModelEvent;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;
use crate::server::server_api::auth::UserAuthenticationError;
use crate::server::server_api::{ServerApi, ServerApiProvider, ServerTime};
use crate::server::telemetry::{LaunchConfigUiLocation, TelemetryEvent};
use crate::settings::cloud_preferences_syncer::{
    CloudPreferencesSyncer, CloudPreferencesSyncerEvent,
};
use crate::settings::{apply_onboarding_settings, AISettings, QuakeModeSettings, ThemeSettings};
use crate::settings_view::mcp_servers_page::MCPServersSettingsPage;
use crate::settings_view::{flags, OpenTeamsSettingsModalArgs, SettingsSection};
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::keys_settings::KeysSettings;
use crate::terminal::shell::ShellType;
use crate::terminal::view::{cell_size_and_padding, TerminalAction};
use crate::themes::onboarding_theme_picker_themes;
use crate::themes::theme::{AnsiColorIdentifier, Blend, Fill, ThemeKind, WarpThemeConfig};
use crate::uri::OpenMCPSettingsArgs;
use crate::util::bindings::{self, is_binding_pty_compliant};
use crate::util::traffic_lights::{traffic_light_data, TrafficLightData, TrafficLightMouseStates};
use crate::view_components::DismissibleToast;
use crate::window_settings::WindowSettings;
use crate::workspace::hoa_onboarding::mark_hoa_onboarding_completed;
use crate::workspace::tab_settings::TabSettings;
use crate::workspace::view::OnboardingTutorial;
use crate::workspace::{PaneViewLocator, Workspace, WorkspaceAction, WorkspaceRegistry};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::{
    report_if_error, send_telemetry_from_app_ctx, send_telemetry_from_ctx, ChannelState,
    GlobalResourceHandles, GlobalResourceHandlesProvider, UpdateQuakeModeEventArg,
};

const WINDOW_TITLE: &str = "Warp";

lazy_static! {
    static ref FALLBACK_WINDOW_SIZE: Vector2F = vec2f(800.0, 600.0);
    static ref QUAKE_STATE: Arc<Mutex<Option<QuakeModeState>>> = Arc::new(Mutex::new(None));
}

/// This is the color of the border wrapping the whole window.
///
/// On MacOS, this is drawn for us by the OS. On other platforms, we must draw it ourselves. Note
/// that this is hard-coded for the default Dark theme. This is because it is only used by the
/// AuthView and OnboardingSurveyModal which do not respect the chosen theme. So, do not use this for Views
/// which respect themes.
pub(crate) fn unthemed_window_border() -> Border {
    if cfg!(all(not(target_os = "macos"), not(target_family = "wasm"))) {
        // The 15% blend of fg into bg is the "ui surface" color.
        Border::all(1.).with_border_fill(Fill::black().blend(&Fill::white().with_opacity(15)))
    } else {
        Border::all(1.).with_border_fill(Fill::black().with_opacity(0))
    }
}

#[derive(Debug, Clone)]
enum WindowState {
    /// Quake mode window is open and visible on the screen.
    Open,
    /// Quake mode window is opening but has not become the key window yet.
    /// This happens when the app is not focused when the quake mode window
    /// is opened.
    PendingOpen,
    /// Quake mode window is open but hidden away from the screen.
    /// In this state, toggling quake mode will show the hidden window rather
    /// than creating a new one.
    Hidden,
}

#[derive(Debug, Clone)]
pub struct QuakeModeState {
    /// State of the opened quake mode window.
    window_state: WindowState,
    window_id: WindowId,
    /// ID of the active screen when we last positioned the quake mode window.
    /// Note that this is not necessarily the screen quake mode lives in if user
    /// set a specific pinned screen.
    active_display_id: DisplayId,
}

/// Configuration for the new quake mode window including the active screen id and the window bound.
struct QuakeModeFrameConfig {
    display_id: DisplayId,
    window_bounds: RectF,
}

/// Trigger of a potential quake window move.
#[derive(Debug)]
enum QuakeModeMoveTrigger {
    /// The screen configuration changed (plug / unplug monitor). We need
    /// to reposition quake mode as it might be in an invalid position.
    ScreenConfigurationChange,
    /// User set "active screen" as the screen to pin to. In this case,
    /// we will attempt to move the quake window if the active screen dimension
    /// changed. If it hasn't change, we will keep the window as is to avoid
    /// meaningless resizing.
    ActiveScreenSetting,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Hash,
    Eq,
    PartialEq,
    Deserialize,
    Serialize,
    Default,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Screen edge to pin the hotkey window to.",
    rename_all = "snake_case"
)]
pub enum QuakeModePinPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

pub struct OpenFromRestoredArg {
    pub app_state: Option<AppState>,
}

pub struct OpenLaunchConfigArg {
    pub launch_config: launch_config::LaunchConfig,
    pub ui_location: LaunchConfigUiLocation,

    /// Tries to open the launch config into the active window, if any.
    ///
    /// Currently, this is only supported by single-window launch configs
    /// and will open the window tabs into the existing window when true.
    pub open_in_active_window: bool,
}

pub struct OpenPath {
    pub path: PathBuf,
}

// Arguments for actions that run a command that should start a subshell.
pub struct SubshellCommandArg {
    pub command: String,
    pub shell_type: Option<ShellType>,
}

// Arguments for creating an ambient agent environment.
pub struct CreateEnvironmentArg {
    pub repos: Vec<String>,
}

impl CreateEnvironmentArg {
    /// Formats the `/create-environment` slash command invocation.
    pub fn to_query(&self) -> String {
        // Filter repos to accept either valid URLs or POSIX portable pathnames for security.
        //
        // Note: we also allow *absolute* POSIX paths (e.g., /Users/me/repo) as long as every
        // component is portable. This is important for local indexed repos.
        let safe_repos = self
            .repos
            .iter()
            .filter(|repo| {
                // Accept valid URLs (e.g., https://github.com/user/repo)
                Url::parse(repo).is_ok()
                    // Or valid POSIX portable pathnames (e.g., user/repo)
                    || warp_util::path::is_posix_portable_pathname(repo)
                    // Or absolute POSIX paths with portable components (e.g., /Users/me/repo)
                    || repo
                        .strip_prefix('/')
                        .is_some_and(warp_util::path::is_posix_portable_pathname)
            })
            .join(" ");

        if safe_repos.is_empty() {
            // Include a trailing space to trigger slash command syntax highlighting and ghost text.
            "/create-environment ".to_string()
        } else {
            format!("/create-environment {}", safe_repos)
        }
    }
}

pub fn init(app: &mut AppContext) {
    app.register_binding_validator::<RootView>(is_binding_pty_compliant);

    app.add_global_action("root_view:open_from_restored", open_from_restored);
    app.add_global_action("root_view:open_new", open_new);
    app.add_global_action("root_view:open_new_with_shell", open_new_with_shell);
    app.add_global_action("root_view:open_new_from_path", |arg, ctx| {
        let _ = open_new_from_path(arg, ctx);
    });
    app.add_global_action(
        "root_view:open_new_tab_insert_subshell_command_and_bootstrap_if_supported",
        open_new_tab_insert_subshell_command_and_bootstrap_if_supported,
    );
    app.add_global_action("root_view:open_launch_config", open_launch_config);
    app.add_global_action("root_view:send_feedback", send_feedback);
    app.add_global_action(
        "root_view:toggle_quake_mode_window",
        toggle_quake_mode_window,
    );
    app.add_global_action(
        "root_view:show_or_hide_non_quake_mode_windows",
        show_or_hide_non_quake_mode_windows,
    );
    app.add_global_action("root_view:update_quake_mode_state", update_quake_mode_state);
    app.add_global_action(
        "root_view:move_quake_mode_window_from_screen_change",
        move_quake_mode_window_from_screen_change,
    );
    #[cfg(feature = "voice_input")]
    app.add_global_action("root_view:abort_voice_input", abort_voice_input);
    #[cfg(feature = "voice_input")]
    app.add_action(
        "root_view:maybe_stop_active_voice_input",
        RootView::maybe_stop_active_voice_input,
    );
    app.add_action("root_view:log_out", RootView::log_out);
    app.add_action(
        "root_view:handle_incoming_auth_url",
        RootView::handle_incoming_auth_url,
    );
    app.add_action(
        "root_view:add_session_at_path",
        RootView::add_session_at_path,
    );
    app.add_action(
        "root_view:handle_team_intent_link_action",
        RootView::handle_team_intent_link_action,
    );
    app.add_action(
        "root_view:open_team_settings_page",
        RootView::open_team_settings_page,
    );
    app.add_action(
        "root_view:handle_notification_click",
        RootView::handle_notification_click,
    );
    app.add_action(
        "root_view:handle_pane_navigation_event",
        RootView::focus_pane,
    );
    app.add_action(
        "root_view:activate_tab_by_pane_group_id",
        RootView::activate_tab_by_pane_group_id,
    );
    app.add_action("root_view:close_window", RootView::close_window);
    app.add_action("root_view:minimize_window", RootView::minimize_window);
    app.add_action(
        "root_view:toggle_maximize_window",
        RootView::toggle_maximize_window,
    );
    app.add_action("root_view:toggle_fullscreen", RootView::toggle_fullscreen);

    if FeatureFlag::ViewingSharedSessions.is_enabled() {
        app.add_global_action(
            "root_view:join_shared_session",
            open_shared_session_as_viewer,
        );
        app.add_action(
            "root_view:join_shared_session_in_existing_window",
            RootView::join_shared_session_in_existing_window,
        );
    }

    app.add_global_action(
        "root_view:open_conversation_viewer",
        open_conversation_viewer,
    );
    app.add_action(
        "root_view:open_cloud_conversation_in_existing_window",
        RootView::open_cloud_conversation_in_existing_window,
    );

    app.add_global_action("root_view:create_environment", create_environment);
    app.add_global_action(
        "root_view:create_environment_and_run",
        create_environment_and_run,
    );
    app.add_action(
        "root_view:create_environment_in_existing_window",
        RootView::create_environment_in_existing_window,
    );
    app.add_action(
        "root_view:create_environment_in_existing_window_and_run",
        RootView::create_environment_in_existing_window_and_run,
    );
    app.add_global_action(
        "root_view:open_drive_object_new_window",
        open_warp_drive_object,
    );
    app.add_action(
        "root_view:open_drive_object_existing_window",
        RootView::open_warp_drive_object_in_existing_window,
    );

    app.add_global_action(
        "root_view:open_team_settings_with_email_invite_in_new_window",
        open_team_settings_with_email_invite_in_new_window,
    );
    app.add_action(
        "root_view:open_team_settings_with_email_invite_in_existing_window",
        RootView::open_team_settings_with_email_invite_in_existing_window,
    );

    app.add_global_action(
        "root_view:open_settings_page_in_new_window",
        open_settings_page_in_new_window,
    );
    app.add_action(
        "root_view:open_settings_page_in_existing_window",
        RootView::open_settings_page_in_existing_window,
    );

    app.add_global_action(
        "root_view:open_mcp_settings_in_new_window",
        open_mcp_settings_in_new_window,
    );
    app.add_action(
        "root_view:open_mcp_settings_in_existing_window",
        RootView::open_mcp_settings_in_existing_window,
    );

    app.add_global_action(
        "root_view:open_codex_in_new_window",
        open_codex_in_new_window,
    );
    app.add_action(
        "root_view:open_codex_in_existing_window",
        RootView::open_codex_in_existing_window,
    );

    app.add_global_action(
        "root_view:open_linear_issue_work_in_new_window",
        open_linear_issue_work_in_new_window,
    );
    app.add_action(
        "root_view:open_linear_issue_work_in_existing_window",
        RootView::open_linear_issue_work_in_existing_window,
    );

    app.add_action("root_view:add_file_pane", RootView::add_file_pane);
    app.add_global_action(
        "root_view:open_new_with_file_notebook",
        open_new_with_file_notebook,
    );

    app.register_fixed_bindings([
        FixedBinding::empty(
            "Hide All Windows",
            RootViewAction::ShowOrHideNonQuakeModeWindows,
            id!("RootView") & id!(flags::ACTIVATION_HOTKEY_FLAG),
        ),
        FixedBinding::empty(
            "Show Dedicated Hotkey Window",
            RootViewAction::ToggleQuakeModeWindow,
            id!("RootView")
                & id!(flags::QUAKE_MODE_ENABLED_CONTEXT_FLAG)
                & !id!(flags::QUAKE_WINDOW_OPEN_FLAG),
        ),
        FixedBinding::empty(
            "Hide Dedicated Hotkey Window",
            RootViewAction::ToggleQuakeModeWindow,
            id!("RootView")
                & id!(flags::QUAKE_MODE_ENABLED_CONTEXT_FLAG)
                & id!(flags::QUAKE_WINDOW_OPEN_FLAG),
        ),
    ]);

    app.register_editable_bindings([
        // Register a binding to toggle fullscreen on Linux and Windows.
        EditableBinding::new(
            "root_view:toggle_fullscreen",
            "Toggle fullscreen",
            RootViewAction::ToggleFullscreen,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("RootView"))
        .with_linux_or_windows_key_binding("f11"),
        // Debug binding for onboarding state
        EditableBinding::new(
            "root_view:enter_onboarding_state",
            "[Debug] Enter Onboarding State",
            RootViewAction::DebugEnterOnboardingState,
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("RootView"))
        .with_key_binding("shift-f12")
        .with_enabled(|| {
            FeatureFlag::AgentOnboarding.is_enabled() && ChannelState::enable_debug_features()
        }),
    ])
}

fn maybe_register_global_window_shortcuts(
    global_resource_handles: GlobalResourceHandles,
    ctx: &mut AppContext,
) {
    // let keys_settings = KeysSettings::handle(ctx).as_ref(ctx);
    if let Some(key) = KeysSettings::as_ref(ctx)
        .quake_mode_settings
        .keybinding
        .clone()
        .filter(|_| *KeysSettings::as_ref(ctx).quake_mode_enabled)
    {
        ctx.register_global_shortcut(
            key.clone(),
            "root_view:toggle_quake_mode_window",
            global_resource_handles,
        );
    }

    if let Some(key) = KeysSettings::as_ref(ctx)
        .activation_hotkey_keybinding
        .clone()
        .filter(|_| *KeysSettings::as_ref(ctx).activation_hotkey_enabled)
    {
        ctx.register_global_shortcut(
            key.clone(),
            "root_view:show_or_hide_non_quake_mode_windows",
            (),
        )
    }
}

/// Find the root [`Workspace`] view for the active window.
fn active_workspace(ctx: &mut AppContext) -> Option<ViewHandle<Workspace>> {
    let window_id = ctx.windows().active_window()?;
    WorkspaceRegistry::as_ref(ctx).get(window_id, ctx)
}

fn open_launch_config(arg: &OpenLaunchConfigArg, ctx: &mut AppContext) {
    let active_window_workspace = active_workspace(ctx);
    if arg.launch_config.windows.is_empty() {
        open_new(&(), ctx);
    } else if arg.open_in_active_window
        && arg.launch_config.windows.len() == 1
        && active_window_workspace.is_some()
    {
        active_window_workspace
            .expect("already checked if there is a workspace for the active window")
            .update(ctx, |workspace, ctx| {
                workspace.open_launch_config_window(arg.launch_config.windows[0].clone(), ctx)
            });
    } else {
        let mut active_index = None;
        for (idx, window_template) in arg.launch_config.windows.iter().enumerate() {
            if arg
                .launch_config
                .active_window_index
                .map(|window_idx| window_idx == idx)
                .unwrap_or(false)
            {
                active_index = Some(idx);
            } else {
                open_new_with_workspace_source(
                    NewWorkspaceSource::FromTemplate {
                        window_template: window_template.clone(),
                    },
                    ctx,
                );
            }
        }

        if let Some(idx) = active_index {
            let window_template = arg
                .launch_config
                .windows
                .get(idx)
                .expect("Window should exist at idx");

            open_new_with_workspace_source(
                NewWorkspaceSource::FromTemplate {
                    window_template: window_template.clone(),
                },
                ctx,
            );
        }
    }

    send_telemetry_from_app_ctx!(
        TelemetryEvent::OpenLaunchConfig {
            ui_location: crate::server::telemetry::LaunchConfigUiLocation::Uri,
            open_in_active_window: arg.open_in_active_window,
        },
        ctx
    );
}

fn send_feedback(_: &(), ctx: &mut AppContext) {
    if let Some(workspace) = active_workspace(ctx) {
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::SendFeedback, ctx);
        });
    } else {
        ctx.open_url(&crate::util::links::feedback_form_url());
    }
}

/// Creates a new window with the transferred pane group.
///
/// If `is_tab_drag_preview` is true, the window is created without stealing
/// focus so it can follow the cursor during a tab drag.
///
/// Returns the new window ID.
pub fn create_transferred_window(
    transferred_tab: crate::workspace::view::TransferredTab,
    source_window_id: WindowId,
    window_size: Vector2F,
    window_position: Vector2F,
    is_tab_drag_preview: bool,
    ctx: &mut AppContext,
) -> WindowId {
    let global_resource_handles = GlobalResourceHandlesProvider::handle(ctx)
        .as_ref(ctx)
        .get()
        .clone();
    let window_settings = WindowSettings::handle(ctx).as_ref(ctx);

    let window_bounds = WindowBounds::ExactPosition(RectF::new(window_position, window_size));

    let window_style = if is_tab_drag_preview {
        WindowStyle::PositionedNoFocus
    } else {
        WindowStyle::Normal
    };

    let (new_window_id, _) = ctx.add_window(
        AddWindowOptions {
            window_style,
            window_bounds,
            title: Some(WINDOW_TITLE.to_owned()),
            background_blur_radius_pixels: Some(*window_settings.background_blur_radius),
            background_blur_texture: *window_settings.background_blur_texture,
            on_gpu_driver_selected: on_gpu_driver_selected_callback(),
            ..Default::default()
        },
        |ctx| {
            let mut view = RootView::new(
                global_resource_handles.clone(),
                NewWorkspaceSource::TransferredTab {
                    tab_color: transferred_tab.color,
                    custom_title: transferred_tab.custom_title.clone(),
                    left_panel_open: transferred_tab.left_panel_open,
                    vertical_tabs_panel_open: transferred_tab.vertical_tabs_panel_open,
                    right_panel_open: transferred_tab.right_panel_open,
                    is_right_panel_maximized: transferred_tab.is_right_panel_maximized,
                    is_tab_drag_preview,
                },
                ctx,
            );
            if !is_tab_drag_preview {
                view.focus(ctx);
            }
            view
        },
    );

    let pane_group_id = transferred_tab.pane_group.id();
    ctx.transfer_view_tree_to_window(pane_group_id, source_window_id, new_window_id);

    if let Some(new_workspace) = WorkspaceRegistry::as_ref(ctx).get(new_window_id, ctx) {
        new_workspace.update(ctx, |workspace, ctx| {
            workspace.adopt_transferred_pane_group(transferred_tab.pane_group.clone(), ctx);
        });
    } else {
        log::warn!("Failed to find workspace in newly created window {new_window_id:?}");
    }
    new_window_id
}

#[cfg(feature = "crash_reporting")]
fn on_gpu_driver_selected_callback() -> Option<Box<OnGPUDeviceSelected>> {
    Some(Box::new(|gpu_device_info| {
        crate::crash_reporting::set_gpu_device_info(gpu_device_info)
    }))
}

#[cfg(not(feature = "crash_reporting"))]
fn on_gpu_driver_selected_callback() -> Option<Box<OnGPUDeviceSelected>> {
    None
}

fn open_from_restored(arg: &OpenFromRestoredArg, ctx: &mut AppContext) {
    let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
    IntervalTimer::handle(ctx).update(ctx, |timer, _| {
        timer.mark_interval_end("HANDLING_OPEN_ACTION");
    });

    if let Some(app_state) = &arg.app_state {
        maybe_register_global_window_shortcuts(global_resource_handles.clone(), ctx);

        let (background_blur_radius_pixels, background_blur_texture) = {
            let window_settings = WindowSettings::as_ref(ctx);
            (
                Some(*window_settings.background_blur_radius),
                *window_settings.background_blur_texture,
            )
        };

        // Check whether user has enabled session restoration.
        if *GeneralSettings::as_ref(ctx).restore_session {
            let mut active_index = None;
            let mut normal_window_count = 0;
            for (idx, window) in app_state.windows.iter().enumerate() {
                // If this window is a quake window, hide it by default.
                if window.quake_mode {
                    // If this is Windows, skip restoring the quake window. Creating a hidden window
                    // is not supported on Windows. We can't have the quake window visible on
                    // startup or else it will get mistaken for a normal window.
                    if cfg!(windows) {
                        continue;
                    }
                    let frame_args = quake_mode_config(
                        &KeysSettings::as_ref(ctx)
                            .quake_mode_settings
                            .value()
                            .clone(),
                        ctx,
                    );

                    let (id, _) = ctx.add_window(
                        AddWindowOptions {
                            window_style: WindowStyle::Pin,
                            window_bounds: WindowBounds::ExactPosition(frame_args.window_bounds),
                            title: Some("Warp".to_owned()),
                            fullscreen_state: window.fullscreen_state,
                            background_blur_radius_pixels,
                            background_blur_texture,
                            // Don't use the quake window for positioning new windows.
                            anchor_new_windows_from_closed_position:
                                NextNewWindowsHasThisWindowsBoundsUponClose::No,
                            on_gpu_driver_selected: on_gpu_driver_selected_callback(),
                            window_instance: Some(ChannelState::app_id().to_string() + "-hotkey"),
                        },
                        |ctx| {
                            let mut view = RootView::new(
                                global_resource_handles.clone(),
                                NewWorkspaceSource::Restored {
                                    window_snapshot: window.clone(),
                                    block_lists: app_state.block_lists.clone(),
                                },
                                ctx,
                            );
                            view.focus(ctx);
                            view
                        },
                    );
                    ctx.windows().hide_window(id);

                    let mut quake_mode_state = QUAKE_STATE.lock();
                    *quake_mode_state = Some(QuakeModeState {
                        window_state: WindowState::Hidden,
                        window_id: id,
                        active_display_id: frame_args.display_id,
                    });
                } else {
                    normal_window_count += 1;
                    if app_state
                        .active_window_index
                        .map(|window_idx| window_idx == idx)
                        .unwrap_or(false)
                    {
                        active_index = Some(idx);
                    } else {
                        ctx.add_window(
                            AddWindowOptions {
                                window_bounds: WindowBounds::new(window.bounds),
                                title: Some("Warp".to_owned()),
                                fullscreen_state: window.fullscreen_state,
                                background_blur_radius_pixels,
                                background_blur_texture,
                                on_gpu_driver_selected: on_gpu_driver_selected_callback(),
                                ..Default::default()
                            },
                            |ctx| {
                                let mut view = RootView::new(
                                    global_resource_handles.clone(),
                                    NewWorkspaceSource::Restored {
                                        window_snapshot: window.clone(),
                                        block_lists: app_state.block_lists.clone(),
                                    },
                                    ctx,
                                );
                                view.focus(ctx);
                                view
                            },
                        );
                    }
                }
            }

            // If only the quake mode window was restored (which starts hidden), create a new normal
            // window so that something visible is created on startup.
            if normal_window_count == 0 {
                let window_settings = WindowSettings::as_ref(ctx);
                let options = default_window_options(window_settings, ctx);
                ctx.add_window(options, |ctx| {
                    let mut view = RootView::new(
                        global_resource_handles.clone(),
                        NewWorkspaceSource::Empty {
                            previous_active_window: None,
                            shell: None,
                        },
                        ctx,
                    );
                    view.focus(ctx);
                    view
                });
            }

            // Create the active window last to make sure it is focused on startup.
            if let Some(idx) = active_index {
                let window = app_state
                    .windows
                    .get(idx)
                    .expect("Window should exist at idx");
                ctx.add_window(
                    AddWindowOptions {
                        window_bounds: WindowBounds::new(window.bounds),
                        title: Some("Warp".to_owned()),
                        fullscreen_state: window.fullscreen_state,
                        background_blur_radius_pixels,
                        background_blur_texture,
                        on_gpu_driver_selected: on_gpu_driver_selected_callback(),
                        ..Default::default()
                    },
                    |ctx| {
                        let mut view = RootView::new(
                            global_resource_handles,
                            NewWorkspaceSource::Restored {
                                window_snapshot: window.clone(),
                                block_lists: app_state.block_lists.clone(),
                            },
                            ctx,
                        );
                        view.focus(ctx);
                        view
                    },
                );
            }
        }
    }
}

fn path_if_directory(path: &Path) -> Option<&Path> {
    path.is_dir().then_some(path)
}

/// Opens a new window with the workspace configured according to `source`. Returns the
/// newly-opened window ID and a handle to the root view in that window.
///
/// This is the canonical way to open a new Warp window - all other entrypoints should delegate to
/// it if possible.
pub(crate) fn open_new_with_workspace_source(
    source: NewWorkspaceSource,
    ctx: &mut AppContext,
) -> (WindowId, ViewHandle<RootView>) {
    let global_resource_handles = GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
    let window_settings = WindowSettings::as_ref(ctx);
    let options = default_window_options(window_settings, ctx);
    ctx.add_window(options, |ctx| {
        let mut view = RootView::new(global_resource_handles, source, ctx);
        view.focus(ctx);
        view
    })
}

pub(crate) fn open_new_from_path(
    arg: &OpenPath,
    ctx: &mut AppContext,
) -> (WindowId, ViewHandle<RootView>) {
    open_new_with_workspace_source(
        NewWorkspaceSource::Session {
            options: Box::new(
                NewTerminalOptions::default()
                    .with_initial_directory_opt(path_if_directory(&arg.path).map(Into::into)),
            ),
        },
        ctx,
    )
}

/// Opens a new window and tries to join session identified by the session ID.
fn open_shared_session_as_viewer(session_id: &SessionId, ctx: &mut AppContext) {
    open_new_with_workspace_source(
        NewWorkspaceSource::SharedSessionAsViewer {
            session_id: *session_id,
        },
        ctx,
    );
}

/// Opens a new window to view a persisted view-only cloud conversation.
/// The conversation data is loaded via GraphQL API.
fn open_conversation_viewer(conversation_id: &ServerConversationToken, ctx: &mut AppContext) {
    // Trigger the workspace loading mechanism by dispatching the LoadConversationData event
    // This will open a new window with a loading state, fetch data via GraphQL, and display it
    open_new_with_workspace_source(
        NewWorkspaceSource::FromCloudConversationId {
            conversation_id: conversation_id.clone(),
        },
        ctx,
    );
}

/// Opens a new window and starts the guided `/create-environment` setup flow.
fn create_environment(arg: &CreateEnvironmentArg, ctx: &mut AppContext) {
    let repos = arg.repos.clone();
    let (window_id, root_handle) = open_new_with_workspace_source(
        NewWorkspaceSource::Session {
            options: Box::default(),
        },
        ctx,
    );

    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_handle) = &root_view.auth_onboarding_state {
            workspace_handle.update(ctx, |workspace, ctx| {
                workspace
                    .active_tab_pane_group()
                    .update(ctx, |pane_group, ctx| {
                        pane_group.set_title("Create Environment", ctx);

                        if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                            terminal_view.update(ctx, |_, ctx| {
                                ctx.dispatch_typed_action_deferred(
                                    TerminalAction::SetupCloudEnvironment(repos.clone()),
                                );
                            });
                        }
                    });
            });
        }
    });

    ctx.windows().show_window_and_focus_app(window_id);
}

/// Opens a new window and starts the guided `/create-environment` setup flow immediately.
fn create_environment_and_run(arg: &CreateEnvironmentArg, ctx: &mut AppContext) {
    let repos = arg.repos.clone();
    let (window_id, root_handle) = open_new_with_workspace_source(
        NewWorkspaceSource::Session {
            options: Box::default(),
        },
        ctx,
    );

    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_handle) = &root_view.auth_onboarding_state {
            workspace_handle.update(ctx, |workspace, ctx| {
                workspace
                    .active_tab_pane_group()
                    .update(ctx, |pane_group, ctx| {
                        pane_group.set_title("Create Environment", ctx);

                        if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                            terminal_view.update(ctx, |_, ctx| {
                                ctx.dispatch_typed_action_deferred(
                                    TerminalAction::SetupCloudEnvironmentAndStart(repos.clone()),
                                );
                            });
                        }
                    });
            });
        }
    });

    ctx.windows().show_window_and_focus_app(window_id);
}
fn open_team_settings_with_email_invite_in_new_window(
    arg: &OpenTeamsSettingsModalArgs,
    ctx: &mut AppContext,
) {
    let root_handle = open_new_window_get_handles(None, ctx).1;
    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_view_handle) =
            &root_view.auth_onboarding_state
        {
            let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
            let email_invite = arg.invite_email.clone();
            workspace_view_handle.update(ctx, |_, ctx| {
                let _ = ctx.spawn(initial_load_complete, move |workspace, _, ctx| {
                    workspace.show_team_settings_page_with_email_invite(email_invite.as_ref(), ctx)
                });
            });
        }
    });
}

fn open_settings_page_in_new_window(section: &SettingsSection, ctx: &mut AppContext) {
    let root_handle = open_new_window_get_handles(None, ctx).1;
    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_view_handle) =
            &root_view.auth_onboarding_state
        {
            let window_id = ctx.window_id();
            ctx.dispatch_typed_action_for_view(
                window_id,
                workspace_view_handle.id(),
                &WorkspaceAction::ShowSettingsPage(*section),
            );
        }
    });
}

/// MCP servers need to wait for initial load to complete, so we have this action in addition
/// to the general-purpose [`open_settings_page_in_new_window`].
fn open_mcp_settings_in_new_window(args: &OpenMCPSettingsArgs, ctx: &mut AppContext) {
    let autoinstall = args.autoinstall.clone();
    let root_handle = open_new_window_get_handles(None, ctx).1;
    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_view_handle) =
            &root_view.auth_onboarding_state
        {
            let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
            workspace_view_handle.update(ctx, |_, ctx| {
                let _ = ctx.spawn(initial_load_complete, move |workspace, _, ctx| {
                    workspace.open_mcp_servers_page(
                        MCPServersSettingsPage::List,
                        autoinstall.as_deref(),
                        ctx,
                    )
                });
            });
        }
    });
}

/// Opens a new window and shows the Codex modal.
fn open_codex_in_new_window(_: &(), ctx: &mut AppContext) {
    let root_handle = open_new_window_get_handles(None, ctx).1;
    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_view_handle) =
            &root_view.auth_onboarding_state
        {
            let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
            workspace_view_handle.update(ctx, |_, ctx| {
                let _ = ctx.spawn(initial_load_complete, move |workspace, _, ctx| {
                    workspace.open_codex_modal(ctx)
                });
            });
        }
    });
}

/// Opens a new window and enters agent view with the Linear issue work prompt.
fn open_linear_issue_work_in_new_window(args: &LinearIssueWork, ctx: &mut AppContext) {
    let (_, root_handle) = open_new_window_get_handles(None, ctx);
    let args = args.clone();
    root_handle.update(ctx, |root_view, ctx| {
        if let AuthOnboardingState::Terminal(workspace_view_handle) =
            &root_view.auth_onboarding_state
        {
            workspace_view_handle.update(ctx, |workspace, ctx| {
                workspace.open_linear_issue_work(&args, ctx);
            });
        }
    });
}

fn open_warp_drive_object(arg: &OpenWarpDriveObjectArgs, ctx: &mut AppContext) {
    match arg.object_type {
        ObjectType::Notebook => open_new_workspace_with_notebook_open(
            SyncId::ServerId(arg.server_id),
            arg.settings.clone(),
            ctx,
        ),
        ObjectType::Workflow => open_new_workspace_with_workflow_open(
            SyncId::ServerId(arg.server_id),
            arg.settings.clone(),
            ctx,
        ),
        _ => log::info!("Open object type {:?} not yet supported", arg.object_type),
    }
}

fn display_object_missing_error_in_window(window_id: WindowId, ctx: &mut AppContext) {
    crate::workspace::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
        let toast = DismissibleToast::error(String::from("Resource not found or access denied"));
        toast_stack.add_ephemeral_toast(toast, window_id, ctx);
    });
}

fn open_new_workspace_with_notebook_open(
    notebook_id: SyncId,
    settings: OpenWarpDriveObjectSettings,
    ctx: &mut AppContext,
) {
    open_new_with_workspace_source(
        NewWorkspaceSource::NotebookById {
            id: notebook_id,
            settings,
        },
        ctx,
    );
}

fn open_new_workspace_with_workflow_open(
    workflow_id: SyncId,
    settings: OpenWarpDriveObjectSettings,
    ctx: &mut AppContext,
) {
    open_new_with_workspace_source(
        NewWorkspaceSource::WorkflowById {
            id: workflow_id,
            settings,
        },
        ctx,
    );
}

/// Opens a new window with a file-based notebook open.
fn open_new_with_file_notebook(arg: &PathBuf, ctx: &mut AppContext) {
    open_new_with_workspace_source(
        NewWorkspaceSource::NotebookFromFilePath {
            file_path: Some(arg.to_owned()),
        },
        ctx,
    );
}

/// Creates a new window and returns its [`WindowId`] and root view's [`ViewHandle`].
pub(crate) fn open_new_window_get_handles(
    shell: Option<AvailableShell>,
    ctx: &mut AppContext,
) -> (WindowId, ViewHandle<RootView>) {
    let active_window_id = ctx.windows().active_window();
    open_new_with_workspace_source(
        NewWorkspaceSource::Empty {
            previous_active_window: active_window_id,
            shell,
        },
        ctx,
    )
}

/// Opens a new window.
fn open_new(_: &(), ctx: &mut AppContext) {
    open_new_window_get_handles(None, ctx);
}

/// Opens a new window with a specific shell
fn open_new_with_shell(shell: &Option<AvailableShell>, ctx: &mut AppContext) {
    open_new_window_get_handles(shell.to_owned(), ctx);
}

/// Global action that performs a few steps:
/// 1. Open a new tab, or open a window if there is none.
/// 2. Set the terminal input buffer to a command that should open a subshell
/// 3. Set a flag that we should automatically bootstrap that subshell if its we can bootstrap its
/// [`ShellType`].
fn open_new_tab_insert_subshell_command_and_bootstrap_if_supported(
    arg: &SubshellCommandArg,
    ctx: &mut AppContext,
) {
    let root_view_handle: Option<ViewHandle<RootView>> = ctx
        .windows()
        .frontmost_window_id()
        .and_then(|window_id| ctx.root_view(window_id));

    let root_view_handle = match root_view_handle {
        Some(root_view_handle) => {
            root_view_handle.update(ctx, |root_view, ctx| {
                if let AuthOnboardingState::Terminal(workspace_view_handle) =
                    &root_view.auth_onboarding_state
                {
                    workspace_view_handle.update(ctx, |workspace, ctx| {
                        workspace.add_terminal_tab(false /* hide_homepage */, ctx);
                    });
                }
            });
            root_view_handle
        }
        None => open_new_window_get_handles(None, ctx).1,
    };

    root_view_handle.update(ctx, |root_view, ctx| {
        root_view.insert_subshell_command_and_bootstrap_if_supported(arg, ctx);
    });
}

/// Returns the common configuration for a new "regular" window (not Quake Mode).
fn default_window_options(window_settings: &WindowSettings, ctx: &AppContext) -> AddWindowOptions {
    let (inherited_bounds, window_style) = ctx.next_window_bounds_and_style();
    let next_bounds =
        bounds_for_opening_at_custom_window_size(inherited_bounds, window_settings, ctx);

    AddWindowOptions {
        window_style,
        window_bounds: next_bounds,
        title: Some("Warp".to_owned()),
        background_blur_radius_pixels: Some(*window_settings.background_blur_radius),
        background_blur_texture: *window_settings.background_blur_texture,
        on_gpu_driver_selected: on_gpu_driver_selected_callback(),
        ..Default::default()
    }
}

/// Returns the bounds to open the next window at taking into account whether
/// the user has configured their settings to open windows at a custom size
/// and whether that feature is flagged on.
fn bounds_for_opening_at_custom_window_size(
    bounds: WindowBounds,
    window_settings: &WindowSettings,
    app: &AppContext,
) -> WindowBounds {
    if *window_settings.open_windows_at_custom_size.value() {
        let font_cache = app.font_cache();
        let appearance = Appearance::as_ref(app);

        let cell_size_and_padding = cell_size_and_padding(
            font_cache,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
            appearance.ui_builder().line_height_ratio(),
        );
        let window_size = vec2f(
            *window_settings.new_windows_num_columns.value() as f32
                * cell_size_and_padding.cell_width_px.as_f32()
                + 2. * cell_size_and_padding.padding_x_px.as_f32(),
            *window_settings.new_windows_num_rows.value() as f32
                * cell_size_and_padding.cell_height_px.as_f32()
                + 2. * cell_size_and_padding.padding_y_px.as_f32(),
        );

        match bounds {
            WindowBounds::ExactPosition(rect) => {
                WindowBounds::ExactPosition(RectF::new(rect.origin(), window_size))
            }
            WindowBounds::ExactSize(_) | WindowBounds::Default => {
                WindowBounds::ExactSize(window_size)
            }
        }
    } else {
        bounds
    }
}

pub fn quake_mode_window_is_open() -> bool {
    let quake_mode_state = QUAKE_STATE.lock();

    quake_mode_state
        .as_ref()
        .map(|state| {
            matches!(
                state.window_state,
                WindowState::Open | WindowState::PendingOpen
            )
        })
        .unwrap_or_default()
}

pub fn quake_mode_window_id() -> Option<WindowId> {
    let quake_mode_state = QUAKE_STATE.lock();

    quake_mode_state.as_ref().map(|state| state.window_id)
}

pub fn set_quake_mode(new_state: Option<QuakeModeState>) {
    let mut quake_mode_state = QUAKE_STATE.lock();
    *quake_mode_state = new_state;
}

fn move_quake_mode_window_from_screen_change(settings: &QuakeModeSettings, ctx: &mut AppContext) {
    fit_quake_mode_window_within_active_screen(
        settings,
        QuakeModeMoveTrigger::ScreenConfigurationChange,
        ctx,
    )
}

/// If there exists a quake window, mutate its size and position, i.e. its bounds, to match the
/// bounds specified by the [`QuakeModeSettings`].
pub fn update_quake_window_bounds(quake_settings: &QuakeModeSettings, ctx: &mut AppContext) {
    let config = quake_mode_config(quake_settings, ctx);
    let Some(ref state) = *QUAKE_STATE.lock() else {
        return;
    };
    ctx.windows()
        .set_window_bounds(state.window_id, config.window_bounds);
}

/// Move Quake Mode window to the active screen if it is already open or hidden.
fn fit_quake_mode_window_within_active_screen(
    settings: &QuakeModeSettings,
    trigger: QuakeModeMoveTrigger,
    ctx: &mut AppContext,
) {
    let mut quake_mode_state = QUAKE_STATE.lock();

    if let Some(state) = quake_mode_state.as_mut() {
        let active_id = ctx.windows().active_display_id();

        // When there is no screen config and active screen id change, we don't need to reposition
        // the quake mode window as its position should still be valid.
        if matches!(trigger, QuakeModeMoveTrigger::ActiveScreenSetting)
            && active_id == state.active_display_id
        {
            return;
        }

        let window_bound = settings.resolve_quake_mode_bounds(ctx);
        ctx.windows()
            .set_window_bounds(state.window_id, window_bound);
        state.active_display_id = active_id;
    }
}

fn update_quake_mode_state(arg: &UpdateQuakeModeEventArg, ctx: &mut AppContext) {
    if !KeysSettings::as_ref(ctx)
        .quake_mode_settings
        .hide_window_when_unfocused
    {
        return;
    }

    {
        let mut quake_mode_state = QUAKE_STATE.lock();

        if let Some(state) = quake_mode_state.as_mut() {
            state.window_state = match state.window_state {
                WindowState::PendingOpen => WindowState::Open,
                WindowState::Open => {
                    if arg.active_window_id.is_some_and(|id| id == state.window_id) {
                        WindowState::Open
                    } else {
                        ctx.windows().hide_window(state.window_id);
                        WindowState::Hidden
                    }
                }
                WindowState::Hidden => WindowState::Hidden,
            }
        }
    }
}

// Configuration of the next positioning of the quake mode window.
fn quake_mode_config(settings: &QuakeModeSettings, ctx: &mut AppContext) -> QuakeModeFrameConfig {
    QuakeModeFrameConfig {
        display_id: ctx.windows().active_display_id(),
        window_bounds: settings.resolve_quake_mode_bounds(ctx),
    }
}

fn get_quake_mode_state(ctx: &mut AppContext) -> Option<QuakeModeState> {
    let quake_mode_state = QUAKE_STATE.lock();

    match quake_mode_state.as_ref() {
        Some(state) if ctx.is_window_open(state.window_id) => Some(state.clone()),
        _ => None,
    }
}

fn toggle_quake_mode_window(global_resource_handles: &GlobalResourceHandles, ctx: &mut AppContext) {
    // Get the current state of quake mode.
    let state = get_quake_mode_state(ctx);
    match state {
        None => {
            send_telemetry_from_app_ctx!(TelemetryEvent::OpenQuakeModeWindow, ctx);

            let config = quake_mode_config(
                &KeysSettings::as_ref(ctx)
                    .quake_mode_settings
                    .value()
                    .clone(),
                ctx,
            );

            let window_settings = WindowSettings::as_ref(ctx);

            let active_window_id = ctx.windows().active_window();
            let (id, _) = ctx.add_window(
                AddWindowOptions {
                    window_style: WindowStyle::Pin,
                    window_bounds: WindowBounds::ExactPosition(config.window_bounds),
                    title: Some("Warp".to_owned()),
                    background_blur_radius_pixels: Some(*window_settings.background_blur_radius),
                    background_blur_texture: *window_settings.background_blur_texture,
                    // Ignore the quake window for positioning the next window
                    anchor_new_windows_from_closed_position:
                        warpui::NextNewWindowsHasThisWindowsBoundsUponClose::No,
                    on_gpu_driver_selected: on_gpu_driver_selected_callback(),
                    window_instance: Some(ChannelState::app_id().to_string() + "-hotkey"),
                    ..Default::default()
                },
                |ctx| {
                    let mut view = RootView::new(
                        global_resource_handles.clone(),
                        NewWorkspaceSource::Empty {
                            previous_active_window: active_window_id,
                            shell: None,
                        },
                        ctx,
                    );
                    view.focus(ctx);
                    view
                },
            );

            // Update quake mode state after the call to prevent deadlocking.
            let mut quake_mode_state = QUAKE_STATE.lock();
            *quake_mode_state = Some(QuakeModeState {
                window_state: WindowState::PendingOpen,
                window_id: id,
                active_display_id: config.display_id,
            });
        }
        Some(state) if matches!(state.window_state, WindowState::Hidden) => {
            send_telemetry_from_app_ctx!(TelemetryEvent::OpenQuakeModeWindow, ctx);

            // If quake mode does not have a set pin screen -- move it to the current active screen.
            if KeysSettings::as_ref(ctx)
                .quake_mode_settings
                .pin_screen
                .is_none()
            {
                fit_quake_mode_window_within_active_screen(
                    &KeysSettings::as_ref(ctx)
                        .quake_mode_settings
                        .value()
                        .clone(),
                    QuakeModeMoveTrigger::ActiveScreenSetting,
                    ctx,
                );
            }
            ctx.windows().show_window_and_focus_app(state.window_id);

            // Update quake mode state after the call to prevent deadlocking.
            let mut quake_mode_state = QUAKE_STATE.lock();

            if let Some(state) = quake_mode_state.as_mut() {
                state.window_state = WindowState::PendingOpen;
            }
        }
        Some(state) => {
            ctx.windows().hide_window(state.window_id);

            // Update quake mode state after the call to prevent deadlocking.
            let mut quake_mode_state = QUAKE_STATE.lock();

            if let Some(state) = quake_mode_state.as_mut() {
                state.window_state = WindowState::Hidden;
            }
        }
    };
}

/// This action will show or hide all of Warp's windows except the quake window
///
/// - If Warp is active and has any windows, hide those windows.
/// - If Warp is hidden, show all windows.
/// - If Warp is active but has 0 normal windows, create a new window with a new session.
fn show_or_hide_non_quake_mode_windows(_: &(), ctx: &mut AppContext) {
    let quake_window_id = get_quake_mode_state(ctx).map(|state| state.window_id);
    let non_quake_mode_window_ids = ctx
        .window_ids()
        .filter(|window_id| Some(window_id) != quake_window_id.as_ref());
    if non_quake_mode_window_ids.count() == 0 {
        // If there are no normal windows, this action should create one.
        open_new(&(), ctx);
    }
    let windowing_model = ctx.windows();
    // Now there is at least one window. If a Warp window is active, hide the app.
    // Otherwise, show activate the app to show it in front.
    let active_window_id = windowing_model.active_window();
    match active_window_id {
        Some(_) => windowing_model.hide_app(),
        None => {
            windowing_model.activate_app();
        }
    };
}

#[cfg(feature = "voice_input")]
fn abort_voice_input(_: &(), ctx: &mut AppContext) {
    let voice_input = voice_input::VoiceInput::handle(ctx);
    if voice_input.as_ref(ctx).is_listening() {
        voice_input.update(ctx, |voice_input, _| {
            voice_input.abort_listening();
        });
    }
}

#[derive(Clone)]
pub enum NewWorkspaceSource {
    Empty {
        previous_active_window: Option<WindowId>,
        shell: Option<AvailableShell>,
    },
    FromTemplate {
        window_template: launch_config::WindowTemplate,
    },
    Restored {
        window_snapshot: WindowSnapshot,
        block_lists: Arc<HashMap<PaneUuid, Vec<SerializedBlockListItem>>>,
    },
    Session {
        options: Box<NewTerminalOptions>,
    },
    SharedSessionAsViewer {
        session_id: SessionId,
    },
    FromCloudConversationId {
        conversation_id: ServerConversationToken,
    },
    NotebookFromFilePath {
        file_path: Option<PathBuf>,
    },
    NotebookById {
        id: SyncId,
        settings: OpenWarpDriveObjectSettings,
    },
    WorkflowById {
        id: SyncId,
        settings: OpenWarpDriveObjectSettings,
    },
    AgentSession {
        options: Box<NewTerminalOptions>,
        initial_query: Option<String>,
    },
    /// Starts the workspace with the Cloud Agent setup tab.
    AmbientAgent,
    /// A tab is being transferred from another window via the transferable views framework.
    /// The workspace will create a placeholder tab, which will be replaced by the transferred
    /// PaneGroup after window creation.
    TransferredTab {
        /// Tab color from the source tab
        tab_color: Option<AnsiColorIdentifier>,
        /// Custom title from the source tab
        custom_title: Option<String>,
        /// Whether the left panel was open in the source tab
        left_panel_open: bool,
        /// Captured from the source window so detached tabs inherit the panel state.
        vertical_tabs_panel_open: bool,
        /// Whether the right panel was open in the source tab
        right_panel_open: bool,
        /// Whether the right panel was maximized in the source tab
        is_right_panel_maximized: bool,
        /// Whether this transferred tab window is currently being used as a drag preview.
        is_tab_drag_preview: bool,
    },
}

impl NewWorkspaceSource {
    pub fn has_horizontal_split(&self) -> bool {
        match self {
            NewWorkspaceSource::Restored {
                window_snapshot, ..
            } => {
                if window_snapshot.tabs.is_empty() {
                    false
                } else {
                    let active_index = window_snapshot.active_tab_index;
                    let active_tab = window_snapshot
                        .tabs
                        .get(active_index)
                        .unwrap_or(&window_snapshot.tabs[0]);
                    active_tab.root.has_horizontal_split()
                }
            }
            _ => false,
        }
    }
}

/// Args needed to construct a `Workspace`.
#[derive(Clone)]
struct WorkspaceArgs {
    global_resource_handles: GlobalResourceHandles,
    server_time: Option<Arc<ServerTime>>,
    workspace_setting: NewWorkspaceSource,
}

// Some onboarding states can either contain a ref to an existing terminal view
// if it exists or, if it doesn't, the args needed to create a new empty one.
#[derive(Clone)]
enum AuthOnboardingTarget {
    Workspace(Box<WorkspaceArgs>),
    Terminal(ViewHandle<Workspace>),
}

/// User preferences key to track whether the user has completed the onboarding slides locally
/// (before login). This is needed because the server-side `is_onboarded` flag requires
/// authentication.
const HAS_COMPLETED_ONBOARDING_KEY: &str = "HasCompletedOnboarding";

/// Returns whether the user has completed the onboarding slides locally (before login).
pub(crate) fn has_completed_local_onboarding(ctx: &AppContext) -> bool {
    ctx.private_user_preferences()
        .read_value(HAS_COMPLETED_ONBOARDING_KEY)
        .unwrap_or_default()
        .and_then(|s| serde_json::from_str::<bool>(&s).ok())
        .unwrap_or(false)
}

/// Persists the local onboarding-completed flag so we don't show onboarding again.
fn mark_local_onboarding_completed(ctx: &AppContext) {
    let _ = ctx.private_user_preferences().write_value(
        HAS_COMPLETED_ONBOARDING_KEY,
        serde_json::to_string(&true).expect("bool serializes to JSON"),
    );
}

/// Whether auth and onboarding have completed and we should render the `Workspace`.
enum AuthOnboardingState {
    Auth(Box<WorkspaceArgs>),
    ConfirmIncomingAuth(Box<WorkspaceArgs>),
    /// The client is importing auth state from the host application.
    #[cfg(target_family = "wasm")]
    WebImport(AuthOnboardingTarget),
    NeedsSsoLink(AuthOnboardingTarget),
    Onboarding {
        onboarding_view: ViewHandle<AgentOnboardingView>,
        target: AuthOnboardingTarget,
    },
    /// Post-onboarding login slide (full-screen, onboarding-style).
    LoginSlide {
        login_slide_view: ViewHandle<LoginSlideView>,
        onboarding_view: ViewHandle<AgentOnboardingView>,
        target: AuthOnboardingTarget,
    },
    Terminal(ViewHandle<Workspace>),
}

pub struct RootView {
    auth_onboarding_state: AuthOnboardingState,
    server_time: Option<Arc<ServerTime>>,
    auth_view: ViewHandle<AuthView>,
    auth_override_view: ViewHandle<AuthOverrideWarningModal>,
    needs_sso_link_view: ViewHandle<NeedsSsoLinkView>,
    #[cfg(target_family = "wasm")]
    web_handoff_view: ViewHandle<WebHandoffView>,
    pub server_api: Arc<ServerApi>,
    pub model_event_sender: Option<SyncSender<ModelEvent>>,
    mouse_states: TrafficLightMouseStates,
    /// The window ID is needed because the "maximize" button needs to change its icon based on
    /// whether or not the current window is maximized. Ideally the window ID could just be fetched
    /// in the [`Self::render`] method, but there is no [`ViewContext`] available there. So, we
    /// need to store it in a field instead.
    window_id: WindowId,
    /// Stores the tutorial from onboarding when the user needs to log in before
    /// the guided tour can start. Consumed after auth completes.
    pending_tutorial: Option<OnboardingTutorial>,
    /// settings to apply after a new user login / initial cloud load completes
    pending_post_auth_onboarding_settings: Option<SelectedSettings>,
    paste_auth_token_modal: Option<ViewHandle<PasteAuthTokenModalView>>,
}

impl RootView {
    pub fn new(
        global_resource_handles: GlobalResourceHandles,
        workspace_setting: NewWorkspaceSource,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let server_api_provider = ServerApiProvider::as_ref(ctx);
        let server_api = server_api_provider.get();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| {
            me.handle_auth_manager_event(event, ctx);
        });

        ctx.subscribe_to_model(&CloudPreferencesSyncer::handle(ctx), |me, _, event, ctx| {
            me.handle_cloud_preferences_syncer_event(event, ctx);
        });

        let auth_view =
            ctx.add_typed_action_view(|ctx| AuthView::new(AuthViewVariant::Initial, ctx));

        let auth_override_view: ViewHandle<_> = ctx.add_typed_action_view(|ctx| {
            AuthOverrideWarningModal::new(ctx, AuthOverrideWarningModalVariant::OnboardingView)
        });

        ctx.subscribe_to_view(&auth_override_view, |me, _, event, ctx| {
            me.handle_auth_override_warning_modal_event(event, ctx);
        });

        let model_event_sender = global_resource_handles.model_event_sender.clone();
        let workspace_args = WorkspaceArgs {
            global_resource_handles,
            server_time: None,
            workspace_setting,
        };

        let auth_onboarding_state = if auth_state.is_logged_in() {
            AuthOnboardingState::Terminal(workspace_args.create_workspace(ctx))
        } else {
            cfg_if! {
                if #[cfg(target_family = "wasm")] {
                    AuthOnboardingState::WebImport(AuthOnboardingTarget::Workspace(workspace_args.into()))
                } else {
                    // When OpenWarpNewSettingsModes is enabled, show onboarding before login for
                    // users who haven't completed it yet (tracked via a local UserPreferences key).
                    let has_completed_local_onboarding = FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
                        && has_completed_local_onboarding(ctx);
                    let should_show_pre_login_onboarding = FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
                        && FeatureFlag::AgentOnboarding.is_enabled()
                        && !has_completed_local_onboarding;
                    if FeatureFlag::ForceLogin.is_enabled() {
                        // ForceLogin is true for Preview
                        AuthOnboardingState::Auth(workspace_args.into())
                    } else if should_show_pre_login_onboarding {
                        let workspace_args_box: Box<WorkspaceArgs> = workspace_args.into();
                        let onboarding_view = Self::create_agent_onboarding_view(ctx);
                        onboarding_view.update(ctx, |view, ctx| {
                            view.start_onboarding(ctx);
                        });
                        AuthOnboardingState::Onboarding {
                            onboarding_view,
                            target: AuthOnboardingTarget::Workspace(workspace_args_box),
                        }
                    } else if FeatureFlag::SkipFirebaseAnonymousUser.is_enabled() {
                        // When SkipFirebaseAnonymousUser is enabled, skip the login screen
                        // entirely and go directly into the workspace.
                        AuthOnboardingState::Terminal(workspace_args.create_workspace(ctx))
                    } else {
                        AuthOnboardingState::Auth(workspace_args.into())
                    }
                }
            }
        };

        let needs_sso_link_view = ctx.add_typed_action_view(|_| NeedsSsoLinkView::new());

        #[cfg(target_family = "wasm")]
        let web_handoff_view = {
            let view = ctx.add_view(WebHandoffView::new);
            ctx.subscribe_to_view(&view, Self::handle_web_handoff_event);
            view
        };

        let root_view = Self {
            auth_onboarding_state,
            server_time: None,
            auth_view,
            auth_override_view,
            needs_sso_link_view,
            #[cfg(target_family = "wasm")]
            web_handoff_view,
            server_api: server_api.clone(),
            model_event_sender,
            mouse_states: Default::default(),
            window_id: ctx.window_id(),
            pending_tutorial: None,
            pending_post_auth_onboarding_settings: None,
            paste_auth_token_modal: None,
        };

        match &root_view.auth_onboarding_state {
            AuthOnboardingState::Terminal(workspace) if FeatureFlag::Changelog.is_enabled() => {
                // Only show the changelog if we aren't about to launch the authentication flow
                workspace.update(ctx, |workspace, ctx| {
                    workspace.check_for_changelog(ChangelogRequestType::WindowLaunch, ctx);
                })
            }
            AuthOnboardingState::Auth(_) => {
                // ApplePressAndHoldEnabled is the setting for whether or not the accent
                // menu is shown when a key is held. If "false", we repeat the character
                // instead of showing the menu like the default terminal. We only override
                // the default if it's not already set and the user is logging in.
                #[cfg(target_os = "macos")]
                {
                    use warpui_extras::user_preferences::UserPreferences;

                    // Make sure we're interacting with user defaults instead
                    // of some other preferences store.  Apple implements some
                    // per-application overrides of system preferences via user
                    // defaults (like press-and-hold being either accented
                    // characters or key repeat), so we need to make sure we're
                    // interacting with the user defaults system.
                    let user_defaults = warpui_extras::user_preferences::user_defaults::UserDefaultsPreferencesStorage::new(None);
                    if user_defaults
                        .read_value("ApplePressAndHoldEnabled")
                        .unwrap_or_default()
                        .is_none()
                    {
                        let _ = user_defaults
                            .write_value("ApplePressAndHoldEnabled", "false".to_owned());
                    }
                }
            }
            #[cfg(target_family = "wasm")]
            AuthOnboardingState::WebImport(_) => {
                root_view
                    .web_handoff_view
                    .update(ctx, |view, ctx| view.import_user(ctx));
            }
            _ => {}
        }

        let autoupdate_handle = AutoupdateState::handle(ctx);
        ctx.subscribe_to_model(&autoupdate_handle, |root_view, _handle, evt, ctx| {
            if let AutoupdateStateEvent::CheckComplete {
                result,
                request_type: RequestType::Poll,
            } = evt
            {
                root_view.polling_update_check_complete(result, ctx)
            }
        });

        // Ensure the onboarding view has focus after all views are created.
        // The auth_view's internal editor may have grabbed focus during construction;
        // this overrides that so keyboard input (Enter, arrow keys) routes to onboarding.
        if let AuthOnboardingState::Onboarding {
            onboarding_view, ..
        } = &root_view.auth_onboarding_state
        {
            ctx.focus(onboarding_view);
        }

        // For users who bypass onboarding (already logged in, or onboarding flags not active),
        // start autoupdate polling immediately. For new users in onboarding, this is a no-op;
        // polling will be started once onboarding completes.
        root_view.start_autoupdate_polling(ctx);

        root_view
    }

    /// Starts the autoupdate polling loop, but only if we are already in the `Terminal` state
    /// (i.e. onboarding has completed or was not shown). Safe to call unconditionally — it is
    /// a no-op when still in a pre-terminal state.
    fn start_autoupdate_polling(&self, ctx: &mut ViewContext<Self>) {
        if matches!(self.auth_onboarding_state, AuthOnboardingState::Terminal(_)) {
            AutoupdateState::handle(ctx).update(ctx, |state, ctx| state.start_polling(ctx));
        }
    }

    /// Used for integration tests.
    pub fn workspace_view(&self) -> Option<&ViewHandle<Workspace>> {
        match &self.auth_onboarding_state {
            AuthOnboardingState::Terminal(workspace) => Some(workspace),
            _ => None,
        }
    }

    fn polling_update_check_complete(
        &mut self,
        result: &Result<UpdateReady>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Ok(UpdateReady::Yes {
            ref new_version, ..
        }) = result
        {
            log::info!("Update ready for channel version {new_version:?}");
            if new_version.update_by.is_some() {
                log::info!("Update ready, there is an update-by time, checking for server time.");
                let server_api = self.server_api.clone();
                let _ = ctx.spawn(
                    async move { server_api.server_time().await },
                    Self::server_time_updated,
                );
            }
        }
    }

    fn server_time_updated(
        &mut self,
        server_time: Result<ServerTime>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Ok(server_time) = server_time {
            let server_time = Arc::new(server_time);
            self.server_time = Some(server_time.clone());

            if let AuthOnboardingState::Terminal(workspace) = &self.auth_onboarding_state {
                workspace.update(ctx, |workspace, ctx| {
                    workspace.set_server_time(server_time);
                    ctx.notify();
                })
            }
        } else {
            log::error!("Error fetching server time {:?}", server_time.err());
        }
    }

    // Switch to Auth Screen while destroying Workspace.
    fn log_out(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        self.auth_onboarding_state.log_out(ctx);
        ctx.focus_self();
        ctx.notify();
        true
    }

    fn show_needs_sso_link_view(&mut self, email: String, ctx: &mut ViewContext<Self>) -> bool {
        self.needs_sso_link_view.update(ctx, |view, _| {
            view.set_email(email);
        });

        self.auth_onboarding_state.show_needs_sso_link_view();
        ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        ctx.notify();
        true
    }

    /// Hand off the authenticated user from the host web application.
    #[cfg(target_family = "wasm")]
    fn web_handoff(&mut self, ctx: &mut ViewContext<Self>) {
        log::debug!("Starting handoff from host application");
        self.web_handoff_view
            .update(ctx, |view, ctx| view.import_user(ctx));
        self.auth_onboarding_state.show_web_handoff_view();
        ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        ctx.notify();
    }

    fn close_window(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        if ContextFlag::CloseWindow.is_enabled() {
            ctx.close_window();
        }
        true
    }

    fn toggle_maximize_window(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        ctx.toggle_maximized_window();
        true
    }

    fn toggle_fullscreen(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        let window_id = ctx.window_id();
        WindowManager::handle(ctx).update(ctx, |state, ctx| {
            state.toggle_fullscreen(window_id, ctx);
        });
        true
    }

    fn create_agent_onboarding_view(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<AgentOnboardingView> {
        LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
            prefs.refresh_available_models(ctx);
        });

        let themes = onboarding_theme_picker_themes();
        let onboarding_view = ctx.add_typed_action_view(move |ctx| {
            let (models, default_model_id) =
                build_onboarding_models(LLMPreferences::as_ref(ctx), ctx);

            let workspace_enforces_autonomy = UserWorkspaces::as_ref(ctx)
                .ai_autonomy_settings()
                .has_any_overrides();

            let auth_state = current_onboarding_auth_state(ctx);

            AgentOnboardingView::new(
                themes.clone(),
                false, // Always use unskippable onboarding.
                models,
                default_model_id,
                workspace_enforces_autonomy,
                FeatureFlag::AgentView.is_enabled(),
                auth_state,
                ctx,
            )
        });

        let onboarding_view_clone = onboarding_view.clone();
        ctx.subscribe_to_model(
            &LLMPreferences::handle(ctx),
            move |_, llm_preferences, event, ctx| match event {
                LLMPreferencesEvent::UpdatedAvailableLLMs => {
                    let (models, default_model_id) =
                        build_onboarding_models(llm_preferences.as_ref(ctx), ctx);
                    onboarding_view_clone.update(ctx, |onboarding_view, ctx| {
                        onboarding_view.set_onboarding_models(models, default_model_id, ctx);
                    })
                }

                LLMPreferencesEvent::UpdatedActiveAgentModeLLM
                | LLMPreferencesEvent::UpdatedActiveCodingLLM => {}
            },
        );

        // Subscribe to workspace changes to update autonomy enforcement state and auth/billing
        // state (e.g. a free→paid upgrade reflected by the workspace/billing metadata poll).
        let onboarding_view_for_workspaces = onboarding_view.clone();
        ctx.subscribe_to_model(
            &UserWorkspaces::handle(ctx),
            move |_, user_workspaces, event, ctx| {
                if matches!(event, UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess) {
                    let workspace_enforces_autonomy = user_workspaces
                        .as_ref(ctx)
                        .ai_autonomy_settings()
                        .has_any_overrides();
                    onboarding_view_for_workspaces.update(ctx, |onboarding_view, ctx| {
                        onboarding_view
                            .set_workspace_enforces_autonomy(workspace_enforces_autonomy, ctx);
                    });
                }
                let auth_state = current_onboarding_auth_state(ctx);
                onboarding_view_for_workspaces.update(ctx, |onboarding_view, ctx| {
                    onboarding_view.set_auth_state(auth_state, ctx);
                });
            },
        );

        let onboarding_view_for_auth = onboarding_view.clone();
        ctx.subscribe_to_model(
            &AuthManager::handle(ctx),
            move |_, _auth_manager, event, ctx| {
                if matches!(
                    event,
                    AuthManagerEvent::AuthComplete | AuthManagerEvent::SkippedLogin
                ) {
                    let auth_state = current_onboarding_auth_state(ctx);
                    onboarding_view_for_auth.update(ctx, |onboarding_view, ctx| {
                        onboarding_view.set_auth_state(auth_state, ctx);
                    });
                    if matches!(event, AuthManagerEvent::AuthComplete) {
                        LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                            prefs.refresh_available_models(ctx);
                        });
                        TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
                            drop(manager.refresh_workspace_metadata(ctx));
                        });
                    }
                }
            },
        );

        ctx.subscribe_to_view(&onboarding_view, |me, _view, event, ctx| {
            me.handle_agent_onboarding_event(event, ctx);
        });
        onboarding_view
    }

    /// Debug method to enter the onboarding state.
    fn debug_enter_onboarding_state(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        if !ChannelState::enable_debug_features() {
            log::warn!("Attempted to enter onboarding state in release build");
            return false;
        }

        if !FeatureFlag::AgentOnboarding.is_enabled() {
            log::warn!("Attempted to enter onboarding state without AgentOnboarding enabled");
            return false;
        }

        self.auth_onboarding_state.try_open_onboarding_slides(ctx);

        ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        ctx.notify();
        true
    }

    fn onboarding_theme_kind(theme_name: &str) -> Option<ThemeKind> {
        WarpThemeConfig::new()
            .theme_items()
            .find_map(|(kind, theme)| {
                (theme.name().as_deref() == Some(theme_name)).then(|| kind.clone())
            })
    }

    fn handle_login_slide_event(&mut self, event: &LoginSlideEvent, ctx: &mut ViewContext<Self>) {
        match event {
            LoginSlideEvent::BackToOnboarding => {
                let AuthOnboardingState::LoginSlide {
                    onboarding_view,
                    target,
                    ..
                } = &self.auth_onboarding_state
                else {
                    return;
                };
                let onboarding_view = onboarding_view.clone();
                let target = target.clone();
                self.auth_onboarding_state = AuthOnboardingState::Onboarding {
                    onboarding_view,
                    target,
                };
                self.pending_tutorial = None;
                self.pending_post_auth_onboarding_settings = None;
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.focus(ctx);
                ctx.notify();
            }
            LoginSlideEvent::LoginLaterConfirmed => {
                let AuthOnboardingState::LoginSlide { target, .. } = &self.auth_onboarding_state
                else {
                    return;
                };
                let workspace = target.to_workspace(ctx);
                // User opted out of login: apply locally (no cloud race).
                // Skipping leaves the user without an account, so AI is disabled.
                if let Some(selected_settings) = self.pending_post_auth_onboarding_settings.take() {
                    apply_onboarding_settings(&selected_settings, false, ctx);
                }
                self.auth_onboarding_state = AuthOnboardingState::Terminal(workspace);
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.start_pending_tutorial(ctx);
                self.start_autoupdate_polling(ctx);
                self.focus(ctx);
                ctx.notify();
            }
        }
    }

    fn handle_agent_onboarding_event(
        &mut self,
        event: &AgentOnboardingEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentOnboardingEvent::ThemeSelected { theme_name } => {
                let Some(theme_kind) = Self::onboarding_theme_kind(theme_name) else {
                    log::warn!("Unknown onboarding theme selected: {theme_name}");
                    return;
                };

                // Update both what we render with immediately, and the user's theme setting.
                ThemeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.use_system_theme.set_value(false, ctx));
                    report_if_error!(settings.theme_kind.set_value(theme_kind.clone(), ctx));
                });
            }
            AgentOnboardingEvent::SyncWithOsToggled { enabled } => {
                ThemeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.use_system_theme.set_value(*enabled, ctx));
                });
            }
            AgentOnboardingEvent::OnboardingCompleted(selected_settings) => {
                let AuthOnboardingState::Onboarding {
                    target,
                    onboarding_view,
                } = &self.auth_onboarding_state
                else {
                    return;
                };
                let target = target.clone();
                let onboarding_view = onboarding_view.clone();

                mark_local_onboarding_completed(ctx);
                if FeatureFlag::HOAOnboardingFlow.is_enabled() {
                    mark_hoa_onboarding_completed(ctx);
                }

                let is_logged_in = AuthStateProvider::as_ref(ctx).get().is_logged_in();
                // If the user isn't logged in, only require login if the applied
                // settings need an account (AI or Warp Drive enabled).
                let ai_enabled = selected_settings.is_ai_enabled();
                let warp_drive_enabled = selected_settings.is_warp_drive_enabled();
                // With old onboarding, we ask user to log in before onboarding, so don't do it after onboarding completes.
                let requires_login = !is_logged_in
                    && (ai_enabled || warp_drive_enabled)
                    && FeatureFlag::OpenWarpNewSettingsModes.is_enabled();

                if requires_login {
                    let tutorial = OnboardingTutorial::from(selected_settings.clone());
                    self.pending_tutorial = Some(tutorial);

                    let appearance = Appearance::as_ref(ctx);
                    let theme_name = appearance
                        .theme()
                        .name()
                        .unwrap_or_else(|| "Dark".to_string());
                    let (use_vertical_tabs, intention, uses_third_party_agents) =
                        match selected_settings {
                            SelectedSettings::AgentDrivenDevelopment {
                                ui_customization,
                                agent_settings,
                                ..
                            } => (
                                ui_customization
                                    .as_ref()
                                    .map(|c| c.use_vertical_tabs)
                                    .unwrap_or(true),
                                OnboardingIntention::AgentDrivenDevelopment,
                                agent_settings.disable_oz,
                            ),
                            SelectedSettings::Terminal {
                                ui_customization, ..
                            } => (
                                ui_customization
                                    .as_ref()
                                    .map(|c| c.use_vertical_tabs)
                                    .unwrap_or(false),
                                OnboardingIntention::Terminal,
                                false,
                            ),
                        };

                    let login_slide_view = ctx.add_typed_action_view(|ctx| {
                        LoginSlideView::new(
                            ai_enabled,
                            uses_third_party_agents,
                            &theme_name,
                            use_vertical_tabs,
                            intention,
                            LoginSlideSource::OnboardingFlow,
                            ctx,
                        )
                    });
                    ctx.subscribe_to_view(&login_slide_view, |me, _view, event, ctx| {
                        me.handle_login_slide_event(event, ctx);
                    });

                    self.pending_post_auth_onboarding_settings = Some(selected_settings.clone());
                    self.auth_onboarding_state = AuthOnboardingState::LoginSlide {
                        login_slide_view,
                        onboarding_view,
                        target,
                    };
                    ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                    self.focus(ctx);
                    ctx.notify();
                    return;
                }

                apply_onboarding_settings(selected_settings, is_logged_in, ctx);

                if is_logged_in {
                    AuthManager::handle(ctx)
                        .update(ctx, |model, ctx| model.set_user_onboarded(ctx));
                }

                let workspace = target.to_workspace(ctx);
                let tutorial = OnboardingTutorial::from(selected_settings.clone());
                self.pending_tutorial = Some(tutorial);
                self.auth_onboarding_state = AuthOnboardingState::Terminal(workspace);
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.start_pending_tutorial(ctx);
                self.start_autoupdate_polling(ctx);
                ctx.notify();
            }
            AgentOnboardingEvent::OnboardingSkipped => {
                let AuthOnboardingState::Onboarding { target, .. } = &self.auth_onboarding_state
                else {
                    return;
                };

                mark_local_onboarding_completed(ctx);
                if FeatureFlag::HOAOnboardingFlow.is_enabled() {
                    mark_hoa_onboarding_completed(ctx);
                }

                if AuthStateProvider::as_ref(ctx).get().is_logged_in() {
                    AuthManager::handle(ctx)
                        .update(ctx, |model, ctx| model.set_user_onboarded(ctx));
                }

                let workspace = target.to_workspace(ctx);
                self.auth_onboarding_state = AuthOnboardingState::Terminal(workspace);
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.start_autoupdate_polling(ctx);
                ctx.notify();
            }
            AgentOnboardingEvent::UpgradeRequested => {
                let upgrade_url = AuthManager::handle(ctx)
                    .update(ctx, |auth_manager, _| auth_manager.upgrade_url());
                ctx.open_url(&upgrade_url);
            }
            AgentOnboardingEvent::UpgradeCopyUrlRequested => {
                let upgrade_url = AuthManager::handle(ctx)
                    .update(ctx, |auth_manager, _| auth_manager.upgrade_url());
                ctx.clipboard().write(ClipboardContent {
                    plain_text: upgrade_url.clone(),
                    paths: Some(vec![upgrade_url]),
                    ..Default::default()
                });
            }
            AgentOnboardingEvent::UpgradePasteTokenFromClipboardRequested => {
                let modal = ctx.add_typed_action_view(PasteAuthTokenModalView::new);
                ctx.subscribe_to_view(&modal, |me, _, event, ctx| match event {
                    PasteAuthTokenModalEvent::Cancelled => {
                        me.paste_auth_token_modal = None;
                        me.focus(ctx);
                        ctx.notify();
                    }
                });
                ctx.focus(&modal);
                self.paste_auth_token_modal = Some(modal);
                ctx.notify();
            }
            AgentOnboardingEvent::PrivacySettingsFromTerminalThemeSlideRequested => {
                let AuthOnboardingState::Onboarding {
                    target,
                    onboarding_view,
                } = &self.auth_onboarding_state
                else {
                    return;
                };
                let target = target.clone();
                let onboarding_view = onboarding_view.clone();

                // This event is only emitted from the terminal-intention theme
                // slide (the variant name encodes this). The terminal intention
                // disables AI once onboarding settings are applied, so treat AI
                // as disabled here — `AISettings::is_any_ai_enabled` still holds
                // the pre-onboarding / default value at this point and would
                // incorrectly surface the cloud-conversation toggle.
                let ai_enabled = false;
                let appearance = Appearance::as_ref(ctx);
                let theme_name = appearance
                    .theme()
                    .name()
                    .unwrap_or_else(|| "Dark".to_string());
                // Match the theme slide's image: read the onboarding view's in-progress
                // customization rather than the globally-applied TabSettings, which still
                // holds the user's pre-onboarding (or default) value until the flow
                // completes.
                let use_vertical_tabs = onboarding_view.as_ref(ctx).use_vertical_tabs(ctx);

                // This event variant encodes that it was emitted from the
                // terminal-intention theme slide, so match its image here.
                let login_slide_view = ctx.add_typed_action_view(|ctx| {
                    LoginSlideView::new(
                        ai_enabled,
                        // Terminal intention is never the third-party-agent path.
                        false,
                        &theme_name,
                        use_vertical_tabs,
                        OnboardingIntention::Terminal,
                        LoginSlideSource::PrivacySettingsFromTerminalIntentionTheme,
                        ctx,
                    )
                });
                ctx.subscribe_to_view(&login_slide_view, |me, _view, event, ctx| {
                    me.handle_login_slide_event(event, ctx);
                });

                self.auth_onboarding_state = AuthOnboardingState::LoginSlide {
                    login_slide_view,
                    onboarding_view,
                    target,
                };
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.focus(ctx);
                ctx.notify();
            }
            AgentOnboardingEvent::LoginFromWelcomeRequested => {
                let AuthOnboardingState::Onboarding {
                    target,
                    onboarding_view,
                } = &self.auth_onboarding_state
                else {
                    return;
                };
                let target = target.clone();
                let onboarding_view = onboarding_view.clone();

                let ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
                let appearance = Appearance::as_ref(ctx);
                let theme_name = appearance
                    .theme()
                    .name()
                    .unwrap_or_else(|| "Dark".to_string());
                let use_vertical_tabs = *TabSettings::as_ref(ctx).use_vertical_tabs;

                // Open the sign-in URL in the browser for existing users.
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    let sign_in_url = auth_manager.sign_in_url();
                    ctx.open_url(&sign_in_url);
                });

                let login_slide_view = ctx.add_typed_action_view(|ctx| {
                    LoginSlideView::new(
                        ai_enabled,
                        // No agent setup choice has been made yet; default to the
                        // Warp Agent login screen rather than the third-party copy.
                        false,
                        &theme_name,
                        use_vertical_tabs,
                        // Existing-user login from the welcome slide happens before the user
                        // picks an intention; default the visual to the agent intention panel.
                        OnboardingIntention::AgentDrivenDevelopment,
                        LoginSlideSource::LoginExistingUserFromWelcome,
                        ctx,
                    )
                });
                ctx.subscribe_to_view(&login_slide_view, |me, _view, event, ctx| {
                    me.handle_login_slide_event(event, ctx);
                });

                self.auth_onboarding_state = AuthOnboardingState::LoginSlide {
                    login_slide_view,
                    onboarding_view,
                    target,
                };
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                self.focus(ctx);
                ctx.notify();
            }
            AgentOnboardingEvent::AppBecameActive => {
                // fetch the models / workspace metadata when the user tabs/intents back
                // into the app during onboarding after potentially upgrading
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.refresh_available_models(ctx);
                });
                TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
                    drop(manager.refresh_workspace_metadata(ctx));
                });
            }
        }
    }

    fn minimize_window(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        ctx.minimize_window();
        true
    }

    fn focus_pane(
        &mut self,
        pane_view_locator: &PaneViewLocator,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Focus the appropriate window.
        let window_id = ctx.window_id();

        let mut quake_mode_state = QUAKE_STATE.lock();
        // If the window we are focusing is the Quake Mode window, then update the QuakeModeState.
        if let Some(mode) = quake_mode_state.as_mut() {
            if mode.window_id == window_id {
                mode.window_state = WindowState::Open;
            }
        }

        ctx.windows().show_window_and_focus_app(window_id);

        // Focus the appropriate tab/pane.
        if let AuthOnboardingState::Terminal(workspace) = &self.auth_onboarding_state {
            workspace.update(ctx, |view, ctx| {
                view.focus_pane(*pane_view_locator, ctx);
            });
        }
        true
    }

    fn activate_tab_by_pane_group_id(
        &mut self,
        pane_group_id: &EntityId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        ctx.windows().show_window_and_focus_app(ctx.window_id());
        if let AuthOnboardingState::Terminal(workspace) = &self.auth_onboarding_state {
            workspace.update(ctx, |view, ctx| {
                view.activate_tab_by_pane_group_id(*pane_group_id, ctx);
            });
        }
        true
    }

    fn handle_notification_click(
        &mut self,
        pane_view_locator: &PaneViewLocator,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Focus the pane that the notification originated from.
        self.focus_pane(pane_view_locator, ctx);
        send_telemetry_from_ctx!(TelemetryEvent::NotificationClicked, ctx);
        true
    }

    #[allow(clippy::ptr_arg)]
    fn handle_incoming_auth_url(&mut self, url: &Url, ctx: &mut ViewContext<Self>) -> bool {
        match AuthRedirectPayload::from_url(url.clone()) {
            Ok(redirect_payload) => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.initialize_user_from_auth_payload(redirect_payload, true, ctx);
                });
            }
            Err(error) => {
                safe_error!(
                    safe: ("Unable to parse AuthResult from url"),
                    full: ("Unable to parse AuthResult from url: {error}")
                );
                self.auth_view.update(ctx, |view, ctx| {
                    view.last_login_failure_reason =
                        Some(LoginFailureReason::InvalidRedirectUrl { was_pasted: false });
                    ctx.notify()
                });
            }
        }
        true
    }

    #[allow(clippy::ptr_arg)]
    fn add_session_at_path(&mut self, path: &PathBuf, ctx: &mut ViewContext<Self>) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |view, ctx| {
                view.add_tab_with_pane_layout(
                    PanesLayout::SingleTerminal(Box::new(
                        NewTerminalOptions::default()
                            .with_initial_directory_opt(path_if_directory(path).map(Into::into)),
                    )),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );
                ctx.windows().show_window_and_focus_app(window_id);
                ctx.notify();
            })
        } else {
            log::warn!("Auth not complete before trying to add new session at path");
        }
        true
    }

    pub fn open_team_settings_with_email_invite_in_existing_window(
        &mut self,
        arg: &OpenTeamsSettingsModalArgs,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.show_team_settings_page_with_email_invite(arg.invite_email.as_ref(), ctx)
            });
            return true;
        } else {
            log::warn!("Auth not complete before trying to open settings pane");
        }
        false
    }

    pub fn open_warp_drive_object_in_existing_window(
        &mut self,
        arg: &OpenWarpDriveObjectArgs,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            let cloud_model = CloudModel::as_ref(ctx);

            match arg.object_type {
                ObjectType::Notebook => {
                    handle.update(ctx, |workspace, ctx| {
                        let initialized_section_states =
                            workspace.has_warp_drive_initialized_sections(ctx);
                        let notebook_id = SyncId::ServerId(arg.server_id);
                        let settings = arg.settings.clone();
                        let _ = ctx.spawn(initialized_section_states, move |workspace, _, ctx| {
                            workspace.open_notebook(
                                &NotebookSource::Existing(notebook_id),
                                &settings,
                                ctx,
                                false,
                            );
                        });
                    });
                }
                ObjectType::Workflow => {
                    handle.update(ctx, |workspace, ctx| {
                        let initialized_section_states =
                            workspace.has_warp_drive_initialized_sections(ctx);
                        let workflow_id = SyncId::ServerId(arg.server_id);
                        let settings = arg.settings.clone();
                        let _ = ctx.spawn(initialized_section_states, move |workspace, _, ctx| {
                            workspace.open_workflow_from_intent(workflow_id, &settings, ctx);
                        });
                    });
                }
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::EnvVarCollection,
                )) => {
                    if cloud_model.get_by_uid(&arg.server_id.uid()).is_none() {
                        display_object_missing_error_in_window(ctx.window_id(), ctx);
                        return false;
                    }

                    let item_id =
                        WarpDriveItemId::Object(CloudObjectTypeAndId::from_generic_string_object(
                            GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
                            SyncId::ServerId(arg.server_id),
                        ));

                    handle.update(ctx, |workspace, ctx| {
                        let initialized_section_states =
                            workspace.has_warp_drive_initialized_sections(ctx);
                        let _ = ctx.spawn(initialized_section_states, move |workspace, _, ctx| {
                            workspace.view_in_and_focus_warp_drive(item_id, ctx);
                        });
                    });
                }
                ObjectType::Folder => {
                    if cloud_model.get_by_uid(&arg.server_id.uid()).is_none() {
                        display_object_missing_error_in_window(ctx.window_id(), ctx);
                        return false;
                    }

                    let item_id = WarpDriveItemId::Object(CloudObjectTypeAndId::Folder(
                        SyncId::ServerId(arg.server_id),
                    ));
                    handle.update(ctx, |workspace, ctx| {
                        let initialized_section_states =
                            workspace.has_warp_drive_initialized_sections(ctx);
                        let _ = ctx.spawn(initialized_section_states, move |workspace, _, ctx| {
                            workspace.view_in_and_focus_warp_drive(item_id, ctx);
                        });
                    });
                }
                _ => {
                    log::info!(
                        "Object type {:?} not support yet for opening via link",
                        arg.object_type
                    )
                }
            }

            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
            ctx.notify();
        } else {
            log::warn!("Auth not complete before trying to open warp drive object");
        }
        true
    }

    pub fn join_shared_session_in_existing_window(
        &mut self,
        session_id: &SessionId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.add_tab_for_joining_shared_session(*session_id, ctx);
            });
            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
            ctx.notify();
            true
        } else {
            log::warn!("Auth not complete before trying to join shared session");
            false
        }
    }

    /// Opens a cloud conversation in an existing window.
    /// If the user owns the conversation, restores or navigates to it directly.
    /// Otherwise, opens a read-only transcript viewer.
    pub fn open_cloud_conversation_in_existing_window(
        &mut self,
        conversation_id: &ServerConversationToken,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.open_cloud_conversation_from_server_token(conversation_id.clone(), ctx);
            });
            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
            ctx.notify();
            true
        } else {
            log::warn!("Auth not complete before trying to open conversation viewer");
            false
        }
    }

    /// Adds a tab and starts the guided `/create-environment` setup flow.
    fn create_environment_in_existing_window(
        &mut self,
        arg: &CreateEnvironmentArg,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            let repos = arg.repos.clone();

            handle.update(ctx, |workspace, ctx| {
                workspace.add_tab_with_pane_layout(
                    PanesLayout::SingleTerminal(Box::default()),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                );

                workspace
                    .active_tab_pane_group()
                    .update(ctx, |pane_group, ctx| {
                        pane_group.set_title("Create Environment", ctx);

                        if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                            terminal_view.update(ctx, |_, ctx| {
                                ctx.dispatch_typed_action_deferred(
                                    TerminalAction::SetupCloudEnvironment(repos.clone()),
                                );
                            });
                        }
                    });
            });
            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
            ctx.notify();
            true
        } else {
            log::warn!("Auth not complete before trying to create environment");
            false
        }
    }

    /// Adds a tab and starts the guided `/create-environment` setup flow immediately.
    fn create_environment_in_existing_window_and_run(
        &mut self,
        arg: &CreateEnvironmentArg,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state else {
            log::warn!("Auth not complete before trying to create environment");
            return false;
        };

        let repos = arg.repos.clone();

        handle.update(ctx, |workspace, ctx| {
            workspace.add_tab_with_pane_layout(
                PanesLayout::SingleTerminal(Box::default()),
                Arc::new(HashMap::new()),
                None,
                ctx,
            );

            workspace
                .active_tab_pane_group()
                .update(ctx, |pane_group, ctx| {
                    pane_group.set_title("Create Environment", ctx);

                    if let Some(terminal_view) = pane_group.active_session_view(ctx) {
                        terminal_view.update(ctx, |_, ctx| {
                            ctx.dispatch_typed_action_deferred(
                                crate::terminal::view::TerminalAction::SetupCloudEnvironmentAndStart(
                                    repos.clone(),
                                ),
                            );
                        });
                    }
                });
        });

        let window_id = ctx.window_id();
        ctx.windows().show_window_and_focus_app(window_id);
        ctx.notify();
        true
    }

    pub fn add_file_pane(&mut self, path: &PathBuf, ctx: &mut ViewContext<Self>) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.add_tab_for_file_notebook(Some(path.to_owned()), ctx);
            });
            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
            ctx.notify();
        } else {
            log::warn!("Auth not complete before trying to open file pane");
        }
        true
    }

    /// Insert a command that should create a subshell. If we support bootstrapping AKA
    /// "warpifying" its [`ShellType`], set a flag to automatically bootstrap it when the command's
    /// block receives the [`AfterBlockStarted`] event.
    pub fn insert_subshell_command_and_bootstrap_if_supported(
        &mut self,
        arg: &SubshellCommandArg,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.insert_subshell_command_and_bootstrap_if_supported(
                    &arg.command,
                    arg.shell_type,
                    ctx,
                );
                ctx.windows().show_window_and_focus_app(window_id);
            })
        } else {
            log::warn!("Auth not complete before trying to fill input");
        }
        true
    }

    /// Shows the user the settings view of their newly joined team
    /// within the app.
    pub fn handle_team_intent_link_action(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        // Force-open warp drive.
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            ctx.dispatch_typed_action_for_view(
                window_id,
                handle.id(),
                &WorkspaceAction::OpenWarpDrive,
            );
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open warp drive");
        }

        // Use the team tester model to notify relevant subscribers to refresh their data.
        TeamTesterStatus::handle(ctx).update(ctx, |model, ctx| {
            model.initiate_data_pollers(true, ctx);
        });
        true
    }

    pub fn open_team_settings_page(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            ctx.dispatch_typed_action_for_view(
                window_id,
                handle.id(),
                &WorkspaceAction::ShowSettingsPage(SettingsSection::Teams),
            );
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open team settings page");
        }
        true
    }

    pub fn open_settings_page_in_existing_window(
        &mut self,
        section: &SettingsSection,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            ctx.dispatch_typed_action_for_view(
                window_id,
                handle.id(),
                &WorkspaceAction::ShowSettingsPage(*section),
            );
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open settings page {section:?}");
        }
        true
    }

    /// Opens the MCP servers settings page in an existing window, optionally triggering auto-install.
    /// Waits for `initial_load_complete` before opening so gallery data is available for autoinstall.
    pub fn open_mcp_settings_in_existing_window(
        &mut self,
        args: &OpenMCPSettingsArgs,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            let autoinstall = args.autoinstall.clone();
            let initial_load_complete = UpdateManager::as_ref(ctx).initial_load_complete();
            handle.update(ctx, |_, ctx| {
                let _ = ctx.spawn(initial_load_complete, move |workspace, _, ctx| {
                    workspace.open_mcp_servers_page(
                        MCPServersSettingsPage::List,
                        autoinstall.as_deref(),
                        ctx,
                    )
                });
            });
            let window_id = ctx.window_id();
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open MCP settings page");
        }
        true
    }

    /// Opens the Codex modal in an existing window.
    pub fn open_codex_in_existing_window(&mut self, _: &(), ctx: &mut ViewContext<Self>) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            handle.update(ctx, |workspace, ctx| {
                workspace.open_codex_modal(ctx);
            });
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open Codex modal");
        }
        true
    }

    /// Opens a new tab with agent view for a Linear issue work deeplink.
    pub fn open_linear_issue_work_in_existing_window(
        &mut self,
        args: &LinearIssueWork,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let window_id = ctx.window_id();
        if let AuthOnboardingState::Terminal(handle) = &self.auth_onboarding_state {
            let args = args.clone();
            handle.update(ctx, |workspace, ctx| {
                workspace.open_linear_issue_work(&args, ctx);
            });
            ctx.windows().show_window_and_focus_app(window_id);
        } else {
            log::error!("Auth not complete before trying to open Linear issue work");
        }
        true
    }

    /// Syncs the local "onboarding completed" flag to the server if the user
    /// finished onboarding pre-login and has since authenticated. Runs on every
    /// `AuthComplete`, so it also covers users who skipped login during onboarding
    /// and later signed up through a different entrypoint (e.g. login modal,
    /// settings, command palette) while already in the `Terminal` state.
    fn sync_local_onboarding_to_server(auth_state: &AuthState, ctx: &mut AppContext) {
        let is_onboarded = auth_state.is_onboarded().unwrap_or(true);
        let is_anonymous = auth_state.is_user_anonymous().unwrap_or(false);
        let has_completed_local_onboarding = has_completed_local_onboarding(ctx);

        if has_completed_local_onboarding && !is_onboarded && !is_anonymous {
            AuthManager::handle(ctx).update(ctx, |model, ctx| model.set_user_onboarded(ctx));
        }
    }

    fn handle_auth_manager_event(&mut self, event: &AuthManagerEvent, ctx: &mut ViewContext<Self>) {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        match event {
            AuthManagerEvent::AuthComplete => {
                self.paste_auth_token_modal = None;

                // If onboarding was completed pre-login, sync the server-side flag now
                // that the user is authenticated. This must happen regardless of the
                // current `auth_onboarding_state` so we also cover users who skipped
                // login during onboarding and later signed up from a different
                // entrypoint (i.e. we're already in the `Terminal` state).
                Self::sync_local_onboarding_to_server(&auth_state, ctx);

                // If the user needs SSO after auth is complete, no matter what their current state is,
                // we need to block their access to the rest of the app.
                if auth_state.needs_sso_link().unwrap_or(false) {
                    self.show_needs_sso_link_view(
                        auth_state.user_email().unwrap_or_default().clone(),
                        ctx,
                    );
                } else if let AuthOnboardingState::Auth(_)
                | AuthOnboardingState::ConfirmIncomingAuth(_) =
                    &self.auth_onboarding_state
                {
                    self.auth_view.update(ctx, |auth_view, ctx| {
                        auth_view.set_variant(ctx, AuthViewVariant::Initial);
                    });
                    self.auth_onboarding_state
                        .complete_auth_and_create_workspace(ctx);
                    self.start_pending_tutorial(ctx);
                } else if let AuthOnboardingState::LoginSlide { .. } = &self.auth_onboarding_state {
                    self.auth_onboarding_state
                        .complete_auth_and_create_workspace(ctx);
                    self.start_pending_tutorial(ctx);
                } else if let AuthOnboardingState::NeedsSsoLink { .. } = &self.auth_onboarding_state
                {
                    // We should be able to access their SSO state; if not, default to true,
                    // since we should err on the side of them _not_ being able to use Warp.
                    if auth_state.needs_sso_link() == Some(false) {
                        self.auth_onboarding_state.complete_sso_link(ctx);
                    }
                }

                #[cfg(target_family = "wasm")]
                if let AuthOnboardingState::WebImport(_) = &self.auth_onboarding_state {
                    self.auth_onboarding_state.complete_web_import(ctx);
                }

                // Skip onboarding survey if in Variant One.
                if let Some(BlockOnboarding::VariantOne) = BlockOnboarding::get_group(ctx) {
                    self.auth_onboarding_state
                        .complete_auth_and_create_workspace(ctx);
                }

                self.start_autoupdate_polling(ctx);
                self.focus(ctx);
            }
            AuthManagerEvent::AuthFailed(err) => match err {
                UserAuthenticationError::DeniedAccessToken(_) => {
                    // On the web, re-import the token from the host application, which should
                    // still be valid.
                    // On native, we show a banner in the app nudging them to do so, but don't
                    // actually log them out.
                    // That is handled in the workspace view.
                    #[cfg(target_family = "wasm")]
                    self.web_handoff(ctx);
                }
                UserAuthenticationError::UserAccountDisabled(_) => {
                    cfg_if! {
                        if #[cfg(target_family = "wasm")] {
                            // On the web, replace the invalid account with the one from the host
                            // application, which ought to be valid.
                            self.web_handoff(ctx);
                        } else {
                            // On native, force sign them out, as they should not be able to continue
                            // to use Warp. Instead, they can sign in or up with a valid account.
                            crate::auth::log_out(ctx);
                        }
                    }
                }
                UserAuthenticationError::Unexpected(err) => {
                    log::error!("Encountered unexpected error when trying to fetch user: {err:#}");
                }
                UserAuthenticationError::InvalidStateParameter => {}
                UserAuthenticationError::MissingStateParameter => {}
            },
            AuthManagerEvent::SkippedLogin => {
                if let AuthOnboardingState::Auth(_) | AuthOnboardingState::ConfirmIncomingAuth(_) =
                    &self.auth_onboarding_state
                {
                    self.auth_onboarding_state
                        .complete_auth_and_create_workspace(ctx);
                    self.start_pending_tutorial(ctx);
                } else if let AuthOnboardingState::LoginSlide { target, .. } =
                    &self.auth_onboarding_state
                {
                    let workspace = target.to_workspace(ctx);
                    if let Some(selected_settings) =
                        self.pending_post_auth_onboarding_settings.take()
                    {
                        // Skipped login → no account → AI disabled.
                        apply_onboarding_settings(&selected_settings, false, ctx);
                    }
                    self.auth_onboarding_state = AuthOnboardingState::Terminal(workspace);
                    ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                    self.start_pending_tutorial(ctx);
                }
                self.start_autoupdate_polling(ctx);
                self.focus(ctx);
            }
            AuthManagerEvent::LoginOverrideDetected(interrupted_auth_payload) => {
                match &self.auth_onboarding_state {
                    AuthOnboardingState::Auth(workspace_args)
                    | AuthOnboardingState::ConfirmIncomingAuth(workspace_args) => {
                        self.open_auth_override_warning_modal(
                            workspace_args.clone(),
                            interrupted_auth_payload.clone(),
                            ctx,
                        );
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_auth_override_warning_modal_event(
        &mut self,
        event: &AuthOverrideWarningModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AuthOverrideWarningModalEvent::Close => {
                if matches!(
                    self.auth_onboarding_state,
                    AuthOnboardingState::ConfirmIncomingAuth(_)
                ) {
                    self.log_out(&(), ctx);
                }
            }
            AuthOverrideWarningModalEvent::BulkExport => {
                self.export_all_warp_drive_objects(ctx);
            }
        }
    }

    fn open_auth_override_warning_modal(
        &mut self,
        workspace_args: Box<WorkspaceArgs>,
        auth_payload: AuthRedirectPayload,
        ctx: &mut ViewContext<Self>,
    ) {
        self.auth_override_view.update(ctx, |modal, _| {
            modal.set_interrupted_auth_payload(auth_payload);
        });
        self.auth_onboarding_state = AuthOnboardingState::ConfirmIncomingAuth(workspace_args);
        ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        self.focus(ctx);
        ctx.notify();
    }

    fn export_all_warp_drive_objects(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let cloud_model = CloudModel::as_ref(ctx);
        let exportable_objects = cloud_model.get_all_exportable_object_ids();
        ExportManager::handle(ctx).update(ctx, move |export_manager, ctx| {
            export_manager.export(window_id, &exportable_objects, ctx);
        });
    }

    /// This is called when importing authentication state from the host app completes.
    #[cfg(target_family = "wasm")]
    fn handle_web_handoff_event(
        &mut self,
        _view: ViewHandle<WebHandoffView>,
        event: &WebHandoffEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WebHandoffEvent::Unsupported => {
                log::warn!("Web auth handoff is unavailable");
                if let AuthOnboardingState::WebImport(target) = &self.auth_onboarding_state {
                    self.auth_onboarding_state = match target {
                        AuthOnboardingTarget::Workspace(args) => {
                            AuthOnboardingState::Auth(args.clone())
                        }
                        AuthOnboardingTarget::Terminal(view) => {
                            // If we're in this state, it means that refreshing the user's stored
                            // token failed _and_ handoff is unavailable. Return to the workspace
                            // view with an error banner.
                            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                                auth_manager.set_needs_reauth(true, ctx);
                            });
                            AuthOnboardingState::Terminal(view.clone())
                        }
                    };
                    ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                } else {
                    log::error!("Received web handoff event in unexpected state");
                }
                self.focus(ctx);
            }
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if let Some(modal) = &self.paste_auth_token_modal {
            ctx.focus(modal);
            ctx.notify();
            return true;
        }
        match &self.auth_onboarding_state {
            AuthOnboardingState::Auth(_) => {
                ctx.focus(&self.auth_view);
            }
            AuthOnboardingState::ConfirmIncomingAuth(_) => {
                ctx.focus(&self.auth_override_view);
            }
            #[cfg(target_family = "wasm")]
            AuthOnboardingState::WebImport(_) => {
                ctx.focus(&self.web_handoff_view);
            }
            AuthOnboardingState::NeedsSsoLink { .. } => {
                ctx.focus(&self.needs_sso_link_view);
            }
            AuthOnboardingState::Onboarding {
                onboarding_view, ..
            } => {
                ctx.focus(onboarding_view);
            }
            AuthOnboardingState::LoginSlide {
                login_slide_view, ..
            } => {
                ctx.focus(login_slide_view);
            }
            AuthOnboardingState::Terminal(workspace) => {
                ctx.focus(workspace);
            }
        }
        ctx.notify();
        true
    }

    /// Stops active voice input, if the configured voice input toggle key is released.
    #[cfg(feature = "voice_input")]
    fn maybe_stop_active_voice_input(
        &mut self,
        key_code: &warpui::platform::keyboard::KeyCode,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        use voice_input::{VoiceInput, VoiceInputState, VoiceInputToggledFrom};
        use warpui::event::KeyState;

        use crate::settings::AISettings;

        // Check that the released key matches the configured voice input toggle key.
        let ai_settings = AISettings::as_ref(ctx);
        if let Some(configured_key_code) = ai_settings.voice_input_toggle_key.value().to_key_code()
        {
            if configured_key_code == *key_code {
                let voice_input = VoiceInput::handle(ctx);
                // Check if we're actively listening and it was started from a key press.
                if let VoiceInputState::Listening { enabled_from, .. } =
                    voice_input.as_ref(ctx).state()
                {
                    if matches!(
                        enabled_from,
                        VoiceInputToggledFrom::Key {
                            state: KeyState::Pressed
                        }
                    ) {
                        log::debug!("Voice input key release detected: {key_code:?}");
                        // Stop listening and proceed to transcription (don't abort).
                        voice_input.update(ctx, |voice_input, ctx| {
                            if let Err(e) = voice_input.stop_listening(ctx) {
                                log::error!("Failed to stop voice input on key release: {e:?}");
                            }
                        });
                    }
                }
            }
        }
        true
    }

    /// If onboarding stashed `SelectedSettings` to be applied after auth + the
    /// initial cloud-pref sync, drain the stash and apply now.
    ///
    /// Mirrors `start_pending_tutorial` in shape but triggers on a later event:
    /// `CloudPreferencesSyncerEvent::InitialLoadCompleted` fires once
    /// `handle_initial_load` has finished reconciling cloud→local, so any
    /// writes we make here are the last writes and won't be clobbered by that
    /// pass. By this point the user is also logged in, so AIExecutionProfile
    /// edits can successfully create cloud objects via `edit_profile_internal`.
    fn handle_cloud_preferences_syncer_event(
        &mut self,
        event: &CloudPreferencesSyncerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if !matches!(event, CloudPreferencesSyncerEvent::InitialLoadCompleted) {
            return;
        }
        let Some(selected_settings) = self.pending_post_auth_onboarding_settings.take() else {
            return;
        };
        // Reached only after a successful login, so the user has an account.
        apply_onboarding_settings(&selected_settings, true, ctx);
    }

    /// If onboarding stored a pending tutorial (because login was required first),
    /// start it now that the workspace exists.
    fn start_pending_tutorial(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(tutorial) = self.pending_tutorial.take() else {
            return;
        };

        let AuthOnboardingState::Terminal(workspace) = &self.auth_onboarding_state else {
            return;
        };

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
            && FeatureFlag::TabConfigs.is_enabled()
        {
            let intention = tutorial.intention();
            if matches!(intention, OnboardingIntention::AgentDrivenDevelopment) {
                workspace.update(ctx, |view, ctx| {
                    view.open_vertical_tabs_panel_if_enabled(ctx);
                    view.start_agent_onboarding_tutorial(tutorial, ctx);
                });
            } else {
                workspace.update(ctx, |view, ctx| {
                    view.open_vertical_tabs_panel_if_enabled(ctx);
                });
            }
        } else if *AISettings::as_ref(ctx).is_any_ai_enabled {
            workspace.update(ctx, |view, ctx| {
                view.start_agent_onboarding_tutorial(tutorial, ctx);
            });
        }
    }

    fn traffic_light_data(&self, ctx: &AppContext) -> Option<TrafficLightData> {
        // The workspace view will handle rendering of the traffic lights (so
        // that they can be hidden when the tab bar is hidden).
        if matches!(self.auth_onboarding_state, AuthOnboardingState::Terminal(_)) {
            return None;
        }

        traffic_light_data(ctx, self.window_id)
    }
}

#[derive(Clone, Debug)]
pub enum RootViewEvent {
    AuthOnboardingStateChanged,
}

impl Entity for RootView {
    type Event = RootViewEvent;
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        } else if self.paste_auth_token_modal.is_some() {
            // Modal is open — focus belongs to the editor inside it.
        } else if matches!(
            self.auth_onboarding_state,
            AuthOnboardingState::Onboarding { .. }
        ) {
            // During onboarding, aggressively redirect focus.
            // This ensures keystrokes (Enter) are handled by the correct view rather
            // than something hidden like the input editor.
            self.focus(ctx);
        } else if let AuthOnboardingState::LoginSlide {
            login_slide_view, ..
        } = &self.auth_onboarding_state
        {
            // Redirect focus unless the auth token editor is visible and should
            // accept user input.
            if !login_slide_view.as_ref(ctx).is_auth_token_input_visible() {
                self.focus(ctx);
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let child = match &self.auth_onboarding_state {
            AuthOnboardingState::Auth(_) => ChildView::new(&self.auth_view).finish(),
            AuthOnboardingState::ConfirmIncomingAuth(_) => {
                ChildView::new(&self.auth_override_view).finish()
            }
            #[cfg(target_family = "wasm")]
            AuthOnboardingState::WebImport(_) => ChildView::new(&self.web_handoff_view).finish(),
            AuthOnboardingState::NeedsSsoLink { .. } => {
                ChildView::new(&self.needs_sso_link_view).finish()
            }
            AuthOnboardingState::Onboarding {
                onboarding_view, ..
            } => ChildView::new(onboarding_view).finish(),
            AuthOnboardingState::LoginSlide {
                login_slide_view, ..
            } => ChildView::new(login_slide_view).finish(),
            AuthOnboardingState::Terminal(workspace) => ChildView::new(workspace).finish(),
        };

        let mut stack = Stack::new();
        stack.add_child(child);

        if let Some(modal) = &self.paste_auth_token_modal {
            stack.add_child(ChildView::new(modal).finish());
        }

        if let Some(traffic_light_data) = self.traffic_light_data(app) {
            let theme = Appearance::as_ref(app).theme();
            let fullscreen_state = app
                .windows()
                .platform_window(self.window_id)
                .map(|window| window.fullscreen_state())
                .unwrap_or_default();
            stack.add_positioned_child(
                traffic_light_data.render(fullscreen_state, &self.mouse_states, theme, app),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        cfg_if::cfg_if! {
            if #[cfg(feature = "voice_input")] {
                use warpui::elements::{EventHandler, DispatchEventResult};
                EventHandler::new(stack.finish())
                    .on_modifier_state_changed(|ctx, _app, key_code, key_state| {
                        if matches!(key_state, warpui::event::KeyState::Released) {
                            ctx.dispatch_action("root_view:maybe_stop_active_voice_input", *key_code);
                        }
                        DispatchEventResult::PropagateToParent
                    })
                    .finish()
            } else {
                stack.finish()
            }
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if quake_mode_window_is_open() {
            context.set.insert(flags::QUAKE_WINDOW_OPEN_FLAG);
        }
        if *KeysSettings::as_ref(app).quake_mode_enabled {
            context.set.insert(flags::QUAKE_MODE_ENABLED_CONTEXT_FLAG);
        }
        if *KeysSettings::as_ref(app).activation_hotkey_enabled.value() {
            context.set.insert(flags::ACTIVATION_HOTKEY_FLAG);
        }
        context
    }
}

#[derive(Clone, Debug)]
pub enum RootViewAction {
    ToggleQuakeModeWindow,
    ShowOrHideNonQuakeModeWindows,
    ToggleFullscreen,
    DebugEnterOnboardingState,
}

impl TypedActionView for RootView {
    type Action = RootViewAction;
    fn handle_action(&mut self, action: &RootViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            RootViewAction::ToggleQuakeModeWindow => {
                let global_resource_handles =
                    GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
                toggle_quake_mode_window(&global_resource_handles, ctx)
            }
            RootViewAction::ShowOrHideNonQuakeModeWindows => {
                show_or_hide_non_quake_mode_windows(&(), ctx)
            }
            RootViewAction::ToggleFullscreen => {
                let window_id = ctx.window_id();
                WindowManager::handle(ctx).update(ctx, |state, ctx| {
                    state.toggle_fullscreen(window_id, ctx);
                });
            }
            RootViewAction::DebugEnterOnboardingState => {
                self.debug_enter_onboarding_state(&(), ctx);
            }
        }
    }
}

impl WorkspaceArgs {
    fn create_workspace(self, ctx: &mut ViewContext<RootView>) -> ViewHandle<Workspace> {
        ctx.add_typed_action_view(|ctx| {
            Workspace::new(
                self.global_resource_handles,
                self.server_time,
                self.workspace_setting,
                ctx,
            )
        })
    }
}

impl AuthOnboardingState {
    fn complete_auth_and_create_workspace(&mut self, ctx: &mut ViewContext<RootView>) {
        // Check if we should show onboarding (only for users who are not yet onboarded).
        // The server-side `is_onboarded` flag is synced separately by
        // `RootView::sync_local_onboarding_to_server`, which runs on every `AuthComplete`
        // before we get here.
        let auth_state = AuthStateProvider::as_ref(ctx).get();
        let is_onboarded = auth_state.is_onboarded().unwrap_or(true);
        let is_anonymous = auth_state.is_user_anonymous().unwrap_or(false);

        let has_completed_local_onboarding = has_completed_local_onboarding(ctx);

        if !is_onboarded
            && !is_anonymous
            && !has_completed_local_onboarding
            && FeatureFlag::AgentOnboarding.is_enabled()
        {
            self.try_open_onboarding_slides(ctx);
        }

        // If we didn't transition to Onboarding, set the Terminal state.
        match self {
            AuthOnboardingState::Auth(ref args)
            | AuthOnboardingState::ConfirmIncomingAuth(ref args) => {
                let workspace = args.clone().create_workspace(ctx);
                *self = AuthOnboardingState::Terminal(workspace);
            }
            AuthOnboardingState::LoginSlide { ref target, .. } => {
                let workspace = target.to_workspace(ctx);
                *self = AuthOnboardingState::Terminal(workspace);
            }
            _ => {}
        };
        ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
    }

    fn try_open_onboarding_slides(&mut self, ctx: &mut ViewContext<RootView>) {
        let target = match self {
            AuthOnboardingState::Auth(args) | AuthOnboardingState::ConfirmIncomingAuth(args) => {
                AuthOnboardingTarget::Workspace(args.clone())
            }
            AuthOnboardingState::Terminal(workspace) => {
                AuthOnboardingTarget::Terminal(workspace.clone())
            }
            _ => {
                // Onboarding slides can only be opened from Auth or Terminal states
                return;
            }
        };

        let onboarding_view = RootView::create_agent_onboarding_view(ctx);
        onboarding_view.update(ctx, |view, ctx| {
            view.start_onboarding(ctx);
        });
        *self = AuthOnboardingState::Onboarding {
            onboarding_view,
            target,
        };
    }

    fn complete_sso_link(&mut self, ctx: &mut ViewContext<RootView>) {
        if let AuthOnboardingState::NeedsSsoLink(needs_sso_link_mode) = self {
            *self = AuthOnboardingState::Terminal(needs_sso_link_mode.to_workspace(ctx));
            ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        }
    }

    #[cfg(target_family = "wasm")]
    fn show_web_handoff_view(&mut self) {
        match self {
            AuthOnboardingState::Auth(args) | AuthOnboardingState::ConfirmIncomingAuth(args) => {
                *self =
                    AuthOnboardingState::WebImport(AuthOnboardingTarget::Workspace(args.clone()));
            }
            AuthOnboardingState::WebImport(_) => (),
            AuthOnboardingState::NeedsSsoLink(target) => {
                *self = AuthOnboardingState::WebImport(target.clone())
            }
            AuthOnboardingState::Onboarding { .. } | AuthOnboardingState::LoginSlide { .. } => {
                // For onboarding/login slide, we don't have a workspace yet, so we can't convert to web import
                // This case shouldn't normally occur
            }
            AuthOnboardingState::Terminal(view) => {
                *self = AuthOnboardingState::WebImport(AuthOnboardingTarget::Terminal(view.clone()))
            }
        }
    }

    #[cfg(target_family = "wasm")]
    fn complete_web_import(&mut self, ctx: &mut ViewContext<RootView>) {
        if let AuthOnboardingState::WebImport(target) = self {
            *self = AuthOnboardingState::Terminal(target.to_workspace(ctx));
            ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
        }
    }

    fn show_needs_sso_link_view(&mut self) {
        match self {
            AuthOnboardingState::Auth(workspace_args)
            | AuthOnboardingState::ConfirmIncomingAuth(workspace_args) => {
                *self = AuthOnboardingState::NeedsSsoLink(AuthOnboardingTarget::Workspace(
                    workspace_args.clone(),
                ))
            }
            #[cfg(target_family = "wasm")]
            AuthOnboardingState::WebImport(_) => {
                // This case _shouldn't_ be possible - if SSO were required, it should be handled
                // in the host app.
                log::error!("SSO link required after web user import");
            }
            AuthOnboardingState::NeedsSsoLink { .. } => (),
            AuthOnboardingState::Onboarding { .. } | AuthOnboardingState::LoginSlide { .. } => {
                // For onboarding/login slide, we don't have a workspace yet, so we can't convert to SSO link
                // This case shouldn't normally occur
            }
            AuthOnboardingState::Terminal(terminal_view_handle) => {
                *self = AuthOnboardingState::NeedsSsoLink(AuthOnboardingTarget::Terminal(
                    terminal_view_handle.clone(),
                ))
            }
        }
    }

    fn log_out(&mut self, ctx: &mut ViewContext<RootView>) {
        match self {
            AuthOnboardingState::Auth(_) => (),
            AuthOnboardingState::ConfirmIncomingAuth(workspace_args) => {
                *self = AuthOnboardingState::Auth(workspace_args.clone());
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
            }
            #[cfg(target_family = "wasm")]
            AuthOnboardingState::WebImport(_) => {
                // TODO(ben): Eventually, we could support logout here by logging out of the JS
                // Firebase client.
            }
            AuthOnboardingState::NeedsSsoLink(needs_sso_link_mode) => match needs_sso_link_mode {
                AuthOnboardingTarget::Workspace(args) => {
                    *self = AuthOnboardingState::Auth(args.clone());
                    ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
                }
                AuthOnboardingTarget::Terminal(_) => {}
            },
            AuthOnboardingState::Onboarding { .. } | AuthOnboardingState::LoginSlide { .. } => {
                // No workspace to clean up for onboarding/login slide state
            }
            AuthOnboardingState::Terminal(workspace) => {
                // Clean up current workspace before resetting.
                workspace.update(ctx, |workspace, ctx| {
                    workspace.on_log_out(ctx);
                });

                let global_resource_handles =
                    GlobalResourceHandlesProvider::as_ref(ctx).get().clone();
                // When a user logs out, reset workspace_setting so user logs back into a
                // fresh workspace.
                let workspace_setting = NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                };
                let workspace_args = WorkspaceArgs {
                    global_resource_handles,
                    server_time: None,
                    workspace_setting,
                };

                // Auth no longer holds the original workspace view handle
                // This way it is destroyed at this step, and we will re-create
                // a new workspace view handle when the user logs in.
                *self = AuthOnboardingState::Auth(workspace_args.into());
                ctx.emit(RootViewEvent::AuthOnboardingStateChanged);
            }
        }
    }
}

impl AuthOnboardingTarget {
    fn to_workspace(&self, ctx: &mut ViewContext<RootView>) -> ViewHandle<Workspace> {
        match self {
            AuthOnboardingTarget::Terminal(workspace) => workspace.clone(),
            AuthOnboardingTarget::Workspace(args) => args.clone().create_workspace(ctx),
        }
    }
}

#[cfg(test)]
#[path = "root_view_tests.rs"]
mod tests;
