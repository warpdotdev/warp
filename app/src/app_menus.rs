use std::borrow::Cow;
use std::fs::File;
use std::path::PathBuf;

use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::auth::AuthStateProvider;
use crate::default_terminal::DefaultTerminal;
use crate::features::{runtime_flags_menu_items, FeatureFlag};
use crate::i18n::{self, I18nKey};
use crate::root_view::OpenLaunchConfigArg;
use crate::server::telemetry::LaunchConfigUiLocation;
use crate::settings::{
    AISettings, BlockVisibilitySettings, DebugSettings, DefaultSessionMode, SelectionSettings,
};
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::settings::{SpacingMode, TerminalSettings};
use crate::undo_close::UndoCloseStack;
use crate::user_config::WarpConfig;
use crate::util::bindings::{self, trigger_to_keystroke, CustomAction};
use crate::util::links;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::{auth, report_if_error};
use ai::workspace::WorkspaceMetadata;
use csv::Writer;
use enclose::enclose;
use itertools::Itertools;
use settings::manager::SettingsManager;
use settings::Setting as _;
use warp_core::context_flag::ContextFlag;
use warp_util::path::user_friendly_path;
use warpui::actions::StandardAction;
use warpui::keymap::{Keystroke, Trigger};
use warpui::platform::menu::{
    CustomMenuItem, Menu, MenuBar, MenuItem, MenuItemProperties, MenuItemPropertyChanges,
};
use warpui::windowing::WindowManager;
use warpui::{AppContext, SingletonEntity};

type CheckmarkStatusGetter = dyn 'static + Fn(&mut AppContext) -> bool;

const ENABLE_SHELL_DEBUG_MODE_MENU_ITEM_NAME: &str =
    "Enable Shell Debug Mode (-x) for New Sessions";
const DISABLE_SHELL_DEBUG_MODE_MENU_ITEM_NAME: &str =
    "Disable Shell Debug Mode (-x) for New Sessions";
const ENABLE_IN_BAND_GENERATORS_MENU_ITEM_NAME: &str = "Enable In-band Generators for New Sessions";
const DISABLE_IN_BAND_GENERATORS_MENU_ITEM_NAME: &str =
    "Disable in-band generators for new sessions";
const ENABLE_PTY_RECORDING: &str = "Enable PTY Recording Mode (warp.pty.recording)";
const DISABLE_PTY_RECORDING: &str = "Disable PTY Recording Mode (warp.pty.recording)";
const SHOW_BOOTSTRAP_BLOCK_MENU_ITEM_NAME: &str = "Show Initialization Block";
const HIDE_BOOTSTRAP_BLOCK_MENU_ITEM_NAME: &str = "Hide Initialization Block";
const SHOW_IN_BAND_COMMAND_BLOCKS_MENU_ITEM_NAME: &str = "Show In-band Command Blocks";
const HIDE_IN_BAND_COMMAND_BLOCKS_MENU_ITEM_NAME: &str = "Hide In-band Command Blocks";
const SHOW_SSH_COMMAND_BLOCKS_MENU_ITEM_NAME: &str = "Show Warpified SSH Blocks";
const HIDE_SSH_COMMAND_BLOCKS_MENU_ITEM_NAME: &str = "Hide Warpified SSH Blocks";
const EXPORT_DEFAULT_SETTINGS_CSV_MENU_ITEM_NAME: &str =
    "Export Default Settings as CSV to home dir";

const SETTINGS_CSV_FILE_NAME: &str = "warp_default_settings.csv";
const MAX_RECENT_REPOS_IN_MENU: usize = 10;

/// Creates the root app menu bar
pub fn menu_bar(ctx: &mut AppContext) -> MenuBar {
    MenuBar::new(vec![
        make_new_app_menu(ctx),
        make_new_file_menu(ctx),
        make_new_edit_menu(ctx),
        make_new_view_menu(ctx),
        make_new_tab_menu(ctx),
        make_new_blocks_menu(ctx),
        make_new_ai_menu(ctx),
        make_new_drive_menu(ctx),
        make_new_window_menu(ctx),
        make_new_help_menu(ctx),
    ])
}

// Creates the app dock menu
// Menu here instead of MenuBar, since we only need one vec<MenuItem> in the dock
// To create submenus, we could use MenuItem::Custom(CustomMenuItem::new_with_submenu(...))
pub fn dock_menu(ctx: &AppContext) -> Menu {
    Menu::new(
        i18n::tr(ctx, I18nKey::MenuNewWindow),
        vec![MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuNewWindow),
            move |ctx| {
                ctx.dispatch_global_action("root_view:open_new", &());
                ctx.dispatch_global_action("workspace:save_app", &());
            },
            no_updates,
            Some(Keystroke::parse("cmd-n").expect("Valid keystroke")),
        ))],
    )
}

fn custom_shortcut(action: CustomAction) -> Option<Keystroke> {
    trigger_to_keystroke(&Trigger::Custom(action.into()))
}

fn default_name(action: CustomAction, ctx: &AppContext) -> String {
    if let Some(label) = localized_custom_action_name(action, ctx) {
        return label.to_string();
    }

    ctx.description_for_custom_action(action.into(), bindings::MAC_MENUS_CONTEXT)
        .unwrap_or_else(|| {
            debug_assert!(false, "action should have a name: {action:?}");
            "<NO DESCRIPTION>".into()
        })
}

