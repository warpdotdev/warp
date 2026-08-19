use session_sharing_protocol::common::SessionId;
use uuid::Uuid;
use warp_errors::report_error;
use warpui::{SingletonEntity, ViewContext};

use super::materialization::{ChildPaneMaterialization, decide_child_pane_materialization};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::{
    AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskId,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::pane_group::{
    AmbientAgentViewModelHandleExt, PaneGroup, PaneId, TerminalPane, TerminalViewResources,
};
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};
use crate::terminal::view::{
    CompletedChildPresentation, ConversationAccess, completed_child_conversation_access,
    completed_child_presentation,
};

/// How to hydrate a restored hidden remote-child pane given its
/// [`AmbientAgentTask`]. Only used while `OrchestrationUnifiedStack` is
/// disabled. See [`decide_remote_child_hydration_action`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::pane_group) enum RemoteChildHydrationAction {
    /// Attachable live session — join it in place.
    LiveAttach,
    /// No live session but a server conversation token is available;
    /// `task_is_terminal` controls whether the post-merge step inserts a
    /// conversation-ended tombstone (only terminal runs do).
    LoadTranscript {
        server_token: ServerConversationToken,
        task_is_terminal: bool,
    },
    /// Neither live nor cloud transcript available; fall through to
    /// `attach_ambient_session_and_maybe_tombstone`. `task_is_terminal`
    /// gates the tombstone so an `ActiveUnattachable` run with no server
    /// token isn't visually marked as ended.
    Fallback { task_is_terminal: bool },
}

/// Pure decision function backing [`PaneGroup::hydrate_task_backed_hidden_child_pane`].
/// Free-standing so it's unit-testable without a `PaneGroup`.
pub(in crate::pane_group) fn decide_remote_child_hydration_action(
    task: &AmbientAgentTask,
) -> RemoteChildHydrationAction {
    let live_session_state = task.active_live_session_state();
    if matches!(
        live_session_state,
        AmbientAgentLiveSessionState::Attachable { .. }
    ) {
        return RemoteChildHydrationAction::LiveAttach;
    }

    let task_is_terminal = matches!(live_session_state, AmbientAgentLiveSessionState::Inactive);

    // Empty/whitespace tokens would drive a no-op cloud fetch followed by
    // a misleading tombstone; route them to `Fallback` instead.
    let server_token = task
        .conversation_id()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| ServerConversationToken::new(t.to_string()));

    match server_token {
        Some(server_token) => RemoteChildHydrationAction::LoadTranscript {
            server_token,
            task_is_terminal,
        },
        None => RemoteChildHydrationAction::Fallback { task_is_terminal },
    }
}

