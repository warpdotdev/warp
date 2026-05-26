//! App-state mutation and visible UI intent handlers for local-control actions.
use std::path::PathBuf;

use ::local_control::protocol::{
    ActionParams, Direction as ControlDirection, DriveObjectId, FileOpenParams, InputMode,
    PaneTarget, TabActivationMode, TabCloseMode, TabTarget, TargetSelector,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warp_util::path::LineAndColumnArg;
use warpui::{ModelContext, SingletonEntity, TypedActionView, ViewHandle};

use crate::cloud_object::model::persistence::CloudModel;
use crate::drive::items::WarpDriveItemId;
use crate::local_control::resolver::target_window_id_for_target;
use crate::local_control::LocalControlBridge;
use crate::palette::PaletteMode;
use crate::pane_group::{Direction, PaneGroup, PaneGroupAction};
use crate::server::ids::{ServerId, SyncId};
use crate::server::telemetry::{PaletteSource, SharingDialogSource};
use crate::tab::SelectedTabColor;
use crate::terminal::view::TerminalAction;
use crate::workspace::{CommandSearchOptions, InitContent, Workspace, WorkspaceAction};

pub(crate) fn handle(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    match action {
        ActionKind::AppFocus | ActionKind::WindowFocus => {
            focus_window(instance_id, action, target, ctx)
        }
        ActionKind::WindowCreate => {
            workspace_action(instance_id, action, WorkspaceAction::AddWindow, ctx)
        }
        ActionKind::WindowClose => {
            workspace_action(instance_id, action, WorkspaceAction::CloseWindow, ctx)
        }
        ActionKind::TabActivate => tab_activate(instance_id, params, target, ctx),
        ActionKind::TabMove => tab_move(instance_id, params, ctx),
        ActionKind::TabClose => tab_close(instance_id, params, ctx),
        ActionKind::TabRename => tab_rename(instance_id, params, ctx),
        ActionKind::TabResetName => active_tab_index_action(instance_id, action, ctx, |index| {
            WorkspaceAction::ResetTabName(index)
        }),
        ActionKind::TabColorClear => workspace_action(
            instance_id,
            action,
            WorkspaceAction::SetActiveTabColor(SelectedTabColor::Cleared),
            ctx,
        ),
        ActionKind::PaneSplit => pane_direction_action(instance_id, action, params, ctx),
        ActionKind::PaneFocus => pane_focus(instance_id, action, target, ctx),
        ActionKind::PaneNavigate => pane_direction_action(instance_id, action, params, ctx),
        ActionKind::PaneResize => pane_resize(instance_id, params, ctx),
        ActionKind::PaneMaximize => pane_maximize(instance_id, true, ctx),
        ActionKind::PaneUnmaximize => pane_maximize(instance_id, false, ctx),
        ActionKind::PaneClose => {
            pane_group_action(instance_id, action, PaneGroupAction::RemoveActive, ctx)
        }
        ActionKind::PaneRename => pane_rename(instance_id, params, ctx),
        ActionKind::PaneResetName => pane_reset_name(instance_id, ctx),
        ActionKind::SessionActivate => pane_focus(instance_id, action, target, ctx),
        ActionKind::SessionPrevious => {
            workspace_action(instance_id, action, WorkspaceAction::CyclePrevSession, ctx)
        }
        ActionKind::SessionNext => {
            workspace_action(instance_id, action, WorkspaceAction::CycleNextSession, ctx)
        }
        ActionKind::SessionReopenClosed => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ReopenClosedSession,
            ctx,
        ),
        ActionKind::InputInsert => input_text(instance_id, action, params, false, ctx),
        ActionKind::InputReplace => input_text(instance_id, action, params, true, ctx),
        ActionKind::InputClear => workspace_action(
            instance_id,
            action,
            WorkspaceAction::InsertInInput {
                content: String::new(),
                replace_buffer: true,
                ensure_agent_mode: false,
            },
            ctx,
        ),
        ActionKind::InputModeSet => input_mode_set(instance_id, params, ctx),
        ActionKind::SurfaceSettingsOpen => surface_settings_open(instance_id, params, ctx),
        ActionKind::SurfaceCommandPaletteOpen => {
            surface_palette_open(instance_id, action, PaletteMode::Command, params, ctx)
        }
        ActionKind::SurfaceCommandSearchOpen => {
            surface_command_search_open(instance_id, params, ctx)
        }
        ActionKind::SurfaceWarpDriveOpen => {
            workspace_action(instance_id, action, WorkspaceAction::OpenWarpDrive, ctx)
        }
        ActionKind::SurfaceWarpDriveToggle => {
            workspace_action(instance_id, action, WorkspaceAction::ToggleWarpDrive, ctx)
        }
        ActionKind::SurfaceResourceCenterToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleResourceCenter,
            ctx,
        ),
        ActionKind::SurfaceAiAssistantToggle => {
            workspace_action(instance_id, action, WorkspaceAction::ToggleAIAssistant, ctx)
        }
        ActionKind::SurfaceCodeReviewToggle | ActionKind::SurfaceRightPanelToggle => {
            workspace_action(instance_id, action, WorkspaceAction::ToggleRightPanel, ctx)
        }
        ActionKind::SurfaceLeftPanelToggle => {
            workspace_action(instance_id, action, WorkspaceAction::ToggleLeftPanel, ctx)
        }
        ActionKind::SurfaceVerticalTabsToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleVerticalTabsPanel,
            ctx,
        ),
        ActionKind::FileOpen => file_open(instance_id, params, ctx),
        ActionKind::DriveOpen => drive_open(instance_id, params, ctx),
        ActionKind::DriveNotebookOpen => drive_notebook_open(instance_id, params, ctx),
        ActionKind::DriveEnvVarCollectionOpen => {
            drive_env_var_collection_open(instance_id, params, ctx)
        }
        ActionKind::DriveObjectShareOpen => drive_object_share_open(instance_id, params, ctx),
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not an app-state handler action", action.as_str()),
        )),
    }
}

