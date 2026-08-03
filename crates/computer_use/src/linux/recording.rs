//! Linux screen recording via a supervised ffmpeg sidecar process.
//!
//! There are two capture paths, selected by [`RecordingConfig::target`]:
//!
//! - `Target::Screen` (default, legacy): ffmpeg `x11grab` captures the whole X display straight
//!   to an ephemeral MP4 on disk (H.264 / yuv420p). `stop` sends SIGINT so ffmpeg finalizes the
//!   container (writes the moov atom) instead of leaving a truncated file.
//! - `Target::Window`: the target window is raised if needed, verified as foreground-visible at
//!   representative points, and captured via ffmpeg `x11grab -window_id`.

use std::fs::File;
use std::path::{Path, PathBuf};
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

    let (path, log_path, log_file) = new_recording_path()?;
    let command = new_ffmpeg_capture_command(&config, &display, width, height, None);
    launch_recording(command, path, log_path, log_file, width, height).await
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

    let (path, log_path, log_file) = new_recording_path()?;
    let command = new_ffmpeg_capture_command(&config, &display, width, height, Some(window));
    launch_recording(command, path, log_path, log_file, width, height).await
}

fn new_recording_path() -> Result<(PathBuf, PathBuf, File), RecordingError> {
    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    let log_path = path.with_extension("log");
    // ffmpeg's progress log goes to a file so its stderr pipe can never fill
    // and stall capture over a long recording.
    let log_file = File::create(&log_path).map_err(|e| RecordingError::Start {
        reason: format!("failed to create the recording log file: {e}"),
    })?;
    Ok((path, log_path, log_file))
}

fn new_ffmpeg_capture_command(
    config: &RecordingConfig,
    display: &str,
    width: u32,
    height: u32,
    window_id: Option<xproto::Window>,
) -> Command {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-f", "x11grab"])
        .args(["-framerate", &config.frame_rate.to_string()])
        .args(["-video_size", &format!("{width}x{height}")]);
    if let Some(window) = window_id {
        command.args(["-window_id", &format!("0x{window:x}")]);
    }
    command
        // Composite the X11 cursor. Must come BEFORE -i so ffmpeg
        // treats it as an x11grab input option, not an output option.
        .args(["-draw_mouse", "1"])
        // Limit capture wall-clock time as an INPUT option so the duration
        // bound is the real capture wall-clock time. The Linux master is
        // captured at 1x; the smart variable cut runs as a post-stop pass,
        // so there is no live setpts speed filter to stretch this bound.
        .arg("-t")
        .arg(format!("{:.3}", config.max_duration.as_secs_f64()))
        .args(["-i", display])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"]);
    // The Linux master is captured at 1x so the post-stop smart cut can keep
    // real action windows at full speed and remove only blocked/thinking gaps.
    // The server/default `playback_speed_multiplier` is intentionally not
    // applied here; it remains accepted for wire compatibility and is still
    // used by the macOS avfoundation fallback.
    // Max file size is an output limit; stays as an output option.
    command
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
    width: u32,
    height: u32,
) -> Result<RecordingHandle, RecordingError> {
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

/// Cuts `input` to only the retained action segments and returns the path to
/// the trimmed file (a sibling of `input` with extension `cut.mp4`). The
/// original 1x master is left untouched; the caller owns cleanup of both.
///
/// Each retained segment is extracted via ffmpeg `trim`/`setpts=PTS-STARTPTS`
/// and the strips are concatenated (`concat=n=N:v=1:a=0`, video-only). Source
/// gaps between segments are removed entirely, producing a compact 1x video
/// that contains only the real action windows. This step is deliberately free
/// of overlay logic; overlays are applied separately in `burn_overlays_into_cut`.
async fn cut_to_segments(
    input: &Path,
    segments: &[crate::overlay::KeepSegment],
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let output_path = input.with_extension("cut.mp4");
    let filter = build_cut_only_filtergraph(segments);
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("[vout]")
        // Force a constant output frame rate so every retained frame — including
        // the cut's final frame, which would otherwise have no defined duration
        // and be dropped by the muxer — is written. The source master is
        // captured at `frame_rate`, so this matches its cadence without
        // duplicating or dropping frames.
        .args(["-r", &frame_rate.to_string()])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(output_path),
        Ok(status) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("ffmpeg segment cut exited with status {status}"),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("failed to run ffmpeg for segment cut: {e}"),
            })
        }
    }
}

