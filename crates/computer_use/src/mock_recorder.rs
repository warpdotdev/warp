use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

const MOCK_WIDTH: u32 = 1280;
const MOCK_HEIGHT: u32 = 720;
const MOCK_MP4_BYTES: &[u8] =
    b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom\x00\x00\x00\x08free\x00\x00\x00\x08mdat";

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, _config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        Ok(RecordingHandle::new_mock(
            MOCK_WIDTH,
            MOCK_HEIGHT,
            mock_recording_path(),
        ))
    }

    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        let width = handle.width();
        let height = handle.height();
        let duration = handle
            .mock_started_at
            .as_ref()
            .map(|started_at| started_at.elapsed())
            .unwrap_or_else(|| Duration::from_secs(1));
        let path = handle.mock_path.ok_or_else(|| RecordingError::Finalize {
            reason: "mock recorder received a non-mock recording handle".to_string(),
        })?;
        std::fs::write(&path, MOCK_MP4_BYTES).map_err(|error| RecordingError::Finalize {
            reason: format!("failed to write mock recording: {error}"),
        })?;
        let size_bytes =
            u64::try_from(MOCK_MP4_BYTES.len()).expect("mock recording byte count fits in u64");
        Ok(RecordingOutput {
            path,
            duration,
            width,
            height,
            size_bytes,
            completion_status: RecordingCompletionStatus::Completed,
        })
    }
}

fn mock_recording_path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "warp-mock-recording-{}-{nanos}.mp4",
        std::process::id()
    ))
}
