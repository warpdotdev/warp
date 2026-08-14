use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use instant::Instant;
use session_sharing_protocol::common::SessionId;
use uuid::Uuid;
use warp_errors::report_error;
use warpui::r#async::Timer;
use warpui::{SingletonEntity, ViewContext};

use super::{HiddenChildAgentTaskContext, apply_hidden_child_agent_task_context};
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::AmbientAgentTask;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::orchestration_event_streamer::agent_task_harness;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::features::FeatureFlag;
use crate::pane_group::{
    AmbientAgentViewModelHandleExt, PaneGroup, PaneId, PendingParentChildSeed, TerminalPane,
    TerminalViewResources,
};
use crate::server::retry_strategies::is_transient_http_error;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::TaskListFilter;
use crate::terminal::shared_session::IsSharedSessionCreator;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

/// Max direct children fetched per ancestor-list restore seed. The server
/// caps at 100 regardless, matching the Observer-side ancestor seed fetch.
/// A parent with more direct children than this only has its first page
/// discovered, since this path does not paginate.
const RESTORE_CHILD_SEED_FETCH_LIMIT: i32 = 100;

/// How long to wait before retrying a parent's ancestor-list fetch after a
/// transient error (network blip, 5xx, 408, 429), so a parent isn't left
/// stranded pending forever, never linking its children, purely because
/// nothing external happens to trigger a retry.
const TRANSIENT_SEED_FETCH_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Returns true if a fetch dispatched at `dispatched_at` must be dropped
/// instead of applied: its seed is gone, or a newer fetch has since been
/// dispatched for the same parent (the seed was removed and a new one
/// created for the same `parent_task_id` while this fetch was still in
/// flight, or a retry raced ahead of it).
pub(in crate::pane_group) fn is_stale_ancestor_list_completion(
    current_seed: Option<&PendingParentChildSeed>,
    dispatched_at: Instant,
) -> bool {
    current_seed.is_none_or(|seed| seed.in_flight_fetch_started_at != Some(dispatched_at))
}

impl PaneGroup {
    /// Lazily restores hidden child panes for the given parent conversation.
    ///
    /// Unlike the old startup sweep, this runs only when the parent agent view
    /// is actually restored or entered. Children that already belong to some
    /// other pane or tab are left alone.
    ///
    /// `trigger_seed_if_empty` gates the local-parent ancestor-list seed
    /// below: pass `true` only from entry points that are *not* themselves
    /// downstream of `finish_seed_child_conversations_from_task` completing.
    /// That function already calls this with `false` after linking children,
    /// so a fresh, still-empty result (a parent that legitimately has no
    /// children) doesn't immediately re-trigger its own seed and loop.
    pub(in crate::pane_group) fn restore_missing_child_agent_panes_for_parent(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_pane_id: PaneId,
        trigger_seed_if_empty: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let child_ids = BlocklistAIHistoryModel::as_ref(ctx)
            .child_conversation_ids_of(&parent_conversation_id)
            .to_vec();

        // Local (non-ambient) parents have no other discovery path for remote
        // children: unlike ambient/viewer restore, nothing else calls
        // `seed_child_conversations_from_task` for them. Always trigger the
        // ancestor-list seed when `trigger_seed_if_empty` is true, regardless
        // of how many remote children are already known locally — some may be
        // missing if the flag state changed between sessions. The seed is
        // idempotent (guarded by `pending_parent_child_seeds`) and a no-op for
        // parents that never spawned any remote children.
        if trigger_seed_if_empty {
            let parent_task_id = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&parent_conversation_id)
                .and_then(AIConversation::task_id);
            if let Some(parent_task_id) = parent_task_id
                && !self
                    .pending_parent_child_seeds
                    .contains_key(&parent_task_id)
            {
                self.seed_child_conversations_from_task(
                    parent_conversation_id,
                    parent_task_id,
                    ctx,
                );
            }
        }

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

    /// Rebuilds the parent→child conversation index for a restored cloud agent
    /// parent from the server's `?ancestor_run_id=` listing — the same query
    /// path the Observer-side ancestor seed (`spawn_ancestor_seed_fetch`) uses
    /// to discover children on cold start. This is the only pill-bar source
    /// on clients without cross-session SQLite (web) and on the first restore
    /// of a run whose parent was never persisted.
    ///
    /// Re-lists (coalesced — never more than one fetch in flight per parent)
    /// for as long as any reported child hasn't resolved locally yet; once
    /// every currently reported child has resolved, the parent is cleared
    /// and this stops. Idempotent: children that already resolve locally
    /// are left untouched, and calling this again for an already-pending
    /// parent is a no-op beyond the coalesced fetch dispatch.
    pub(in crate::pane_group) fn seed_child_conversations_from_task(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::OrchestrationUnifiedStack.is_enabled() {
            return;
        }

        // Mark pending until every direct child resolves locally.
        self.pending_parent_child_seeds
            .entry(parent_task_id)
            .or_insert_with(|| PendingParentChildSeed {
                parent_conversation_id,
                fetch_in_flight: false,
                in_flight_fetch_started_at: None,
                retry_handle: None,
            });
        self.ensure_pending_ambient_restoration_subscription(ctx);

        self.spawn_ancestor_list_fetch_if_needed(parent_task_id, ctx);
    }

