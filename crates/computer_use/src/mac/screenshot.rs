use command::blocking::Command;
use image::GenericImageView;

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
/// plus metadata describing the captured pixels.
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

    let image = image::ImageReader::open(&output_path)
        .map_err(|e| format!("Failed to open screenshot file: {e}"))?
        .decode()
        .map_err(|e| format!("Failed to decode screenshot: {e}"))?;
    let (full_width_px, full_height_px) = image.dimensions();
    let window = window::window_by_id(window_id)
        .ok_or_else(|| format!("Failed to resolve captured window {window_id}."))?;
    if window.width <= 0.0 {
        return Err(format!(
            "Captured window {window_id} has invalid point width."
        ));
    }

    // A single capture-derived ratio correctly maps windows wholly contained on any one display.
    // Windows spanning displays with different backing scale factors are not yet supported,
    // because no single pixels-per-point ratio can accurately map the entire window.
    let pixels_per_point = full_width_px as f64 / window.width;
    let image = if let Some(region) = params.region {
        region.validate()?;
        if region.bottom_right.x() as u32 > full_width_px
            || region.bottom_right.y() as u32 > full_height_px
        {
            return Err(format!(
                "Screenshot region ({}, {}) to ({}, {}) is outside window {window_id} dimensions {full_width_px}x{full_height_px}.",
                region.top_left.x(),
                region.top_left.y(),
                region.bottom_right.x(),
                region.bottom_right.y(),
            ));
        }
        image.crop_imm(
            region.top_left.x() as u32,
            region.top_left.y() as u32,
            (region.bottom_right.x() - region.top_left.x()) as u32,
            (region.bottom_right.y() - region.top_left.y()) as u32,
        )
    } else {
        image
    };
    let screenshot = crate::screenshot_utils::process_screenshot(image, params)?;

    // Diagnostics for target-relative capture and coordinate mapping. The model is shown the
    // `sent` image after resize, while server coordinate translation uses the `native` size.
    // Logging both sizes and the capture-derived point ratio makes mapping errors visible.
    if std::env::var_os("COMPUTER_USE_DEBUG").is_some() {
        let (pt_w, pt_h) = (window.width, window.height);
        let downscale = screenshot.width as f64 / (screenshot.original_width.max(1) as f64);
        log::info!(
            "[computer_use] window capture window#={window_id} native_px={}x{} sent_px={}x{} \
             downscale={downscale:.4} window_pt={pt_w:.1}x{pt_h:.1} pixels_per_point={pixels_per_point:.3} \
             (max_long_edge_px={:?} max_total_px={:?})",
            screenshot.original_width,
            screenshot.original_height,
            screenshot.width,
            screenshot.height,
            params.max_long_edge_px,
            params.max_total_px,
        );
    }

    // The captured metadata refers to the native (pre-downscale) capture, so window-local pixel
    // coordinates sent by the agent map directly onto the captured window image.
    let captured = CapturedWindow {
        window_id,
        width_px: screenshot.original_width as i32,
        height_px: screenshot.original_height as i32,
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
