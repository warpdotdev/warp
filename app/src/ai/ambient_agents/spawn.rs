//! Stream-based API for spawning and monitoring ambient agents.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use futures::{FutureExt, Stream, StreamExt, select};
use session_sharing_protocol::common::SessionId;

use super::{AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState};
use crate::server::retry_strategies::with_bounded_retry;
use crate::server::server_api::ai::{
    AIClient, RunFollowupRequest, SpawnAgentRequest, TaskStatusMessage,
};
use crate::terminal::shared_session;

/// How long to poll for the agent to be ready.
/// This should be long enough that the shared session will be joinable.
pub const TASK_STATUS_POLLING_DURATION: Duration = Duration::from_secs(80);

#[cfg(not(test))]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(3);
#[cfg(test)]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Maximum number of consecutive polls that may observe a follow-up's residual prior-run
/// state before we give up and surface a failure. `state_changed_at` normally settles this
/// in one poll; this is the backstop for a server that has not caught up yet (or does not
/// report the timestamp at all, in which case it is the primary mechanism). Bounds the
/// worst-case wait when the server is wedged and never transitions the task off its prior
/// terminal state. At the production `TASK_STATUS_POLL_INTERVAL` of 3s, 10 skipped
/// observations is ~30s — comfortably longer than the dispatcher's `ProcessingInterval`
/// plus typical worker claim latency.
const MAX_STALE_POLLS_BEFORE_FAILURE: usize = 10;

/// Information about a session join link for an ambient agent task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionJoinInfo {
    pub session_id: Option<SessionId>,
    pub session_link: String,
}

impl SessionJoinInfo {
    pub fn from_task(task: &AmbientAgentTask) -> Option<Self> {
        let run_execution = task.active_run_execution();
        // The cloud-mode pane joins on `session_id`; a standalone `session_link` isn't
        // actionable without it.
        let session_id_str = run_execution.session_id?;
        let session_id = SessionId::from_str(session_id_str).ok()?;

        // Prefer the server-provided `session_link`; fall back to constructing one from
        // `session_id`. `active_run_execution()` already filters out empty links.
        let session_link = run_execution
            .session_link
            .map(String::from)
            .unwrap_or_else(|| shared_session::join_link(&session_id));
        Some(Self {
            session_id: Some(session_id),
            session_link,
        })
    }
}

/// Lifecycle events during ambient agent startup.
#[derive(Debug)]
pub enum AmbientAgentEvent {
    /// The task was successfully spawned with the given task ID and run ID.
    TaskSpawned {
        task_id: AmbientAgentTaskId,
        run_id: String,
    },
    /// The task state changed.
    StateChanged {
        state: AmbientAgentTaskState,
        status_message: Option<TaskStatusMessage>,
    },
    /// Session started and join information became available.
    SessionStarted { session_join_info: SessionJoinInfo },
    /// Timed out waiting for the agent session to be ready.
    TimedOut,
    /// Cloud agent capacity limit has been reached. This does not block
    /// the task from eventually starting.
    AtCapacity,
}

enum RunPollMode {
    InitialRun,
    Followup {
        previous_session_id: Option<SessionId>,
        /// When this follow-up was accepted, by the server's own clock. Compared
        /// against the server's `state_changed_at` on each poll so a residual
        /// observation of the prior run's terminal state (reported before this
        /// moment) can be told apart from a state change the follow-up's own
        /// execution actually caused.
        ///
        /// `None` when an older server accepted the follow-up without reporting
        /// `accepted_at` (either omitting the field or returning no body at all).
        /// Without it there is nothing to compare `state_changed_at` against, so
        /// polling falls back to the pre-timestamp heuristic instead.
        submitted_at: Option<DateTime<Utc>>,
    },
}

/// Spawns an ambient agent task and monitors its state.
///
/// The stream completes when:
/// - The task completes (either successfully or with a failure)
/// - The task's shared session is ready to join
/// - The timeout expires (if provided)
/// - An error occurs
///
/// If `timeout` is `None`, there is no timeout.
pub fn spawn_task(
    request: SpawnAgentRequest,
    ai_client: Arc<dyn AIClient>,
    timeout: Option<Duration>,
) -> impl Stream<Item = Result<AmbientAgentEvent, anyhow::Error>> {
    // We can't use try_stream! because of the select! macro invocation.
    // See https://github.com/tokio-rs/async-stream/issues/63.
    async_stream::stream! {
        // First, spawn the ambient agent task.
        let (task_id, run_id, at_capacity) = match ai_client.spawn_agent(request).await {
            Ok(response) => (response.task_id, response.run_id, response.at_capacity),
            Err(err) => {
                yield Err(err);
                return;
            },
        };

        let mut stream = Box::pin(monitor_spawned_task(
            task_id,
            run_id,
            at_capacity,
            ai_client,
            timeout,
        ));
        while let Some(event) = stream.next().await {
            yield event;
        }
    }
}

