use std::collections::HashMap;

use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui::{ModelContext, SingletonEntity};

use super::BlocklistAIController;
use super::shared_session::SharedSessionPromptTarget;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAttachment, BaseUserQuery};
use crate::ai::attachment_utils::{
    build_file_attachment_map, download_task_file_attachments, resolve_agent_attachments,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedPromptDeliveryMode, QueuedQuery, QueuedQueryId, QueuedQueryModel,
};
use crate::server::server_api::ServerApiProvider;

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
        base: Option<&BaseUserQuery>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(bound_id) = self.native_prompt_conversation_id else {
            log::info!(
                "event=injection_bypassed_queue task_id={:?} terminal_id={:?} participant_id={participant_id} reason=no_native_binding has_target_token={}",
                self.ambient_agent_task_id,
                self.terminal_surface_id,
                token.is_some(),
            );
            return false;
        };
        let id = match self.resolve_shared_session_prompt_target(token, ctx) {
            SharedSessionPromptTarget::Existing(id) => id,
            SharedSessionPromptTarget::Rejected { target } => {
                report_error!(
                    "Rejected a startup injection targeting a different native conversation",
                    extra: { "conversation_id" => %bound_id, "target_conversation_id" => ?target, "terminal_id" => ?self.terminal_surface_id }
                );
                return true;
            }
            // Defensive: while bound, the resolver should always resolve to `bound_id` when
            // there's no conflicting token (see its doc comment), so this shouldn't happen in
            // practice -- but nothing in the resolver's signature guarantees that, so drop the
            // prompt rather than assume it can't occur.
            SharedSessionPromptTarget::NoToken => {
                report_error!(
                    "Shared-session prompt resolver unexpectedly returned NoToken while bound \
                     to a native conversation",
                    extra: { "conversation_id" => %bound_id, "terminal_id" => ?self.terminal_surface_id }
                );
                return true;
            }
        };
        let row = QueuedQuery::new_shared_session_prompt(
            prompt.to_owned(),
            participant_id.clone(),
            attachments.to_vec(),
            base.cloned(),
        );
        let query_id = row.id();
        let needs_preparation = !row.is_ready();
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.append(id, row, ctx);
        });
        if needs_preparation {
            self.prepare_queued_attachments(id, query_id, attachments.to_vec(), ctx);
        }
        if self.can_dispatch_queued_warp_agent_prompt(id, ctx) {
            self.dispatch_queued_warp_agent_prompt(id, None, ctx);
        }
        true
    }

    /// Sends a prepared queued prompt. An explicit ID may interrupt; automatic sends preserve FIFO.
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
        let queue = QueuedQueryModel::as_ref(ctx);
        let row = match query_id {
            Some(query_id) => queue.ready_query(conversation_id, query_id),
            None => queue.ready_head(conversation_id),
        }
        .filter(|row| !row.is_command())
        .cloned();
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
            self.send_native_startup_injection(conversation_id, row, ctx);
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.remove_fired_row(conversation_id, row_id, ctx);
            });
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

    /// Sends a prepared shared-session prompt with its attributed context.
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
        let text = row.text().to_owned();
        let Some(file_attachments) = row.prepared_files().cloned() else {
            return;
        };
        let base = row.base_user_query().cloned();

        let (block_ids, selected_text_parts, _) = resolve_agent_attachments(attachments);
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

        self.dispatch_native_startup_injection(
            conversation_id,
            text,
            participant_id,
            file_attachments,
            base,
            ctx,
        );
    }

    fn prepare_queued_attachments(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        attachments: Vec<AgentAttachment>,
        ctx: &mut ModelContext<Self>,
    ) {
        let (_, _, file_downloads) = resolve_agent_attachments(attachments);
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
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.complete_preparation(conversation_id, query_id, HashMap::new(), ctx);
            });
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
            move |_, downloaded, ctx| {
                QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                    queue.complete_preparation(
                        conversation_id,
                        query_id,
                        build_file_attachment_map(&downloaded),
                        ctx,
                    );
                });
            },
        );
    }

    /// Submits an attributed prompt with resolved file attachments.
    fn dispatch_native_startup_injection(
        &mut self,
        conversation_id: AIConversationId,
        text: String,
        participant_id: ParticipantId,
        file_attachments: HashMap<String, AIAgentAttachment>,
        base: Option<BaseUserQuery>,
        ctx: &mut ModelContext<Self>,
    ) {
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
            base,
            ctx,
        );
    }
}

#[cfg(test)]
#[path = "startup_queue_tests.rs"]
mod tests;
