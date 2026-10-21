//! The completion funnel for tool boundaries. Every terminal result of an action passes through
//! here once. A tool that ran and returned is a `tool.result`, whatever it returned; a tool that
//! never ran (denied, cancelled, or stopped before it could start) is a `tool.failure`. The
//! result is never held; the dispatch runs detached.
//!
//! A command the agent runs in interact mode first produces a snapshot of the still-running
//! command, and its real exit is seen later by whichever poll, write, or handoff happens to be
//! waiting on it. The funnel remembers the command when the snapshot passes and, when the exit
//! is seen, reports it once more as a completed result of the command's own action, so a hook
//! watching for finished shell commands sees every command exactly once, with its exit code.

use std::collections::HashMap;
use std::time::Duration;

use ai::agent::action_result::{
    ReadShellCommandOutputResult, RequestCommandOutputResult,
    TransferShellCommandControlToUserResult, WriteToLongRunningShellCommandResult,
};
use ai::hooks::{HookEnvelope, HookPayload};
use chrono::{DateTime, Local};
use warp_core::command::ExitCode;
use warp_terminal::model::BlockId;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::denied_result::DENIED_BY_HOOK_PREFIX;
use super::envelope::{EnvelopeSite, envelope_for_tool_failure, envelope_for_tool_result};
use super::{HookRuntime, executes_tool_calls_locally};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionResultType, AIAgentActionType, CancellationReason,
    RunAgentsResult,
};
use crate::terminal::model::session::active_session::ActiveSession;

/// What the funnel recorded about one terminal result, kept so tests can count calls per path.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedTerminalResult {
    pub action_id: crate::ai::agent::AIAgentActionId,
    pub boundary: ai::hooks::HookBoundary,
}

/// A long-running command whose snapshot was reported and whose exit has not been seen yet.
struct PendingShellCompletion {
    conversation_id: AIConversationId,
    action: AIAgentAction,
}

/// Long-running commands waiting for their real exit, by the block they run in.
#[derive(Default)]
pub(crate) struct ShellCompletions {
    pending: HashMap<BlockId, PendingShellCompletion>,
}

impl ShellCompletions {
    pub(super) fn forget(&mut self, conversation_id: AIConversationId) {
        self.pending
            .retain(|_, pending| pending.conversation_id != conversation_id);
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// Fires `tool.result` or `tool.failure` for a terminal result. Only conversations whose tool
/// calls this client runs fire hooks; a result mirrored from another client does not.
pub(crate) fn observe_terminal_result(
    action: &AIAgentAction,
    result: &AIAgentActionResultType,
    cancellation_reason: Option<CancellationReason>,
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_active(ctx) || !executes_tool_calls_locally(conversation_id, ctx) {
        return;
    }
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = if ran_and_returned(result) && cancellation_reason.is_none() {
        envelope_for_tool_result(action, result, &site, ctx)
    } else {
        envelope_for_tool_failure(action, result, cancellation_reason, &site, ctx)
    };
    #[cfg(test)]
    record_observation(action, &envelope, ctx);
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);

    match shell_progress(result) {
        ShellProgress::Snapshot { block_id } => {
            HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.shell_completions.pending.insert(
                    block_id,
                    PendingShellCompletion {
                        conversation_id,
                        action: action.clone(),
                    },
                );
            });
        }
        ShellProgress::Finished(finished) => {
            let pending = HookRuntime::handle(ctx).update(ctx, |runtime, _| {
                runtime.shell_completions.pending.remove(&finished.block_id)
            });
            if let Some(pending) = pending {
                report_shell_completion(pending, finished, active_session, ctx);
            }
        }
        ShellProgress::None => {}
    }
}

/// Fires the `tool.result` for the real exit of a long-running command, on the command's own
/// action and in the command's own conversation.
fn report_shell_completion(
    pending: PendingShellCompletion,
    finished: FinishedCommand,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    let AIAgentActionType::RequestCommandOutput { command, .. } = &pending.action.action else {
        return;
    };
    let duration = finished
        .start_ts
        .zip(finished.completed_ts)
        .and_then(|(start, end)| end.signed_duration_since(start).to_std().ok());
    let result = AIAgentActionResultType::RequestCommandOutput(RequestCommandOutputResult::Completed {
        block_id: finished.block_id,
        command: command.clone(),
        output: finished.output,
        exit_code: finished.exit_code,
        start_ts: finished.start_ts,
        completed_ts: finished.completed_ts,
    });
    let site = EnvelopeSite::new(pending.conversation_id, active_session);
    let envelope = envelope_for_tool_result_with_duration(
        &pending.action,
        &result,
        duration,
        &site,
        ctx,
    );
    #[cfg(test)]
    record_observation(&pending.action, &envelope, ctx);
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
}

fn envelope_for_tool_result_with_duration(
    action: &AIAgentAction,
    result: &AIAgentActionResultType,
    duration: Option<Duration>,
    site: &EnvelopeSite<'_>,
    ctx: &AppContext,
) -> HookEnvelope {
    let mut envelope = envelope_for_tool_result(action, result, site, ctx);
    if let HookPayload::ToolResult {
        duration: slot, ..
    } = &mut envelope.payload
    {
        *slot = duration;
    }
    envelope
}

/// A long-running command's exit as one of the shell actions reported it.
struct FinishedCommand {
    block_id: BlockId,
    output: String,
    exit_code: ExitCode,
    start_ts: Option<DateTime<Local>>,
    completed_ts: Option<DateTime<Local>>,
}

