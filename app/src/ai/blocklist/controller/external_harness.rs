// Plumbing for harnesses that run outside Warp's agent loop but render through a native
// conversation: the harness translates its own protocol into MAA response events, which flow
// through the same controller path as events from the MAA server. The conversation stays owned
// by this client (so task status syncing keeps working); only the turn itself is the harness's.
use session_sharing_protocol::common::{AgentAttachment, ParticipantId};
use uuid::Uuid;
use warp_errors::report_error;
use warp_multi_agent_api::response_event::StreamInit;
use warp_multi_agent_api::{ResponseEvent, response_event};
use warpui::{ModelContext, SingletonEntity};

use super::response_stream::ResponseStreamId;
use super::{BlocklistAIController, RequestInput};
use crate::ai::agent::BaseUserQuery;
use crate::ai::agent::conversation::{AIConversationId, ConversationDriver, ConversationStatus};
use crate::ai::blocklist::QueuedQueryModel;
use crate::ai::blocklist::history_model::{BlocklistAIHistoryModel, UpdateHistoryError};
use crate::workspaces::user_workspaces::ResolvedTeamScope;

/// Identifiers for one harness-driven turn.
#[derive(Clone, Debug)]
pub(in crate::ai) struct ExternalHarnessTurn {
    pub stream_id: ResponseStreamId,
    pub request_id: String,
}

/// A prompt relayed to the external harness driving a native conversation.
#[derive(Clone, Debug)]
pub(in crate::ai) struct ExternalHarnessPrompt {
    pub text: String,
    /// Context the prompt's author attached, kept in its shared-session wire shape so the
    /// harness can decide how to present each kind to its agent.
    pub attachments: Vec<AgentAttachment>,
    /// The shared-session participant who injected the prompt, so the turn it opens is
    /// attributed to them for viewers.
    pub participant_id: ParticipantId,
    /// The `Request.Input.UserQuery` warp-server injected with the prompt, when it sent one.
    /// Its attribution is copied onto the user query echoed into the conversation.
    pub base: Option<BaseUserQuery>,
}

impl BlocklistAIController {
    /// Diverts prompts injected into the bound native conversation to an external harness.
    /// Shared-session prompts that were queued for the conversation before the sink existed
    /// (the conversation is bound at driver construction, long before the harness is up) are
    /// relayed first, in arrival order, so the harness sees them ahead of anything injected
    /// later. The native setup barrier is left in place: nothing may dispatch from the queue
    /// to Warp's agent while the harness owns the conversation.
    pub(in crate::ai) fn set_external_harness_prompt_sink(
        &mut self,
        sink: async_channel::Sender<ExternalHarnessPrompt>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation_id) = self.native_prompt_conversation_id {
            self.relay_queued_shared_session_prompts(conversation_id, &sink, ctx);
        }
        self.external_harness_prompt_sink = Some(sink);
    }

    fn relay_queued_shared_session_prompts(
        &self,
        conversation_id: AIConversationId,
        sink: &async_channel::Sender<ExternalHarnessPrompt>,
        ctx: &mut ModelContext<Self>,
    ) {
        let queued: Vec<_> = QueuedQueryModel::as_ref(ctx)
            .queue(conversation_id)
            .iter()
            .filter_map(|row| {
                let (participant_id, attachments) = row.shared_session_prompt()?;
                Some((
                    row.id(),
                    ExternalHarnessPrompt {
                        text: row.text().to_owned(),
                        attachments: attachments.to_vec(),
                        participant_id: participant_id.clone(),
                        base: row.base_user_query().cloned(),
                    },
                ))
            })
            .collect();
        if queued.is_empty() {
            return;
        }
        log::info!(
            "event=queued_injections_relayed_to_external_harness task_id={:?} terminal_id={:?} conversation_id={conversation_id} count={}",
            self.ambient_agent_task_id,
            self.terminal_surface_id,
            queued.len(),
        );
        QueuedQueryModel::handle(ctx).update(ctx, |queue, ctx| {
            for (query_id, prompt) in queued {
                if sink.try_send(prompt).is_err() {
                    report_error!(
                        "Dropped a queued shared-session prompt because the external harness is not accepting prompts",
                        extra: { "conversation_id" => %conversation_id, "query_id" => ?query_id }
                    );
                }
                queue.remove_fired_row(conversation_id, query_id, ctx);
            }
        });
    }

    /// Opens a harness-driven turn on `conversation_id`: registers an exchange with no inputs
    /// for a fresh response stream, marks the conversation in progress, and applies the locally
    /// minted `StreamInit`. The harness must add the user query and task messages through
    /// [`Self::apply_external_harness_event`], as the MAA server does.
    pub(in crate::ai) fn begin_external_harness_turn(
        &mut self,
        conversation_id: AIConversationId,
        run_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<ExternalHarnessTurn, UpdateHistoryError> {
        let history = BlocklistAIHistoryModel::handle(ctx);
        let (conversation_token, root_task_id) = {
            let conversation = history
                .as_ref(ctx)
                .conversation(&conversation_id)
                .ok_or(UpdateHistoryError::ConversationNotFound(conversation_id))?;
            (
                conversation
                    .server_conversation_token()
                    .map(|token| token.as_str().to_string())
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                conversation.get_root_task_id().clone(),
            )
        };
        let request_id = Uuid::new_v4().to_string();
        let init = StreamInit {
            conversation_id: conversation_token,
            request_id: request_id.clone(),
            run_id: run_id.unwrap_or_default(),
        };
        let stream_id = ResponseStreamId::for_shared_session(&init);

        let scope = ResolvedTeamScope::from_scope(&self.team_context(ctx));
        let request_input = RequestInput::for_task(
            vec![],
            root_task_id,
            &self.active_session,
            self.get_current_response_initiator(),
            conversation_id,
            self.terminal_surface_id,
            &scope,
            ctx,
        );
        let terminal_surface_id = self.terminal_surface_id;
        history.update(ctx, |history, ctx| {
            history
                .set_driver_for_conversation(conversation_id, ConversationDriver::ExternalHarness);
            history.update_conversation_for_new_request_input(
                request_input,
                stream_id.clone(),
                terminal_surface_id,
                ctx,
            )?;
            history.update_conversation_status(
                terminal_surface_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
            history.set_active_conversation_id(conversation_id, terminal_surface_id, ctx);
            Ok::<_, UpdateHistoryError>(())
        })?;

        self.apply_response_event(
            &stream_id,
            conversation_id,
            ResponseEvent {
                r#type: Some(response_event::Type::Init(init)),
            },
            None,
            ctx,
        );

        Ok(ExternalHarnessTurn {
            stream_id,
            request_id,
        })
    }

    /// Applies a harness-authored response event to the turn open on `stream_id`.
    pub(in crate::ai) fn apply_external_harness_event(
        &mut self,
        stream_id: &ResponseStreamId,
        event: ResponseEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation_id) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation_for_response_stream(stream_id)
        else {
            log::warn!("No conversation for external harness response stream {stream_id:?}");
            return;
        };
        self.apply_response_event(stream_id, conversation_id, event, None, ctx);
    }
}

#[cfg(test)]
#[path = "external_harness_tests.rs"]
mod tests;
