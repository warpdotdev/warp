use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ai::hooks::{
    DenyCause, DispatchOutcome, HookBoundary, HookExecutionHost, HookNotice, HookNoticeKind,
    HookPermissionPolicy, HookReason, HookVerdict, MergedEffects, PermissionDecision,
    PermissionFloors, RunFacts, WarpPermissionResult,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_tool_pre};
use super::{HookRuntime, HookRuntimeEvent};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::terminal::model::session::active_session::ActiveSession;

/// How many times one action's hook check may start over because the facts it was keyed on
/// changed underneath it. A hook configuration that keeps changing (a file written in a loop)
/// would otherwise re-run the hooks forever; past this many restarts the action is denied.
const MAX_RESTARTS_PER_ACTION: u32 = 3;

/// Reason an action is denied with when its hook check restarted too many times.
pub(crate) const CONFIGURATION_UNSTABLE_DENY_REASON: &str = "The hook configuration kept changing while this action waited for its hooks, so the \
     action was not run.";

/// What a hook's `ask` does on a surface with no one to ask.
// Only the run's launcher, which is later work, selects anything but the default.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HeadlessAskBehavior {
    /// The action is denied with a reason that tells the model approval was unavailable.
    #[default]
    DenyWithReason,
    /// The action is denied the same way, and the runtime also announces the refusal so the
    /// run's owner can end the run.
    FailRun,
}

/// What a cached verdict is keyed by: the action within its conversation (tool call ids are
/// only unique within a conversation, and the cache is shared by every conversation in the
/// process), its arguments, the working directory the operation would run in, and the hook
/// configuration it was judged under. A change to any of them makes the verdict describe a
/// different operation, so the hooks run again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VerdictKey {
    pub conversation_id: AIConversationId,
    pub action_id: AIAgentActionId,
    /// A hash of the action's arguments as they were when the hooks ran.
    pub argument_revision: u64,
    pub cwd: Option<PathBuf>,
    pub registry_revision: u64,
}

impl VerdictKey {
    fn action_key(&self) -> ActionKey {
        ActionKey {
            conversation_id: self.conversation_id,
            action_id: self.action_id.clone(),
        }
    }
}

/// The part of a [`VerdictKey`] that names the action. One entry is kept per action; the rest
/// of the key says which operation the entry's verdict describes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActionKey {
    conversation_id: AIConversationId,
    action_id: AIAgentActionId,
}

/// A stable digest of an action's arguments. The derived `Debug` form covers every field, so a
/// change the hooks should see again changes the digest; a field that prints differently for
/// the same value can only cause an extra hook run, never a stale verdict.
pub(crate) fn argument_revision(action: &AIAgentActionType) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{action:?}").hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone)]
pub(crate) enum PreToolHookState {
    Pending,
    Resolved(Box<MergedEffects>),
}

#[derive(Debug)]
struct VerdictEntry {
    key: VerdictKey,
    state: PreToolHookState,
    /// Set once any caller asked for this action with user approval, so the resumed attempt
    /// keeps that approval.
    is_user_initiated: bool,
    cancel: Arc<AtomicBool>,
    /// How many times this action's hook check has started over.
    restarts: u32,
}

