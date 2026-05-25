//! Layout mutation handlers for local-control actions.
use ::local_control::protocol::{
    AppSurfaceParams, TargetSelector, WindowCloseParams, WindowCreateParams, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warpui::platform::TerminationMode;
use warpui::{ModelContext, TypedActionView, ViewHandle, WindowId};

use crate::local_control::resolver::{target_window_id, validate_tab_create_target};
use crate::local_control::LocalControlBridge;
use crate::palette::PaletteMode;
use crate::root_view;
use crate::server::telemetry::PaletteSource;
use crate::settings_view::SettingsSection;
use crate::workspace::{CommandSearchOptions, InitContent, Workspace, WorkspaceAction};

pub(crate) fn create_terminal_tab(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_tab_create_target(target)?;
    let window_id = target_window_id(ctx)?;
    let workspace = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "tab.create requires a workspace in the target window",
            )
        })?;
    let (previous_tab_count, tab_count, active_tab_index) =
        workspace.update(ctx, |workspace, ctx| {
            let previous_tab_count = workspace.tab_count();
            workspace.handle_action(
                &WorkspaceAction::AddTerminalTab {
                    hide_homepage: false,
                },
                ctx,
            );
            (
                previous_tab_count,
                workspace.tab_count(),
                workspace.active_tab_index(),
            )
        });
    Ok(json!({
        "action": ActionKind::TabCreate.as_str(),
        "created": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "window": {
            "selector": "active",
            "id": window_id.to_string(),
        },
        "tab": {
            "previous_count": previous_tab_count,
            "count": tab_count,
            "active_index": active_tab_index,
        },
    }))
}

pub(crate) fn focus_app(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_app_focus_target(target)?;
    let window_id = ctx.windows().activate_app();
    Ok(json!({
        "action": ActionKind::AppFocus.as_str(),
        "focused": true,
        "window_id": window_id.map(|id| id.to_string()),
    }))
}

pub(crate) fn create_window(
    target: &TargetSelector,
    params: WindowCreateParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_window_create_target(target, &params)?;
    let (window_id, _) = root_view::open_new_window_get_handles(None, ctx);
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::WindowCreate.as_str(),
        "created": true,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn focus_window(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let window_id = select_window_for_app_state_target(ActionKind::WindowFocus, target, ctx)?;
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::WindowFocus.as_str(),
        "focused": true,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn close_window(
    target: &TargetSelector,
    params: WindowCloseParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let window_id = select_window_for_app_state_target(ActionKind::WindowClose, target, ctx)?;
    let termination_mode = if params.force {
        TerminationMode::ForceTerminate
    } else {
        TerminationMode::Cancellable
    };
    ctx.windows().close_window(window_id, termination_mode);
    Ok(json!({
        "action": ActionKind::WindowClose.as_str(),
        "closed": true,
        "force": params.force,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn open_or_toggle_surface(
    action: ActionKind,
    target: &TargetSelector,
    params: AppSurfaceParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let window_id = select_window_for_app_state_target(action, target, ctx)?;
    let workspace = workspace_for_window(action, window_id, ctx)?;
    let workspace_action = workspace_action_for_surface(action, params)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(&workspace_action, ctx);
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": action.as_str(),
        "handled": true,
        "window_id": window_id.to_string(),
    }))
}

fn validate_app_focus_target(target: &TargetSelector) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "app.focus does not accept target selectors",
        ));
    }
    Ok(())
}

fn validate_window_create_target(
    target: &TargetSelector,
    params: &WindowCreateParams,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "window.create does not accept target selectors",
        ));
    }
    if params.profile.is_some() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "window.create does not support selecting a profile yet",
        ));
    }
    Ok(())
}

fn select_window_for_app_state_target(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    if target.tab.is_some() || target.pane.is_some() || target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept tab, pane, or session selectors",
                action.as_str()
            ),
        ));
    }
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => ctx.windows().active_window().ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires an active Warp window", action.as_str()),
            )
        }),
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { .. } | WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active and opaque window id selectors",
                action.as_str()
            ),
        )),
    }
}

fn workspace_for_window(
    action: ActionKind,
    window_id: WindowId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the target window",
                    action.as_str()
                ),
            )
        })
}

