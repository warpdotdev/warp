//! Layout mutation handlers for local-control actions.
use ::local_control::protocol::{
    HorizontalDirection, PaneDirection, PaneMaximizeParams, PaneMutationResult, PaneNavigateParams,
    PaneResizeParams, PaneSplitParams, PaneTarget, SessionTarget, TabActivateParams,
    TabActivationTarget, TabCloseParams, TabCloseScope, TabMoveParams, TabMutationResult,
    TabTarget, TargetSelector, WindowCloseParams, WindowCreateParams, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warpui::platform::TerminationMode;
use warpui::{ModelContext, TypedActionView, ViewHandle, WindowId};

use crate::local_control::resolver::{
    require_active_window_id_for_action, target_window_id, validate_tab_create_target,
};
use crate::local_control::LocalControlBridge;
use crate::pane_group::{Direction as PaneGroupDirection, PaneGroup, PaneGroupAction, PaneId};
use crate::root_view;
use crate::workspace::{Workspace, WorkspaceAction};

#[derive(Clone, Debug)]
pub(crate) struct TabEntry {
    pub(crate) window_id: WindowId,
    pub(crate) index: usize,
    pub(crate) workspace_active_tab_index: usize,
    pub(crate) pane_group: ViewHandle<PaneGroup>,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneEntry {
    pub(crate) tab_id: String,
    pub(crate) index: usize,
    pub(crate) pane_group: ViewHandle<PaneGroup>,
    pub(crate) pane_id: PaneId,
}

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
    if target.window.is_none() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            "window.close requires an explicit window selector; use --window active to close the active window",
        ));
    }
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

pub(crate) fn activate_tab(
    target: &TargetSelector,
    params: TabActivateParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let (window_id, tab_id) = if let Some(relative) = params.relative {
        reject_concrete_tab_selector_for_relative_activation(target)?;
        let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabActivate, ctx)?;
        let workspace = workspace_for_window(entry.window_id, ActionKind::TabActivate, ctx)?;
        workspace.update(ctx, |workspace, ctx| {
            let action = match relative {
                TabActivationTarget::Previous => WorkspaceAction::ActivatePrevTab,
                TabActivationTarget::Next => WorkspaceAction::ActivateNextTab,
                TabActivationTarget::Last => WorkspaceAction::ActivateLastTab,
            };
            workspace.handle_action(&action, ctx);
            let pane_group = workspace.active_tab_pane_group();
            (entry.window_id, pane_group.id().to_string())
        })
    } else {
        let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabActivate, ctx)?;
        let workspace = workspace_for_window(entry.window_id, ActionKind::TabActivate, ctx)?;
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::ActivateTab(entry.index), ctx);
            (entry.window_id, entry.pane_group.id().to_string())
        })
    };
    to_tab_mutation_result(tab_id, window_id)
}

pub(crate) fn move_tab(
    target: &TargetSelector,
    params: TabMoveParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabMove, ctx)?;
    let workspace = workspace_for_window(entry.window_id, ActionKind::TabMove, ctx)?;
    let tab_count = workspace.read(ctx, |workspace, _| workspace.tab_count());
    match params.direction {
        HorizontalDirection::Left if entry.index == 0 => {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "tab.move cannot move the leftmost tab further left",
            ));
        }
        HorizontalDirection::Right if entry.index + 1 >= tab_count => {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "tab.move cannot move the rightmost tab further right",
            ));
        }
        _ => {}
    }
    let tab_id = entry.pane_group.id().to_string();
    workspace.update(ctx, |workspace, ctx| {
        let action = match params.direction {
            HorizontalDirection::Left => WorkspaceAction::MoveTabLeft(entry.index),
            HorizontalDirection::Right => WorkspaceAction::MoveTabRight(entry.index),
        };
        workspace.handle_action(&action, ctx);
    });
    to_tab_mutation_result(tab_id, entry.window_id)
}

