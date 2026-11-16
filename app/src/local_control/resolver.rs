//! Target resolution and parameter validation for retained local-control actions.
use ::local_control::protocol::{
    ActionNameParams, ActionParameterSpec, BindingNameParams, BooleanValueParams, ColorValueParams,
    DirectionParams, EmptyParams, FileOpenParams, KeyParams, KeyValueParams, NamespaceParams,
    PageQueryParams, PaneTarget, QueryParams, RenameParams, ResizeParams, SessionTarget,
    TabActivateParams, TabCloseParams, TabCreateParams, TabTarget, TargetSelector, TextParams,
    ThemeNameParams, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, TargetScope};
use warpui::{AppContext, ModelContext, TypedActionView, ViewHandle, WindowId};

use crate::local_control::LocalControlBridge;
use crate::local_control::handlers::metadata::action_metadata_for_name;
use crate::pane_group::{ActivationReason, PaneGroup, PaneGroupAction, PaneId};
use crate::workspace::{Workspace, WorkspaceAction};

pub(crate) fn validate_tab_create_target(target: &TargetSelector) -> Result<(), ControlError> {
    if target.tab.is_some() || target.pane.is_some() || target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.create accepts only a window selector",
        ));
    }
    Ok(())
}

pub(crate) fn validate_action_params(action: &::local_control::Action) -> Result<(), ControlError> {
    if !action.kind.is_implemented() {
        return Ok(());
    }
    match action.kind.metadata().parameter_spec {
        ActionParameterSpec::None => parse_params::<EmptyParams>(action),
        ActionParameterSpec::ActionName => {
            let params = action.params_as::<ActionNameParams>()?;
            action_metadata_for_name(&params.action).map(|_| ())
        }
        ActionParameterSpec::BindingName => parse_params::<BindingNameParams>(action),
        ActionParameterSpec::BooleanValue => parse_params::<BooleanValueParams>(action),
        ActionParameterSpec::ColorValue => parse_params::<ColorValueParams>(action),
        ActionParameterSpec::Direction => parse_params::<DirectionParams>(action),
        ActionParameterSpec::FileOpen => parse_params::<FileOpenParams>(action),
        ActionParameterSpec::Key => parse_params::<KeyParams>(action),
        ActionParameterSpec::KeyValue => parse_params::<KeyValueParams>(action),
        ActionParameterSpec::Namespace => parse_params::<NamespaceParams>(action),
        ActionParameterSpec::PageQuery => parse_params::<PageQueryParams>(action),
        ActionParameterSpec::Query => parse_params::<QueryParams>(action),
        ActionParameterSpec::Rename => parse_params::<RenameParams>(action),
        ActionParameterSpec::Resize => parse_params::<ResizeParams>(action),
        ActionParameterSpec::TabActivate => parse_params::<TabActivateParams>(action),
        ActionParameterSpec::TabClose => parse_params::<TabCloseParams>(action),
        ActionParameterSpec::TabCreate => parse_params::<TabCreateParams>(action),
        ActionParameterSpec::Text => parse_params::<TextParams>(action),
        ActionParameterSpec::ThemeName => parse_params::<ThemeNameParams>(action),
    }
}

pub(crate) fn validate_action_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    let has_target = target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some();
    let rejects_all_targets = match action.metadata().target_scope {
        TargetScope::Instance => action != ActionKind::AppFocus,
        TargetScope::Appearance
        | TargetScope::Settings
        | TargetScope::Keybinding
        | TargetScope::Action
        | TargetScope::Capability => true,
        TargetScope::Window
        | TargetScope::Tab
        | TargetScope::Pane
        | TargetScope::Session
        | TargetScope::Input
        | TargetScope::Surface
        | TargetScope::File => false,
    };
    if rejects_all_targets && has_target {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} does not accept target selectors", action.as_str()),
        ));
    }
    Ok(())
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
        Some(WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active, opaque window id, and window index selectors",
                action.as_str()
            ),
        )),
    }
}

