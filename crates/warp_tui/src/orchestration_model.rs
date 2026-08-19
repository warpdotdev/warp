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
    AIConversation, AIConversationId, AgentRunDisplayStatus, AmbientAgentTaskId,
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, CloudAgentStartupIssue,
    CloudConversationData, ConversationStatus, Harness, LoadedSubtreeRollup,
    OrchestrationEventStreamer, OrchestrationEventStreamerEvent, PreparedRemoteChildLaunch,
    RemoteChildLaunchConfig, RenderableAIError, ServerApiProvider, StartAgentExecutionMode,
    StartAgentRequest, aggregated_orchestrator_status, apply_child_agent_model_override,
    child_conversations_in_pill_order, classify_cloud_agent_startup_error,
    descendant_conversation_ids_in_spawn_order, descendant_conversations_in_pill_order,
    inherit_child_agent_settings, loaded_subtree_rollup, orchestration_root_conversation_id,
    oz_run_url, prepare_local_oz_child_launch, prepare_remote_child_launch,
    register_agent_event_consumer, unregister_agent_event_consumer,
};
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::cloud_run::TuiCloudRunState;
use crate::session_registry::{RemoteChildSession, TuiSessionId, TuiSessions};
use crate::tab_bar::{TuiTabBarNavigationDirection, TuiTabBarPagingState};
use crate::terminal_session_view::TuiTerminalSessionView;

/// The main-tab label used for the orchestration tree root.
pub(crate) const ORCHESTRATOR_TAB_LABEL: &str = "orchestrator";

/// One navigable child conversation in an orchestration snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationChild {
    pub(crate) conversation_id: AIConversationId,
    pub(crate) label: String,
    pub(crate) spawn_index: usize,
    pub(crate) status: ConversationStatus,
    /// Rolled-up loaded subtree beneath this child. `Some` only for a
    /// **group** child (at least one loaded descendant) while the
    /// multi-level presentation is enabled.
    pub(crate) subtree_rollup: Option<LoadedSubtreeRollup>,
}

/// One selectable ancestor chip shown while the bar is drilled below the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationBreadcrumb {
    pub(crate) conversation_id: AIConversationId,
    pub(crate) label: String,
}

