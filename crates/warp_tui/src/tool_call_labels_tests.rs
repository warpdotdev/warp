use std::sync::Arc;

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, BlockId, RequestCommandOutputResult, TaskId,
};
use warp_core::command::ExitCode;

use super::{tool_call_label, CommandBlockState, ResolvedCommandBlock};

/// Builds a `Finished` status wrapping the given result.
fn finished(result: AIAgentActionResultType) -> AIActionStatus {
    AIActionStatus::Finished(Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from("action-1".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        result,
    }))
}

/// Builds a resolved command block without a command of its own.
fn block(state: CommandBlockState) -> ResolvedCommandBlock {
    ResolvedCommandBlock {
        command: None,
        state,
    }
}

/// Builds a `RequestCommandOutput` tool-call action for `command`.
fn command_action(command: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("action-1".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: command.to_owned(),
            is_read_only: None,
            is_risky: None,
            wait_until_completion: true,
            uses_pager: None,
            rationale: None,
            citations: Vec::new(),
        },
        requires_result: true,
    }
}

/// One end-to-end pass over a tool call's lifecycle: the label text must
/// change as the action moves through constructing (args still streaming),
/// pending, awaiting approval, running, and terminal states.
#[test]
fn label_changes_across_action_lifecycle() {
    let action = command_action("git status");
    // No status while the output is still streaming: args may be partial.
    assert_eq!(
        tool_call_label(&action, None, true, None),
        "Generating command…"
    );
    assert_eq!(
        tool_call_label(&action, None, false, None),
        "Run `git status`"
    );
    assert_eq!(
        tool_call_label(&action, Some(&AIActionStatus::Blocked), false, None),
        "Run `git status` (awaiting approval)"
    );
    assert_eq!(
        tool_call_label(&action, Some(&AIActionStatus::RunningAsync), false, None),
        "Running `git status`"
    );
    let cancelled = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::CancelledBeforeExecution,
    ));
    assert_eq!(
        tool_call_label(&action, Some(&cancelled), false, None),
        "Cancelled `git status`"
    );
    let failed = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::Denylisted {
            command: "git status".to_owned(),
        },
    ));
    assert_eq!(
        tool_call_label(&action, Some(&failed), false, None),
        "`git status` denied (denylisted)"
    );

    // Agent-monitored command: the stored result stays a snapshot forever, so
    // the terminal block's resolved state drives the label whenever the block
    // exists; the snapshot is only the no-block fallback.
    let snapshot = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::LongRunningCommandSnapshot {
            block_id: BlockId::new(),
            command: "git status".to_owned(),
            grid_contents: String::new(),
            cursor: String::new(),
            is_alt_screen_active: false,
        },
    ));
    assert_eq!(
        tool_call_label(&action, Some(&snapshot), false, None),
        "`git status` is still running"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(&block(CommandBlockState::Running))
        ),
        "Running `git status`"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(&block(CommandBlockState::Finished {
                exit_code: ExitCode::from(0)
            }))
        ),
        "Ran `git status`"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(&block(CommandBlockState::Finished {
                exit_code: ExitCode::from(1)
            }))
        ),
        "`git status` exited with code 1"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(&block(CommandBlockState::Finished {
                exit_code: ExitCode::from(130)
            }))
        ),
        "Cancelled `git status`"
    );
}

/// An accepted command can be edited before execution, so the streamed
/// command may be stale: the executed command from the finished result or
/// the resolved block must supersede it in the label.
#[test]
fn label_prefers_executed_command_over_streamed_command() {
    let action = command_action("git status");

    // Finished result carries the executed (edited) command.
    let completed = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::Completed {
            block_id: BlockId::new(),
            command: "git status -sb".to_owned(),
            output: String::new(),
            exit_code: ExitCode::from(0),
            start_ts: None,
            completed_ts: None,
        },
    ));
    assert_eq!(
        tool_call_label(&action, Some(&completed), false, None),
        "Ran `git status -sb`"
    );

    // No result yet while executing: the resolved block's command wins.
    let running_block = ResolvedCommandBlock {
        command: Some("git status -sb".to_owned()),
        state: CommandBlockState::Running,
    };
    assert_eq!(
        tool_call_label(
            &action,
            Some(&AIActionStatus::RunningAsync),
            false,
            Some(&running_block)
        ),
        "Running `git status -sb`"
    );

    // A block without a command falls back to the streamed command.
    assert_eq!(
        tool_call_label(
            &action,
            Some(&AIActionStatus::RunningAsync),
            false,
            Some(&block(CommandBlockState::Running))
        ),
        "Running `git status`"
    );
}
