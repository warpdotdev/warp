//! Windows screen recording via a supervised ffmpeg `gdigrab` process.
//!
//! `start` spawns ffmpeg capturing the full virtual desktop and waits for the output file to
//! begin growing before returning a live [`RecordingHandle`]. `stop` finalizes the capture and
//! validates the resulting file before handing it back to the caller.
//!
//! Finalization here differs from the macOS/Linux recorders (which send `SIGINT`): Windows
//! processes have no POSIX signals, and there is no reliable way to deliver a console control
//! event to only this child, so `stop` instead asks ffmpeg to quit gracefully over its own stdin
//! (`q\n`), which ffmpeg treats the same as an interactive quit and flushes the container
//! accordingly. See [`finalize_capture`] for the full sequence.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::dpi::DpiAwarenessGuard;
use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

/// How long to wait for ffmpeg to open the desktop capture device and produce first output.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for ffmpeg to finalize the container after requesting a graceful quit.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for capture to begin.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long to retry deleting an abandoned recording's files after its process is reaped.
const ABANDONED_RECORDING_CLEANUP_TIMEOUT: Duration = Duration::from_secs(15);

/// The virtual desktop's bounding box, in physical pixels, spanning all monitors.
///
/// `origin_x`/`origin_y` can be negative when a monitor is positioned left of or above the
/// primary monitor; `width`/`height` are normalized to even values (see
/// [`normalize_virtual_screen_geometry`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct VirtualScreenGeometry {
    pub(super) origin_x: i32,
    pub(super) origin_y: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub struct Recorder {
    ffmpeg: PathBuf,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }

    #[cfg(test)]
    fn with_ffmpeg(ffmpeg: PathBuf) -> Self {
        Self { ffmpeg }
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        let geometry = query_virtual_screen_geometry()?;
        let (path, log_path, log_file) = crate::recording_paths::new_recording_path()?;
        let command = new_ffmpeg_capture_command(&self.ffmpeg, &config, geometry);
        launch_recording(command, path, log_path, log_file, geometry, START_TIMEOUT).await
    }

    async fn stop(&self, mut handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
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

        // Ask ffmpeg to quit gracefully so the moov atom is written; on failure
        // `finalize_capture` has already killed the process and deleted the output.
        let completion_status = match finalize_capture(&mut process, &path, STOP_TIMEOUT).await {
            Ok(status) => status,
            Err(error) => {
                remove_recording_files(&path);
                return Err(error);
            }
        };
        // The progress log is only useful for diagnosing a failed start/finalize; drop it now
        // that the recording finished.
        let _ = std::fs::remove_file(path.with_extension("log"));

        let size_bytes = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size_bytes == 0 {
            remove_recording_files(&path);
            return Err(RecordingError::Finalize {
                reason: "recording produced an empty file".to_string(),
            });
        }
        // A nonempty file can still be an unplayable, truncated container (e.g. a missing moov
        // atom) if ffmpeg exited uncleanly; probe it before handing it back to the caller.
        if let Err(error) =
            crate::recording_metadata::video_duration_with_ffmpeg(&self.ffmpeg, &path).await
        {
            remove_recording_files(&path);
            return Err(error);
        }

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
}

/// Queries the virtual desktop's bounding box across all monitors, in physical pixels.
///
/// `GetSystemMetrics(SM_*VIRTUALSCREEN)` returns DPI-scaled logical coordinates unless the
/// calling thread is per-monitor DPI aware, which would misalign the capture region on HiDPI
/// setups; `DpiAwarenessGuard` opts in for the duration of this call.
pub(super) fn query_virtual_screen_geometry() -> Result<VirtualScreenGeometry, RecordingError> {
    let _dpi_guard = DpiAwarenessGuard::enter_per_monitor_v2();
    // SAFETY: `GetSystemMetrics` has no preconditions.
    let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    normalize_virtual_screen_geometry(origin_x, origin_y, width, height)
}

fn normalize_virtual_screen_geometry(
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
) -> Result<VirtualScreenGeometry, RecordingError> {
    // Reject non-positive dimensions before the `as u32` casts below: a negative width/height
    // would otherwise wrap around to a huge positive value instead of failing here.
    if width <= 0 || height <= 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid virtual screen dimensions {width}x{height}"),
        });
    }
    // libx264 with yuv420p requires even dimensions.
    let width = (width as u32) & !1;
    let height = (height as u32) & !1;
    // A width/height of exactly 1 is positive but odd, so it passes the check above and only
    // becomes 0 once its low bit is cleared; catch that case separately since it can't be
    // rejected before rounding.
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid even virtual screen dimensions {width}x{height}"),
        });
    }
    Ok(VirtualScreenGeometry {
        origin_x,
        origin_y,
        width,
        height,
    })
}