fn localized_custom_action_name(action: CustomAction, ctx: &AppContext) -> Option<&'static str> {
    match action {
        CustomAction::ReferAFriend => Some(i18n::tr(ctx, I18nKey::MenuInviteAFriend)),
        CustomAction::ShowSettings => Some(i18n::tr(ctx, I18nKey::MenuSettings)),
        CustomAction::ToggleKeybindingsPage => Some(i18n::tr(ctx, I18nKey::MenuKeyboardShortcuts)),
        CustomAction::ShowAppearance => Some(i18n::tr(ctx, I18nKey::SettingsNavAppearance)),
        CustomAction::ViewChangelog => Some(i18n::tr(ctx, I18nKey::MenuWhatsNew)),
        CustomAction::NewFile => Some(i18n::tr(ctx, I18nKey::CodeNewFile)),
        CustomAction::SplitPaneRight => Some(i18n::tr(ctx, I18nKey::CommonSplitPaneRight)),
        CustomAction::SplitPaneLeft => Some(i18n::tr(ctx, I18nKey::CommonSplitPaneLeft)),
        CustomAction::SplitPaneDown => Some(i18n::tr(ctx, I18nKey::CommonSplitPaneDown)),
        CustomAction::SplitPaneUp => Some(i18n::tr(ctx, I18nKey::CommonSplitPaneUp)),
        CustomAction::RenameTab => Some(i18n::tr(ctx, I18nKey::TabRenameTab)),
        CustomAction::CloseTab => Some(i18n::tr(ctx, I18nKey::TabCloseTab)),
        CustomAction::CloseOtherTabs => Some(i18n::tr(ctx, I18nKey::TabCloseOtherTabs)),
        CustomAction::CloseTabsRight => Some(i18n::tr(ctx, I18nKey::TabCloseTabsRight)),
        CustomAction::ReopenClosedSession => {
            Some(i18n::tr(ctx, I18nKey::CommonReopenClosedSession))
        }
        CustomAction::CloseCurrentSession => Some(i18n::tr(ctx, I18nKey::MenuCloseFocusedPanel)),
        CustomAction::CloseWindow => Some(i18n::tr(ctx, I18nKey::MenuCloseWindow)),
        CustomAction::ViewSharedBlocks => Some(i18n::tr(ctx, I18nKey::SettingsNavSharedBlocks)),
        CustomAction::ToggleWarpDrive => Some(i18n::tr(ctx, I18nKey::SettingsNavWarpDrive)),
        CustomAction::OpenTeamSettings => Some(i18n::tr(ctx, I18nKey::SettingsNavTeams)),
        CustomAction::OpenAIFactCollection => Some(i18n::tr(ctx, I18nKey::SettingsNavKnowledge)),
        CustomAction::OpenMCPServerCollection => {
            Some(i18n::tr(ctx, I18nKey::SettingsNavMcpServers))
        }
        CustomAction::OpenRepository => Some(i18n::tr(ctx, I18nKey::MenuOpenRepository)),
        CustomAction::NewTerminalTab => Some(i18n::tr(ctx, I18nKey::MenuNewTerminalTab)),
        CustomAction::NewAgentTab => Some(i18n::tr(ctx, I18nKey::MenuNewAgentTab)),
        CustomAction::NewAgentModePane => Some(i18n::tr(ctx, I18nKey::CommonAgent)),
        CustomAction::Undo => Some(i18n::tr(ctx, I18nKey::CommonUndo)),
        CustomAction::Redo => Some(i18n::tr_static(ctx, "Redo")),
        CustomAction::Cut => Some(i18n::tr(ctx, I18nKey::CommonCut)),
        CustomAction::Copy => Some(i18n::tr(ctx, I18nKey::CommonCopy)),
        CustomAction::Paste => Some(i18n::tr(ctx, I18nKey::CommonPaste)),
        CustomAction::SelectAll => Some(i18n::tr(ctx, I18nKey::CommonSelectAll)),
        CustomAction::ClearEditor => Some(i18n::tr_static(ctx, "Clear command editor")),
        CustomAction::AddNextOccurrence => {
            Some(i18n::tr_static(ctx, "Add selection for next occurrence"))
        }
        CustomAction::AddCursorAbove => Some(i18n::tr_static(ctx, "Add cursor above")),
        CustomAction::AddCursorBelow => Some(i18n::tr_static(ctx, "Add cursor below")),
        CustomAction::Find => Some(i18n::tr_static(ctx, "Find")),
        CustomAction::GoToLine => Some(i18n::tr_static(ctx, "Go to line")),
        CustomAction::FocusInput => Some(i18n::tr_static(ctx, "Focus terminal input")),
        CustomAction::ToggleSyncAllTerminalInputsInAllTabs => Some(i18n::tr_static(
            ctx,
            "Toggle Synchronizing All Panes in All Tabs",
        )),
        CustomAction::ToggleSyncTerminalInputsInCurrentTab => Some(i18n::tr_static(
            ctx,
            "Toggle Synchronizing All Panes in Current Tab",
        )),
        CustomAction::DisableSyncTerminalInputs => {
            Some(i18n::tr_static(ctx, "Stop Synchronizing Any Panes"))
        }
        CustomAction::CommandPalette => Some(i18n::tr_static(ctx, "Command Palette")),
        CustomAction::AISearch => Some(i18n::tr(ctx, I18nKey::TerminalAiCommandSearch)),
        CustomAction::AttachSelectionAsAgentModeContext => {
            Some(i18n::tr_static(ctx, "Attach Selection as Agent Context"))
        }
        CustomAction::NavigationPalette => Some(i18n::tr_static(ctx, "Navigation Palette")),
        CustomAction::LaunchConfigPalette => Some(i18n::tr(ctx, I18nKey::MenuLaunchConfigurations)),
        CustomAction::FilesPalette => Some(i18n::tr_static(ctx, "Project Explorer")),
        CustomAction::ToggleConversationListView => {
            Some(i18n::tr_static(ctx, "Agent conversation list view"))
        }
        CustomAction::ToggleProjectExplorer => Some(i18n::tr_static(ctx, "Project Explorer")),
        CustomAction::ToggleGlobalSearch => Some(i18n::tr_static(ctx, "Global Search")),
        CustomAction::History => Some(i18n::tr_static(ctx, "History")),
        CustomAction::CommandSearch => Some(i18n::tr_static(ctx, "Command Search")),
        CustomAction::Workflows => Some(i18n::tr_static(ctx, "Workflows")),
        CustomAction::IncreaseZoom => Some(i18n::tr_static(ctx, "Zoom In")),
        CustomAction::DecreaseZoom => Some(i18n::tr_static(ctx, "Zoom Out")),
        CustomAction::ResetZoom => Some(i18n::tr_static(ctx, "Reset Zoom")),
        CustomAction::IncreaseFontSize => Some(i18n::tr_static(ctx, "Increase font size")),
        CustomAction::DecreaseFontSize => Some(i18n::tr_static(ctx, "Decrease font size")),
        CustomAction::ResetFontSize => Some(i18n::tr_static(ctx, "Reset font size to default")),
        CustomAction::MoveTabLeft => Some(i18n::tr_static(ctx, "Move tab left")),
        CustomAction::MoveTabRight => Some(i18n::tr_static(ctx, "Move tab right")),
        CustomAction::CycleNextSession => Some(i18n::tr_static(ctx, "Switch to next tab")),
        CustomAction::CyclePrevSession => Some(i18n::tr_static(ctx, "Switch to previous tab")),
        CustomAction::ActivateNextPane => Some(i18n::tr_static(ctx, "Activate next pane")),
        CustomAction::ActivatePreviousPane => Some(i18n::tr_static(ctx, "Activate previous pane")),
        CustomAction::ToggleMaximizePane => {
            Some(i18n::tr_static(ctx, "Toggle Maximize Active Pane"))
        }
        CustomAction::ClearBlocks => Some(i18n::tr_static(ctx, "Clear Blocks")),
        CustomAction::SelectBlockAbove => Some(i18n::tr_static(ctx, "Select previous block")),
        CustomAction::SelectBlockBelow => Some(i18n::tr_static(ctx, "Select next block")),
        CustomAction::SelectAllBlocks => Some(i18n::tr_static(ctx, "Select all blocks")),
        CustomAction::ScrollToTopOfSelectedBlocks => {
            Some(i18n::tr_static(ctx, "Scroll to top of selected block"))
        }
        CustomAction::ScrollToBottomOfSelectedBlocks => {
            Some(i18n::tr_static(ctx, "Scroll to bottom of selected block"))
        }
        CustomAction::CreateBlockPermalink => Some(i18n::tr_static(ctx, "Share selected block")),
        CustomAction::ToggleBookmarkBlock => Some(i18n::tr_static(ctx, "Bookmark selected block")),
        CustomAction::FindWithinBlock => Some(i18n::tr_static(ctx, "Find within selected block")),
        CustomAction::CopyBlock => Some(i18n::tr_static(ctx, "Copy command and output")),
        CustomAction::CopyBlockCommand => Some(i18n::tr_static(ctx, "Copy command")),
        CustomAction::CopyBlockOutput => Some(i18n::tr_static(ctx, "Copy command output")),
        CustomAction::NewPersonalWorkflow => Some(i18n::tr_static(ctx, "New Personal Workflow")),
        CustomAction::NewPersonalNotebook => Some(i18n::tr_static(ctx, "New Personal Notebook")),
        CustomAction::NewPersonalAIPrompt => Some(i18n::tr_static(ctx, "New Personal Prompt")),
        CustomAction::NewPersonalEnvVars => {
            Some(i18n::tr_static(ctx, "New Personal Environment Variables"))
        }
        CustomAction::NewTeamWorkflow => Some(i18n::tr_static(ctx, "New Team Workflow")),
        CustomAction::NewTeamNotebook => Some(i18n::tr_static(ctx, "New Team Notebook")),
        CustomAction::NewTeamAIPrompt => Some(i18n::tr_static(ctx, "New Team Prompt")),
        CustomAction::NewTeamEnvVars => {
            Some(i18n::tr_static(ctx, "New Team Environment Variables"))
        }
        CustomAction::SearchDrive => Some(i18n::tr_static(ctx, "Search Drive")),
        CustomAction::SharePaneContents => Some(i18n::tr_static(ctx, "Share pane contents")),
        CustomAction::ShareCurrentSession => Some(i18n::tr(ctx, I18nKey::TabShareSession)),
        _ => None,
    }
}

