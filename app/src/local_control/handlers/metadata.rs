//! Metadata response builders for local-control introspection actions.
use ::local_control::protocol::{
    ActionNameParams, ActiveTargetChain, PaneTarget, SessionTarget, TabTarget, TargetSelector,
    WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId, PROTOCOL_VERSION};
use serde_json::{json, Value};
use warp_core::channel::ChannelState;
use warpui::{ModelContext, ViewHandle, WindowId};

use crate::local_control::LocalControlBridge;
use crate::pane_group::{PaneGroup, PaneId};
use crate::workspace::Workspace;

#[derive(Clone)]
struct TabEntry {
    window_id: WindowId,
    index: usize,
    workspace_active_tab_index: usize,
    pane_group: ViewHandle<PaneGroup>,
}

#[derive(Clone)]
struct PaneEntry {
    tab_id: String,
    index: usize,
    pane_group: ViewHandle<PaneGroup>,
    pane_id: PaneId,
}

pub(crate) fn instance(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::InstanceList.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "pid": std::process::id(),
        "channel": ChannelState::channel().to_string(),
        "app_id": ChannelState::app_id().to_string(),
        "app_version": ChannelState::app_version(),
        "protocol_version": PROTOCOL_VERSION,
        "actions": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn ping(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::AppPing.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "protocol_version": PROTOCOL_VERSION,
    })
}

pub(crate) fn version(instance_id: &Option<InstanceId>) -> serde_json::Value {
    json!({
        "action": ActionKind::AppVersion.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "protocol_version": PROTOCOL_VERSION,
        "channel": ChannelState::channel().to_string(),
        "app_id": ChannelState::app_id().to_string(),
        "app_version": ChannelState::app_version(),
    })
}

pub(crate) fn active(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> serde_json::Value {
    json!({
        "action": ActionKind::AppActive.as_str(),
        "active": active_chain(instance_id, ctx),
    })
}

pub(crate) fn inspect(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> serde_json::Value {
    json!({
        "action": ActionKind::InstanceInspect.as_str(),
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
        "version": {
            "protocol_version": PROTOCOL_VERSION,
            "channel": ChannelState::channel().to_string(),
            "app_id": ChannelState::app_id().to_string(),
            "app_version": ChannelState::app_version(),
        },
        "active": active_chain(instance_id, ctx),
        "actions": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn action_list() -> serde_json::Value {
    json!({
        "action": ActionKind::ActionList.as_str(),
        "actions": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn action_inspect(
    action: &::local_control::Action,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ActionNameParams>()?;
    let metadata = action_metadata_for_name(&params.action)?;
    Ok(json!({
        "action": ActionKind::ActionInspect.as_str(),
        "requested_action": params.action,
        "metadata": metadata,
    }))
}

pub(crate) fn window_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let window_ids = select_window_ids(target, false, ActionKind::WindowList, ctx)?;
    let active_window = ctx.windows().active_window();
    let windows = window_ids
        .into_iter()
        .map(|window_id| {
            let title = ctx
                .views_of_type::<Workspace>(window_id)
                .and_then(|workspaces| workspaces.into_iter().next())
                .map(|workspace| {
                    workspace.read(ctx, |workspace, ctx| {
                        workspace
                            .active_tab_pane_group()
                            .as_ref(ctx)
                            .display_title(ctx)
                    })
                });
            json!({
                "window_id": window_id.to_string(),
                "is_active": Some(window_id) == active_window,
                "title": title,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "action": ActionKind::WindowList.as_str(),
        "windows": windows,
    }))
}

pub(crate) fn tab_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabList,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let tabs = select_tab_entries(target, ActionKind::TabList, ctx)?
        .into_iter()
        .map(|entry| {
            let title = entry
                .pane_group
                .read(ctx, |pane_group, ctx| pane_group.display_title(ctx));
            json!({
                "tab_id": entry.pane_group.id().to_string(),
                "window_id": entry.window_id.to_string(),
                "index": entry.index as u32,
                "is_active": entry.index == entry.workspace_active_tab_index,
                "title": title,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "action": ActionKind::TabList.as_str(),
        "tabs": tabs,
    }))
}

pub(crate) fn pane_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::PaneList,
        target.session.is_some(),
        "session selectors",
    )?;
    let panes = select_pane_entries(target, ActionKind::PaneList, ctx)?
        .into_iter()
        .map(|entry| {
            let (is_active, has_terminal_session) =
                entry.pane_group.read(ctx, |pane_group, ctx| {
                    (
                        pane_group.focused_pane_id(ctx) == entry.pane_id,
                        pane_group
                            .terminal_view_from_pane_id(entry.pane_id, ctx)
                            .is_some(),
                    )
                });
            json!({
                "pane_id": entry.pane_id.to_string(),
                "tab_id": entry.tab_id,
                "index": entry.index as u32,
                "is_active": is_active,
                "has_terminal_session": has_terminal_session,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "action": ActionKind::PaneList.as_str(),
        "panes": panes,
    }))
}

pub(crate) fn session_list(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let session_target = target.session.as_ref();
    let session_id_filter = matches!(session_target, Some(SessionTarget::Id { .. }));
    let sessions = select_pane_entries(target, ActionKind::SessionList, ctx)?
        .into_iter()
        .filter_map(|entry| {
            let (is_active, has_terminal_session) =
                entry.pane_group.read(ctx, |pane_group, ctx| {
                    (
                        pane_group.active_session_id(ctx).map(PaneId::from) == Some(entry.pane_id),
                        pane_group
                            .terminal_view_from_pane_id(entry.pane_id, ctx)
                            .is_some(),
                    )
                });
            if !has_terminal_session {
                return None;
            }
            let session_id = entry.pane_id.to_string();
            let matches_session = match session_target {
                None => true,
                Some(SessionTarget::Active) => is_active,
                Some(SessionTarget::Id { id }) => id.0 == session_id,
            };
            matches_session.then(|| {
                json!({
                    "session_id": session_id,
                    "pane_id": entry.pane_id.to_string(),
                    "is_active": is_active,
                })
            })
        })
        .collect::<Vec<_>>();
    if session_id_filter && sessions.is_empty() {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "session.list cannot resolve the requested session id",
        ));
    }
    Ok(json!({
        "action": ActionKind::SessionList.as_str(),
        "sessions": sessions,
    }))
}


pub(crate) fn capability_list() -> serde_json::Value {
    json!({
        "action": ActionKind::CapabilityList.as_str(),
        "capabilities": ActionKind::implemented_metadata(),
    })
}

pub(crate) fn capability_inspect(
    action: &::local_control::Action,
) -> Result<serde_json::Value, ControlError> {
    let params = action.params_as::<ActionNameParams>()?;
    let metadata = action_metadata_for_name(&params.action)?;
    Ok(json!({
        "action": ActionKind::CapabilityInspect.as_str(),
        "requested_action": params.action,
        "capability": metadata,
    }))
}

pub(crate) fn window_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = TargetSelector {
        window: target.window.clone().or(Some(WindowTarget::Active)),
        tab: None,
        pane: None,
        session: None,
    };
    let data = window_list(&target, ctx)?;
    let window = single_entry(data.get("windows"), ActionKind::WindowInspect)?;
    Ok(json!({
        "action": ActionKind::WindowInspect.as_str(),
        "window": window,
    }))
}

pub(crate) fn tab_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone().or(Some(TabTarget::Active)),
        pane: None,
        session: None,
    };
    let data = tab_list(&target, ctx)?;
    let tab = single_entry(data.get("tabs"), ActionKind::TabInspect)?;
    Ok(json!({
        "action": ActionKind::TabInspect.as_str(),
        "tab": tab,
    }))
}

pub(crate) fn pane_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone(),
        pane: target.pane.clone().or(Some(PaneTarget::Active)),
        session: None,
    };
    let data = pane_list(&target, ctx)?;
    let pane = single_entry(data.get("panes"), ActionKind::PaneInspect)?;
    Ok(json!({
        "action": ActionKind::PaneInspect.as_str(),
        "pane": pane,
    }))
}

