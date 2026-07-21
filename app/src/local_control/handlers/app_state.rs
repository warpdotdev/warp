//! Safe app-state mutation and visible UI intent handlers for local-control actions.
#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;

#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::str::FromStr;

use ::local_control::protocol::{
    Direction as ControlDirection, DirectionParams, FileOpenParams, PageQueryParams, QueryParams,
    ResizeParams, TabActivateParams, TabActivationMode, TabCreateParams, TabTarget, TabType,
    TargetSelector, TextParams,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use serde_json::json;
use warp_util::path::LineAndColumnArg;
#[cfg(feature = "local_fs")]
use warpui::SingletonEntity;
use warpui::{AppContext, ModelContext, TypedActionView};

#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::local_control::LocalControlBridge;
use crate::local_control::handlers::ack;
use crate::local_control::handlers::layout::{create_tab, resolve_shell};
use crate::local_control::handlers::metadata::{SurfaceDestination, surface_unavailable_reason};
use crate::local_control::resolver::{
    activate_target, active_target_pane_group, decode_params, focus_explicit_pane_target,
    input_target_pane_id, reject_target_families, tab_index_from_target, target_pane_group,
    target_pane_id, target_session_pane_id, target_window_id_for_target, target_workspace,
};
use crate::palette::PaletteMode;
use crate::pane_group::{ActivationReason, Direction, PaneGroupAction};
use crate::server::telemetry::PaletteSource;
use crate::settings_view::SettingsSection;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{EditorLayout, resolve_file_target_to_open_in_warp};
#[cfg(feature = "local_fs")]
use crate::workspace::PaneViewLocator;
use crate::workspace::{CommandSearchOptions, InitContent, WorkspaceAction};

const MAX_PANE_RESIZE_STEPS: u32 = 1_000;

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
        ActionKind::WindowCreate => window_create(instance_id, params, target, ctx),
        ActionKind::TabCreate => create_tab(instance_id, params, target, ctx),
        ActionKind::TabActivate => tab_activate(instance_id, params, target, ctx),
        ActionKind::TabMove => tab_move(instance_id, params, target, ctx),
        ActionKind::PaneSplit => pane_split(instance_id, params, target, ctx),
        ActionKind::PaneFocus | ActionKind::SessionActivate => {
            pane_focus(instance_id, action, target, ctx)
        }
        ActionKind::PaneNavigate => pane_direction_action(instance_id, action, params, target, ctx),
        ActionKind::PaneResize => pane_resize(instance_id, params, target, ctx),
        ActionKind::PaneMaximize => pane_maximize(instance_id, true, target, ctx),
        ActionKind::PaneUnmaximize => pane_maximize(instance_id, false, target, ctx),
        ActionKind::SessionPrevious => workspace_action(
            instance_id,
            action,
            WorkspaceAction::CyclePrevSession,
            target,
            ctx,
        ),
        ActionKind::SurfaceKeybindingsOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::Keybindings,
            WorkspaceAction::ShowSettingsPage(SettingsSection::Keybindings),
            target,
            ctx,
        ),
        ActionKind::SurfaceWarpDriveOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::WarpDrive,
            WorkspaceAction::OpenWarpDrive,
            target,
            ctx,
        ),
        ActionKind::SurfaceAgentManagementOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::AgentManagement,
            WorkspaceAction::OpenAgentManagementView,
            target,
            ctx,
        ),
        ActionKind::SessionNext => workspace_action(
            instance_id,
            action,
            WorkspaceAction::CycleNextSession,
            target,
            ctx,
        ),
        ActionKind::SessionReopenClosed => session_reopen_closed(instance_id, target, ctx),
        ActionKind::InputInsert => input_text(instance_id, action, params, target, false, ctx),
        ActionKind::InputReplace => input_text(instance_id, action, params, target, true, ctx),
        ActionKind::SurfaceSettingsOpen => surface_settings_open(instance_id, params, target, ctx),
        ActionKind::SurfaceCommandPaletteOpen => surface_palette_open(
            instance_id,
            action,
            PaletteMode::Command,
            params,
            target,
            ctx,
        ),
        ActionKind::SurfaceCommandSearchOpen => {
            surface_command_search_open(instance_id, params, target, ctx)
        }
        ActionKind::SurfaceThemePickerOpen => surface_theme_picker_open(instance_id, target, ctx),
        ActionKind::SurfaceWarpDriveToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleWarpDrive,
            target,
            ctx,
        ),
        ActionKind::SurfaceResourceCenterToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleResourceCenter,
            target,
            ctx,
        ),
        ActionKind::SurfaceAiAssistantToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleAIAssistant,
            target,
            ctx,
        ),
        ActionKind::SurfaceCodeReviewOpen => surface_code_review_open(instance_id, target, ctx),
        ActionKind::SurfaceCodeReviewToggle | ActionKind::SurfaceRightPanelToggle => {
            workspace_action(
                instance_id,
                action,
                WorkspaceAction::ToggleRightPanel,
                target,
                ctx,
            )
        }
        ActionKind::SurfaceProjectExplorerOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::ProjectExplorer,
            WorkspaceAction::OpenProjectExplorer,
            target,
            ctx,
        ),
        ActionKind::SurfaceGlobalSearchOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::GlobalSearch,
            WorkspaceAction::OpenGlobalSearch,
            target,
            ctx,
        ),
        ActionKind::SurfaceConversationListOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::ConversationList,
            WorkspaceAction::OpenConversationListView,
            target,
            ctx,
        ),
        ActionKind::SurfaceLeftPanelToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleLeftPanel,
            target,
            ctx,
        ),
        ActionKind::SurfaceVerticalTabsOpen => surface_workspace_action(
            instance_id,
            action,
            SurfaceDestination::VerticalTabs,
            WorkspaceAction::OpenVerticalTabsPanel,
            target,
            ctx,
        ),
        ActionKind::SurfaceVerticalTabsToggle => workspace_action(
            instance_id,
            action,
            WorkspaceAction::ToggleVerticalTabsPanel,
            target,
            ctx,
        ),
        ActionKind::FileOpen => file_open(instance_id, params, target, ctx),
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a safe app-state handler action", action.as_str()),
        )),
    }
}

