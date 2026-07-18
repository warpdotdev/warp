//! Runtime-global state machine for the single per-runtime video recording.

use std::mem;

use ai::agent::action_result::StopRecordingResult;
use futures::channel::oneshot;
use instant::Instant;
use thiserror::Error;
use warpui::{Entity, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;

#[derive(Debug, Error)]
pub enum StartRecordingControllerError {
    #[error("A recording is already in progress in this runtime.")]
    AlreadyInProgress,
    #[error(
        "Recording '{recording_id}' is being finalized. Call stop_recording with that id before starting another recording."
    )]
    FinalizationInProgress { recording_id: String },
    #[error(
        "Recording '{recording_id}' has finalized, but its result has not been delivered. Call stop_recording with that id before starting another recording."
    )]
    FinalizedResultPendingDelivery { recording_id: String },
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Error)]
pub enum StopRecordingControllerError {
    #[error("No recording with id '{recording_id}'.")]
    RecordingNotFound { recording_id: String },
    #[error("Current conversation has not been synced to the server yet.")]
    ConversationNotSynced,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) struct ActiveRecording {
    pub(crate) id: String,
    pub(crate) conversation_id: AIConversationId,
    pub(crate) handle: computer_use::RecordingHandle,
    /// When capture went live; action offsets are measured from here.
    pub(crate) started_at: Instant,
    /// Action groups to burn into the video, in dispatch order.
    pub(crate) actions: Vec<computer_use::ActionLogEntry>,
}

enum RecordingState {
    Idle,
    Starting {
        conversation_id: AIConversationId,
    },
    Active(ActiveRecording),
    Finalizing {
        id: String,
        conversation_id: AIConversationId,
        waiters: Vec<oneshot::Sender<StopRecordingResult>>,
    },
    Finalized {
        id: String,
        conversation_id: AIConversationId,
        result: StopRecordingResult,
    },
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) enum FinalizationClaim {
    Claimed {
        recording: Box<ActiveRecording>,
        result_receiver: oneshot::Receiver<StopRecordingResult>,
    },
    InProgress(oneshot::Receiver<StopRecordingResult>),
    Finished(StopRecordingResult),
    NotFound,
}

pub struct RecordingController {
    state: RecordingState,
}

impl RecordingController {
    pub fn new() -> Self {
        Self {
            state: RecordingState::Idle,
        }
    }

    pub fn try_begin_start(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Result<(), StartRecordingControllerError> {
        match &self.state {
            RecordingState::Idle => {
                self.state = RecordingState::Starting { conversation_id };
                Ok(())
            }
            // Do not wait and start implicitly: the prior result remains
            // canonical until a matching explicit stop delivers it.
            RecordingState::Finalizing { id, .. } => {
                Err(StartRecordingControllerError::FinalizationInProgress {
                    recording_id: id.clone(),
                })
            }
            RecordingState::Finalized { id, .. } => Err(
                StartRecordingControllerError::FinalizedResultPendingDelivery {
                    recording_id: id.clone(),
                },
            ),
            RecordingState::Starting { .. } | RecordingState::Active(_) => {
                Err(StartRecordingControllerError::AlreadyInProgress)
            }
        }
    }

    pub fn finish_start(
        &mut self,
        recording_id: String,
        conversation_id: AIConversationId,
        handle: computer_use::RecordingHandle,
    ) {
        if matches!(
            self.state,
            RecordingState::Starting {
                conversation_id: owner
            } if owner == conversation_id
        ) {
            self.state = RecordingState::Active(ActiveRecording {
                id: recording_id,
                conversation_id,
                handle,
                started_at: Instant::now(),
                actions: Vec::new(),
            });
        }
    }

    /// Appends an overlay group when the active recording belongs to the
    /// originating conversation.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn record_action(&mut self, conversation_id: AIConversationId, labels: Vec<String>) {
        if !labels.is_empty() {
            if let RecordingState::Active(recording) = &mut self.state {
                if recording.conversation_id == conversation_id {
                    recording.actions.push(computer_use::ActionLogEntry {
                        offset: recording.started_at.elapsed(),
                        labels,
                    });
                }
            }
        }
    }

