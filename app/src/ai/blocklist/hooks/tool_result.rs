use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::HookRuntime;
use super::envelope::{EnvelopeSite, envelope_for_tool_failure, envelope_for_tool_result};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionResultType, CancellationReason};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::terminal::model::session::active_session::ActiveSession;

/// What the funnel recorded about one terminal result, kept so tests can count calls per path.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedTerminalResult {
    pub action_id: crate::ai::agent::AIAgentActionId,
    pub boundary: ai::hooks::HookBoundary,
}

/// Fires `tool.failure` for failed or cancelled results and `tool.result` for everything else.
/// The result is never held; the dispatch runs detached. Only conversations whose tool calls
/// this client runs fire hooks; a result mirrored from another client does not.
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
    let is_failure = result.is_failed() || result.is_cancelled() || cancellation_reason.is_some();
    let envelope = if is_failure {
        envelope_for_tool_failure(action, result, cancellation_reason, &site, ctx)
    } else {
        envelope_for_tool_result(action, result, &site, ctx)
    };
    #[cfg(test)]
    {
        let observed = ObservedTerminalResult {
            action_id: action.id.clone(),
            boundary: envelope.boundary(),
        };
        HookRuntime::handle(ctx).update(ctx, |runtime, _| {
            runtime.observed_terminal_results.push(observed);
        });
    }
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
}

/// Whether this client runs `conversation_id`'s tool calls. Unknown conversations do not.
fn executes_tool_calls_locally(conversation_id: AIConversationId, ctx: &AppContext) -> bool {
    ctx.has_singleton_model::<BlocklistAIHistoryModel>()
        && BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.driver().executes_tool_calls_locally())
}

#[cfg(test)]
pub(crate) fn observed_terminal_results(ctx: &AppContext) -> Vec<ObservedTerminalResult> {
    HookRuntime::as_ref(ctx).observed_terminal_results.clone()
}

#[cfg(test)]
#[path = "tool_result_tests.rs"]
mod tests;
