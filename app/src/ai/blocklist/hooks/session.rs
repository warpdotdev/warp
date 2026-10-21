use warp_core::execution_mode::AppExecutionMode;
use warpui::{AppContext, SingletonEntity};

use super::HookRuntime;
use super::envelope::{EnvelopeSite, envelope_for_session_start};
use crate::ai::agent::conversation::AIConversationId;

/// Fires `session.start` for a conversation this client just created and will drive. The
/// envelope has no working directory because conversation creation happens before any session
/// is attached to it.
pub(crate) fn notify_session_start(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !HookRuntime::is_active(ctx) {
        return;
    }
    let is_background_agent = ctx.has_singleton_model::<AppExecutionMode>()
        && AppExecutionMode::as_ref(ctx).is_autonomous();
    let site = EnvelopeSite::for_new_conversation(conversation_id);
    let envelope = envelope_for_session_start(is_background_agent, &site, ctx);
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
