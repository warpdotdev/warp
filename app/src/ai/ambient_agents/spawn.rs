//! Stream-based API for spawning and monitoring ambient agents.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use std::{str::FromStr, sync::Arc, time::Duration};

use anyhow::anyhow;
use futures::{select, FutureExt, Stream, StreamExt};
use session_sharing_protocol::common::SessionId;

use super::AmbientAgentTaskId;
use super::{AmbientAgentTask, AmbientAgentTaskState};
use crate::{
    server::retry_strategies::with_bounded_retry,
    server::server_api::ai::{AIClient, RunFollowupRequest, SpawnAgentRequest, TaskStatusMessage},
    terminal::shared_session,
};

/// How long to poll for the agent to be ready.
/// This should be long enough that the shared session will be joinable.
pub const TASK_STATUS_POLLING_DURATION: Duration = Duration::from_secs(80);

#[cfg(not(test))]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(3);
#[cfg(test)]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Maximum number of consecutive polls that may observe a non-working, non-cancelled task
/// state for a follow-up before we give up and surface a failure. Bounds the worst-case
/// wait when the server is wedged and never transitions the task off its prior terminal
/// state. At the production `TASK_STATUS_POLL_INTERVAL` of 3s, 10 skipped observations
/// is ~30s — comfortably longer than the dispatcher's `ProcessingInterval` plus typical
/// worker claim latency.
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
        if let Err(err) = ai_client.submit_run_followup(&run_id, request).await {
            yield Err(err);
            return;
        }

        let mut stream = Box::pin(poll_run_until_joinable_session(
            run_id,
            ai_client,
            RunPollMode::Followup {
                previous_session_id,
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
        // For follow-ups, the server does NOT synchronously transition the task off its
        // prior terminal state when `submit_run_followup` returns — the transition happens
        // later, asynchronously, when the dispatcher loop picks up the newly enqueued
        // execution and the worker claims it. If we treated the first observed state as
        // the follow-up's outcome we'd misreport the prior run's `status_message` as a
        // failure and end the stream, leaving the model permanently stuck in `Failed`
        // even though the new run is actually about to start.
        //
        // To avoid that, gate event emission for follow-ups on having observed at least
        // one working state. Initial spawns don't need this — they start from a fresh
        // task whose first observation reflects the spawn itself.
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
                            } else if !seen_working_state
                                && task.state != AmbientAgentTaskState::Cancelled
                                && skipped_stale_polls < MAX_STALE_POLLS_BEFORE_FAILURE
                            {
                                // Likely the prior run's residual terminal state; the
                                // server hasn't transitioned the task yet. Skip without
                                // emitting events or ending the stream.
                                //
                                // Carve-outs:
                                // - `Cancelled` always falls through: it's only reached
                                //   via explicit user/admin/server cancellation and the
                                //   server will never transition out of it on its own.
                                // - After `MAX_STALE_POLLS_BEFORE_FAILURE` skipped polls
                                //   we give up so a wedged server can't keep the stream
                                //   alive indefinitely.
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
                                    let exhausted_stale_skips = !seen_working_state
                                        && skipped_stale_polls >= MAX_STALE_POLLS_BEFORE_FAILURE;
                                    let message = if exhausted_stale_skips {
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

                            if task.state == AmbientAgentTaskState::InProgress {
                                if let Some(session_join_info) = SessionJoinInfo::from_task(&task) {
                                    let has_new_session = match &mode {
                                        RunPollMode::InitialRun
                                        | RunPollMode::Followup {
                                            previous_session_id: None,
                                        } => true,
                                        RunPollMode::Followup {
                                            previous_session_id: Some(previous_session_id),
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