    /// Spawns the `?ancestor_run_id=` fetch for `parent_task_id` unless one
    /// is already in flight, so a parent never has more than one request
    /// outstanding regardless of how often this is called.
    fn spawn_ancestor_list_fetch_if_needed(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(seed) = self.pending_parent_child_seeds.get(&parent_task_id) else {
            return;
        };
        if seed.fetch_in_flight {
            return;
        }
        let parent_conversation_id = seed.parent_conversation_id;
        let dispatched_at = Instant::now();
        if let Some(seed) = self.pending_parent_child_seeds.get_mut(&parent_task_id) {
            seed.fetch_in_flight = true;
            seed.in_flight_fetch_started_at = Some(dispatched_at);
        }

        #[cfg(test)]
        {
            self.parent_child_seed_fetch_dispatch_count += 1;
        }

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let filter = TaskListFilter {
            ancestor_run_id: Some(parent_task_id.to_string()),
            ..TaskListFilter::default()
        };
        ctx.spawn(
            async move {
                ai_client
                    .list_ambient_agent_tasks(RESTORE_CHILD_SEED_FETCH_LIMIT, filter)
                    .await
            },
            move |me, result, ctx| {
                if is_stale_ancestor_list_completion(
                    me.pending_parent_child_seeds.get(&parent_task_id),
                    dispatched_at,
                ) {
                    return;
                }
                if let Some(seed) = me.pending_parent_child_seeds.get_mut(&parent_task_id) {
                    seed.fetch_in_flight = false;
                }
                me.finish_seed_child_conversations_from_task(
                    parent_conversation_id,
                    parent_task_id,
                    result,
                    ctx,
                );
            },
        );
    }

    /// Applies an ancestor-list fetch result: links every reported direct
    /// child under `parent_conversation_id`, and clears the pending entry
    /// once every child's own task data has resolved. Must stay pending
    /// (not be treated as a one-shot cache) while any reported child
    /// hasn't resolved yet, so a child spawned while others are still
    /// resolving is still picked up by a later re-list.
    pub(in crate::pane_group) fn finish_seed_child_conversations_from_task(
        &mut self,
        parent_conversation_id: AIConversationId,
        parent_task_id: AmbientAgentTaskId,
        result: anyhow::Result<Vec<AmbientAgentTask>>,
        ctx: &mut ViewContext<Self>,
    ) {
        let children = match result {
            Ok(children) => children,
            Err(err) => {
                log::warn!(
                    "seed_child_conversations_from_task: ancestor-list fetch failed for \
                     parent_task_id={parent_task_id}: {err:#}"
                );
                if is_transient_http_error(&err) {
                    // Guarantee a retry so the parent isn't stranded pending
                    // forever, silently never linking its children.
                    self.schedule_transient_seed_fetch_retry(parent_task_id, ctx);
                } else {
                    // Permanent failure (401/403/404/...): retrying blindly
                    // can't succeed until something external changes, so
                    // give up instead of leaving this pending forever.
                    self.remove_pending_parent_child_seed(parent_task_id);
                }
                return;
            }
        };

        // No surface now means none will appear on its own, so give up
        // rather than leaving this parent pending and re-listing forever.
        let Some(terminal_surface_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&parent_conversation_id)
        else {
            log::warn!(
                "seed_child_conversations_from_task: parent conversation \
                 {parent_conversation_id:?} has no terminal surface; giving up"
            );
            self.remove_pending_parent_child_seed(parent_task_id);
            return;
        };

        // Children whose task data is still being fetched keep the parent
        // pending.
        let mut all_children_resolved = true;
        for child_run_id in children
            .iter()
            .map(|task| task.task_id)
            .filter(|task_id| *task_id != parent_task_id)
        {
            let child_task = AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.get_or_async_fetch_task_data(&child_run_id, ctx)
            });
            let Some(child_task) = child_task else {
                all_children_resolved = false;
                continue;
            };