fn focus_window(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let window_id = target_window_id_for_target(ctx, target, action)?;
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(ack(instance_id, action))
}

fn workspace_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    action: WorkspaceAction,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let workspace = active_workspace(action_kind, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, action_kind))
}

fn active_tab_index_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
    action: impl FnOnce(usize) -> WorkspaceAction,
) -> Result<serde_json::Value, ControlError> {
    let workspace = active_workspace(action_kind, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let action = action(workspace.active_tab_index());
        workspace.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, action_kind))
}

fn tab_activate(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let mode = match action_params(params)? {
        ActionParams::TabActivate { mode } => mode,
        ActionParams::None => TabActivationMode::Target,
        _ => return invalid_params(ActionKind::TabActivate),
    };
    let workspace = target_workspace(ActionKind::TabActivate, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let action = match mode {
            TabActivationMode::Target => {
                WorkspaceAction::ActivateTab(tab_index_from_target(target, workspace)?)
            }
            TabActivationMode::Previous => WorkspaceAction::ActivatePrevTab,
            TabActivationMode::Next => WorkspaceAction::ActivateNextTab,
            TabActivationMode::Last => WorkspaceAction::ActivateLastTab,
        };
        workspace.handle_action(&action, ctx);
        Ok::<_, ControlError>(())
    })?;
    Ok(ack(instance_id, ActionKind::TabActivate))
}

fn tab_move(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let direction = direction_param(ActionKind::TabMove, params)?;
    let action = match direction {
        ControlDirection::Left | ControlDirection::Previous => WorkspaceAction::MoveActiveTabLeft,
        ControlDirection::Right | ControlDirection::Next => WorkspaceAction::MoveActiveTabRight,
        ControlDirection::Up | ControlDirection::Down => {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "tab.move only accepts left, right, previous, or next",
            ));
        }
    };
    workspace_action(instance_id, ActionKind::TabMove, action, ctx)
}

fn tab_close(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let mode = match action_params(params)? {
        ActionParams::TabClose { mode } => mode,
        ActionParams::None => TabCloseMode::Active,
        _ => return invalid_params(ActionKind::TabClose),
    };
    let action = match mode {
        TabCloseMode::Target | TabCloseMode::Active => WorkspaceAction::CloseActiveTab,
        TabCloseMode::Others => WorkspaceAction::CloseNonActiveTabs,
        TabCloseMode::RightOf => WorkspaceAction::CloseTabsRightActiveTab,
    };
    workspace_action(instance_id, ActionKind::TabClose, action, ctx)
}

fn tab_rename(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let title = rename_param(ActionKind::TabRename, params)?;
    workspace_action(
        instance_id,
        ActionKind::TabRename,
        WorkspaceAction::SetActiveTabName(title),
        ctx,
    )
}