fn workspace_action_for_surface(
    action: ActionKind,
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    match action {
        ActionKind::AppSettingsOpen => settings_surface_action(params),
        ActionKind::AppCommandPaletteOpen => command_palette_surface_action(params),
        ActionKind::AppCommandSearchOpen => command_search_surface_action(params),
        ActionKind::AppWarpDriveOpen => {
            no_params_surface_action(action, params, WorkspaceAction::OpenWarpDrive)
        }
        ActionKind::AppWarpDriveToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleWarpDrive)
        }
        ActionKind::AppResourceCenterToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleResourceCenter)
        }
        ActionKind::AppAiAssistantToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleAIAssistant)
        }
        ActionKind::AppCodeReviewToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleRightPanel)
        }
        ActionKind::AppVerticalTabsToggle => {
            no_params_surface_action(action, params, WorkspaceAction::ToggleVerticalTabsPanel)
        }
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a supported app surface action", action.as_str()),
        )),
    }
}

fn settings_surface_action(params: AppSurfaceParams) -> Result<WorkspaceAction, ControlError> {
    let section = params
        .page
        .as_deref()
        .map(settings_section_from_param)
        .transpose()?;
    match (section, params.query) {
        (Some(section), Some(query)) => Ok(WorkspaceAction::ShowSettingsPageWithSearch {
            search_query: query,
            section: Some(section),
        }),
        (None, Some(query)) => Ok(WorkspaceAction::ShowSettingsPageWithSearch {
            search_query: query,
            section: None,
        }),
        (Some(section), None) => Ok(WorkspaceAction::ShowSettingsPage(section)),
        (None, None) => Ok(WorkspaceAction::ShowSettings),
    }
}

fn command_palette_surface_action(
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    if params.page.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} does not accept a page parameter",
                ActionKind::AppCommandPaletteOpen.as_str()
            ),
        ));
    }
    Ok(WorkspaceAction::OpenPalette {
        mode: PaletteMode::Command,
        source: PaletteSource::Keybinding,
        query: params.query,
    })
}

fn command_search_surface_action(
    params: AppSurfaceParams,
) -> Result<WorkspaceAction, ControlError> {
    if params.page.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} does not accept a page parameter",
                ActionKind::AppCommandSearchOpen.as_str()
            ),
        ));
    }
    let init_content = params
        .query
        .map(InitContent::Custom)
        .unwrap_or(InitContent::FromInputBuffer);
    Ok(WorkspaceAction::ShowCommandSearch(CommandSearchOptions {
        filter: None,
        init_content,
    }))
}

fn no_params_surface_action(
    action: ActionKind,
    params: AppSurfaceParams,
    workspace_action: WorkspaceAction,
) -> Result<WorkspaceAction, ControlError> {
    if params.query.is_some() || params.page.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} does not accept query or page parameters",
                action.as_str()
            ),
        ));
    }
    Ok(workspace_action)
}

fn settings_section_from_param(page: &str) -> Result<SettingsSection, ControlError> {
    let normalized = page.replace(['-', '_'], " ");
    let mut words = normalized.split_whitespace();
    let title_case = words.try_fold(String::new(), |mut output, word| {
        if !output.is_empty() {
            output.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.push_str(&chars.as_str().to_lowercase());
        }
        Some(output)
    });
    let mut candidates = vec![page.to_owned(), normalized];
    if let Some(title_case) = title_case {
        candidates.push(title_case);
    }
    candidates
        .iter()
        .find_map(|candidate| candidate.parse::<SettingsSection>().ok())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!("unknown settings page {page}"),
            )
        })
}

#[cfg(test)]
pub(crate) use tests::validate_app_focus_target_pub as validate_app_focus_target_test;
#[cfg(test)]
pub(crate) use tests::validate_window_create_target_pub as validate_window_create_target_test;

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn validate_app_focus_target_pub(
        target: &TargetSelector,
    ) -> Result<(), ControlError> {
        validate_app_focus_target(target)
    }

    pub(crate) fn validate_window_create_target_pub(
        target: &TargetSelector,
        params: &WindowCreateParams,
    ) -> Result<(), ControlError> {
        validate_window_create_target(target, params)
    }
}