pub(crate) fn session_inspect(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let target = TargetSelector {
        window: target.window.clone(),
        tab: target.tab.clone(),
        pane: target.pane.clone(),
        session: target.session.clone().or(Some(SessionTarget::Active)),
    };
    let data = session_list(&target, ctx)?;
    let session = single_entry(data.get("sessions"), ActionKind::SessionInspect)?;
    Ok(json!({
        "action": ActionKind::SessionInspect.as_str(),
        "session": session,
    }))
}

fn single_entry(value: Option<&Value>, action: ActionKind) -> Result<Value, ControlError> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Err(ControlError::new(
            ErrorCode::Internal,
            format!("{} handler returned malformed metadata", action.as_str()),
        ));
    };
    if items.len() == 1 {
        return Ok(items[0].clone());
    }
    if items.is_empty() {
        return Err(ControlError::new(
            ErrorCode::MissingTarget,
            format!("{} could not resolve a target", action.as_str()),
        ));
    }
    Err(ControlError::new(
        ErrorCode::AmbiguousTarget,
        format!("{} resolved multiple targets", action.as_str()),
    ))
}

pub(crate) fn action_metadata_for_name(
    action_name: &str,
) -> Result<::local_control::ActionMetadata, ControlError> {
    ActionKind::ALL
        .iter()
        .copied()
        .find(|kind| kind.as_str() == action_name)
        .map(ActionKind::metadata)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::NotAllowlisted,
                format!("{action_name} is not an allowlisted local-control action"),
            )
        })
}

fn active_chain(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> ActiveTargetChain {
    let instance_id = instance_id.as_ref().map(|id| id.0.clone());
    let active_window = ctx.windows().active_window();
    let Some(window_id) = active_window else {
        return ActiveTargetChain {
            instance_id,
            window_id: None,
            tab_id: None,
            pane_id: None,
            session_id: None,
        };
    };
    let window_id_string = window_id.to_string();
    let workspace = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next());
    let Some(workspace) = workspace else {
        return ActiveTargetChain {
            instance_id,
            window_id: Some(window_id_string),
            tab_id: None,
            pane_id: None,
            session_id: None,
        };
    };
    let (tab_id, pane_id, session_id) = workspace.read(ctx, |workspace, ctx| {
        let pane_group = workspace.active_tab_pane_group();
        let pane_group_ref = pane_group.as_ref(ctx);
        let pane_id = pane_group_ref.focused_pane_id(ctx);
        let session_id = pane_group_ref
            .active_session_id(ctx)
            .map(|session_id| PaneId::from(session_id).to_string());
        (
            Some(pane_group.id().to_string()),
            Some(pane_id.to_string()),
            session_id,
        )
    });
    ActiveTargetChain {
        instance_id,
        window_id: Some(window_id_string),
        tab_id,
        pane_id,
        session_id,
    }
}

fn reject_target_families(
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

fn select_window_ids(
    target: &TargetSelector,
    force_active_default: bool,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Vec<WindowId>, ControlError> {
    if action == ActionKind::WindowList {
        reject_target_families(
            action,
            target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
            "tab, pane, or session selectors",
        )?;
    }
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