fn new_ffmpeg_capture_command(
    ffmpeg: &Path,
    config: &RecordingConfig,
    geometry: VirtualScreenGeometry,
) -> Command {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .args(["-f", "gdigrab"])
        .args(["-framerate", &config.frame_rate.to_string()])
        .args(["-offset_x", &geometry.origin_x.to_string()])
        .args(["-offset_y", &geometry.origin_y.to_string()])
        .args([
            "-video_size",
            &format!("{}x{}", geometry.width, geometry.height),
        ])
        .args(["-draw_mouse", "0"])
        .arg("-t")
        .arg(format!("{:.3}", config.max_duration.as_secs_f64()))
        .args(["-i", "desktop"])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg("-fs")
        .arg(config.max_size_bytes.to_string());
    command
}

async fn launch_recording(
    mut command: Command,
    path: PathBuf,
    log_path: PathBuf,
    log_file: File,
    geometry: VirtualScreenGeometry,
    timeout: Duration,
) -> Result<RecordingHandle, RecordingError> {
    command
        .arg(&path)
        // Piped so `finalize_capture` can later send ffmpeg its graceful quit command.
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .kill_on_drop(true);
    let mut process = match command.spawn() {
        Ok(process) => process,
        Err(error) => {
            remove_recording_files(&path);
            return Err(RecordingError::Start {
                reason: format!("failed to start ffmpeg gdigrab capture: {error}"),
            });
        }
    };

    // Resolves once capture is confirmed live (the output file has grown, meaning ffmpeg opened
    // the desktop capture device and the muxer is writing).
    if let Err(error) = wait_for_first_output(&path, &mut process, timeout).await {
        kill_and_reap(&mut process).await;
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let reason = capture_start_failure_reason(&error, &log);
        remove_recording_files(&path);
        return Err(RecordingError::Start { reason });
    }

    Ok(RecordingHandle {
        width: geometry.width,
        height: geometry.height,
        capture_origin: crate::Vector2I::new(geometry.origin_x, geometry.origin_y),
        exit_state: Arc::new(Mutex::new(None)),
        path,
        started_at: Instant::now(),
        process: Some(process),
        cleanup_on_drop: true,
    })
}

/// Polls `path`'s size until it grows above zero (capture is live), the process exits early, or
/// `timeout` elapses.
async fn wait_for_first_output(
    path: &Path,
    process: &mut Child,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = process
            .try_wait()
            .map_err(|error| format!("failed to poll ffmpeg: {error}"))?
        {
            return Err(format!("ffmpeg exited early with status {status}"));
        }
        if std::fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            > 0
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for ffmpeg capture output".to_string());
        }
        tokio::time::sleep(POLL_INTERVAL.min(timeout)).await;
    }
}

/// Finalizes a gdigrab recording by asking ffmpeg to quit gracefully so it writes the moov atom,
/// instead of leaving a truncated, unplayable container.
///
/// Windows processes have no POSIX signals, and there is no reliable way to deliver a console
/// control event to only this child process, so this sends ffmpeg's own interactive quit command
/// (`q\n`) over stdin rather than signaling it, unlike the macOS/Linux recorders. Any failure
/// along the way kills and reaps the process (see [`kill_and_reap`] for why that must happen
/// before the file is removed) and discards the recording rather than risking a corrupt file.
async fn finalize_capture(
    process: &mut Child,
    path: &Path,
    timeout: Duration,
) -> Result<RecordingCompletionStatus, RecordingError> {
    // ffmpeg may have already exited on its own (e.g. crashed, or hit the `-fs` size cap) before
    // `stop` was called; there's nothing to finalize in that case.
    if process
        .try_wait()
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to poll ffmpeg: {error}"),
        })?
        .is_some()
    {
        return Ok(RecordingCompletionStatus::StoppedEarly);
    }

    let Some(mut stdin) = process.stdin.take() else {
        kill_and_reap(process).await;
        remove_recording_files(path);
        return Err(RecordingError::Finalize {
            reason: "ffmpeg stdin is unavailable for graceful finalization".to_string(),
        });
    };
    if let Err(error) = stdin.write_all(b"q\n").await {
        drop(stdin);
        kill_and_reap(process).await;
        remove_recording_files(path);
        return Err(RecordingError::Finalize {
            reason: format!("failed to request ffmpeg finalization: {error}"),
        });
    }
    // Dropping stdin closes the pipe, signaling EOF so ffmpeg processes the queued quit command.
    drop(stdin);

    match tokio::time::timeout(timeout, process.wait()).await {
        Ok(Ok(_)) => Ok(RecordingCompletionStatus::Completed),
        Ok(Err(error)) => {
            kill_and_reap(process).await;
            remove_recording_files(path);
            Err(RecordingError::Finalize {
                reason: format!("failed to wait for ffmpeg finalization: {error}"),
            })
        }
        Err(_) => {
            // ffmpeg missed the finalization deadline, so the container is likely missing its
            // moov atom and unplayable. Force-kill and discard the file rather than returning a
            // corrupt recording.
            kill_and_reap(process).await;
            remove_recording_files(path);
            Err(RecordingError::Finalize {
                reason: "ffmpeg did not finalize the recording in time".to_string(),
            })
        }
    }
}

