//! Drives the orchestration pill bar in shared session viewers.
//!
//! After the viewer joins a parent ambient-agent session, [`OrchestrationViewerModel`]:
//!
//! 1. Calls `GET /agent/runs?ancestor_run_id={task_id}` to discover child agents.
//! 2. Creates a local conversation for each child via [`BlocklistAIHistoryModel`]
//!    marked as `is_viewing_shared_session = true` so server-side status
//!    reporters do not echo viewer-side state back to the server.
//! 3. Polls the children list periodically (~5s) until all reach a terminal
//!    state, updating each child conversation's [`ConversationStatus`] when the
//!    server-side state changes.
//! 4. When a child first reports a `session_id`, emits a
//!    [`crate::terminal::Event::EnsureSharedSessionViewerChildPane`] on the
//!    parent's `TerminalView` so the pane group can materialize a hidden
//!    shared-session viewer pane for that child. Each hidden child pane owns
//!    its own `TerminalView`, `BlocklistAIController`, and viewer-side
//!    `Network`, so child-session traffic never crosses the parent
//!    controller's single-stream state.
//! 5. Pill clicks navigate via `SwapPaneToConversation` (the existing
//!    local-orchestration mechanism), swapping the parent pane for the
//!    hidden child pane.
use std::collections::HashMap;
use std::time::Duration;

use session_sharing_protocol::common::SessionId;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{Entity, EntityId, ModelContext, SingletonEntity, WeakViewHandle};

use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ai::TaskListFilter;
use crate::server::server_api::ServerApiProvider;
use crate::terminal::{Event as TerminalViewEvent, TerminalView};

/// Max child runs per `GET /agent/runs?ancestor_run_id=` page.
const CHILD_DISCOVERY_FETCH_LIMIT: i32 = 100;
/// Poll cadence while at least one child is non-terminal.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Slower cadence once every known child is terminal. We don't stop
/// polling entirely because follow-up input can spawn new children.
const STATUS_POLL_INTERVAL_IDLE: Duration = Duration::from_secs(30);

/// Per-child orchestration metadata, keyed by `AmbientAgentTaskId`.
struct ChildAgentEntry {
    conversation_id: AIConversationId,
    /// Server-side session id; `None` until execution has been claimed.
    session_id: Option<SessionId>,
    /// Most recent state observed; compared against fresh polls to decide
    /// whether to push a `ConversationStatus` update.
    last_state: AmbientAgentTaskState,
    /// True once we've emitted `EnsureSharedSessionViewerChildPane` for
    /// this child, so re-polls don't spam the event bus.
    pane_materialization_requested: bool,
}

/// Owns child discovery + status polling for a shared session viewer of an
/// orchestrated session.
pub struct OrchestrationViewerModel {
    /// `ancestor_run_id` filter for REST fetches: the orchestrator's own
    /// run id.
    parent_task_id: AmbientAgentTaskId,
    /// Owns the child conversations and anchors the orchestrator lookup.
    terminal_view_id: EntityId,
    /// Used to emit `EnsureSharedSessionViewerChildPane` on the parent's
    /// view when a child becomes joinable.
    terminal_view: WeakViewHandle<TerminalView>,
    children: HashMap<AmbientAgentTaskId, ChildAgentEntry>,
    /// Aborted and replaced by every `schedule_next_poll` so we never have
    /// more than one timer chain in flight.
    polling_handle: Option<SpawnedFutureHandle>,
    /// Bumped before each fetch; stale responses (older generation) are
    /// dropped so a slow timer-fired fetch can't clobber a fresher kick.
    fetch_generation: u64,
    /// Set when the most recent fetch returned no children; we wait for
    /// an `AppendedExchange` on the orchestrator before polling again.
    /// Distinct from the in-flight state (`polling_handle = None`).
    idle_due_to_no_children: bool,
}

impl Entity for OrchestrationViewerModel {
    type Event = ();
}