fn focus_window(
    instance_id: &Option<InstanceId>,
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        action,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, target, action)?;
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(ack(instance_id, action))
}

fn window_create(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::WindowCreate,
        target.window.is_some()
            || target.tab.is_some()
            || target.pane.is_some()
            || target.session.is_some(),
        "target selectors",
    )?;
    let params = decode_params::<TabCreateParams>(params)?;
    match params.tab_type {
        None | Some(TabType::Terminal | TabType::Default) => {}
        Some(TabType::Agent | TabType::CloudAgent) => {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "window.create only supports terminal or default window types",
            ));
        }
    }
    match params.shell.as_deref() {
        Some(shell_name) => {
            let shell = resolve_shell(shell_name, ctx)?;
            ctx.dispatch_global_action("root_view:open_new_with_shell", Some(shell));
        }
        None => ctx.dispatch_global_action("root_view:open_new", ()),
    }
    Ok(ack(instance_id, ActionKind::WindowCreate))
}

fn workspace_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    action: WorkspaceAction,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let workspace = target_workspace(action_kind, target, ctx)?;
    activate_target(&workspace, action_kind, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        workspace.handle_action(&action, ctx);
    });
    Ok(ack(instance_id, action_kind))
}

fn surface_workspace_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    destination: SurfaceDestination,
    action: WorkspaceAction,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    ensure_surface_available(action_kind, destination, ctx)?;
    workspace_action(instance_id, action_kind, action, target, ctx)
}

