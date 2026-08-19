//! Runtime-global state machine for the single per-runtime video recording.

use std::mem;
use std::path::Path;
use std::time::Duration;

use ai::agent::action_result::StopRecordingResult;
use futures::channel::oneshot;
use instant::Instant;
use thiserror::Error;
use warpui::{Entity, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;

/// Why a recording finalization ran. Distinct from the caller's claimed reason
/// (see [`FinalizationClaim::InProgress`]): the reason lives here so the
/// controller can carry the *actual* reason that drove finalization to
/// completion back to every waiter, including callers that only joined work
/// started by a different path.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
// Every variant is constructed only in non-wasm finalization paths.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) enum FinalizeReason {
    StoppedByAgent,
    RunEnded,
    LimitReached,
    FfmpegExited,
    RunCancelled,
    FinalizationDropped,
}

impl FinalizeReason {
    /// Stable, machine-readable key identifying why finalization ran, used by
    /// the `Recording.Stopped` telemetry event. Distinct from
    /// [`FinalizeReason::termination_reason`], which is human-readable prose.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn telemetry_key(self) -> &'static str {
        match self {
            FinalizeReason::StoppedByAgent => "agent_stopped",
            FinalizeReason::RunEnded => "run_ended",
            FinalizeReason::LimitReached => "limit_reached",
            FinalizeReason::FfmpegExited => "encoding_failed",
            FinalizeReason::RunCancelled => "run_cancelled",
            FinalizeReason::FinalizationDropped => "finalization_dropped",
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn termination_reason(
        self,
        completion_status: computer_use::RecordingCompletionStatus,
    ) -> String {
        match self {
            FinalizeReason::StoppedByAgent => match completion_status {
                computer_use::RecordingCompletionStatus::Completed => {
                    "Stopped by agent".to_string()
                }
                computer_use::RecordingCompletionStatus::StoppedEarly => {
                    "Recording stopped before the agent requested it".to_string()
                }
            },
            FinalizeReason::RunEnded => {
                "Finalized because the agent run ended without stopping the recording".to_string()
            }
            FinalizeReason::LimitReached => {
                "Stopped at the configured duration or size limit".to_string()
            }
            FinalizeReason::FfmpegExited => {
                "Capture process exited before the recording was stopped".to_string()
            }
            FinalizeReason::RunCancelled => {
                "Recording was interrupted when the conversation was cancelled".to_string()
            }
            FinalizeReason::FinalizationDropped => {
                "Recording finalization ended without producing a result".to_string()
            }
        }
    }
}

/// The finalized outcome of a recording, paired with the actual
/// [`FinalizeReason`] that drove finalization to completion. Callers that only
/// joined an in-progress finalization see the reason that started the work,
/// not the reason they claimed when joining.
pub(crate) type FinalizedRecording = (StopRecordingResult, FinalizeReason);

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
    /// How many times faster the finalized video should play back relative to
    /// real time, applied by the post-stop pipeline (Linux) or already baked
    /// in live during capture (macOS). A value <= 1.0 means real-time.
    pub(crate) playback_speed_multiplier: f32,
    /// The surface being recorded, used to resolve pointer-event coordinates
    /// into capture space for the post-stop burn-in.
    pub(crate) target: computer_use::Target,
    /// Recording-scoped pointer session shared with each `UseComputer` call's
    /// `PointerSink`, persisting the last resolved point and active button across
    /// calls so a drag split into separate `Down`/`Move`/`Up` calls records its
    /// release. Reset when a call fails or is cancelled.
    pub(crate) pointer_session: computer_use::PointerSession,
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

impl ActiveRecording {
    /// Commits any in-flight action group using the current elapsed time as its
    /// finish offset (clamped to the group's start). The in-flight call's
    /// pointer events live in that call's own buffer and are not reachable
    /// here, so the entry keeps the labels but no pointer geometry. No-op when
    /// no group is pending.
    fn commit_pending_group_now(&mut self) {
        if let Some(pending) = self.pending_group.take() {
            let finish_offset = self.started_at.elapsed().max(pending.start_offset);
            self.actions.push(computer_use::ActionLogEntry {
                offset: pending.start_offset,
                finish_offset,
                labels: pending.labels,
                pointer_events: Vec::new(),
            });
        }
    }
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
        waiters: Vec<oneshot::Sender<FinalizedRecording>>,
    },
    Finalized {
        id: String,
        conversation_id: AIConversationId,
        result: StopRecordingResult,
        /// The actual reason that drove finalization to completion, captured
        /// so a caller that only joined the in-progress work still learns why
        /// it ran (rather than the reason the caller itself claimed).
        reason: FinalizeReason,
    },
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) enum FinalizationClaim {
    Claimed {
        recording: Box<ActiveRecording>,
        result_receiver: oneshot::Receiver<FinalizedRecording>,
    },
    InProgress(oneshot::Receiver<FinalizedRecording>),
    Finished(FinalizedRecording),
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
        playback_speed_multiplier: f32,
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
                playback_speed_multiplier,
                target,
                pointer_session: computer_use::PointerSession::new(),
                actions: Vec::new(),
                summary,
                description,
                pending_group: None,
            }));
        }
    }

    /// Begins an in-flight `UseComputer` action group for the owning
    /// conversation, recording the group's start offset and labels. Returns the
    /// recording's capture start instant, its capture target, and a clone of the
    /// recording-scoped pointer session so the caller can share it with this
    /// call's `PointerSink` and a later split-call release can reuse the last
    /// resolved point. A pointer-only group is begun with empty labels;
    /// wait-only/no-op calls should not call this. The pending group is
    /// committed with its finish offset on success ([`commit_action_group`]) or
    /// discarded on failure ([`discard_action_group`]). Returns `None` (and
    /// begins nothing) if no recording is active for this conversation.
    ///
    /// [`commit_action_group`]: Self::commit_action_group
    /// [`discard_action_group`]: Self::discard_action_group
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn begin_action_group(
        &mut self,
        conversation_id: AIConversationId,
        labels: Vec<String>,
    ) -> Option<(Instant, computer_use::Target, computer_use::PointerSession)> {
        if let RecordingState::Active(recording) = &mut self.state
            && recording.conversation_id == conversation_id
        {
            // If a prior group was never committed or discarded, auto-commit it
            // with the current clock as its implicit finish offset. This can
            // happen when a `UseComputer` call completes and `begin_action_group`
            // is called for the next call before `commit_action_group` fires.
            recording.commit_pending_group_now();
            let start_offset = recording.started_at.elapsed();
            recording.pending_group = Some(PendingActionGroup {
                start_offset,
                labels,
            });
            return Some((
                recording.started_at,
                recording.target,
                recording.pointer_session.clone(),
            ));
        }
        None
    }

    /// Opens a recording action group for a shell `command` whose on-screen
    /// work should survive the smart cut (currently `playwright-cli` browser
    /// automation). Returns whether a group was opened, so the caller can settle
    /// it with [`commit_action_group_now`] or [`discard_action_group`] once the
    /// command resolves. Returns `false` for other commands or when no recording
    /// is active for this conversation.
    ///
    /// [`commit_action_group_now`]: Self::commit_action_group_now
    /// [`discard_action_group`]: Self::discard_action_group
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn maybe_begin_action_group(
        &mut self,
        conversation_id: AIConversationId,
        command: &str,
    ) -> bool {
        is_playwright_cli_command(command)
            && self
                .begin_action_group(conversation_id, Vec::new())
                .is_some()
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

    /// Commits the in-flight action group using the active recording's current
    /// elapsed time as the finish offset, for callers that cannot thread the
    /// capture start instant through to completion. No-op unless a recording is
    /// active for this conversation with a pending group.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn commit_action_group_now(&mut self, conversation_id: AIConversationId) {
        if let RecordingState::Active(recording) = &mut self.state
            && recording.conversation_id == conversation_id
        {
            recording.commit_pending_group_now();
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
            // Reset the pointer session so a later `UseComputer` call cannot
            // inherit an abandoned press from this failed/cancelled call.
            recording.pointer_session.clear();
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
            RecordingState::Active(mut recording)
                if matches(&recording.id, recording.conversation_id) =>
            {
                // A group can still be pending here (e.g. a long-running
                // `playwright-cli` command whose finish was never observed);
                // settle it so its window up to the stop point is kept rather
                // than dropped by the smart cut.
                recording.commit_pending_group_now();
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
                reason,
            } if matches(&id, conversation_id) => {
                let ready = (result.clone(), reason);
                self.state = RecordingState::Finalized {
                    id,
                    conversation_id,
                    result,
                    reason,
                };
                FinalizationClaim::Finished(ready)
            }
            state => {
                self.state = state;
                FinalizationClaim::NotFound
            }
        }
    }

    /// Completes an in-flight finalization with its resolved result and the
    /// *actual* reason that drove the work (not the reason any caller merely
    /// claimed when joining). Every waiter receives the same reason, and the
    /// reason is retained with the result until it is consumed, so telemetry
    /// downstream always reflects why finalization actually ran.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn complete_finalization(
        &mut self,
        recording_id: &str,
        result: StopRecordingResult,
        reason: FinalizeReason,
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
                    reason,
                };
                for waiter in waiters {
                    let _ = waiter.send((result.clone(), reason));
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

/// Whether a requested command invokes the `playwright-cli` binary, whose
/// on-screen browser automation should be kept in an active computer-use
/// recording rather than trimmed away with other shell work.
fn is_playwright_cli_command(command: &str) -> bool {
    command
        .split_whitespace()
        .find(|token| {
            let is_env_assignment = token
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && token.contains('=');
            !is_env_assignment
        })
        .is_some_and(|program| {
            Path::new(program)
                .file_name()
                .is_some_and(|name| name == "playwright-cli")
        })
}

impl Entity for RecordingController {
    type Event = ();
}

impl SingletonEntity for RecordingController {}

#[cfg(test)]
#[path = "recording_controller_tests.rs"]
mod tests;
