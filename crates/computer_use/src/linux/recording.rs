//! Linux screen recording via a supervised ffmpeg sidecar process.
//!
//! There are two capture paths, selected by [`RecordingConfig::target`]:
//!
//! - `Target::Screen` (default, legacy): ffmpeg `x11grab` captures the whole X display straight
//!   to an ephemeral MP4 on disk (H.264 / yuv420p). `stop` sends SIGINT so ffmpeg finalizes the
//!   container (writes the moov atom) instead of leaving a truncated file.
//! - `Target::Window`: the target window is raised if needed, verified as foreground-visible at
//!   representative points, and captured via ffmpeg `x11grab -window_id`.

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use pathfinder_geometry::vector::Vector2I;
use tokio::process::{Child, Command};
use x11rb::connection::Connection;
use x11rb::protocol::xproto;
use x11rb::rust_connection::RustConnection;

use super::x11::windows;
use crate::{
    RecordingCompletionStatus, RecordingConfig, RecordingError, RecordingHandle, RecordingOutput,
    Target,
};

/// How long to wait for ffmpeg to open the display and produce first output.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for ffmpeg to finalize the container after stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// Poll interval while waiting for capture to begin.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How often to check whether a requested window raise has taken effect.
const RAISE_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long to wait for a target window to become visible enough for native recording.
const RAISE_TIMEOUT: Duration = Duration::from_millis(500);

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl crate::Recorder for Recorder {
    async fn start(&self, config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
        match config.target {
            Target::Window { window_id, .. } => start_window(config, window_id).await,
            // Record the whole display via ffmpeg x11grab (legacy behavior).
            Target::Screen => start_screen(config).await,
        }
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

        let completion_status = finalize_capture(&mut process, &path).await?;

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

/// Starts a full-display recording via ffmpeg `x11grab` (legacy behavior).
async fn start_screen(config: RecordingConfig) -> Result<RecordingHandle, RecordingError> {
    let display = std::env::var("DISPLAY").map_err(|_| RecordingError::Environment {
        reason: "DISPLAY is not set (X11 required)".to_string(),
    })?;

    // libx264 with yuv420p requires even dimensions.
    let (width, height) = query_display_dimensions()?;
    let width = width & !1;
    let height = height & !1;
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid display dimensions {width}x{height}"),
        });
    }

    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    // ffmpeg's progress log goes to a file so its stderr pipe can never fill
    // and stall capture over a long recording.
    let log_path = path.with_extension("log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| RecordingError::Start {
        reason: format!("failed to create the recording log file: {e}"),
    })?;

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

/// Starts a single-window recording via ffmpeg `x11grab -window_id`.
///
/// This is a foreground-visible capture path: the target is raised if representative points are
/// not already visible, then ffmpeg records that window directly.
async fn start_window(
    config: RecordingConfig,
    window: xproto::Window,
) -> Result<RecordingHandle, RecordingError> {
    let (display, width, height) = prepare_window_capture(window).await?;

    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    let log_path = path.with_extension("log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| RecordingError::Start {
        reason: format!("failed to create the recording log file: {e}"),
    })?;

    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-f", "x11grab"])
        .args(["-framerate", &config.frame_rate.to_string()])
        .args(["-video_size", &format!("{width}x{height}")])
        .args(["-window_id", &format!("0x{window:x}")])
        .args(["-i", &display])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"]);
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

    // Resolve once capture is confirmed live (the output file has grown).
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
async fn prepare_window_capture(
    window: xproto::Window,
) -> Result<(String, u32, u32), RecordingError> {
    let display = std::env::var("DISPLAY").map_err(|_| RecordingError::Environment {
        reason: "DISPLAY is not set (X11 required)".to_string(),
    })?;
    let (conn, screen_index) =
        RustConnection::connect(None).map_err(|e| RecordingError::Environment {
            reason: format!("failed to connect to X11: {e}"),
        })?;
    let root = conn.setup().roots[screen_index].root;
    let geometry =
        windows::geometry(&conn, root, window).map_err(|e| RecordingError::Environment {
            reason: format!("failed to resolve window {window} geometry: {e}"),
        })?;
    let width = u32::from(geometry.width) & !1;
    let height = u32::from(geometry.height) & !1;
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid window dimensions {width}x{height}"),
        });
    }
    ensure_window_visible_for_recording(&conn, root, window, geometry)
        .await
        .map_err(|e| RecordingError::Start { reason: e })?;
    Ok((display, width, height))
}

async fn ensure_window_visible_for_recording(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    geometry: windows::WindowGeometry,
) -> Result<(), String> {
    let points = visibility_sample_points(geometry);
    if window_visible_at_points(conn, root, window, &points)? {
        return Ok(());
    }

    windows::raise(conn, window)?;
    let start = Instant::now();
    loop {
        if window_visible_at_points(conn, root, window, &points)? {
            return Ok(());
        }
        if start.elapsed() >= RAISE_TIMEOUT {
            return Err(format!(
                "Target window {window} could not be made foreground-visible for native recording."
            ));
        }
        tokio::time::sleep(RAISE_POLL_INTERVAL).await;
    }
}

fn window_visible_at_points(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    points: &[Vector2I],
) -> Result<bool, String> {
    for &point in points {
        if !windows::window_hit_at_point(conn, root, window, point)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn visibility_sample_points(geometry: windows::WindowGeometry) -> Vec<Vector2I> {
    let x = geometry.x;
    let y = geometry.y;
    let width = i32::from(geometry.width);
    let height = i32::from(geometry.height);
    let mut points = vec![Vector2I::new(x + width / 2, y + height / 2)];

    if width > 2 && height > 2 {
        let inset_x = (width / 10).clamp(1, 8);
        let inset_y = (height / 10).clamp(1, 8);
        let right = x + width - 1;
        let bottom = y + height - 1;
        points.extend([
            Vector2I::new(x + inset_x, y + inset_y),
            Vector2I::new(right - inset_x, y + inset_y),
            Vector2I::new(x + inset_x, bottom - inset_y),
            Vector2I::new(right - inset_x, bottom - inset_y),
        ]);
    }

    points
}

/// Finalizes an x11grab recording: SIGINT makes ffmpeg flush and write the moov atom.
async fn finalize_capture(
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

/// Waits up to [`STOP_TIMEOUT`] for ffmpeg to exit after being asked to finalize; on timeout the
/// container is likely missing its moov atom, so the file is discarded.
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

/// Queries the X11 root window's dimensions in physical pixels via `$DISPLAY`.
fn query_display_dimensions() -> Result<(u32, u32), RecordingError> {
    let (conn, screen_index) =
        RustConnection::connect(None).map_err(|e| RecordingError::Environment {
            reason: format!("failed to connect to X11: {e}"),
        })?;
    let screen = &conn.setup().roots[screen_index];
    Ok((
        screen.width_in_pixels as u32,
        screen.height_in_pixels as u32,
    ))
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

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