fn surface_theme_picker_open(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action = ActionKind::SurfaceThemePickerOpen;
    ensure_surface_available(action, SurfaceDestination::ThemePicker, ctx)?;
    let workspace = target_workspace(action, target, ctx)?;
    activate_target(&workspace, action, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        if !workspace.is_theme_chooser_open() {
            workspace.handle_action(&WorkspaceAction::ShowThemeChooserForActiveTheme, ctx);
        }
    });
    Ok(ack(instance_id, action))
}

fn surface_code_review_open(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action = ActionKind::SurfaceCodeReviewOpen;
    ensure_surface_available(action, SurfaceDestination::CodeReview, ctx)?;
    let workspace = target_workspace(action, target, ctx)?;
    activate_target(&workspace, action, target, ctx)?;
    #[cfg(feature = "local_fs")]
    {
        let pane_group = target_pane_group(action, target, ctx)?;
        let pane_id = target_pane_id(action, target, &pane_group, ctx)?;
        let has_repository = pane_group.read(ctx, |pane_group, ctx| {
            pane_group
                .terminal_view_from_pane_id(pane_id, ctx)
                .is_some_and(|terminal| terminal.as_ref(ctx).current_repo_path().is_some())
        });
        if !has_repository {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "surface.code_review.open requires an active terminal in a repository",
            ));
        }
        workspace.update(ctx, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::OpenCodeReviewPanel(PaneViewLocator {
                    pane_group_id: pane_group.id(),
                    pane_id,
                }),
                ctx,
            );
        });
        Ok(ack(instance_id, action))
    }
    #[cfg(not(feature = "local_fs"))]
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        "surface.code_review.open is unavailable without local filesystem support",
    ))
}

fn ensure_surface_available(
    action: ActionKind,
    destination: SurfaceDestination,
    ctx: &AppContext,
) -> Result<(), ControlError> {
    let Some(reason) = surface_unavailable_reason(destination, ctx) else {
        return Ok(());
    };
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        format!("{} is unavailable: {reason}", action.as_str()),
    ))
}

fn session_reopen_closed(
    instance_id: &Option<InstanceId>,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::SessionReopenClosed,
        target.tab.is_some() || target.pane.is_some() || target.session.is_some(),
        "tab, pane, or session selectors",
    )?;
    let window_id = target_window_id_for_target(ctx, target, ActionKind::SessionReopenClosed)?;
    ctx.windows().show_window_and_focus_app(window_id);
    workspace_action(
        instance_id,
        ActionKind::SessionReopenClosed,
        WorkspaceAction::ReopenClosedSession,
        target,
        ctx,
    )
}

fn tab_activate(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabActivate,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let mode = decode_params::<TabActivateParams>(params)?.mode;
    if !matches!(mode, TabActivationMode::Target)
        && !matches!(target.tab.as_ref(), None | Some(TabTarget::Active))
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "tab.activate navigation modes do not accept a concrete tab selector",
        ));
    }
    let workspace = target_workspace(ActionKind::TabActivate, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let action = match mode {
            TabActivationMode::Target => {
                WorkspaceAction::ActivateTab(tab_index_from_target(target, workspace, ctx)?)
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
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    reject_target_families(
        ActionKind::TabMove,
        target.pane.is_some() || target.session.is_some(),
        "pane or session selectors",
    )?;
    let direction = direction_param(params)?;
    let workspace = target_workspace(ActionKind::TabMove, target, ctx)?;
    workspace.update(ctx, |workspace, ctx| {
        let index = tab_index_from_target(target, workspace, ctx)?;
        let action = match direction {
            ControlDirection::Left => WorkspaceAction::MoveTabLeft(index),
            ControlDirection::Right => WorkspaceAction::MoveTabRight(index),
            ControlDirection::Up
            | ControlDirection::Down
            | ControlDirection::Previous
            | ControlDirection::Next => {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    "tab.move only accepts left or right",
                ));
            }
        };
        workspace.handle_action(&action, ctx);
        Ok::<_, ControlError>(())
    })?;
    Ok(ack(instance_id, ActionKind::TabMove))
}

