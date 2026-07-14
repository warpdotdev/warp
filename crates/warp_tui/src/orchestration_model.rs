//! [`TuiOrchestrationModel`]: the TUI's child-agent coordinator.
//!
//! The shared `StartAgentExecutor` (one per session surface) emits
//! `CreateAgent` and waits for a frontend to materialize the child. In the
//! GUI that materializer is `TerminalView` → `PaneGroup`'s hidden child
//! panes; in the TUI it is this singleton. It subscribes to every session
//! registered with [`TuiSessions`] (so children can orchestrate
//! grandchildren), spawns native Oz children into background sessions, and
//! tracks the session dimension of the orchestration tree — conversation
//! lineage itself stays in `BlocklistAIHistoryModel`.
//!
//! Native (Oz) local children run in background TUI sessions. Local
//! CLI-harness and remote child requests resolve with an explicit failure.

use std::collections::{HashMap, HashSet};

use warp::tui_export::{
    register_agent_event_consumer, unregister_agent_event_consumer, AIConversationId,
    AIExecutionProfilesModel, AgentConfigSnapshot, AmbientAgentTaskId, BlocklistAIHistoryModel,
    ConversationStatus, Harness, LLMId, LLMPreferences, RenderableAIError, ServerApiProvider,
    StartAgentExecutionMode, StartAgentExecutorEvent, StartAgentRequest,
};
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ReadModel as _};

use crate::session::create_local_terminal_session;
use crate::session_registry::{TuiSessionId, TuiSessions, TuiSessionsEvent};

/// The TUI's child-agent coordinator singleton. See the module docs.
pub(crate) struct TuiOrchestrationModel {
    /// Session hosting each live child conversation. The session dimension
    /// only — conversation lineage is read from `BlocklistAIHistoryModel`
    /// (`children_by_parent` / `parent_conversation_id`), never mirrored.
    child_session_by_conversation: HashMap<AIConversationId, TuiSessionId>,
    /// Sessions that have dispatched at least one child agent.
    parent_sessions: HashSet<TuiSessionId>,
}

impl Entity for TuiOrchestrationModel {
    type Event = ();
}

impl SingletonEntity for TuiOrchestrationModel {}

impl TuiOrchestrationModel {
    /// Registers the singleton and subscribes it to [`TuiSessions`] so every
    /// session's `StartAgentExecutor` gets wired as sessions register. Must
    /// run before any session is created.
    pub(crate) fn register(ctx: &mut AppContext) -> ModelHandle<Self> {
        let sessions = TuiSessions::handle(ctx);
        ctx.add_singleton_model(|ctx| {
            ctx.subscribe_to_model(&sessions, Self::handle_sessions_event);
            Self {
                child_session_by_conversation: HashMap::new(),
                parent_sessions: HashSet::new(),
            }
        })
    }

    /// Wires a newly registered session's `StartAgentExecutor` to this model.
    fn handle_sessions_event(
        &mut self,
        sessions: ModelHandle<TuiSessions>,
        event: &TuiSessionsEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let TuiSessionsEvent::SessionAdded(session_id) = event else {
            return;
        };
        let session_id = *session_id;
        let Some(session_view) = ctx.read_model(&sessions, |sessions, _| {
            sessions
                .session(session_id)
                .map(|session| session.view().clone())
        }) else {
            return;
        };
        let action_model = session_view.as_ref(ctx).ai_action_model().clone();
        let executor = ctx.read_model(&action_model, |model, app| model.start_agent_executor(app));
        ctx.subscribe_to_model(&executor, move |me, _, event, ctx| {
            me.handle_executor_event(session_id, event, ctx);
        });
    }