/// Live semantic state for the orchestration tab bar.
///
/// With `FeatureFlag::MultiLevelOrchestration` enabled the snapshot holds one
/// **level** of the tree — the drill-down anchor plus its direct children —
/// mirroring the GUI pill bar's `drill_down_anchor_id` rule. With the flag
/// disabled it keeps the historical flat projection: the anchor is the tree
/// root and `children` lists every navigable descendant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationSnapshot {
    pub(crate) root_conversation_id: AIConversationId,
    /// The conversation whose level the bar renders. Equals the root at
    /// orchestration depth 1 and whenever multi-level is disabled.
    pub(crate) anchor_conversation_id: AIConversationId,
    /// Main-tab label: the anchor's agent name, or [`ORCHESTRATOR_TAB_LABEL`]
    /// when the anchor is the tree root.
    pub(crate) anchor_label: String,
    /// Aggregated status of the anchor's subtree, shown as the main tab's
    /// leading glyph. `None` while multi-level is disabled.
    pub(crate) anchor_status: Option<ConversationStatus>,
    /// Whether the anchor maps to a retained, focusable session. A leaf's
    /// parent can be loaded but sessionless (e.g. a restored non-Oz child);
    /// its tab still frames the level but cannot be selected.
    pub(crate) anchor_navigable: bool,
    /// Ancestor chips rendered before the anchor: the tree root, plus the
    /// anchor's direct parent when the anchor sits 2+ levels below the root
    /// and maps to a retained session.
    pub(crate) breadcrumbs: Vec<TuiOrchestrationBreadcrumb>,
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
    /// Paging intent shared by the per-session tab-bar views, tracked per
    /// rendered level (keyed by the level's anchor conversation) so paging
    /// within a drilled-in level does not disturb any other level's page.
    tab_bar_paging_by_anchor: HashMap<AIConversationId, TuiTabBarPagingState<AIConversationId>>,
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
    KillLocalChildSession {
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
    },
    RemoveChildSession(TuiSessionId),
    /// Materialize a restored local Oz child on a fresh background terminal
    /// session hosted by `root_session_id`, without relaunching it.
    RestoreLocalChildSession {
        root_session_id: TuiSessionId,
        conversation: Box<AIConversation>,
    },
    /// Materialize a restored remote/cloud child on a fresh lightweight cloud
    /// session hosted by `root_session_id`, from its persisted identity.
    RestoreRemoteChildSession {
        root_session_id: TuiSessionId,
        conversation: Box<AIConversation>,
        task_id: AmbientAgentTaskId,
        run_id: String,
    },
    RestoredRemoteChildStatusUpdated {
        conversation_id: AIConversationId,
    },
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
            tab_bar_paging_by_anchor: HashMap::new(),
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

    /// Builds the current navigable tab level for a selected conversation.
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

        let multi_level = FeatureFlag::MultiLevelOrchestration.is_enabled();
        let anchor_conversation_id = if multi_level {
            drill_down_anchor_id(
                history,
                &session_ids_by_conversation,
                selected_conversation_id,
                root_conversation_id,
            )
        } else {
            root_conversation_id
        };

        // One level (the anchor's DIRECT children) while multi-level is
        // enabled; the historical flat all-descendants projection otherwise.
        let ordered_children = if multi_level {
            child_conversations_in_pill_order(history, anchor_conversation_id)
        } else {
            descendant_conversations_in_pill_order(history, root_conversation_id)
        };
        let children = ordered_children
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
                    subtree_rollup: multi_level
                        .then(|| loaded_subtree_rollup(history, conversation_id))
                        .flatten(),
                })
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            return None;
        }

        let anchor_label = if anchor_conversation_id == root_conversation_id {
            ORCHESTRATOR_TAB_LABEL.to_owned()
        } else {
            conversation_agent_label(history, anchor_conversation_id)
        };
        let anchor_status =
            multi_level.then(|| aggregated_orchestrator_status(history, anchor_conversation_id));
        let anchor_navigable = session_ids_by_conversation.contains_key(&anchor_conversation_id);
        let breadcrumbs = if multi_level {
            breadcrumbs_for_anchor(
                history,
                &session_ids_by_conversation,
                anchor_conversation_id,
                root_conversation_id,
            )
        } else {
            Vec::new()
        };

        let resolved_page = self
            .tab_bar_paging_by_anchor
            .get(&anchor_conversation_id)
            .cloned()
            .unwrap_or_default()
            .resolve(children.first().map(|child| child.conversation_id), {
                let children = &children;
                move |anchor| {
                    children
                        .iter()
                        .any(|child| child.conversation_id == *anchor)
                }
            });
        Some(TuiOrchestrationSnapshot {
            root_conversation_id,
            anchor_conversation_id,
            anchor_label,
            anchor_status,
            anchor_navigable,
            breadcrumbs,
            selected_conversation_id,
            children,
            page_anchor: resolved_page.page_anchor,
            reveal_selected: resolved_page.reveal_selected,
        })
    }

    /// Stores an explicitly selected secondary page for one rendered level
    /// without switching sessions.
    pub(crate) fn set_explicit_page(
        &mut self,
        level_anchor: AIConversationId,
        page_anchor: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.tab_bar_paging_by_anchor
            .entry(level_anchor)
            .or_default()
            .set_explicit_anchor(page_anchor);
        ctx.notify();
    }

    /// Resolves the adjacent conversation in the whole orchestration tree:
    /// the root followed by every navigable descendant in pill order,
    /// wrapping at either end. This is the GUI's keyboard-cycling order
    /// (`adjacent_orchestration_child_conversation_id`) restricted to
    /// conversations with retained TUI sessions.
    pub(crate) fn adjacent_tree_conversation(
        &self,
        selected_conversation_id: AIConversationId,
        direction: TuiTabBarNavigationDirection,
        ctx: &AppContext,
    ) -> Option<AIConversationId> {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let root_conversation_id =
            orchestration_root_conversation_id(history, selected_conversation_id)?;
        let session_ids_by_conversation =
            TuiSessions::as_ref(ctx).session_ids_by_conversation(history);
        let order = std::iter::once(root_conversation_id)
            .chain(
                descendant_conversations_in_pill_order(history, root_conversation_id)
                    .into_iter()
                    .map(|descendant| descendant.conversation_id),
            )
            .filter(|conversation_id| session_ids_by_conversation.contains_key(conversation_id))
            .collect::<Vec<_>>();
        let selected_index = order
            .iter()
            .position(|conversation_id| *conversation_id == selected_conversation_id)?;
        let target_index = match direction {
            TuiTabBarNavigationDirection::Previous => {
                selected_index.checked_sub(1).unwrap_or(order.len() - 1)
            }
            TuiTabBarNavigationDirection::Next => (selected_index + 1) % order.len(),
        };
        order.get(target_index).copied()
    }

    /// Fetches the authoritative lifecycle for a restored remote child once.
    /// The completion validates the retained session and task identity, so a
    /// late response cannot update a child whose restored tree was replaced.
    fn fetch_restored_remote_child_status(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_client.get_ambient_agent_task(&task_id).await },
            move |me, result, ctx| match result {
                Ok(task) => {
                    let status =
                        AgentRunDisplayStatus::from_task(&task, ctx).to_conversation_status();
                    me.apply_restored_remote_child_status(
                        session_id,
                        conversation_id,
                        task_id,
                        status,
                        ctx,
                    );
                }
                Err(error) => {
                    log::warn!(
                        "TUI restore: failed to fetch remote child status for \
                         {conversation_id:?}: {error:#}"
                    );
                }
            },
        );
    }

    fn apply_restored_remote_child_status(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        status: ConversationStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.child_session_by_conversation.get(&conversation_id) != Some(&session_id)
            || TuiSessions::as_ref(ctx).session(session_id).is_none()
        {
            return;
        }
        let should_update = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let Some(conversation) = history.conversation(&conversation_id) else {
                return;
            };
            if !conversation.is_remote_child() || conversation.task_id() != Some(task_id) {
                return;
            }
            conversation.status() != &status
        };
        if should_update {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.update_conversation_status(
                    session_id.surface_id(),
                    conversation_id,
                    status,
                    ctx,
                );
            });
            ctx.emit(TuiOrchestrationEvent::RestoredRemoteChildStatusUpdated { conversation_id });
        }
    }

    /// Focuses the retained session for a conversation and resumes automatic
    /// reveal on the level the bar will anchor to for that conversation.
    pub(crate) fn focus_conversation_session(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiSessionId> {
        let (session_id, level_anchor) = {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let root_conversation_id =
                orchestration_root_conversation_id(history, conversation_id)?;
            let session_ids_by_conversation =
                TuiSessions::as_ref(ctx).session_ids_by_conversation(history);
            let session_id = *session_ids_by_conversation.get(&conversation_id)?;
            let level_anchor = if FeatureFlag::MultiLevelOrchestration.is_enabled() {
                drill_down_anchor_id(
                    history,
                    &session_ids_by_conversation,
                    conversation_id,
                    root_conversation_id,
                )
            } else {
                root_conversation_id
            };
            (session_id, level_anchor)
        };
        self.tab_bar_paging_by_anchor.remove(&level_anchor);
        TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.focus_session(session_id, ctx);
        });
        Some(session_id)
    }

    fn topology_changed(&mut self, ctx: &mut ModelContext<Self>) {
        // Drop explicit page state for levels whose anchor no longer exists.
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        self.tab_bar_paging_by_anchor
            .retain(|anchor, _| history.conversation(anchor).is_some());
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
                        "Local {harness_type} child agents aren't supported in Warp Agent CLI yet."
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
                        working_dir: working_directory.unwrap_or_default(),
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
                true,
                ctx,
            );
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
                        Some(RenderableAIError::CloudStartupFailed(message)),
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
                false,
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

    /// Restores retained TUI sessions for every supported, locally-known
    /// descendant of a just-restored parent conversation, so restored children
    /// appear in the orchestration tab bar. Descendants are walked in recursive
    /// spawn order; each conversation keeps its actual parent linkage in
    /// history, while all new child sessions are hosted by the root session's
    /// window. Materialization is idempotent: a descendant that already has a
    /// live or previously-restored session is skipped, and a child failure is
    /// isolated so the rest of the tree still restores.
    pub(crate) fn restore_descendant_sessions(
        &mut self,
        parent_conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            parent_conversation_id,
        );
        if descendant_ids.is_empty() {
            return;
        }
        // The root parent remains responsible for descendant status watching.
        self.register_event_consumer(root_session_id, parent_conversation_id, ctx);
        for descendant_id in descendant_ids {
            self.restore_descendant_child(
                parent_conversation_id,
                descendant_id,
                root_session_id,
                ctx,
            );
        }
        ctx.notify();
    }

    /// Removes the retained child-session projections of a previously restored
    /// tree when a different parent replaces it. Only the TUI projections and
    /// their event-consumer registrations are removed; the underlying history
    /// records are preserved and no local process or cloud task is cancelled or
    /// deleted.
    pub(crate) fn discard_restored_descendant_sessions(
        &mut self,
        previous_parent_conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            previous_parent_conversation_id,
        );
        for descendant_id in descendant_ids {
            if let Some(session_id) = self.child_session_by_conversation.remove(&descendant_id) {
                // `RemoveChildSession` drops only the retained session and its
                // event consumers (via `handle_session_removed`); it never
                // deletes the conversation or cancels the child's execution.
                ctx.emit(TuiOrchestrationEvent::RemoveChildSession(session_id));
            }
        }
        self.unregister_event_consumer(root_session_id, previous_parent_conversation_id, ctx);
        ctx.notify();
    }

    fn restore_descendant_child(
        &mut self,
        root_parent_conversation_id: AIConversationId,
        conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.is_child_already_materialized(conversation_id, ctx) {
            return;
        }
        // Prefer the already-hydrated conversation (orchestration children are
        // eagerly hydrated at startup); fall back to the shared loader for an
        // indexed-but-not-loaded descendant. Both flow into the same
        // materializer.
        match BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .cloned()
        {
            Some(conversation) => {
                self.emit_restore_child_session(conversation, root_session_id, ctx);
            }
            None => self.load_and_restore_descendant_child(
                root_parent_conversation_id,
                conversation_id,
                root_session_id,
                ctx,
            ),
        }
    }

    fn is_child_already_materialized(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        if self
            .child_session_by_conversation
            .contains_key(&conversation_id)
        {
            return true;
        }
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        TuiSessions::as_ref(ctx)
            .session_ids_by_conversation(history)
            .contains_key(&conversation_id)
    }

    /// Loads an indexed-but-not-hydrated descendant, then materializes it. The
    /// completion is guarded so a stale load cannot attach a child to a session
    /// that is gone, to an already-materialized child, or to a tree that a
    /// different parent has since replaced.
    fn load_and_restore_descendant_child(
        &mut self,
        root_parent_conversation_id: AIConversationId,
        conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let future = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.load_conversation_data(conversation_id, ctx)
        });
        ctx.spawn(future, move |me, result, ctx| {
            if TuiSessions::as_ref(ctx).session(root_session_id).is_none()
                || me.is_child_already_materialized(conversation_id, ctx)
                || !descendant_conversation_ids_in_spawn_order(
                    BlocklistAIHistoryModel::as_ref(ctx),
                    root_parent_conversation_id,
                )
                .contains(&conversation_id)
            {
                return;
            }
            match result {
                Some(CloudConversationData::Oz(conversation)) => {
                    me.emit_restore_child_session(*conversation, root_session_id, ctx);
                }
                Some(CloudConversationData::CLIAgent(_)) | None => {
                    log::warn!(
                        "TUI restore: could not load descendant conversation \
                         {conversation_id:?} for restoration."
                    );
                }
            }
        });
    }

    /// Classifies a resolved child conversation and requests the matching
    /// restore-session materialization. Unsupported kinds are skipped with
    /// diagnostics so the rest of the tree still restores: shared-session
    /// viewers and explicit local non-Oz harnesses have no matching TUI view,
    /// and a remote child without stable task/run identity cannot be shown.
    fn emit_restore_child_session(
        &mut self,
        conversation: AIConversation,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let conversation_id = conversation.id();
        if conversation.is_viewing_shared_session() {
            log::debug!(
                "TUI restore: skipping shared-session viewer child {conversation_id:?}; \
                 the TUI has no shared-session view."
            );
            return;
        }
        if conversation.is_remote_child() {
            let (Some(task_id), Some(run_id)) = (conversation.task_id(), conversation.run_id())
            else {
                log::warn!(
                    "TUI restore: skipping remote child {conversation_id:?} without stable \
                     task/run identity."
                );
                return;
            };
            ctx.emit(TuiOrchestrationEvent::RestoreRemoteChildSession {
                root_session_id,
                conversation: Box::new(conversation),
                task_id,
                run_id,
            });
            return;
        }
        // A local child with no explicit harness predates orchestration-harness
        // stamping and is treated as legacy Oz.
        match conversation.orchestration_harness() {
            None | Some(Harness::Oz) => {
                ctx.emit(TuiOrchestrationEvent::RestoreLocalChildSession {
                    root_session_id,
                    conversation: Box::new(conversation),
                });
            }
            Some(
                harness @ (Harness::Claude
                | Harness::OpenCode
                | Harness::Gemini
                | Harness::Codex
                | Harness::Unknown),
            ) => {
                log::debug!(
                    "TUI restore: skipping local non-Oz child {conversation_id:?} \
                     (harness {harness:?}); the TUI has no matching retained view."
                );
            }
        }
    }

    /// Registers a restored local Oz child after its session was materialized
    /// and its transcript restored by the session registry.
    pub(crate) fn register_restored_local_oz_child_session(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        // A local child consumes its own event stream, mirroring live children.
        self.register_event_consumer(session_id, conversation_id, ctx);
        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        ctx.notify();
    }

    /// Registers a restored remote child after its lightweight cloud session was
    /// materialized by the session registry.
    pub(crate) fn register_restored_remote_child_session(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        // Remote placeholders stay passive; the root parent watches descendant
        // status (registered in `restore_descendant_sessions`).
        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        self.fetch_restored_remote_child_status(session_id, conversation_id, task_id, ctx);
        ctx.notify();
    }

    /// Deletes a child conversation and removes its retained TUI session.
    pub(crate) fn cleanup_child(
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

    /// Kills a child agent: tombstones late events, cancels any in-flight
    /// execution (cloud task or local controller), deletes the conversation
    /// from history, and removes the retained TUI session. Equivalent to the
    /// GUI's `KillAgentConversation` path.
    pub(crate) fn kill_child_agent(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        // 1. Tombstone so late SSE events cannot resurrect the killed child.
        OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
            streamer.mark_conversation_killed(conversation_id, ctx);
        });

        // 2. Cancel in-flight execution BEFORE deletion (AC 6).
        //    Read what we need up front to avoid borrow-checker issues with the
        //    subsequent mutable operations.
        let is_in_progress = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| c.status().is_in_progress() || c.status().is_blocked());
        let is_remote = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|c| c.is_remote_child());
        let task_id: Option<AmbientAgentTaskId> = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.task_id());
        let child_session_id = self
            .child_session_by_conversation
            .get(&conversation_id)
            .copied();

        if is_in_progress {
            if is_remote {
                // Remote (cloud) child: best-effort cancel the server-side task.
                // This is async and fire-and-forget; the tombstone above ensures
                // late events cannot resurrect the child regardless.
                if let Some(task_id) = task_id {
                    let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
                    ctx.spawn(
                        async move { ai_client.cancel_ambient_agent_task(&task_id).await },
                        |_, result, _| {
                            if let Err(e) = result {
                                log::warn!("kill_child_agent: failed to cancel cloud task: {e:#}");
                            }
                        },
                    );
                }
            } else if let Some(session_id) = child_session_id {
                // Cancelling through the session is deferred until the current
                // view update completes, then cleanup resumes from the event
                // handler while the conversation is still available.
                ctx.emit(TuiOrchestrationEvent::KillLocalChildSession {
                    session_id,
                    conversation_id,
                });
                return;
            }
        }

        // 3. Delete conversation and remove session.
        self.cleanup_child(&conversation_id, ctx);
    }

    /// Kills every descendant spawned by `conversation_id`, including nested
    /// descendants. Children are removed deepest-first so each retained
    /// session can tear down while its ancestry is still available.
    pub(crate) fn kill_descendant_agents(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            conversation_id,
        );
        for descendant_id in descendant_ids.into_iter().rev() {
            self.kill_child_agent(descendant_id, ctx);
        }
    }

    /// Kills a child agent together with its entire loaded subtree,
    /// deepest-first, so no descendant session is orphaned. With multi-level
    /// orchestration disabled this only kills the child itself — depth > 1
    /// cannot arise from new activity while the flag is off.
    pub(crate) fn kill_child_agent_subtree(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        if FeatureFlag::MultiLevelOrchestration.is_enabled() {
            self.kill_descendant_agents(conversation_id, ctx);
        }
        self.kill_child_agent(conversation_id, ctx);
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
                false,
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

    /// Unregisters a single (session, conversation) event-consumer pairing,
    /// dropping the session's entry once it has no remaining consumers.
    fn unregister_event_consumer(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conversations) = self.event_consumers_by_session.get_mut(&session_id) else {
            return;
        };
        if conversations.remove(&conversation_id) {
            unregister_agent_event_consumer(conversation_id, session_id.surface_id(), ctx);
        }
        if conversations.is_empty() {
            self.event_consumers_by_session.remove(&session_id);
        }
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

/// Resolves which conversation's level the drill-down bar shows for the
/// selection — the GUI's `drill_down_anchor_id` rule with the TUI's
/// navigability filter: a conversation with at least one navigable child
/// anchors its own level, while a leaf anchors its parent's level so sibling
/// navigation stays symmetric. At orchestration depth 1 this reproduces the
/// historical root-anchored behavior exactly.
fn drill_down_anchor_id(
    history: &BlocklistAIHistoryModel,
    session_ids_by_conversation: &HashMap<AIConversationId, TuiSessionId>,
    selected_conversation_id: AIConversationId,
    root_conversation_id: AIConversationId,
) -> AIConversationId {
    let has_navigable_child = history
        .child_conversation_ids_of(&selected_conversation_id)
        .iter()
        .any(|child_id| {
            session_ids_by_conversation.contains_key(child_id)
                && history.conversation(child_id).is_some()
        });
    if has_navigable_child {
        return selected_conversation_id;
    }
    history
        .conversation(&selected_conversation_id)
        .and_then(|conversation| {
            history.resolved_parent_conversation_id_for_conversation(conversation)
        })
        .unwrap_or(root_conversation_id)
}

/// Resolves the breadcrumb chips shown while the bar is drilled below the
/// tree root, mirroring the GUI's `breadcrumb_ids` rule: one chip for the
/// root, plus one for the anchor's direct parent when that parent is a
/// distinct intermediate level. Never more than two chips at any depth.
/// Chips must be selectable (they switch sessions), so a loaded but
/// sessionless parent contributes no chip.
fn breadcrumbs_for_anchor(
    history: &BlocklistAIHistoryModel,
    session_ids_by_conversation: &HashMap<AIConversationId, TuiSessionId>,
    anchor_conversation_id: AIConversationId,
    root_conversation_id: AIConversationId,
) -> Vec<TuiOrchestrationBreadcrumb> {
    if anchor_conversation_id == root_conversation_id {
        return Vec::new();
    }
    let mut breadcrumbs = vec![TuiOrchestrationBreadcrumb {
        conversation_id: root_conversation_id,
        label: ORCHESTRATOR_TAB_LABEL.to_owned(),
    }];
    let parent_id = history
        .conversation(&anchor_conversation_id)
        .and_then(|anchor| history.resolved_parent_conversation_id_for_conversation(anchor))
        .filter(|parent_id| {
            *parent_id != root_conversation_id
                && *parent_id != anchor_conversation_id
                && session_ids_by_conversation.contains_key(parent_id)
        });
    if let Some(parent_id) = parent_id {
        breadcrumbs.push(TuiOrchestrationBreadcrumb {
            conversation_id: parent_id,
            label: conversation_agent_label(history, parent_id),
        });
    }
    breadcrumbs
}

/// A conversation's non-empty agent name, or the shared fallback label.
fn conversation_agent_label(
    history: &BlocklistAIHistoryModel,
    conversation_id: AIConversationId,
) -> String {
    history
        .conversation(&conversation_id)
        .and_then(|conversation| conversation.agent_name())
        .filter(|name| !name.is_empty())
        .unwrap_or("Agent")
        .to_owned()
}

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