fn non_updateable_custom_item(action: CustomAction, ctx: &AppContext) -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        &default_name(action, ctx),
        custom_action_dispatcher(action),
        no_updates,
        custom_shortcut(action),
    ))
}

/// Return a Custom Menu Item whose CustomAction can be updated and
/// whose checkmark status is determined by
/// the bool result of should_be_checked, a provided closure
fn updateable_custom_item_with_checkmark(
    action: CustomAction,
    ctx: &AppContext,
    should_be_checked: Box<CheckmarkStatusGetter>,
) -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        &default_name(action, ctx),
        custom_action_dispatcher(action),
        custom_action_updater(action, should_be_checked),
        custom_shortcut(action),
    ))
}

/// Return a Custom Menu Item whose CustomAction can be updated
/// and is always unchecked
fn updateable_custom_item_without_checkmark(action: CustomAction, ctx: &AppContext) -> MenuItem {
    updateable_custom_item_with_checkmark(action, ctx, Box::new(|_| false))
}

fn make_new_app_menu(ctx: &AppContext) -> Menu {
    let mut menu_items = vec![updateable_custom_item_without_checkmark(
        CustomAction::ShowAboutWarp,
        ctx,
    )];

    if !FeatureFlag::AvatarInTabBar.is_enabled() {
        menu_items.push(updateable_custom_item_without_checkmark(
            CustomAction::ToggleResourceCenter,
            ctx,
        ))
    }

    menu_items.extend([
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ReferAFriend, ctx),
        MenuItem::Separator,
    ]);

    let preferences_menu_items = vec![
        updateable_custom_item_without_checkmark(CustomAction::ShowSettings, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ToggleKeybindingsPage, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ConfigureKeybindings, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ShowAppearance, ctx),
        MenuItem::Separator,
    ];

    menu_items.push(MenuItem::Custom(CustomMenuItem::new_with_submenu(
        i18n::tr(ctx, I18nKey::MenuPreferences),
        |_| (),
        no_updates,
        None,
        preferences_menu_items,
    )));

    if FeatureFlag::Changelog.is_enabled() {
        menu_items.push(updateable_custom_item_without_checkmark(
            CustomAction::ViewChangelog,
            ctx,
        ));
    }

    #[cfg(target_os = "macos")]
    {
        menu_items.push(MenuItem::Services);
    }

    menu_items.push(MenuItem::Separator);
    menu_items.push(link_menu_item(
        i18n::tr(ctx, I18nKey::MenuPrivacyPolicy),
        links::PRIVACY_POLICY_URL.into(),
    ));

    let debug_menu_items = debug_menu_items();
    if !debug_menu_items.is_empty() {
        menu_items.push(MenuItem::Custom(CustomMenuItem::new_with_submenu(
            i18n::tr(ctx, I18nKey::MenuDebug),
            |_| (),
            no_updates,
            None,
            debug_menu_items,
        )));
    }

    menu_items.push(MenuItem::Separator);
    menu_items.push(MenuItem::Standard(StandardAction::Hide));
    menu_items.push(MenuItem::Standard(StandardAction::HideOtherApps));
    menu_items.push(MenuItem::Standard(StandardAction::ShowAllApps));
    menu_items.push(MenuItem::Separator);
    menu_items.push(MenuItem::Custom(CustomMenuItem::new(
        i18n::tr(ctx, I18nKey::MenuSetDefaultTerminal),
        move |ctx| {
            DefaultTerminal::handle(ctx).update(ctx, |default_terminal, ctx| {
                default_terminal.make_warp_default(ctx)
            });
        },
        move |_props, ctx| {
            let default_terminal = DefaultTerminal::handle(ctx).as_ref(ctx);
            MenuItemPropertyChanges {
                disabled: Some(
                    !DefaultTerminal::can_warp_become_default()
                        || default_terminal.is_warp_default(),
                ),
                ..Default::default()
            }
        },
        None,
    )));
    menu_items.push(MenuItem::Separator);
    menu_items.push(MenuItem::Custom(CustomMenuItem::new(
        i18n::tr(ctx, I18nKey::MenuLogOut),
        auth::maybe_log_out,
        move |_, ctx| {
            let is_anonymous = AuthStateProvider::handle(ctx)
                .as_ref(ctx)
                .get()
                .is_anonymous_or_logged_out();
            MenuItemPropertyChanges {
                disabled: Some(is_anonymous),
                ..Default::default()
            }
        },
        None,
    )));
    menu_items.push(MenuItem::Standard(StandardAction::Quit));
    Menu::new("Warp", menu_items)
}

