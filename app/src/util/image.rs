//! Shared image processing utilities for agent mode.
//!
//! This module provides common functionality for processing images before they are
//! sent to the AI agent, whether attached by the user or read via the read_files tool.

use std::path::Path;

use image::{GenericImageView, ImageError};
use mime_guess::from_path;

/// Max image size is 3.75 MB.
/// The max size of an image we will send is 5MB. However, due to the 33% inflation of Base64, this means
/// the largest size a user can attach is actually ~3.75MB.
pub const MAX_IMAGE_SIZE_BYTES: usize = 3750 * 1000;

/// Cap on individual image bytes when relaying a dropped/pasted image to a
/// CLI agent (Claude Code etc.) via the system clipboard. CLI agents handle
/// their own compression, so this limit only exists to keep us from loading
/// arbitrarily large files into memory.
pub const MAX_IMAGE_SIZE_BYTES_FOR_CLI_AGENT: usize = 500 * 1_000_000;

/// How many leading bytes of a file are enough for `infer_mime_type` to
/// match a magic-number signature. Callers that already have the full bytes
/// in memory should pass only the first `MIME_SNIFF_BYTES` to avoid handing
/// arbitrarily large slices to the sniffer.
pub const MIME_SNIFF_BYTES: usize = 8 * 1024;

/// Returns the MIME type for `path`, preferring magic-byte detection from
/// `file_bytes` and falling back to the path's extension when the magic
/// bytes don't yield a confident match. Falls all the way back to
/// `application/octet-stream`. `file_bytes` only needs to contain the first
/// `MIME_SNIFF_BYTES` of the file for the magic-number check.
pub fn infer_mime_type(path: &Path, file_bytes: &[u8]) -> String {
    infer::get(file_bytes)
        .map(|kind| kind.mime_type().to_string())
        .unwrap_or_else(|| from_path(path).first_or_octet_stream().to_string())
}

/// 1.15 Megapixels
pub const MAX_IMAGE_PIXELS: f64 = 1150. * 1000.;

/// Maximum dimension (width or height) for images.
pub const MAX_IMAGE_DIMENSION: f64 = 2000.;

/// Maximum number of images that can be attached per query/task.
pub const MAX_IMAGE_COUNT_FOR_QUERY: usize = 20;

/// Minimum bytes needed for image format detection using magic number signatures.
pub const MIN_IMAGE_HEADER_SIZE: usize = 8;

/// Supported image MIME types for agent mode.
pub const SUPPORTED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/gif",
    "image/webp",
];

/// Checks if the given MIME type is a supported image type.
pub fn is_supported_image_mime_type(mime_type: &str) -> bool {
    SUPPORTED_IMAGE_MIME_TYPES.contains(&mime_type)
}

/// Resizes an image if it exceeds the maximum pixel count, and ensures
/// resized outputs also respect the maximum dimension (width or height).
///
/// Returns the original image bytes if the image is already within the
/// pixel limit; otherwise returns the resized image bytes in the original
/// format.
pub fn resize_image(image: &[u8]) -> Result<Vec<u8>, ImageError> {
    let img = image::load_from_memory(image)?;

    let (current_width, current_height) = img.dimensions();
    let current_pixels = (current_width * current_height) as f64;

    if current_pixels <= MAX_IMAGE_PIXELS {
        return Ok(image.to_vec());
    }

    let original_format = image::guess_format(image)?;

    let scale = (MAX_IMAGE_PIXELS / current_pixels).sqrt();

    let mut new_width = current_width as f64 * scale;
    let mut new_height = current_height as f64 * scale;

    let scale_by_width = MAX_IMAGE_DIMENSION / new_width;
    let scale_by_height = MAX_IMAGE_DIMENSION / new_height;
    let scale = scale_by_width.min(scale_by_height).min(1.0);

    new_width *= scale;
    new_height *= scale;

    let resized_img = img.thumbnail(new_width.round() as u32, new_height.round() as u32);

    let mut output_bytes: Vec<u8> = Vec::new();
    let mut writer = std::io::Cursor::new(&mut output_bytes);

    resized_img.write_to(&mut writer, original_format)?;

    Ok(output_bytes)
}

/// Result of processing an image for agent mode.
#[derive(Debug)]
pub enum ProcessImageResult {
    /// Image was successfully processed and is within size limits.
    Success {
        /// The processed image bytes (resized if needed).
        data: Vec<u8>,
    },
    /// Image is too large even after resizing.
    TooLarge,
    /// Error processing the image.
    Error(ImageError),
}

/// Processes an image for agent mode: resizes if needed and checks size limits.
///
/// This applies the same processing that user-attached images go through.
pub fn process_image_for_agent(image_data: &[u8]) -> ProcessImageResult {
    match resize_image(image_data) {
        Ok(resized_bytes) => {
            if resized_bytes.len() > MAX_IMAGE_SIZE_BYTES {
                ProcessImageResult::TooLarge
            } else {
                ProcessImageResult::Success {
                    data: resized_bytes,
                }
            }
        }
        Err(err) => ProcessImageResult::Error(err),
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
