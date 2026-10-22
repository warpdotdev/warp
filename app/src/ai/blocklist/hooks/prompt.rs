use ai::hooks::PromptProvenance;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_prompt_submit};
use super::{HookRuntime, owns_turn_lifecycle};
use crate::ai::agent::conversation::AIConversationId;
use crate::terminal::model::session::active_session::ActiveSession;

/// Fires `prompt.submit` for a prompt this client is about to send. Only conversations whose turn
/// lifecycle this client owns fire hooks; mirrored and external conversations do not.
pub(crate) fn notify_prompt_submit(
    text: &str,
    attachments_summary: Vec<String>,
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
    let envelope = envelope_for_prompt_submit(
        text.to_owned(),
        attachments_summary,
        PromptProvenance::User,
        &site,
        ctx,
    );
    HookRuntime::as_ref(ctx).dispatch_detached(envelope, ctx);
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