fn make_new_file_menu(ctx: &AppContext) -> Menu {
    let mut file_menu_options = make_new_elements_menu_items(ctx);
    file_menu_options.extend([
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::OpenRepository, ctx),
        MenuItem::Custom(CustomMenuItem::new_with_submenu(
            i18n::tr(ctx, I18nKey::MenuOpenRecent),
            |_| (),
            |_props, ctx| {
                let recent_repos = generate_recent_repos_for_menu(ctx);
                MenuItemPropertyChanges {
                    submenu: Some(Some(make_recent_repos_menu_items(ctx))),
                    disabled: Some(recent_repos.is_empty()),
                    ..Default::default()
                }
            },
            None,
            vec![],
        )),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::CloseCurrentSession, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CloseWindow, ctx),
    ]);

    Menu::new(i18n::tr(ctx, I18nKey::MenuFile), file_menu_options)
}

fn make_new_edit_menu(ctx: &AppContext) -> Menu {
    let mut edit_menu_items = vec![];

    let group_1 = vec![
        updateable_custom_item_without_checkmark(CustomAction::Undo, ctx),
        updateable_custom_item_without_checkmark(CustomAction::Redo, ctx),
    ];
    let group_2 = vec![
        updateable_custom_item_without_checkmark(CustomAction::Cut, ctx),
        updateable_custom_item_without_checkmark(CustomAction::Copy, ctx),
        updateable_custom_item_without_checkmark(CustomAction::Paste, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SelectAll, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ClearEditor, ctx),
    ];
    let group_3 = vec![
        updateable_custom_item_without_checkmark(CustomAction::AddNextOccurrence, ctx),
        updateable_custom_item_without_checkmark(CustomAction::AddCursorAbove, ctx),
        updateable_custom_item_without_checkmark(CustomAction::AddCursorBelow, ctx),
    ];
    let group_4 = vec![
        updateable_custom_item_without_checkmark(CustomAction::Find, ctx),
        updateable_custom_item_without_checkmark(CustomAction::GoToLine, ctx),
        updateable_custom_item_without_checkmark(CustomAction::FocusInput, ctx),
    ];
    let group_5 = vec![
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuUseWarpsPrompt),
            move |ctx| ctx.dispatch_global_action("app:toggle_user_ps1", &()),
            move |_props, ctx| MenuItemPropertyChanges {
                checked: Some(
                    SessionSettings::handle(ctx).read(ctx, |session_settings, _ctx| {
                        !session_settings.honor_ps1.value()
                    }),
                ),
                ..Default::default()
            },
            None,
        )),
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuCopyOnSelectWithinTerminal),
            move |ctx| {
                ctx.dispatch_global_action("app:toggle_copy_on_select", &());
            },
            move |_props, ctx| MenuItemPropertyChanges {
                checked: Some(
                    SelectionSettings::handle(ctx)
                        .as_ref(ctx)
                        .copy_on_select_enabled(),
                ),
                ..Default::default()
            },
            None,
        )),
    ];

    edit_menu_items.extend(group_1);
    edit_menu_items.push(MenuItem::Separator);
    edit_menu_items.extend(group_2);
    edit_menu_items.push(MenuItem::Separator);
    edit_menu_items.extend(group_3);
    edit_menu_items.push(MenuItem::Separator);
    edit_menu_items.extend(group_4);
    edit_menu_items.push(MenuItem::Separator);

    edit_menu_items.push(MenuItem::Custom(CustomMenuItem::new_with_submenu(
        i18n::tr(ctx, I18nKey::MenuSynchronizeInputs),
        |_| (),
        no_updates,
        None,
        vec![
            updateable_custom_item_without_checkmark(
                CustomAction::ToggleSyncAllTerminalInputsInAllTabs,
                ctx,
            ),
            updateable_custom_item_without_checkmark(
                CustomAction::ToggleSyncTerminalInputsInCurrentTab,
                ctx,
            ),
            updateable_custom_item_with_checkmark(
                CustomAction::DisableSyncTerminalInputs,
                ctx,
                Box::new(|ctx| {
                    if let Some(window_id) = WindowManager::handle(ctx).as_ref(ctx).active_window()
                    {
                        SyncedInputState::handle(ctx)
                            .read(ctx, |status, _| !status.is_syncing_any_inputs(window_id))
                    } else {
                        false
                    }
                }),
            ),
        ],
    )));
    edit_menu_items.push(MenuItem::Separator);

    edit_menu_items.extend(group_5);

    Menu::new(i18n::tr(ctx, I18nKey::MenuEdit), edit_menu_items)
}

