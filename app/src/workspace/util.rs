use serde::{Deserialize, Serialize};
use warpui::{
    elements::MouseStateHandle, AppContext, EntityId, SingletonEntity, ViewContext, ViewHandle,
    WindowId,
};

use super::OneTimeModalModel;
use crate::window_settings::WindowSettings;
use crate::{
    appearance::Appearance, pane_group::PaneId, terminal::TerminalView, workspace::Workspace,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// What composes a pane (i.e. the pane group and the pane itself).
pub struct PaneViewLocator {
    pub pane_group_id: EntityId,
    pub pane_id: PaneId,
}

#[derive(Default)]
pub(super) struct WorkspaceMouseStates {
    pub(super) new_tab_button: MouseStateHandle,
    pub(super) new_tab_menu: MouseStateHandle,
    pub(super) new_tab: MouseStateHandle,
    pub(super) overflow_button: MouseStateHandle,
    pub(super) banner_button: MouseStateHandle,
    pub(super) banner_secondary_button: MouseStateHandle,
    pub(super) more_info_banner_button: MouseStateHandle,
    pub(super) resource_center_icon: MouseStateHandle,
    pub(super) ai_tab_bar_button: MouseStateHandle,
    pub(super) agent_management_view_button: MouseStateHandle,
    pub(super) left_panel_icon: MouseStateHandle,
    pub(super) settings_icon: MouseStateHandle,
    pub(super) dismiss_banner_button: MouseStateHandle,
    pub(super) sign_in_button: MouseStateHandle,
    pub(super) sign_up_button: MouseStateHandle,
    pub(super) offline_icon: MouseStateHandle,
    pub(super) avatar_icon: MouseStateHandle,
    pub(super) header_dimming: MouseStateHandle,
    pub(super) right_panel_icon: MouseStateHandle,
    pub(super) notifications_mailbox: MouseStateHandle,
    pub(super) session_config_tab_config_chip_close: MouseStateHandle,
    pub(super) tools_panel_icon: MouseStateHandle,
    pub(super) title_bar_search_bar: MouseStateHandle,
    #[cfg(target_family = "wasm")]
    pub(super) warp_logo: MouseStateHandle,
}

#[derive(Debug)]
pub enum WelcomeTipsViewState {
    Unavailable,
    Available { is_popup_open: bool },
}

impl WelcomeTipsViewState {
    pub fn is_popup_open(&self) -> bool {
        matches!(
            self,
            WelcomeTipsViewState::Available {
                is_popup_open: true,
                ..
            }
        )
    }

    pub fn close_popup(&mut self) {
        if let WelcomeTipsViewState::Available {
            ref mut is_popup_open,
            ..
        } = self
        {
            *is_popup_open = false;
        }
    }

    pub fn toggle_popup(&mut self) {
        if let WelcomeTipsViewState::Available {
            ref mut is_popup_open,
            ..
        } = self
        {
            *is_popup_open = !*is_popup_open;
        }
    }
}

// TODO change this struct to enum (as we can only have 1 of them set to true at a time)
#[derive(Clone, Copy, Debug, Default)]
pub struct WorkspaceState {
    pub is_palette_open: bool,
    pub is_ctrl_tab_palette_open: bool,
    pub is_theme_chooser_open: bool,
    pub is_theme_creator_modal_open: bool,
    pub is_theme_deletion_modal_open: bool,
    pub is_changelog_modal_open: bool,
    pub is_tab_being_dragged: bool,
    pub is_reward_modal_open: bool,
    pub is_launch_config_save_modal_open: bool,
    pub is_resource_center_open: bool,
    pub is_command_search_open: bool,
    pub is_warp_drive_open: bool,
    pub is_ai_assistant_panel_open: bool,
    pub is_agent_management_popup_open: bool,
    pub is_auth_override_modal_open: bool,
    pub is_require_login_modal_open: bool,
    pub is_workflow_modal_open: bool,
    pub is_prompt_editor_open: bool,
    pub is_agent_toolbar_editor_open: bool,
    pub is_header_toolbar_editor_open: bool,
    pub is_import_modal_open: bool,
    pub is_close_session_confirmation_dialog_open: bool,
    pub is_rewind_confirmation_dialog_open: bool,
    pub is_delete_conversation_confirmation_dialog_open: bool,
    pub is_native_quit_modal_open: bool,
    pub is_shared_objects_creation_denied_modal_open: bool,
    pub is_suggested_agent_mode_workflow_modal_open: bool,
    pub is_suggested_rule_modal_open: bool,
    pub is_enable_auto_reload_modal_open: bool,
    pub is_notification_mailbox_open: bool,
    pub is_agent_management_view_open: bool,
    pub is_codex_modal_open: bool,
    pub is_cloud_agent_capacity_modal_open: bool,
    pub is_free_tier_limit_hit_modal_open: bool,
    pub is_tab_config_params_modal_open: bool,
    pub is_session_config_modal_open: bool,
    pub is_new_worktree_modal_open: bool,
    pub is_remove_tab_config_dialog_open: bool,
    /// Whether the transcript details panel is open (WASM only, for conversation transcript viewing).
    pub is_transcript_details_panel_open: bool,
    tab_being_renamed: Option<usize>, // The index of the tab being renamed
    pane_being_renamed: Option<PaneViewLocator>,
}

impl WorkspaceState {
    pub fn is_any_non_terminal_view_open(&self, app: &AppContext) -> bool {
        self.is_any_modal_open(app)
            || self.is_theme_chooser_open
            || self.is_ai_assistant_panel_open
            || self.is_workflow_modal_open
            || self.is_warp_drive_open
    }

    pub fn is_any_non_palette_modal_open(&self, app: &AppContext) -> bool {
        self.is_theme_creator_modal_open
            || self.is_theme_deletion_modal_open
            || self.is_changelog_modal_open
            || self.tab_being_renamed.is_some()
            || self.pane_being_renamed.is_some()
            || self.is_reward_modal_open
            || self.is_launch_config_save_modal_open
            || self.is_command_search_open
            || self.is_prompt_editor_open
            || self.is_agent_toolbar_editor_open
            || self.is_header_toolbar_editor_open
            || self.is_agent_management_popup_open
            || self.is_import_modal_open
            || self.is_shared_objects_creation_denied_modal_open
            || self.is_suggested_rule_modal_open
            || self.is_suggested_agent_mode_workflow_modal_open
            || self.is_enable_auto_reload_modal_open
            || self.is_codex_modal_open
            || self.is_cloud_agent_capacity_modal_open
            || self.is_free_tier_limit_hit_modal_open
            || self.is_tab_config_params_modal_open
            || self.is_session_config_modal_open
            || self.is_new_worktree_modal_open
            || self.is_remove_tab_config_dialog_open
            || {
                let one_time_modal = OneTimeModalModel::as_ref(app);
                one_time_modal.is_oz_launch_modal_open()
                    || one_time_modal.is_build_plan_migration_modal_open()
            }
    }

    /// Returns whether any modal (sitting over terminal views) is open.
    pub fn is_any_modal_open(&self, app: &AppContext) -> bool {
        self.is_any_non_palette_modal_open(app)
            || self.is_palette_open
            || self.is_ctrl_tab_palette_open
    }

    pub fn close_all_modals(&mut self) {
        self.is_palette_open = false;
        self.is_ctrl_tab_palette_open = false;
        self.is_theme_creator_modal_open = false;
        self.is_theme_deletion_modal_open = false;
        self.is_changelog_modal_open = false;
        self.tab_being_renamed = None;
        self.pane_being_renamed = None;
        self.is_reward_modal_open = false;
        self.is_launch_config_save_modal_open = false;
        self.is_command_search_open = false;
        self.is_workflow_modal_open = false;
        self.is_prompt_editor_open = false;
        self.is_agent_toolbar_editor_open = false;
        self.is_header_toolbar_editor_open = false;
        self.is_import_modal_open = false;
        self.is_shared_objects_creation_denied_modal_open = false;
        self.is_auth_override_modal_open = false;
        self.is_require_login_modal_open = false;
        self.is_suggested_rule_modal_open = false;
        self.is_suggested_agent_mode_workflow_modal_open = false;
        self.is_enable_auto_reload_modal_open = false;
        self.is_codex_modal_open = false;
        self.is_cloud_agent_capacity_modal_open = false;
        self.is_free_tier_limit_hit_modal_open = false;
        self.is_tab_config_params_modal_open = false;
        self.is_session_config_modal_open = false;
        self.is_new_worktree_modal_open = false;
        self.is_remove_tab_config_dialog_open = false;
    }

    pub fn is_right_panel_open(&self) -> bool {
        self.is_resource_center_open || self.is_ai_assistant_panel_open
    }

    pub fn is_left_panel_open(&self) -> bool {
        self.is_theme_chooser_open
    }

    pub fn close_all_left_panels(&mut self) {
        self.is_warp_drive_open = false;
        self.is_theme_chooser_open = false;
    }

    pub fn is_tab_being_renamed(&self) -> bool {
        self.tab_being_renamed.is_some()
    }

    pub fn set_tab_being_renamed(&mut self, index: usize) {
        self.tab_being_renamed = Some(index);
        self.pane_being_renamed = None;
    }

    pub fn clear_tab_being_renamed(&mut self) {
        self.tab_being_renamed = None;
    }

    pub fn tab_being_renamed(&self) -> Option<usize> {
        self.tab_being_renamed
    }

    pub fn is_pane_being_renamed(&self, pane: PaneViewLocator) -> bool {
        self.pane_being_renamed == Some(pane)
    }

    pub fn is_any_pane_being_renamed(&self) -> bool {
        self.pane_being_renamed.is_some()
    }

    pub fn set_pane_being_renamed(&mut self, pane: PaneViewLocator) {
        self.pane_being_renamed = Some(pane);
        self.tab_being_renamed = None;
    }

    pub fn clear_pane_being_renamed(&mut self) {
        self.pane_being_renamed = None;
    }

    pub fn pane_being_renamed(&self) -> Option<PaneViewLocator> {
        self.pane_being_renamed
    }
}

/// Used to represent left and right movement for tabs in WorkspaceActions
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize, Copy)]
pub enum TabMovement {
    Left,
    Right,
}

