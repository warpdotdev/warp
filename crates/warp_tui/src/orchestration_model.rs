//! [`TuiOrchestrationModel`]: TUI orchestration runtime and navigation state.
//!
//! The shared `StartAgentExecutor` (one per session surface) emits
//! `CreateAgent` and waits for a frontend to materialize the child. In the
//! GUI that materializer is `TerminalView` → `PaneGroup`'s hidden child
//! panes; in the TUI, [`crate::session_registry::TuiSessions`] owns
//! materialization. This singleton prepares native Oz children, requests
//! background session lifecycle changes, tracks the session dimension of the
//! orchestration tree, and projects that tree into the single visible tab bar.
//! Conversation lineage and ordering policy stay in `BlocklistAIHistoryModel`
//! and the shared topology helpers.
//!
//! Native (Oz) local children run in background TUI sessions. Local
//! CLI-harness and remote child requests resolve with an explicit failure.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp::tui_export::{
    apply_child_agent_model_override, descendant_conversation_ids_in_pill_order,
    descendant_conversation_ids_in_spawn_order, inherit_child_agent_settings,
    orchestration_root_conversation_id, prepare_local_oz_child_launch,
    register_agent_event_consumer, unregister_agent_event_consumer, AIConversationId,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatus, Harness,
    RenderableAIError, StartAgentExecutionMode, StartAgentRequest,
};
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::orchestrated_agent_identity_styling::assign_agent_identity_indices;
use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::terminal_session_view::TuiTerminalSessionView;
/// One navigable child tab in an orchestration snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationTab {
    pub(crate) conversation_id: AIConversationId,
    pub(crate) label: String,
    pub(crate) spawn_index: usize,
}

/// Live semantic state for the orchestration tab bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationSnapshot {
    pub(crate) root_conversation_id: AIConversationId,
    pub(crate) selected_conversation_id: AIConversationId,
    pub(crate) tabs: Vec<TuiOrchestrationTab>,
    pub(crate) page_anchor: Option<AIConversationId>,
    pub(crate) reveal_selected: bool,
}


/// The TUI's orchestration singleton. See the module docs.
pub(crate) struct TuiOrchestrationModel {
    /// Session hosting each live child conversation. The session dimension
    /// only — conversation lineage is read from `BlocklistAIHistoryModel`
    /// (`children_by_parent` / `parent_conversation_id`), never mirrored.
    child_session_by_conversation: HashMap<AIConversationId, TuiSessionId>,
    /// Conversations whose event streams are consumed by each live session.
    event_consumers_by_session: HashMap<TuiSessionId, HashSet<AIConversationId>>,
    /// Page state for the single orchestration tab bar visible at a time.
    page_root_conversation_id: Option<AIConversationId>,
    page_anchor: Option<AIConversationId>,
    explicitly_paged: bool,
}
pub(crate) enum TuiOrchestrationEvent {
    CreateLocalOzChildSession {
        parent_session_id: TuiSessionId,
        request: Box<StartAgentRequest>,
        model_id: Option<String>,
        working_directory: Option<PathBuf>,
        task_id: warp::tui_export::AmbientAgentTaskId,
        conversation_name: String,
    },
    RemoveChildSession(TuiSessionId),
    TabBarChanged,
}
pub(crate) struct MaterializedLocalOzChildSession {
    pub(crate) parent_session_id: TuiSessionId,
    pub(crate) session_id: TuiSessionId,
    pub(crate) session_view: ViewHandle<TuiTerminalSessionView>,
    pub(crate) request: StartAgentRequest,
    pub(crate) model_id: Option<String>,
    pub(crate) task_id: warp::tui_export::AmbientAgentTaskId,
    pub(crate) conversation_name: String,
}

impl Entity for TuiOrchestrationModel {
    type Event = TuiOrchestrationEvent;
}

impl SingletonEntity for TuiOrchestrationModel {}

impl TuiOrchestrationModel {
    /// Registers the singleton before sessions are created and wired to it.
    pub(crate) fn register(ctx: &mut AppContext) -> ModelHandle<Self> {
        let history = BlocklistAIHistoryModel::handle(ctx);
        let model = ctx.add_singleton_model(|_| Self {
            child_session_by_conversation: HashMap::new(),
            event_consumers_by_session: HashMap::new(),
            page_root_conversation_id: None,
            page_anchor: None,
            explicitly_paged: false,
        });
        let model_for_history = model.clone();
        ctx.subscribe_to_model(&history, move |_, event, ctx| {
            let topology_changed = match event {
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
                | BlocklistAIHistoryEvent::SplitConversation { .. }
                | BlocklistAIHistoryEvent::RemoveConversation { .. }
                | BlocklistAIHistoryEvent::DeletedConversation { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                    ..
                } => true,
                BlocklistAIHistoryEvent::CreatedSubtask { .. }
                | BlocklistAIHistoryEvent::UpgradedTask { .. }
                | BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::ReassignedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
                | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
                | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
                | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
                | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
                | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => false,
            };

            if topology_changed {
                model_for_history.update(ctx, |model, ctx| model.topology_changed(ctx));
            }
        });
        model
    }