fn make_new_view_menu(ctx: &AppContext) -> Menu {
    let mut items = vec![
        updateable_custom_item_without_checkmark(CustomAction::ToggleWarpDrive, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::CommandPalette, ctx),
        updateable_custom_item_without_checkmark(CustomAction::NavigationPalette, ctx),
        updateable_custom_item_without_checkmark(CustomAction::LaunchConfigPalette, ctx),
        updateable_custom_item_without_checkmark(CustomAction::FilesPalette, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ToggleConversationListView, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ToggleProjectExplorer, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ToggleGlobalSearch, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::History, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CommandSearch, ctx),
        updateable_custom_item_without_checkmark(CustomAction::Workflows, ctx),
        MenuItem::Separator,
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuToggleMouseReporting),
            move |ctx| {
                ctx.dispatch_global_action("workspace:toggle_mouse_reporting", &());
            },
            move |_props, ctx| {
                let mouse_reporting_enabled = AltScreenReporting::handle(ctx)
                    .as_ref(ctx)
                    .mouse_reporting_enabled
                    .value();
                MenuItemPropertyChanges {
                    checked: Some(*mouse_reporting_enabled),
                    ..Default::default()
                }
            },
            None,
        )),
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuToggleScrollReporting),
            move |ctx| {
                ctx.dispatch_global_action("workspace:toggle_scroll_reporting", &());
            },
            move |_props, ctx| {
                let reporting = AltScreenReporting::handle(ctx).as_ref(ctx);
                MenuItemPropertyChanges {
                    disabled: Some(!*reporting.mouse_reporting_enabled.value()),
                    checked: Some(*reporting.scroll_reporting_enabled.value()),
                    ..Default::default()
                }
            },
            None,
        )),
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuToggleFocusReporting),
            move |ctx| {
                ctx.dispatch_global_action("workspace:toggle_focus_reporting", &());
            },
            move |_props, ctx| {
                let reporting = AltScreenReporting::handle(ctx).as_ref(ctx);
                MenuItemPropertyChanges {
                    checked: Some(*reporting.focus_reporting_enabled.value()),
                    ..Default::default()
                }
            },
            None,
        )),
    ];

    let is_compact_mode = matches!(
        TerminalSettings::handle(ctx)
            .as_ref(ctx)
            .spacing_mode
            .value(),
        SpacingMode::Compact
    );

    items.extend([
        MenuItem::Separator,
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuCompactMode),
            move |ctx| {
                TerminalSettings::handle(ctx).update(ctx, |terminal_settings, ctx| {
                    let current_value = *terminal_settings.spacing_mode;
                    report_if_error!(terminal_settings
                        .spacing_mode
                        .set_value(current_value.other_mode(), ctx));
                });
            },
            move |_props, _| MenuItemPropertyChanges {
                checked: Some(is_compact_mode),
                ..Default::default()
            },
            None,
        )),
        MenuItem::Separator,
    ]);

    if FeatureFlag::UIZoom.is_enabled() {
        items.extend([
            updateable_custom_item_without_checkmark(CustomAction::IncreaseZoom, ctx),
            updateable_custom_item_without_checkmark(CustomAction::DecreaseZoom, ctx),
            updateable_custom_item_without_checkmark(CustomAction::ResetZoom, ctx),
            MenuItem::Separator,
        ]);
    } else {
        items.extend([
            updateable_custom_item_without_checkmark(CustomAction::IncreaseFontSize, ctx),
            updateable_custom_item_without_checkmark(CustomAction::DecreaseFontSize, ctx),
            updateable_custom_item_without_checkmark(CustomAction::ResetFontSize, ctx),
            MenuItem::Separator,
        ]);
    }

    Menu::new(i18n::tr(ctx, I18nKey::MenuView), items)
}

fn make_new_tab_menu(ctx: &AppContext) -> Menu {
    let items = vec![
        updateable_custom_item_without_checkmark(CustomAction::RenameTab, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::SplitPaneRight, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SplitPaneLeft, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SplitPaneDown, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SplitPaneUp, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::MoveTabLeft, ctx),
        updateable_custom_item_without_checkmark(CustomAction::MoveTabRight, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::CycleNextSession, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CyclePrevSession, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ActivateNextPane, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ActivatePreviousPane, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ToggleMaximizePane, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::CloseTab, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CloseOtherTabs, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CloseTabsRight, ctx),
    ];
    Menu::new(i18n::tr(ctx, I18nKey::MenuTab), items)
}

fn make_new_ai_menu(ctx: &AppContext) -> Menu {
    let mut items = vec![updateable_custom_item_without_checkmark(
        CustomAction::NewAgentModePane,
        ctx,
    )];

    items.push(updateable_custom_item_without_checkmark(
        CustomAction::AttachSelectionAsAgentModeContext,
        ctx,
    ));

    items.extend([
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::AISearch, ctx),
    ]);

    if FeatureFlag::AIRules.is_enabled() {
        items.extend([
            MenuItem::Separator,
            updateable_custom_item_without_checkmark(CustomAction::OpenAIFactCollection, ctx),
        ]);
    }

    if FeatureFlag::McpServer.is_enabled() && ContextFlag::ShowMCPServers.is_enabled() {
        items.push(updateable_custom_item_without_checkmark(
            CustomAction::OpenMCPServerCollection,
            ctx,
        ));
    }

    Menu::new(i18n::tr(ctx, I18nKey::MenuAi), items)
}

fn make_new_blocks_menu(ctx: &AppContext) -> Menu {
    let mut items = vec![
        updateable_custom_item_without_checkmark(CustomAction::ClearBlocks, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::SelectBlockAbove, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SelectBlockBelow, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SelectAllBlocks, ctx),
        MenuItem::Separator,
    ];
    items.push(updateable_custom_item_without_checkmark(
        CustomAction::ScrollToTopOfSelectedBlocks,
        ctx,
    ));
    items.push(updateable_custom_item_without_checkmark(
        CustomAction::ScrollToBottomOfSelectedBlocks,
        ctx,
    ));
    items.push(MenuItem::Separator);
    items.extend([
        updateable_custom_item_without_checkmark(CustomAction::CreateBlockPermalink, ctx),
        non_updateable_custom_item(CustomAction::ViewSharedBlocks, ctx),
        updateable_custom_item_without_checkmark(CustomAction::ToggleBookmarkBlock, ctx),
        updateable_custom_item_without_checkmark(CustomAction::FindWithinBlock, ctx),
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::CopyBlock, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CopyBlockCommand, ctx),
        updateable_custom_item_without_checkmark(CustomAction::CopyBlockOutput, ctx),
    ]);

    let debug_items = block_menu_debug_items();
    if !debug_items.is_empty() {
        items.push(MenuItem::Separator);
        items.extend(debug_items);
    }

    Menu::new(i18n::tr(ctx, I18nKey::MenuBlocks), items)
}

fn make_new_drive_menu(ctx: &AppContext) -> Menu {
    let mut items = vec![
        updateable_custom_item_without_checkmark(CustomAction::NewPersonalWorkflow, ctx),
        updateable_custom_item_without_checkmark(CustomAction::NewPersonalNotebook, ctx),
        updateable_custom_item_without_checkmark(CustomAction::NewPersonalAIPrompt, ctx),
    ];
    items.push(updateable_custom_item_without_checkmark(
        CustomAction::NewPersonalEnvVars,
        ctx,
    ));
    items.extend([
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::NewTeamWorkflow, ctx),
        updateable_custom_item_without_checkmark(CustomAction::NewTeamNotebook, ctx),
        updateable_custom_item_without_checkmark(CustomAction::NewTeamAIPrompt, ctx),
    ]);
    items.push(updateable_custom_item_without_checkmark(
        CustomAction::NewTeamEnvVars,
        ctx,
    ));
    items.extend([
        MenuItem::Separator,
        updateable_custom_item_without_checkmark(CustomAction::ToggleWarpDrive, ctx),
        updateable_custom_item_without_checkmark(CustomAction::SearchDrive, ctx),
        updateable_custom_item_without_checkmark(CustomAction::OpenTeamSettings, ctx),
        updateable_custom_item_without_checkmark(CustomAction::OpenAIFactCollection, ctx),
        updateable_custom_item_without_checkmark(CustomAction::OpenMCPServerCollection, ctx),
    ]);

    items.push(updateable_custom_item_without_checkmark(
        CustomAction::SharePaneContents,
        ctx,
    ));

    if FeatureFlag::CreatingSharedSessions.is_enabled() {
        items.extend([
            MenuItem::Separator,
            updateable_custom_item_without_checkmark(CustomAction::ShareCurrentSession, ctx),
        ])
    }

    Menu::new(i18n::tr(ctx, I18nKey::MenuDrive), items)
}

