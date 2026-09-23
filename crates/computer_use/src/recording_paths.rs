use std::fs::File;
use std::path::PathBuf;

use crate::RecordingError;

/// Filename prefix for every recording's temp-directory output file, shared by the macOS,
/// Linux, and Windows recorders so they can't drift on the naming convention.
const RECORDING_FILE_PREFIX: &str = "warp-recording";

/// Allocates a fresh recording output path (`<prefix>-<uuid>.mp4` in the system temp
/// directory) and creates its sibling `.log` file already open for writing.
///
/// ffmpeg's progress log goes to a file, not a pipe, so its stderr can never fill and stall
/// capture over a long recording. This is the cross-platform entry point behind each platform
/// recorder's `start`, which is the only place any of the three name a recording's output file.
pub(super) fn new_recording_path() -> Result<(PathBuf, PathBuf, File), RecordingError> {
    let path = std::env::temp_dir().join(format!(
        "{RECORDING_FILE_PREFIX}-{}.mp4",
        uuid::Uuid::new_v4()
    ));
    let log_path = path.with_extension("log");
    let log_file = File::create(&log_path).map_err(|error| RecordingError::Start {
        reason: format!("failed to create the recording log file: {error}"),
    })?;
    Ok((path, log_path, log_file))
}

#[cfg(test)]
#[path = "recording_paths_tests.rs"]
mod tests;