/// Fallback behavior for when a terminal input is needed, but none are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalSessionFallbackBehavior {
    /// Never open a new terminal session; fail if there is not an active session with no running
    /// command.
    RequireExisting,
    /// Open a new terminal session if and only if there is no active session. If the active
    /// session is busy, fail.
    #[default]
    OpenIfNone,
    /// Open a new terminal session if there is active session OR if the active session is busy.
    OpenIfNeeded,
}

/// Given a [`WindowId`], see if its [`Workspace`] contains an active [`TerminalView`] and return
/// that.
///
/// Note that "active" is not the same as "focused" in Warp's pane management.
pub fn active_terminal_in_window<T, F>(
    window_id: WindowId,
    ctx: &mut AppContext,
    update: F,
) -> Option<T>
where
    F: FnOnce(&mut TerminalView, &mut ViewContext<TerminalView>) -> T,
{
    ctx.views_of_type::<Workspace>(window_id)
        .as_ref()
        .and_then(|v| v.first())
        .and_then(|handle| {
            handle.update(ctx, |workspace, w_ctx| {
                workspace
                    .active_tab_pane_group()
                    .update(w_ctx, |active_group, a_ctx| {
                        active_group
                            .active_session_view(a_ctx)
                            .map(|terminal| terminal.update(a_ctx, update))
                    })
            })
        })
}

