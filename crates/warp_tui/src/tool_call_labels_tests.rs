use std::sync::Arc;

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, RequestCommandOutputResult, TaskId,
};

use super::tool_call_label;

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
    assert_eq!(tool_call_label(&action, None, true), "Generating command…");
    assert_eq!(tool_call_label(&action, None, false), "Run `git status`");
    assert_eq!(
        tool_call_label(&action, Some(&AIActionStatus::Blocked), false),
        "Run `git status` (awaiting approval)"
    );
    assert_eq!(
        tool_call_label(&action, Some(&AIActionStatus::RunningAsync), false),
        "Running `git status`"
    );
    let cancelled = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::CancelledBeforeExecution,
    ));
    assert_eq!(
        tool_call_label(&action, Some(&cancelled), false),
        "Cancelled `git status`"
    );
    let failed = finished(AIAgentActionResultType::RequestCommandOutput(
        RequestCommandOutputResult::Denylisted {
            command: "git status".to_owned(),
        },
    ));
    assert_eq!(
        tool_call_label(&action, Some(&failed), false),
        "`git status` denied (denylisted)"
    );
}
