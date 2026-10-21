use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ai::hooks::{DispatchOutcome, HookBoundary, HookExecutionHost, MergedEffects};
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::HookRuntime;
use super::envelope::{EnvelopeSite, envelope_for_tool_pre};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId};
use crate::features::FeatureFlag;
use crate::terminal::model::session::active_session::ActiveSession;

/// What a cached verdict is keyed by. Action ids are only unique within a conversation, and the
/// cache is shared by every conversation in the process. Argument revision, working directory,
/// and registry revision join the key when verdict invalidation is widened.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VerdictKey {
    pub conversation_id: AIConversationId,
    pub action_id: AIAgentActionId,
}

#[derive(Debug, Clone)]
pub(crate) enum PreToolHookState {
    Pending,
    Resolved(Box<MergedEffects>),
}

#[derive(Debug)]
struct Entry {
    state: PreToolHookState,
    /// Set once any caller asked for this action with user approval, so the resumed attempt
    /// keeps that approval.
    is_user_initiated: bool,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
pub(crate) struct PreToolVerdictCache {
    entries: HashMap<VerdictKey, Entry>,
}

/// Names one dispatch of one action. A later dispatch for the same action gets a new ticket, so
/// a verdict arriving from an earlier dispatch is recognized as stale and dropped.
#[derive(Debug, Clone)]
pub(crate) struct DispatchTicket {
    pub conversation_id: AIConversationId,
    pub action_id: AIAgentActionId,
    cancel: Arc<AtomicBool>,
}

impl DispatchTicket {
    fn key(&self) -> VerdictKey {
        VerdictKey {
            conversation_id: self.conversation_id,
            action_id: self.action_id.clone(),
        }
    }
}

/// A dispatch the caller must spawn. The verdict arrives through [`on_resolved`].
pub(crate) struct PendingDispatch {
    pub ticket: DispatchTicket,
    pub future: BoxFuture<'static, DispatchOutcome>,
}

pub(crate) enum PreToolLookup {
    /// Hooks are off, or no binding applies to this action. Proceed as if hooks did not exist.
    Inactive,
    /// A verdict is being computed. `dispatch` is present the first time an action is seen.
    Pending {
        dispatch: Option<PendingDispatch>,
    },
    Resolved(Box<MergedEffects>),
}

/// Everything the executor needs to resume an action once its verdict is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Resumption {
    pub conversation_id: AIConversationId,
    pub is_user_initiated: bool,
}

/// Sets the cancel flag if the dispatch future is dropped before it finishes, so a runner that
/// polls the flag stops even when nobody is left to wait for the verdict.
struct CancelOnDrop(Option<Arc<AtomicBool>>);

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.store(true, Ordering::Release);
        }
    }
}

/// Looks up the verdict for `action`, starting a dispatch if this is the first time it is seen.
pub(crate) fn begin_or_lookup(
    action: &AIAgentAction,
    conversation_id: AIConversationId,
    is_user_initiated: bool,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> PreToolLookup {
    if !HookRuntime::is_active(ctx) {
        return PreToolLookup::Inactive;
    }
    let key = VerdictKey {
        conversation_id,
        action_id: action.id.clone(),
    };
    let runtime = HookRuntime::handle(ctx);
    let cached = runtime.update(ctx, |runtime, _| {
        runtime.pre_tool.entries.get_mut(&key).map(|entry| {
            entry.is_user_initiated |= is_user_initiated;
            entry.state.clone()
        })
    });
    match cached {
        Some(PreToolHookState::Pending) => return PreToolLookup::Pending { dispatch: None },
        Some(PreToolHookState::Resolved(merged)) => return PreToolLookup::Resolved(merged),
        None => {}
    }

    // Building the envelope clones the action and reads the history model, so make sure some
    // pre-tool binding exists before paying for that.
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::ToolPre) {
        return PreToolLookup::Inactive;
    }
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = envelope_for_tool_pre(action, &site, ctx);
    let has_matching_binding = snapshot
        .bindings_for(
            HookBoundary::ToolPre,
            envelope.cwd.as_deref(),
            HookExecutionHost::Local,
        )
        .iter()
        .any(|active| active.adapter.matches(&active.binding, &envelope));
    if !has_matching_binding {
        return PreToolLookup::Inactive;
    }

    let ticket = DispatchTicket {
        conversation_id,
        action_id: action.id.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let dispatch = runtime.update(ctx, |runtime, _| {
        runtime.pre_tool.entries.insert(
            key,
            Entry {
                state: PreToolHookState::Pending,
                is_user_initiated,
                cancel: ticket.cancel.clone(),
            },
        );
        runtime
            .dispatcher()
            .dispatch(envelope, snapshot, ticket.cancel.clone())
    });
    let mut guard = CancelOnDrop(Some(ticket.cancel.clone()));
    let future = async move {
        let outcome = dispatch.await;
        guard.disarm();
        outcome
    }
    .boxed();
    PreToolLookup::Pending {
        dispatch: Some(PendingDispatch { ticket, future }),
    }
}

/// Stores a finished dispatch's verdict. Returns `None`, and drops the verdict, when the dispatch
/// was cancelled or replaced while it ran.
pub(crate) fn on_resolved(
    ticket: &DispatchTicket,
    outcome: DispatchOutcome,
    ctx: &mut AppContext,
) -> Option<Resumption> {
    if ticket.cancel.load(Ordering::Acquire) || !ctx.has_singleton_model::<HookRuntime>() {
        return None;
    }
    let key = ticket.key();
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        let entry = runtime.pre_tool.entries.get_mut(&key)?;
        if !Arc::ptr_eq(&entry.cancel, &ticket.cancel) {
            return None;
        }
        entry.state = PreToolHookState::Resolved(Box::new(outcome.effects));
        Some(Resumption {
            conversation_id: ticket.conversation_id,
            is_user_initiated: entry.is_user_initiated,
        })
    })
}

/// Forgets whatever is known about `action_id` in `conversation_id`. An in-flight dispatch is
/// told to stop and its verdict is dropped when it arrives.
pub(crate) fn invalidate(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &mut AppContext,
) {
    if !FeatureFlag::AgentHooks.is_enabled() || !ctx.has_singleton_model::<HookRuntime>() {
        return;
    }
    let key = VerdictKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        if let Some(entry) = runtime.pre_tool.entries.remove(&key) {
            entry.cancel.store(true, Ordering::Release);
        }
    });
}

/// Forgets every action of `conversation_id` and stops its in-flight dispatches.
pub(crate) fn cancel_all_for_conversation(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !FeatureFlag::AgentHooks.is_enabled() || !ctx.has_singleton_model::<HookRuntime>() {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.pre_tool.entries.retain(|key, entry| {
            if key.conversation_id == conversation_id {
                entry.cancel.store(true, Ordering::Release);
                false
            } else {
                true
            }
        });
    });
}

#[cfg(test)]
pub(crate) fn cached_state(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<PreToolHookState> {
    let key = VerdictKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::as_ref(ctx)
        .pre_tool
        .entries
        .get(&key)
        .map(|entry| entry.state.clone())
}

#[cfg(test)]
pub(crate) fn cancel_flag(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<Arc<AtomicBool>> {
    let key = VerdictKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::as_ref(ctx)
        .pre_tool
        .entries
        .get(&key)
        .map(|entry| entry.cancel.clone())
}

#[cfg(test)]
#[path = "pre_tool_tests.rs"]
mod tests;