    /// Builds the current navigable tab tree for a selected conversation.
    pub(crate) fn snapshot(
        &self,
        selected_conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<TuiOrchestrationSnapshot> {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let root_conversation_id =
            orchestration_root_conversation_id(history, selected_conversation_id)?;
        let sessions = TuiSessions::as_ref(ctx);
        sessions.session_id_for_conversation(history, root_conversation_id)?;

        let spawn_order = descendant_conversation_ids_in_spawn_order(history, root_conversation_id)
            .into_iter()
            .filter(|conversation_id| {
                sessions
                    .session_id_for_conversation(history, *conversation_id)
                    .is_some()
            })
            .collect::<Vec<_>>();
        if spawn_order.is_empty() {
            return None;
        }
        let spawn_index_by_conversation = spawn_order
            .iter()
            .enumerate()
            .map(|(index, conversation_id)| (*conversation_id, index))
            .collect::<HashMap<_, _>>();

        let tabs = descendant_conversation_ids_in_pill_order(history, root_conversation_id)
            .into_iter()
            .filter_map(|conversation_id| {
                sessions.session_id_for_conversation(history, conversation_id)?;
                let conversation = history.conversation(&conversation_id)?;
                Some(TuiOrchestrationTab {
                    conversation_id,
                    label: conversation
                        .agent_name()
                        .filter(|name| !name.is_empty())
                        .unwrap_or("Agent")
                        .to_owned(),
                    spawn_index: *spawn_index_by_conversation.get(&conversation_id)?,
                })
            })
            .collect::<Vec<_>>();
        if tabs.is_empty() {
            return None;
        }

        let page_state_applies = self.page_root_conversation_id == Some(root_conversation_id);
        let page_anchor = page_state_applies
            .then_some(self.page_anchor)
            .flatten()
            .filter(|anchor| tabs.iter().any(|tab| tab.conversation_id == *anchor))
            .or_else(|| tabs.first().map(|tab| tab.conversation_id));
        Some(TuiOrchestrationSnapshot {
            root_conversation_id,
            selected_conversation_id,
            tabs,
            page_anchor,
            reveal_selected: !page_state_applies || !self.explicitly_paged,
        })
    }

    pub(crate) fn has_tabs(
        &self,
        selected_conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        self.snapshot(selected_conversation_id, ctx).is_some()
    }

    /// Stores an explicitly selected secondary page without switching sessions.
    pub(crate) fn set_explicit_page(
        &mut self,
        root_conversation_id: AIConversationId,
        page_anchor: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.page_root_conversation_id = Some(root_conversation_id);
        self.page_anchor = Some(page_anchor);
        self.explicitly_paged = true;
        self.emit_changed(ctx);
    }

    /// Focuses the retained session for a conversation and resumes automatic reveal.
    pub(crate) fn focus_conversation_session(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiSessionId> {
        let current_page_anchor = self
            .snapshot(conversation_id, ctx)
            .and_then(|snapshot| snapshot.page_anchor);
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let root_conversation_id = orchestration_root_conversation_id(history, conversation_id)?;
        let session_id =
            TuiSessions::as_ref(ctx).session_id_for_conversation(history, conversation_id)?;
        if self.page_root_conversation_id != Some(root_conversation_id) {
            // Capture the page before focus changes any ordering inputs. The
            // first switch in a tree must not replace the visible page with a
            // new fallback anchor.
            self.page_anchor = current_page_anchor;
        }
        self.page_root_conversation_id = Some(root_conversation_id);
        self.explicitly_paged = false;
        TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.focus_session(session_id, ctx);
        });
        self.emit_changed(ctx);
        Some(session_id)
    }

    fn topology_changed(&mut self, ctx: &mut ModelContext<Self>) {
        if self.page_root_conversation_id.is_some_and(|root| {
            orchestration_root_conversation_id(BlocklistAIHistoryModel::as_ref(ctx), root).is_none()
        }) {
            self.page_root_conversation_id = None;
            self.page_anchor = None;
            self.explicitly_paged = false;
        }
        self.emit_changed(ctx);
    }

    fn emit_changed(&self, ctx: &mut ModelContext<Self>) {
        ctx.emit(TuiOrchestrationEvent::TabBarChanged);
        ctx.notify();
    }