#[derive(Debug, Default)]
pub(crate) struct PreToolVerdictCache {
    entries: HashMap<ActionKey, VerdictEntry>,
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
    fn action_key(&self) -> ActionKey {
        ActionKey {
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

/// What the cache held for an action when a lookup arrived.
enum Cached {
    /// The entry describes the operation being looked up.
    Current(PreToolHookState),
    /// The entry described an earlier revision of the operation; it was dropped and its
    /// dispatch told to stop.
    Outdated {
        restarts: u32,
        is_user_initiated: bool,
    },
    Nothing,
}

/// Looks up the verdict for `action`, starting a dispatch if this is the first time the action
/// is seen in its current form.
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
    let runtime = HookRuntime::handle(ctx);
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    let key = VerdictKey {
        conversation_id,
        action_id: action.id.clone(),
        argument_revision: argument_revision(&action.action),
        cwd: active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .map(PathBuf::from),
        registry_revision: snapshot.revision,
    };
    let action_key = key.action_key();
    let cached = runtime.update(ctx, |runtime, _| {
        let entries = &mut runtime.pre_tool_verdicts.entries;
        match entries.get_mut(&action_key) {
            Some(entry) if entry.key == key => {
                entry.is_user_initiated |= is_user_initiated;
                Cached::Current(entry.state.clone())
            }
            Some(_) => {
                let entry = entries
                    .remove(&action_key)
                    .expect("the entry was just found");
                entry.cancel.store(true, Ordering::Release);
                log::info!(
                    "hook verdict for {} no longer describes the action; running its hooks \
                     again (restart {})",
                    entry.key.action_id,
                    entry.restarts + 1
                );
                Cached::Outdated {
                    restarts: entry.restarts + 1,
                    // Approval covers the arguments the user saw. It carries over when only the
                    // working directory or the configuration moved, never to different
                    // arguments.
                    is_user_initiated: entry.is_user_initiated
                        && entry.key.argument_revision == key.argument_revision,
                }
            }
            None => Cached::Nothing,
        }
    });
    let (restarts, is_user_initiated) = match cached {
        Cached::Current(PreToolHookState::Pending) => {
            return PreToolLookup::Pending { dispatch: None };
        }
        Cached::Current(PreToolHookState::Resolved(merged)) => {
            return PreToolLookup::Resolved(merged);
        }
        Cached::Outdated {
            restarts,
            is_user_initiated: earlier_approval,
        } => (restarts, is_user_initiated || earlier_approval),
        Cached::Nothing => (0, is_user_initiated),
    };
    if restarts > MAX_RESTARTS_PER_ACTION {
        let denied = Box::new(unstable_configuration_denial());
        runtime.update(ctx, |runtime, _| {
            runtime.pre_tool_verdicts.entries.insert(
                action_key,
                VerdictEntry {
                    key,
                    state: PreToolHookState::Resolved(denied.clone()),
                    is_user_initiated,
                    cancel: Arc::new(AtomicBool::new(false)),
                    restarts,
                },
            );
        });
        return PreToolLookup::Resolved(denied);
    }

    // Building the envelope clones the action and reads the history model, so make sure some
    // pre-tool binding exists before paying for that.
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
        runtime.pre_tool_verdicts.entries.insert(
            action_key,
            VerdictEntry {
                key,
                state: PreToolHookState::Pending,
                is_user_initiated,
                cancel: ticket.cancel.clone(),
                restarts,
            },
        );
        runtime.dispatcher().dispatch(
            envelope,
            snapshot,
            runtime.dispatch_policy(),
            ticket.cancel.clone(),
        )
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

fn unstable_configuration_denial() -> MergedEffects {
    MergedEffects {
        verdict: HookVerdict::Deny,
        reasons: vec![HookReason {
            text: CONFIGURATION_UNSTABLE_DENY_REASON.to_owned(),
            binding: None,
        }],
        ..Default::default()
    }
}

/// Stores a finished dispatch's verdict. Returns `None`, and drops the verdict, when the dispatch
/// was cancelled or replaced while it ran. A verdict judged under a hook configuration that has
/// since changed is dropped too, but the action is still resumed: its next lookup sees the
/// entry's key no longer matches and runs the hooks again against the new configuration.
pub(crate) fn on_resolved(
    ticket: &DispatchTicket,
    outcome: DispatchOutcome,
    ctx: &mut AppContext,
) -> Option<Resumption> {
    if ticket.cancel.load(Ordering::Acquire) || !HookRuntime::is_installed(ctx) {
        return None;
    }
    let action_key = ticket.action_key();
    let current_revision = HookRuntime::as_ref(ctx).snapshot(ctx).revision;
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        let entry = runtime.pre_tool_verdicts.entries.get_mut(&action_key)?;
        if !Arc::ptr_eq(&entry.cancel, &ticket.cancel) {
            return None;
        }
        if outcome.registry_revision != current_revision {
            log::info!(
                "hook verdict for {} was judged under hook configuration revision {} but the \
                 configuration is now at revision {current_revision}; it was dropped",
                ticket.action_id,
                outcome.registry_revision
            );
        } else {
            entry.state = PreToolHookState::Resolved(Box::new(outcome.effects));
        }
        Some(Resumption {
            conversation_id: ticket.conversation_id,
            is_user_initiated: entry.is_user_initiated,
        })
    })
}

/// Applies the permission policy to a resolved verdict and reports what it decided. A denial is
/// delivered to the user as a notice; a denial because nobody could answer a hook's `ask` is
/// also announced to the run's owner when the run asked to end on it.
pub(crate) fn decide(
    merged: &MergedEffects,
    warp: WarpPermissionResult,
    floors: PermissionFloors,
    action: &AIAgentAction,
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> PermissionDecision {
    let runtime = HookRuntime::as_ref(ctx);
    let run = RunFacts {
        headless: runtime.is_headless_for(conversation_id, ctx),
        is_strict: runtime.strict_policy().enabled,
    };
    let headless_ask_behavior = runtime.headless_ask_behavior();
    let messenger = runtime.messenger();

    let decision = HookPermissionPolicy::compose(merged, warp, &floors, run);
    if let PermissionDecision::Deny { reason, cause } = &decision {
        messenger.notify(HookNotice {
            conversation_id,
            action_id: Some(action.id.clone()),
            kind: HookNoticeKind::Denied,
            text: reason.text.clone(),
        });
        let ends_the_run = match (cause, headless_ask_behavior) {
            (DenyCause::ApprovalUnavailable, HeadlessAskBehavior::FailRun) => true,
            (DenyCause::ApprovalUnavailable, HeadlessAskBehavior::DenyWithReason)
            | (
                DenyCause::Hook | DenyCause::RewriteUnsupported | DenyCause::DeferralUnsupported,
                HeadlessAskBehavior::DenyWithReason | HeadlessAskBehavior::FailRun,
            ) => false,
        };
        if ends_the_run {
            let reason = reason.text.clone();
            let action_id = action.id.clone();
            HookRuntime::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(HookRuntimeEvent::ApprovalUnavailable {
                    conversation_id,
                    action_id,
                    reason,
                });
            });
        }
    }
    decision
}

