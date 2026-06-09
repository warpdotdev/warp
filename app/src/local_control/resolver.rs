//! Target and parameter validation for the first local-control action slice.
use ::local_control::protocol::{PaneTarget, TabTarget, TargetSelector, WindowTarget};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use warpui::{ModelContext, WindowId};

use crate::local_control::LocalControlBridge;
use crate::workspace::Workspace;

pub(crate) fn validate_tab_create_target(target: &TargetSelector) -> Result<(), ControlError> {
    if matches!(target.tab.as_ref(), Some(TabTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested tab id",
        ));
    }
    if !matches!(target.tab.as_ref(), None | Some(TabTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept a concrete tab selector",
        ));
    }
    if matches!(target.pane.as_ref(), Some(PaneTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "tab.create cannot resolve the requested pane id",
        ));
    }
    if !matches!(target.pane.as_ref(), None | Some(PaneTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create does not accept a concrete pane selector",
        ));
    }
    Ok(())
}

/// Validates action-specific params implemented by this branch stack layer.
///
/// This is intentionally narrow for the current implementation slice. Later
/// slices add their own params and expand this validation alongside the
/// corresponding action handlers.
pub(crate) fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    if !action.kind.is_implemented() {
        return Ok(());
    }
    if action
        .params
        .as_object()
        .is_some_and(serde_json::Map::is_empty)
    {
        return Ok(());
    }
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!(
            "{} does not accept parameters in the first implementation slice",
            action.kind.as_str()
        ),
    ))
}

pub(super) fn target_window_id_for_target(
    ctx: &mut ModelContext<LocalControlBridge>,
    target: &TargetSelector,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => active_or_single_window_id(ctx, action),
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { index }) => {
            resolve_index_from_ids(ctx.window_ids(), *index, action)
        }
        Some(WindowTarget::Title { title }) => target_window_id_by_title(ctx, title, action),
    }
}

#[cfg(test)]
pub(crate) fn require_active_window_id(
    active_window: Option<WindowId>,
) -> Result<WindowId, ControlError> {
    active_window.ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            "tab.create requires an active Warp window",
        )
    })
}

fn active_or_single_window_id(
    ctx: &mut ModelContext<LocalControlBridge>,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    if let Some(window_id) = ctx.windows().active_window() {
        return Ok(window_id);
    }
    let window_ids = ctx.window_ids().collect::<Vec<_>>();
    match window_ids.as_slice() {
        [window_id] => Ok(*window_id),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!(
                "{} requires an explicit window selector when no Warp window is active",
                action.as_str()
            ),
        )),
    }
}

fn target_window_id_by_title(
    ctx: &mut ModelContext<LocalControlBridge>,
    title: &str,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    let mut matching = Vec::new();
    for window_id in ctx.window_ids().collect::<Vec<_>>() {
        if window_title(window_id, ctx).as_deref() == Some(title) {
            matching.push(window_id);
        }
    }
    resolve_title_from_matches(&matching, action)
}

pub(crate) fn resolve_index_from_ids(
    ids: impl Iterator<Item = WindowId>,
    index: u32,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    ids.into_iter().nth(index as usize).ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!(
                "{} cannot resolve the requested window index",
                action.as_str()
            ),
        )
    })
}

pub(crate) fn resolve_title_from_matches(
    matching: &[WindowId],
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    match matching {
        [window_id] => Ok(*window_id),
        [] => Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!(
                "{} cannot resolve the requested window title",
                action.as_str()
            ),
        )),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!("{} resolved multiple windows by title", action.as_str()),
        )),
    }
}

fn window_title(window_id: WindowId, ctx: &mut ModelContext<LocalControlBridge>) -> Option<String> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .map(|workspace| {
            workspace.read(ctx, |workspace, ctx| {
                workspace
                    .active_tab_pane_group()
                    .as_ref(ctx)
                    .display_title(ctx)
            })
        })
}
