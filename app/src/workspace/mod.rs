mod action;
mod active_session;
pub mod bonus_grant_notification_model;
#[cfg(target_os = "macos")]
mod cli_install;
mod close_session_confirmation_dialog;
pub(crate) mod cross_window_tab_drag;
pub mod delete_conversation_confirmation_dialog;
mod global_actions;
pub mod header_toolbar_editor;
pub mod header_toolbar_item;
pub mod hoa_onboarding;
mod home;
mod lightbox_view;
mod native_modal;
mod one_time_modal_model;
mod registry;
pub mod rewind_confirmation_dialog;
pub mod sync_inputs;
pub mod tab_settings;
mod toast_stack;
pub mod util;
pub mod view;

use crate::ai::blocklist::NEW_AGENT_PANE_LABEL;
use crate::ai::skills::SkillManager;
use crate::ai::AIRequestUsageModel;
use crate::channel::Channel;
use crate::code;
use crate::features::FeatureFlag;
use crate::modal;
use crate::notebooks;
use crate::pane_group::TabBarHoverIndex;
use crate::server::telemetry::AgentModeEntrypoint;
use crate::server::telemetry::PaletteSource;
use crate::settings::AISettings;
use crate::settings_view::{self, flags, SettingsSection};
use crate::tab::uses_vertical_tabs;
use crate::tab_configs;
use warpui::SingletonEntity;

use crate::channel::ChannelState;

use crate::util::bindings::{self, cmd_or_ctrl_shift, is_binding_pty_compliant, CustomAction};

use crate::palette::PaletteMode;
use serde::{Deserialize, Serialize};
use warp_core::context_flag::ContextFlag;
use warpui::accessibility::AccessibilityVerbosity;
use warpui::elements::DropTargetData;
use warpui::keymap::FixedBinding;
use warpui::keymap::{BindingDescription, EditableBinding};
use warpui::AppContext;

pub use action::{
    CommandSearchOptions, InitContent, RestoreConversationLayout, TabContextMenuAnchor,
    VerticalTabsPaneContextMenuTarget, WorkspaceAction,
};
pub use active_session::ActiveSession;
pub use global_actions::{
    ForkAIConversationParams, ForkFromExchange, ForkedConversationDestination,
};
pub use util::{active_terminal_in_window, PaneViewLocator, TabMovement};
pub use view::{
    Workspace, NEW_SESSION_MENU_BUTTON_POSITION_ID, NEW_TAB_BUTTON_POSITION_ID,
    PANEL_HEADER_HEIGHT, TAB_BAR_HEIGHT, TOTAL_TAB_BAR_HEIGHT, WORKSPACE_PADDING,
};

// Helper function to access panel header corner radius from other modules
pub fn panel_header_corner_radius() -> warpui::elements::CornerRadius {
    warpui::elements::CornerRadius::with_top(warpui::elements::Radius::Pixels(8.))
}

/// Returns `true` when `WorkspaceAction::SendFeedback` will launch the guided
/// feedback skill in a new agent pane. When `false`, the action falls back to
/// opening the GitHub issue form in the browser.
///
/// Kept in sync with the availability check in `Workspace::send_feedback` so
/// the command palette label and the menu item behavior never diverge.
pub fn is_feedback_skill_available(ctx: &AppContext) -> bool {
    AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
        && AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx)
        && SkillManager::as_ref(ctx)
            .active_bundled_skill("feedback", ctx)
            .is_some()
}

use crate::workspace::view::{
    LEFT_PANEL_AGENT_CONVERSATIONS_BINDING_NAME, LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME,
    LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME, LEFT_PANEL_WARP_DRIVE_BINDING_NAME,
    NEW_AGENT_TAB_BINDING_NAME, NEW_AMBIENT_AGENT_TAB_BINDING_NAME, NEW_TAB_BINDING_NAME,
    NEW_TERMINAL_TAB_BINDING_NAME, OPEN_GLOBAL_SEARCH_BINDING_NAME,
    TOGGLE_CONVERSATION_LIST_VIEW_BINDING_NAME, TOGGLE_NOTIFICATION_MAILBOX_BINDING_NAME,
    TOGGLE_PROJECT_EXPLORER_BINDING_NAME, TOGGLE_RIGHT_PANEL_BINDING_NAME,
    TOGGLE_TAB_CONFIGS_MENU_BINDING_NAME, TOGGLE_VERTICAL_TABS_PANEL_BINDING_NAME,
    TOGGLE_WARP_DRIVE_BINDING_NAME,
};
pub use one_time_modal_model::OneTimeModalModel;
pub use registry::WorkspaceRegistry;
pub use toast_stack::ToastStack;

