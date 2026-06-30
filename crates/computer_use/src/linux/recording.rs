//! Linux screen recording via a supervised ffmpeg `x11grab` sidecar process.
//!
//! Capture is streamed straight to an ephemeral MP4 on disk (H.264 / yuv420p);
//! nothing is buffered in memory. `stop` sends SIGINT so ffmpeg finalizes the
//! container (writes the moov atom) instead of leaving a truncated file.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use tokio::process::{Child, Command};
use x11rb::connection::Connection;
use x11rb::rust_connection::RustConnection;

use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
};

/// How long to wait for ffmpeg to open the display and produce first output.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for ffmpeg to finalize the container after SIGINT.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for capture to begin.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        let display = std::env::var("DISPLAY").map_err(|_| RecordingError::MissingDisplay)?;

        // libx264 with yuv420p requires even dimensions.
        let (width, height) = query_display_dimensions()?;
        let width = width & !1;
        let height = height & !1;
        if width == 0 || height == 0 {
            return Err(RecordingError::InvalidDimensions { width, height });
        }

        let path =
            std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
        // ffmpeg's progress log goes to a file so its stderr pipe can never fill
        // and stall capture over a long recording.
        let log_path = path.with_extension("log");
        let log_file = std::fs::File::create(&log_path)
            .map_err(|source| RecordingError::CreateLogFile { source })?;

        let mut command = Command::new("ffmpeg");
        command
            .arg("-y")
            .args(["-f", "x11grab"])
            .args(["-framerate", &config.frame_rate.to_string()])
            .args(["-video_size", &format!("{width}x{height}")])
            .args(["-i", &display])
            .args(["-c:v", "libx264"])
            .args(["-preset", "ultrafast"])
            .args(["-pix_fmt", "yuv420p"])
            .args(["-movflags", "+faststart"]);
        // Enforce capture limits in ffmpeg so abandoned recordings remain bounded.
        if let Some(max_duration) = config.max_duration {
            command
                .arg("-t")
                .arg(format!("{:.3}", max_duration.as_secs_f64()));
        }
        if let Some(max_size_bytes) = config.max_size_bytes {
            command.arg("-fs").arg(max_size_bytes.to_string());
        }
        command
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file))
            .kill_on_drop(true);

        let mut process = command
            .spawn()
            .map_err(|source| RecordingError::SpawnFfmpeg { source })?;

        // Resolve once capture is confirmed live (the output file has grown,
        // meaning ffmpeg opened the display and the muxer is writing).
        if let Err(e) = wait_for_first_output(&path, &mut process).await {
            let _ = process.start_kill();
            let detail = ffmpeg_error_tail(&std::fs::read_to_string(&log_path).unwrap_or_default());
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&log_path);
            return Err(RecordingError::StartFailed {
                error: e.to_string(),
                detail,
            });
        }
        let _ = std::fs::remove_file(&log_path);

        Ok(RecordingHandle {
            width,
            height,
            frame_rate: config.frame_rate,
            max_duration: config.max_duration,
            max_size_bytes: config.max_size_bytes,
            path,
            started_at: Instant::now(),
            process,
        })
    }

    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        let RecordingHandle {
            width,
            height,
            frame_rate: _,
            max_duration: _,
            max_size_bytes: _,
            path,
            started_at,
            mut process,
        } = handle;

        let duration = started_at.elapsed();

        // Finalize gracefully: SIGINT makes ffmpeg flush and write the moov atom.
        let completion_status = match process
            .try_wait()
            .map_err(|source| RecordingError::PollFfmpeg { source })?
        {
            Some(_) => RecordingCompletionStatus::StoppedEarly,
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

                match tokio::time::timeout(STOP_TIMEOUT, process.wait()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => completion_status = RecordingCompletionStatus::StoppedEarly,
                    Err(_) => {
                        // ffmpeg did not finalize in time; force-kill so we don't leak it.
                        let _ = process.start_kill();
                        let _ = process.wait().await;
                        completion_status = RecordingCompletionStatus::StoppedEarly;
                    }
                }
                completion_status
            }
        };

        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size_bytes == 0 {
            let _ = std::fs::remove_file(&path);
            return Err(RecordingError::EmptyOutput);
        }

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

/// Queries the X11 root window's dimensions in physical pixels via `$DISPLAY`.
fn query_display_dimensions() -> Result<(u32, u32), RecordingError> {
    let (conn, screen_index) =
        RustConnection::connect(None).map_err(|e| RecordingError::X11Connection(e.to_string()))?;
    let screen = &conn.setup().roots[screen_index];
    Ok((
        screen.width_in_pixels as u32,
        screen.height_in_pixels as u32,
    ))
}

/// Waits until the recording file has grown (capture is live) or ffmpeg exits.
async fn wait_for_first_output(path: &Path, process: &mut Child) -> Result<(), RecordingError> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(status) = process
            .try_wait()
            .map_err(|source| RecordingError::PollFfmpeg { source })?
        {
            return Err(RecordingError::FfmpegExitedEarly { status });
        }
        if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > 0 {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(RecordingError::StartTimedOut);
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