pub(crate) fn close_tab(
    target: &TargetSelector,
    params: TabCloseParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if target.tab.is_none() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            "tab.close requires an explicit tab selector; use --tab active to close the active tab",
        ));
    }
    let entry = select_single_tab_entry_for_mutation(target, ActionKind::TabClose, ctx)?;
    let workspace = workspace_for_window(entry.window_id, ActionKind::TabClose, ctx)?;
    let tab_count = workspace.read(ctx, |workspace, _| workspace.tab_count());
    match params.scope {
        TabCloseScope::Others if tab_count <= 1 => {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "tab.close others requires at least one other tab",
            ));
        }
        TabCloseScope::Right if entry.index + 1 >= tab_count => {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "tab.close right requires at least one tab to the right",
            ));
        }
        _ => {}
    }
    let tab_id = entry.pane_group.id().to_string();
    workspace.update(ctx, |workspace, ctx| {
        let action = match params.scope {
            TabCloseScope::Target => WorkspaceAction::CloseTab(entry.index),
            TabCloseScope::Others => WorkspaceAction::CloseOtherTabs(entry.index),
            TabCloseScope::Right => WorkspaceAction::CloseTabsRight(entry.index),
        };
        workspace.handle_action(&action, ctx);
    });
    to_tab_mutation_result(tab_id, entry.window_id)
}

pub(crate) fn split_pane(
    target: &TargetSelector,
    params: PaneSplitParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if params.profile.is_some() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "pane.split profile selection is not implemented by this local-control bridge",
        ));
    }
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneSplit, ctx)?;
    let direction = map_pane_direction(params.direction);
    let new_pane_id = entry.pane_group.update(ctx, |pane_group, ctx| {
        let existing_ids = pane_group.visible_pane_ids();
        pane_group.focus_pane_by_id(entry.pane_id, ctx);
        pane_group.handle_action(&PaneGroupAction::Add(direction), ctx);
        pane_group
            .visible_pane_ids()
            .into_iter()
            .find(|pane_id| !existing_ids.contains(pane_id))
    });
    let pane_id = new_pane_id.ok_or_else(|| {
        ControlError::new(
            ErrorCode::TargetStateConflict,
            "pane.split did not create a new pane in the target pane group",
        )
    })?;
    to_pane_mutation_result(pane_id.to_string(), entry.tab_id)
}

pub(crate) fn focus_pane(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneFocus, ctx)?;
    entry.pane_group.update(ctx, |pane_group, ctx| {
        pane_group.focus_pane_by_id(entry.pane_id, ctx);
    });
    to_pane_mutation_result(entry.pane_id.to_string(), entry.tab_id)
}

pub(crate) fn navigate_pane(
    target: &TargetSelector,
    params: PaneNavigateParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneNavigate, ctx)?;
    let action = match params.direction {
        PaneDirection::Left => PaneGroupAction::NavigateLeft,
        PaneDirection::Right => PaneGroupAction::NavigateRight,
        PaneDirection::Up => PaneGroupAction::NavigateUp,
        PaneDirection::Down => PaneGroupAction::NavigateDown,
    };
    let focused_pane_id = entry.pane_group.update(ctx, |pane_group, ctx| {
        pane_group.focus_pane_by_id(entry.pane_id, ctx);
        pane_group.handle_action(&action, ctx);
        pane_group.focused_pane_id(ctx)
    });
    if focused_pane_id == entry.pane_id {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "pane.navigate could not find a pane in the requested direction",
        ));
    }
    to_pane_mutation_result(focused_pane_id.to_string(), entry.tab_id)
}

pub(crate) fn close_pane(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if target.pane.is_none() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            "pane.close requires an explicit pane selector; use --pane active to close the active pane",
        ));
    }
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneClose, ctx)?;
    let pane_id = entry.pane_id.to_string();
    entry.pane_group.update(ctx, |pane_group, ctx| {
        pane_group.close_pane(entry.pane_id, ctx);
    });
    to_pane_mutation_result(pane_id, entry.tab_id)
}

pub(crate) fn maximize_pane(
    target: &TargetSelector,
    params: PaneMaximizeParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneMaximize, ctx)?;
    let pane_id = entry.pane_id.to_string();
    entry.pane_group.update(ctx, |pane_group, ctx| {
        if pane_group.pane_count() <= 1 {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "pane.maximize requires a split pane group",
            ));
        }
        pane_group.focus_pane_by_id(entry.pane_id, ctx);
        let currently_enabled = pane_group.is_focused_pane_maximized(ctx);
        if params
            .enabled
            .is_none_or(|enabled| enabled != currently_enabled)
        {
            pane_group.handle_action(&PaneGroupAction::ToggleMaximizePane, ctx);
        }
        Ok(())
    })?;
    to_pane_mutation_result(pane_id, entry.tab_id)
}

