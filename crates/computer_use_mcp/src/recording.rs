//! Screen recordings of a computer-use session, published when they stop.

use std::path::PathBuf;

use computer_use::{
    Recorder, RecordingActionLog, RecordingFinalizeReason, RecordingHandle, Target,
    prepare_recording,
};
use serde_json::{Value, json};

use crate::{ToolError, publish};

/// Where finished recordings are published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingDestination {
    /// Uploaded as artifacts of an Oz run by the Warp CLI at `cli`.
    OzRun { cli: PathBuf, run_id: String },
    /// Moved into a local directory.
    Directory(PathBuf),
}

pub(crate) struct ActiveRecording {
    id: String,
    handle: RecordingHandle,
    frame_rate: u32,
    /// The input batches performed while recording, used to trim idle time from the video and
    /// annotate it.
    pub action_log: RecordingActionLog,
    title: Option<String>,
    description: Option<String>,
}

impl ActiveRecording {
    pub fn new(
        id: String,
        handle: RecordingHandle,
        frame_rate: u32,
        title: Option<String>,
        description: Option<String>,
    ) -> Self {
        let action_log = RecordingActionLog::new(Target::Screen, handle.geometry());
        Self {
            id,
            handle,
            frame_rate,
            action_log,
            title,
            description,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Stops `recording` and publishes its video to `destination`, returning a summary for the model.
pub(crate) async fn finish(
    recorder: &dyn Recorder,
    recording: ActiveRecording,
    destination: &RecordingDestination,
    reason: RecordingFinalizeReason,
) -> Result<Value, ToolError> {
    let ActiveRecording {
        handle,
        frame_rate,
        action_log,
        title,
        description,
        ..
    } = recording;
    let output = recorder
        .stop(handle)
        .await
        .map_err(|error| format!("Failed to stop the recording: {error}"))?;
    // The harness may also drive the screen through other tools, so a recording without logged
    // actions is still published, just untrimmed.
    let video = prepare_recording(&output, action_log.entries(), frame_rate)
        .await
        .map_err(|error| format!("The recording could not be finalized: {error}"))?;
    let published = publish::publish(
        &video.path,
        destination,
        title.as_deref(),
        description.as_deref(),
    )
    .await;
    for path in &video.intermediates {
        let _ = std::fs::remove_file(path);
    }
    let mut summary = published?;
    summary["duration_secs"] = json!(video.duration.as_secs_f64());
    summary["width_px"] = json!(output.width);
    summary["height_px"] = json!(output.height);
    summary["termination_reason"] = json!(reason.termination_reason(output.completion_status));
    Ok(summary)
}
