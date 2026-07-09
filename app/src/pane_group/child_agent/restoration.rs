use std::collections::HashMap;
use std::path::PathBuf;

use session_sharing_protocol::common::SessionId;
use uuid::Uuid;
use warp_errors::report_error;
use warpui::{SingletonEntity, ViewContext};

use super::{apply_hidden_child_agent_task_context, HiddenChildAgentTaskContext};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::pane_group::{
    AmbientAgentViewModelHandleExt, PaneGroup, PaneId, TerminalPane, TerminalViewResources,
};
use crate::terminal::shared_session::IsSharedSessionCreator;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

impl PaneGroup {
    /// Lazily restores hidden child panes for the given parent conversation.
    ///
    /// Unlike the old startup sweep, this runs only when the parent agent view
    /// is actually restored or entered. Children that already belong to some
    /// other pane or tab are left alone.
    pub(in crate::pane_group) fn restore_missing_child_agent_panes_for_parent(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_ids = BlocklistAIHistoryModel::as_ref(ctx)
            .child_conversation_ids_of(&parent_conversation_id)
            .to_vec();

        for child_id in child_ids {
            if self
                .child_agent_panes
                .get(&child_id)
                .is_some_and(|pane_id| self.has_pane_id(*pane_id))
            {
                continue;
            }

            if self.is_conversation_owned_outside_pane(child_id, parent_pane_id, ctx) {
                continue;
            }

            let child_conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_id)
                .cloned()
                .or_else(|| {
                    RestoredAgentConversations::handle(ctx)
                        .update(ctx, |store, _| store.take_conversation(&child_id))
                });
            let Some(child_conversation) = child_conversation else {
                log::warn!("Child conversation {child_id:?} not found in memory or restored store");
                continue;
            };

            self.create_hidden_child_agent_pane(child_conversation, parent_pane_id, ctx);
        }
    }

    /// Restores hidden child panes if this terminal pane is already showing a
    /// fullscreen agent view. This covers restored or replaced panes whose
    /// terminal view entered agent view before pane-group attachment finished.
    pub(in crate::pane_group) fn restore_missing_child_agent_panes_for_terminal_pane_if_needed(
        &mut self,
        pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(terminal_pane_id) = pane_id.as_terminal_pane_id() else {
            return;
        };
        let Some(parent_conversation_id) = self
            .terminal_view_from_pane_id(terminal_pane_id, ctx)
            .and_then(|terminal_view| {
                let terminal_view = terminal_view.as_ref(ctx);
                let controller = terminal_view.agent_view_controller().as_ref(ctx);
                if controller.is_fullscreen() {
                    controller.agent_view_state().active_conversation_id()
                } else {
                    None
                }
            })
        else {
            return;
        };

        self.restore_missing_child_agent_panes_for_parent(
            parent_conversation_id,
            terminal_pane_id.into(),
            ctx,
        );
    }

    /// Ensures `child_conversation_id` has a hidden child pane if it still
    /// belongs under a parent conversation in this pane group.
    ///
    /// Returns true if the conversation is already reachable through an
    /// existing pane or if lazy restoration successfully materialized the child
    /// pane.
    pub(in crate::pane_group) fn ensure_hidden_child_agent_pane_for_conversation(
        &mut self,
        child_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self
            .child_agent_panes
            .get(&child_conversation_id)
            .is_some_and(|pane_id| self.has_pane_id(*pane_id))
        {
            return true;
        }

        let parent_conversation_id =
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model
                    .conversation(&child_conversation_id)
                    .and_then(|conversation| {
                        history_model.resolved_parent_conversation_id_for_conversation(conversation)
                    })
                    .or_else(|| {
                        RestoredAgentConversations::handle(ctx).update(ctx, |store, _| {
                            store.get_conversation(&child_conversation_id).and_then(
                                |conversation| {
                                    history_model.resolved_parent_conversation_id_for_conversation(
                                        conversation,
                                    )
                                },
                            )
                        })
                    })
            });

        let Some(parent_conversation_id) = parent_conversation_id else {
            return self
                .terminal_view_id_for_owned_conversation(child_conversation_id, ctx)
                .is_some();
        };

        let child_owner_terminal_view_id =
            self.terminal_view_id_for_owned_conversation(child_conversation_id, ctx);
        let Some(parent_pane_id) = self.pane_id_for_owned_conversation(parent_conversation_id, ctx)
        else {
            return child_owner_terminal_view_id.is_some();
        };

        if self.is_conversation_owned_outside_pane(child_conversation_id, parent_pane_id, ctx) {
            return true;
        }

        self.restore_missing_child_agent_panes_for_parent(
            parent_conversation_id,
            parent_pane_id,
            ctx,
        );

        self.child_agent_panes
            .get(&child_conversation_id)
            .is_some_and(|pane_id| self.has_pane_id(*pane_id))
            || self.is_conversation_owned_outside_pane(child_conversation_id, parent_pane_id, ctx)
    }

    /// Creates a hidden child agent pane for an existing child conversation,
    /// restoring the conversation and tracking it in `child_agent_panes`.
    pub(in crate::pane_group) fn create_hidden_child_agent_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        // Viewer-side child clicked before `OrchestrationViewerModel`
        // surfaced a `session_id`: render a loading placeholder; the real
        // pane gets swapped in by `ensure_shared_session_viewer_child_pane`.
        if child_conversation.is_viewing_shared_session() {
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
                    "create_hidden_child_agent_pane: failed to attach loading placeholder for \
                     viewer-side child",
                    extra: { "child_id" => ?child_id }
                );
                return;
            }

            // Restore the conversation and enter agent view so the pill bar
            // renders (its gate requires `is_fullscreen()`). The output area
            // stays a loading spinner because the loading view's
            // `ConversationTranscriptViewerStatus::Loading` short-circuits
            // the block list render in `TerminalView::render`.
            loading_view.update(ctx, |terminal_view, ctx| {
                terminal_view.restore_conversation_after_view_creation(
                    RestoredAIConversation::new(child_conversation),
                    true,
                    RestoreConversationEntryBehavior::PreserveAgentViewState,
                    ctx,
                );
                terminal_view.enter_agent_view(
                    None,
                    Some(child_id),
                    AgentViewEntryOrigin::SharedSessionSelection,
                    ctx,
                );
            });

            self.child_agent_panes.insert(child_id, new_pane_id.into());
            return;
        }

        if child_conversation.is_remote_child() {
            let Some(task_id) = child_conversation.task_id() else {
                log::warn!(
                    "Cannot restore remote child conversation {child_id:?} without a task ID"
                );
                return;
            };
            self.hydrate_task_backed_hidden_child_pane(
                child_conversation,
                parent_pane_id,
                task_id,
                ctx,
            );
            return;
        }
        let child_task_context =
            child_conversation
                .task_id()
                .map(|task_id| HiddenChildAgentTaskContext {
                    task_id,
                    working_dir: child_conversation
                        .current_working_directory()
                        .or_else(|| child_conversation.initial_working_directory())
                        .map(PathBuf::from),
                });
        // Restored hidden child panes don't inherit the host's shared
        // session — the host's share decision is handled at original
        // dispatch time, not on subsequent restores.
        let new_pane_id = self.insert_terminal_pane_hidden_for_child_agent(
            parent_pane_id,
            HashMap::new(),
            IsSharedSessionCreator::No,
            ctx,
        );

        if let Some(new_terminal_view) = self.terminal_view_from_pane_id(new_pane_id, ctx) {
            if let Some(task_context) = child_task_context.as_ref() {
                apply_hidden_child_agent_task_context(&new_terminal_view, task_context, ctx);
            }
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
                    AgentViewEntryOrigin::ChildAgent,
                    ctx,
                );
            });

            self.child_agent_panes.insert(child_id, new_pane_id.into());
        } else {
            report_error!(
                "Failed to get terminal view for child agent pane",
                extra: { "child_id" => ?child_id }
            );
            self.discard_pane(new_pane_id.into(), ctx);
        }
    }

    /// Materializes a hidden shared-session viewer pane for a viewer-
    /// discovered child agent. Triggered by
    /// `Event::EnsureSharedSessionViewerChildPane`, which
    /// `OrchestrationViewerModel` emits on the parent's view the first
    /// time it observes a `session_id` for a child. The new pane gets its
    /// own `BlocklistAIController` and viewer-side `Network` so child
    /// traffic doesn't cross the parent's single-stream state.
    pub(in crate::pane_group) fn ensure_shared_session_viewer_child_pane(
        &mut self,
        child_conversation_id: AIConversationId,
        child_session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Race recovery: a pill click before materialization had a
        // `session_id` falls through to `create_hidden_child_agent_pane`,
        // which leaves a loading placeholder in `child_agent_panes`. The
        // emission gate in `OrchestrationViewerModel` guarantees this
        // helper runs at most once per child per model lifetime, so any
        // existing entry must be that fallback — safe to discard.
        let fallback_was_swapped_anchor = if let Some(prior_pane_id) = self
            .child_agent_panes
            .get(&child_conversation_id)
            .copied()
            .filter(|pane_id| self.has_pane_id(*pane_id))
        {
            let anchor = self.panes.original_pane_for_replacement(prior_pane_id);
            self.discard_child_agent_pane_for_conversation(child_conversation_id, ctx);
            anchor
        } else {
            None
        };

        let Some(child_conversation) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&child_conversation_id)
            .cloned()
        else {
            log::warn!(
                "ensure_shared_session_viewer_child_pane: no local conversation {child_conversation_id:?}"
            );
            return;
        };
        let child_task_id = child_conversation.task_id();

        let resources = TerminalViewResources {
            tips_completed: self.tips_completed.clone(),
            server_api: self.server_api.clone(),
            model_event_sender: self.model_event_sender.clone(),
        };
        let view_size = Self::estimated_view_bounds(ctx).size();
        // Per-child viewer: parent's model already discovers descendants, and
        // hidden child viewers aren't snapshotted, so `is_cloud_mode` stays
        // `false` (no `ambient_agent_view_model` needed for snapshot round-trip).
        let (new_terminal_view, terminal_manager) = Self::create_shared_session_viewer(
            child_session_id,
            resources,
            view_size,
            false, // enable_orchestration_polling
            false, // is_ambient_agent
            ctx,
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
                "ensure_shared_session_viewer_child_pane: failed to attach pane",
                extra: { "child_conversation_id" => ?child_conversation_id }
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
                Some(child_conversation_id),
                AgentViewEntryOrigin::SharedSessionSelection,
                ctx,
            );
            // Shared-session viewer is `is_cloud_mode=false`, so
            // `ambient_agent_view_model()` is typically `None`. Update
            // opportunistically; the network's `JoinedSuccessfully` is the
            // authoritative source for ambient agent state.
            if let Some(ambient_agent_view_model) = terminal_view
                .ambient_agent_view_model()
                .into_optional_handle()
                .cloned()
            {
                ambient_agent_view_model.update(ctx, |model, ctx| {
                    model.set_conversation_id(Some(child_conversation_id));
                    if let Some(task_id) = child_task_id {
                        model.enter_viewing_existing_session(task_id, ctx);
                    }
                });
            }
        });

        self.child_agent_panes
            .insert(child_conversation_id, new_pane_id.into());
        // If the discarded fallback was occupying a tree slot via temporary
        // replacement, re-swap so the user lands on the new pane.
        if let Some(anchor) = fallback_was_swapped_anchor {
            self.swap_active_pane_to_conversation(anchor, child_conversation_id, ctx);
        }
    }
}
