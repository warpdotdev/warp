use std::collections::HashMap;

use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui::{ModelContext, SingletonEntity};

use super::BlocklistAIController;
use crate::ai::agent::AIAgentAttachment;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::attachment_utils::{build_file_attachment_map, download_task_file_attachments};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::{
    AutofireAction, BlocklistAIHistoryModel, QueuedPromptDeliveryMode, QueuedQuery, QueuedQueryId,
    QueuedQueryModel,
};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::BlockId;

impl BlocklistAIController {
    /// Binds this controller to a native conversation before session sharing can begin
    /// delivering startup follow-ups, so `route_native_startup_injection` knows which
    /// conversation to target for the rest of this run. Idempotent: returns the existing
    /// binding if one is already in place.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn bind_native_prompt_conversation(
        &mut self,
        restored_conversation_id: Option<AIConversationId>,
        ctx: &mut ModelContext<Self>,
    ) -> AIConversationId {
        if let Some(id) = self.native_prompt_conversation_id {
            return id;
        }
        let id = restored_conversation_id.unwrap_or_else(|| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.start_new_conversation(self.terminal_surface_id, false, false, false, ctx)
            })
        });
        self.native_prompt_conversation_id = Some(id);
        log::info!(
            "event=native_queue_initialized task_id={:?} terminal_id={:?} conversation_id={id} resumed={}",
            self.ambient_agent_task_id,
            self.terminal_surface_id,
            restored_conversation_id.is_some(),
        );
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.begin_native_setup(id, ctx);
            queue.set_delivery_mode(id, QueuedPromptDeliveryMode::Steering);
        });
        id
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn native_prompt_conversation_id(&self) -> Option<AIConversationId> {
        self.native_prompt_conversation_id
    }

    /// Unbinds this controller from its native conversation, dropping any prompts still queued
    /// for it (e.g. the run ended before setup finished, or before a dispatch that was deferred
    /// behind an active CLI subagent could go out) and releasing the native setup barrier if it
    /// was still held -- otherwise this conversation would stay permanently dispatch-blocked for
    /// any future local queueing against it, since nothing else would ever release that barrier.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn unbind_native_prompt_conversation(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(id) = self.native_prompt_conversation_id.take() else {
            return;
        };
        let unsent_count = QueuedQueryModel::as_ref(ctx).queue(id).len();
        log::info!(
            "event=native_queue_stopped task_id={:?} terminal_id={:?} conversation_id={id} unsent_count={unsent_count}",
            self.ambient_agent_task_id,
            self.terminal_surface_id,
        );
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.clear_queue(id, ctx);
            queue.finish_native_setup(id, ctx);
        });
    }

    /// Routes a shared-session-injected prompt while this controller is bound to a native
    /// conversation: always queues it (preserving FIFO order with anything already queued),
    /// then immediately attempts to dispatch the queue's head via
    /// [`Self::dispatch_queued_warp_agent_prompt`] when nothing is currently streaming for the
    /// conversation. When a stream *is* active, the row is left queued for the `Steering`
    /// dispatch mechanism to pick up at the next natural request boundary (or the existing
    /// idle-triggered drain once the turn finishes) -- dispatching immediately in that case
    /// would interrupt whatever's already in flight, dropping it before it produces any output.
    ///
    /// Returns `true` when the caller (`execute_warp_agent_prompt_from_shared_session_injection`
    /// and friends) must not fall through to legacy conversation resolution for this prompt:
    /// covers every case where this controller is bound to a native conversation, whether the
    /// prompt was queued, dispatched immediately, or rejected outright for naming a different
    /// conversation's token. Returns `false` only when this controller isn't bound to a native
    /// conversation at all, in which case the caller's own token/selected-conversation
    /// resolution is the correct behavior. Bypassing to that legacy resolution once bound would
    /// be wrong: it has no way to find this binding once the conversation already has exchanges
    /// but hasn't been assigned a server token yet, which is routinely true for every follow-up
    /// after the first.
    pub(super) fn route_native_startup_injection(
        &mut self,
        prompt: &str,
        token: Option<&ServerConversationToken>,
        attachments: &[AgentAttachment],
        participant_id: &ParticipantId,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(id) = self.native_prompt_conversation_id else {
            log::info!(
                "event=injection_bypassed_queue task_id={:?} terminal_id={:?} participant_id={participant_id} reason=no_native_binding has_target_token={}",
                self.ambient_agent_task_id,
                self.terminal_surface_id,
                token.is_some(),
            );
            return false;
        };
        if let Some(token) = token
            && let Some(target) =
                self.find_existing_conversation_by_server_token(&token.to_string(), ctx)
            && target != id
        {
            report_error!(
                "Rejected a startup injection targeting a different native conversation",
                extra: { "conversation_id" => %id, "target_conversation_id" => %target, "terminal_id" => ?self.terminal_surface_id }
            );
            return true;
        }
        let row = QueuedQuery::new_shared_session_prompt(
            prompt.to_owned(),
            participant_id.clone(),
            attachments.to_vec(),
        );
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.append(id, row, ctx);
        });
        if self.can_dispatch_queued_warp_agent_prompt(id, ctx) {
            self.dispatch_queued_warp_agent_prompt(id, None, ctx);
        }
        true
    }

    /// Dispatches a queued prompt row for `conversation_id` as its own fresh request, regardless
    /// of whether it originated locally or from a shared-session injection: a local prompt via
    /// [`Self::send_queued_user_query_in_conversation`], a shared-session-injected prompt via
    /// the attachment-staging/download pipeline in [`Self::send_native_startup_injection`]. A
    /// shell command, a locked row, or a row currently being edited is left queued for
    /// `TerminalInput`/`TerminalView::drain_queued_prompts` to handle instead, since those need
    /// the editor and PTY access this controller doesn't have.
    ///
    /// `query_id` selects a specific row -- used by an explicit override such as "Send now",
    /// which may target a row other than the head and is allowed to interrupt an active stream
    /// on purpose. `None` dispatches the head row in FIFO order, and only if
    /// [`QueuedQueryModel::peek_autofire`] says it's a plain, unlocked, non-edited row; used by
    /// every automatic trigger, which must check [`Self::can_dispatch_queued_warp_agent_prompt`]
    /// first so this never interrupts an active stream.
    ///
    /// Either way, deferred (leaving the row queued) while a CLI subagent is active for
    /// `conversation_id`, since interrupting an in-progress shell command is more disruptive
    /// than interrupting an LLM turn; `TerminalView::drain_queued_prompts` re-attempts this the
    /// next time any turn completes, so a deferred row is not stuck.
    ///
    /// Only ever dispatches **one** row: callers that want every currently-queued row flushed
    /// must call this again once the previous row's send settles, not loop over it synchronously
    /// -- looping would cancel each row's request before the previous one produced any output,
    /// silently dropping every row but the last.
    pub(crate) fn dispatch_queued_warp_agent_prompt(
        &mut self,
        conversation_id: AIConversationId,
        query_id: Option<QueuedQueryId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let has_active_subagent = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.has_active_subagent());
        if has_active_subagent {
            log::info!(
                "event=native_queue_drain_deferred conversation_id={conversation_id} reason=active_subagent",
            );
            return;
        }
        let row = match query_id {
            Some(query_id) => QueuedQueryModel::as_ref(ctx)
                .queue(conversation_id)
                .iter()
                .find(|row| row.id() == query_id)
                .filter(|row| !row.is_command())
                .cloned(),
            // Only a plain, unlocked, non-edited row is safe to fire automatically here; a
            // command, a locked row, or one being edited needs `TerminalInput`/
            // `drain_queued_prompts` instead, so leave it queued for those to pick up.
            None => match QueuedQueryModel::as_ref(ctx).peek_autofire(conversation_id) {
                Some(AutofireAction::Submit { query_id, .. }) => QueuedQueryModel::as_ref(ctx)
                    .queue(conversation_id)
                    .iter()
                    .find(|row| row.id() == query_id)
                    .cloned(),
                Some(
                    AutofireAction::ExecuteCommand { .. } | AutofireAction::PopFromEditMode { .. },
                )
                | None => None,
            },
        };
        let Some(row) = row else {
            return;
        };
        let row_id = row.id();
        log::info!(
            "event=native_queue_row_dispatched task_id={:?} terminal_id={:?} conversation_id={conversation_id} query_id={row_id:?}",
            self.ambient_agent_task_id,
            self.terminal_surface_id,
        );
        if row.shared_session_prompt().is_some() {
            // `send_native_startup_injection` takes ownership of the row directly rather than
            // re-resolving it by id, so it's safe to remove up front. Arming the in-flight
            // marker first (in the same update call, before removal's `Removed` event is
            // delivered) keeps `has_pending_native_injections` true across the async download
            // gap that can follow -- see that method's doc comment.
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.arm_download_in_flight(conversation_id);
                queue.remove_fired_row(conversation_id, row_id, ctx);
            });
            self.send_native_startup_injection(conversation_id, row, ctx);
        } else {
            // The send path resolves this row's attachments by id, so it must still be in the
            // queue when this is called; remove it only afterward.
            self.send_queued_user_query_in_conversation(
                row.text().to_owned(),
                conversation_id,
                None,
                row_id,
                ctx,
            );
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.remove_fired_row(conversation_id, row_id, ctx);
            });
        }
    }

    /// Resolves `row`'s attachments and sends it into `conversation_id` via the normal
    /// existing-conversation follow-up path, which cancels any turn already in flight before
    /// sending. Block and plain-text attachments are staged onto the live context model (the
    /// same way `execute_warp_agent_prompt_from_shared_session_injection` stages them for a
    /// live, non-startup injection) so the standard send path picks them up automatically; file
    /// attachments are downloaded asynchronously first.
    fn send_native_startup_injection(
        &mut self,
        conversation_id: AIConversationId,
        row: QueuedQuery,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some((participant_id, attachments)) = row.shared_session_prompt() else {
            report_error!(
                "Expected a shared-session-injected row to send to a native conversation"
            );
            return;
        };
        let participant_id = participant_id.clone();
        let attachments = attachments.to_vec();
        let query_id = row.id();
        let text = row.text().to_owned();

        let mut block_ids = Vec::new();
        let mut selected_text_parts = Vec::new();
        let mut file_downloads: Vec<(String, String)> = Vec::new();
        for attachment in attachments {
            match attachment {
                AgentAttachment::BlockReference { block_id } => {
                    block_ids.push(BlockId::from(block_id.to_string()));
                }
                AgentAttachment::PlainText { content } => {
                    selected_text_parts.push(content);
                }
                AgentAttachment::FileReference {
                    attachment_id,
                    file_name,
                } => {
                    file_downloads.push((attachment_id, file_name));
                }
            }
        }
        self.context_model.update(ctx, |context_model, ctx| {
            if !block_ids.is_empty() {
                context_model.set_pending_context_block_ids(block_ids, false, ctx);
            }
            if !selected_text_parts.is_empty() {
                context_model.set_pending_context_selected_text(
                    Some(selected_text_parts.join("\n")),
                    false,
                    ctx,
                );
            }
        });

        if file_downloads.is_empty() {
            self.dispatch_native_startup_injection(
                conversation_id,
                text,
                participant_id,
                HashMap::new(),
                ctx,
            );
            return;
        }

        log::info!(
            "event=queued_attachment_download_started conversation_id={conversation_id} query_id={query_id:?} file_count={}",
            file_downloads.len(),
        );
        let Some((task_id, directory)) = self
            .ambient_agent_task_id
            .zip(self.attachments_download_dir.clone())
        else {
            report_error!(
                "Missing native attachment download configuration for a queued startup injection",
                extra: { "conversation_id" => %conversation_id, "query_id" => ?query_id }
            );
            self.dispatch_native_startup_injection(
                conversation_id,
                text,
                participant_id,
                HashMap::new(),
                ctx,
            );
            return;
        };
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let http_client = ServerApiProvider::as_ref(ctx).get_http_client();
        ctx.spawn(
            download_task_file_attachments(
                ai_client,
                http_client,
                task_id,
                directory,
                file_downloads,
            ),
            move |controller, downloaded, ctx| {
                let file_attachments = build_file_attachment_map(&downloaded);
                controller.dispatch_native_startup_injection(
                    conversation_id,
                    text,
                    participant_id,
                    file_attachments,
                    ctx,
                );
            },
        );
    }

    /// Sends the fully-resolved `text`/`file_attachments` into `conversation_id`, via the same
    /// path used for a live (non-startup) shared-session follow-up targeting an existing
    /// conversation (`send_warp_agent_prompt_from_shared_session_injection`). Every path that
    /// removed a row via [`QueuedQueryModel::arm_download_in_flight`] funnels through here, so
    /// clearing that marker unconditionally at the top covers all of them, including the two
    /// synchronous paths that never needed a download in the first place.
    fn dispatch_native_startup_injection(
        &mut self,
        conversation_id: AIConversationId,
        text: String,
        participant_id: ParticipantId,
        file_attachments: HashMap<String, AIAgentAttachment>,
        ctx: &mut ModelContext<Self>,
    ) {
        QueuedQueryModel::handle(ctx).update(ctx, |queue, _ctx| {
            queue.clear_download_in_flight(conversation_id);
        });
        if FeatureFlag::AgentView.is_enabled() {
            self.context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_query_state_for_existing_conversation(
                    conversation_id,
                    AgentViewEntryOrigin::SharedSessionSelection,
                    ctx,
                );
            });
        }
        self.send_user_query_in_conversation_with_attachments(
            text,
            conversation_id,
            Some(participant_id),
            file_attachments,
            ctx,
        );
    }
}

#[cfg(test)]
#[path = "startup_queue_tests.rs"]
mod tests;
