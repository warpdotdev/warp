//! [`TuiOrchestrationModel`]: the TUI's child-agent coordinator.
//!
//! The shared `StartAgentExecutor` (one per session surface) emits
//! `CreateAgent` and waits for a frontend to materialize the child. In the
//! GUI that materializer is `TerminalView` → `PaneGroup`'s hidden child
//! panes; in the TUI, [`crate::session_registry::TuiSessions`] owns
//! materialization. This singleton prepares native Oz children, requests
//! background session lifecycle changes, and tracks the session dimension of
//! the orchestration tree — conversation lineage itself stays in
//! `BlocklistAIHistoryModel`.
//!
//! Native (Oz) local children run in background TUI sessions. Local
//! CLI-harness and remote child requests resolve with an explicit failure.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp::tui_export::{
    apply_child_agent_model_override, inherit_child_agent_settings, prepare_local_oz_child_launch,
    register_agent_event_consumer, unregister_agent_event_consumer, AIConversationId,
    BlocklistAIHistoryModel, ConversationStatus, Harness, RenderableAIError,
    StartAgentExecutionMode, StartAgentRequest,
};
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::session_registry::TuiSessionId;
use crate::terminal_session_view::TuiTerminalSessionView;

/// The TUI's child-agent coordinator singleton. See the module docs.
pub(crate) struct TuiOrchestrationModel {
    /// Session hosting each live child conversation. The session dimension
    /// only — conversation lineage is read from `BlocklistAIHistoryModel`
    /// (`children_by_parent` / `parent_conversation_id`), never mirrored.
    child_session_by_conversation: HashMap<AIConversationId, TuiSessionId>,
    /// Conversations whose event streams are consumed by each live session.
    event_consumers_by_session: HashMap<TuiSessionId, HashSet<AIConversationId>>,
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
        ctx.add_singleton_model(|_| Self {
            child_session_by_conversation: HashMap::new(),
            event_consumers_by_session: HashMap::new(),
        })
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

        let prompt = request.prompt;
        session_view.update(ctx, |view, ctx| {
            view.start_orchestrated_child(task_id, prompt, conversation_id, ctx);
        });

        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        ctx.notify();
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
    }
}

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