fn pane_direction_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let direction = direction_param(params)?;
    let action = match action_kind {
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
    pane_group_action(instance_id, action_kind, target, action, 1, ctx)
}

/// Splits the targeted pane and reports the created pane's opaque id so
/// callers do not need to diff `pane.list` to find the new pane.
fn pane_split(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action_kind = ActionKind::PaneSplit;
    let direction = pane_direction(direction_param(params)?)?;
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    focus_explicit_pane_target(action_kind, target, &pane_group, ctx)?;
    let panes_before = pane_group.read(ctx, |pane_group, _| pane_group.visible_pane_ids());
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(&PaneGroupAction::Add(direction), ctx);
    });
    let created = pane_group
        .read(ctx, |pane_group, _| pane_group.visible_pane_ids())
        .into_iter()
        .find(|pane_id| !panes_before.contains(pane_id));
    let mut response = ack(instance_id, action_kind);
    if let Some(created) = created {
        response["pane"] = json!({ "id": created.to_string() });
    }
    Ok(response)
}

fn pane_focus(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    if target.pane.is_none()
        && (action_kind != ActionKind::SessionActivate || target.session.is_none())
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{} requires a pane or session target", action_kind.as_str()),
        ));
    }
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    let pane_id = if action_kind == ActionKind::SessionActivate {
        target_session_pane_id(action_kind, target, &pane_group, ctx)?
    } else {
        target_pane_id(action_kind, target, &pane_group, ctx)?
    };
    pane_group.update(ctx, |pane_group, ctx| {
        pane_group.handle_action(
            &PaneGroupAction::Activate(pane_id, ActivationReason::Click),
            ctx,
        );
    });
    Ok(ack(instance_id, action_kind))
}

fn pane_resize(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let ResizeParams { direction, amount } = decode_params(params)?;
    let amount = amount.unwrap_or(1);
    if amount > MAX_PANE_RESIZE_STEPS {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("pane.resize amount cannot exceed {MAX_PANE_RESIZE_STEPS}"),
        ));
    }
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
    pane_group_action(
        instance_id,
        ActionKind::PaneResize,
        target,
        action,
        amount,
        ctx,
    )
}

fn pane_maximize(
    instance_id: &Option<InstanceId>,
    should_maximize: bool,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let action_kind = if should_maximize {
        ActionKind::PaneMaximize
    } else {
        ActionKind::PaneUnmaximize
    };
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    focus_explicit_pane_target(action_kind, target, &pane_group, ctx)?;
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

fn pane_group_action(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    target: &TargetSelector,
    action: PaneGroupAction,
    repetitions: u32,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let pane_group = active_target_pane_group(action_kind, target, ctx)?;
    focus_explicit_pane_target(action_kind, target, &pane_group, ctx)?;
    pane_group.update(ctx, |pane_group, ctx| {
        for _ in 0..repetitions {
            pane_group.handle_action(&action, ctx);
        }
    });
    Ok(ack(instance_id, action_kind))
}

fn input_text(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    params: &serde_json::Value,
    target: &TargetSelector,
    replace_buffer: bool,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let text = text_param(params)?;
    validate_staged_input_text(action_kind, &text)?;
    let pane_group = target_pane_group(action_kind, target, ctx)?;
    let pane_id = input_target_pane_id(action_kind, target, &pane_group, ctx)?;
    let terminal_view = pane_group
        .read(ctx, |pane_group, ctx| {
            pane_group.terminal_view_from_pane_id(pane_id, ctx)
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires a terminal input target", action_kind.as_str()),
            )
        })?;
    terminal_view.update(ctx, |terminal_view, ctx| {
        terminal_view.input().update(ctx, |input, ctx| {
            if replace_buffer {
                input.replace_buffer_content(&text, ctx);
            } else {
                input.append_to_buffer(&text, ctx);
            }
        });
    });
    Ok(ack(instance_id, action_kind))
}

