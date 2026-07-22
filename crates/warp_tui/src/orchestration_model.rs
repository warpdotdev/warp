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
//! Native local and remote Oz children run in retained TUI sessions. Local
//! CLI-harness requests resolve with an explicit failure.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, CloudAgentStartupIssue,
    ConversationStatus, Harness, OrchestrationEventStreamer, OrchestrationEventStreamerEvent,
    PreparedRemoteChildLaunch, RemoteChildLaunchConfig, RenderableAIError, ServerApiProvider,
    StartAgentExecutionMode, StartAgentRequest, apply_child_agent_model_override,
    classify_cloud_agent_startup_error, descendant_conversations_in_pill_order,
    inherit_child_agent_settings, orchestration_root_conversation_id, oz_run_url,
    prepare_local_oz_child_launch, prepare_remote_child_launch, register_agent_event_consumer,
    unregister_agent_event_consumer,
};
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::cloud_run::TuiCloudRunState;
use crate::session_registry::{RemoteChildSession, TuiSessionId, TuiSessions};
use crate::tab_bar::TuiTabBarPagingState;
use crate::terminal_session_view::TuiTerminalSessionView;

/// One navigable child conversation in an orchestration snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationChild {
    pub(crate) conversation_id: AIConversationId,
    pub(crate) label: String,
    pub(crate) spawn_index: usize,
    pub(crate) status: ConversationStatus,
}

