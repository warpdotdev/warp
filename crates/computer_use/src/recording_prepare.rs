//! Turning a stopped recording's capture into the video to publish.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{
    ActionLogEntry, RecordingError, RecordingOutput, finalized_video_duration,
    post_process_recording,
};

/// A stopped recording's video, ready to publish.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PreparedRecording {
    /// The video to publish.
    pub path: PathBuf,
    /// The length of the video at `path`.
    pub duration: Duration,
    /// Files left over from producing the video, to delete once it's published.
    pub intermediates: Vec<PathBuf>,
}

/// Trims idle time from `output`'s capture and annotates it with `entries`, falling back to the
/// untouched capture when that fails or `entries` is empty. When no valid video results, the
/// capture's files are deleted.
pub async fn prepare_recording(
    output: &RecordingOutput,
    entries: &[ActionLogEntry],
    frame_rate: u32,
) -> Result<PreparedRecording, RecordingError> {
    let raw = &output.path;
    let capture_log = raw.with_extension("log");
    let processed = if entries.is_empty() {
        None
    } else {
        match post_process_recording(
            raw,
            entries,
            (output.width, output.height),
            output.duration,
            frame_rate,
        )
        .await
        {
            Ok(path) if &path != raw => Some(path),
            Ok(_) => None,
            Err(error) => {
                log::warn!(
                    "Failed to trim and annotate the recording; using the original capture: {error}"
                );
                None
            }
        }
    };
    match validated_recording(raw, processed).await {
        Ok((path, duration)) => {
            let intermediates = if &path == raw {
                vec![capture_log]
            } else {
                vec![raw.clone(), capture_log]
            };
            Ok(PreparedRecording {
                path,
                duration,
                intermediates,
            })
        }
        Err(error) => {
            let _ = std::fs::remove_file(raw);
            let _ = std::fs::remove_file(&capture_log);
            Err(error)
        }
    }
}

/// Returns `processed` and its duration when it's a valid video, and otherwise `raw`, deleting an
/// invalid `processed`.
async fn validated_recording(
    raw: &Path,
    processed: Option<PathBuf>,
) -> Result<(PathBuf, Duration), RecordingError> {
    if let Some(processed) = processed {
        match finalized_video_duration(&processed).await {
            Ok(duration) => return Ok((processed, duration)),
            Err(error) => {
                log::warn!("The trimmed recording is invalid; using the original capture: {error}");
                let _ = std::fs::remove_file(processed);
            }
        }
    }
    let duration = finalized_video_duration(raw).await?;
    Ok((raw.to_path_buf(), duration))
}

#[cfg(test)]
#[path = "recording_prepare_tests.rs"]
mod tests;