impl OrchestrationViewerModel {
    /// Builds a viewer model attached to the given parent shared session.
    ///
    /// Kicks off the initial children fetch and schedules the first poll.
    pub fn new(
        parent_task_id: AmbientAgentTaskId,
        terminal_view_id: EntityId,
        terminal_view: WeakViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Kick to fast cadence on `AppendedExchange` so follow-up input
        // that spawns new children surfaces without waiting for the next
        // 30s idle poll.
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), |me, event, ctx| {
            me.maybe_kick_polling(event, ctx);
            me.maybe_backfill_parent_agent_ids(event, ctx);
        });

        let mut model = Self {
            parent_task_id,
            terminal_view_id,
            terminal_view,
            children: HashMap::new(),
            polling_handle: None,
            fetch_generation: 0,
            idle_due_to_no_children: false,
        };

        // Each fetch reschedules itself via its response callback.
        model.fetch_children(ctx);
        model
    }

    /// Schedules the next poll: fast cadence while any child is
    /// non-terminal, slow once all are terminal. Skipped while
    /// [`Self::idle_due_to_no_children`] is set; [`Self::maybe_kick_polling`]
    /// resumes on the next orchestrator `AppendedExchange`.
    fn schedule_next_poll(&mut self, ctx: &mut ModelContext<Self>) {
        // `SpawnedFutureHandle` doesn't abort on drop, so abort
        // explicitly to avoid stacking parallel timer chains.
        if let Some(prior) = self.polling_handle.take() {
            prior.abort();
        }

        // Stay idle until an `AppendedExchange` on the orchestrator wakes
        // us up. `apply_children_fetch` is responsible for setting this
        // flag when an empty descendant list comes back.
        if self.idle_due_to_no_children {
            return;
        }

        let all_terminal = !self.children.is_empty()
            && self
                .children
                .values()
                .all(|child| child.last_state.is_terminal());
        let interval = if all_terminal {
            STATUS_POLL_INTERVAL_IDLE
        } else {
            STATUS_POLL_INTERVAL
        };

        let handle = ctx.spawn(
            async move {
                Timer::after(interval).await;
            },
            |me, _, ctx| me.fetch_children(ctx),
        );
        self.polling_handle = Some(handle);
    }

    /// Tightens polling on `AppendedExchange` during the idle→active
    /// transition, and resumes from `idle_due_to_no_children` on an
    /// orchestrator-scoped exchange. The idle-resume check runs first
    /// because it would otherwise be conflated with the
    /// "fetch in flight" state by the `polling_handle.is_none()` guard.
    fn maybe_kick_polling(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let BlocklistAIHistoryEvent::AppendedExchange {
            conversation_id, ..
        } = event
        else {
            return;
        };
        let conversation_id = *conversation_id;
        let is_orchestrator = self.find_parent_conversation_id(ctx) == Some(conversation_id);

        // Resume from idle-due-to-no-children. Only orchestrator-scoped
        // exchanges count: child events are ignored because we have no
        // tracked children to update yet, and an unrelated conversation's
        // exchange does not imply this orchestrator just spawned a child.
        if self.idle_due_to_no_children {
            if is_orchestrator {
                self.idle_due_to_no_children = false;
                if let Some(prior) = self.polling_handle.take() {
                    prior.abort();
                }
                self.fetch_children(ctx);
            }
            return;
        }

        let all_terminal = !self.children.is_empty()
            && self
                .children
                .values()
                .all(|child| child.last_state.is_terminal());
        if !all_terminal {
            return;
        }
        // `polling_handle = None` here means a kick fetch is already in
        // flight (the idle-due-to-no-children case is handled above);
        // skipping prevents pile-up when exchanges arrive in bursts.
        if self.polling_handle.is_none() {
            return;
        }
        let is_tracked_child = self
            .children
            .values()
            .any(|child| child.conversation_id == conversation_id);
        if !is_orchestrator && !is_tracked_child {
            return;
        }
        if let Some(prior) = self.polling_handle.take() {
            prior.abort();
        }
        self.fetch_children(ctx);
    }

    /// Backfills `parent_agent_id` on viewer-created children once the
    /// orchestrator receives its server token / run id. First-poll
    /// children are created with `parent_agent_id = None` because the
    /// orchestrator hasn't been identified yet; this fixes them up so
    /// `parent_conversation_id` resolution works.
    fn maybe_backfill_parent_agent_ids(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
            conversation_id, ..
        } = event
        else {
            return;
        };
        let conversation_id = *conversation_id;
        if self.find_parent_conversation_id(ctx) != Some(conversation_id) {
            return;
        }
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        let parent_agent_id = history_handle
            .as_ref(ctx)
            .conversation(&conversation_id)
            .and_then(|c| c.orchestration_agent_id());
        let Some(parent_agent_id) = parent_agent_id else {
            return;
        };
        let child_conversation_ids: Vec<AIConversationId> = self
            .children
            .values()
            .map(|child| child.conversation_id)
            .collect();
        history_handle.update(ctx, |history, _ctx| {
            for child_id in child_conversation_ids {
                let Some(child) = history.conversation_mut(&child_id) else {
                    continue;
                };
                if child.parent_agent_id().is_some() {
                    continue;
                }
                child.set_parent_agent_id(parent_agent_id.clone());
            }
        });
    }

    /// Issues a `GET /agent/runs?ancestor_run_id={parent_task_id}` request
    /// and routes the response into [`Self::apply_children_fetch`]. Errors
    /// are logged and ignored; the next poll retries.
    fn fetch_children(&mut self, ctx: &mut ModelContext<Self>) {
        // Bump generation BEFORE dispatch so any in-flight stale fetch
        // is invalidated when its response callback compares.
        self.fetch_generation = self.fetch_generation.wrapping_add(1);
        let fetch_generation = self.fetch_generation;

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let filter = TaskListFilter {
            ancestor_run_id: Some(self.parent_task_id.to_string()),
            ..TaskListFilter::default()
        };
        let parent_task_id = self.parent_task_id;

        ctx.spawn(
            async move {
                ai_client
                    .list_ambient_agent_tasks(CHILD_DISCOVERY_FETCH_LIMIT, filter)
                    .await
            },
            move |me, result, ctx| {
                // Stale fetch: a newer one's already in flight (or applied).
                // The newer fetch owns rescheduling.
                if me.fetch_generation != fetch_generation {
                    return;
                }
                match result {
                    Ok(tasks) => me.apply_children_fetch(tasks, ctx),
                    Err(err) => {
                        log::warn!(
                            "OrchestrationViewerModel: failed to fetch children for {parent_task_id}: {err:#}"
                        );
                    }
                }
                // Always reschedule (even on error) so transient failures
                // don't break the polling loop.
                me.schedule_next_poll(ctx);
            },
        );
    }

    /// Consumes a children-list response: creates conversations for new
    /// children, updates status / session_id on existing ones, and
    /// requests pane materialization for any child whose `session_id` is
    /// freshly known.
    fn apply_children_fetch(&mut self, tasks: Vec<AmbientAgentTask>, ctx: &mut ModelContext<Self>) {
        let history_handle = BlocklistAIHistoryModel::handle(ctx);

        // Collect materialization requests to dispatch outside the
        // `&mut self.children` borrow.
        let mut to_materialize: Vec<(AIConversationId, SessionId)> = Vec::new();

        for task in tasks {
            // `ancestor_run_id` returns every descendant. Trust it for
            // membership (locally-spawned children may have empty or
            // sibling `parent_run_id`s), only skipping the parent itself.
            if task.task_id == self.parent_task_id {
                continue;
            }

            let task_id = task.task_id;
            let session_id = task
                .session_id
                .as_deref()
                .and_then(|s| s.parse::<SessionId>().ok());
            let new_state = task.state.clone();
            let conversation_status = conversation_status_from_state(&new_state);

            if let Some(entry) = self.children.get_mut(&task_id) {
                // Existing child: update status if it changed and fill in
                // session id once it becomes available.
                if entry.last_state != new_state {
                    let conversation_id = entry.conversation_id;
                    let terminal_view_id = self.terminal_view_id;
                    let status_for_update = conversation_status.clone();
                    history_handle.update(ctx, |history, ctx| {
                        history.update_conversation_status(
                            terminal_view_id,
                            conversation_id,
                            status_for_update,
                            ctx,
                        );
                    });
                    entry.last_state = new_state;
                }
                let was_missing_session_id = entry.session_id.is_none();
                if entry.session_id.is_none() {
                    entry.session_id = session_id;
                }
                // Queue a materialization request for this child if its
                // `session_id` just became known and we haven't requested
                // it before.
                if was_missing_session_id && entry.session_id.is_some() {
                    let conversation_id = entry.conversation_id;
                    let sid = entry.session_id.expect("session_id checked just above");
                    if !entry.pane_materialization_requested {
                        entry.pane_materialization_requested = true;
                        to_materialize.push((conversation_id, sid));
                    }
                }
                continue;
            }

            // New child: register under the orchestrator's local
            // conversation. Without it, `start_new_child_conversation`
            // would lose the parent linkage. Retry on the next poll.
            let Some(parent_conversation_id) = self.find_parent_conversation_id(ctx) else {
                continue;
            };

            let name = task.display_name().to_string();
            // Trim to stay in sync with `display_name()`, which also trims;
            // the descriptive title flows through `set_fallback_display_title`
            // so `AIConversation::title()` keeps surfacing it.
            let fallback_title = task.title.trim().to_string();
            let harness = task
                .agent_config_snapshot
                .as_ref()
                .and_then(|c| c.harness.as_ref())
                .map(|h| h.harness_type);
            let terminal_view_id = self.terminal_view_id;
            let status_for_initial = conversation_status.clone();

            let conversation_id = history_handle.update(ctx, |history, ctx| {
                let conversation_id = history.start_new_child_conversation(
                    terminal_view_id,
                    name,
                    parent_conversation_id,
                    harness,
                    ctx,
                );
                // Suppress server-side status reporting (viewer-side); also
                // disambiguates viewer-spawned children downstream.
                history.set_viewing_shared_session_for_conversation(conversation_id, true);
                if let Some(conversation) = history.conversation_mut(&conversation_id) {
                    if !fallback_title.is_empty() {
                        conversation.set_fallback_display_title(fallback_title);
                    }
                }
                // Stamp the child's `run_id` / `task_id` and populate the
                // `agent_id_to_conversation_id` index so transcript references
                // (received-message, send-message, lifecycle blocks) resolve
                // to this child via `conversation_id_for_agent_id`. Replaces
                // the earlier `set_task_id` call, which set the conversation
                // field but never updated the reverse index.
                history.assign_run_id_for_conversation(
                    conversation_id,
                    task_id.to_string(),
                    Some(task_id),
                    terminal_view_id,
                    ctx,
                );
                history.update_conversation_status(
                    terminal_view_id,
                    conversation_id,
                    status_for_initial,
                    ctx,
                );
                conversation_id
            });

            let pane_materialization_requested = session_id.is_some();
            if let Some(sid) = session_id {
                to_materialize.push((conversation_id, sid));
            }
            self.children.insert(
                task_id,
                ChildAgentEntry {
                    conversation_id,
                    session_id,
                    last_state: new_state,
                    pane_materialization_requested,
                },
            );
        }

        // Polling-cost mitigation: if no children are tracked after this
        // fetch, stop scheduling timers. The resume signal is an
        // `AppendedExchange` on the orchestrator (see
        // `maybe_kick_polling`). `schedule_next_poll` honours this flag
        // and bails before spawning a new timer.
        if self.children.is_empty() {
            self.idle_due_to_no_children = true;
            if let Some(prior) = self.polling_handle.take() {
                prior.abort();
            }
        } else {
            self.idle_due_to_no_children = false;
        }

        // Dispatch materialization events outside the children-borrow.
        for (conversation_id, session_id) in to_materialize {
            self.request_child_pane_materialization(conversation_id, session_id, ctx);
        }
    }

    /// Resolves the orchestrator's local conversation id via the view's
    /// active conversation, which `on_shared_init` sets on first join.
    fn find_parent_conversation_id(&self, ctx: &ModelContext<Self>) -> Option<AIConversationId> {
        BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.terminal_view_id)
    }

    /// Tells the parent's `TerminalView` to materialize a hidden
    /// shared-session viewer pane for this child.
    fn request_child_pane_materialization(
        &self,
        conversation_id: AIConversationId,
        session_id: SessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(view) = self.terminal_view.upgrade(ctx) else {
            log::warn!(
                "[orch-viewer] cannot request child pane materialization for conv={conversation_id:?}: \
                 parent terminal view is gone"
            );
            return;
        };
        view.update(ctx, |_view, ctx| {
            ctx.emit(TerminalViewEvent::EnsureSharedSessionViewerChildPane {
                conversation_id,
                session_id,
            });
        });
    }
}