#[cfg(test)]
pub(crate) fn require_active_window_id(
    active_window: Option<WindowId>,
) -> Result<WindowId, ControlError> {
    require_active_window_id_for_action(active_window, ActionKind::TabCreate)
}

pub(crate) fn require_active_window_id_for_action(
    active_window: Option<WindowId>,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    active_window.ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires an active Warp window", action.as_str()),
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
        [] => require_active_window_id_for_action(None, action),
        _ => Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            format!(
                "{} requires an explicit window selector when no Warp window is active",
                action.as_str()
            ),
        )),
    }
}

pub(crate) fn resolve_index_from_ids(
    ids: impl Iterator<Item = WindowId>,
    index: u32,
    action: ActionKind,
) -> Result<WindowId, ControlError> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_by_key(ToString::to_string);
    ids.get(index as usize).copied().ok_or_else(|| {
        ControlError::new(
            ErrorCode::MissingTarget,
            format!(
                "{} cannot resolve the requested window index",
                action.as_str()
            ),
        )
    })
}

#[cfg(test)]
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
fn parse_params<T: serde::de::DeserializeOwned>(
    action: &::local_control::Action,
) -> Result<(), ControlError> {
    action.params_as::<T>().map(|_| ())
}

pub(crate) fn decode_params<T: serde::de::DeserializeOwned>(
    params: &serde_json::Value,
) -> Result<T, ControlError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "failed to decode action parameters",
            err.to_string(),
        )
    })
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

pub(crate) fn workspace_for_window(
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

pub(crate) fn target_workspace(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    let window_id = target_window_id_for_target(ctx, target, action)?;
    workspace_for_window(window_id, action, ctx)
}

pub(crate) fn target_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    let workspace = target_workspace(action, target, ctx)?;
    workspace.read(ctx, |workspace, ctx| {
        let tab_index = tab_index_from_target(target, workspace, ctx)?;
        workspace
            .tab_views()
            .nth(tab_index)
            .cloned()
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested tab", action.as_str()),
                )
            })
    })
}

pub(crate) fn active_target_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    reject_target_families(
        action,
        action != ActionKind::SessionActivate && target.session.is_some(),
        "session selectors",
    )?;
    let workspace = target_workspace(action, target, ctx)?;
    if target.tab.is_some() {
        workspace.update(ctx, |workspace, ctx| {
            let tab_index = tab_index_from_target(target, workspace, ctx)?;
            workspace.handle_action(&WorkspaceAction::ActivateTab(tab_index), ctx);
            Ok::<_, ControlError>(())
        })?;
    }
    target_pane_group(action, target, ctx)
}

pub(crate) fn activate_target(
    workspace: &ViewHandle<Workspace>,
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if target.tab.is_some() {
        workspace.update(ctx, |workspace, ctx| {
            let tab_index = tab_index_from_target(target, workspace, ctx)?;
            workspace.handle_action(&WorkspaceAction::ActivateTab(tab_index), ctx);
            Ok::<_, ControlError>(())
        })?;
    }
    if target.session.is_some() {
        let pane_group = target_pane_group(action, target, ctx)?;
        let pane_id = target_session_pane_id(action, target, &pane_group, ctx)?;
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(
                &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
                ctx,
            );
        });
    } else if target.pane.is_some() {
        let pane_group = target_pane_group(action, target, ctx)?;
        focus_explicit_pane_target(action, target, &pane_group, ctx)?;
    }
    Ok(())
}

pub(crate) fn focus_explicit_pane_target(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<(), ControlError> {
    if target.pane.is_none() {
        return Ok(());
    }
    let pane_id = target_pane_id(action, target, pane_group, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(
            &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
            ctx,
        );
    });
    Ok(())
}

