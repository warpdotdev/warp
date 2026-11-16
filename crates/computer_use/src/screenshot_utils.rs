//! Shared utilities for screenshot processing.

use std::io::Cursor;
#[cfg(target_os = "macos")]
use std::path::Path;

use image::{DynamicImage, GenericImageView};
#[cfg(linux)]
use pathfinder_geometry::vector::Vector2I;

use crate::{Screenshot, ScreenshotParams};

/// Loads an image from a file, processes it according to the given parameters, and returns a
/// Screenshot.
#[cfg(target_os = "macos")]
pub fn load_and_process_screenshot(
    path: &Path,
    params: ScreenshotParams,
) -> Result<Screenshot, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| format!("Failed to open screenshot file: {e}"))?
        .decode()
        .map_err(|e| format!("Failed to decode screenshot: {e}"))?;

    process_screenshot(img, params)
}

/// Processes a DynamicImage according to the given parameters and returns a Screenshot.
///
/// This validates dimensions, applies scaling if needed, and encodes the result to PNG.
pub fn process_screenshot(
    img: DynamicImage,
    params: ScreenshotParams,
) -> Result<Screenshot, String> {
    let (original_width, original_height) = img.dimensions();
    if original_width == 0 || original_height == 0 {
        return Err(format!(
            "Screenshot has invalid dimensions (width: {original_width}, height: {original_height})"
        ));
    }

    // Apply scaling if the image is larger than the constraints.
    let scale_factor = get_scale_factor(original_width, original_height, params);
    let img = if scale_factor < 1.0 {
        let new_width = (original_width as f64 * scale_factor).max(1.0).round() as u32;
        let new_height = (original_height as f64 * scale_factor).max(1.0).round() as u32;
        img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let (width, height) = img.dimensions();

    // Encode to PNG.
    let mut data = Vec::new();
    let mut writer = Cursor::new(&mut data);
    img.write_to(&mut writer, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode screenshot to PNG: {e}"))?;

    Ok(Screenshot {
        width: width as usize,
        height: height as usize,
        original_width: original_width as usize,
        original_height: original_height as usize,
        data,
        mime_type: "image/png".into(),
    })
}

/// Crops a `DynamicImage` to the specified region.
///
/// The coordinates are in pixels, with (0, 0) at the top-left of the image.
#[cfg(linux)]
pub fn crop_to_region(
    img: DynamicImage,
    top_left: Vector2I,
    bottom_right: Vector2I,
) -> DynamicImage {
    let x = top_left.x() as u32;
    let y = top_left.y() as u32;
    let width = (bottom_right.x() - top_left.x()) as u32;
    let height = (bottom_right.y() - top_left.y()) as u32;
    img.crop_imm(x, y, width, height)
}

/// Returns the scaling factor to apply to a screenshot to meet the size constraints.
///
/// The scale factor is chosen to ensure that:
/// 1. The longer edge is at most `max_long_edge_px` pixels (if specified)
/// 2. The total number of pixels is at most `max_total_px` (if specified)
/// 3. The scale factor is at most 1.0 (no upscaling)
///
/// This must stay in sync with the server-side logic in logic/ai/computer_use/utils.go.
pub fn get_scale_factor(width: u32, height: u32, params: ScreenshotParams) -> f64 {
    let long_edge = width.max(height);
    let total_pixels = width * height;

    let long_edge_scale = params
        .max_long_edge_px
        .map(|max| max as f64 / long_edge as f64)
        .unwrap_or(1.0);
    let total_pixels_scale = params
        .max_total_px
        .map(|max| (max as f64 / total_pixels as f64).sqrt())
        .unwrap_or(1.0);

    long_edge_scale.min(total_pixels_scale).min(1.0)
}
