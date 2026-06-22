use session_sharing_protocol::common::SessionId;
use uuid::Uuid;
use warpui::{SingletonEntity, ViewContext, ViewHandle};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent_conversations_model::{
    AgentConversationEntryId, AgentConversationNavigationSubject, AgentConversationsModel,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::pane_group::{PaneGroup, PaneId, TerminalPane, TerminalViewResources};
use crate::terminal::TerminalView;
use crate::workspace::WorkspaceAction;

/// The restoration path for an ambient agent pane.
pub(in crate::pane_group) enum AmbientRestoreKind {
    /// Active shared session
    SharedSession { session_id: SessionId },
    /// Conversation data isn't loaded yet — show a loading pane and
    /// defer the real restoration to the pending-restoration subscription
    /// (which waits for the data to be loaded async).
    PendingRestoration { task_id: AmbientAgentTaskId },
    /// If there's no task ID to restore, we open a fresh cloud mode pane
    /// (this is a valid state from when a user quits with an empty cloud mode pane).
    NewCloudConversation,
}

impl PaneGroup {
    /// Stores the pending ambient agent restorations, triggers async fetches for
    /// their task data, and sets up a single long-lived subscription that will
    /// process each pane as its task data arrives.
    pub(in crate::pane_group) fn register_pending_ambient_restorations(
        &mut self,
        pending: Vec<(AmbientAgentTaskId, PaneId)>,
        ctx: &mut ViewContext<Self>,
    ) {
        for (task_id, _) in &pending {
            AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(task_id, ctx);
            });
        }

        self.pending_ambient_agent_conversation_restorations = pending.into_iter().collect();

        self.ensure_pending_ambient_restoration_subscription(ctx);
    }

    /// Drains entries from `pending_ambient_agent_conversation_restorations`
    /// for which task data is now available, replacing or hydrating the
    /// corresponding panes.
    pub(in crate::pane_group) fn process_pending_ambient_restorations(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self
            .pending_ambient_agent_conversation_restorations
            .is_empty()
        {
            return;
        }

        let ready_tasks: Vec<_> = self
            .pending_ambient_agent_conversation_restorations
            .keys()
            .filter(|task_id| {
                AgentConversationsModel::as_ref(ctx)
                    .get_task_data(task_id)
                    .is_some()
            })
            .copied()
            .collect();

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();

        for task_id in ready_tasks {
            let Some(pane_id) = self
                .pending_ambient_agent_conversation_restorations
                .remove(&task_id)
            else {
                continue;
            };
            let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
                continue;
            };

            match AgentConversationsModel::resolve_open_action(
                AgentConversationNavigationSubject::Entry(AgentConversationEntryId::AmbientRun(
                    task.task_id,
                )),
                None,
                ctx,
            ) {
                Some(WorkspaceAction::OpenOrAttachAmbientAgentConversation {
                    session_id,
                    task_id: _,
                }) => {
                    let (view, terminal_manager) = Self::create_shared_session_viewer(
                        session_id,
                        resources.clone(),
                        view_size,
                        true, // enable_orchestration_polling
                        true, // is_cloud_mode
                        ctx,
                    );
                    let new_pane = TerminalPane::new(
                        Uuid::new_v4().as_bytes().to_vec(),
                        terminal_manager,
                        view,
                        self.model_event_sender.clone(),
                        ctx,
                    );
                    self.replace_pane(pane_id, new_pane, false, ctx);
                }
                Some(WorkspaceAction::OpenConversationTranscriptViewer {
                    conversation_id,
                    ambient_agent_task_id,
                }) => {
                    if let Some(target_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
                        Self::fetch_and_load_transcript(
                            target_view,
                            conversation_id,
                            ambient_agent_task_id,
                            ctx,
                        );
                    } else {
                        self.pending_ambient_agent_conversation_restorations
                            .insert(task_id, pane_id);
                    }
                }
                _ => {
                    self.replace_pane_with_new_cloud_conversation(pane_id, ctx);
                }
            }
        }
    }

    /// Replaces a pane with a new cloud conversation.
    fn replace_pane_with_new_cloud_conversation(
        &mut self,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (view, terminal_manager) =
            Self::create_ambient_agent_terminal(resources, view_size, ctx);
        let new_pane = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            view,
            self.model_event_sender.clone(),
            ctx,
        );
        self.replace_pane(pane_id, new_pane, false, ctx);
    }

    /// Fetches conversation data and loads it into the given transcript viewer.
    fn fetch_and_load_transcript(
        target_view: ViewHandle<TerminalView>,
        server_conversation_token: ServerConversationToken,
        ambient_agent_task_id: Option<AmbientAgentTaskId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model_handle = BlocklistAIHistoryModel::handle(ctx);

        let future = history_model_handle.update(ctx, |history_model, ctx| {
            history_model.load_conversation_by_server_token(&server_conversation_token, ctx)
        });
        ctx.spawn(future, move |group, conversation, ctx| {
            if let Some(conversation) = conversation {
                group.load_data_into_transcript_viewer(
                    target_view,
                    conversation,
                    ambient_agent_task_id,
                    ctx,
                );
            } else if let Some(pane_id) =
                group.find_pane_id_for_terminal_view(target_view.id(), ctx)
            {
                log::error!(
                    "Failed to restore ambient agent pane, replacing with new cloud conversation"
                );
                group.replace_pane_with_new_cloud_conversation(pane_id, ctx);
            }
        });
    }
}
