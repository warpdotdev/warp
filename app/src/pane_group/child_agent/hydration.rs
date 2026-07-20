use warp_errors::report_error;
use warpui::{SingletonEntity, ViewContext};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::{
    AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskId,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::history_model::CloudConversationData;
use crate::pane_group::{AmbientAgentViewModelHandleExt, PaneGroup, PaneId};
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

/// How to hydrate a restored hidden remote-child pane given its
/// [`AmbientAgentTask`]. See [`decide_remote_child_hydration_action`].
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

/// Pure decision function backing [`PaneGroup::attempt_remote_child_hydration`].
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
    /// Task-backed restore path for the `is_remote_child` branch of
    /// `create_hidden_child_agent_pane`. Always creates the hidden ambient
    /// pane, registers it in `child_agent_panes` keyed by the placeholder's
    /// local `AIConversationId`, then dispatches via
    /// `attempt_remote_child_hydration` (or queues a pending entry while
    /// task data is fetched).
    ///
    /// Idempotent: skipped when the placeholder already has a live tracked
    /// pane, so repeat calls from `restore_missing_child_agent_panes_for_parent`
    /// — including while the initial async hydration is still in flight —
    /// don't create a duplicate hidden pane and orphan the first one.
    pub(super) fn hydrate_task_backed_hidden_child_pane(
        &mut self,
        child_conversation: AIConversation,
        parent_pane_id: PaneId,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_id = child_conversation.id();

        // Idempotency guard — see fn doc.
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
    /// [`decide_remote_child_hydration_action`]. Inspects the
    /// [`AmbientAgentTask`] directly because `resolve_open_action` collapses
    /// the navigate-to-local and hydrate-cloud-transcript intents into one
    /// variant once `conversations_by_id` carries the placeholder.
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

    /// Attaches the hidden child pane's ambient agent view model to the
    /// live ambient session for `task_id`. Wrapper around
    /// `AmbientAgentViewModel::enter_viewing_existing_session` that also
    /// sets the active conversation id.
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

    /// Fetches the cloud transcript identified by `server_token`, hydrates
    /// the placeholder via
    /// `hydrate_remote_child_placeholder_with_cloud_transcript`, and
    /// re-restores the merged conversation into the pane.
    /// `task_is_terminal` gates the conversation-ended tombstone in
    /// `attach_ambient_session_and_maybe_tombstone` so an
    /// `ActiveUnattachable` run isn't visually marked as ended.
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

    /// Post-match step for `hydrate_remote_child_transcript_in_place`:
    /// attach the live ambient session and insert the conversation-ended
    /// tombstone iff `task_is_terminal`. Centralised so the gate stays
    /// consistent across the Ok-merge / Err-merge / non-Oz fallback arms.
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
    /// data is now available, hydrating each hidden child pane in place.
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
            let Some(placeholder_conversation_id) =
                self.pending_remote_child_hydrations.remove(&task_id)
            else {
                continue;
            };
            self.attempt_remote_child_hydration(placeholder_conversation_id, task_id, ctx);
        }
    }
}
