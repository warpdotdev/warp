//! Screen recordings of a computer-use session, published when they stop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use computer_use::{
    ActionLogEntry, PointerEvent, PointerSession, PointerSink, Recorder, RecordingCompletionStatus,
    RecordingHandle, RecordingOutput, Target,
};
use instant::Instant;
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

/// Why a recording stopped.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum StopReason {
    /// The model called `stop_recording`.
    Requested,
    /// The MCP session ended while recording.
    SessionEnded,
}

pub(crate) struct ActiveRecording {
    id: String,
    handle: RecordingHandle,
    started_at: Instant,
    frame_rate: u32,
    /// Shared with every action batch's pointer sink, so a drag split across tool calls is
    /// annotated as one gesture.
    pointer_session: PointerSession,
    /// The input batches performed while recording, used to trim idle time from the video and
    /// annotate it.
    actions: Vec<ActionLogEntry>,
    title: Option<String>,
    description: Option<String>,
}

/// An input batch in progress while recording.
pub(crate) struct ActionGroup {
    start: Duration,
    labels: Vec<String>,
    pointer_events: Arc<Mutex<Vec<PointerEvent>>>,
}

impl ActiveRecording {
    pub fn new(
        id: String,
        handle: RecordingHandle,
        frame_rate: u32,
        title: Option<String>,
        description: Option<String>,
    ) -> Self {
        Self {
            id,
            handle,
            started_at: Instant::now(),
            frame_rate,
            pointer_session: PointerSession::new(),
            actions: Vec::new(),
            title,
            description,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Starts tracking an input batch, returning the sink the actor reports pointer events to.
    pub fn begin_group(&self, labels: Vec<String>) -> (ActionGroup, PointerSink) {
        let pointer_events = Arc::new(Mutex::new(Vec::new()));
        let sink = PointerSink {
            started_at: self.started_at,
            recording_target: Target::Screen,
            recording_geometry: self.handle.geometry(),
            events: pointer_events.clone(),
            session: self.pointer_session.clone(),
        };
        let group = ActionGroup {
            start: self.started_at.elapsed(),
            labels,
            pointer_events,
        };
        (group, sink)
    }

    pub fn commit_group(&mut self, group: ActionGroup) {
        let pointer_events = std::mem::take(
            &mut *group
                .pointer_events
                .lock()
                .expect("pointer event buffer poisoned"),
        );
        self.actions.push(ActionLogEntry {
            offset: group.start,
            finish_offset: self.started_at.elapsed().max(group.start),
            labels: group.labels,
            pointer_events,
        });
    }

    /// Forgets a failed batch's pointer state, so a later release can't complete its abandoned
    /// press.
    pub fn discard_group(&self) {
        self.pointer_session.clear();
    }

    #[cfg(test)]
    pub fn actions(&self) -> &[ActionLogEntry] {
        &self.actions
    }
}

/// Stops `recording` and publishes its video to `destination`, returning a summary for the model.
pub(crate) async fn finish(
    recorder: &dyn Recorder,
    recording: ActiveRecording,
    destination: &RecordingDestination,
    reason: StopReason,
) -> Result<Value, ToolError> {
    let ActiveRecording {
        handle,
        frame_rate,
        actions,
        title,
        description,
        ..
    } = recording;
    let output = recorder
        .stop(handle)
        .await
        .map_err(|error| format!("Failed to stop the recording: {error}"))?;
    let video = prepare_video(&output, &actions, frame_rate).await?;
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
    summary["termination_reason"] = json!(termination_reason(reason, output.completion_status));
    Ok(summary)
}

/// A finished video, ready to publish.
struct PreparedVideo {
    path: PathBuf,
    duration: Duration,
    /// Files left over from producing the video, to delete once it's published.
    intermediates: Vec<PathBuf>,
}

/// Trims idle time from the raw capture and annotates its input, falling back to the raw capture
/// when that fails.
async fn prepare_video(
    output: &RecordingOutput,
    actions: &[ActionLogEntry],
    frame_rate: u32,
) -> Result<PreparedVideo, ToolError> {
    let raw = output.path.clone();
    let capture_log = raw.with_extension("log");
    // The harness may also drive the screen through other tools, so a recording without logged
    // actions is still published, just untrimmed.
    if !actions.is_empty() {
        match computer_use::post_process_recording(
            &raw,
            actions,
            (output.width, output.height),
            output.duration,
            frame_rate,
        )
        .await
        {
            Ok(processed) if processed != raw => {
                match computer_use::finalized_video_duration(&processed).await {
                    Ok(duration) => {
                        return Ok(PreparedVideo {
                            path: processed,
                            duration,
                            intermediates: vec![raw, capture_log],
                        });
                    }
                    Err(error) => {
                        log::warn!(
                            "The trimmed recording is invalid; publishing the original: {error}"
                        );
                        let _ = std::fs::remove_file(&processed);
                    }
                }
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "Failed to trim and annotate the recording; publishing the original: {error}"
                );
            }
        }
    }
    match computer_use::finalized_video_duration(&raw).await {
        Ok(duration) => Ok(PreparedVideo {
            path: raw,
            duration,
            intermediates: vec![capture_log],
        }),
        Err(error) => {
            let _ = std::fs::remove_file(&raw);
            let _ = std::fs::remove_file(&capture_log);
            Err(format!("The recording could not be finalized: {error}").into())
        }
    }
}

fn termination_reason(
    reason: StopReason,
    completion_status: RecordingCompletionStatus,
) -> &'static str {
    match (reason, completion_status) {
        (StopReason::Requested, RecordingCompletionStatus::Completed) => "Stopped by agent",
        (StopReason::Requested, RecordingCompletionStatus::StoppedEarly) => {
            "Recording stopped before the agent requested it"
        }
        (StopReason::SessionEnded, RecordingCompletionStatus::Completed)
        | (StopReason::SessionEnded, RecordingCompletionStatus::StoppedEarly) => {
            "Finalized because the agent run ended without stopping the recording"
        }
    }
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
