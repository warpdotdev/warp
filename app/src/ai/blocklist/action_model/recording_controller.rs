//! Runtime-global registry of in-progress video recordings.
//!
//! A recording's capture process must outlive the `StartRecording` tool call
//! that launches it and survive until a later `StopRecording` call (possibly
//! from a later resumed turn), so the live handle lives here rather than in a
//! per-call executor.

use thiserror::Error;
use warpui::{Entity, SingletonEntity};

#[derive(Debug, Error)]
pub enum StartRecordingControllerError {
    #[error("A recording is already in progress in this runtime.")]
    AlreadyInProgress,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Error)]
pub enum StopRecordingControllerError {
    #[error("No active recording with id '{recording_id}'.")]
    NoActiveRecording { recording_id: String },
    #[error("Current conversation has not been synced to the server yet.")]
    ConversationNotSynced,
}

/// The single in-progress recording: its controller id and live capture handle.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct ActiveRecording {
    id: String,
    handle: computer_use::RecordingHandle,
}

/// Enforces a single active recording per client runtime.
pub struct RecordingController {
    active: Option<ActiveRecording>,
    /// Set while a start is in flight (after reservation, before the recording is
    /// registered) so a concurrent start cannot race past the single-slot guard.
    starting: bool,
}

impl RecordingController {
    pub fn new() -> Self {
        Self {
            active: None,
            starting: false,
        }
    }

    /// Reserves the single recording slot, failing if one is already active or
    /// starting.
    pub fn try_begin_start(&mut self) -> Result<(), StartRecordingControllerError> {
        if self.starting || self.active.is_some() {
            return Err(StartRecordingControllerError::AlreadyInProgress);
        }
        self.starting = true;
        Ok(())
    }

    /// Registers a successfully started recording, releasing the start reservation.
    pub fn finish_start(&mut self, recording_id: String, handle: computer_use::RecordingHandle) {
        self.starting = false;
        self.active = Some(ActiveRecording {
            id: recording_id,
            handle,
        });
    }

    /// Releases the start reservation after a failed start.
    pub fn abort_start(&mut self) {
        self.starting = false;
    }

    /// Removes and returns the live handle for `recording_id`, leaving any
    /// non-matching active recording in place.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn take_handle_or_err(
        &mut self,
        recording_id: &str,
    ) -> Result<computer_use::RecordingHandle, StopRecordingControllerError> {
        match self.active.take() {
            Some(active) if active.id == recording_id => Ok(active.handle),
            other => {
                self.active = other;
                Err(StopRecordingControllerError::NoActiveRecording {
                    recording_id: recording_id.to_string(),
                })
            }
        }
    }
}

impl Entity for RecordingController {
    type Event = ();
}

impl SingletonEntity for RecordingController {}
