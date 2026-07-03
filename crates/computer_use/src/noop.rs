use async_trait::async_trait;

use crate::{ActionResult, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput};

pub fn is_supported_on_current_platform() -> bool {
    false
}

/// A recorder that reports recording as unsupported on the current platform.
pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl super::Recorder for Recorder {
    async fn start(&self, _config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        Err(RecordingError::Environment {
            reason: "video recording is not supported on this platform".to_string(),
        })
    }

    async fn stop(&self, _handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        Err(RecordingError::Environment {
            reason: "video recording is not supported on this platform".to_string(),
        })
    }
}

/// Reports whether background, per-window control is available. The noop backend performs no
/// real actions, so per-window background control is unsupported.
pub fn background_supported() -> bool {
    false
}

pub struct Actor;

impl Actor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl super::Actor for Actor {
    fn platform(&self) -> Option<super::Platform> {
        None
    }

    async fn perform_actions(
        &mut self,
        _actions: &[super::TargetedAction],
        _options: super::Options,
    ) -> Result<ActionResult, String> {
        Ok(ActionResult::legacy(None, None))
    }
}