pub fn init(app: &mut AppContext) {
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(|_| cross_window_tab_drag::CrossWindowTabDrag::new());
    use warpui::keymap::macros::*;
    app.register_binding_validator::<Workspace>(is_binding_pty_compliant);

    modal::init(app);
    native_modal::init(app);
    lightbox_view::init(app);
    rewind_confirmation_dialog::init(app);
    delete_conversation_confirmation_dialog::init(app);
    crate::tab_configs::remove_confirmation_dialog::init(app);
    hoa_onboarding::init(app);
    tab_configs::session_config_modal::init(app);
    view::launch_modal::oz_launch::init(app);
    view::openwarp_launch_modal::init(app);
    view::orchestration_launch_modal::init(app);
    view::cloud_agent_capacity_modal::init(app);
    view::codex_modal::init(app);
    view::free_tier_limit_hit_modal::init(app);
    view::global_search::view::GlobalSearchView::init(app);
    view::right_panel::RightPanelView::init(app);
    header_toolbar_editor::init(app);
    view::conversation_list::view::register_conversation_list_view_bindings(app);

    settings_view::init_actions_from_parent_view(app, &id!("Workspace"), |settings_action| {
        WorkspaceAction::DispatchToSettingsTab(settings_action)
    });
    global_actions::init_global_actions(app);
    notebooks::init(app);
    code::init(app);
    sync_inputs::init(app);
    lsp::init(app);

    app.register_fixed_bindings([FixedBinding::empty(
        "Dump debug info",
        WorkspaceAction::DumpDebugInfo,
        id!("Workspace"),
    )]);
    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            WorkspaceAction::DismissSessionConfigTabConfigChip,
            id!("Workspace") & id!(flags::SESSION_CONFIG_TAB_CONFIG_CHIP_OPEN),
        ),
        FixedBinding::new(
            "enter",
            WorkspaceAction::DismissSessionConfigTabConfigChip,
            id!("Workspace") & id!(flags::SESSION_CONFIG_TAB_CONFIG_CHIP_OPEN),
        ),
    ]);

    if ChannelState::enable_debug_features() {
        let crash_description = if cfg!(target_os = "macos") {
            "Crash the app (for testing sentry-cocoa)"
        } else {
            "Crash the app (for testing sentry-native)"
        };
        app.register_editable_bindings([
            EditableBinding::new("workspace:crash", crash_description, WorkspaceAction::Crash)
                .with_context_predicate(id!("Workspace")),
            EditableBinding::new(
                "workspace:log_review_comment_send_status_for_active_tab",
                "[Debug] Log review comment send status for active tab",
                WorkspaceAction::LogReviewCommentSendStatusForActiveTab,
            )
            .with_context_predicate(id!("Workspace")),
            EditableBinding::new(
                "workspace:panic",
                "Trigger a panic (for testing sentry-rust)",
                WorkspaceAction::Panic,
            )
            .with_context_predicate(id!("Workspace")),
            EditableBinding::new(
                "workspace:open_view_tree_debug_view",
                "Open view tree debugger",
                WorkspaceAction::OpenViewTreeDebugWindow,
            )
            .with_context_predicate(id!("Workspace")),
        ]);
        app.register_fixed_bindings([FixedBinding::empty(
            "[Debug] View first-time user experience",
            WorkspaceAction::AddGetStartedTab,
            id!("Workspace"),
        )]);
        #[cfg(debug_assertions)]
        {
            // Debug actions for build plan migration modal (command palette only)
            app.register_editable_bindings([
                EditableBinding::new(
                    "workspace:open_build_plan_migration_modal",
                    "[Debug] Open Build Plan Migration Modal",
                    WorkspaceAction::OpenBuildPlanMigrationModal,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:reset_build_plan_migration_modal_state",
                    "[Debug] Reset Build Plan Migration Modal State",
                    WorkspaceAction::ResetBuildPlanMigrationModalState,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:debug_reset_aws_bedrock_login_banner_dismissed",
                    "[Debug] Un-dismiss AWS login banner",
                    WorkspaceAction::DebugResetAwsBedrockLoginBannerDismissed,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:open_oz_launch_modal",
                    "[Debug] Open Oz Launch Modal",
                    WorkspaceAction::OpenOzLaunchModal,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:reset_oz_launch_modal_state",
                    "[Debug] Reset Oz Launch Modal State",
                    WorkspaceAction::ResetOzLaunchModalState,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:open_openwarp_launch_modal",
                    "[Debug] Open OpenWarp Launch Modal",
                    WorkspaceAction::OpenOpenWarpLaunchModal,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:reset_openwarp_launch_modal_state",
                    "[Debug] Reset OpenWarp Launch Modal State",
                    WorkspaceAction::ResetOpenWarpLaunchModalState,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:open_orchestration_launch_modal",
                    "[Debug] Open Orchestration Launch Modal",
                    WorkspaceAction::OpenOrchestrationLaunchModal,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:reset_orchestration_launch_modal_state",
                    "[Debug] Reset Orchestration Launch Modal State",
                    WorkspaceAction::ResetOrchestrationLaunchModalState,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:install_opencode_warp_plugin",
                    "[Debug] Install OpenCode Warp plugin",
                    WorkspaceAction::InstallOpenCodeWarpPlugin,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:use_local_opencode_warp_plugin",
                    "[Debug] Use local OpenCode Warp plugin (testing only)",
                    WorkspaceAction::UseLocalOpenCodeWarpPlugin,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:open_session_config_modal",
                    "[Debug] Open Session Config Modal",
                    WorkspaceAction::ShowSessionConfigModal,
                )
                .with_context_predicate(id!("Workspace")),
                EditableBinding::new(
                    "workspace:show_hoa_onboarding_flow",
                    "[Debug] Start HOA Onboarding Flow",
                    WorkspaceAction::ShowHoaOnboardingFlow,
                )
                .with_context_predicate(id!("Workspace")),
            ]);
        }
    }

    #[cfg(target_os = "macos")]
    app.register_editable_bindings([EditableBinding::new(
        "workspace:sample_process",
        "Sample Process",
        WorkspaceAction::SampleProcess,
    )
    .with_context_predicate(id!("Workspace"))]);

    #[cfg(feature = "dhat_heap_profiling")]
    {
        app.register_editable_bindings([EditableBinding::new(
            "workspace:dump_heap_profile",
            "Dump heap profile (can only be done once)",
            WorkspaceAction::DumpHeapProfile,
        )
        .with_context_predicate(id!("Workspace"))]);
    }

    app.register_fixed_bindings([
        FixedBinding::custom(
            CustomAction::CycleNextSession,
            WorkspaceAction::CycleNextSession,
            "Switch to next tab",
            id!("Workspace") & id!("Workspace_MultipleTabs"),
        ),
        FixedBinding::custom(
            CustomAction::CyclePrevSession,
            WorkspaceAction::CyclePrevSession,
            "Switch to previous tab",
            id!("Workspace") & id!("Workspace_MultipleTabs"),
        ),
        FixedBinding::custom(
            CustomAction::AddWindow,
            WorkspaceAction::AddWindow,
            "Create New Window",
            id!("Workspace"),
        )
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        FixedBinding::custom(
            CustomAction::NewFile,
            WorkspaceAction::NewCodeFile,
            "New File",
            id!("Workspace") & !id!("Workspace_ViewOnlySharedSession"),
        ),
    ]);

    if FeatureFlag::UIZoom.is_enabled() {
        app.register_fixed_bindings([
            FixedBinding::custom(
                CustomAction::IncreaseZoom,
                WorkspaceAction::IncreaseZoom,
                "Zoom In",
                id!("Workspace"),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
            FixedBinding::custom(
                CustomAction::DecreaseZoom,
                WorkspaceAction::DecreaseZoom,
                "Zoom Out",
                id!("Workspace"),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
            FixedBinding::custom(
                CustomAction::ResetZoom,
                WorkspaceAction::ResetZoom,
                "Reset Zoom",
                id!("Workspace"),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
        ]);
    } else {
        app.register_fixed_bindings([
            FixedBinding::custom(
                CustomAction::IncreaseFontSize,
                WorkspaceAction::IncreaseFontSize,
                "Increase font size",
                id!("Workspace"),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
            FixedBinding::custom(
                CustomAction::DecreaseFontSize,
                WorkspaceAction::DecreaseFontSize,
                "Decrease font size",
                id!("Workspace"),
            )
            .with_group(bindings::BindingGroup::Settings.as_str()),
        ]);
    }

    if ContextFlag::LaunchConfigurations.is_enabled() {
        app.register_fixed_bindings([FixedBinding::custom(
            CustomAction::SaveCurrentConfig,
            WorkspaceAction::OpenLaunchConfigSaveModal,
            "Save new launch configuration",
            id!("Workspace"),
        )]);
    }

    if ChannelState::channel() == Channel::Integration {
        // Hack: Add explicit bindings for the tests, since the tests' injected
        // keypresses won't trigger Mac menu items. Unfortunately we can't use
        // cfg[test] because we are a separate process!
        app.register_fixed_bindings([
            FixedBinding::new(
                cmd_or_ctrl_shift("t"),
                WorkspaceAction::AddDefaultTab,
                id!("Workspace"),
            ),
            FixedBinding::new(
                cmd_or_ctrl_shift("p"),
                WorkspaceAction::TogglePalette {
                    mode: PaletteMode::Command,
                    source: PaletteSource::IntegrationTest,
                },
                id!("Workspace"),
            ),
            FixedBinding::new(
                "cmdorctrl-,",
                WorkspaceAction::ShowSettings,
                id!("Workspace"),
            ),
        ]);
    }

    if FeatureFlag::UIZoom.is_enabled() {
        app.register_editable_bindings([
            EditableBinding::new(
                "workspace:increase_zoom",
                "Increase zoom level",
                WorkspaceAction::IncreaseZoom,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("cmdorctrl-="),
            EditableBinding::new(
                "workspace:decrease_zoom",
                "Decrease zoom level",
                WorkspaceAction::DecreaseZoom,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("cmdorctrl--"),
            EditableBinding::new(
                "workspace:reset_zoom",
                "Reset zoom level to default",
                WorkspaceAction::ResetZoom,
            )
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_context_predicate(id!("Workspace")),
            EditableBinding::new(
                "workspace:increase_font_size",
                "Increase font size",
                WorkspaceAction::IncreaseFontSize,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("alt-shift->"),
            EditableBinding::new(
                "workspace:decrease_font_size",
                "Decrease font size",
                WorkspaceAction::DecreaseFontSize,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("alt-shift-<"),
            EditableBinding::new(
                "workspace:reset_font_size",
                "Reset font size to default",
                WorkspaceAction::ResetFontSize,
            )
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_context_predicate(id!("Workspace")),
        ]);
    } else {
        app.register_editable_bindings([
            EditableBinding::new(
                "workspace:increase_font_size",
                "Increase font size",
                WorkspaceAction::IncreaseFontSize,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("cmdorctrl-="),
            EditableBinding::new(
                "workspace:decrease_font_size",
                "Decrease font size",
                WorkspaceAction::DecreaseFontSize,
            )
            .with_context_predicate(id!("Workspace"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_key_binding("cmdorctrl--"),
            EditableBinding::new(
                "workspace:reset_font_size",
                "Reset font size to default",
                WorkspaceAction::ResetFontSize,
            )
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_context_predicate(id!("Workspace"))
            .with_key_binding("cmdorctrl-0")
            .with_custom_action(CustomAction::ResetFontSize),
        ]);
    }

    app.register_fixed_bindings([
        // Menu dispatch for the "Open File Picker" custom action.
        FixedBinding::custom(
            CustomAction::ToggleProjectExplorer,
            WorkspaceAction::ToggleProjectExplorer,
            BindingDescription::new("Toggle project explorer")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Project Explorer"),
            id!("Workspace") & id!(flags::SHOW_PROJECT_EXPLORER),
        ),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:show_theme_chooser",
            "Open theme picker",
            WorkspaceAction::ShowThemeChooserForActiveTheme,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Settings.as_str()),
        EditableBinding::new(
            TOGGLE_TAB_CONFIGS_MENU_BINDING_NAME,
            "Open tab configs menu",
            WorkspaceAction::ToggleTabConfigsMenu,
        )
        .with_context_predicate(id!("Workspace"))
        .with_mac_key_binding("cmd-ctrl-t")
        .with_linux_or_windows_key_binding("ctrl-alt-shift-T"),
        EditableBinding::new(
            "workspace:activate_first_tab",
            "Switch to 1st tab",
            WorkspaceAction::ActivateTabByNumber(1),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-1"),
        EditableBinding::new(
            "workspace:activate_second_tab",
            "Switch to 2nd tab",
            WorkspaceAction::ActivateTabByNumber(2),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-2"),
        EditableBinding::new(
            "workspace:activate_third_tab",
            "Switch to 3rd tab",
            WorkspaceAction::ActivateTabByNumber(3),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-3"),
        EditableBinding::new(
            "workspace:activate_fourth_tab",
            "Switch to 4th tab",
            WorkspaceAction::ActivateTabByNumber(4),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-4"),
        EditableBinding::new(
            "workspace:activate_fifth_tab",
            "Switch to 5th tab",
            WorkspaceAction::ActivateTabByNumber(5),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-5"),
        EditableBinding::new(
            "workspace:activate_sixth_tab",
            "Switch to 6th tab",
            WorkspaceAction::ActivateTabByNumber(6),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-6"),
        EditableBinding::new(
            "workspace:activate_seventh_tab",
            "Switch to 7th tab",
            WorkspaceAction::ActivateTabByNumber(7),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-7"),
        EditableBinding::new(
            "workspace:activate_eighth_tab",
            "Switch to 8th tab",
            WorkspaceAction::ActivateTabByNumber(8),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-8"),
        EditableBinding::new(
            "workspace:activate_last_tab",
            "Switch to last tab",
            WorkspaceAction::ActivateLastTab,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_key_binding("cmdorctrl-9"),
        EditableBinding::new(
            "workspace:activate_prev_tab",
            "Activate previous tab",
            WorkspaceAction::ActivatePrevTab,
        )
        .with_context_predicate(
            id!("Workspace") & id!("Workspace_MultipleTabs") & !id!("Workspace_PaneDragging"),
        )
        .with_mac_key_binding("shift-cmd-{")
        .with_linux_or_windows_key_binding("ctrl-pageup"),
        EditableBinding::new(
            "workspace:activate_next_tab",
            "Activate next tab",
            WorkspaceAction::ActivateNextTab,
        )
        .with_context_predicate(
            id!("Workspace") & id!("Workspace_MultipleTabs") & !id!("Workspace_PaneDragging"),
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_mac_key_binding("shift-cmd-}")
        .with_linux_or_windows_key_binding("ctrl-pagedown"),
        EditableBinding::new(
            "pane_group:navigate_prev",
            "Activate previous pane",
            WorkspaceAction::NavigatePrevPaneOrPanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_custom_action(CustomAction::ActivatePreviousPane),
        EditableBinding::new(
            "pane_group:navigate_next",
            "Activate next pane",
            WorkspaceAction::NavigateNextPaneOrPanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_custom_action(CustomAction::ActivateNextPane),
        EditableBinding::new(
            "workspace:toggle_mouse_reporting",
            "Toggle Mouse Reporting",
            WorkspaceAction::ToggleMouseReporting,
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:create_team_notebook",
            BindingDescription::new("Create a new team notebook")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Team Notebook"),
            WorkspaceAction::CreateTeamNotebook,
        )
        .with_custom_action(CustomAction::NewTeamNotebook)
        .with_context_predicate(
            id!("Workspace")
                & id!(flags::ENABLE_WARP_DRIVE)
                & id!("WarpDrive_BelongsToTeam")
                & id!("IsOnline"),
        )
        .with_group(bindings::BindingGroup::Notebooks.as_str()),
        EditableBinding::new(
            "workspace:create_personal_notebook",
            BindingDescription::new("Create a new personal notebook")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Personal Notebook"),
            WorkspaceAction::CreatePersonalNotebook,
        )
        .with_group(bindings::BindingGroup::Notebooks.as_str())
        .with_custom_action(CustomAction::NewPersonalNotebook)
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE)),
        EditableBinding::new(
            "workspace:create_team_workflow",
            BindingDescription::new("Create a new team workflow")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Team Workflow"),
            WorkspaceAction::CreateTeamWorkflow,
        )
        .with_custom_action(CustomAction::NewTeamWorkflow)
        .with_context_predicate(
            id!("Workspace")
                & id!(flags::ENABLE_WARP_DRIVE)
                & id!("IsOnline")
                & id!("WarpDrive_BelongsToTeam"),
        )
        .with_group(bindings::BindingGroup::Workflow.as_str()),
        EditableBinding::new(
            "workspace:create_personal_workflow",
            BindingDescription::new("Create a new personal workflow")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Personal Workflow"),
            WorkspaceAction::CreatePersonalWorkflow,
        )
        .with_group(bindings::BindingGroup::Workflow.as_str())
        .with_custom_action(CustomAction::NewPersonalWorkflow)
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE)),
        EditableBinding::new(
            "workspace:create_team_folder",
            BindingDescription::new("Create a new team folder")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Team Folder"),
            WorkspaceAction::CreateTeamFolder,
        )
        .with_context_predicate(
            id!("Workspace")
                & id!(flags::ENABLE_WARP_DRIVE)
                & id!("IsOnline")
                & id!("WarpDrive_BelongsToTeam"),
        )
        .with_group(bindings::BindingGroup::Folders.as_str()),
        EditableBinding::new(
            "workspace:create_personal_folder",
            BindingDescription::new("Create a new personal folder")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Personal Folder"),
            WorkspaceAction::CreatePersonalFolder,
        )
        .with_group(bindings::BindingGroup::Folders.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE) & id!("IsOnline")),
        EditableBinding::new(
            NEW_TAB_BINDING_NAME,
            BindingDescription::new("Create new tab"),
            WorkspaceAction::AddDefaultTab,
        )
        .with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))
        .with_custom_action(CustomAction::NewTab)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            NEW_TERMINAL_TAB_BINDING_NAME,
            BindingDescription::new("New Terminal Tab"),
            WorkspaceAction::AddTerminalTab {
                hide_homepage: false,
            },
        )
        .with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))
        .with_custom_action(CustomAction::NewTerminalTab)
        .with_enabled(|| ContextFlag::CreateNewSession.is_enabled()),
        EditableBinding::new(
            NEW_AGENT_TAB_BINDING_NAME,
            BindingDescription::new("New Agent Tab"),
            WorkspaceAction::AddAgentTab,
        )
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_custom_action(CustomAction::NewAgentTab)
        .with_context_predicate(
            id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED) & !id!("Workspace_PaneDragging"),
        ),
        EditableBinding::new(
            NEW_AMBIENT_AGENT_TAB_BINDING_NAME,
            BindingDescription::new("New Cloud Agent Tab"),
            WorkspaceAction::AddAmbientAgentTab,
        )
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_context_predicate(
            id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED) & !id!("Workspace_PaneDragging"),
        )
        .with_enabled(|| {
            FeatureFlag::AgentView.is_enabled() && FeatureFlag::CloudMode.is_enabled()
        }),
        EditableBinding::new(
            "workspace:toggle_left_panel",
            BindingDescription::new("Open Left Panel"),
            WorkspaceAction::ToggleLeftPanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ToggleWarpDrive),
        EditableBinding::new(
            TOGGLE_RIGHT_PANEL_BINDING_NAME,
            BindingDescription::new("Toggle code review")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Toggle Code Review"),
            WorkspaceAction::ToggleRightPanel,
        )
        .with_enabled(|| cfg!(feature = "local_fs"))
        .with_context_predicate(id!("Workspace"))
        .with_mac_key_binding("cmd-shift-+")
        .with_linux_or_windows_key_binding("ctrl-shift-+"),
        EditableBinding::new(
            TOGGLE_VERTICAL_TABS_PANEL_BINDING_NAME,
            BindingDescription::new("Toggle vertical tabs panel")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Toggle Vertical Tabs Panel"),
            WorkspaceAction::ToggleVerticalTabsPanel,
        )
        .with_context_predicate(id!("Workspace") & id!(flags::USE_VERTICAL_TABS_FLAG))
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_enabled(|| FeatureFlag::VerticalTabs.is_enabled())
        .with_key_binding(cmd_or_ctrl_shift("b")),
        EditableBinding::new(
            LEFT_PANEL_AGENT_CONVERSATIONS_BINDING_NAME,
            BindingDescription::new("Left Panel: Agent conversations"),
            WorkspaceAction::ToggleConversationListView,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_CONVERSATION_HISTORY))
        .with_enabled(|| FeatureFlag::AgentViewConversationListView.is_enabled())
        .with_custom_action(CustomAction::ToggleConversationListView),
        EditableBinding::new(
            LEFT_PANEL_PROJECT_EXPLORER_BINDING_NAME,
            BindingDescription::new("Left Panel: Project explorer"),
            WorkspaceAction::ToggleProjectExplorer,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_PROJECT_EXPLORER))
        .with_custom_action(CustomAction::ToggleProjectExplorer),
        EditableBinding::new(
            LEFT_PANEL_GLOBAL_SEARCH_BINDING_NAME,
            BindingDescription::new("Left Panel: Global search"),
            WorkspaceAction::ToggleGlobalSearch,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_GLOBAL_SEARCH))
        .with_enabled(|| FeatureFlag::GlobalSearch.is_enabled())
        .with_custom_action(CustomAction::ToggleGlobalSearch),
        EditableBinding::new(
            "file_tree:toggle_hidden_files",
            BindingDescription::new("Toggle hidden files in Project Explorer"),
            WorkspaceAction::ToggleHiddenFiles,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_PROJECT_EXPLORER))
        .with_mac_key_binding("cmd-shift->")
        .with_linux_or_windows_key_binding("ctrl-shift->"),
        EditableBinding::new(
            LEFT_PANEL_WARP_DRIVE_BINDING_NAME,
            BindingDescription::new("Left Panel: Warp Drive"),
            WorkspaceAction::ToggleWarpDrive,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE))
        .with_mac_key_binding("ctrl-4")
        .with_linux_or_windows_key_binding("alt-4"),
        EditableBinding::new(
            TOGGLE_PROJECT_EXPLORER_BINDING_NAME,
            BindingDescription::new("Toggle project explorer")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Project Explorer"),
            WorkspaceAction::ToggleProjectExplorer,
        )
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_PROJECT_EXPLORER)),
        EditableBinding::new(
            OPEN_GLOBAL_SEARCH_BINDING_NAME,
            BindingDescription::new("Open global search")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Global Search"),
            WorkspaceAction::OpenGlobalSearch,
        )
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_GLOBAL_SEARCH))
        .with_mac_key_binding("cmd-shift-F")
        // we use alt because we use ctrl-shift-f for find because ctrl-f needs to be reserved for the shell
        .with_linux_or_windows_key_binding("alt-shift-F"),
        EditableBinding::new(
            TOGGLE_WARP_DRIVE_BINDING_NAME,
            BindingDescription::new("Toggle Warp Drive")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Warp Drive"),
            WorkspaceAction::ToggleWarpDrive,
        )
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE)),
        EditableBinding::new(
            TOGGLE_CONVERSATION_LIST_VIEW_BINDING_NAME,
            BindingDescription::new("Toggle Agent conversation list view").with_custom_description(
                bindings::MAC_MENUS_CONTEXT,
                "Agent conversation list view",
            ),
            WorkspaceAction::ToggleConversationListView,
        )
        .with_enabled(|| FeatureFlag::AgentViewConversationListView.is_enabled())
        .with_context_predicate(id!("Workspace") & id!(flags::SHOW_CONVERSATION_HISTORY))
        .with_mac_key_binding("cmd-shift-A")
        .with_linux_or_windows_key_binding("ctrl-shift-A")
        .with_group(bindings::BindingGroup::WarpAi.as_str()),
        EditableBinding::new(
            "workspace:close_panel",
            BindingDescription::new("Close focused panel")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Close focused panel"),
            WorkspaceAction::ClosePanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::CloseCurrentSession),
        EditableBinding::new(
            "workspace:toggle_command_palette",
            BindingDescription::new("Toggle command palette")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Command Palette"),
            WorkspaceAction::TogglePalette {
                mode: PaletteMode::Command,
                source: PaletteSource::Keybinding,
            },
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace") & !id!("Workspace_CloudConversationWebViewer"))
        .with_custom_action(CustomAction::CommandPalette),
        EditableBinding::new(
            "workspace:move_tab_left",
            BindingDescription::new("Move tab left")
                .with_dynamic_override(|ctx| uses_vertical_tabs(ctx).then(|| "move tab up".into())),
            WorkspaceAction::MoveActiveTabLeft,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(
            id!("Workspace")
                & id!("Workspace_MultipleTabs")
                & !id!("Workspace_LeftmostTabActive")
                & !id!("Workspace_PaneDragging"),
        )
        .with_custom_action(CustomAction::MoveTabLeft),
        EditableBinding::new(
            "workspace:move_tab_right",
            BindingDescription::new("Move tab right").with_dynamic_override(|ctx| {
                uses_vertical_tabs(ctx).then(|| "move tab down".into())
            }),
            WorkspaceAction::MoveActiveTabRight,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(
            id!("Workspace")
                & id!("Workspace_MultipleTabs")
                & !id!("Workspace_RightmostTabActive")
                & !id!("Workspace_PaneDragging"),
        )
        .with_custom_action(CustomAction::MoveTabRight),
        EditableBinding::new(
            "workspace:toggle_keybindings_page",
            "Toggle keyboard shortcuts",
            WorkspaceAction::ToggleKeybindingsPage,
        )
        .with_group(bindings::BindingGroup::KeyboardShortcuts.as_str())
        .with_context_predicate(id!("Workspace") & !id!("Workspace_TextOpen"))
        .with_custom_action(CustomAction::ToggleKeybindingsPage),
        EditableBinding::new(
            "workspace:show_keybinding_settings",
            "Open keybindings editor",
            WorkspaceAction::ConfigureKeybindingSettings {
                keybinding_name: None,
            },
        )
        .with_group(bindings::BindingGroup::KeyboardShortcuts.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_mac_key_binding("cmd-ctrl-k"),
        EditableBinding::new(
            "workspace:toggle_block_snackbar",
            "Toggle sticky command header",
            WorkspaceAction::ToggleBlockSnackbar,
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
    ]);

    // TODO(PLAT-113): Support a11y on non-MacOS platforms
    if cfg!(target_os = "macos") {
        app.register_editable_bindings([
            EditableBinding::new(
                "workspace:set_a11y_concise_verbosity_level",
                "[a11y] Set concise accessibility announcements",
                WorkspaceAction::SetA11yVerbosityLevel(AccessibilityVerbosity::Concise),
            )
            .with_context_predicate(id!("Workspace"))
            .with_key_binding("cmdorctrl-alt-c"),
            EditableBinding::new(
                "workspace:set_a11y_verbose_verbosity_level",
                "[a11y] Set verbose accessibility announcements",
                WorkspaceAction::SetA11yVerbosityLevel(AccessibilityVerbosity::Verbose),
            )
            .with_context_predicate(id!("Workspace"))
            .with_key_binding("cmdorctrl-alt-v"),
        ]);
    }

    app.register_editable_bindings([EditableBinding::new(
        "workspace:rename_active_tab",
        "Rename the current tab",
        WorkspaceAction::RenameActiveTab,
    )
    .with_group(bindings::BindingGroup::Settings.as_str())
    .with_custom_action(CustomAction::RenameTab)
    .with_context_predicate(id!("Workspace"))]);

    // Pane rename — same shape as RenameActiveTab but acts on the focused pane
    // in the active tab. Ships with no default keybinding so it surfaces in
    // Settings → Keyboard shortcuts as remappable; resolves issue #9351, where
    // the action existed only in the right-click context menu and was not
    // reachable via the binding registry.
    app.register_editable_bindings([EditableBinding::new(
        "workspace:rename_active_pane",
        "Rename the current pane",
        WorkspaceAction::RenameActivePane,
    )
    .with_group(bindings::BindingGroup::Settings.as_str())
    .with_context_predicate(id!("Workspace"))]);

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:terminate_app",
            "Quit Warp",
            WorkspaceAction::TerminateApp,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Close.as_str())
        .with_enabled(|| ContextFlag::CloseWindow.is_enabled()),
        EditableBinding::new(
            "workspace:close_window",
            BindingDescription::new("Close Window")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Close Window"),
            WorkspaceAction::CloseWindow,
        )
        .with_mac_key_binding("cmd-shift-W")
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Close.as_str())
        .with_custom_action(CustomAction::CloseWindow)
        .with_enabled(|| ContextFlag::CloseWindow.is_enabled()),
        EditableBinding::new(
            "workspace:close_active_tab",
            "Close the current tab",
            WorkspaceAction::CloseActiveTab,
        )
        .with_custom_action(CustomAction::CloseTab)
        .with_group(bindings::BindingGroup::Close.as_str())
        .with_context_predicate(
            id!("Workspace") & (id!("Workspace_CloseWindow") | id!("Workspace_MultipleTabs")),
        ),
        EditableBinding::new(
            "workspace:close_other_tabs",
            "Close other tabs",
            WorkspaceAction::CloseNonActiveTabs,
        )
        .with_custom_action(CustomAction::CloseOtherTabs)
        .with_group(bindings::BindingGroup::Close.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:close_tabs_right_active_tab",
            BindingDescription::new("Close tabs to the right").with_dynamic_override(|ctx| {
                uses_vertical_tabs(ctx).then(|| "close tabs below".into())
            }),
            WorkspaceAction::CloseTabsRightActiveTab,
        )
        .with_group(bindings::BindingGroup::Close.as_str())
        .with_custom_action(CustomAction::CloseTabsRight)
        .with_context_predicate(id!("Workspace")),
        // We have two actions depending on the current state
        // (i.e. whether notifications are already on or off).
        EditableBinding::new(
            "workspace:toggle_notifications_on",
            "Turn notifications on",
            WorkspaceAction::ToggleNotifications,
        )
        .with_group(bindings::BindingGroup::Notifications.as_str())
        .with_context_predicate(id!("Workspace") & !id!("Notifications_Enabled")),
        EditableBinding::new(
            "workspace:toggle_notifications_off",
            "Turn notifications off",
            WorkspaceAction::ToggleNotifications,
        )
        .with_group(bindings::BindingGroup::Notifications.as_str())
        .with_context_predicate(id!("Workspace") & id!("Notifications_Enabled")),
        EditableBinding::new(
            "workspace:toggle_navigation_palette",
            BindingDescription::new("Toggle navigation palette")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Navigation Palette"),
            WorkspaceAction::TogglePalette {
                mode: PaletteMode::Navigation,
                source: PaletteSource::Keybinding,
            },
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::NavigationPalette),
        EditableBinding::new(
            "workspace:toggle_launch_config_palette",
            "Launch configuration palette",
            WorkspaceAction::TogglePalette {
                mode: PaletteMode::LaunchConfig,
                source: PaletteSource::Keybinding,
            },
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::LaunchConfigPalette)
        .with_enabled(|| ContextFlag::LaunchConfigurations.is_enabled()),
        EditableBinding::new(
            "workspace:toggle_files_palette",
            "Toggle Files Palette",
            WorkspaceAction::TogglePalette {
                mode: PaletteMode::Files,
                source: PaletteSource::Keybinding,
            },
        )
        .with_context_predicate(id!("Workspace") & !id!("Workspace_ViewOnlySharedSession"))
        .with_custom_action(CustomAction::FilesPalette),
        EditableBinding::new(
            "workspace:open_launch_config_save_modal",
            "Save new launch configuration",
            WorkspaceAction::OpenLaunchConfigSaveModal,
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::SaveCurrentConfig)
        .with_enabled(|| ContextFlag::LaunchConfigurations.is_enabled()),
        EditableBinding::new(
            // If you rename this name, please update the name in command_palette/action/data_source.rs
            "workspace:search_drive",
            "Search Warp Drive",
            WorkspaceAction::OpenPalette {
                mode: PaletteMode::WarpDrive,
                source: PaletteSource::Keybinding,
                query: None,
            },
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::SearchDrive),
    ]);

    if FeatureFlag::Autoupdate.is_enabled() {
        app.register_editable_bindings([
            EditableBinding::new(
                "workspace:update_and_relaunch",
                "Install update and relaunch",
                // TODO(vorporeal): I wonder if we should change wording here?
                WorkspaceAction::ApplyUpdate,
            )
            .with_group(bindings::BindingGroup::AutoUpdate.as_str())
            .with_context_predicate(id!("Workspace") & id!("AutoupdateState_UpdateReady"))
            .with_enabled(|| ContextFlag::PromptForVersionUpdates.is_enabled()),
            EditableBinding::new(
                "workspace:check_for_updates",
                "Check for updates",
                WorkspaceAction::CheckForUpdate,
            )
            .with_group(bindings::BindingGroup::AutoUpdate.as_str())
            .with_context_predicate(id!("Workspace") & !id!("AutoupdateState_UpdateReady"))
            .with_enabled(|| ContextFlag::PromptForVersionUpdates.is_enabled()),
        ]);
    }

    app.register_editable_bindings([EditableBinding::new(
        "workspace:log_out",
        "Log out",
        WorkspaceAction::LogOut,
    )
    .with_group(bindings::BindingGroup::Settings.as_str())
    .with_context_predicate(id!("Workspace") & !id!("IsAnonymousUser"))]);

    if !FeatureFlag::AvatarInTabBar.is_enabled() {
        app.register_editable_bindings([EditableBinding::new(
            "workspace:toggle_resource_center",
            "Toggle resource center",
            WorkspaceAction::ToggleResourceCenter,
        )
        .with_group(bindings::BindingGroup::Navigation.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ToggleResourceCenter)]);
    }

    if cfg!(not(target_family = "wasm")) {
        app.register_editable_bindings([EditableBinding::new(
            "workspace:export_all_warp_drive_objects",
            "Export all Warp Drive objects",
            WorkspaceAction::ExportAllWarpDriveObjects,
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE))]);
    }

    // CLI install/uninstall actions (macOS only)
    #[cfg(target_os = "macos")]
    {
        app.register_editable_bindings([
            EditableBinding::new(
                "workspace:install_cli",
                "Install Oz CLI command",
                WorkspaceAction::InstallCLI,
            )
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_context_predicate(id!("Workspace")),
            EditableBinding::new(
                "workspace:uninstall_cli",
                "Uninstall Oz CLI command",
                WorkspaceAction::UninstallCLI,
            )
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_context_predicate(id!("Workspace")),
        ]);
    }

    if FeatureFlag::Changelog.is_enabled() {
        app.register_editable_bindings([
            // Always show the "View latest changelog" action in the command palette,
            // but without a keybinding when the update toast is not visible.
            EditableBinding::new(
                "workspace:view_changelog",
                "View latest changelog",
                WorkspaceAction::ViewLatestChangelog,
            )
            .with_context_predicate(id!("Workspace") & !id!("UpdateToastVisible"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            // Note that while the changelog resides in WarpEssentials, we should gate access to
            // the changelog based on whether WarpEssentials is an available view.
            .with_enabled(|| ContextFlag::WarpEssentials.is_enabled()),
            // When the update toast is visible, register the keybinding as well.
            EditableBinding::new(
                "workspace:view_changelog",
                "View latest changelog",
                WorkspaceAction::ViewLatestChangelog,
            )
            .with_context_predicate(id!("Workspace") & id!("UpdateToastVisible"))
            .with_group(bindings::BindingGroup::Settings.as_str())
            .with_custom_action(CustomAction::ViewChangelog)
            .with_linux_or_windows_key_binding(format!("alt-{}", cmd_or_ctrl_shift("o")))
            .with_enabled(|| ContextFlag::WarpEssentials.is_enabled()),
        ]);
    }

    // We use the same binding name for the AI Assistant and block list AI to preserve custom
    // keybindings between them.
    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:toggle_ai_assistant",
            *NEW_AGENT_PANE_LABEL,
            WorkspaceAction::NewPaneInAgentMode {
                entrypoint: AgentModeEntrypoint::NewPaneBinding,
                zero_state_prompt_suggestion_type: None,
            },
        )
        .with_enabled(|| FeatureFlag::AgentMode.is_enabled())
        .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_custom_action(CustomAction::NewAgentModePane),
        EditableBinding::new(
            "workspace:toggle_ai_assistant",
            "Toggle Warp AI",
            WorkspaceAction::ToggleAIAssistant,
        )
        .with_enabled(|| !FeatureFlag::AgentMode.is_enabled())
        .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        // We use the same custom action as AM so that we don't have
        // two mac menu items for AM vs Warp AI since they are mutually exclusive.
        .with_custom_action(CustomAction::NewAgentModePane),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:create_team_env_vars",
            BindingDescription::new("Create new team environment variables")
                .with_custom_description(
                    bindings::MAC_MENUS_CONTEXT,
                    "New Team Environment Variables",
                ),
            WorkspaceAction::CreateTeamEnvVarCollection,
        )
        .with_custom_action(CustomAction::NewTeamEnvVars)
        .with_context_predicate(
            id!("Workspace")
                & id!(flags::ENABLE_WARP_DRIVE)
                & id!("WarpDrive_BelongsToTeam")
                & id!("IsOnline"),
        )
        .with_group(bindings::BindingGroup::EnvVarCollection.as_str()),
        EditableBinding::new(
            "workspace:create_personal_env_vars",
            BindingDescription::new("Create new personal environment variables")
                .with_custom_description(
                    bindings::MAC_MENUS_CONTEXT,
                    "New Personal Environment Variables",
                ),
            WorkspaceAction::CreatePersonalEnvVarCollection,
        )
        .with_group(bindings::BindingGroup::EnvVarCollection.as_str())
        .with_custom_action(CustomAction::NewPersonalEnvVars)
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE)),
        EditableBinding::new(
            "workspace:create_personal_ai_prompt",
            BindingDescription::new("Create a new personal prompt")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Personal Prompt"),
            WorkspaceAction::CreatePersonalAIPrompt,
        )
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_custom_action(CustomAction::NewPersonalAIPrompt)
        .with_context_predicate(
            id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE) & id!(flags::IS_ANY_AI_ENABLED),
        ),
        EditableBinding::new(
            "workspace:create_team_ai_prompt",
            BindingDescription::new("Create a new team prompt")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "New Team Prompt"),
            WorkspaceAction::CreateTeamAIPrompt,
        )
        .with_group(bindings::BindingGroup::WarpAi.as_str())
        .with_custom_action(CustomAction::NewTeamAIPrompt)
        .with_context_predicate(
            id!("Workspace")
                & id!(flags::ENABLE_WARP_DRIVE)
                & id!("WarpDrive_BelongsToTeam")
                & id!("IsOnline")
                & id!(flags::IS_ANY_AI_ENABLED),
        ),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:shift_focus_left",
            "Switch Focus to Left Panel",
            WorkspaceAction::FocusLeftPanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_key_binding("cmdorctrl-shift-("),
        EditableBinding::new(
            "workspace:shift_focus_right",
            "Switch Focus to Right Panel",
            WorkspaceAction::FocusRightPanel,
        )
        .with_context_predicate(id!("Workspace"))
        .with_key_binding("cmdorctrl-shift-)"),
    ]);

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:import_to_personal_drive",
            "Import To Personal Drive",
            WorkspaceAction::ImportToPersonalDrive,
        )
        .with_context_predicate(id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE)),
        EditableBinding::new(
            "workspace:import_to_team_drive",
            "Import To Team Drive",
            WorkspaceAction::ImportToTeamDrive,
        )
        .with_context_predicate(
            id!("Workspace") & id!(flags::ENABLE_WARP_DRIVE) & id!("WarpDrive_BelongsToTeam"),
        ),
    ]);

    // Register a debug-only action for writing the user's access token to the system clipboard
    // to aid debugging and development.
    #[cfg(not(feature = "skip_login"))]
    if ChannelState::enable_debug_features() {
        app.register_editable_bindings([EditableBinding::new(
            "workspace:copy_access_token_to_clipboard",
            "Copy access token to clipboard",
            WorkspaceAction::CopyAccessTokenToClipboard,
        )
        .with_context_predicate(id!("Workspace"))]);
    }

    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:open_repository",
            BindingDescription::new("Open repository")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Open Repository"),
            WorkspaceAction::OpenRepository { path: None },
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::OpenRepository)
        .with_group(bindings::BindingGroup::Folders.as_str()),
        EditableBinding::new(
            "workspace:open_ai_fact_collection",
            BindingDescription::new("Open AI Rules")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Open AI Rules"),
            WorkspaceAction::OpenAIFactCollection,
        )
        .with_enabled(|| FeatureFlag::AIRules.is_enabled())
        .with_custom_action(CustomAction::OpenAIFactCollection)
        .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
        .with_group(bindings::BindingGroup::WarpAi.as_str()),
    ]);

    app.register_editable_bindings([EditableBinding::new(
        "workspace:open_mcp_servers",
        BindingDescription::new("Open MCP Servers")
            .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Open MCP Servers"),
        WorkspaceAction::OpenMCPServerCollection,
    )
    .with_enabled(|| {
        FeatureFlag::McpServer.is_enabled() && ContextFlag::ShowMCPServers.is_enabled()
    })
    .with_custom_action(CustomAction::OpenMCPServerCollection)
    .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
    .with_group(bindings::BindingGroup::WarpAi.as_str())]);

    app.register_editable_bindings([EditableBinding::new(
        "workspace:jump_to_latest_toast",
        "Jump to latest agent task",
        WorkspaceAction::JumpToLatestToast,
    )
    .with_enabled(|| FeatureFlag::AgentMode.is_enabled())
    .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
    .with_mac_key_binding("cmd-shift-G")
    .with_linux_or_windows_key_binding("ctrl-shift-G")
    .with_group(bindings::BindingGroup::WarpAi.as_str())]);

    app.register_editable_bindings([EditableBinding::new(
        TOGGLE_NOTIFICATION_MAILBOX_BINDING_NAME,
        "Toggle notification mailbox",
        WorkspaceAction::ToggleNotificationMailbox { select_first: true },
    )
    .with_enabled(|| FeatureFlag::HOANotifications.is_enabled())
    .with_context_predicate(id!("Workspace"))
    .with_mac_key_binding("cmd-shift-U")
    .with_linux_or_windows_key_binding("ctrl-shift-U")
    .with_group(bindings::BindingGroup::WarpAi.as_str())]);

    add_open_setting_pages_as_editable_binding(app);
    add_overflow_menu_items_as_editable_binding(app);

    app.register_editable_bindings([EditableBinding::new(
        "workspace:toggle_agent_management_view",
        "Toggle the agent management view",
        WorkspaceAction::ToggleAgentManagementView,
    )
    .with_enabled(|| FeatureFlag::AgentManagementView.is_enabled())
    .with_context_predicate(id!("Workspace") & id!(flags::IS_ANY_AI_ENABLED))
    .with_mac_key_binding("cmd-shift-M")
    .with_linux_or_windows_key_binding("ctrl-shift-M")
    .with_group(bindings::BindingGroup::WarpAi.as_str())]);
}

