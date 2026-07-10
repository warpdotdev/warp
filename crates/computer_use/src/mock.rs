//! Mock recorder for exercising the recording UI on macOS, where real capture
//! is unsupported.
//!
//! Enabled by setting `WARP_MOCK_RECORDER`; `stop` publishes a copy of the MP4
//! at `WARP_MOCK_RECORDING_FIXTURE`, and `WARP_MOCK_RECORDING_STOPPED_EARLY`
//! exercises the partial-recording copy.

use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use instant::Instant;

use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

// The executors build a fresh recorder per tool call, so the start time lives
// in a module-level static rather than on the recorder instance.
static STARTED_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn started_at() -> &'static Mutex<Option<Instant>> {
    STARTED_AT.get_or_init(|| Mutex::new(None))
}

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, _config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        *started_at().lock().expect("mock recorder mutex poisoned") = Some(Instant::now());
        Ok(RecordingHandle {
            width: 1280,
            height: 720,
        })
    }

    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        let duration = started_at()
            .lock()
            .expect("mock recorder mutex poisoned")
            .take()
            .map(|started| started.elapsed())
            .unwrap_or_default();

        let fixture = std::env::var("WARP_MOCK_RECORDING_FIXTURE").map_err(|_| {
            RecordingError::Environment {
                reason: "WARP_MOCK_RECORDING_FIXTURE is not set".to_string(),
            }
        })?;
        let path =
            std::env::temp_dir().join(format!("warp-mock-recording-{}.mp4", std::process::id()));
        std::fs::copy(&fixture, &path).map_err(|e| RecordingError::Finalize {
            reason: format!("failed to copy mock fixture '{fixture}': {e}"),
        })?;
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        let completion_status = if std::env::var_os("WARP_MOCK_RECORDING_STOPPED_EARLY").is_some() {
            RecordingCompletionStatus::StoppedEarly
        } else {
            RecordingCompletionStatus::Completed
        };

        Ok(RecordingOutput {
            path,
            duration,
            width: handle.width(),
            height: handle.height(),
            size_bytes,
            completion_status,
        })
    }
}