/// Check if two terminal views are in the same tab pane group.
///
/// Returns true if both terminal views are in the same tab, false otherwise.
pub fn is_terminal_view_in_same_tab(
    terminal_view_id_1: &EntityId,
    terminal_view_id_2: &EntityId,
    app: &AppContext,
) -> bool {
    if terminal_view_id_1 == terminal_view_id_2 {
        return true;
    }
    let Some(active_window) = app.windows().active_window() else {
        return false;
    };
    let Some(workspace) = app
        .views_of_type::<Workspace>(active_window)
        .and_then(|views| views.first().cloned())
    else {
        return false;
    };
    let workspace = workspace.as_ref(app);
    workspace
        .list_tab_pane_groups(app)
        .into_iter()
        .any(|tab_pane_group| {
            tab_pane_group.terminal_ids.contains(terminal_view_id_1)
                && tab_pane_group.terminal_ids.contains(terminal_view_id_2)
        })
}

/// Returns the active terminal session view in the active tab. This is used as the target for any
/// selections in the adjacent editor.
pub fn get_context_target_terminal_view(
    window_id: WindowId,
    ctx: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    ctx.views_of_type::<Workspace>(window_id)
        .as_ref()
        .and_then(|v| v.first())
        .and_then(|handle| {
            handle.read(ctx, |workspace, w_ctx| {
                workspace
                    .active_tab_pane_group()
                    .read(w_ctx, |active_group, a_ctx| {
                        active_group.active_session_view(a_ctx)
                    })
            })
        })
}

pub fn get_terminal_background_fill(
    window_id: WindowId,
    app: &AppContext,
) -> warpui::elements::Fill {
    let theme = Appearance::as_ref(app).theme();
    let terminal_opacity = get_terminal_background_opacity(window_id, app);
    theme.background().with_opacity(terminal_opacity).into()
}

fn get_terminal_background_opacity(window_id: WindowId, app: &AppContext) -> u8 {
    let theme = Appearance::as_ref(app).theme();
    let window_settings = WindowSettings::as_ref(app);
    let background_opacity = window_settings
        .background_opacity
        .effective_opacity(window_id, app);

    if theme.background_image().is_some() {
        let opacity_ratio = background_opacity as f32 / 100.;
        let overlay_opacity = *window_settings.background_image_overlay_opacity;
        // Scale the user-configured overlay opacity with the window opacity ratio.
        (overlay_opacity as f32 * opacity_ratio) as u8
    } else {
        background_opacity
    }
}
