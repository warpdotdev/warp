use ::local_control::protocol::{
    BlockListParams, BlockListResult, BlockOutputFormat, BlockOutputParams, BlockOutputResult,
    BlockSummary, BlockTarget, HistoryEntrySummary, HistoryListParams, HistoryListResult,
    InputRunParams, InputStateResult, PaneTarget, SessionTarget, TabTarget, TargetSelector,
    WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use warpui::{ModelContext, SingletonEntity, ViewHandle};

use crate::local_control::handlers::metadata::require_active_window_id_for_action;
use crate::local_control::LocalControlBridge;
use crate::pane_group::{PaneGroup, PaneId};
use crate::terminal::model::session::SessionId;
use crate::terminal::model::TerminalModel;
use crate::terminal::view::TerminalView;
use crate::terminal::History;
use crate::workspace::Workspace;

pub(super) struct ResolvedTerminalTarget {
    pub(super) terminal_view: ViewHandle<TerminalView>,
}

pub(crate) fn get_input_state(
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let resolved = resolve_terminal_read_target(ActionKind::InputGet, target, ctx)?;
    let session_id = resolved
        .terminal_view
        .read(ctx, |terminal, _| terminal.active_block_session_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "input.get requires a target terminal session",
            )
        })?;
    let input = resolved
        .terminal_view
        .read(ctx, |terminal, _| terminal.input().clone());
    let (text, cursor_offset) = input.read(ctx, |input, ctx| {
        let cursor_offset = input
            .editor()
            .as_ref(ctx)
            .start_byte_index_of_first_selection(ctx)
            .as_usize();
        (input.buffer_text(ctx), cursor_offset)
    });
    let cursor_offset = u32::try_from(cursor_offset).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "input cursor offset is too large to encode",
            err.to_string(),
        )
    })?;
    to_control_data(InputStateResult {
        session_id: session_id.as_u64().to_string(),
        text,
        cursor_offset,
    })
}

pub(crate) fn list_history(
    target: &TargetSelector,
    params: HistoryListParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let resolved = resolve_terminal_read_target(ActionKind::HistoryList, target, ctx)?;
    let session_id = resolved
        .terminal_view
        .read(ctx, |terminal, _| terminal.active_block_session_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "history.list requires a target terminal session",
            )
        })?;
    let commands = History::as_ref(ctx)
        .is_queryable(&session_id)
        .then(|| {
            History::as_ref(ctx)
                .commands_shared(session_id)
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let start_index = params
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
        .map(|limit| commands.len().saturating_sub(limit))
        .unwrap_or_default();
    let entries = commands
        .iter()
        .enumerate()
        .skip(start_index)
        .map(|(index, entry)| HistoryEntrySummary {
            entry_id: format!("history:{}:{index}", session_id.as_u64()),
            command: entry.command.clone(),
            cwd: entry.pwd.clone().unwrap_or_default(),
        })
        .collect();
    to_control_data(HistoryListResult { entries })
}

pub(crate) fn list_blocks(
    target: &TargetSelector,
    params: BlockListParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_block_list_target(target)?;
    let terminal = target_terminal_view(ctx)?;
    let result = terminal.read(ctx, |view, _| {
        let session_id = resolve_session_selector(
            target.session.as_ref(),
            view.active_block_session_id(),
            ActionKind::BlockList,
        )?;
        let model = view.model.lock();
        block_list_result_from_model(&model, session_id, target.session.is_some(), params)
    })?;
    to_control_data(result)
}

pub(crate) fn output_block(
    target: &TargetSelector,
    params: BlockOutputParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_block_output_target(target)?;
    let terminal = target_terminal_view(ctx)?;
    let result = terminal.read(ctx, |view, _| {
        let session_id = resolve_session_selector(
            target.session.as_ref(),
            view.active_block_session_id(),
            ActionKind::BlockOutput,
        )?;
        let model = view.model.lock();
        block_output_result_from_model(&model, session_id, target.block.as_ref(), params)
    })?;
    to_control_data(result)
}

pub(crate) fn run_input_command(
    target: &TargetSelector,
    params: InputRunParams,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let resolved = resolve_terminal_read_target(ActionKind::InputRun, target, ctx)?;
    let session_id = resolved
        .terminal_view
        .read(ctx, |terminal, _| terminal.active_block_session_id())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "input.run requires a target terminal session",
            )
        })?;
    resolved.terminal_view.update(ctx, |terminal, ctx| {
        terminal.execute_command_or_set_pending(&params.command, ctx);
    });
    Ok(serde_json::json!({
        "action": ActionKind::InputRun.as_str(),
        "submitted": true,
        "session_id": session_id.as_u64().to_string(),
    }))
}