pub(crate) fn target_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    pane_group.read(ctx, |pane_group, ctx| match target.pane.as_ref() {
        None | Some(PaneTarget::Active) => Ok(pane_group.focused_pane_id(ctx)),
        Some(PaneTarget::Index { index }) => {
            let pane_index = usize::try_from(*index).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidSelector,
                    "pane index is out of range",
                    err.to_string(),
                )
            })?;
            pane_group.pane_id_from_index(pane_index).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    format!(
                        "{} cannot resolve the requested pane index",
                        action.as_str()
                    ),
                )
            })
        }
        Some(PaneTarget::Id { id }) => pane_group
            .visible_pane_ids()
            .into_iter()
            .find(|pane_id| pane_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested pane id", action.as_str()),
                )
            }),
    })
}

pub(crate) fn tab_index_from_target(
    target: &TargetSelector,
    workspace: &Workspace,
    ctx: &AppContext,
) -> Result<usize, ControlError> {
    match target.tab.as_ref() {
        Some(TabTarget::Index { index }) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < workspace.tab_count())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "tab selector index did not match a visible tab",
                )
            }),
        Some(TabTarget::Active) | None => Ok(workspace.active_tab_index()),
        Some(TabTarget::Id { id }) => workspace
            .tab_views()
            .enumerate()
            .find_map(|(index, pane_group)| (pane_group.id().to_string() == id.0).then_some(index))
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    "tab selector id did not match a visible tab",
                )
            }),
        Some(TabTarget::Title { title }) => {
            let matching = workspace
                .tab_views()
                .enumerate()
                .filter_map(|(index, pane_group)| {
                    (pane_group.as_ref(ctx).display_title(ctx) == *title).then_some(index)
                })
                .collect::<Vec<_>>();
            match matching.as_slice() {
                [index] => Ok(*index),
                [] => Err(ControlError::new(
                    ErrorCode::MissingTarget,
                    "tab selector title did not match a visible tab",
                )),
                _ => Err(ControlError::new(
                    ErrorCode::AmbiguousTarget,
                    "tab selector title matched multiple visible tabs",
                )),
            }
        }
    }
}

pub(crate) fn input_target_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    if target.session.is_some() {
        return target_session_pane_id(action, target, pane_group, ctx);
    }
    if target.pane.is_some() {
        return target_pane_id(action, target, pane_group, ctx);
    }
    pane_group
        .read(ctx, |pane_group, ctx| {
            pane_group.active_session_id(ctx).map(PaneId::from)
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires an active terminal session", action.as_str()),
            )
        })
}

pub(crate) fn target_session_pane_id(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: &ViewHandle<PaneGroup>,
    ctx: &AppContext,
) -> Result<PaneId, ControlError> {
    if target.session.is_none() && target.pane.is_some() {
        let pane_id = target_pane_id(action, target, pane_group, ctx)?;
        let has_terminal = pane_group.read(ctx, |pane_group, ctx| {
            pane_group
                .terminal_view_from_pane_id(pane_id, ctx)
                .is_some()
        });
        if has_terminal {
            return Ok(pane_id);
        }
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} requires a terminal session target", action.as_str()),
        ));
    }
    let session_pane_id =
        pane_group.read(ctx, |pane_group, ctx| match target.session.as_ref() {
            None | Some(SessionTarget::Active) => pane_group
                .active_session_id(ctx)
                .map(PaneId::from)
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!("{} requires an active terminal session", action.as_str()),
                    )
                }),
            Some(SessionTarget::Id { id }) => pane_group
                .visible_pane_ids()
                .into_iter()
                .find(|pane_id| pane_id.to_string() == id.0)
                .filter(|pane_id| {
                    pane_group
                        .terminal_view_from_pane_id(*pane_id, ctx)
                        .is_some()
                })
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::StaleTarget,
                        format!(
                            "{} cannot resolve the requested session id",
                            action.as_str()
                        ),
                    )
                }),
        })?;
    if target.pane.is_some() {
        let pane_id = target_pane_id(action, target, pane_group, ctx)?;
        if pane_id != session_pane_id {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                format!(
                    "{} pane and session selectors resolve different targets",
                    action.as_str()
                ),
            ));
        }
    }
    Ok(session_pane_id)
}