pub(super) fn validate_staged_input_text(
    action: ActionKind,
    text: &str,
) -> Result<(), ControlError> {
    if text
        .chars()
        .any(|character| character == '\n' || character == '\r' || character.is_control())
    {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} rejects newlines, carriage returns, and control characters",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn surface_settings_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let PageQueryParams { page, query } = decode_params(params)?;
    let section = page.map(settings_section).transpose()?;
    let action = match (section, query) {
        (Some(section), Some(search_query)) => WorkspaceAction::ShowSettingsPageWithSearch {
            search_query,
            section: Some(section),
        },
        (Some(section), None) => WorkspaceAction::ShowSettingsPage(section),
        (None, Some(search_query)) => WorkspaceAction::ShowSettingsPageWithSearch {
            search_query,
            section: None,
        },
        (None, None) => WorkspaceAction::ShowSettings,
    };
    workspace_action(
        instance_id,
        ActionKind::SurfaceSettingsOpen,
        action,
        target,
        ctx,
    )
}

fn settings_section(page: String) -> Result<SettingsSection, ControlError> {
    let section = SettingsSection::from_str(&page).map_err(|_| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("surface.settings.open cannot resolve settings page {page:?}"),
        )
    })?;
    if section == SettingsSection::WarpDrive {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "surface.settings.open does not open Warp Drive settings",
        ));
    }
    Ok(section)
}

fn surface_palette_open(
    instance_id: &Option<InstanceId>,
    action_kind: ActionKind,
    mode: PaletteMode,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = decode_params::<QueryParams>(params)?.query;
    workspace_action(
        instance_id,
        action_kind,
        WorkspaceAction::OpenPalette {
            mode,
            source: PaletteSource::Keybinding,
            query,
        },
        target,
        ctx,
    )
}

fn surface_command_search_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let query = decode_params::<QueryParams>(params)?.query;
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
        target,
        ctx,
    )
}

fn file_open(
    instance_id: &Option<InstanceId>,
    params: &serde_json::Value,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = decode_params::<FileOpenParams>(params)?;
    if params.path.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "file.open requires a non-empty path",
        ));
    }
    let line_and_column = line_and_column(&params)?;
    let workspace = target_workspace(ActionKind::FileOpen, target, ctx)?;
    activate_target(&workspace, ActionKind::FileOpen, target, ctx)?;
    #[cfg(feature = "local_fs")]
    {
        let path = PathBuf::from(params.path);
        let layout = params.new_tab.then_some(EditorLayout::NewTab);
        let file_target =
            resolve_file_target_to_open_in_warp(&path, EditorSettings::as_ref(ctx), layout);
        workspace.update(ctx, |workspace, ctx| {
            workspace.open_file_with_target(
                path.clone(),
                file_target,
                line_and_column,
                CodeSource::Link {
                    path,
                    range_start: None,
                    range_end: None,
                },
                ctx,
            );
        });
        Ok(ack(instance_id, ActionKind::FileOpen))
    }
    #[cfg(not(feature = "local_fs"))]
    Err(ControlError::new(
        ErrorCode::UnsupportedAction,
        "file.open is unavailable without local filesystem support",
    ))
}

fn direction_param(params: &serde_json::Value) -> Result<ControlDirection, ControlError> {
    Ok(decode_params::<DirectionParams>(params)?.direction)
}
fn text_param(params: &serde_json::Value) -> Result<String, ControlError> {
    Ok(decode_params::<TextParams>(params)?.text)
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

fn line_and_column(params: &FileOpenParams) -> Result<Option<LineAndColumnArg>, ControlError> {
    let Some(line) = params.line else {
        if params.column.is_some() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "file.open column requires a line",
            ));
        }
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
