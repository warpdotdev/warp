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
        Err(RecordingError::UnsupportedPlatform)
    }

    async fn stop(&self, _handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        Err(RecordingError::UnsupportedPlatform)
    }
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
        _actions: &[super::Action],
        _options: super::Options,
    ) -> Result<ActionResult, String> {
        Ok(ActionResult {
            screenshot: None,
            cursor_position: None,
        })
    }
}