    /// Routes a `CreateAgent` request the same two ways as the GUI's
    /// per-mode dispatch, with unsupported modes resolving as clean per-child
    /// failures.
    pub(crate) fn dispatch_create_agent(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        working_directory: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        match request.execution_mode.clone() {
            StartAgentExecutionMode::Local {
                harness_type: None,
                model_id,
            } => self.begin_local_oz_child_launch(
                parent_session_id,
                request,
                model_id,
                working_directory,
                ctx,
            ),
            StartAgentExecutionMode::Local {
                harness_type: Some(harness_type),
                ..
            } => {
                // Local non-oz children are not supported outside of dogfood in the GUI,
                // and would be odd in the TUI. For now, we don't offer this option in the
                // orchestration card, so this should never be reached.
                self.fail_child_request(
                    &request,
                    format!(
                        "Local {harness_type} child agents aren't supported in the Warp TUI yet."
                    ),
                    ctx,
                );
            }
            StartAgentExecutionMode::Remote { .. } => {
                // TODO(code-1822): remote children need a TUI materializer;
                // the GUI's spawn path is coupled to ambient-agent panes.
                self.fail_child_request(
                    &request,
                    "Cloud child agents aren't supported in the Warp TUI yet.".to_string(),
                    ctx,
                );
            }
        }
    }

    /// Starts server-side task creation. The completion callback creates the
    /// TUI session only after the task has a stable run id.
    fn begin_local_oz_child_launch(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        model_id: Option<String>,
        working_directory: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let launch = prepare_local_oz_child_launch(
            &request.name,
            &request.prompt,
            request.parent_run_id.as_deref(),
            ctx,
        );
        ctx.spawn(launch, move |me, result, ctx| match result {
            Ok(prepared) => ctx.emit(TuiOrchestrationEvent::CreateLocalOzChildSession {
                parent_session_id,
                request: Box::new(request),
                model_id,
                working_directory,
                task_id: prepared.task_id,
                conversation_name: prepared.conversation_name,
            }),
            Err(error) => me.fail_child_request(
                &request,
                format!("Failed to create local child task: {error}"),
                ctx,
            ),
        });
    }

    /// Registers a materialized background session and child conversation for
    /// a prepared task, then sends the child's first prompt.
    pub(crate) fn register_local_oz_child_session(
        &mut self,
        child: MaterializedLocalOzChildSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let MaterializedLocalOzChildSession {
            parent_session_id,
            session_id,
            session_view,
            request,
            model_id,
            task_id,
            conversation_name,
        } = child;
        let child_surface_id = session_id.surface_id();

        let parent_surface_id = parent_session_id.surface_id();
        inherit_child_agent_settings(parent_surface_id, child_surface_id, ctx);
        apply_child_agent_model_override(child_surface_id, model_id.as_deref(), ctx);

        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                child_surface_id,
                conversation_name,
                request.parent_conversation_id,
                Some(Harness::Oz),
                ctx,
            );
            // Stamp the task id before completing the request so the
            // executor and the local task-status sync see it immediately.
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_task_id(task_id);
            }
            history.set_active_conversation_id(conversation_id, child_surface_id, ctx);
            history.record_new_conversation_request_complete(request.id, conversation_id, ctx);
            conversation_id
        });

        self.register_event_consumer(parent_session_id, request.parent_conversation_id, ctx);
        self.register_event_consumer(session_id, conversation_id, ctx);
        session_view.update(ctx, |view, ctx| {
            view.initialize_orchestrated_child_conversation(conversation_id, ctx);
        });

        let prompt = request.prompt;
        session_view.update(ctx, |view, ctx| {
            view.start_orchestrated_child(task_id, prompt, conversation_id, ctx);
        });

        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        self.emit_changed(ctx);
    }

    /// Tears down the background session of a child that failed at the
    /// launch stage (the executor's `CleanupFailedChildLaunch`).
    pub(crate) fn cleanup_failed_child(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_surface_id = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(conversation_id);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.delete_conversation(*conversation_id, terminal_surface_id, ctx);
        });
        if let Some(session_id) = self.child_session_by_conversation.remove(conversation_id) {
            ctx.emit(TuiOrchestrationEvent::RemoveChildSession(session_id));
        }
        ctx.notify();
    }

    /// Resolves a child request as failed without creating a TUI session.
    fn fail_child_request(
        &mut self,
        request: &StartAgentRequest,
        message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let request_id = request.id;
        log::warn!("Failing TUI child agent request: request_id={request_id:?}");
        let surface_id = EntityId::new();
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                surface_id,
                request.name.trim().to_owned(),
                request.parent_conversation_id,
                None,
                ctx,
            );
            history.update_conversation_status_with_error(
                surface_id,
                conversation_id,
                ConversationStatus::Error,
                Some(RenderableAIError::other(message, false)),
                ctx,
            );
            history.record_new_conversation_request_complete(request.id, conversation_id, ctx);
        });
    }

    fn register_event_consumer(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        register_agent_event_consumer(conversation_id, session_id.surface_id(), ctx);
        self.event_consumers_by_session
            .entry(session_id)
            .or_default()
            .insert(conversation_id);
    }

    pub(crate) fn handle_session_removed(
        &mut self,
        session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(conversation_ids) = self.event_consumers_by_session.remove(&session_id) {
            for conversation_id in conversation_ids {
                unregister_agent_event_consumer(conversation_id, session_id.surface_id(), ctx);
            }
        }
        self.child_session_by_conversation
            .retain(|_, child_session_id| *child_session_id != session_id);
        ctx.emit(TuiOrchestrationEvent::TabBarChanged);
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
