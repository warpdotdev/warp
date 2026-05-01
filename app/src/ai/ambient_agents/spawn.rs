//! Stream-based API for spawning and monitoring ambient agents.
#![cfg_attr(target_family = "wasm", expect(dead_code))]

use std::{str::FromStr, sync::Arc, time::Duration};

use futures::{select, FutureExt, Stream};
use session_sharing_protocol::common::SessionId;

use super::AmbientAgentTaskId;
use super::{AmbientAgentTask, AmbientAgentTaskState};
use crate::{
    server::server_api::ai::{AIClient, SpawnAgentRequest, TaskStatusMessage},
    terminal::shared_session,
};

/// How long to poll for the agent to be ready.
/// This should be long enough that the shared session will be joinable.
pub const TASK_STATUS_POLLING_DURATION: Duration = Duration::from_secs(80);

#[cfg(not(test))]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const TASK_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Information about a session join link for an ambient agent task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionJoinInfo {
    pub session_id: Option<SessionId>,
    pub session_link: String,
}

impl SessionJoinInfo {
    pub fn from_task(task: &AmbientAgentTask) -> Option<Self> {
        let run_execution = task.active_run_execution();
        // Prefer the server-provided session_link when available; it is a better signal
        // that a session-sharing link is ready to be shown to the user.
        if let Some(link) = run_execution.session_link {
            let session_id = run_execution
                .session_id
                .and_then(|session_id| SessionId::from_str(session_id).ok());
            return Some(Self {
                session_id,
                session_link: link.to_string(),
            });
        }

        // Fallback to constructing a link from the session_id.
        if let Some(session_id) = run_execution.session_id {
            if let Ok(session_id) = SessionId::from_str(session_id) {
                return Some(Self {
                    session_id: Some(session_id),
                    session_link: shared_session::join_link(&session_id),
                });
            }
        }

        None
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

        // Poll for the task until it completes OR has session join info.
        // We use a timeout to ensure we don't wait indefinitely for session info.
        // If no timeout is provided, we use a future that never completes.
        let mut timeout_timer = match timeout {
            Some(d) => warpui::r#async::Timer::after(d),
            None => warpui::r#async::Timer::never(),
        }.fuse();
        let mut last_state = None;
        loop {
            let mut poll_timer = warpui::r#async::Timer::after(TASK_STATUS_POLL_INTERVAL).fuse();

            select! {
                _ = timeout_timer => {
                    yield Ok(AmbientAgentEvent::TimedOut);
                    return;
                }
                _ = poll_timer => {
                    match ai_client.get_ambient_agent_task(&task_id).await {
                        Ok(task) => {
                            // Only emit a state-change event if the state has changed.
                            if last_state.as_ref() != Some(&task.state) {
                                last_state = Some(task.state.clone());
                                yield Ok(AmbientAgentEvent::StateChanged {
                                    state: task.state.clone(),
                                    status_message: task.status_message.clone(),
                                });
                            }

                            // Check if the task has completed or started sharing its session.
                            if task.state.is_terminal() {
                                // Task completed, stream ends.
                                return;
                            }

                            if task.state == AmbientAgentTaskState::InProgress {
                                if let Some(session_join_info) = SessionJoinInfo::from_task(&task) {
                                    yield Ok(AmbientAgentEvent::SessionStarted {
                                        session_join_info,
                                    });
                                    return;
                                }
                                // Continue polling.
                            } else {
                                log::info!("Agent {task_id} state: {:?}", task.state);
                                // Continue polling.
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