/// Monitors an ambient task that has already been created.
///
/// This preserves the event contract of [`spawn_task`] while allowing callers
/// that own task creation to avoid issuing a second spawn request.
pub fn monitor_spawned_task(
    task_id: AmbientAgentTaskId,
    run_id: String,
    at_capacity: bool,
    ai_client: Arc<dyn AIClient>,
    timeout: Option<Duration>,
) -> impl Stream<Item = Result<AmbientAgentEvent, anyhow::Error>> {
    async_stream::stream! {
        yield Ok(AmbientAgentEvent::TaskSpawned { task_id, run_id });

        // Emit AtCapacity event if the server indicates capacity limit reached.
        if at_capacity {
            yield Ok(AmbientAgentEvent::AtCapacity);
        }

        let mut stream = Box::pin(poll_run_until_joinable_session(
            task_id,
            ai_client,
            RunPollMode::InitialRun,
            timeout,
        ));
        while let Some(event) = stream.next().await {
            yield event;
        }
    }
}

pub fn submit_run_followup(
    message: String,
    run_id: AmbientAgentTaskId,
    previous_session_id: Option<SessionId>,
    ai_client: Arc<dyn AIClient>,
    timeout: Option<Duration>,
) -> impl Stream<Item = Result<AmbientAgentEvent, anyhow::Error>> {
    async_stream::stream! {
        let request = RunFollowupRequest { message };
        // The server's own clock at acceptance: its synchronous requeue (if any) happens
        // no earlier than this moment, so a `state_changed_at` at or after it can only
        // belong to this follow-up's own execution, never to the prior run. Using the
        // server's timestamp instead of a client-local one avoids misjudging that
        // comparison when the client and server clocks have drifted apart. An older
        // server that does not report `accepted_at` yields `None`, and polling falls
        // back to the pre-timestamp heuristic below.
        let submitted_at = match ai_client.submit_run_followup(&run_id, request).await {
            Ok(response) => response.accepted_at,
            Err(err) => {
                yield Err(err);
                return;
            },
        };

        let mut stream = Box::pin(poll_run_until_joinable_session(
            run_id,
            ai_client,
            RunPollMode::Followup {
                previous_session_id,
                submitted_at,
            },
            timeout,
        ));
        while let Some(event) = stream.next().await {
            yield event;
        }
    }
}