            let name = child_task.display_name().to_string();
            let fallback_title = child_task.title.trim().to_string();
            let harness = agent_task_harness(&child_task);
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.ensure_remote_child_conversation(
                    terminal_surface_id,
                    parent_conversation_id,
                    child_run_id.to_string(),
                    child_task.task_id,
                    name,
                    fallback_title,
                    harness,
                    ctx,
                )
            });
        }

        if all_children_resolved {
            self.remove_pending_parent_child_seed(parent_task_id);
        }
        // Otherwise the parent stays pending so a later re-list can retry.

        // Pills render straight off the conversation index, so a parent pane
        // that isn't resolvable yet is not an error — children materialize
        // lazily on click.
        if let Some(parent_pane_id) =
            self.pane_id_for_owned_conversation(parent_conversation_id, ctx)
        {
            // `false`: this call is itself the completion of a seed fetch,
            // so an empty result here must not immediately kick off another
            // one (that would loop forever for a parent with no children).
            self.restore_missing_child_agent_panes_for_parent(
                parent_conversation_id,
                parent_pane_id,
                false,
                ctx,
            );
        }
        ctx.notify();
    }

    /// Re-drives every pending parent seed using the shared `TasksUpdated`
    /// subscription: a (coalesced) `?ancestor_run_id=` fetch is dispatched
    /// for each.
    pub(in crate::pane_group) fn process_pending_parent_child_seeds(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::OrchestrationUnifiedStack.is_enabled()
            || self.pending_parent_child_seeds.is_empty()
        {
            return;
        }

        let parent_task_ids: Vec<AmbientAgentTaskId> =
            self.pending_parent_child_seeds.keys().copied().collect();
        for parent_task_id in parent_task_ids {
            self.spawn_ancestor_list_fetch_if_needed(parent_task_id, ctx);
        }
    }

    /// Schedules a one-shot retry of the ancestor-list fetch for
    /// `parent_task_id` after `TRANSIENT_SEED_FETCH_RETRY_DELAY`, aborting
    /// any previously scheduled retry first so repeated transient failures
    /// don't stack timers.
    fn schedule_transient_seed_fetch_retry(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(seed) = self.pending_parent_child_seeds.get_mut(&parent_task_id)
            && let Some(prior) = seed.retry_handle.take()
        {
            prior.abort();
        }
        let handle = ctx.spawn(
            async move {
                Timer::after(TRANSIENT_SEED_FETCH_RETRY_DELAY).await;
            },
            move |me, _, ctx| {
                if let Some(seed) = me.pending_parent_child_seeds.get_mut(&parent_task_id) {
                    seed.retry_handle = None;
                }
                me.spawn_ancestor_list_fetch_if_needed(parent_task_id, ctx);
            },
        );
        match self.pending_parent_child_seeds.get_mut(&parent_task_id) {
            Some(seed) => seed.retry_handle = Some(handle),
            // Removed synchronously between scheduling and now shouldn't
            // happen (we still hold `&mut self`), but abort defensively.
            None => handle.abort(),
        }
    }

    /// Removes a parent from `pending_parent_child_seeds`, aborting its
    /// scheduled transient-retry timer (if any) so it can't fire after the
    /// entry is gone.
    pub(in crate::pane_group) fn remove_pending_parent_child_seed(
        &mut self,
        parent_task_id: AmbientAgentTaskId,
    ) {
        if let Some(seed) = self.pending_parent_child_seeds.remove(&parent_task_id)
            && let Some(handle) = seed.retry_handle
        {
            handle.abort();
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
            true,
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
            true,
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
        let flag_on = FeatureFlag::OrchestrationUnifiedStack.is_enabled();

        if flag_on {
            // Viewer and owner children share one task-driven dispatch; only
            // local in-process children fall through to the branch below.
            if child_conversation.is_viewing_shared_session()
                || child_conversation.is_remote_child()
            {
                self.materialize_child_pane(child_conversation, ctx);
                return;
            }
        } else {
            // Viewer and owner children take separate dispatches.
            if child_conversation.is_viewing_shared_session() {
                let _ = self.create_child_loading_placeholder(
                    child_conversation,
                    AgentViewEntryOrigin::SharedSessionSelection,
                    ctx,
                );
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

        match self.terminal_view_from_pane_id(new_pane_id, ctx) {
            Some(new_terminal_view) => {
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
            }
            _ => {
                report_error!(
                    "Failed to get terminal view for child agent pane",
                    extra: { "child_id" => ?child_id }
                );
                self.discard_pane(new_pane_id.into(), ctx);
            }
        }
    }

    // =========================================================================
    // flag-OFF path (OrchestrationUnifiedStack disabled)
    // =========================================================================

    /// Materializes a hidden shared-session viewer pane for a viewer-
    /// discovered child agent when `OrchestrationUnifiedStack` is disabled.
    /// Triggered by `Event::EnsureSharedSessionViewerChildPane`, which
    /// `OrchestrationViewerModel` emits on the parent's view the first time
    /// it observes a `session_id` for a child.
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