/// Returns [`MenuItem`]s that aid debugging to be included in the Block menu.
fn block_menu_debug_items() -> Vec<MenuItem> {
    let mut items = vec![];
    if FeatureFlag::ToggleBootstrapBlock.is_enabled() {
        items.push(toggle_bootstrap_block_menu_item());
    }

    items.push(MenuItem::Custom(CustomMenuItem::new(
        SHOW_IN_BAND_COMMAND_BLOCKS_MENU_ITEM_NAME,
        move |ctx| {
            let handle = BlockVisibilitySettings::handle(ctx);
            handle.update(ctx, |block_visibility_settings, ctx| {
                let new_value = !block_visibility_settings
                    .should_show_in_band_command_blocks
                    .value();
                if let Err(e) = block_visibility_settings
                    .should_show_in_band_command_blocks
                    .set_value(new_value, ctx)
                {
                    log::error!("Failed to persist 'Show in-band command blocks' setting: {e}");
                }
            });
        },
        move |_props, ctx| {
            let name = if BlockVisibilitySettings::handle(ctx).read(ctx, |settings, _ctx| {
                *settings.should_show_in_band_command_blocks.value()
            }) {
                HIDE_IN_BAND_COMMAND_BLOCKS_MENU_ITEM_NAME.to_owned()
            } else {
                SHOW_IN_BAND_COMMAND_BLOCKS_MENU_ITEM_NAME.to_owned()
            };

            MenuItemPropertyChanges {
                name: Some(name),
                ..Default::default()
            }
        },
        None,
    )));

    items.push(MenuItem::Custom(CustomMenuItem::new(
        SHOW_SSH_COMMAND_BLOCKS_MENU_ITEM_NAME,
        move |ctx| {
            let handle = BlockVisibilitySettings::handle(ctx);
            handle.update(ctx, |block_visibility_settings, ctx| {
                let new_value = !block_visibility_settings.should_show_ssh_block.value();
                if let Err(e) = block_visibility_settings
                    .should_show_ssh_block
                    .set_value(new_value, ctx)
                {
                    log::error!("Failed to persist 'Show ssh command blocks' setting: {e}");
                }
            });
        },
        move |_props, ctx| {
            let name = if BlockVisibilitySettings::handle(ctx).read(ctx, |settings, _ctx| {
                *settings.should_show_ssh_block.value()
            }) {
                HIDE_SSH_COMMAND_BLOCKS_MENU_ITEM_NAME.to_owned()
            } else {
                SHOW_SSH_COMMAND_BLOCKS_MENU_ITEM_NAME.to_owned()
            };

            MenuItemPropertyChanges {
                name: Some(name),
                ..Default::default()
            }
        },
        None,
    )));

    items
}

fn toggle_bootstrap_block_menu_item() -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        SHOW_BOOTSTRAP_BLOCK_MENU_ITEM_NAME,
        move |ctx| {
            BlockVisibilitySettings::handle(ctx).update(ctx, |block_visibility_settings, ctx| {
                let new_value = !block_visibility_settings
                    .should_show_bootstrap_block
                    .value();
                if let Err(e) = block_visibility_settings
                    .should_show_bootstrap_block
                    .set_value(new_value, ctx)
                {
                    log::error!("Failed to persist 'Show bootstrap block' setting: {e}");
                }
            });
        },
        move |_props, ctx| {
            let name = if BlockVisibilitySettings::handle(ctx).read(ctx, |settings, _ctx| {
                *settings.should_show_bootstrap_block.value()
            }) {
                HIDE_BOOTSTRAP_BLOCK_MENU_ITEM_NAME.to_owned()
            } else {
                SHOW_BOOTSTRAP_BLOCK_MENU_ITEM_NAME.to_owned()
            };

            MenuItemPropertyChanges {
                name: Some(name),
                ..Default::default()
            }
        },
        None,
    ))
}

fn make_new_window_menu(ctx: &AppContext) -> Menu {
    Menu::new(
        i18n::tr(ctx, I18nKey::MenuWindow),
        vec![
            MenuItem::Standard(StandardAction::Minimize),
            MenuItem::Standard(StandardAction::Zoom),
            MenuItem::Standard(StandardAction::ToggleFullScreen),
            MenuItem::Separator,
            MenuItem::Standard(StandardAction::BringAllToFront),
        ],
    )
}

