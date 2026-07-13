//! Linux screen recording via a supervised ffmpeg sidecar process.
//!
//! There are two capture paths, selected by [`RecordingConfig::target`]:
//!
//! - `Target::Screen` (default, legacy): ffmpeg `x11grab` captures the whole X display straight
//!   to an ephemeral MP4 on disk (H.264 / yuv420p). `stop` sends SIGINT so ffmpeg finalizes the
//!   container (writes the moov atom) instead of leaving a truncated file.
//! - `Target::Window`: a background loop captures just the targeted window every frame via the
//!   X Composite extension's off-screen backing pixmap — so the window is recorded even while
//!   covered by another window — and pipes raw RGB frames into an ffmpeg `rawvideo` encoder over
//!   stdin. `stop` closes ffmpeg's stdin; the resulting EOF is what finalizes a stdin/rawvideo
//!   ffmpeg (SIGINT is the finalize trigger only for the x11grab path).

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use tokio::io::AsyncWriteExt as _;
use tokio::process::{Child, Command};
use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::xproto::{self, ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

use super::x11::convert_x11_image_to_rgb;
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
            // Record a single window via the Composite per-frame path, which sees the window even
            // when it is covered by another window.
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

        // The window-capture path finalizes ffmpeg by closing its stdin (EOF); the x11grab path
        // finalizes via SIGINT. Presence of a capture task/stop-flag distinguishes them.
        let is_window_capture = handle.capture_stop.is_some();

        let mut process = handle
            .process
            .take()
            .ok_or_else(|| RecordingError::Finalize {
                reason: "recording process is unavailable".to_string(),
            })?;

        let completion_status = if is_window_capture {
            finalize_window_capture(&mut handle, &mut process, &path).await?
        } else {
            finalize_screen_capture(&mut process, &path).await?
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
        capture_stop: None,
        capture_task: None,
    })
}

/// Starts a single-window recording via the Composite per-frame capture path.
///
/// The window's geometry is locked at start; every captured frame is padded (letterboxed) or
/// cropped to that constant `W`x`H` so the encoder never sees a dimension change.
async fn start_window(
    config: RecordingConfig,
    window: xproto::Window,
) -> Result<RecordingHandle, RecordingError> {
    let (conn, screen_index) =
        RustConnection::connect(None).map_err(|e| RecordingError::Environment {
            reason: format!("failed to connect to X11: {e}"),
        })?;
    let root = conn.setup().roots[screen_index].root;

    // Lock the capture dimensions to the window's geometry at start. libx264 with yuv420p
    // requires even dimensions.
    let geometry =
        windows::geometry(&conn, root, window).map_err(|e| RecordingError::Environment {
            reason: format!("failed to resolve window {window} geometry: {e}"),
        })?;
    let width = u32::from(geometry.width) & !1;
    let height = u32::from(geometry.height) & !1;
    let border_width = geometry.border_width;
    if width == 0 || height == 0 {
        return Err(RecordingError::Environment {
            reason: format!("invalid window dimensions {width}x{height}"),
        });
    }

    // Redirect the window so the server maintains its full contents off-screen, mirroring the
    // window-screenshot path. AUTOMATIC redirections are per-client and released when this
    // connection closes (so the connection must live for the whole recording). A redundant
    // redirect error is ignored: under a compositor's existing manual redirection,
    // NameWindowPixmap already works.
    conn.composite_query_version(0, 4)
        .map_err(|e| RecordingError::Environment {
            reason: format!("Composite extension not available: {e}"),
        })?
        .reply()
        .map_err(|e| RecordingError::Environment {
            reason: format!("Composite extension not available: {e}"),
        })?;
    if let Ok(cookie) = conn.composite_redirect_window(window, Redirect::AUTOMATIC) {
        let _ = cookie.check();
    }

    let path = std::env::temp_dir().join(format!("warp-recording-{}.mp4", uuid::Uuid::new_v4()));
    let log_path = path.with_extension("log");
    let log_file = std::fs::File::create(&log_path).map_err(|e| RecordingError::Start {
        reason: format!("failed to create the recording log file: {e}"),
    })?;

    let mut command = Command::new("ffmpeg");
    command
        .arg("-y")
        .args(["-f", "rawvideo"])
        .args(["-pix_fmt", "rgb24"])
        .args(["-video_size", &format!("{width}x{height}")])
        .args(["-framerate", &config.frame_rate.to_string()])
        .args(["-i", "-"])
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
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .kill_on_drop(true);

    let mut process = command.spawn().map_err(|e| RecordingError::Environment {
        reason: format!("failed to spawn ffmpeg: {e}"),
    })?;
    let stdin = process.stdin.take().ok_or_else(|| RecordingError::Start {
        reason: "failed to capture ffmpeg stdin".to_string(),
    })?;

    // Run the capture loop in the background, paced at the configured frame rate. It owns the X
    // connection (keeping the redirect alive) and ffmpeg's stdin; closing stdin on stop yields
    // the EOF that finalizes the encoder.
    let stop = Arc::new(AtomicBool::new(false));
    let capture_task = tokio::spawn(run_capture_loop(
        conn,
        root,
        window,
        border_width,
        width,
        height,
        config.frame_rate.max(1),
        stdin,
        stop.clone(),
    ));

    // Resolve once capture is confirmed live (ffmpeg has produced output from the frames the
    // loop is feeding it).
    if let Err(e) = wait_for_first_output(&path, &mut process).await {
        stop.store(true, Ordering::Release);
        capture_task.abort();
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
        capture_stop: Some(stop),
        capture_task: Some(capture_task),
    })
}