fn pane_direction_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let direction = direction_param(action_kind, params)?;
    let action = match action_kind {
        ActionKind::PaneSplit => PaneGroupAction::Add(pane_direction(direction)?),
        ActionKind::PaneNavigate => match direction {
            ControlDirection::Left => PaneGroupAction::NavigateLeft,
            ControlDirection::Right => PaneGroupAction::NavigateRight,
            ControlDirection::Up => PaneGroupAction::NavigateUp,
            ControlDirection::Down => PaneGroupAction::NavigateDown,
            ControlDirection::Previous => PaneGroupAction::NavigatePrev,
            ControlDirection::Next => PaneGroupAction::NavigateNext,
        },
        _ => return invalid_params(action_kind),
    };
    pane_group_action(instance_id, action_kind, action, ctx)
}

fn pane_focus(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let pane_target = target.pane.as_ref().ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidSelector,
            "pane focus requires a pane target",
        )
    })?;
    let pane_group = target_pane_group(action_kind, target, ctx)?;
    let action = pane_group.read(ctx, |pane_group, _| match pane_target {
        PaneTarget::Active => Ok(None),
        PaneTarget::Index { index } => {
            let pane_index = usize::try_from(*index).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidSelector,
                    "pane index is out of range",
                    err.to_string(),
                )
            })?;
            let pane_id = pane_group.pane_id_from_index(pane_index).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::MissingTarget,
                    "pane index did not match a visible pane",
                )
            })?;
            Ok(Some(PaneGroupAction::Activate(
                pane_id,
                crate::pane_group::ActivationReason::Click,
            )))
        }
        PaneTarget::Id { id } => pane_group
            .visible_pane_ids()
            .into_iter()
            .find(|pane_id| pane_id.to_string() == id.0)
            .map(|pane_id| {
                PaneGroupAction::Activate(pane_id, crate::pane_group::ActivationReason::Click)
            })
            .map(Some)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!(
                        "{} cannot resolve the requested pane id",
                        action_kind.as_str()
                    ),
                )
            }),
    })?;
    if let Some(action) = action {
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(&action, ctx);
        });
    }
    Ok(ack(instance_id, action_kind))
}

fn pane_resize(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let direction = match action_params(params)? {
        ActionParams::Resize { direction, .. } => direction,
        _ => return invalid_params(ActionKind::PaneResize),
    };
    let action = match direction {
        ControlDirection::Left => PaneGroupAction::ResizeLeft,
        ControlDirection::Right => PaneGroupAction::ResizeRight,
        ControlDirection::Up => PaneGroupAction::ResizeUp,
        ControlDirection::Down => PaneGroupAction::ResizeDown,
        ControlDirection::Previous | ControlDirection::Next => {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "pane.resize only accepts left, right, up, or down",
            ));
        }
    };
    pane_group_action(instance_id, ActionKind::PaneResize, action, ctx)
}

fn pane_maximize(
    instance_id: &Option<InstanceId>,
    should_maximize: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action_kind = if should_maximize {
        ActionKind::PaneMaximize
    } else {
        ActionKind::PaneUnmaximize
    };
    let pane_group = active_pane_group(action_kind, ctx)?;
    let is_maximized = pane_group.read(ctx, |pane_group, ctx| {
        pane_group.is_focused_pane_maximized(ctx)
    });
    if is_maximized != should_maximize {
        pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(&PaneGroupAction::ToggleMaximizePane, ctx);
        });
    }
    Ok(ack(instance_id, action_kind))
}

fn pane_rename(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let title = rename_param(ActionKind::PaneRename, params)?;
    let pane_group = active_pane_group(ActionKind::PaneRename, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        let pane_id = pane_group.focused_pane_id(ctx);
        if let Some(pane) = pane_group.pane_by_id(pane_id) {
            let configuration = pane.pane_configuration();
            configuration.update(ctx, |configuration, ctx| {
                configuration.set_custom_vertical_tabs_title(title, ctx);
            });
        }
    });
    Ok(ack(instance_id, ActionKind::PaneRename))
}

fn pane_reset_name(
    instance_id: &Option<InstanceId>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let pane_group = active_pane_group(ActionKind::PaneResetName, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        let pane_id = pane_group.focused_pane_id(ctx);
        if let Some(pane) = pane_group.pane_by_id(pane_id) {
            let configuration = pane.pane_configuration();
            configuration.update(ctx, |configuration, ctx| {
                configuration.clear_custom_vertical_tabs_title(ctx);
            });
        }
    });
    Ok(ack(instance_id, ActionKind::PaneResetName))
}

fn pane_group_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    action: PaneGroupAction,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let pane_group = active_pane_group(action_kind, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, action_kind))
}

fn input_text(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    replace_buffer: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let text = text_param(action_kind, params)?;
    workspace_action(
        instance_id,
        action_kind,
        WorkspaceAction::InsertInInput {
            content: text,
            replace_buffer,
            ensure_agent_mode: false,
        },
        ctx,
    )
}

