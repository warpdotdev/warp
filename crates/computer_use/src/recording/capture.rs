//! Supervised ffmpeg capture lifecycle: ephemeral files, the encode and output
//! settings every capture shares, launching until capture is confirmed live, and
//! SIGINT finalization of the container.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;
use tokio::process::{Child, Command};

use crate::{RecordingCompletionStatus, RecordingError, RecordingHandle, RecordingOutput};

/// How long to wait for ffmpeg to open the capture source and produce first output.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for ffmpeg to finalize the container after stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for capture to begin.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The ephemeral files backing one capture: the MP4 the muxer writes and the log
/// ffmpeg's stderr is redirected to.
pub(crate) struct CaptureFiles {
    path: PathBuf,
    log_path: PathBuf,
    log_file: File,
}

/// Explains, from the tail of ffmpeg's log, why capture never went live. The
/// substrates fail for different reasons — a denied Screen Recording grant on
/// macOS, an unreachable `$DISPLAY` on Linux — so the platform adapter owns the
/// classification. It returns `None` for a failure it does not recognize, and a
/// platform with nothing useful to add supplies no hook at all.
pub(crate) type StartDiagnosis = fn(&str) -> Option<String>;

/// Allocates the capture output path and its sibling log.
///
/// ffmpeg's progress log goes to a file so its stderr pipe can never fill and
/// stall capture over a long recording.
pub(crate) fn new_capture_files() -> Result<CaptureFiles, RecordingError> {
    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    let log_path = path.with_extension("log");
    let log_file = File::create(&log_path).map_err(|e| RecordingError::Start {
        reason: format!("failed to create the recording log file: {e}"),
    })?;
    Ok(CaptureFiles {
        path,
        log_path,
        log_file,
    })
}

/// Bounds capture wall-clock time.
///
/// This must be pushed before the input so ffmpeg treats it as an input option:
/// the bound then measures real capture time rather than output timeline time.
pub(crate) fn push_duration_limit(command: &mut Command, max_duration: Duration) {
    command
        .arg("-t")
        .arg(format!("{:.3}", max_duration.as_secs_f64()));
}

/// Pushes the encode settings and output-side bounds shared by every capture.
///
/// `video_filter` is the substrate's output filtergraph, if it needs one. No
/// speed filter belongs here: both masters are captured at 1x so the post-stop
/// cut can keep real action windows at full speed and remove only the gaps
/// between them.
pub(crate) fn push_encode_args(
    command: &mut Command,
    max_size_bytes: u64,
    video_filter: Option<&str>,
) {
    command
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"]);
    if let Some(filter) = video_filter {
        command.args(["-vf", filter]);
    }
    command
        .args(["-movflags", "+faststart"])
        .arg("-fs")
        .arg(max_size_bytes.to_string());
}

