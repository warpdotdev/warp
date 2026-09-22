use std::sync::Arc;

use ai::agent::action_result::{
    RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
    RunAgentsResult,
};
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionId, AIAgentActionResult, AIAgentActionResultType,
    AIAgentActionType, Appearance, BlockId, RequestCommandOutputResult, TaskId,
};
use warp_core::command::ExitCode;
use warpui::App;
use warpui_core::elements::tui::Modifier;

use super::{
    CommandBlockState, ResolvedCommandBlock, ToolCallDisplayState, launched_agents_label,
    styled_tool_call_label_spans, tool_call_display_state, tool_call_label,
    tool_call_label_with_server,
};
use crate::tui_builder::TuiUiBuilder;

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

fn failed_agent(name: &str) -> RunAgentsAgentOutcome {
    RunAgentsAgentOutcome {
        name: name.to_owned(),
        resolved_model_id: String::new(),
        kind: RunAgentsAgentOutcomeKind::Failed {
            error: "launch failed".to_owned(),
        },
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

/// Builds a `CallMCPTool` action for `tool`. The `server_id` is left `None`
/// because `tool_call_label_with_server` takes the resolved server name as a
/// direct argument, bypassing the action's server-id -> name lookup.
fn mcp_tool_action(tool: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("action-1".to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::CallMCPTool {
            server_id: None,
            name: tool.to_owned(),
            input: serde_json::Value::Null,
        },
        requires_result: true,
    }
}

#[test]
fn tool_call_statuses_map_to_tool_call_display_states() {
    assert_eq!(
        tool_call_display_state(None, true, None),
        ToolCallDisplayState::Constructing
    );
    assert_eq!(
        tool_call_display_state(None, false, None),
        ToolCallDisplayState::Pending
    );
    assert_eq!(
        tool_call_display_state(Some(&AIActionStatus::Blocked), false, None),
        ToolCallDisplayState::Blocked
    );
    assert_eq!(
        tool_call_display_state(Some(&AIActionStatus::RunningAsync), false, None),
        ToolCallDisplayState::Running
    );
}

#[test]
fn all_failed_run_agents_uses_failure_glyph() {
    let agents = vec![failed_agent("first"), failed_agent("second")];
    assert_eq!(launched_agents_label(&agents), "Failed to spawn 2 agents");
    let status = finished(AIAgentActionResultType::RunAgents(
        RunAgentsResult::Launched {
            model_id: "auto".to_owned(),
            harness_type: "oz".to_owned(),
            execution_mode: RunAgentsLaunchedExecutionMode::Local,
            agents,
        },
    ));

    let state = tool_call_display_state(Some(&status), false, None);

    assert_eq!(state, ToolCallDisplayState::Failed);
    assert_eq!(state.glyph(), "×");
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
            activity: None,
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

#[test]
fn shell_command_label_preserves_a_long_path_without_an_ellipsis() {
    let command = "ls -la /Users/moirahuang/.warp-dev/worktrees/warp/moira/pr-14381-combined/crates/warp_tui/src/tui_shell_command_view.rs";
    let action = command_action(command);

    assert_eq!(
        tool_call_label(&action, None, false, None),
        format!("Run `{command}`")
    );
}
/// An MCP tool call's transcript label must surface both the tool name and its
/// originating server across every lifecycle state, with a deterministic
/// no-server fallback (legacy/flat MCP call or unknown server).
#[test]
fn mcp_tool_call_label_surfaces_tool_and_server_across_lifecycle() {
    let action = mcp_tool_action("create_issue");
    let server = Some("github");

    // Constructing: the tool name may still be empty while args stream in.
    let constructing_empty = mcp_tool_action("");
    assert_eq!(
        tool_call_label_with_server(&constructing_empty, None, true, None, None),
        "Calling MCP tool…"
    );
    assert_eq!(
        tool_call_label_with_server(&constructing_empty, None, true, None, server),
        "Calling MCP tool on github…"
    );
    assert_eq!(
        tool_call_label_with_server(&action, None, true, None, None),
        "Calling \"create_issue\" MCP tool…"
    );
    assert_eq!(
        tool_call_label_with_server(&action, None, true, None, server),
        "Calling \"create_issue\" MCP tool on github…"
    );

    // Pending.
    assert_eq!(
        tool_call_label_with_server(&action, None, false, None, None),
        "Call MCP tool create_issue"
    );
    assert_eq!(
        tool_call_label_with_server(&action, None, false, None, server),
        "Call MCP tool create_issue on github"
    );

    // Blocked / awaiting approval.
    assert_eq!(
        tool_call_label_with_server(&action, Some(&AIActionStatus::Blocked), false, None, None),
        "Call MCP tool create_issue (awaiting approval)"
    );
    assert_eq!(
        tool_call_label_with_server(&action, Some(&AIActionStatus::Blocked), false, None, server),
        "Call MCP tool create_issue on github (awaiting approval)"
    );

    // Running.
    assert_eq!(
        tool_call_label_with_server(
            &action,
            Some(&AIActionStatus::RunningAsync),
            false,
            None,
            server
        ),
        "Calling MCP tool create_issue on github"
    );

    // Terminal states are driven through a resolved command block so the label
    // text can be exercised without constructing an rmcp `CallToolResult`.
    let succeeded = block(CommandBlockState::Finished {
        exit_code: ExitCode::from(0),
    });
    assert_eq!(
        tool_call_label_with_server(&action, None, false, Some(&succeeded), server),
        "Called MCP tool create_issue on github"
    );
    let failed = block(CommandBlockState::Finished {
        exit_code: ExitCode::from(1),
    });
    assert_eq!(
        tool_call_label_with_server(&action, None, false, Some(&failed), server),
        "MCP tool create_issue on github failed"
    );
    let cancelled = block(CommandBlockState::Finished {
        exit_code: ExitCode::from(130),
    });
    assert_eq!(
        tool_call_label_with_server(&action, None, false, Some(&cancelled), server),
        "MCP tool create_issue on github cancelled"
    );

    // No server (legacy/flat MCP call or unknown server): tool name only.
    assert_eq!(
        tool_call_label_with_server(&action, None, false, Some(&succeeded), None),
        "Called MCP tool create_issue"
    );
}

#[test]
fn tool_call_label_spans_bold_only_the_first_word() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let spans = styled_tool_call_label_spans("Grepped for needle in src", &builder);
            assert_eq!(spans[0].0, "Grepped");
            assert_eq!(spans[1].0, " for needle in src");
            assert_eq!(spans[0].1.fg, builder.primary_text_style().fg);
            assert!(spans[0].1.add_modifier.contains(Modifier::BOLD));
            assert_eq!(spans[1].1.fg, builder.neutral_7_text_style().fg);
            assert!(!spans[1].1.add_modifier.contains(Modifier::BOLD));

            let subject_first =
                styled_tool_call_label_spans("MCP tool create_issue failed", &builder);
            assert_eq!(
                subject_first
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                vec!["MCP", " tool create_issue failed"]
            );
            assert!(subject_first[0].1.add_modifier.contains(Modifier::BOLD));
            assert_eq!(subject_first[1].1.fg, builder.neutral_7_text_style().fg);
        });
    });
}
