use std::sync::Arc;

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, BlockId, RequestCommandOutputResult, TaskId,
};
use warp_core::command::ExitCode;

use super::{tool_call_label, CommandBlockState};

/// Builds a `Finished` status wrapping the given result.
fn finished(result: AIAgentActionResultType) -> AIActionStatus {
    AIActionStatus::Finished(Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from("action-1".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        result,
    }))
}

/// One end-to-end pass over a tool call's lifecycle: the label text must
/// change as the action moves through constructing (args still streaming),
/// pending, awaiting approval, running, and terminal states.
#[test]
fn label_changes_across_action_lifecycle() {
    let action = AIAgentAction {
        id: AIAgentActionId::from("action-1".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::RequestCommandOutput {
            command: "git status".to_owned(),
            is_read_only: None,
            is_risky: None,
            wait_until_completion: true,
            uses_pager: None,
            rationale: None,
            citations: Vec::new(),
        },
        requires_result: true,
    };
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
            Some(CommandBlockState::Running)
        ),
        "Running `git status`"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(CommandBlockState::Finished {
                exit_code: ExitCode::from(0)
            })
        ),
        "Ran `git status`"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(CommandBlockState::Finished {
                exit_code: ExitCode::from(1)
            })
        ),
        "`git status` exited with code 1"
    );
    assert_eq!(
        tool_call_label(
            &action,
            Some(&snapshot),
            false,
            Some(CommandBlockState::Finished {
                exit_code: ExitCode::from(130)
            })
        ),
        "Cancelled `git status`"
    );
}