/// Kills `process` and waits for it to exit.
///
/// Unlike POSIX, Windows won't let a file be deleted while another process still holds it open;
/// callers must reap ffmpeg here before removing its output, or cleanup will silently fail.
async fn kill_and_reap(process: &mut Child) {
    let _ = process.start_kill();
    let _ = process.wait().await;
}

fn remove_recording_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("log"));
}

/// Spawns a background thread that kills, reaps, and deletes an abandoned recording process and
/// its output files, without blocking the caller.
///
/// [`RecordingHandle`]'s `Drop` calls this when a recording is torn down without an explicit
/// `stop` (e.g. a cancelled or panicked caller). `Drop` cannot `.await` a process reap, and
/// unlike POSIX, Windows won't delete a file that ffmpeg still has open (see [`kill_and_reap`]),
/// so doing this work synchronously in `drop` would either block the dropping thread until
/// ffmpeg exits or silently leak the file.
pub(crate) fn spawn_abandoned_cleanup(mut process: Child, path: PathBuf) {
    let result = std::thread::Builder::new()
        .name("recording-cleanup".to_string())
        .spawn(move || {
            match process.try_wait() {
                Ok(Some(_)) => {
                    remove_abandoned_recording_files(&path);
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!("Failed to poll abandoned recording process: {error}");
                    return;
                }
            }
            if let Err(error) = process.start_kill() {
                log::warn!("Failed to terminate abandoned recording process: {error}");
                return;
            }
            loop {
                match process.try_wait() {
                    Ok(Some(_)) => {
                        remove_abandoned_recording_files(&path);
                        return;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(error) => {
                        log::warn!("Failed to reap abandoned recording process: {error}");
                        return;
                    }
                }
            }
        });
    if let Err(error) = result {
        log::warn!("Failed to start abandoned recording cleanup: {error}");
    }
}

/// Deletes an abandoned recording's files, retrying for up to
/// [`ABANDONED_RECORDING_CLEANUP_TIMEOUT`] instead of failing on the first attempt.
///
/// Unlike [`remove_recording_files`] (used right after `stop`/`launch_recording` reap ffmpeg
/// themselves), the process here was force-killed rather than exiting on its own, so another
/// process (e.g. antivirus scanning the freshly-written file) can transiently hold it open just
/// after ffmpeg releases it. Retrying absorbs that race instead of silently leaking the file.
fn remove_abandoned_recording_files(path: &Path) {
    let deadline = Instant::now() + ABANDONED_RECORDING_CLEANUP_TIMEOUT;
    let mut pending = vec![path.to_path_buf(), path.with_extension("log")];
    loop {
        let mut failed = Vec::new();
        for path in pending {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failed.push((path, error)),
            }
        }
        if failed.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            for (path, error) in failed {
                log::warn!(
                    "Failed to remove abandoned recording file {}: {}",
                    path.display(),
                    error
                );
            }
            return;
        }
        pending = failed.into_iter().map(|(path, _)| path).collect();
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Enriches a capture-start failure with a diagnostic tail from ffmpeg's log, special-casing the
/// known gdigrab access-denied signature (e.g. a locked or UAC-secured desktop) with an
/// actionable message.
fn capture_start_failure_reason(error: &str, log: &str) -> String {
    let diagnostic = diagnostic_tail(log);
    if log
        .to_ascii_lowercase()
        .contains("failed to capture image (error 5)")
    {
        format!(
            "{error}: gdigrab was denied access to the current Windows desktop session{diagnostic}"
        )
    } else {
        format!("{error}{diagnostic}")
    }
}

/// Returns a bounded, single-line tail of `text`'s last few non-empty lines (formatted as
/// `" (...)"`, or empty if `text` has no content), suitable for appending to an error message.
fn diagnostic_tail(text: &str) -> String {
    const MAX_CHARS: usize = 512;
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" ");
    if tail.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = tail.chars().collect();
    let tail = if chars.len() > MAX_CHARS {
        chars[chars.len() - MAX_CHARS..].iter().collect()
    } else {
        tail
    };
    format!(" ({tail})")
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
