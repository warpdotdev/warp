use std::collections::HashMap;

use anyhow::{Context as _, anyhow};
use session_sharing_protocol::common::{AgentAttachment, ParticipantId, ServerConversationToken};
use warp_errors::report_error;
use warp_multi_agent_api::AgentType;
use warpui::{ModelContext, SingletonEntity};

use super::input_context::{input_context_for_request, parse_context_attachments};
use super::response_stream::RecoveryBudget;
use super::{BlocklistAIController, RequestInput};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentAttachment, AIAgentContext, AIAgentInput, EntrypointType, RequestMetadata,
    extract_user_query_mode,
};
use crate::ai::attachment_utils::{
    DownloadedAttachment, build_file_attachment_map, download_file, sanitize_filename,
};
use crate::ai::blocklist::{BlocklistAIHistoryModel, QueuedQuery, QueuedQueryId, QueuedQueryModel};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::model::BlockId;
use crate::workspaces::user_workspaces::ResolvedTeamScope;

impl BlocklistAIController {
    pub(crate) fn prepare_native_prompt_queue(
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
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| queue.begin_native_setup(id, ctx));
        id
    }

    pub(crate) fn native_prompt_conversation_id(&self) -> Option<AIConversationId> {
        self.native_prompt_conversation_id
    }
    pub(crate) fn stop_native_prompt_queue(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(id) = self.native_prompt_conversation_id.take() {
            log::info!(
                "event=native_queue_stopped task_id={:?} terminal_id={:?} conversation_id={id} unsent_count={}",
                self.ambient_agent_task_id,
                self.terminal_surface_id,
                QueuedQueryModel::as_ref(ctx).queue(id).len(),
            );
            QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
                queue.cancel_injection_dispatch(id, ctx);
                queue.begin_native_setup(id, ctx);
            });
        }
    }

    pub(super) fn queue_native_startup_injection(
        &self,
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
        if !QueuedQueryModel::as_ref(ctx).has_pending_native_injections(id) {
            log::info!(
                "event=injection_bypassed_queue task_id={:?} terminal_id={:?} conversation_id={id} participant_id={participant_id} reason=no_startup_backlog",
                self.ambient_agent_task_id,
                self.terminal_surface_id,
            );
            return false;
        }
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
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.append(
                id,
                QueuedQuery::new_shared_session_prompt(
                    prompt.to_owned(),
                    participant_id.clone(),
                    attachments.to_vec(),
                )
                .with_shared_session_target(token.cloned()),
                ctx,
            );
        });
        true
    }

    pub(crate) fn send_queued_shared_session_prompt(
        &mut self,
        conversation_id: AIConversationId,
        query_id: QueuedQueryId,
        ctx: &mut ModelContext<Self>,
    ) {
        let has_active_stream = self.has_active_stream_for_conversation(conversation_id, ctx);
        let has_active_subagent = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| conversation.has_active_subagent());
        if has_active_stream || has_active_subagent {
            log::info!(
                "event=dispatch_deferred conversation_id={conversation_id} query_id={query_id:?} active_stream={has_active_stream} active_subagent={has_active_subagent}",
            );
            return;
        }
        let row = QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.claim_injection(conversation_id, query_id, ctx)
        });
        let Some(row) = row else { return };
        let (_, attachments) = row.shared_session_prompt().unwrap();
        let files: Vec<_> = attachments
            .iter()
            .filter_map(|attachment| match attachment {
                AgentAttachment::FileReference {
                    attachment_id,
                    file_name,
                } => Some((attachment_id.clone(), file_name.clone())),
                AgentAttachment::PlainText { .. } | AgentAttachment::BlockReference { .. } => None,
            })
            .collect();
        if files.is_empty() {
            self.finish_queued_injection(conversation_id, row, Ok(HashMap::new()), ctx);
            return;
        }
        log::info!(
            "event=queued_attachment_download_started conversation_id={conversation_id} query_id={query_id:?} file_count={}",
            files.len(),
        );
        let Some((task_id, directory)) = self
            .ambient_agent_task_id
            .zip(self.attachments_download_dir.clone())
        else {
            self.finish_queued_injection(
                conversation_id,
                row,
                Err(anyhow!("Missing native attachment download configuration")),
                ctx,
            );
            return;
        };
        let client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let ids = files.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
                let urls = client.download_task_attachments(&task_id, &ids).await?;
                async_fs::create_dir_all(&directory).await?;
                let mut downloads = Vec::new();
                for (id, name) in files {
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
                Ok(build_file_attachment_map(&downloads))
            },
            move |controller, result, ctx| {
                controller.finish_queued_injection(conversation_id, row, result, ctx)
            },
        );
    }

    fn finish_queued_injection(
        &mut self,
        conversation_id: AIConversationId,
        row: QueuedQuery,
        files: anyhow::Result<HashMap<String, AIAgentAttachment>>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !QueuedQueryModel::as_ref(ctx).is_injection_dispatch_current(conversation_id, row.id()) {
            log::info!(
                "event=dispatch_abandoned conversation_id={conversation_id} query_id={:?} reason=claim_invalidated",
                row.id(),
            );
            return;
        }
        let result = files.and_then(|files| {
            let request = self.queued_injection_request(conversation_id, &row, files, ctx)?;
            if let Some(participant_id) = request.shared_session_response_initiator.clone() {
                self.set_current_response_initiator(participant_id);
            }
            self.send_request_input(
                request,
                Some(RequestMetadata {
                    is_autodetected_user_query: false,
                    entrypoint: EntrypointType::SharedSession,
                    is_auto_resume_after_error: false,
                }),
                RecoveryBudget::fresh(),
                true,
                ctx,
            )
            .map(|(_, stream_id)| {
                log::info!(
                    "event=dispatch_accepted task_id={:?} terminal_id={:?} conversation_id={conversation_id} query_id={:?} stream_id={stream_id:?} queue_len_after={}",
                    self.ambient_agent_task_id, self.terminal_surface_id, row.id(),
                    QueuedQueryModel::as_ref(ctx).queue(conversation_id).len().saturating_sub(1),
                );
            })
        });
        let error = result.err().map(|error| {
            report_error!(
                error.context("Could not dispatch queued native prompt"),
                extra: { "conversation_id" => %conversation_id, "query_id" => ?row.id(), "terminal_id" => ?self.terminal_surface_id }
            );
            "Could not send a queued prompt. The unsent prompt remains queued.".to_owned()
        });
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            queue.finish_injection_dispatch(conversation_id, row.id(), error, ctx);
        });
    }

    fn queued_injection_request(
        &self,
        conversation_id: AIConversationId,
        row: &QueuedQuery,
        files: HashMap<String, AIAgentAttachment>,
        ctx: &ModelContext<Self>,
    ) -> anyhow::Result<RequestInput> {
        let conversation = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .context("Queued native conversation no longer exists")?;
        if let Some(token) = row.shared_session_target()
            && conversation
                .server_conversation_token()
                .map(|value| value.as_str())
                != Some(token.to_string().as_str())
        {
            return Err(anyhow!(
                "Queued prompt targets a different server conversation"
            ));
        }
        if matches!(
            conversation.status(),
            ConversationStatus::Cancelled | ConversationStatus::Error
        ) {
            return Err(anyhow!(
                "Queued native conversation stopped before dispatch"
            ));
        }
        let task_id = conversation.get_root_task_id().clone();
        let (participant_id, attachments) = row
            .shared_session_prompt()
            .context("Expected an injected prompt")?;
        let context_model = self.context_model.as_ref(ctx);
        let mut extra_context = Vec::new();
        for attachment in attachments {
            match attachment {
                AgentAttachment::PlainText { content } => {
                    extra_context.push(AIAgentContext::SelectedText(content.clone()))
                }
                AgentAttachment::BlockReference { block_id } => {
                    if let Some(context) = context_model
                        .transform_block_to_context(&BlockId::from(block_id.to_string()), false)
                    {
                        extra_context.push(context);
                    }
                }
                AgentAttachment::FileReference { .. } => {}
            }
        }
        let context = input_context_for_request(
            false,
            context_model,
            self.active_session.as_ref(ctx),
            Some(conversation_id),
            extra_context,
            ctx,
        );
        let (query, user_query_mode) = extract_user_query_mode(row.text().to_owned());
        let mut referenced_attachments = parse_context_attachments(&query, context_model, ctx);
        referenced_attachments.extend(files);
        let input = AIAgentInput::UserQuery {
            query,
            context,
            static_query_type: None,
            referenced_attachments,
            user_query_mode,
            running_command: None,
            intended_agent: Some(AgentType::Primary),
        };
        let scope = ResolvedTeamScope::from_scope(&self.team_context(ctx));
        Ok(RequestInput::for_task(
            vec![input],
            task_id,
            &self.active_session,
            Some(participant_id.clone()),
            conversation_id,
            self.terminal_surface_id,
            &scope,
            ctx,
        ))
    }
}

#[cfg(test)]
#[path = "startup_queue_tests.rs"]
mod tests;