/// Burns the remapped ASS overlay pills into an already-cut `input` video,
/// returning the path to the annotated file (a sibling with extension
/// `overlay.mp4`). The cut input is left untouched; the caller owns cleanup of
/// both. This step is deliberately free of segment-cut logic; cutting is done
/// separately in `cut_to_segments`.
async fn burn_overlays_into_cut(
    input: &Path,
    ass_path: &Path,
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let output_path = input.with_extension("overlay.mp4");
    let subtitles_filter = format!("subtitles=filename='{}'", ass_path.display());
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-vf", &subtitles_filter])
        .args(["-r", &frame_rate.to_string()])
        .args(["-c:v", "libx264"])
        .args(["-preset", "ultrafast"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-movflags", "+faststart"])
        .arg(&output_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    match status {
        Ok(status) if status.success() => Ok(output_path),
        Ok(status) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("ffmpeg overlay burn-in exited with status {status}"),
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&output_path);
            Err(RecordingError::Finalize {
                reason: format!("failed to run ffmpeg for overlay burn-in: {e}"),
            })
        }
    }
}

/// Post-stop pipeline: cut the 1x source to retained action segments, then
/// burn remapped overlay pills into the result. Returns the path to the final
/// annotated file (a sibling of `input`). The original 1x master and the
/// intermediate cut file are left untouched; the caller owns cleanup of all
/// produced paths. ffmpeg demuxes each mp4 from disk frame-by-frame, so the
/// whole recording is never buffered in memory.
///
/// The two steps are independent: `cut_to_segments` knows nothing about
/// overlays, and `burn_overlays_into_cut` knows nothing about segment
/// boundaries. A recording whose committed actions yield no qualifying segment
/// returns an error rather than producing a video; the caller falls back to
/// uploading the untouched source for an unexpected processing failure after at
/// least one committed action.
pub async fn post_process_recording(
    input: &Path,
    entries: &[crate::ActionLogEntry],
    dimensions: (u32, u32),
    source_duration: Duration,
    frame_rate: u32,
) -> Result<PathBuf, RecordingError> {
    let segments = crate::overlay::build_keep_segments(entries, source_duration, frame_rate);
    if segments.is_empty() {
        return Err(RecordingError::Finalize {
            reason: "recording has no qualifying action segments to keep".to_string(),
        });
    }

    // Step 1: cut the source to retained segments only.
    let cut_path = cut_to_segments(input, &segments, frame_rate).await?;

    // Step 2: write the remapped ASS and burn overlays into the cut video.
    let ass_path = input.with_extension("ass");
    let write_result = std::fs::write(
        &ass_path,
        crate::overlay::build_overlay_ass(entries, dimensions, source_duration, frame_rate),
    );
    let overlay_result = match write_result {
        Ok(()) => burn_overlays_into_cut(&cut_path, &ass_path, frame_rate).await,
        Err(e) => Err(RecordingError::Finalize {
            reason: format!("failed to write overlay subtitle file: {e}"),
        }),
    };
    // The subtitle file is an implementation detail; drop it regardless of outcome.
    let _ = std::fs::remove_file(&ass_path);
    // The intermediate cut file is no longer needed once overlays are applied
    // (or failed); the caller uploads the overlay output or falls back to the
    // original source on any error.
    let _ = std::fs::remove_file(&cut_path);

    overlay_result
}

/// Builds the ffmpeg `filter_complex` for the segment-cut step only (no
/// overlays). For each retained segment the input video is `trim`med to its
/// source `[start, end)` window and reset to a zero-based PTS
/// (`setpts=PTS-STARTPTS`); the trimmed strips are concatenated in source
/// order (`concat=n=N:v=1:a=0`, video-only). The result is mapped to the
/// `[vout]` label by the caller. This removes only the dead source frames
/// and preserves the 1x frame cadence inside each retained segment.
fn build_cut_only_filtergraph(segments: &[crate::overlay::KeepSegment]) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(segments.len() + 1);
    for (index, segment) in segments.iter().enumerate() {
        let start = segment.source_start.as_secs_f64();
        let end = segment.source_end.as_secs_f64();
        // `trim` selects the source frame range; `setpts=PTS-STARTPTS` relabels
        // the strip's first frame as time zero so the old gap timestamp is not
        // carried into the concatenated output.
        parts.push(format!(
            "[0:v]trim=start={start:.6}:end={end:.6},setpts=PTS-STARTPTS[v{index}]"
        ));
    }
    let inputs: String = (0..segments.len())
        .map(|index| format!("[v{index}]"))
        .collect::<Vec<_>>()
        .join("");
    let n = segments.len();
    parts.push(format!("{inputs}concat=n={n}:v=1:a=0[vout]"));
    parts.join(";")
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