/// Spawns `command`, waits until capture is confirmed live, and returns the
/// handle that owns the running process.
///
/// `width`/`height` are the encoded frame dimensions the handle reports, which
/// is the cropped size when the substrate crops its input.
pub(crate) async fn launch_capture(
    mut command: Command,
    files: CaptureFiles,
    width: u32,
    height: u32,
    diagnose: Option<StartDiagnosis>,
) -> Result<RecordingHandle, RecordingError> {
    let CaptureFiles {
        path,
        log_path,
        log_file,
    } = files;
    command
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .kill_on_drop(true);

    let mut process = command.spawn().map_err(|e| RecordingError::Environment {
        reason: format!("failed to spawn ffmpeg: {e}"),
    })?;

    // Resolve once capture is confirmed live (the output file has grown, meaning
    // ffmpeg opened the capture source and the muxer is writing).
    if let Err(e) = wait_for_first_output(&path, &mut process).await {
        let _ = process.start_kill();
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let detail = ffmpeg_error_tail(&log);
        let hint = diagnose
            .and_then(|diagnose| diagnose(&log))
            .map_or_else(String::new, |hint| format!(" {hint}"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&log_path);
        return Err(RecordingError::Start {
            reason: format!("{e}{detail}{hint}"),
        });
    }
    let _ = std::fs::remove_file(&log_path);

    Ok(RecordingHandle {
        width,
        height,
        exit_state: Arc::new(Mutex::new(None)),
        path,
        started_at: Instant::now(),
        process: Some(process),
        cleanup_on_drop: true,
    })
}

/// Stops capture, validates the container, and hands the file to the caller.
pub(crate) async fn finalize_capture(
    mut handle: RecordingHandle,
) -> Result<RecordingOutput, RecordingError> {
    let width = handle.width;
    let height = handle.height;
    let path = handle.path.clone();
    let duration = handle.started_at.elapsed();

    let mut process = handle
        .process
        .take()
        .ok_or_else(|| RecordingError::Finalize {
            reason: "recording process is unavailable".to_string(),
        })?;

    let completion_status = stop_capture_process(&mut process, &path).await?;

    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size_bytes == 0 {
        let _ = std::fs::remove_file(&path);
        return Err(RecordingError::Finalize {
            reason: "recording produced an empty file".to_string(),
        });
    }
    // The caller now owns the validated file through `RecordingOutput`.
    handle.cleanup_on_drop = false;

    Ok(RecordingOutput {
        path,
        duration,
        width,
        height,
        size_bytes,
        completion_status,
    })
}

/// SIGINT makes ffmpeg flush and write the moov atom instead of leaving a
/// truncated container.
async fn stop_capture_process(
    process: &mut Child,
    path: &Path,
) -> Result<RecordingCompletionStatus, RecordingError> {
    match process.try_wait().map_err(|e| RecordingError::Finalize {
        reason: format!("failed to poll ffmpeg: {e}"),
    })? {
        Some(_) => Ok(RecordingCompletionStatus::StoppedEarly),
        None => {
            let mut completion_status = RecordingCompletionStatus::Completed;
            if let Some(pid) = process.id() {
                let pid = nix::unistd::Pid::from_raw(pid as i32);
                if nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT).is_err() {
                    completion_status = RecordingCompletionStatus::StoppedEarly;
                }
            } else {
                completion_status = RecordingCompletionStatus::StoppedEarly;
            }
            wait_for_finalization(process, path, completion_status).await
        }
    }
}

/// Waits up to [`STOP_TIMEOUT`] for ffmpeg to exit after being asked to finalize;
/// on timeout the container is likely missing its moov atom, so the file is
/// discarded.
async fn wait_for_finalization(
    process: &mut Child,
    path: &Path,
    completion_status: RecordingCompletionStatus,
) -> Result<RecordingCompletionStatus, RecordingError> {
    match tokio::time::timeout(STOP_TIMEOUT, process.wait()).await {
        Ok(Ok(_)) => Ok(completion_status),
        Ok(Err(_)) => Ok(RecordingCompletionStatus::StoppedEarly),
        Err(_) => {
            // ffmpeg missed the finalization deadline, so the container is likely missing its
            // moov atom and unplayable. Force-kill and discard the file rather than returning a
            // corrupt recording.
            let _ = process.start_kill();
            let _ = process.wait().await;
            let _ = std::fs::remove_file(path);
            Err(RecordingError::Finalize {
                reason: "ffmpeg did not finalize the recording in time".to_string(),
            })
        }
    }
}

/// Waits until the recording file has grown (capture is live) or ffmpeg exits.
async fn wait_for_first_output(path: &Path, process: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(status) = process
            .try_wait()
            .map_err(|e| format!("failed to poll ffmpeg: {e}"))?
        {
            return Err(format!("ffmpeg exited early with status {status}"));
        }
        if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for capture to begin".to_string());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Returns a short, parenthesized tail of ffmpeg's stderr log for diagnostics.
fn ffmpeg_error_tail(log: &str) -> String {
    let lines: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(3);
    let tail = lines[start..].join(" ");
    if tail.is_empty() {
        String::new()
    } else {
        format!(" ({tail})")
    }
}
