use std::sync::Arc;

use session_sharing_protocol::common::SessionId;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::agent::conversation::AIConversationId;
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::ServerApiProvider;

/// Ensures that session ID for locally owned shared conversations is linked
/// to their `ai_tasks` row in the DB. This enables viewers to reconstruct
/// the conversation's orchestration state.
pub struct LocalSharedSessionLinkModel {
    ai_client: Arc<dyn AIClient>,
}

pub enum LocalSharedSessionLinkModelEvent {}

impl LocalSharedSessionLinkModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        Self::new_with_ai_client(ai_client, ctx)
    }

    /// Test-friendly constructor.
    fn new_with_ai_client(ai_client: Arc<dyn AIClient>, ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |me, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        Self { ai_client }
    }

    /// Test-only constructor that lets tests inject a mock `AIClient`.
    #[cfg(test)]
    pub(super) fn new_with_ai_client_for_test(
        ai_client: Arc<dyn AIClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self::new_with_ai_client(ai_client, ctx)
    }

    fn handle_history_event(&self, event: &BlocklistAIHistoryEvent, ctx: &mut ModelContext<Self>) {
        if let BlocklistAIHistoryEvent::LocalSharedSessionEstablished {
            conversation_id,
            session_id,
        } = event
        {
            self.on_local_shared_session_established(*conversation_id, *session_id, ctx);
        }
    }

    /// Links the conversation's `task_id` to `session_id` on the server.
    /// Skips viewers, remote-child placeholders, and conversations without
    /// a `task_id` (pre-StreamInit).
    fn on_local_shared_session_established(
        &self,
        conversation_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        if conversation.is_viewing_shared_session() {
            return;
        }
        if conversation.is_remote_child() {
            return;
        }
        let Some(task_id) = conversation.task_id() else {
            return;
        };

        let ai_client = self.ai_client.clone();
        ctx.spawn(
            async move {
                if let Err(err) = ai_client
                    .update_agent_task(task_id, None, Some(session_id), None, None)
                    .await
                {
                    log::warn!(
                        "LocalSharedSessionLinkModel: failed to link task {task_id} to shared session {session_id}: {err:#}"
                    );
                }
            },
            |_, _, _| {},
        );
    }
}

impl Entity for LocalSharedSessionLinkModel {
    type Event = LocalSharedSessionLinkModelEvent;
}

impl SingletonEntity for LocalSharedSessionLinkModel {}

#[cfg(test)]
#[path = "local_shared_session_link_model_tests.rs"]
mod tests;
