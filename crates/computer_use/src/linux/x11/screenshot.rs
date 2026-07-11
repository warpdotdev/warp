//! Screenshot capture for X11.

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::xproto::{self, ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

use super::windows;
use crate::{CapturedWindow, Screenshot, ScreenshotParams};

/// The maximum number of native pixels a window capture will read from the server.
///
/// The `GetImage` reply and the RGB conversion buffer are allocated at full native size before
/// the params' downscaling limits apply, so an enormous (or hostile) window could otherwise
/// make a capture allocate gigabytes. X11 window dimensions go up to `u16::MAX`, far beyond any
/// real display; this cap (32 * 1024 * 1024 ≈ 33.5M pixels) still comfortably covers a full 8K
/// display (7680 x 4320 ≈ 33.2M pixels).
const MAX_WINDOW_CAPTURE_PIXELS: usize = 32 * 1024 * 1024;

/// Takes a screenshot of the root window or a region of it.
pub fn take(
    conn: &RustConnection,
    screen: &xproto::Screen,
    root: xproto::Window,
    params: ScreenshotParams,
) -> Result<Screenshot, String> {
    // Determine the capture region.
    let (x, y, width, height) = if let Some(region) = params.region {
        region.validate()?;
        let x = region.top_left.x() as i16;
        let y = region.top_left.y() as i16;
        let width = (region.bottom_right.x() - region.top_left.x()) as u16;
        let height = (region.bottom_right.y() - region.top_left.y()) as u16;
        (x, y, width, height)
    } else {
        (0, 0, screen.width_in_pixels, screen.height_in_pixels)
    };

    // Get the image from the root window.
    // TODO: Consider compositing the cursor into the screenshot in the future.
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            x,
            y,
            width,
            height,
            !0, // plane_mask: all planes
        )
        .map_err(|e| format!("Failed to request screenshot: {e}"))?
        .reply()
        .map_err(|e| format!("Failed to get screenshot reply: {e}"))?;

    // Convert the X11 image data to an image::RgbImage.
    // X11 typically returns BGRA or BGR depending on depth.
    let depth = image.depth;
    let data = image.data;

    let rgb_data = convert_x11_image_to_rgb(&data, width as usize, height as usize, depth)?;

    let img = image::RgbImage::from_raw(width as u32, height as u32, rgb_data)
        .ok_or("Failed to create image from raw data")?;

    let img = image::DynamicImage::ImageRgb8(img);

    crate::screenshot_utils::process_screenshot(img, params)
}

/// Captures a single window by its X window id without raising it, returning the processed
/// image plus metadata describing the captured pixels.
pub fn take_window(
    conn: &RustConnection,
    root: xproto::Window,
    window: xproto::Window,
    params: ScreenshotParams,
) -> Result<(Screenshot, Option<CapturedWindow>), String> {
    let geometry = windows::geometry(conn, root, window)?;

    // Determine the capture rectangle in window-local coordinates.
    let (x, y, width, height) = if let Some(region) = params.region {
        region.validate()?;
        if region.bottom_right.x() > i32::from(geometry.width)
            || region.bottom_right.y() > i32::from(geometry.height)
        {
            return Err(format!(
                "Screenshot region ({}, {}) to ({}, {}) is outside window {window} dimensions {}x{}.",
                region.top_left.x(),
                region.top_left.y(),
                region.bottom_right.x(),
                region.bottom_right.y(),
                geometry.width,
                geometry.height,
            ));
        }
        (
            region.top_left.x() as i16,
            region.top_left.y() as i16,
            (region.bottom_right.x() - region.top_left.x()) as u16,
            (region.bottom_right.y() - region.top_left.y()) as u16,
        )
    } else {
        (0, 0, geometry.width, geometry.height)
    };

    // Bound the native capture before issuing GetImage; see MAX_WINDOW_CAPTURE_PIXELS.
    if usize::from(width) * usize::from(height) > MAX_WINDOW_CAPTURE_PIXELS {
        return Err(format!(
            "Capturing {width}x{height} pixels of window {window} exceeds the \
             {MAX_WINDOW_CAPTURE_PIXELS}-pixel capture limit. Capture a smaller region of the \
             window instead."
        ));
    }

    let image = capture_window_image(conn, window, geometry.border_width, x, y, width, height)?;
    let rgb_data =
        convert_x11_image_to_rgb(&image.data, width as usize, height as usize, image.depth)?;
    let img = image::RgbImage::from_raw(u32::from(width), u32::from(height), rgb_data)
        .ok_or("Failed to create image from raw data")?;
    let img = image::DynamicImage::ImageRgb8(img);
    let screenshot = crate::screenshot_utils::process_screenshot(img, params)?;

    // The captured metadata refers to the native (pre-downscale) capture, so window-local pixel
    // coordinates sent by the agent map directly onto the captured window image.
    let captured = CapturedWindow {
        window_id: window,
        width_px: screenshot.original_width as i32,
        height_px: screenshot.original_height as i32,
    };
    Ok((screenshot, Some(captured)))
}