    fn handle_executor_event(
        &mut self,
        parent_session_id: TuiSessionId,
        event: &StartAgentExecutorEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            StartAgentExecutorEvent::CreateAgent(request) => {
                self.dispatch_create_agent(parent_session_id, (**request).clone(), ctx);
            }
            StartAgentExecutorEvent::CleanupFailedChildLaunch { conversation_id } => {
                self.cleanup_failed_child(conversation_id, ctx);
            }
        }
    }

    /// Routes a `CreateAgent` request the same two ways as the GUI's
    /// per-mode dispatch, with unsupported modes resolving as clean per-child
    /// failures.
    fn dispatch_create_agent(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        ctx: &mut ModelContext<Self>,
    ) {
        // Dispatching a child makes the parent an orchestrator: register it
        // as a streamer consumer so its SSE stream (child lifecycle + inbox
        // messages) opens. The GUI gets this from the agent view's
        // `ActiveAgentViewsModel` bridge, which the TUI does not have.
        register_agent_event_consumer(
            request.parent_conversation_id,
            parent_session_id.surface_id(),
            ctx,
        );
        match request.execution_mode.clone() {
            StartAgentExecutionMode::Local {
                harness_type: None,
                model_id,
            } => self.launch_native_child(parent_session_id, request, model_id, ctx),
            StartAgentExecutionMode::Local {
                harness_type: Some(harness_type),
                ..
            } => {
                // TODO(code-1822): support local CLI-harness children by
                // reusing the frontend-neutral
                // `prepare_local_harness_child_launch` command builder.
                fail_child_request(
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
                fail_child_request(
                    &request,
                    "Cloud child agents aren't supported in the Warp TUI yet.".to_string(),
                    ctx,
                );
            }
        }
    }

    /// Native (Oz) local child: eagerly creates the server task row (which
    /// activates messaging/lifecycle for the child), then materializes the
    /// background session, mirroring the GUI's
    /// `launch_local_no_harness_child`.
    fn launch_native_child(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        model_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let agent_name = Some(request.name.trim().to_owned()).filter(|name| !name.is_empty());
        let prompt = request.prompt.clone();
        let parent_run_id = request.parent_run_id.clone();
        ctx.spawn(
            async move {
                ai_client
                    .create_agent_task(
                        prompt,
                        None,
                        parent_run_id,
                        Some(AgentConfigSnapshot {
                            name: agent_name,
                            ..Default::default()
                        }),
                    )
                    .await
            },
            move |me, result, ctx| match result {
                Ok(task_id) => me.materialize_native_child(
                    parent_session_id,
                    &request,
                    model_id.as_deref(),
                    task_id,
                    ctx,
                ),
                Err(error) => fail_child_request(
                    &request,
                    format!("Failed to create local child task: {error}"),
                    ctx,
                ),
            },
        );
    }

    /// Creates the background session, links the child conversation to the
    /// parent, echoes it back to the executor, and dispatches the prompt.
    fn materialize_native_child(
        &mut self,
        parent_session_id: TuiSessionId,
        request: &StartAgentRequest,
        model_id: Option<&str>,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let sessions = TuiSessions::handle(ctx);
        let window_id = ctx.read_model(&sessions, |sessions, ctx| {
            sessions
                .session(parent_session_id)
                .expect("the dispatching parent session must remain registered")
                .view()
                .window_id(ctx)
        });
        let (session_id, session_view) =
            create_local_terminal_session(&sessions, window_id, false, ctx);
        let child_surface_id = session_id.surface_id();

        // Inherit the parent's execution profile and base model, then apply
        // the run-wide model override — the TUI counterpart of the GUI's
        // `propagate_parent_agent_settings` + `apply_child_model_id_override`.
        let parent_surface_id = parent_session_id.surface_id();
        let parent_profile_id = *AIExecutionProfilesModel::as_ref(ctx)
            .active_profile(Some(parent_surface_id), ctx)
            .id();
        AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
            profiles.set_active_profile(child_surface_id, parent_profile_id, ctx);
        });
        let parent_base_model_id = LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, Some(parent_surface_id))
            .id
            .clone();
        LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
            prefs.update_preferred_agent_mode_llm(&parent_base_model_id, child_surface_id, ctx);
        });
        if let Some(model_id) = model_id.map(str::trim).filter(|id| !id.is_empty()) {
            let llm_id: LLMId = model_id.into();
            LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                prefs.update_preferred_agent_mode_llm(&llm_id, child_surface_id, ctx);
            });
        }

        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                child_surface_id,
                request.name.clone(),
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

        // Register the child as a streamer consumer so its own inbox stream
        // (parent→child messages, wake events) opens.
        register_agent_event_consumer(conversation_id, child_surface_id, ctx);

        let ai_controller = session_view.as_ref(ctx).ai_controller().clone();
        let prompt = request.prompt.clone();
        ai_controller.update(ctx, |controller, ctx| {
            controller.set_ambient_agent_task_id(Some(task_id), ctx);
            controller.send_agent_query_in_conversation(prompt, conversation_id, ctx);
        });

        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        self.parent_sessions.insert(parent_session_id);
        ctx.notify();
    }

    /// Tears down the background session of a child that failed at the
    /// launch stage (the executor's `CleanupFailedChildLaunch`).
    fn cleanup_failed_child(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session_id) = self.child_session_by_conversation.remove(conversation_id) else {
            return;
        };
        unregister_agent_event_consumer(*conversation_id, session_id.surface_id(), ctx);
        TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.remove_session(session_id, ctx);
        });
        ctx.notify();
    }
}

/// Resolves a child request as failed without materializing a session:
/// creates the child conversation on a synthetic surface, marks it errored,
/// then echoes it to the executor — which completes the pending slot with
/// the error message instead of hanging into the spawn timeout.
fn fail_child_request(
    request: &StartAgentRequest,
    message: String,
    ctx: &mut ModelContext<TuiOrchestrationModel>,
) {
    log::warn!(
        "Failing TUI child agent request '{}': {message}",
        request.name
    );
    let surface_id = EntityId::new();
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
        let conversation_id = history.start_new_child_conversation(
            surface_id,
            request.name.clone(),
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

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