fn input_mode_set(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let mode = match action_params(params)? {
        ActionParams::InputMode { mode } => mode,
        _ => return invalid_params(ActionKind::InputModeSet),
    };
    let action = match mode {
        InputMode::Terminal => TerminalAction::SetInputModeTerminal,
        InputMode::Agent => TerminalAction::SetInputModeAgent,
    };
    let pane_group = active_pane_group(ActionKind::InputModeSet, ctx)?;
    let terminal_view = pane_group
        .as_ref(ctx)
        .active_session_view(ctx)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "input.mode.set requires an active terminal input",
            )
        })?;
    terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, ActionKind::InputModeSet))
}

fn surface_settings_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action = match action_params(params)? {
        ActionParams::PageQuery { query, .. } => match query {
            Some(search_query) => WorkspaceAction::ShowSettingsPageWithSearch {
                search_query,
                section: None,
            },
            None => WorkspaceAction::ShowSettings,
        },
        ActionParams::None => WorkspaceAction::ShowSettings,
        _ => return invalid_params(ActionKind::SurfaceSettingsOpen),
    };
    workspace_action(instance_id, ActionKind::SurfaceSettingsOpen, action, ctx)
}

fn surface_palette_open(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    mode: PaletteMode,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = match action_params(params)? {
        ActionParams::Query { query } => query,
        ActionParams::None => None,
        _ => return invalid_params(action_kind),
    };
    workspace_action(
        instance_id,
        action_kind,
        WorkspaceAction::OpenPalette {
            mode,
            source: PaletteSource::Keybinding,
            query,
        },
        ctx,
    )
}

fn surface_command_search_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = match action_params(params)? {
        ActionParams::Query { query } => query,
        ActionParams::None => None,
        _ => return invalid_params(ActionKind::SurfaceCommandSearchOpen),
    };
    let init_content = query
        .map(InitContent::Custom)
        .unwrap_or(InitContent::FromInputBuffer);
    workspace_action(
        instance_id,
        ActionKind::SurfaceCommandSearchOpen,
        WorkspaceAction::ShowCommandSearch(CommandSearchOptions {
            filter: None,
            init_content,
        }),
        ctx,
    )
}

fn file_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = match action_params(params)? {
        ActionParams::FileOpen(params) => params,
        _ => return invalid_params(ActionKind::FileOpen),
    };
    let line_and_column = line_and_column(&params)?;
    workspace_action(
        instance_id,
        ActionKind::FileOpen,
        WorkspaceAction::OpenFileInNewTab {
            full_path: PathBuf::from(params.path),
            line_and_column,
        },
        ctx,
    )
}

fn drive_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_id_param(ActionKind::DriveOpen, params)?;
    let object_id = cloud_object_type_and_id(&id, ctx)?;
    workspace_action(
        instance_id,
        ActionKind::DriveOpen,
        WorkspaceAction::ViewObjectInWarpDrive(WarpDriveItemId::Object(object_id)),
        ctx,
    )
}

fn drive_notebook_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_id_param(ActionKind::DriveNotebookOpen, params)?;
    workspace_action(
        instance_id,
        ActionKind::DriveNotebookOpen,
        WorkspaceAction::OpenNotebook {
            id: sync_id_from_drive_id(&id)?,
        },
        ctx,
    )
}

fn drive_env_var_collection_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_id_param(ActionKind::DriveEnvVarCollectionOpen, params)?;
    let object_id = crate::drive::CloudObjectTypeAndId::GenericStringObject {
        object_type: crate::cloud_object::GenericStringObjectFormat::Json(
            crate::cloud_object::JsonObjectType::EnvVarCollection,
        ),
        id: sync_id_from_drive_id(&id)?,
    };
    workspace_action(
        instance_id,
        ActionKind::DriveEnvVarCollectionOpen,
        WorkspaceAction::ViewObjectInWarpDrive(WarpDriveItemId::Object(object_id)),
        ctx,
    )
}

fn drive_object_share_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_id_param(ActionKind::DriveObjectShareOpen, params)?;
    let object_id = cloud_object_type_and_id(&id, ctx)?;
    workspace_action(
        instance_id,
        ActionKind::DriveObjectShareOpen,
        WorkspaceAction::OpenObjectSharingSettings {
            object_id,
            source: SharingDialogSource::CommandPalette,
        },
        ctx,
    )
}

fn active_workspace(
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    target_workspace(action, &TargetSelector::default(), ctx)
}