/// Fetches a rectangle of the window's image, preferring a Composite-extension capture (which
/// sees the full window contents even where other windows overlap it) and falling back to
/// reading the window drawable directly (whose overlapped regions are undefined).
fn capture_window_image(
    conn: &RustConnection,
    window: xproto::Window,
    border_width: u16,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
) -> Result<xproto::GetImageReply, String> {
    match capture_via_composite(conn, window, border_width, x, y, width, height) {
        Ok(image) => Ok(image),
        Err(composite_error) => {
            log::debug!(
                "Composite capture of window {window} failed ({composite_error}); falling back \
                 to direct capture."
            );
            conn.get_image(ImageFormat::Z_PIXMAP, window, x, y, width, height, !0)
                .map_err(|e| format!("Failed to request window screenshot: {e}"))?
                .reply()
                .map_err(|e| format!("Failed to capture window {window}: {e}"))
        }
    }
}

/// Captures a rectangle of the window via the Composite extension's off-screen backing pixmap.
fn capture_via_composite(
    conn: &RustConnection,
    window: xproto::Window,
    border_width: u16,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
) -> Result<xproto::GetImageReply, String> {
    conn.composite_query_version(0, 4)
        .map_err(|e| format!("Composite extension not available: {e}"))?
        .reply()
        .map_err(|e| format!("Composite extension not available: {e}"))?;

    // Redirect the window so the server maintains its full contents off-screen. Automatic
    // redirections are per-client and released when this connection closes, so the error from a
    // redundant redirect (or from a compositor's existing manual redirection, under which
    // NameWindowPixmap already works) is ignored. Content that was covered before redirection
    // may be stale until the application repaints, which its interactions quickly trigger.
    if let Ok(cookie) = conn.composite_redirect_window(window, Redirect::AUTOMATIC) {
        let _ = cookie.check();
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
            x + border_width as i16,
            y + border_width as i16,
            width,
            height,
            !0, // plane_mask: all planes
        )
        .map_err(|e| format!("Failed to request window screenshot: {e}"))
        .and_then(|cookie| {
            cookie
                .reply()
                .map_err(|e| format!("Failed to capture window {window}: {e}"))
        });
    let _ = conn.free_pixmap(pixmap);
    let _ = conn.flush();
    image
}

/// Converts X11 image data (typically BGRA or BGR) to RGB.
fn convert_x11_image_to_rgb(
    data: &[u8],
    width: usize,
    height: usize,
    depth: u8,
) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity(width * height * 3);

    match depth {
        24 => {
            // 24-bit: BGR format, 3 bytes per pixel (but often padded to 4).
            // X11 often uses 32-bit alignment even for 24-bit depth.
            let bytes_per_pixel = if data.len() >= width * height * 4 {
                4
            } else {
                3
            };

            for y in 0..height {
                for x in 0..width {
                    let offset = (y * width + x) * bytes_per_pixel;
                    if offset + 2 < data.len() {
                        let b = data[offset];
                        let g = data[offset + 1];
                        let r = data[offset + 2];
                        rgb.push(r);
                        rgb.push(g);
                        rgb.push(b);
                    }
                }
            }
        }
        32 => {
            // 32-bit: BGRA format, 4 bytes per pixel.
            for y in 0..height {
                for x in 0..width {
                    let offset = (y * width + x) * 4;
                    if offset + 2 < data.len() {
                        let b = data[offset];
                        let g = data[offset + 1];
                        let r = data[offset + 2];
                        // Skip alpha at offset + 3.
                        rgb.push(r);
                        rgb.push(g);
                        rgb.push(b);
                    }
                }
            }
        }
        _ => {
            return Err(format!("Unsupported screen depth: {depth}"));
        }
    }

    Ok(rgb)
}
