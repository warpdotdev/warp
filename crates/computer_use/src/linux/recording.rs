//! Linux screen recording: the `x11grab` input adapter over the shared recording core.
//!
//! There are two capture paths, selected by [`RecordingConfig::target`]:
//!
//! - `Target::Screen` (default, legacy): ffmpeg `x11grab` captures the whole X display straight
//!   to an ephemeral MP4 on disk (H.264 / yuv420p).
//! - `Target::Window`: the target window is raised if needed, verified as foreground-visible at
//!   representative points, and captured via ffmpeg `x11grab -window_id`.
//!
//! The temp-file lifecycle, encode settings, launch/finalize supervision, and post-stop
//! processing all live in [`crate::recording`].

use std::time::Duration;

use async_trait::async_trait;
use instant::Instant;
use pathfinder_geometry::vector::Vector2I;
use tokio::process::Command;
use x11rb::connection::Connection;
use x11rb::protocol::xproto;
use x11rb::rust_connection::RustConnection;

use super::x11::windows;
use crate::recording::capture;
use crate::{RecordingConfig, RecordingError, RecordingHandle, RecordingOutput, Target};

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

    async fn stop(&self, handle: RecordingHandle) -> Result<RecordingOutput, RecordingError> {
        capture::finalize_capture(handle).await
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

    let files = capture::new_capture_files()?;
    let command = new_ffmpeg_capture_command(&config, &display, width, height, None);
    // x11grab's own error text already names the display problem, so there is
    // nothing platform-specific to add to a failed start.
    capture::launch_capture(command, files, width, height, None).await
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

    let files = capture::new_capture_files()?;
    let command = new_ffmpeg_capture_command(&config, &display, width, height, Some(window));
    capture::launch_capture(command, files, width, height, None).await
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
    // Do NOT composite the X11 cursor (must come BEFORE -i so ffmpeg treats it
    // as an x11grab input option, not an output option; it defaults to 1).
    // XFixes only reports the user's core cursor: background computer use drives
    // a private MPX agent seat whose cursor x11grab can never capture, so
    // compositing would show a frozen, unrelated cursor next to the burned-in
    // click annotations. The post-stop burn-in synthesizes a cursor from the
    // recorded pointer events instead, identically for screen and window scopes.
    command.args(["-draw_mouse", "0"]);
    capture::push_duration_limit(&mut command, config.max_duration);
    command.args(["-i", display]);
    capture::push_encode_args(&mut command, config.max_size_bytes, None);
    command
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

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
