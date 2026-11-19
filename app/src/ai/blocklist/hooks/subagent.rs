//! `subagent.start`, fired once per child of a `run_agents` call before any of its children are
//! launched. A hook decides per child: a child it denies is left out of the launch and reported
//! to the model as failed with the hook's reason, while the other children launch as usual.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ai::hooks::{
    DispatchOutcome, HookBoundary, HookNotice, HookNoticeKind, HookPermissionPolicy,
    PermissionDecision, PermissionFloors, RunFacts, SubagentLaunch, WarpPermissionResult,
};
use futures::FutureExt;
use futures::future::{BoxFuture, join_all};
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_subagent_start};
use super::{HookRuntime, report_denial};
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::model::session::active_session::ActiveSession;

/// Reason a child is kept back with when a hook asked for a confirmation nobody can give.
pub(crate) const ASK_UNAVAILABLE_REASON: &str = "An agent hook asked for confirmation before launching this child agent, which is not \
     available for child agents, so it was not launched.";

/// Reason a child is kept back with when its hooks were cancelled before they answered.
pub(crate) const CANCELLED_REASON: &str = "The agent hooks for this child agent were cancelled before they answered, so it was not \
     launched.";

/// What the hooks decided for one child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChildDecision {
    Launch,
    /// The child is not launched; `reason` is what the model reads for it.
    Denied {
        reason: String,
    },
}

/// The hooks running for the children of one `run_agents` call.
pub(crate) struct PendingChildren {
    /// Stops the hooks; whatever they answer afterwards is not a decision.
    pub cancel: Arc<AtomicBool>,
    /// One outcome per child, in the order the children were given.
    pub outcomes: BoxFuture<'static, Vec<DispatchOutcome>>,
}

/// Starts the `subagent.start` hooks for every child in `children`, all at once. `None` means no
/// such hook is active, so every child may launch now.
pub(crate) fn begin(
    children: &[SubagentLaunch],
    action_id: &AIAgentActionId,
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &AppContext,
) -> Option<PendingChildren> {
    if !HookRuntime::is_active(ctx) {
        return None;
    }
    let runtime = HookRuntime::as_ref(ctx);
    let snapshot = runtime.snapshot(ctx);
    if !snapshot.has_bindings_for(HookBoundary::SubagentStart) {
        return None;
    }
    let site = EnvelopeSite::new(conversation_id, active_session);
    let is_headless = runtime.is_headless(ctx);
    let dispatcher = runtime.dispatcher();
    let cancel = Arc::new(AtomicBool::new(false));
    let dispatches = children
        .iter()
        .map(|launch| {
            let envelope = envelope_for_subagent_start(launch.clone(), action_id, &site, ctx);
            dispatcher.dispatch(envelope, snapshot.clone(), is_headless, cancel.clone())
        })
        .collect::<Vec<_>>();
    Some(PendingChildren {
        cancel,
        outcomes: join_all(dispatches).boxed(),
    })
}

/// Applies the permission policy to each child's outcome, in order. A child the hooks denied, or
/// asked a confirmation for that nobody can give, is kept back and the person is told why; the
/// run's owner is told too when a hook that asked came from a source that ends the run on that.
/// The call's own permission check already passed, so a hook can only keep a child back.
pub(crate) fn decide(
    outcomes: Vec<DispatchOutcome>,
    action_id: &AIAgentActionId,
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> Vec<ChildDecision> {
    let run = RunFacts {
        is_headless: HookRuntime::as_ref(ctx).is_headless(ctx),
    };
    let warp = WarpPermissionResult {
        can_auto_execute: true,
        is_user_initiated: false,
        is_autonomous_shell_exception: false,
    };
    outcomes
        .into_iter()
        .map(|outcome| {
            if outcome.cancelled {
                return ChildDecision::Denied {
                    reason: CANCELLED_REASON.to_owned(),
                };
            }
            match HookPermissionPolicy::compose(
                &outcome.effects,
                warp,
                &PermissionFloors::default(),
                run,
            ) {
                PermissionDecision::Execute => ChildDecision::Launch,
                PermissionDecision::Deny { reason, cause } => {
                    report_denial(
                        conversation_id,
                        action_id,
                        &reason,
                        cause,
                        &outcome.effects,
                        ctx,
                    );
                    ChildDecision::Denied {
                        reason: reason.text,
                    }
                }
                // No confirmation step exists for a child launch, so an ask that would reach a
                // person on another boundary keeps the child back here.
                PermissionDecision::RequireConfirmation { reason } => {
                    let reason = match reason {
                        Some(reason) => format!("{ASK_UNAVAILABLE_REASON} ({})", reason.text),
                        None => ASK_UNAVAILABLE_REASON.to_owned(),
                    };
                    HookRuntime::as_ref(ctx).messenger().notify(HookNotice {
                        conversation_id,
                        action_id: Some(action_id.clone()),
                        kind: HookNoticeKind::Denied,
                        text: reason.clone(),
                    });
                    ChildDecision::Denied { reason }
                }
            }
        })
        .collect()
}