fn add_open_setting_pages_as_editable_binding(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Add the ability to open setting modals to the command palette.
    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:show_settings",
            BindingDescription::new("Open Settings")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Settings"),
            WorkspaceAction::ShowSettings,
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::ShowSettings),
        EditableBinding::new(
            "workspace:show_settings_account_page",
            "Open Settings: Account",
            WorkspaceAction::ShowSettingsPage(SettingsSection::Account),
        )
        .with_context_predicate(id!("Workspace"))
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::ShowAccount),
        EditableBinding::new(
            "workspace:show_settings_appearance_page",
            BindingDescription::new("Open Settings: Appearance")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Appearance..."),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Appearance),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ShowAppearance),
        EditableBinding::new(
            "workspace:show_settings_features_page",
            "Open Settings: Features",
            WorkspaceAction::ShowSettingsPage(SettingsSection::Features),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_shared_blocks_page",
            BindingDescription::new("Open Settings: Shared Blocks")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "View Shared Blocks..."),
            WorkspaceAction::ShowSettingsPage(SettingsSection::SharedBlocks),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ViewSharedBlocks),
        EditableBinding::new(
            "workspace:show_settings_keyboard_shortcuts_page",
            BindingDescription::new("Open Settings: Keyboard Shortcuts").with_custom_description(
                bindings::MAC_MENUS_CONTEXT,
                "Configure Keyboard Shortcuts...",
            ),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Keybindings),
        )
        .with_group(bindings::BindingGroup::KeyboardShortcuts.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ConfigureKeybindings),
        EditableBinding::new(
            "workspace:show_settings_about_page",
            BindingDescription::new("Open Settings: About")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "About Warp"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::About),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ShowAboutWarp),
        EditableBinding::new(
            "workspace:show_settings_teams_page",
            BindingDescription::new("Open Settings: Teams")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Open Team Settings"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Teams),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_custom_action(CustomAction::OpenTeamSettings)
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_privacy_page",
            BindingDescription::new("Open Settings: Privacy"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Privacy),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_warpify_page",
            BindingDescription::new("Open Settings: Warpify")
                .with_custom_description(bindings::MAC_MENUS_CONTEXT, "Configure Warpify..."),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Warpify),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_ai_settings_page",
            BindingDescription::new("Open Settings: AI"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::WarpAgent),
        )
        .with_enabled(|| FeatureFlag::AgentMode.is_enabled())
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_billing_and_usage_page",
            BindingDescription::new("Open Settings: Billing and usage"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::BillingAndUsage),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_code_page",
            BindingDescription::new("Open Settings: Code"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::CodeIndexing),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_referrals_page",
            BindingDescription::new("Open Settings: Referrals"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::Referrals),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_settings_environments_page",
            BindingDescription::new("Open Settings: Environments"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::CloudEnvironments),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:show_mcp_servers_settings_page",
            BindingDescription::new("Open Settings: MCP Servers"),
            WorkspaceAction::ShowSettingsPage(SettingsSection::MCPServers),
        )
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:open_settings_file",
            "Open settings file",
            WorkspaceAction::OpenSettingsFile,
        )
        .with_enabled(|| FeatureFlag::SettingsFile.is_enabled() && cfg!(feature = "local_fs"))
        .with_group(bindings::BindingGroup::Settings.as_str())
        .with_context_predicate(id!("Workspace")),
    ]);
}