fn debug_menu_items() -> Vec<MenuItem> {
    let mut debug_menu_items = vec![];

    if FeatureFlag::DebugMode.is_enabled() {
        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            ENABLE_SHELL_DEBUG_MODE_MENU_ITEM_NAME,
            move |ctx| {
                DebugSettings::handle(ctx).update(ctx, |debug_settings, ctx| {
                    let new_value = !debug_settings.is_shell_debug_mode_enabled.value();
                    if let Err(e) = debug_settings
                        .is_shell_debug_mode_enabled
                        .set_value(new_value, ctx)
                    {
                        log::error!("Failed to persist 'Debug mode' setting: {e}");
                    }
                });
            },
            move |_props, ctx| {
                let name = if DebugSettings::handle(ctx).read(ctx, |settings, _ctx| {
                    *settings.is_shell_debug_mode_enabled.value()
                }) {
                    DISABLE_SHELL_DEBUG_MODE_MENU_ITEM_NAME.to_owned()
                } else {
                    ENABLE_SHELL_DEBUG_MODE_MENU_ITEM_NAME.to_owned()
                };

                MenuItemPropertyChanges {
                    name: Some(name),
                    ..Default::default()
                }
            },
            None,
        )));

        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            ENABLE_PTY_RECORDING,
            move |ctx| {
                DebugSettings::handle(ctx).update(ctx, |debug_settings, ctx| {
                    let new_value = !debug_settings.recording_mode.value();
                    let _ = debug_settings.recording_mode.set_value(new_value, ctx);
                });
            },
            move |_props, ctx| {
                let name = if DebugSettings::handle(ctx)
                    .read(ctx, |settings, _ctx| *settings.recording_mode.value())
                {
                    DISABLE_PTY_RECORDING.to_owned()
                } else {
                    ENABLE_PTY_RECORDING.to_owned()
                };

                MenuItemPropertyChanges {
                    name: Some(name),
                    ..Default::default()
                }
            },
            None,
        )));

        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            ENABLE_IN_BAND_GENERATORS_MENU_ITEM_NAME,
            move |ctx| {
                DebugSettings::handle(ctx).update(ctx, |debug_settings, ctx| {
                    let new_value = !debug_settings
                        .are_in_band_generators_for_all_sessions_enabled
                        .value();
                    if let Err(e) = debug_settings
                        .are_in_band_generators_for_all_sessions_enabled
                        .set_value(new_value, ctx)
                    {
                        log::error!("Failed to persist 'Enable in-band generators' setting: {e}");
                    }
                });
            },
            move |_props, ctx| {
                let name = if DebugSettings::handle(ctx).read(ctx, |settings, _ctx| {
                    *settings
                        .are_in_band_generators_for_all_sessions_enabled
                        .value()
                }) {
                    DISABLE_IN_BAND_GENERATORS_MENU_ITEM_NAME.to_owned()
                } else {
                    ENABLE_IN_BAND_GENERATORS_MENU_ITEM_NAME.to_owned()
                };

                MenuItemPropertyChanges {
                    name: Some(name),
                    ..Default::default()
                }
            },
            None,
        )));

        if !FeatureFlag::ToggleBootstrapBlock.is_enabled() {
            debug_menu_items.push(toggle_bootstrap_block_menu_item());
        }

        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            "Manually Toggle Network Status",
            move |ctx| ctx.dispatch_global_action("workspace:toggle_debug_network_status", &()),
            no_updates,
            None,
        )));

        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            EXPORT_DEFAULT_SETTINGS_CSV_MENU_ITEM_NAME,
            move |ctx| {
                let default_settings = SettingsManager::handle(ctx).as_ref(ctx).default_values();
                let mut writer = Writer::from_writer(
                    File::create(
                        dirs::home_dir()
                            .unwrap_or_default()
                            .join(SETTINGS_CSV_FILE_NAME),
                    )
                    .expect("Failed to create settings csv file"),
                );
                writer
                    .write_record(["storage_key", "value"])
                    .expect("Failed to write header record");
                for (storage_key, value) in default_settings {
                    log::debug!("Writing setting: {storage_key} = {value}");
                    writer
                        .write_record(&[storage_key, value])
                        .expect("Failed to write settings record");
                }
                let _ = writer.flush();
            },
            no_updates,
            None,
        )));

        debug_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            "Create anonymous user",
            move |ctx| ctx.dispatch_global_action("workspace:debug_create_anonymous_user", &()),
            no_updates,
            None,
        )));
    }

    if FeatureFlag::RuntimeFeatureFlags.is_enabled() {
        debug_menu_items.extend(runtime_flags_menu_items());
    }

    debug_menu_items
}

fn link_menu_item(title: &'static str, link: Cow<'static, str>) -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        title,
        move |ctx| {
            ctx.open_url(&link);
        },
        no_updates,
        None,
    ))
}

fn feedback_menu_item(ctx: &AppContext) -> MenuItem {
    MenuItem::Custom(CustomMenuItem::new(
        i18n::tr(ctx, I18nKey::MenuSendFeedback),
        move |ctx| {
            // Route through the root-view action so workspace windows can open the
            // guided AI flow, while non-workspace windows still fall back to the
            // browser-based feedback form.
            ctx.dispatch_global_action("root_view:send_feedback", &());
        },
        no_updates,
        None,
    ))
}

fn make_new_help_menu(ctx: &AppContext) -> Menu {
    Menu::new(
        i18n::tr(ctx, I18nKey::MenuHelp),
        vec![
            feedback_menu_item(ctx),
            link_menu_item(
                i18n::tr(ctx, I18nKey::MenuWarpDocumentation),
                links::USER_DOCS_URL.into(),
            ),
            link_menu_item(
                i18n::tr(ctx, I18nKey::MenuGithubIssues),
                links::GITHUB_ISSUES_URL.into(),
            ),
            link_menu_item(
                i18n::tr(ctx, I18nKey::MenuWarpSlackCommunity),
                links::SLACK_URL.into(),
            ),
        ],
    )
}

fn make_launch_config_menu_items(ctx: &mut AppContext) -> Vec<MenuItem> {
    let mut launch_config_menu_items = vec![];

    let launch_configs = WarpConfig::handle(ctx).as_ref(ctx).launch_configs();
    for config in launch_configs {
        launch_config_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
            &config.name,
            enclose!((config) move |ctx| {
                ctx.dispatch_global_action(
                    "root_view:open_launch_config",
                    &OpenLaunchConfigArg {
                        launch_config: config.clone(),
                        ui_location: LaunchConfigUiLocation::AppMenu,
                        open_in_active_window: false,
                    }
                );
                ctx.dispatch_global_action("workspace:save_app", &());
            }),
            no_updates,
            None,
        )));
    }

    if !launch_config_menu_items.is_empty() {
        launch_config_menu_items.push(MenuItem::Separator);
    }

    // TODO(vorporeal): use non_updateable_custom_item() here instead
    launch_config_menu_items.push(MenuItem::Custom(CustomMenuItem::new(
        i18n::tr(ctx, I18nKey::MenuSaveNew),
        custom_action_dispatcher(CustomAction::SaveCurrentConfig),
        no_updates,
        custom_shortcut(CustomAction::SaveCurrentConfig),
    )));

    launch_config_menu_items
}