/// Live semantic state for the orchestration tab bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationSnapshot {
    pub(crate) root_conversation_id: AIConversationId,
    pub(crate) selected_conversation_id: AIConversationId,
    pub(crate) children: Vec<TuiOrchestrationChild>,
    /// Stable child ID used to resolve the page start at the current width.
    pub(crate) page_anchor: Option<AIConversationId>,
    /// Whether the tab bar may override the anchor to reveal the selection.
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
    /// Paging intent shared by the per-session tab-bar views.
    tab_bar_paging: TuiTabBarPagingState<AIConversationId>,
}
#[allow(clippy::enum_variant_names)]
pub(crate) enum TuiOrchestrationEvent {
    CreateLocalChildSession {
        parent_session_id: TuiSessionId,
        request: Box<StartAgentRequest>,
        model_id: Option<String>,
        working_directory: Option<PathBuf>,
        task_id: warp::tui_export::AmbientAgentTaskId,
        conversation_name: String,
    },
    CreateRemoteChildSession {
        parent_session_id: TuiSessionId,
        request: Box<StartAgentRequest>,
        prepared: Box<PreparedRemoteChildLaunch>,
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
        let history = BlocklistAIHistoryModel::handle(ctx);
        let streamer = OrchestrationEventStreamer::handle(ctx);
        let model = ctx.add_singleton_model(|_| Self {
            child_session_by_conversation: HashMap::new(),
            event_consumers_by_session: HashMap::new(),
            tab_bar_paging: TuiTabBarPagingState::default(),
        });
        let model_for_history = model.clone();
        ctx.subscribe_to_model(&history, move |_, event, ctx| {
            let topology_changed = match event {
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::AppendedExchange { .. }
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
        let model_for_streamer = model.clone();
        ctx.subscribe_to_model(&streamer, move |_, event, ctx| {
            model_for_streamer.update(ctx, |model, ctx| {
                model.handle_streamer_event(event, ctx);
            });
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
        let session_ids_by_conversation = sessions.session_ids_by_conversation(history);
        session_ids_by_conversation.get(&root_conversation_id)?;

        let children = descendant_conversations_in_pill_order(history, root_conversation_id)
            .into_iter()
            .filter_map(|descendant| {
                let conversation_id = descendant.conversation_id;
                session_ids_by_conversation.get(&conversation_id)?;
                let conversation = history.conversation(&conversation_id)?;
                Some(TuiOrchestrationChild {
                    conversation_id,
                    label: conversation
                        .agent_name()
                        .filter(|name| !name.is_empty())
                        .unwrap_or("Agent")
                        .to_owned(),
                    spawn_index: descendant.spawn_index,
                    status: conversation.status().clone(),
                })
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            return None;
        }

        let resolved_page = self.tab_bar_paging.resolve(
            children.first().map(|child| child.conversation_id),
            |anchor| {
                children
                    .iter()
                    .any(|child| child.conversation_id == *anchor)
            },
        );
        Some(TuiOrchestrationSnapshot {
            root_conversation_id,
            selected_conversation_id,
            children,
            page_anchor: resolved_page.page_anchor,
            reveal_selected: resolved_page.reveal_selected,
        })
    }

    /// Stores an explicitly selected secondary page without switching sessions.
    pub(crate) fn set_explicit_page(
        &mut self,
        page_anchor: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.tab_bar_paging.set_explicit_anchor(page_anchor);
        ctx.notify();
    }

    /// Focuses the retained session for a conversation and resumes automatic reveal.
    pub(crate) fn focus_conversation_session(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiSessionId> {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        orchestration_root_conversation_id(history, conversation_id)?;
        let session_id = *TuiSessions::as_ref(ctx)
            .session_ids_by_conversation(history)
            .get(&conversation_id)?;
        self.tab_bar_paging.clear_explicit_anchor();
        TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.focus_session(session_id, ctx);
        });
        Some(session_id)
    }

    fn topology_changed(&mut self, ctx: &mut ModelContext<Self>) {
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
            StartAgentExecutionMode::Remote {
                environment_id,
                skill_references,
                model_id,
                computer_use_enabled,
                worker_host,
                harness_type,
                title,
                auth_secret_name,
                runner_id,
                agent_identity_uid,
            } => {
                self.register_event_consumer(
                    parent_session_id,
                    request.parent_conversation_id,
                    ctx,
                );
                self.begin_remote_child_launch(
                    parent_session_id,
                    request,
                    RemoteChildLaunchConfig {
                        environment_id,
                        skill_references,
                        model_id,
                        computer_use_enabled,
                        worker_host,
                        harness_type,
                        title,
                        auth_secret_name,
                        runner_id,
                        agent_identity_uid,
                    },
                    ctx,
                );
            }
        }
    }

    fn begin_remote_child_launch(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        config: RemoteChildLaunchConfig,
        ctx: &mut ModelContext<Self>,
    ) {
        let prepared = match prepare_remote_child_launch(&request, config, ctx) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.fail_child_request(&request, error.user_message(), ctx);
                return;
            }
        };
        ctx.emit(TuiOrchestrationEvent::CreateRemoteChildSession {
            parent_session_id,
            request: Box::new(request),
            prepared: Box::new(prepared),
        });
    }

    /// Registers the remote child's conversation state, then starts its server-side launch.
    pub(crate) fn register_remote_child_session(
        &mut self,
        child: RemoteChildSession,
        request: StartAgentRequest,
        prepared: PreparedRemoteChildLaunch,
        ctx: &mut ModelContext<Self>,
    ) {
        let PreparedRemoteChildLaunch {
            display_name,
            orchestration_harness,
            spawn_request,
        } = prepared;
        let conversation_id = self.initialize_remote_child_session(
            &child,
            &request,
            display_name,
            orchestration_harness,
            ctx,
        );
        let RemoteChildSession {
            session_id,
            cloud_run_state,
        } = child;
        let surface_id = session_id.surface_id();
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let cloud_run_state_for_launch = cloud_run_state.clone();
        ctx.spawn(
            async move { ai_client.spawn_agent(spawn_request).await },
            move |me, result, ctx| {
                let result = result.map_err(|error| classify_cloud_agent_startup_error(&error));
                me.finish_remote_child_launch(
                    conversation_id,
                    surface_id,
                    cloud_run_state_for_launch,
                    result,
                    ctx,
                );
            },
        );
        ctx.notify();
    }

    fn initialize_remote_child_session(
        &mut self,
        child: &RemoteChildSession,
        request: &StartAgentRequest,
        display_name: String,
        orchestration_harness: Harness,
        ctx: &mut ModelContext<Self>,
    ) -> AIConversationId {
        let surface_id = child.session_id.surface_id();
        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                surface_id,
                display_name,
                request.parent_conversation_id,
                Some(orchestration_harness),
                ctx,
            );
            history.mark_conversation_as_remote_child(conversation_id, ctx);
            history.set_active_conversation_id(conversation_id, surface_id, ctx);
            history.record_new_conversation_request_complete(request.id, conversation_id, ctx);
            conversation_id
        });
        child.cloud_run_state.update(ctx, |state, ctx| {
            state.set_conversation_id(conversation_id, ctx);
        });
        self.child_session_by_conversation
            .insert(conversation_id, child.session_id);
        conversation_id
    }

    fn finish_remote_child_launch(
        &mut self,
        conversation_id: AIConversationId,
        child_surface_id: EntityId,
        cloud_run_state: ModelHandle<TuiCloudRunState>,
        result: Result<warp::tui_export::SpawnAgentResponse, CloudAgentStartupIssue>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(response) => {
                let run_url = oz_run_url(&response.run_id);
                cloud_run_state.update(ctx, |state, ctx| {
                    state.set_spawned(response.task_id, response.run_id.clone(), run_url, ctx);
                });
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.assign_run_id_for_conversation(
                        conversation_id,
                        response.run_id,
                        Some(response.task_id),
                        child_surface_id,
                        ctx,
                    );
                });
            }
            Err(CloudAgentStartupIssue::Blocked(blocker)) => {
                let message = blocker.message().to_string();
                cloud_run_state.update(ctx, |state, ctx| {
                    state.set_blocked(blocker, ctx);
                });
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status(
                        child_surface_id,
                        conversation_id,
                        ConversationStatus::Blocked {
                            blocked_action: message,
                        },
                        ctx,
                    );
                });
            }
            Err(CloudAgentStartupIssue::Failed(failure)) => {
                let message = failure.message().to_string();
                cloud_run_state.update(ctx, |state, ctx| {
                    state.set_failed(failure, ctx);
                });
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status_with_error(
                        child_surface_id,
                        conversation_id,
                        ConversationStatus::Error,
                        Some(RenderableAIError::other(message, false)),
                        ctx,
                    );
                });
            }
        }
    }

    fn handle_streamer_event(
        &mut self,
        event: &OrchestrationEventStreamerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let OrchestrationEventStreamerEvent::WatchedRunStatusChanged {
            owner_conversation_id,
            run_id,
            status,
        } = event
        else {
            return;
        };
        let child = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation_id) = history.conversation_id_for_agent_id(run_id) else {
                return;
            };
            let Some(conversation) = history.conversation(&conversation_id) else {
                return;
            };
            let parent_matches = history
                .resolved_parent_conversation_id_for_conversation(conversation)
                == Some(*owner_conversation_id);
            if !conversation.is_remote_child() || !parent_matches {
                return;
            }
            let Some(surface_id) = history.terminal_surface_id_for_conversation(&conversation_id)
            else {
                return;
            };
            if TuiSessions::as_ref(ctx)
                .session_id_for_surface(surface_id)
                .is_none()
            {
                return;
            }
            (conversation_id, surface_id)
        };
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.update_conversation_status(child.1, child.0, status.clone(), ctx);
        });
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
            Ok(prepared) => ctx.emit(TuiOrchestrationEvent::CreateLocalChildSession {
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
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
