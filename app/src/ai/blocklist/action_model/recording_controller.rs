//! Runtime-global state machine for the single per-runtime video recording.

use std::mem;
use std::time::Duration;

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
    /// The capture frame rate, used by the post-stop smart cut to enforce the
    /// one-source-frame minimum for instantaneous action groups.
    pub(crate) frame_rate: u32,
    /// The surface being recorded, used to resolve pointer-event coordinates
    /// into capture space for the post-stop burn-in.
    pub(crate) target: computer_use::Target,
    /// Action groups committed to the video, in completion order.
    pub(crate) actions: Vec<computer_use::ActionLogEntry>,
    /// Short agent-authored title shown in badges (from StartRecording.summary).
    pub(crate) summary: Option<String>,
    /// Optional longer description shown in detail views (from StartRecording.description).
    pub(crate) description: Option<String>,
    /// The currently in-flight `UseComputer` group, if any. It is committed with
    /// its finish offset on success or discarded on failure/cancellation.
    pub(crate) pending_group: Option<PendingActionGroup>,
}

/// A pending (in-flight) `UseComputer` action group: its start offset and labels
/// are captured when the call begins, and the entry is committed with its
/// finish offset only when the call's action sequence returns successfully.
/// Failed or cancelled calls discard the pending group without committing.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) struct PendingActionGroup {
    pub(crate) start_offset: Duration,
    pub(crate) labels: Vec<String>,
}

enum RecordingState {
    Idle,
    Starting {
        conversation_id: AIConversationId,
    },
    // Boxed so the `Active` variant (which carries the recording handle and
    // action log) does not balloon the enum's overall size.
    Active(Box<ActiveRecording>),
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

    #[allow(clippy::too_many_arguments)]
    pub fn finish_start(
        &mut self,
        recording_id: String,
        conversation_id: AIConversationId,
        handle: computer_use::RecordingHandle,
        frame_rate: u32,
        summary: Option<String>,
        description: Option<String>,
        target: computer_use::Target,
    ) {
        if matches!(
            self.state,
            RecordingState::Starting {
                conversation_id: owner
            } if owner == conversation_id
        ) {
            self.state = RecordingState::Active(Box::new(ActiveRecording {
                id: recording_id,
                conversation_id,
                handle,
                started_at: Instant::now(),
                frame_rate,
                target,
                actions: Vec::new(),
                summary,
                description,
                pending_group: None,
            }));
        }
    }

    /// Begins an in-flight `UseComputer` action group for the owning
    /// conversation, recording the group's start offset and labels. Returns the
    /// recording's capture start instant so the caller can measure the finish
    /// offset from the same clock when the action sequence returns. A
    /// pointer-only group is begun with empty labels; wait-only/no-op calls
    /// should not call this. The pending group is committed with its finish
    /// offset on success ([`commit_action_group`]) or discarded on failure
    /// ([`discard_action_group`]). Returns `None` (and begins nothing) if no
    /// recording is active for this conversation.
    ///
    /// [`commit_action_group`]: Self::commit_action_group
    /// [`discard_action_group`]: Self::discard_action_group
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn begin_action_group(
        &mut self,
        conversation_id: AIConversationId,
        labels: Vec<String>,
    ) -> Option<(Instant, computer_use::Target)> {
        if let RecordingState::Active(recording) = &mut self.state
            && recording.conversation_id == conversation_id
        {
            // If a prior group was never committed or discarded, auto-commit it
            // with the current clock as its implicit finish offset. This can
            // happen when a `UseComputer` call completes and `begin_action_group`
            // is called for the next call before `commit_action_group` fires.
            if let Some(pending) = recording.pending_group.take() {
                let implicit_finish = recording.started_at.elapsed().max(pending.start_offset);
                // Defensive fallback: in the normal flow the executor commits or
                // discards each group in its completion callback before the next
                // `begin`, so this rarely fires. The prior group's pointer events
                // live in that call's own buffer and are not reachable here, so
                // this path keeps the labels but no pointer geometry.
                recording.actions.push(computer_use::ActionLogEntry {
                    offset: pending.start_offset,
                    finish_offset: implicit_finish,
                    labels: pending.labels,
                    pointer_events: Vec::new(),
                });
            }
            let start_offset = recording.started_at.elapsed();
            recording.pending_group = Some(PendingActionGroup {
                start_offset,
                labels,
            });
            return Some((recording.started_at, recording.target));
        }
        None
    }

    /// Commits the in-flight action group with its finish offset, derived from
    /// the capture start instant returned by [`begin_action_group`]. The finish
    /// is clamped to be no earlier than the start so the segment builder's
    /// one-frame minimum can apply. No-op if the recording is no longer active
    /// for this conversation (for example it was finalized while the action was
    /// in flight), so a late commit from a completed call never lands on the
    /// wrong recording.
    ///
    /// [`begin_action_group`]: Self::begin_action_group
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn commit_action_group(
        &mut self,
        conversation_id: AIConversationId,
        finish_offset: Duration,
        pointer_events: Vec<computer_use::PointerEvent>,
    ) {
        if let RecordingState::Active(recording) = &mut self.state
            && recording.conversation_id == conversation_id
            && let Some(pending) = recording.pending_group.take()
        {
            let finish_offset = finish_offset.max(pending.start_offset);
            recording.actions.push(computer_use::ActionLogEntry {
                offset: pending.start_offset,
                finish_offset,
                labels: pending.labels,
                pointer_events,
            });
        }
    }

    /// Discards the in-flight action group without committing it (a failed or
    /// cancelled `UseComputer` call). No-op if the recording is no longer active
    /// for this conversation.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn discard_action_group(&mut self, conversation_id: AIConversationId) {
        if let RecordingState::Active(recording) = &mut self.state
            && recording.conversation_id == conversation_id
        {
            recording.pending_group = None;
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
                    recording,
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