fn make_new_elements_menu_items(ctx: &AppContext) -> Vec<MenuItem> {
    // Dynamically assign the workspace:new_tab keystroke (cmd-t) to whichever item
    // matches the user's "Default mode for new sessions" setting. The non-default item
    // shows its dedicated keystroke instead.
    let mut new_elements_menu = vec![
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuNewWindow),
            open_new_window,
            no_updates,
            Some(Keystroke::parse("cmd-n").expect("Valid keystroke")),
        )),
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuNewTerminalTab),
            open_new_default_tab_or_window,
            move |_props: &MenuItemProperties, ctx: &mut AppContext| {
                let mut changes = MenuItemPropertyChanges::default();
                let is_default_session_mode_agent =
                    AISettings::handle(ctx).read(ctx, |ai_settings, ctx| {
                        ai_settings.is_any_ai_enabled(ctx)
                            && ai_settings.default_session_mode(ctx) == DefaultSessionMode::Agent
                    });
                let trigger = if is_default_session_mode_agent {
                    Trigger::Custom(CustomAction::NewTerminalTab.into())
                } else {
                    Trigger::Custom(CustomAction::NewTab.into())
                };
                let binding = ctx
                    .get_key_bindings()
                    .find(|b| b.trigger == &trigger || b.original_trigger == Some(&trigger));
                if let Some(binding) = binding {
                    changes.keystroke = Some(bindings::trigger_to_keystroke(binding.trigger));
                }
                changes
            },
            Some(Keystroke::parse("cmd-t").expect("Valid keystroke")),
        )),
        MenuItem::Custom(CustomMenuItem::new(
            i18n::tr(ctx, I18nKey::MenuNewAgentTab),
            open_new_agent_tab_or_window,
            move |_props: &MenuItemProperties, ctx: &mut AppContext| {
                let mut changes = MenuItemPropertyChanges::default();
                let (is_any_ai_enabled, is_default_session_mode_agent) = AISettings::handle(ctx)
                    .read(ctx, |ai_settings, ctx| {
                        let enabled = ai_settings.is_any_ai_enabled(ctx);
                        let agent = enabled
                            && ai_settings.default_session_mode(ctx) == DefaultSessionMode::Agent;
                        (enabled, agent)
                    });
                if !is_any_ai_enabled {
                    changes.disabled = Some(true);
                    return changes;
                }
                let trigger = if is_default_session_mode_agent {
                    Trigger::Custom(CustomAction::NewTab.into())
                } else {
                    Trigger::Custom(CustomAction::NewAgentTab.into())
                };
                let binding = ctx
                    .get_key_bindings()
                    .find(|b| b.trigger == &trigger || b.original_trigger == Some(&trigger));
                if let Some(binding) = binding {
                    changes.keystroke = Some(bindings::trigger_to_keystroke(binding.trigger));
                }
                changes
            },
            None,
        )),
        non_updateable_custom_item(CustomAction::NewFile, ctx),
    ];

    let reopen_session_action_updater =
        custom_action_updater(CustomAction::ReopenClosedSession, Box::new(|_| false));
    new_elements_menu.push(MenuItem::Custom(CustomMenuItem::new(
        i18n::tr(ctx, I18nKey::CommonReopenClosedSession),
        |ctx| {
            UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| {
                stack.undo_close(ctx);
            });
        },
        move |props, ctx| {
            let mut changes = reopen_session_action_updater(props, ctx);
            changes.disabled = Some(UndoCloseStack::handle(ctx).as_ref(ctx).is_empty());
            changes
        },
        Some(Keystroke::parse("cmd-shift-T").expect("Valid keystroke")),
    )));

    new_elements_menu.push(MenuItem::Custom(CustomMenuItem::new_with_submenu(
        i18n::tr(ctx, I18nKey::MenuLaunchConfigurations),
        |_| (),
        |_props, ctx| MenuItemPropertyChanges {
            submenu: Some(Some(make_launch_config_menu_items(ctx))),
            ..Default::default()
        },
        None,
        vec![],
    )));

    new_elements_menu
}

/// \return a callback that dispatches a CustomAction, appropriate for plugging into a CustomMenuItem.
fn custom_action_dispatcher(action: CustomAction) -> impl Fn(&mut AppContext) + 'static {
    move |ctx| {
        if let Some(wid) = WindowManager::handle(ctx).as_ref(ctx).active_window() {
            ctx.dispatch_custom_action(action, wid)
        }
    }
}

/// Dispatch events to open the user's default tab type in the active window
/// or make a new window if there is no active window.
fn open_new_default_tab_or_window(ctx: &mut AppContext) {
    if let Some(wid) = WindowManager::handle(ctx).as_ref(ctx).active_window() {
        ctx.dispatch_custom_action(CustomAction::NewTab, wid)
    } else {
        open_new_window(ctx)
    }
}

/// Dispatch events to open an agent tab in the active window
/// or make a new window if there is no active window.
fn open_new_agent_tab_or_window(ctx: &mut AppContext) {
    if let Some(wid) = WindowManager::handle(ctx).as_ref(ctx).active_window() {
        ctx.dispatch_custom_action(CustomAction::NewAgentTab, wid)
    } else {
        open_new_window(ctx)
    }
}

/// Dispatch event to open a new Warp window
fn open_new_window(ctx: &mut AppContext) {
    ctx.dispatch_global_action("root_view:open_new", &());
    ctx.dispatch_global_action("workspace:save_app", &());
}

/// No-op updater function for custom menu items that never change.
fn no_updates(_: &MenuItemProperties, _: &mut AppContext) -> MenuItemPropertyChanges {
    Default::default()
}

fn make_recent_repos_menu_items(ctx: &AppContext) -> Vec<MenuItem> {
    let recent_repos = generate_recent_repos_for_menu(ctx);

    if recent_repos.is_empty() {
        return vec![];
    }

    let home = dirs::home_dir().map(|p| p.display().to_string());

    recent_repos
        .into_iter()
        .map(|path| {
            let full_path = path.display().to_string();
            let display_path = user_friendly_path(&full_path, home.as_deref()).into_owned();

            MenuItem::Custom(CustomMenuItem::new(
                &display_path,
                move |ctx| {
                    ctx.dispatch_global_action("workspace:open_repository", &full_path);
                },
                no_updates,
                None,
            ))
        })
        .collect()
}

fn generate_recent_repos_for_menu(ctx: &AppContext) -> Vec<PathBuf> {
    PersistedWorkspace::handle(ctx)
        .as_ref(ctx)
        .workspaces()
        .sorted_by(WorkspaceMetadata::most_recently_navigated)
        .take(MAX_RECENT_REPOS_IN_MENU)
        .map(|cbm| cbm.path)
        .collect::<Vec<_>>()
}

/// \return a callback that updates a custom action based menu item based on the
/// current keybinding context:
/// 1) Whether the item should be enabled / disabled
/// 2) The current keystroke bound to the item (either the default or a custom one)
/// 3) The name of the item
///
/// This updated is called by the mac menu system before the menu is opened.
///
/// Note, that correctly disabling the key binding is important not just for limiting
/// what a user can do in a given context, but also because if the keybinding is not
/// disabled correctly, the mac menu system will capture key events when they should
/// be flowing into the underlying views (e.g. ctrl-c needs to flow directly to the
/// alt grid in emacs, whereas if our editor is active it's ok to dispatch a custom action)
fn custom_action_updater(
    action: CustomAction,
    checkmark_status: Box<CheckmarkStatusGetter>,
) -> impl Fn(&MenuItemProperties, &mut AppContext) -> MenuItemPropertyChanges + 'static {
    move |_props, ctx| {
        let mut changes = MenuItemPropertyChanges::default();
        ctx.update_custom_action_binding(action.into(), |binding| {
            changes.disabled = Some(binding.is_none());
            if let Some(binding) = binding {
                if let Some(description) = binding.description {
                    let resolved_name = description.resolve(ctx, bindings::MAC_MENUS_CONTEXT);
                    changes.name = Some(
                        localized_custom_action_name(action, ctx)
                            .unwrap_or(resolved_name.as_ref())
                            .to_owned(),
                    );
                }
                changes.keystroke = Some(bindings::trigger_to_keystroke(binding.trigger));
            }
        });

        changes.checked = Some(checkmark_status(ctx));

        changes
    }
}
