use std::collections::HashMap;

use anyhow::Context as _;
use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui::{ModelContext, SingletonEntity};

use super::BlocklistAIController;
use crate::ai::agent::AIAgentAttachment;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::attachment_utils::{
    DownloadedAttachment, build_file_attachment_map, download_file, sanitize_filename,
};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedPromptDeliveryMode, QueuedQuery, QueuedQueryModel,
};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::BlockId;

impl BlocklistAIController {
    /// Binds this controller to a native conversation before session sharing can begin
    /// delivering startup follow-ups, so `route_native_startup_injection` knows which
    /// conversation to target for the rest of this run. Idempotent: returns the existing
    /// binding if one is already in place.
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

    pub(crate) fn native_prompt_conversation_id(&self) -> Option<AIConversationId> {
        self.native_prompt_conversation_id
    }

    /// Unbinds this controller from its native conversation, dropping any startup follow-ups
    /// that never made it out (e.g. the run ended before setup finished).
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
            queue.drain_shared_session_injections(id, ctx);
        });
    }

    /// Routes a shared-session-injected prompt while this controller is bound to a native
    /// conversation: always queues it (preserving FIFO order with anything already queued),
    /// then immediately attempts to dispatch the queue's head via
    /// [`Self::dispatch_next_shared_session_row`] when nothing is currently streaming for the
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
        if !QueuedQueryModel::as_ref(ctx).is_dispatch_blocked(id)
            && !self.has_active_stream_for_conversation(id, ctx)
        {
            self.dispatch_next_shared_session_row(id, ctx);
        }
        true
    }

    /// Dispatches the head shared-session-injected row queued for `conversation_id`, if any --
    /// used at points already known to be safe (native setup just finished, no stream currently
    /// active for the conversation, or an explicit "Send now" override) so a queued injection
    /// isn't left waiting behind `Steering`'s piggyback opportunities, which only fire while a
    /// turn is actually in flight producing tool results or orchestration events. No-ops when
    /// the head row isn't a shared-session injection.
    ///
    /// Deferred (leaving the row queued) while a CLI subagent is active for `conversation_id`
    /// (interrupting an in-progress shell command is more disruptive than interrupting an LLM
    /// turn); `TerminalView::drain_queued_prompts` re-attempts this the next time any turn
    /// completes, so a deferred row is not stuck.
    ///
    /// Only ever dispatches **one** row: callers that want every currently-queued row flushed
    /// (e.g. an explicit "Send now") must call this again once the previous row's send settles,
    /// not loop over it synchronously -- looping would cancel each row's request before the
    /// previous one produced any output, silently dropping every row but the last.
    pub(crate) fn dispatch_next_shared_session_row(
        &mut self,
        conversation_id: AIConversationId,
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
        let Some(row) = QueuedQueryModel::as_ref(ctx)
            .queue(conversation_id)
            .first()
            .filter(|row| row.shared_session_prompt().is_some())
            .cloned()
        else {
            return;
        };
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.remove_fired_row(conversation_id, row.id(), ctx);
        });
        log::info!(
            "event=native_queue_row_dispatched task_id={:?} terminal_id={:?} conversation_id={conversation_id} query_id={:?}",
            self.ambient_agent_task_id,
            self.terminal_surface_id,
            row.id(),
        );
        self.send_native_startup_injection(conversation_id, row, ctx);
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
        let client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let ids = file_downloads
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                let urls = client.download_task_attachments(&task_id, &ids).await?;
                async_fs::create_dir_all(&directory).await?;
                let mut downloads = Vec::new();
                for (id, name) in file_downloads {
                    let url = urls
                        .attachments
                        .iter()
                        .find(|attachment| attachment.attachment_id == id)
                        .context("Missing queued attachment download URL")?;
                    let name = sanitize_filename(&name).to_owned();
                    let path = directory.join(format!("{id}_{name}"));
                    download_file(api.http_client(), &url.download_url, &path).await?;
                    downloads.push(DownloadedAttachment {
                        file_id: id,
                        file_name: name,
                        file_path: path.to_string_lossy().into_owned(),
                    });
                }
                anyhow::Ok(build_file_attachment_map(&downloads))
            },
            move |controller, result, ctx| match result {
                Ok(file_attachments) => controller.dispatch_native_startup_injection(
                    conversation_id,
                    text,
                    participant_id,
                    file_attachments,
                    ctx,
                ),
                Err(error) => {
                    report_error!(
                        error.context("Could not download attachments for a queued startup prompt"),
                        extra: { "conversation_id" => %conversation_id, "query_id" => ?query_id }
                    );
                }
            },
        );
    }

    /// Sends the fully-resolved `text`/`file_attachments` into `conversation_id`, via the same
    /// path used for a live (non-startup) shared-session follow-up targeting an existing
    /// conversation (`send_warp_agent_prompt_from_shared_session_injection`).
    fn dispatch_native_startup_injection(
        &mut self,
        conversation_id: AIConversationId,
        text: String,
        participant_id: ParticipantId,
        file_attachments: HashMap<String, AIAgentAttachment>,
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
            ctx,
        );
    }
}

#[cfg(test)]
#[path = "startup_queue_tests.rs"]
mod tests;