impl PaneGroup {
    /// Applies the unified viewer materialization decision to a task snapshot
    /// supplied by the parent shared-session viewer.
    pub(in crate::pane_group) fn materialize_viewer_child_pane_from_task(
        &mut self,
        child_id: AIConversationId,
        task: AmbientAgentTask,
        ctx: &mut ViewContext<Self>,
    ) {
        // Skip when this child already has a live tracked pane.
        if let Some(existing_pane_id) = self.child_agent_panes.get(&child_id).copied()
            && self.has_pane_id(existing_pane_id)
        {
            return;
        }
        let Some(child_conversation) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_id)
            .cloned()
        else {
            log::warn!(
                "materialize_viewer_child_pane_from_task: no child conversation {child_id:?}"
            );
            return;
        };
        self.apply_child_pane_materialization(child_conversation, task, ctx);
    }

    /// Materializes the pane for a placeholder child conversation from its
    /// [`AmbientAgentTask`](crate::ai::ambient_agents::AmbientAgentTask),
    /// leaving a loading pane in place while the task is still being fetched.
    ///
    /// Idempotent: repeat calls for a placeholder that already has a live
    /// tracked pane are skipped rather than creating a duplicate pane and
    /// orphaning the first.
    pub(in crate::pane_group) fn materialize_child_pane(
        &mut self,
        child_conversation: AIConversation,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        if let Some(existing_pane_id) = self.child_agent_panes.get(&child_id).copied()
            && self.has_pane_id(existing_pane_id)
        {
            return;
        }

        let Some(task_id) = child_conversation.task_id() else {
            log::warn!("Cannot restore remote child conversation {child_id:?} without a task ID");
            return;
        };
        let task = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        let Some(task) = task else {
            // Show the child loading presentation rather than the generic
            // cloud-agent composing zero state, and register for a re-drive
            // once the task fetch lands.
            if self
                .create_child_loading_placeholder(
                    child_conversation,
                    AgentViewEntryOrigin::CloudAgent,
                    ctx,
                )
                .is_none()
            {
                return;
            }
            self.pending_child_hydrations.insert(task_id, child_id);
            self.ensure_pending_ambient_restoration_subscription(ctx);
            return;
        };

        self.apply_child_pane_materialization(child_conversation, task, ctx);
    }

    /// Applies the materialization decision for a child whose task snapshot is
    /// available: `AttachLive` joins the live session, `LoadTranscript`
    /// fetches and merges the cloud transcript, and `Pending` leaves a loading
    /// placeholder in place until fresher task data arrives.
    fn apply_child_pane_materialization(
        &mut self,
        child_conversation: AIConversation,
        task: AmbientAgentTask,
        ctx: &mut ViewContext<Self>,
    ) {
        match decide_child_pane_materialization(&task) {
            ChildPaneMaterialization::AttachLive { session_id } => {
                let child_id = child_conversation.id();
                if self.failed_viewer_child_sessions.get(&child_id) == Some(&session_id) {
                    // The session already failed to join; wait for a later
                    // execution rather than retrying the same dead session.
                    if let Some(task_id) = child_conversation.task_id() {
                        self.pending_child_hydrations.insert(task_id, child_id);
                        self.ensure_pending_ambient_restoration_subscription(ctx);
                    }
                    if let Some(pane_id) = self.child_agent_panes.get(&child_id).copied()
                        && let Some(view) = self.terminal_view_from_pane_id(pane_id, ctx)
                    {
                        view.update(ctx, |view, ctx| {
                            view.set_orchestration_child_live_unavailable(true, ctx);
                        });
                    }
                    return;
                }
                self.failed_viewer_child_sessions.remove(&child_id);
                if let Some(task_id) = child_conversation.task_id() {
                    self.pending_child_hydrations.remove(&task_id);
                }
                self.attach_ambient_orchestration_child_session(child_id, session_id, ctx);
            }
            ChildPaneMaterialization::LoadTranscript { server_token } => {
                let child_id = child_conversation.id();
                let task_id = task.task_id;
                let pane_id = self
                    .child_agent_panes
                    .get(&child_id)
                    .copied()
                    .filter(|pane_id| self.has_pane_id(*pane_id))
                    .or_else(|| {
                        self.create_child_loading_placeholder(
                            child_conversation,
                            AgentViewEntryOrigin::CloudAgent,
                            ctx,
                        )
                    });
                let Some(pane_id) = pane_id else {
                    return;
                };
                self.pending_child_hydrations.remove(&task_id);
                self.failed_viewer_child_sessions.remove(&child_id);
                self.hydrate_child_transcript(pane_id, child_id, task_id, server_token, ctx);
            }
            ChildPaneMaterialization::Pending => {
                let child_id = child_conversation.id();
                let task_id = task.task_id;
                if !self
                    .child_agent_panes
                    .get(&child_id)
                    .is_some_and(|pane_id| self.has_pane_id(*pane_id))
                {
                    let _ = self.create_child_loading_placeholder(
                        child_conversation,
                        AgentViewEntryOrigin::CloudAgent,
                        ctx,
                    );
                }
                self.pending_child_hydrations.insert(task_id, child_id);
                self.ensure_pending_ambient_restoration_subscription(ctx);
            }
        }
    }

    /// Builds a child pane that joins `session_id` from the start. Any loading
    /// pane already showing for this child is discarded and the replacement is
    /// swapped into the same anchor only once its session manager, ambient
    /// model, and conversation are initialized, so the user never sees a
    /// half-built pane.
    fn attach_ambient_orchestration_child_session(
        &mut self,
        child_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(child_conversation) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_id)
            .cloned()
        else {
            log::warn!(
                "ambient child live replacement: no conversation \
                 child_conversation_id={child_id:?}"
            );
            return;
        };
        let Some(task_id) = child_conversation.task_id() else {
            log::warn!(
                "ambient child live replacement: no task id \
                 child_conversation_id={child_id:?}"
            );
            return;
        };
        let fallback_was_swapped_anchor = if let Some(prior_pane_id) = self
            .child_agent_panes
            .get(&child_id)
            .copied()
            .filter(|pane_id| self.has_pane_id(*pane_id))
        {
            let anchor = self.panes.original_pane_for_replacement(prior_pane_id);
            self.discard_child_agent_pane_for_conversation(child_id, ctx);
            anchor
        } else {
            None
        };

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (new_terminal_view, terminal_manager) = Self::create_ambient_orchestration_child_pane(
            session_id, child_id, resources, view_size, ctx,
        );
        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            new_terminal_view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            report_error!(
                "attach_ambient_orchestration_child_session: failed to attach pane",
                extra: { "child_conversation_id" => ?child_id }
            );
            return;
        }

        new_terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.suppress_initial_conversation_details_panel_auto_open();
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_id),
                AgentViewEntryOrigin::CloudAgent,
                ctx,
            );
            if let Some(ambient_agent_view_model) =
                terminal_view.ambient_agent_view_model().cloned()
            {
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_conversation_id(Some(child_id));
                    model.enter_viewing_existing_session(task_id, ctx);
                    model.set_live_execution_session(session_id);
                });
            }
        });

        self.child_agent_panes.insert(child_id, new_pane_id.into());
        if let Some(anchor) = fallback_was_swapped_anchor {
            self.swap_active_pane_to_conversation(anchor, child_id, ctx);
        }
    }

    /// Points the pane's ambient agent view model at the existing session for
    /// `task_id` and at the child's conversation.
    fn apply_existing_ambient_task_to_pane(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) else {
            return;
        };
        terminal_view.update(ctx, |terminal_view, ctx| {
            let Some(ambient_agent_view_model) = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .cloned()
            else {
                return;
            };
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.set_conversation_id(Some(child_id));
                model.enter_viewing_existing_session(task_id, ctx);
            });
        });
    }

    /// Loads a completed child conversation's cloud transcript and chooses
    /// continuation or passive presentation based on the caller's access to
    /// the conversation.
    fn hydrate_child_transcript(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        server_token: ServerConversationToken,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let future = history_handle.update(ctx, |history_model, ctx| {
            history_model.load_conversation_by_server_token(&server_token, ctx)
        });
        ctx.spawn(future, move |group, conversation, ctx| {
            let still_canonical = group
                .child_agent_panes
                .get(&child_id)
                .copied()
                .is_some_and(|p| p == pane_id && group.has_pane_id(p));
            if !still_canonical {
                return;
            }
            // The pane may have been swapped to another conversation while
            // the fetch was in flight; don't overwrite what it now shows.
            let active_conversation = group
                .terminal_view_from_pane_id(pane_id, ctx)
                .and_then(|view| view.as_ref(ctx).active_conversation_id(ctx));
            if active_conversation != Some(child_id) {
                return;
            }
            let task = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id);
            let access = match conversation.as_ref() {
                Some(CloudConversationData::Oz(cloud)) => {
                    completed_child_conversation_access(cloud.server_metadata(), task.as_ref(), ctx)
                }
                _ => ConversationAccess::Unknown,
            };

            let merged = match conversation {
                Some(CloudConversationData::Oz(cloud)) => {
                    let tasks: Vec<warp_multi_agent_api::Task> = cloud
                        .all_tasks()
                        .filter_map(|task| task.source().cloned())
                        .collect();
                    let cloud_conversation = *cloud;
                    match BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                        history.hydrate_remote_child_placeholder_with_cloud_transcript(
                            child_id,
                            tasks,
                            cloud_conversation,
                        )
                    }) {
                        Ok(merged) => merged,
                        Err(err) => {
                            log::warn!(
                                "child transcript upgrade merge-error \
                                 child_conversation_id={child_id:?} error={err:#}"
                            );
                            return;
                        }
                    }
                }
                Some(CloudConversationData::CLIAgent(_)) => {
                    log::warn!(
                        "child transcript upgrade unsupported \
                         CLI transcript child_conversation_id={child_id:?}"
                    );
                    return;
                }
                None => {
                    log::warn!(
                        "child transcript upgrade fetch-empty \
                         child_conversation_id={child_id:?}"
                    );
                    // Re-queue without evicting the task so the retry is
                    // driven by the normal TasksUpdated cadence rather than
                    // firing immediately on every round-trip. Evicting would
                    // create an unbounded loop on permanent failures such as
                    // a 403 from an Observer that can see the task row but
                    // not the conversation transcript.
                    group.pending_child_hydrations.insert(task_id, child_id);
                    return;
                }
            };

            let blocks_cloud_followups = task
                .as_ref()
                .is_none_or(AmbientAgentTask::blocks_cloud_followups);
            match completed_child_presentation(access, blocks_cloud_followups) {
                CompletedChildPresentation::Continuation => {
                    group.replace_child_loading_with_continuation_pane(
                        pane_id, child_id, task_id, merged, ctx,
                    );
                }
                CompletedChildPresentation::PassiveTranscript => {
                    group.restore_child_passive_transcript(pane_id, child_id, task_id, merged, ctx);
                }
            }
        });
    }

    /// Replaces an off-tree child loading pane with the established ambient
    /// cloud-mode continuation presentation.
    pub(in crate::pane_group) fn replace_child_loading_with_continuation_pane(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        merged: AIConversation,
        ctx: &mut ViewContext<Self>,
    ) {
        let fallback_was_swapped_anchor = self.panes.original_pane_for_replacement(pane_id);
        self.discard_child_agent_pane_for_conversation(child_id, ctx);

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (terminal_view, terminal_manager) =
            Self::create_cloud_mode_terminal(resources, view_size, false, ctx);
        Self::load_data_into_restored_ambient_cloud_mode_view(
            terminal_view.clone(),
            CloudConversationData::Oz(Box::new(merged)),
            task_id,
            false,
            ctx,
        );
        terminal_view.update(ctx, |view, ctx| {
            view.enable_completed_cloud_continuation(task_id, ctx);
        });
        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            terminal_manager,
            terminal_view,
            self.model_event_sender.clone(),
            ctx,
        );
        let replacement_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            report_error!(
                "replace_child_loading_with_continuation_pane: failed to attach restored child pane",
                extra: { "child_conversation_id" => ?child_id }
            );
            return;
        }
        self.child_agent_panes
            .insert(child_id, replacement_pane_id.into());
        if let Some(anchor) = fallback_was_swapped_anchor {
            self.swap_active_pane_to_conversation(anchor, child_id, ctx);
        }
    }

    /// Restores a child transcript in place without enabling continuation.
    fn restore_child_passive_transcript(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        merged: AIConversation,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(terminal_manager) = self
            .terminal_session_by_id(pane_id)
            .map(|session| session.terminal_manager(ctx))
        {
            terminal_manager.update(ctx, |manager, _ctx| {
                let model_handle = manager.model();
                let mut model = model_handle.lock();
                model.set_shared_session_status(
                    crate::terminal::shared_session::SharedSessionStatus::FinishedViewer,
                );
                model.set_conversation_transcript_viewer_status(Some(
                    ConversationTranscriptViewerStatus::ViewingAmbientConversation(task_id),
                ));
            });
        }
        if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                history.mark_terminal_surface_as_conversation_transcript_viewer(terminal_view.id());
            });
            terminal_view.update(ctx, |view, ctx| {
                view.set_orchestration_child_live_unavailable(false, ctx);
                view.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(merged),
                    true,
                    RestoreConversationEntryBehavior::PreserveAgentViewState,
                    ctx,
                );
                view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
            });
        }
        self.child_agent_panes.insert(child_id, pane_id);
    }

    /// Renders the shared loading placeholder presentation used while a child
    /// has neither an attachable session nor a loadable transcript.
    pub(in crate::pane_group) fn create_child_loading_placeholder(
        &mut self,
        child_conversation: AIConversation,
        origin: AgentViewEntryOrigin,
        ctx: &mut ViewContext<Self>,
    ) -> Option<PaneId> {
        let child_id = child_conversation.id();
        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        let (loading_view, loading_manager) = Self::create_loading_terminal_manager_and_view(
            resources,
            view_size,
            ctx.window_id(),
            ctx,
        );
        let pane_data = TerminalPane::new(
            Uuid::new_v4().as_bytes().to_vec(),
            loading_manager,
            loading_view.clone(),
            self.model_event_sender.clone(),
            ctx,
        );
        let new_pane_id = pane_data.terminal_pane_id();
        if self
            .attach_child_pane_off_tree(Box::new(pane_data), ctx)
            .is_none()
        {
            report_error!(
                "create_child_loading_placeholder: failed to attach child loading pane",
                extra: { "child_id" => ?child_id }
            );
            return None;
        }

        // Entering agent view is what makes the pill bar render; the loading
        // view keeps the output area a spinner until hydration completes.
        loading_view.update(ctx, |terminal_view, ctx| {
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(None, Some(child_id), origin, ctx);
        });

        self.child_agent_panes.insert(child_id, new_pane_id.into());
        Some(new_pane_id.into())
    }

    /// Recovers a child viewer whose live session no longer exists or is not
    /// accessible. The same session is not retried; refreshed task metadata
    /// can upgrade this pane to a transcript or attach a later execution.
    pub(in crate::pane_group) fn recover_viewer_child_join_failure(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.child_agent_panes.get(&child_id) != Some(&pane_id) {
            return;
        }
        let Some(task_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_id)
            .filter(|conversation| {
                conversation.is_viewing_shared_session() || conversation.is_remote_child()
            })
            .and_then(|conversation| conversation.task_id())
        else {
            return;
        };

        self.failed_viewer_child_sessions
            .insert(child_id, session_id);
        self.pending_child_hydrations.insert(task_id, child_id);
        self.ensure_pending_ambient_restoration_subscription(ctx);
        if let Some(view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            view.update(ctx, |view, ctx| {
                view.set_orchestration_child_live_unavailable(true, ctx);
            });
        }
        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.evict_and_refetch_task(&task_id, ctx);
        });
    }

    /// Re-drives child panes after task metadata changes. A failed live
    /// session stays unavailable without retrying, while terminal tasks
    /// upgrade the existing pane to a passive transcript.
    pub(in crate::pane_group) fn process_pending_child_hydrations(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !crate::features::FeatureFlag::OrchestrationUnifiedStack.is_enabled()
            || self.pending_child_hydrations.is_empty()
        {
            return;
        }

        let ready_tasks: Vec<_> = self
            .pending_child_hydrations
            .keys()
            .filter(|task_id| {
                AgentConversationsModel::as_ref(ctx)
                    .get_task_data(task_id)
                    .is_some()
            })
            .copied()
            .collect();

        for task_id in ready_tasks {
            let Some(child_id) = self.pending_child_hydrations.remove(&task_id) else {
                continue;
            };
            let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
                continue;
            };
            let Some(pane_id) = self
                .child_agent_panes
                .get(&child_id)
                .copied()
                .filter(|pane_id| self.has_pane_id(*pane_id))
            else {
                continue;
            };

            match decide_child_pane_materialization(&task) {
                ChildPaneMaterialization::AttachLive { session_id }
                    if self.failed_viewer_child_sessions.get(&child_id) == Some(&session_id) =>
                {
                    self.pending_child_hydrations.insert(task_id, child_id);
                }
                ChildPaneMaterialization::AttachLive { session_id } => {
                    self.failed_viewer_child_sessions.remove(&child_id);
                    self.attach_ambient_orchestration_child_session(child_id, session_id, ctx);
                }
                ChildPaneMaterialization::LoadTranscript { server_token } => {
                    self.failed_viewer_child_sessions.remove(&child_id);
                    self.hydrate_child_transcript(pane_id, child_id, task_id, server_token, ctx);
                }
                ChildPaneMaterialization::Pending => {
                    self.pending_child_hydrations.insert(task_id, child_id);
                }
            }
        }
    }

    // =========================================================================
    // flag-OFF path (OrchestrationUnifiedStack disabled)
    // =========================================================================

    /// Task-backed restore path for remote children while
    /// `OrchestrationUnifiedStack` is disabled. Creates the hidden ambient
    /// pane, registers it in `child_agent_panes` under the placeholder's local
    /// `AIConversationId`, then hydrates it (or queues a pending entry while
    /// task data is fetched).
    ///
    /// Idempotent: repeat calls for a placeholder that already has a live
    /// tracked pane — including while the initial async hydration is still in
    /// flight — are skipped rather than orphaning the first pane.
    pub(super) fn hydrate_task_backed_hidden_child_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        if let Some(existing_pane_id) = self.child_agent_panes.get(&child_id).copied()
            && self.has_pane_id(existing_pane_id)
        {
            return;
        }

        let new_pane_id =
            self.insert_ambient_agent_pane_hidden_for_child_agent(parent_pane_id, ctx);

        let Some(new_terminal_view) = self.terminal_view_from_pane_id(new_pane_id, ctx) else {
            report_error!(
                "Failed to get terminal view for remote child agent pane",
                extra: { "child_id" => ?child_id }
            );
            self.discard_pane(new_pane_id.into(), ctx);
            return;
        };

        // Restore the placeholder so the pane has parent linkage + agent
        // name before task-backed hydration runs.
        let mut restored = false;
        new_terminal_view.update(ctx, |terminal_view, ctx| {
            terminal_view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(child_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            terminal_view.enter_agent_view(
                None,
                Some(child_id),
                AgentViewEntryOrigin::CloudAgent,
                ctx,
            );
            restored = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .is_some();
        });

        if !restored {
            report_error!(
                "Failed to restore remote child agent pane: missing ambient agent view model",
                extra: { "child_id" => ?child_id }
            );
            self.discard_pane(new_pane_id.into(), ctx);
            return;
        }

        // Placeholder's local id stays the canonical `child_agent_panes`
        // key across live-attach and transcript hydration.
        self.child_agent_panes.insert(child_id, new_pane_id.into());

        let task_now = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.get_or_async_fetch_task_data(&task_id, ctx)
        });

        if task_now.is_none() {
            // Task data not yet cached: queue a pending hydration and
            // attempt a live-attach in the meantime so streaming runs are
            // not stalled while waiting on the fetch.
            self.pending_remote_child_hydrations
                .insert(task_id, child_id);
            self.ensure_pending_ambient_restoration_subscription(ctx);
            self.apply_existing_ambient_task_to_pane(new_pane_id.into(), child_id, task_id, ctx);
            return;
        }

        self.attempt_remote_child_hydration(child_id, task_id, ctx);
    }

    /// Dispatches the hydration action chosen by
    /// [`decide_remote_child_hydration_action`] for a restored hidden child
    /// pane when `OrchestrationUnifiedStack` is disabled.
    fn attempt_remote_child_hydration(
        &mut self,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(pane_id) = self
            .child_agent_panes
            .get(&child_id)
            .copied()
            .filter(|pane_id| self.has_pane_id(*pane_id))
        else {
            return;
        };

        let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
            // Defensive: callers only reach here after `get_task_data`
            // returned `Some`. If it's gone now, leave the pending entry
            // alone so the next `TasksUpdated` can re-drive.
            return;
        };

        match decide_remote_child_hydration_action(&task) {
            RemoteChildHydrationAction::LiveAttach => {
                self.apply_existing_ambient_task_to_pane(pane_id, child_id, task_id, ctx);
            }
            RemoteChildHydrationAction::LoadTranscript {
                server_token,
                task_is_terminal,
            } => {
                self.hydrate_remote_child_transcript_in_place(
                    pane_id,
                    child_id,
                    task_id,
                    server_token,
                    task_is_terminal,
                    ctx,
                );
            }
            RemoteChildHydrationAction::Fallback { task_is_terminal } => {
                // No live session, no server token: attach to the
                // (possibly empty) ambient session, then insert the
                // conversation-ended tombstone iff the run is terminal so
                // an `ActiveUnattachable` child isn't visually ended.
                self.attach_ambient_session_and_maybe_tombstone(
                    pane_id,
                    child_id,
                    task_id,
                    task_is_terminal,
                    ctx,
                );
            }
        }
    }

    /// Fetches the cloud transcript for a restored hidden child pane when
    /// `OrchestrationUnifiedStack` is disabled. `task_is_terminal` gates
    /// the conversation-ended tombstone in the post-match step.
    fn hydrate_remote_child_transcript_in_place(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        server_token: ServerConversationToken,
        task_is_terminal: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let future = history_handle.update(ctx, |history_model, ctx| {
            history_model.load_conversation_by_server_token(&server_token, ctx)
        });
        ctx.spawn(future, move |group, conversation, ctx| {
            // Guard against a stale target while the fetch was in flight:
            // the pane id must still be the canonical one for `child_id`
            // AND the pane's terminal view must still be displaying it.
            let still_canonical = group
                .child_agent_panes
                .get(&child_id)
                .copied()
                .is_some_and(|p| p == pane_id && group.has_pane_id(p));
            if !still_canonical {
                return;
            }
            let terminal_view_active_conversation = group
                .terminal_view_from_pane_id(pane_id, ctx)
                .and_then(|tv| tv.as_ref(ctx).active_conversation_id(ctx));
            if terminal_view_active_conversation != Some(child_id) {
                return;
            }

            match conversation {
                Some(CloudConversationData::Oz(cloud)) => {
                    let tasks: Vec<warp_multi_agent_api::Task> = cloud
                        .all_tasks()
                        .filter_map(|task| task.source().cloned())
                        .collect();
                    let cloud_conversation = *cloud;
                    let merge_result =
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                            history.hydrate_remote_child_placeholder_with_cloud_transcript(
                                child_id,
                                tasks,
                                cloud_conversation,
                            )
                        });
                    match merge_result {
                        Ok(merged) => {
                            if let Some(terminal_view) =
                                group.terminal_view_from_pane_id(pane_id, ctx)
                            {
                                terminal_view.update(ctx, |view, ctx| {
                                    view.restore_conversation_after_view_creation(
                                        RestoredAIConversation::new(merged),
                                        true,
                                        RestoreConversationEntryBehavior::PreserveAgentViewState,
                                        ctx,
                                    );
                                });
                            }
                        }
                        Err(err) => {
                            log::warn!(
                                "hydrate_remote_child_placeholder_with_cloud_transcript failed for {child_id:?}: {err:#}"
                            );
                        }
                    }
                }
                Some(CloudConversationData::CLIAgent(_)) | None => {
                    // Non-Oz transcript or fetch failure — the post-match
                    // call handles attach + conditional tombstone.
                }
            }

            // Uniform post-match step so the `task_is_terminal` gate
            // applies to all three branches above.
            group.attach_ambient_session_and_maybe_tombstone(
                pane_id,
                child_id,
                task_id,
                task_is_terminal,
                ctx,
            );
        });
    }

    /// Post-match step for `hydrate_remote_child_transcript_in_place` when
    /// `OrchestrationUnifiedStack` is disabled: attaches the live ambient
    /// session and conditionally inserts the conversation-ended tombstone.
    fn attach_ambient_session_and_maybe_tombstone(
        &mut self,
        pane_id: PaneId,
        child_id: AIConversationId,
        task_id: AmbientAgentTaskId,
        task_is_terminal: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.apply_existing_ambient_task_to_pane(pane_id, child_id, task_id, ctx);
        if !task_is_terminal {
            return;
        }
        if let Some(terminal_view) = self.terminal_view_from_pane_id(pane_id, ctx) {
            terminal_view.update(ctx, |view, ctx| {
                view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
            });
        }
    }

    /// Drains entries from `pending_remote_child_hydrations` for which task
    /// data is now available, hydrating each hidden child pane in place. That
    /// map is only populated while `OrchestrationUnifiedStack` is disabled.
    pub(in crate::pane_group) fn process_pending_remote_child_hydrations(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.pending_remote_child_hydrations.is_empty() {
            return;
        }

        let ready_tasks: Vec<_> = self
            .pending_remote_child_hydrations
            .keys()
            .filter(|task_id| {
                AgentConversationsModel::as_ref(ctx)
                    .get_task_data(task_id)
                    .is_some()
            })
            .copied()
            .collect();

        for task_id in ready_tasks {
            let Some(child_id) = self.pending_remote_child_hydrations.remove(&task_id) else {
                continue;
            };

            self.attempt_remote_child_hydration(child_id, task_id, ctx);
        }
    }
}