fn target_workspace(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    let window_id = target_window_id_for_target(ctx, target, action)?;
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the active window",
                    action.as_str()
                ),
            )
        })
}

fn active_pane_group(
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    target_pane_group(action, &TargetSelector::default(), ctx)
}

fn target_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<PaneGroup>, ControlError> {
    let workspace = target_workspace(action, target, ctx)?;
    workspace.read(ctx, |workspace, _| {
        let tab_index = tab_index_from_target(target, workspace)?;
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

fn action_params(params: &serde_json::Value) -> Result<ActionParams, ControlError> {
    if params.as_object().is_some_and(serde_json::Map::is_empty) {
        return Ok(ActionParams::None);
    }
    serde_json::from_value(params.clone()).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "failed to decode action parameters",
            err.to_string(),
        )
    })
}

fn direction_param(
    action: ActionKind,
    params: &serde_json::Value,
) -> Result<ControlDirection, ControlError> {
    match action_params(params)? {
        ActionParams::Direction { direction } => Ok(direction),
        _ => invalid_params(action),
    }
}

fn rename_param(action: ActionKind, params: &serde_json::Value) -> Result<String, ControlError> {
    match action_params(params)? {
        ActionParams::Rename { title } => Ok(title),
        _ => invalid_params(action),
    }
}

fn text_param(action: ActionKind, params: &serde_json::Value) -> Result<String, ControlError> {
    match action_params(params)? {
        ActionParams::Text { text } => Ok(text),
        _ => invalid_params(action),
    }
}

fn drive_id_param(
    action: ActionKind,
    params: &serde_json::Value,
) -> Result<DriveObjectId, ControlError> {
    match action_params(params)? {
        ActionParams::DriveObjectId { id } => Ok(id),
        _ => invalid_params(action),
    }
}

fn invalid_params<T>(action: ActionKind) -> Result<T, ControlError> {
    Err(ControlError::new(
        ErrorCode::InvalidParams,
        format!(
            "{} received parameters with the wrong shape",
            action.as_str()
        ),
    ))
}

fn pane_direction(direction: ControlDirection) -> Result<Direction, ControlError> {
    match direction {
        ControlDirection::Left => Ok(Direction::Left),
        ControlDirection::Right => Ok(Direction::Right),
        ControlDirection::Up => Ok(Direction::Up),
        ControlDirection::Down => Ok(Direction::Down),
        ControlDirection::Previous | ControlDirection::Next => Err(ControlError::new(
            ErrorCode::InvalidParams,
            "pane.split only accepts left, right, up, or down",
        )),
    }
}

fn tab_index_from_target(
    target: &TargetSelector,
    workspace: &Workspace,
) -> Result<usize, ControlError> {
    match target.tab.as_ref() {
        Some(TabTarget::Index { index }) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < workspace.tab_count())
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
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
        Some(TabTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "app-state tab mutations do not support tab title selectors",
        )),
    }
}

fn line_and_column(params: &FileOpenParams) -> Result<Option<LineAndColumnArg>, ControlError> {
    let Some(line) = params.line else {
        return Ok(None);
    };
    let line_num = usize::try_from(line).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "file.open line is out of range",
            err.to_string(),
        )
    })?;
    let column_num = params
        .column
        .map(usize::try_from)
        .transpose()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                "file.open column is out of range",
                err.to_string(),
            )
        })?;
    Ok(Some(LineAndColumnArg {
        line_num,
        column_num,
    }))
}

fn sync_id_from_drive_id(id: &DriveObjectId) -> Result<SyncId, ControlError> {
    parse_drive_server_id(id).map(SyncId::ServerId)
}

fn cloud_object_type_and_id(
    id: &DriveObjectId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<crate::drive::CloudObjectTypeAndId, ControlError> {
    let server_id = parse_drive_server_id(id)?;
    let uid = server_id.uid();
    CloudModel::as_ref(ctx)
        .get_by_uid(&uid)
        .map(|object| object.cloud_object_type_and_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "drive object id did not match a loaded Warp Drive object",
            )
        })
}

fn parse_drive_server_id(id: &DriveObjectId) -> Result<ServerId, ControlError> {
    ServerId::try_from(id.0.as_str()).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive object id must be a server id",
            err.to_string(),
        )
    })
}

fn ack(instance_id: &Option<InstanceId>, action: ActionKind) -> serde_json::Value {
    json!({
        "action": action.as_str(),
        "ok": true,
        "instance_id": instance_id.as_ref().map(|id| id.0.as_str()),
    })
}