/// The background loop that captures the window every frame and writes raw RGB frames into
/// ffmpeg's stdin. Returns when stopped, when the window disappears, or when ffmpeg's stdin is
/// closed; on return it drops `stdin`, whose EOF finalizes the encoder.
#[allow(clippy::too_many_arguments)]
async fn run_capture_loop(
    conn: RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    border_width: u16,
    width: u32,
    height: u32,
    frame_rate: u32,
    mut stdin: tokio::process::ChildStdin,
    stop: Arc<AtomicBool>,
) {
    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(frame_rate));
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match capture_window_frame(&conn, root, window, border_width, width, height) {
            Ok(frame) => {
                if stdin.write_all(&frame).await.is_err() {
                    // ffmpeg exited (a limit was reached, or it crashed); stop feeding it.
                    break;
                }
            }
            // The window disappeared or a GetImage failed: finalize gracefully with whatever was
            // captured rather than error out a partial-but-valid recording.
            Err(_) => break,
        }
        tokio::time::sleep(frame_interval).await;
    }
    // Dropping stdin sends EOF, which ffmpeg needs to finalize the container.
    let _ = stdin.shutdown().await;
}

/// Captures one frame of `window` via the Composite backing pixmap and returns exactly
/// `width * height * 3` RGB bytes. If the window is now smaller than the locked capture size the
/// frame is padded with black; if larger it is cropped, so the encoder always sees constant
/// dimensions.
fn capture_window_frame(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    border_width: u16,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let mut frame = vec![0u8; (width as usize) * (height as usize) * 3];

    // Clamp the capture rectangle to the window's current content box, so a shrunk window does
    // not read past its pixmap.
    let geometry = windows::geometry(conn, root, window)?;
    let capture_width = u32::from(geometry.width).min(width) as u16;
    let capture_height = u32::from(geometry.height).min(height) as u16;
    if capture_width == 0 || capture_height == 0 {
        // Nothing to copy this frame; emit a black frame to keep the stream going.
        return Ok(frame);
    }

    let pixmap = conn
        .generate_id()
        .map_err(|e| format!("Failed to allocate a pixmap id: {e}"))?;
    conn.composite_name_window_pixmap(window, pixmap)
        .map_err(|e| format!("Failed to name the window pixmap: {e}"))?
        .check()
        .map_err(|e| format!("Failed to name the window pixmap: {e}"))?;
    // The backing pixmap includes the window border; the content box starts at
    // (border_width, border_width).
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            border_width as i16,
            border_width as i16,
            capture_width,
            capture_height,
            !0, // plane_mask: all planes
        )
        .map_err(|e| format!("Failed to request window frame: {e}"))
        .and_then(|cookie| {
            cookie
                .reply()
                .map_err(|e| format!("Failed to capture window {window}: {e}"))
        });
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.flush();
    let image = image?;

    let rgb = convert_x11_image_to_rgb(
        &image.data,
        capture_width as usize,
        capture_height as usize,
        image.depth,
    )?;

    // Copy the captured region into the top-left of the constant-size (letterboxed) frame.
    let src_stride = capture_width as usize * 3;
    let dst_stride = width as usize * 3;
    for row in 0..capture_height as usize {
        let src = row * src_stride;
        let dst = row * dst_stride;
        if src + src_stride <= rgb.len() && dst + src_stride <= frame.len() {
            frame[dst..dst + src_stride].copy_from_slice(&rgb[src..src + src_stride]);
        }
    }
    Ok(frame)
}

/// Finalizes an x11grab (screen) recording: SIGINT makes ffmpeg flush and write the moov atom.
async fn finalize_screen_capture(
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

/// Finalizes a window-capture recording: stop the capture loop, then close ffmpeg's stdin (the
/// EOF is what finalizes a stdin/rawvideo ffmpeg).
async fn finalize_window_capture(
    handle: &mut RecordingHandle,
    process: &mut Child,
    path: &Path,
) -> Result<RecordingCompletionStatus, RecordingError> {
    if let Some(stop) = handle.capture_stop.take() {
        stop.store(true, Ordering::Release);
    }
    // Wait for the capture loop to exit; it drops ffmpeg's stdin on the way out, sending EOF.
    if let Some(task) = handle.capture_task.take() {
        let _ = tokio::time::timeout(STOP_TIMEOUT, task).await;
    }

    match process.try_wait().map_err(|e| RecordingError::Finalize {
        reason: format!("failed to poll ffmpeg: {e}"),
    })? {
        // ffmpeg already exited (e.g. a capture limit was reached before stop).
        Some(_) => Ok(RecordingCompletionStatus::StoppedEarly),
        None => wait_for_finalization(process, path, RecordingCompletionStatus::Completed).await,
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
