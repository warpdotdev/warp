//! Runtime-global registry of in-progress video recordings.
//!
//! A recording's capture process must outlive the `StartRecording` tool call
//! that launches it and survive until a later `StopRecording` call (possibly
//! from a later resumed turn), so the live handle lives here rather than in a
//! per-call executor.

use std::collections::HashMap;

use thiserror::Error;
use warpui::{Entity, SingletonEntity};

#[derive(Debug, Error)]
pub enum StartRecordingControllerError {
    #[error("A recording is already in progress in this runtime.")]
    AlreadyInProgress,
}

#[derive(Debug, Error)]
pub enum StopRecordingControllerError {
    #[error("No active recording with id '{recording_id}'.")]
    NoActiveRecording { recording_id: String },
    #[error("Current conversation has not been synced to the server yet.")]
    ConversationNotSynced,
}

/// Holds the live capture handle for a single in-progress recording.
pub struct RecordingSession {
    handle: computer_use::RecordingHandle,
}

impl RecordingSession {
    pub fn new(handle: computer_use::RecordingHandle) -> Self {
        Self { handle }
    }

    pub fn into_handle(self) -> computer_use::RecordingHandle {
        self.handle
    }
}

/// Tracks recordings keyed by id and enforces one active recording per client runtime.
///
/// NOTE: Ambient agent environment setup currently provides one Xvfb instance
/// per runtime, which exposes a single display to the recorder. If that changes,
/// this controller should key active recordings by display.
pub struct RecordingController {
    sessions: HashMap<String, RecordingSession>,
    /// Set while a start is in flight (after reservation, before the session is
    /// registered) so a concurrent start cannot race past the single-slot guard.
    starting: bool,
}

impl RecordingController {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            starting: false,
        }
    }

    /// Reserves the single recording slot, failing if one is already active or
    /// starting.
    pub fn try_begin_start(&mut self) -> Result<(), StartRecordingControllerError> {
        if self.starting || !self.sessions.is_empty() {
            return Err(StartRecordingControllerError::AlreadyInProgress);
        }
        self.starting = true;
        Ok(())
    }

    /// Registers a successfully started recording, releasing the start reservation.
    pub fn finish_start(&mut self, recording_id: String, session: RecordingSession) {
        self.starting = false;
        self.sessions.insert(recording_id, session);
    }

    /// Releases the start reservation after a failed start.
    pub fn abort_start(&mut self) {
        self.starting = false;
    }

    /// Removes and returns the session for `recording_id`.
    pub fn take_session_or_err(
        &mut self,
        recording_id: &str,
    ) -> Result<RecordingSession, StopRecordingControllerError> {
        self.sessions.remove(recording_id).ok_or_else(|| {
            StopRecordingControllerError::NoActiveRecording {
                recording_id: recording_id.to_string(),
            }
        })
    }
}

impl Entity for RecordingController {
    type Event = ();
}

impl SingletonEntity for RecordingController {}
