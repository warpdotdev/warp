use ai::hooks::TurnStopStatus;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_turn_stop};
use super::{HookRuntime, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::model::session::active_session::ActiveSession;

/// Fires `turn.stop` for a turn this client just finished. Continuations are not counted yet.
pub(crate) fn notify_turn_stop(
    status: TurnStopStatus,
    conversation_id: AIConversationId,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_active(ctx) {
        return;
    }
    if !owns_turn_lifecycle(conversation_id, ctx) {
        return;
    }
    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = envelope_for_turn_stop(status, 0, &site, ctx);
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
