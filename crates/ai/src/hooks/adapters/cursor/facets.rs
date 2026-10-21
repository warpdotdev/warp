//! Views of an envelope payload for the shell and MCP facets of the tool boundaries. A facet is
//! one vendor event that projects onto a shared boundary and sees only part of its payload
//! (architecture.md §4.1): Cursor's shell and MCP events sit beside `preToolUse` and
//! `postToolUse` on `tool.pre` and `tool.result`. Matching and encoding both pull the same pieces
//! out of Warp's action and result types, so they share these views.

use chrono::{DateTime, Local};
use rmcp::model::CallToolResult;
use uuid::Uuid;

use crate::agent::action::AIAgentActionType;
use crate::agent::action_result::{
    AIAgentActionResultType, CallMCPToolResult, ReadShellCommandOutputResult,
    RequestCommandOutputResult,
};
use crate::hooks::model::HookPayload;

/// The model's shell command before it runs.
pub(super) fn pending_shell_command(payload: &HookPayload) -> Option<&str> {
    match payload {
        HookPayload::ToolPre {
            action: AIAgentActionType::RequestCommandOutput { command, .. },
            ..
        } => Some(command),
        HookPayload::ToolPre { .. }
        | HookPayload::SessionStart { .. }
        | HookPayload::PromptSubmit { .. }
        | HookPayload::ToolResult { .. }
        | HookPayload::ToolFailure { .. }
        | HookPayload::TurnStop { .. }
        | HookPayload::ResponseComplete { .. }
        | HookPayload::ReasoningComplete { .. } => None,
    }
}

/// A shell command that ran to completion, as a tool result reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FinishedShellCommand<'a> {
    pub command: &'a str,
    pub output: &'a str,
    /// Absent when the shell itself ended before reporting a code.
    pub exit_code: Option<i32>,
    pub start_ts: Option<DateTime<Local>>,
    pub completed_ts: Option<DateTime<Local>>,
}