pub(crate) fn validate_terminal_read_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    validate_active_terminal_target(action, target)?;
    if matches!(target.session.as_ref(), Some(SessionTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!(
                "{} cannot resolve the requested session id",
                action.as_str()
            ),
        ));
    }
    if !matches!(target.session.as_ref(), None | Some(SessionTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports the active session selector",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_block_list_target(target: &TargetSelector) -> Result<(), ControlError> {
    validate_active_terminal_target(ActionKind::BlockList, target)
}

pub(crate) fn validate_block_output_target(target: &TargetSelector) -> Result<(), ControlError> {
    validate_active_terminal_target(ActionKind::BlockOutput, target)
}

fn validate_active_terminal_target(
    action: ActionKind,
    target: &TargetSelector,
) -> Result<(), ControlError> {
    let action_name = action.as_str();
    if matches!(target.window.as_ref(), Some(WindowTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested window id"),
        ));
    }
    if !matches!(target.window.as_ref(), None | Some(WindowTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active window selector"),
        ));
    }
    if matches!(target.tab.as_ref(), Some(TabTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested tab id"),
        ));
    }
    if !matches!(target.tab.as_ref(), None | Some(TabTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active tab selector"),
        ));
    }
    if matches!(target.pane.as_ref(), Some(PaneTarget::Id { .. })) {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            format!("{action_name} cannot resolve the requested pane id"),
        ));
    }
    if !matches!(target.pane.as_ref(), None | Some(PaneTarget::Active)) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!("{action_name} only supports the active pane selector"),
        ));
    }
    Ok(())
}

pub(super) fn resolve_terminal_read_target(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ResolvedTerminalTarget, ControlError> {
    validate_terminal_read_target(action, target)?;
    let window_id = require_active_window_id_for_action(ctx.windows().active_window(), action)?;
    if let Some(workspace) = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
    {
        let pane_group = workspace.read(ctx, |workspace, _| {
            workspace.active_tab_pane_group().clone()
        });
        return resolve_terminal_in_pane_group(action, target, pane_group, ctx);
    }
    let terminal_view = ctx
        .views_of_type::<TerminalView>(window_id)
        .and_then(|terminals| terminals.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires a target terminal session", action.as_str()),
            )
        })?;
    Ok(ResolvedTerminalTarget { terminal_view })
}

fn resolve_terminal_in_pane_group(
    action: ActionKind,
    target: &TargetSelector,
    pane_group: ViewHandle<PaneGroup>,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ResolvedTerminalTarget, ControlError> {
    let terminal_view = pane_group.read(ctx, |pane_group, ctx| {
        let pane_id = if matches!(target.pane, Some(PaneTarget::Active)) {
            pane_group.focused_pane_id(ctx)
        } else {
            pane_group
                .active_session_id(ctx)
                .map(PaneId::from)
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::MissingTarget,
                        format!("{} requires an active terminal session", action.as_str()),
                    )
                })?
        };
        let terminal_view = pane_group
            .terminal_view_from_pane_id(pane_id, ctx)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::TargetStateConflict,
                    format!(
                        "{} target pane does not contain a terminal session",
                        action.as_str()
                    ),
                )
            })?;
        Ok::<_, ControlError>(terminal_view)
    })?;
    Ok(ResolvedTerminalTarget { terminal_view })
}