pub(crate) fn resize_pane(
    target: &TargetSelector,
    params: PaneResizeParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let entry = select_single_pane_entry_for_mutation(target, ActionKind::PaneResize, ctx)?;
    if params.amount == Some(0) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "pane.resize amount must be greater than zero",
        ));
    }
    let action = match params.direction {
        PaneDirection::Left => PaneGroupAction::ResizeLeft,
        PaneDirection::Right => PaneGroupAction::ResizeRight,
        PaneDirection::Up => PaneGroupAction::ResizeUp,
        PaneDirection::Down => PaneGroupAction::ResizeDown,
    };
    entry.pane_group.update(ctx, |pane_group, ctx| {
        if pane_group.pane_count() <= 1 {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "pane.resize requires a split pane group",
            ));
        }
        pane_group.focus_pane_by_id(entry.pane_id, ctx);
        let repeat_count = params.amount.unwrap_or(1);
        for _ in 0..repeat_count {
            pane_group.handle_action(&action, ctx);
        }
        Ok(())
    })?;
    to_pane_mutation_result(entry.pane_id.to_string(), entry.tab_id)
}

pub(crate) fn reject_target_families(
    action: ActionKind,
    rejected: bool,
    families: &str,
) -> Result<(), ControlError> {
    if rejected {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept {families}", action.as_str()),
        ));
    }
    Ok(())
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

fn to_tab_mutation_result(
    tab_id: String,
    window_id: WindowId,
) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(TabMutationResult {
        tab_id,
        window_id: window_id.to_string(),
    })
    .map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode response",
            err.to_string(),
        )
    })
}

fn to_pane_mutation_result(
    pane_id: String,
    tab_id: String,
) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(PaneMutationResult { pane_id, tab_id }).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode response",
            err.to_string(),
        )
    })
}

fn select_window_ids(
    target: &TargetSelector,
    force_active_default: bool,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<WindowId>, ControlError> {
    match target.window.as_ref() {
        None if force_active_default => {
            let window_id =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            Ok(vec![window_id])
        }
        None => Ok(ctx.window_ids().collect()),
        Some(WindowTarget::Active) => {
            let window_id =
                require_active_window_id_for_action(ctx.windows().active_window(), action)?;
            Ok(vec![window_id])
        }
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .map(|window_id| vec![window_id])
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

fn tab_entries_for_windows(
    window_ids: Vec<WindowId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<TabEntry> {
    window_ids
        .into_iter()
        .filter_map(|window_id| {
            let workspace = ctx
                .views_of_type::<Workspace>(window_id)
                .and_then(|workspaces| workspaces.into_iter().next())?;
            Some(workspace.read(ctx, |workspace, _| {
                workspace
                    .tab_views()
                    .enumerate()
                    .map(|(index, pane_group)| TabEntry {
                        window_id,
                        index,
                        workspace_active_tab_index: workspace.active_tab_index(),
                        pane_group: pane_group.clone(),
                    })
                    .collect::<Vec<_>>()
            }))
        })
        .flatten()
        .collect()
}

fn select_tab_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<TabEntry>, ControlError> {
    let force_active_window = matches!(
        target.tab,
        Some(TabTarget::Active | TabTarget::Index { .. })
    ) || matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    ) || matches!(target.session, Some(SessionTarget::Active));
    let window_ids = select_window_ids(target, force_active_window, action, ctx)?;
    let all_entries = tab_entries_for_windows(window_ids, ctx);
    let requires_active_tab_default = matches!(
        target.pane,
        Some(PaneTarget::Active | PaneTarget::Index { .. })
    ) || matches!(target.session, Some(SessionTarget::Active));
    match target.tab.as_ref() {
        None if requires_active_tab_default => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active tab", action.as_str()),
                ));
            }
            Ok(entries)
        }
        None => Ok(all_entries),
        Some(TabTarget::Active) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index == entry.workspace_active_tab_index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active tab", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Id { id }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.pane_group.id().to_string() == id.0)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab id", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Index { index }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab index", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(TabTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque tab id, and tab index selectors",
                action.as_str()
            ),
        )),
    }
}