/// The finished shell command in a result payload. A snapshot of a still-running command, a
/// command that never started, and results whose text does not name the command are not
/// finished commands.
pub(super) fn finished_shell_command(payload: &HookPayload) -> Option<FinishedShellCommand<'_>> {
    let result = match payload {
        HookPayload::ToolResult { result, .. } | HookPayload::ToolFailure { result, .. } => result,
        HookPayload::ToolPre { .. }
        | HookPayload::SessionStart { .. }
        | HookPayload::PromptSubmit { .. }
        | HookPayload::TurnStop { .. }
        | HookPayload::ResponseComplete { .. }
        | HookPayload::ReasoningComplete { .. } => return None,
    };
    match result {
        AIAgentActionResultType::RequestCommandOutput(result) => match result {
            RequestCommandOutputResult::Completed {
                command,
                output,
                exit_code,
                start_ts,
                completed_ts,
                block_id: _,
            } => Some(FinishedShellCommand {
                command,
                output,
                exit_code: Some(exit_code.value()),
                start_ts: *start_ts,
                completed_ts: *completed_ts,
            }),
            RequestCommandOutputResult::ShellRecovered {
                command,
                output,
                start_ts,
                completed_ts,
                ..
            } => Some(FinishedShellCommand {
                command,
                output,
                exit_code: None,
                start_ts: *start_ts,
                completed_ts: *completed_ts,
            }),
            RequestCommandOutputResult::LongRunningCommandSnapshot { .. }
            | RequestCommandOutputResult::CancelledBeforeExecution
            | RequestCommandOutputResult::Denylisted { .. }
            | RequestCommandOutputResult::HookDenied { .. }
            | RequestCommandOutputResult::TerminalBusy { .. } => None,
        },
        AIAgentActionResultType::ReadShellCommandOutput(result) => match result {
            ReadShellCommandOutputResult::CommandFinished {
                command,
                output,
                exit_code,
                start_ts,
                completed_ts,
                block_id: _,
            } => Some(FinishedShellCommand {
                command,
                output,
                exit_code: Some(exit_code.value()),
                start_ts: *start_ts,
                completed_ts: *completed_ts,
            }),
            ReadShellCommandOutputResult::ShellRecovered {
                command,
                output,
                start_ts,
                completed_ts,
                ..
            } => Some(FinishedShellCommand {
                command,
                output,
                exit_code: None,
                start_ts: *start_ts,
                completed_ts: *completed_ts,
            }),
            ReadShellCommandOutputResult::LongRunningCommandSnapshot { .. }
            | ReadShellCommandOutputResult::Cancelled
            | ReadShellCommandOutputResult::Error(_) => None,
        },
        // These results do not carry the command text, which is what the event's matcher is
        // tested against.
        AIAgentActionResultType::WriteToLongRunningShellCommand(_)
        | AIAgentActionResultType::TransferShellCommandControlToUser(_) => None,
        AIAgentActionResultType::RequestFileEdits(_)
        | AIAgentActionResultType::ReadFiles(_)
        | AIAgentActionResultType::UploadArtifact(_)
        | AIAgentActionResultType::SearchCodebase(_)
        | AIAgentActionResultType::Grep(_)
        | AIAgentActionResultType::FileGlob(_)
        | AIAgentActionResultType::FileGlobV2(_)
        | AIAgentActionResultType::ReadMCPResource(_)
        | AIAgentActionResultType::CallMCPTool(_)
        | AIAgentActionResultType::ReadSkill(_)
        | AIAgentActionResultType::SuggestNewConversation(_)
        | AIAgentActionResultType::SuggestPrompt(_)
        | AIAgentActionResultType::OpenCodeReview
        | AIAgentActionResultType::InitProject
        | AIAgentActionResultType::ReadDocuments(_)
        | AIAgentActionResultType::EditDocuments(_)
        | AIAgentActionResultType::CreateDocuments(_)
        | AIAgentActionResultType::UseComputer(_)
        | AIAgentActionResultType::InsertReviewComments(_)
        | AIAgentActionResultType::RequestComputerUse(_)
        | AIAgentActionResultType::StartRecording(_)
        | AIAgentActionResultType::StopRecording(_)
        | AIAgentActionResultType::FetchConversation(_)
        | AIAgentActionResultType::SendMessageToAgent(_)
        | AIAgentActionResultType::AskUserQuestion(_)
        | AIAgentActionResultType::RunAgents(_)
        | AIAgentActionResultType::WaitForEvents(_) => None,
    }
}

/// An MCP tool call, before or after it ran.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct McpToolCall<'a> {
    pub server_id: Option<Uuid>,
    pub name: &'a str,
    pub input: &'a serde_json::Value,
    /// Present on the result boundaries.
    pub outcome: Option<McpCallOutcome<'a>>,
}

/// How an MCP tool call that ran ended. A cancelled call never ran, so the MCP events do not see
/// it and there is no variant for it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum McpCallOutcome<'a> {
    Returned(&'a CallToolResult),
    Failed(&'a str),
}

pub(super) fn mcp_tool_call(payload: &HookPayload) -> Option<McpToolCall<'_>> {
    let (action, result) = match payload {
        HookPayload::ToolPre { action, .. } => (action, None),
        HookPayload::ToolResult { action, result, .. }
        | HookPayload::ToolFailure { action, result, .. } => (action, Some(result)),
        HookPayload::SessionStart { .. }
        | HookPayload::PromptSubmit { .. }
        | HookPayload::TurnStop { .. }
        | HookPayload::ResponseComplete { .. }
        | HookPayload::ReasoningComplete { .. } => return None,
    };
    let AIAgentActionType::CallMCPTool {
        server_id,
        name,
        input,
    } = action
    else {
        return None;
    };
    let outcome = match result {
        None => None,
        Some(result) => {
            let AIAgentActionResultType::CallMCPTool(result) = result else {
                return None;
            };
            match result {
                CallMCPToolResult::Success { result } => Some(McpCallOutcome::Returned(result)),
                CallMCPToolResult::Error(message) => Some(McpCallOutcome::Failed(message)),
                CallMCPToolResult::Cancelled => return None,
            }
        }
    };
    Some(McpToolCall {
        server_id: *server_id,
        name,
        input,
        outcome,
    })
}