fn target_terminal_view(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<TerminalView>, ControlError> {
    let window_id =
        require_active_window_id_for_action(ctx.windows().active_window(), ActionKind::BlockList)?;
    let workspace = ctx
        .views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "block read requires a workspace in the target window",
            )
        })?;
    workspace
        .read(ctx, |workspace, ctx| {
            workspace
                .active_tab_pane_group()
                .read(ctx, |pane_group, ctx| pane_group.active_session_view(ctx))
        })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "block read requires an active terminal session",
            )
        })
}

fn resolve_session_selector(
    target: Option<&SessionTarget>,
    active_session_id: Option<SessionId>,
    action: ActionKind,
) -> Result<SessionId, ControlError> {
    match target {
        None | Some(SessionTarget::Active) => active_session_id.ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!("{} requires an active terminal session", action.as_str()),
            )
        }),
        Some(SessionTarget::Id { id }) => id.0.parse::<u64>().map(SessionId::from).map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidSelector,
                format!("{} received an invalid session id", action.as_str()),
                err.to_string(),
            )
        }),
        Some(SessionTarget::Index { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not support indexed session selectors",
                action.as_str()
            ),
        )),
    }
}

fn block_summary(
    block: &crate::terminal::model::block::Block,
    index: usize,
) -> Option<BlockSummary> {
    let session_id = block.session_id()?;
    let command = block.command_to_string();
    Some(BlockSummary {
        block_id: block.id().to_string(),
        session_id: session_id.as_u64().to_string(),
        index: index as u32,
        command: (!command.is_empty()).then_some(command),
    })
}

fn block_list_result_from_model(
    model: &TerminalModel,
    session_id: SessionId,
    explicit_session: bool,
    params: BlockListParams,
) -> Result<BlockListResult, ControlError> {
    let mut blocks: Vec<BlockSummary> = model
        .block_list()
        .blocks()
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let summary = block_summary(block, index)?;
            (summary.session_id == session_id.as_u64().to_string()).then_some(summary)
        })
        .collect();
    if explicit_session && blocks.is_empty() {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "block.list cannot resolve the requested session id",
        ));
    }
    if let Some(limit) = params.limit {
        let start = blocks.len().saturating_sub(limit as usize);
        blocks = blocks.split_off(start);
    }
    Ok(BlockListResult { blocks })
}

fn block_output_result_from_model(
    model: &TerminalModel,
    session_id: SessionId,
    block_target: Option<&BlockTarget>,
    params: BlockOutputParams,
) -> Result<BlockOutputResult, ControlError> {
    let blocks = model.block_list().blocks();
    let maybe_match = blocks.iter().enumerate().find_map(|(index, block)| {
        if block.session_id() != Some(session_id) {
            return None;
        }
        let matches_block = match block_target {
            None | Some(BlockTarget::Active) => index == blocks.len().saturating_sub(1),
            Some(BlockTarget::Id { id }) => block.id().as_str() == id.0,
            Some(BlockTarget::Index {
                index: target_index,
            }) => index as u32 == *target_index,
        };
        matches_block.then(|| (index, block))
    });
    let Some((index, block)) = maybe_match else {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "block.output cannot resolve the requested block selector",
        ));
    };
    let output = match params.format {
        BlockOutputFormat::Plain | BlockOutputFormat::Ansi | BlockOutputFormat::Json => {
            Some(block.output_to_string())
        }
    };
    let block = block_summary(block, index).ok_or_else(|| {
        ControlError::new(
            ErrorCode::TargetStateConflict,
            "block.output target block has no terminal session",
        )
    })?;
    Ok(BlockOutputResult { block, output })
}

fn to_control_data<T: serde::Serialize>(data: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(data).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode local-control response",
            err.to_string(),
        )
    })
}