fn poll_run_until_joinable_session(
    run_id: AmbientAgentTaskId,
    ai_client: Arc<dyn AIClient>,
    mode: RunPollMode,
    timeout: Option<Duration>,
) -> impl Stream<Item = Result<AmbientAgentEvent, anyhow::Error>> {
    async_stream::stream! {
        // Poll for the task until it completes OR has session join info.
        // We use a timeout to ensure we don't wait indefinitely for session info.
        // If no timeout is provided, we use a future that never completes.
        let mut timeout_timer = FutureExt::fuse(match timeout {
            Some(d) => warpui::r#async::Timer::after(d),
            None => warpui::r#async::Timer::never(),
        });
        let mut last_state = None;
        // For follow-ups, a poll can still observe the prior run's residual terminal
        // state after the follow-up was accepted, because the server's own transition
        // to the new execution's state may not have landed (or replicated to whatever
        // this read hits) by the time this stream starts polling. If we treated that
        // observation as the follow-up's outcome we'd misreport the prior run's
        // `status_message` as a failure and end the stream, leaving the model
        // permanently stuck in `Failed` even though the new run is actually about to
        // start.
        //
        // `state_changed_at` is the authoritative signal: the server only moves it when
        // `state` itself changes, so a value at or after `submitted_at` can only belong
        // to a transition the follow-up caused, never to state left over from before it.
        // A server that does not yet report it (`state_changed_at: None`) falls back to
        // the older heuristic of waiting for a working state, kept for compatibility
        // during rollout. Initial spawns don't need either check — they start from a
        // fresh task whose first observation reflects the spawn itself.
        let mut seen_working_state = matches!(&mode, RunPollMode::InitialRun);
        let mut skipped_stale_polls: usize = 0;
        loop {
            let mut poll_timer = FutureExt::fuse(warpui::r#async::Timer::after(TASK_STATUS_POLL_INTERVAL));

            select! {
                _ = timeout_timer => {
                    yield Ok(AmbientAgentEvent::TimedOut);
                    return;
                }
                _ = poll_timer => {
                    // Wrap the status poll in with_bounded_retry so transient
                    // HTTP errors (429, 5xx) are retried with exponential
                    // backoff instead of immediately killing the CLI.
                    let poll_result = {
                        let client = ai_client.clone();
                        with_bounded_retry(
                            &format!("poll agent {run_id}"),
                            || {
                                let client = client.clone();
                                async move { client.get_ambient_agent_task(&run_id).await }
                            },
                        )
                        .await
                    };
                    match poll_result {
                        Ok(task) => {
                            // Log every non-InProgress observation BEFORE the skip check
                            // so we retain visibility into stalls where the server is
                            // wedged on the prior terminal state.
                            if task.state != AmbientAgentTaskState::InProgress {
                                log::info!("Agent {run_id} state: {:?}", task.state);
                            }

                            if task.state.is_working() {
                                seen_working_state = true;
                            }

                            // A residual observation of the prior run's state: authoritatively
                            // when both the server's `state_changed_at` and this follow-up's
                            // `submitted_at` are known and the former predates the latter, or by
                            // the fallback heuristic when either is unavailable (an older server
                            // omitting one or the other). Never true for a working state, nor for
                            // `InitialRun`.
                            let residual_prior_state = !task.state.is_working()
                                && match &mode {
                                    RunPollMode::InitialRun => false,
                                    RunPollMode::Followup { submitted_at, .. } => {
                                        match (task.state_changed_at, submitted_at) {
                                            (Some(state_changed_at), Some(submitted_at)) => {
                                                state_changed_at < *submitted_at
                                            }
                                            _ => {
                                                !seen_working_state
                                                    && task.state != AmbientAgentTaskState::Cancelled
                                            }
                                        }
                                    }
                                };

                            if residual_prior_state && skipped_stale_polls < MAX_STALE_POLLS_BEFORE_FAILURE {
                                // Skip without emitting events or ending the stream: the server
                                // hasn't reported the follow-up's own state yet. Bounded so a
                                // wedged server can't keep the stream alive indefinitely.
                                skipped_stale_polls += 1;
                                continue;
                            }

                            if last_state.as_ref() != Some(&task.state) {
                                last_state = Some(task.state.clone());
                                yield Ok(AmbientAgentEvent::StateChanged {
                                    state: task.state.clone(),
                                    status_message: task.status_message.clone(),
                                });
                            }

                            if task.state.is_terminal() {
                                if matches!(&mode, RunPollMode::Followup { .. }) {
                                    // Only a residual observation that exhausted the bounded skip
                                    // budget gets the synthetic message: everything else is a real
                                    // outcome for the follow-up's own execution and must surface
                                    // as such, including a genuine spawn failure.
                                    let message = if residual_prior_state {
                                        "Cloud follow-up did not start in time".to_string()
                                    } else {
                                        task.status_message
                                            .as_ref()
                                            .map(|msg| msg.message.clone())
                                            .unwrap_or_else(|| {
                                                if task.state.is_failure_like() {
                                                    "Cloud agent failed".to_string()
                                                } else {
                                                    "Cloud follow-up finished before a new session became available".to_string()
                                                }
                                            })
                                    };
                                    yield Err(anyhow!(message));
                                }
                                return;
                            }

                            if task.state == AmbientAgentTaskState::InProgress
                                && let Some(session_join_info) = SessionJoinInfo::from_task(&task) {
                                    let has_new_session = match &mode {
                                        RunPollMode::InitialRun
                                        | RunPollMode::Followup {
                                            previous_session_id: None,
                                            ..
                                        } => true,
                                        RunPollMode::Followup {
                                            previous_session_id: Some(previous_session_id),
                                            ..
                                        } => session_join_info
                                            .session_id
                                            .as_ref()
                                            .is_some_and(|session_id| session_id != previous_session_id),
                                    };
                                    if has_new_session {
                                        yield Ok(AmbientAgentEvent::SessionStarted {
                                            session_join_info,
                                        });
                                        return;
                                    }
                                }
                        }
                        Err(err) => {
                            yield Err(err);
                            return;
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
