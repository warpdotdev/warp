//! macOS screen recording via a supervised ffmpeg `avfoundation` sidecar process.
//!
//! Capture is streamed straight to an ephemeral MP4 on disk (H.264 / yuv420p);
//! nothing is buffered in memory. `stop` sends SIGINT so ffmpeg finalizes the
//! container (writes the moov atom) instead of leaving a truncated file.
//!
//! This mirrors [`crate::linux::recording`] structurally, swapping only the
//! ffmpeg input device from `x11grab` (`$DISPLAY`) to `avfoundation`
//! (`Capture screen 0:none`). The shared
//! [`RecordingHandle`](crate::RecordingHandle) lifecycle, encode settings, and
//! bounded-capture (`-t` / `-fs`) contract are identical to Linux; see the
//! code-split note in the REMOTE-2160 spec for why the
//! `wait_for_first_output` / `ffmpeg_error_tail` / SIGINT-finalize logic is
//! duplicated between the two recorder modules.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use nix::sys::signal::Signal;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use tokio::process::{Child, Command};

use super::util::main_display_dimensions;
use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

/// How long to wait for ffmpeg to open the display and produce first output.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for ffmpeg to finalize the container after SIGINT.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for capture to begin.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The avfoundation input spec for the main display, with no audio device.
///
/// The screen is selected by NAME rather than integer index: ffmpeg parses
/// `Capture screen %d` directly, and the name is stable/English where the index
/// shifts when the camera count changes (cameras precede screens in
/// avfoundation's combined index space). `none` disables audio capture. This
/// matches the macOS screenshot path's main-display-only behavior
/// (`screencapture -m`); multi-display support is out of scope.
const AVFOUNDATION_INPUT: &str = "Capture screen 0:none";

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        // libx264 with yuv420p requires even dimensions.
        let (width, height) = main_display_dimensions();
        let width = width & !1;
        let height = height & !1;
        if width == 0 || height == 0 {
            return Err(RecordingError::Environment {
                reason: format!("invalid display dimensions {width}x{height}"),
            });
        }

        let path =
            std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
        // ffmpeg's progress log goes to a file so its stderr pipe can never fill
        // and stall capture over a long recording.
        let log_path = path.with_extension("log");
        let log_file = std::fs::File::create(&log_path).map_err(|e| RecordingError::Start {
            reason: format!("failed to create the recording log file: {e}"),
        })?;

        let mut command = Command::new("ffmpeg");
        command
            .arg("-y")
            .args(["-f", "avfoundation"])
            .args(["-framerate", &config.frame_rate.to_string()])
            .args(["-capture_cursor", "1"])
            .args(["-capture_mouse_clicks", "1"])
            .args(["-pixel_format", "uyvy422"])
            .args(["-video_size", &format!("{width}x{height}")])
            .args(["-i", AVFOUNDATION_INPUT])
            .args(["-c:v", "libx264"])
            .args(["-preset", "ultrafast"])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-movflags", "+faststart"]);
        // Enforce capture limits in ffmpeg so abandoned recordings remain bounded.
        command
            .arg("-t")
            .arg(format!("{:.3}", config.max_duration.as_secs_f64()));
        command.arg("-fs").arg(config.max_size_bytes.to_string());
        command
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file))
            .kill_on_drop(true);

        let mut process = command.spawn().map_err(|e| RecordingError::Environment {
            reason: format!("failed to spawn ffmpeg: {e}"),
        })?;

        // Resolve once capture is confirmed live (the output file has grown,
        // meaning ffmpeg opened the display and the muxer is writing).
        if let Err(e) = wait_for_first_output(&path, &mut process).await {
            let _ = process.start_kill();
            let detail = ffmpeg_error_tail(&std::fs::read_to_string(&log_path).unwrap_or_default());
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&log_path);
            return Err(RecordingError::Start {
                reason: format!("{e}{detail}"),
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

        // Finalize gracefully: SIGINT makes ffmpeg flush and write the moov atom.
        let completion_status = match process.try_wait().map_err(|e| RecordingError::Finalize {
            reason: format!("failed to poll ffmpeg: {e}"),
        })? {
            Some(_) => RecordingCompletionStatus::StoppedEarly,
            None => {
                let mut completion_status = RecordingCompletionStatus::Completed;
                if let Some(pid) = process.id() {
                    let pid = Pid::from_raw(pid as i32);
                    if kill(pid, Signal::SIGINT).is_err() {
                        completion_status = RecordingCompletionStatus::StoppedEarly;
                    }
                } else {
                    completion_status = RecordingCompletionStatus::StoppedEarly;
                }

                match tokio::time::timeout(STOP_TIMEOUT, process.wait()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => completion_status = RecordingCompletionStatus::StoppedEarly,
                    Err(_) => {
                        // ffmpeg missed the finalization deadline, so the container is
                        // likely missing its moov atom and unplayable. Force-kill and
                        // discard the file rather than returning a corrupt recording.
                        let _ = process.start_kill();
                        let _ = process.wait().await;
                        let _ = std::fs::remove_file(&path);
                        return Err(RecordingError::Finalize {
                            reason: "ffmpeg did not finalize the recording in time".to_string(),
                        });
                    }
                }
                completion_status
            }
        };

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