/// Maps a server-side run state to the [`ConversationStatus`] used by the
/// pill bar and the conversation list. Working states (queued/pending/claimed/
/// in-progress) all collapse to [`ConversationStatus::InProgress`] so the
/// pill badge stays in the loading spinner until the run terminates.
fn conversation_status_from_state(state: &AmbientAgentTaskState) -> ConversationStatus {
    match state {
        AmbientAgentTaskState::Queued
        | AmbientAgentTaskState::Pending
        | AmbientAgentTaskState::Claimed
        | AmbientAgentTaskState::InProgress => ConversationStatus::InProgress,
        AmbientAgentTaskState::Succeeded => ConversationStatus::Success,
        AmbientAgentTaskState::Failed | AmbientAgentTaskState::Error => ConversationStatus::Error,
        AmbientAgentTaskState::Blocked => ConversationStatus::Blocked {
            blocked_action: String::new(),
        },
        AmbientAgentTaskState::Cancelled => ConversationStatus::Cancelled,
        // The `Unknown` variant is a forward-compat catch-all for server
        // states the client doesn't recognize yet. The rest of the codebase
        // (`is_terminal`, `is_failure_like`, `Display`, `status_icon_and_color`)
        // consistently treats it as a terminal error, so we follow suit.
        AmbientAgentTaskState::Unknown => ConversationStatus::Error,
    }
}

#[cfg(test)]
#[path = "orchestration_viewer_model_tests.rs"]
mod tests;