fn add_overflow_menu_items_as_editable_binding(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    // Add the ability to open all overflow menu items to the command palette.
    app.register_editable_bindings([
        EditableBinding::new(
            "workspace:show_invite_modal",
            "Invite People...",
            WorkspaceAction::ShowReferralSettingsPage,
        )
        .with_context_predicate(id!("Workspace"))
        .with_custom_action(CustomAction::ReferAFriend),
        EditableBinding::new(
            "workspace:link_to_slack",
            "Join our Slack community (opens external link)",
            WorkspaceAction::JoinSlack,
        )
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:link_to_user_docs",
            "View user docs (opens external link)",
            WorkspaceAction::ViewUserDocs,
        )
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:send_feedback",
            BindingDescription::new("Send feedback (opens external link)").with_dynamic_override(
                |ctx| is_feedback_skill_available(ctx).then(|| "Send feedback with Oz".into()),
            ),
            WorkspaceAction::SendFeedback,
        )
        .with_context_predicate(id!("Workspace")),
        #[cfg(not(target_family = "wasm"))]
        EditableBinding::new(
            "workspace:view_logs",
            "View Warp logs",
            WorkspaceAction::ViewLogs,
        )
        .with_context_predicate(id!("Workspace")),
        EditableBinding::new(
            "workspace:link_to_privacy_policy",
            "View privacy policy (opens external link)",
            WorkspaceAction::ViewPrivacyPolicy,
        )
        .with_context_predicate(id!("Workspace")),
    ]);
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct TabBarDropTargetData {
    pub tab_bar_location: TabBarLocation,
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct VerticalTabsPaneDropTargetData {
    pub tab_bar_location: TabBarLocation,
    pub tab_hover_index: TabBarHoverIndex,
}

#[derive(PartialEq, Copy, Clone, Debug, Serialize, Deserialize)]
pub enum TabBarLocation {
    TabIndex(usize),
    AfterTabIndex(usize), // Indicates any area after the tabs in the tab bar, includes the total tab count
}

impl DropTargetData for TabBarDropTargetData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl DropTargetData for VerticalTabsPaneDropTargetData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