fn pane_entries_for_tabs(
    tab_entries: Vec<TabEntry>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Vec<PaneEntry> {
    tab_entries
        .into_iter()
        .flat_map(|entry| {
            let tab_id = entry.pane_group.id().to_string();
            let pane_group = entry.pane_group.clone();
            entry
                .pane_group
                .read(ctx, |pane_group, _| pane_group.visible_pane_ids())
                .into_iter()
                .enumerate()
                .map(move |(index, pane_id)| PaneEntry {
                    tab_id: tab_id.clone(),
                    index,
                    pane_group: pane_group.clone(),
                    pane_id,
                })
        })
        .collect()
}

fn select_pane_entries(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<PaneEntry>, ControlError> {
    let tab_entries = select_tab_entries(target, action, ctx)?;
    let all_entries = pane_entries_for_tabs(tab_entries, ctx);
    match target.pane.as_ref() {
        None if matches!(target.session, Some(SessionTarget::Active)) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| {
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        pane_group.active_session_id(ctx).map(PaneId::from) == Some(entry.pane_id)
                    })
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active terminal session", action.as_str()),
                ));
            }
            Ok(entries)
        }
        None => Ok(all_entries),
        Some(PaneTarget::Active) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| {
                    entry.pane_group.read(ctx, |pane_group, ctx| {
                        pane_group.focused_pane_id(ctx) == entry.pane_id
                    })
                })
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    format!("{} requires an active pane", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(PaneTarget::Id { id }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.pane_id.to_string() == id.0)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested pane id", action.as_str()),
                ));
            }
            Ok(entries)
        }
        Some(PaneTarget::Index { index }) => {
            let entries = all_entries
                .into_iter()
                .filter(|entry| entry.index as u32 == *index)
                .collect::<Vec<_>>();
            if entries.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::StaleTarget,
                    format!(
                        "{} cannot resolve the requested pane index",
                        action.as_str()
                    ),
                ));
            }
            Ok(entries)
        }
    }
}

pub(crate) fn select_single_tab_entry_for_mutation(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<TabEntry, ControlError> {
    reject_target_families(
        action,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let mut target = target.clone();
    if target.tab.is_none() {
        target.tab = Some(TabTarget::Active);
    }
    let entries = select_tab_entries(&target, action, ctx)?;
    single_entry(entries, action, "tab")
}

fn select_single_pane_entry_for_mutation(
    target: &TargetSelector,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<PaneEntry, ControlError> {
    reject_target_families(action, target.session.is_some(), "session selectors")?;
    let mut target = target.clone();
    if target.tab.is_none() && target.pane.is_none() {
        target.tab = Some(TabTarget::Active);
    }
    if target.pane.is_none() {
        target.pane = Some(PaneTarget::Active);
    }
    let entries = select_pane_entries(&target, action, ctx)?;
    single_entry(entries, action, "pane")
}

fn single_entry<T>(
    mut entries: Vec<T>,
    action: ActionKind,
    target_name: &str,
) -> Result<T, ControlError> {
    if entries.len() == 1 {
        return Ok(entries.remove(0));
    }
    if entries.is_empty() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a target {target_name}", action.as_str()),
        ));
    }
    Err(ControlError::new(
        ErrorCode::TargetStateConflict,
        format!(
            "{} resolved more than one {target_name}; provide a more specific selector",
            action.as_str()
        ),
    ))
}

fn workspace_for_window(
    window_id: WindowId,
    action: ActionKind,
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

fn map_pane_direction(direction: PaneDirection) -> PaneGroupDirection {
    match direction {
        PaneDirection::Left => PaneGroupDirection::Left,
        PaneDirection::Right => PaneGroupDirection::Right,
        PaneDirection::Up => PaneGroupDirection::Up,
        PaneDirection::Down => PaneGroupDirection::Down,
    }
}

fn reject_concrete_tab_selector_for_relative_activation(
    target: &TargetSelector,
) -> Result<(), ControlError> {
    if matches!(
        target.tab,
        Some(TabTarget::Id { .. } | TabTarget::Index { .. } | TabTarget::Title { .. })
    ) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.activate relative navigation only accepts the default or active tab selector",
        ));
    }
    Ok(())
}