    pub fn abort_start(&mut self, conversation_id: AIConversationId) {
        if matches!(
            self.state,
            RecordingState::Starting {
                conversation_id: owner
            } if owner == conversation_id
        ) {
            self.state = RecordingState::Idle;
        }
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn claim_finalization_by_id(&mut self, recording_id: &str) -> FinalizationClaim {
        self.claim_matching_finalization(|id, _| id == recording_id)
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn claim_finalization_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Option<FinalizationClaim> {
        // A start has no recording ID yet, but its conversation can still
        // cancel the reservation before the recorder finishes starting.
        if matches!(
            self.state,
            RecordingState::Starting {
                conversation_id: owner
            } if owner == conversation_id
        ) {
            self.state = RecordingState::Idle;
            return None;
        }

        match self.claim_matching_finalization(|_, owner| owner == conversation_id) {
            FinalizationClaim::NotFound => None,
            claim => Some(claim),
        }
    }

    /// Applies the shared terminal transitions after the caller selects how a
    /// recording identity should match.
    fn claim_matching_finalization(
        &mut self,
        matches: impl Fn(&str, AIConversationId) -> bool,
    ) -> FinalizationClaim {
        match mem::replace(&mut self.state, RecordingState::Idle) {
            RecordingState::Active(recording)
                if matches(&recording.id, recording.conversation_id) =>
            {
                let (sender, receiver) = oneshot::channel();
                self.state = RecordingState::Finalizing {
                    id: recording.id.clone(),
                    conversation_id: recording.conversation_id,
                    waiters: vec![sender],
                };
                FinalizationClaim::Claimed {
                    recording: Box::new(recording),
                    result_receiver: receiver,
                }
            }
            RecordingState::Finalizing {
                id,
                conversation_id,
                mut waiters,
            } if matches(&id, conversation_id) => {
                let (sender, receiver) = oneshot::channel();
                waiters.push(sender);
                self.state = RecordingState::Finalizing {
                    id,
                    conversation_id,
                    waiters,
                };
                FinalizationClaim::InProgress(receiver)
            }
            RecordingState::Finalized {
                id,
                conversation_id,
                result,
            } if matches(&id, conversation_id) => {
                let ready = result.clone();
                self.state = RecordingState::Finalized {
                    id,
                    conversation_id,
                    result,
                };
                FinalizationClaim::Finished(ready)
            }
            state => {
                self.state = state;
                FinalizationClaim::NotFound
            }
        }
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn complete_finalization(
        &mut self,
        recording_id: &str,
        result: StopRecordingResult,
    ) {
        match mem::replace(&mut self.state, RecordingState::Idle) {
            RecordingState::Finalizing {
                id,
                conversation_id,
                waiters,
            } if id == recording_id => {
                self.state = RecordingState::Finalized {
                    id,
                    conversation_id,
                    result: result.clone(),
                };
                for waiter in waiters {
                    let _ = waiter.send(result.clone());
                }
            }
            state => self.state = state,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn consume_finalized(&mut self, recording_id: &str) {
        match mem::replace(&mut self.state, RecordingState::Idle) {
            RecordingState::Finalized { id, .. } if id == recording_id => {}
            state => self.state = state,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn poll_active_exit(
        &mut self,
        recording_id: &str,
    ) -> Option<computer_use::RecordingExitKind> {
        match &mut self.state {
            RecordingState::Active(recording) if recording.id == recording_id => {
                recording.handle.poll_exit()
            }
            RecordingState::Idle
            | RecordingState::Starting { .. }
            | RecordingState::Active(_)
            | RecordingState::Finalizing { .. }
            | RecordingState::Finalized { .. } => None,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn active_recording_id(&self) -> Option<&str> {
        match &self.state {
            RecordingState::Active(recording) => Some(&recording.id),
            RecordingState::Idle
            | RecordingState::Starting { .. }
            | RecordingState::Finalizing { .. }
            | RecordingState::Finalized { .. } => None,
        }
    }
}

impl Entity for RecordingController {
    type Event = ();
}

impl SingletonEntity for RecordingController {}

#[cfg(test)]
#[path = "recording_controller_tests.rs"]
mod tests;
