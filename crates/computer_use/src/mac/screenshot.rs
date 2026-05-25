use command::blocking::Command;

use super::util::main_display_scale_factor;
use super::window;
use crate::{CapturedWindow, ScreenshotParams, Target};

/// Captures a screenshot according to `params`, using the built-in macOS `screencapture` CLI.
///
/// When the params target a window, that specific window is captured (without raising it) and the
/// returned [`CapturedWindow`] describes the image so window-local coordinates can be mapped onto
/// it. Otherwise the main display is captured and the second tuple element is `None`.
pub fn take(
    params: ScreenshotParams,
) -> Result<(crate::Screenshot, Option<CapturedWindow>), String> {
    match params.target {
        Target::Window { window_id, .. } => take_window(window_id, params),
        Target::Screen => Ok((take_screen(params)?, None)),
    }
}

/// Captures the main display, optionally restricted to a region (legacy behavior).
fn take_screen(params: ScreenshotParams) -> Result<crate::Screenshot, String> {
    let output_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temporary directory for screenshot: {e}"))?;
    let output_path = output_dir.path().join("screenshot.png");

    let mut cmd = Command::new("/usr/sbin/screencapture");
    cmd.args([
        "-x",    // Do not play sounds.
        "-tpng", // Capture to PNG format.
        "-m",    // Only capture the main display (not all displays).
    ]);

    if let Some(region) = params.region {
        region.validate()?;
        // -R x,y,w,h captures a specific rectangle in point coordinates.
        // Convert from physical pixel coordinates to point coordinates.
        let scale = main_display_scale_factor();
        let x = (region.top_left.x() as f64 / scale) as i32;
        let y = (region.top_left.y() as f64 / scale) as i32;
        let w = ((region.bottom_right.x() - region.top_left.x()) as f64 / scale) as i32;
        let h = ((region.bottom_right.y() - region.top_left.y()) as f64 / scale) as i32;
        cmd.arg("-R").arg(format!("{x},{y},{w},{h}"));
    }

    let output = cmd
        .arg(&output_path)
        .output()
        .map_err(|e| format!("Failed to run screencapture: {e}"))?;

    check_status(&output)?;

    crate::screenshot_utils::load_and_process_screenshot(&output_path, params)
}

/// Captures a single window by its `CGWindowID` without raising it, returning the processed image
/// plus metadata describing the captured pixels and scale factor.
fn take_window(
    window_id: u32,
    params: ScreenshotParams,
) -> Result<(crate::Screenshot, Option<CapturedWindow>), String> {
    let output_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temporary directory for screenshot: {e}"))?;
    let output_path = output_dir.path().join("window.png");

    let output = Command::new("/usr/sbin/screencapture")
        .args([
            "-x",    // Do not play sounds.
            "-tpng", // Capture to PNG format.
            "-o",    // Omit the window's drop shadow.
        ])
        // -l <windowid> captures only the window with the given id, even when it is not frontmost.
        .arg("-l")
        .arg(window_id.to_string())
        .arg(&output_path)
        .output()
        .map_err(|e| format!("Failed to run screencapture: {e}"))?;

    check_status(&output)?;

    let screenshot = crate::screenshot_utils::load_and_process_screenshot(&output_path, params)?;

    // Derive the scale factor (pixels per point) from the native capture dimensions and the
    // window's point bounds. This assumes the window lives on the main display and shares its
    // backing scale; multi-display / mixed-scale handling is a follow-up.
    let scale_factor = window::window_by_id(window_id)
        .filter(|info| info.width > 0.0)
        .map(|info| screenshot.original_width as f64 / info.width)
        .unwrap_or_else(main_display_scale_factor);

    // The captured metadata refers to the native (pre-downscale) capture, so window-local pixel
    // coordinates sent by the agent map directly onto the captured window image.
    let captured = CapturedWindow {
        window_id,
        width_px: screenshot.original_width as i32,
        height_px: screenshot.original_height as i32,
        scale_factor,
    };

    Ok((screenshot, Some(captured)))
}

/// Returns an error describing a failed `screencapture` invocation.
fn check_status(output: &std::process::Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = if stderr.trim().is_empty() {
        format!("exit code {}", output.status)
    } else {
        format!("exit code {}: {}", output.status, stderr.trim())
    };
    Err(format!("screencapture failed with {detail}"))
}