/// What a result says about a long-running command.
enum ShellProgress {
    /// The command is still running; its exit will be reported later.
    Snapshot { block_id: BlockId },
    /// The command exited.
    Finished(FinishedCommand),
    None,
}

fn shell_progress(result: &AIAgentActionResultType) -> ShellProgress {
    match result {
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::LongRunningCommandSnapshot { block_id, .. },
        ) => ShellProgress::Snapshot {
            block_id: block_id.clone(),
        },
        AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
                ..
            },
        )
        | AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            },
        )
        | AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            },
        ) => ShellProgress::Finished(FinishedCommand {
            block_id: block_id.clone(),
            output: output.clone(),
            exit_code: *exit_code,
            start_ts: *start_ts,
            completed_ts: *completed_ts,
        }),
        // The shell itself ended under the command; the command is over, with whatever exit the
        // shell reported.
        AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::ShellRecovered {
                block_id,
                output,
                status,
                start_ts,
                completed_ts,
                ..
            },
        ) => ShellProgress::Finished(FinishedCommand {
            block_id: block_id.clone(),
            output: output.clone(),
            exit_code: ExitCode::from(status.exit_code()),
            start_ts: *start_ts,
            completed_ts: *completed_ts,
        }),
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::Completed { .. }
            | RequestCommandOutputResult::ShellRecovered { .. }
            | RequestCommandOutputResult::CancelledBeforeExecution
            | RequestCommandOutputResult::Denylisted { .. }
            | RequestCommandOutputResult::HookDenied { .. }
            | RequestCommandOutputResult::TerminalBusy { .. },
        )
        | AIAgentActionResultType::ReadShellCommandOutput(
            ReadShellCommandOutputResult::LongRunningCommandSnapshot { .. }
            | ReadShellCommandOutputResult::Cancelled
            | ReadShellCommandOutputResult::Error(_),
        )
        | AIAgentActionResultType::WriteToLongRunningShellCommand(
            WriteToLongRunningShellCommandResult::Snapshot { .. }
            | WriteToLongRunningShellCommandResult::Cancelled
            | WriteToLongRunningShellCommandResult::Error(_),
        )
        | AIAgentActionResultType::TransferShellCommandControlToUser(
            TransferShellCommandControlToUserResult::Snapshot { .. }
            | TransferShellCommandControlToUserResult::Cancelled
            | TransferShellCommandControlToUserResult::Error(_),
        )
        | AIAgentActionResultType::RequestFileEdits(_)
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
        | AIAgentActionResultType::WaitForEvents(_) => ShellProgress::None,
    }
}

/// Whether the tool ran and this is what it returned. A failing exit code, an error the tool
/// reported, or a shell that ended under the command are all answers from a tool that ran. A
/// cancelled result, a result Warp produced because the tool was not allowed to start, and a
/// reason an agent hook wrote are not.
pub(crate) fn ran_and_returned(result: &AIAgentActionResultType) -> bool {
    if result.is_cancelled() && !result.is_cancelled_during_requested_command_execution() {
        return false;
    }
    match result {
        AIAgentActionResultType::RequestCommandOutput(result) => match result {
            RequestCommandOutputResult::Completed { .. }
            | RequestCommandOutputResult::LongRunningCommandSnapshot { .. }
            | RequestCommandOutputResult::ShellRecovered { .. } => true,
            RequestCommandOutputResult::CancelledBeforeExecution
            | RequestCommandOutputResult::Denylisted { .. }
            | RequestCommandOutputResult::HookDenied { .. }
            | RequestCommandOutputResult::TerminalBusy { .. } => false,
        },
        AIAgentActionResultType::RunAgents(RunAgentsResult::Denied { .. }) => false,
        AIAgentActionResultType::RunAgents(
            RunAgentsResult::Launched { .. }
            | RunAgentsResult::Failure { .. }
            | RunAgentsResult::Cancelled,
        ) => true,
        AIAgentActionResultType::ReadShellCommandOutput(_)
        | AIAgentActionResultType::WriteToLongRunningShellCommand(_)
        | AIAgentActionResultType::TransferShellCommandControlToUser(_)
        | AIAgentActionResultType::RequestFileEdits(_)
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
        | AIAgentActionResultType::WaitForEvents(_) => !is_hook_denial(result),
    }
}

/// Whether a failed result is the one an agent hook's denial finishes the action with. The
/// families without a result made for this carry the reason in their error text, behind a fixed
/// prefix. Only a failed result is rendered to look for it; a success can be large.
fn is_hook_denial(result: &AIAgentActionResultType) -> bool {
    result.is_failed() && result.to_string().contains(DENIED_BY_HOOK_PREFIX)
}

#[cfg(test)]
fn record_observation(action: &AIAgentAction, envelope: &HookEnvelope, ctx: &mut AppContext) {
    let observed = ObservedTerminalResult {
        action_id: action.id.clone(),
        boundary: envelope.boundary(),
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.observed_terminal_results.push(observed);
    });
}

#[cfg(test)]
pub(crate) fn observed_terminal_results(ctx: &AppContext) -> Vec<ObservedTerminalResult> {
    HookRuntime::as_ref(ctx).observed_terminal_results.clone()
}

#[cfg(test)]
#[path = "tool_result_tests.rs"]
mod tests;