/// Forgets whatever is known about `action_id` in `conversation_id`. An in-flight dispatch is
/// told to stop and its verdict is dropped when it arrives.
pub(crate) fn invalidate(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    let action_key = ActionKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        if let Some(entry) = runtime.pre_tool_verdicts.entries.remove(&action_key) {
            entry.cancel.store(true, Ordering::Release);
        }
    });
}

/// Forgets every action of `conversation_id` and stops its in-flight dispatches.
pub(crate) fn cancel_all_for_conversation(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.pre_tool_verdicts.entries.retain(|key, entry| {
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
fn entry<'a>(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &'a AppContext,
) -> Option<&'a VerdictEntry> {
    HookRuntime::as_ref(ctx)
        .pre_tool_verdicts
        .entries
        .get(&ActionKey {
            conversation_id,
            action_id: action_id.clone(),
        })
}

#[cfg(test)]
pub(crate) fn cached_state(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<PreToolHookState> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.state.clone())
}

#[cfg(test)]
pub(crate) fn cached_key(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<VerdictKey> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.key.clone())
}

#[cfg(test)]
pub(crate) fn cancel_flag(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<Arc<AtomicBool>> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.cancel.clone())
}

#[cfg(test)]
#[path = "pre_tool_tests.rs"]
mod tests;
