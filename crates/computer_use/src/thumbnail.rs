//! PR video thumbnails.
//!
//! Extracts a representative, downscaled frame from a finalized recording with
//! ffmpeg, composites a centered play-button glyph onto it, and encodes the
//! result as a PNG. The glyph is burned into the image bytes because GitHub
//! sanitizes PR HTML and ignores CSS, so a play affordance cannot be overlaid
//! via markup — it must live in the pixels.
//!
//! Everything here is best-effort: the caller treats any failure as "no
//! thumbnail" and falls back to a plain link, never blocking the video upload
//! or PR creation.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use image::{GrayImage, ImageReader, Luma, Rgba, RgbaImage};

use crate::RecordingError;

/// Maximum width (px) a thumbnail is downscaled to. Keeps the PNG small while
/// staying readable. Passed in by the caller so the upload path can use one
/// constant; ~1280 matches a standard shareable preview width.
pub(crate) const DEFAULT_THUMBNAIL_MAX_WIDTH: u32 = 1280;

/// Supersampling factor for the play-button glyph: the shape coverage mask is
/// rendered at this multiple of the target diameter and Lanczos-downscaled so
/// the disc and triangle edges are anti-aliased.
const PLAY_BUTTON_SUPER: u32 = 4;

/// Generates a PR video thumbnail for `video` and returns the path to the
/// written PNG (a sibling of `video` named `{artifact_uid}-thumb.png`). The
/// caller owns cleanup of both the video and the thumbnail.
///
/// The frame is chosen by ffmpeg's `thumbnail` filter (the frame whose
/// histogram is closest to the average — deterministic for a given input) and
/// downscaled to at most `max_width` pixels wide, preserving aspect ratio. A
/// semi-transparent dark disc with an opaque white triangle is then composited
/// at the center so a reader can tell at a glance that it is a video.
pub async fn generate_video_thumbnail(
    video: &Path,
    max_width: u32,
    artifact_uid: &str,
) -> Result<PathBuf, RecordingError> {
    let frame_path = extract_representative_frame(video, max_width).await?;
    let result = build_thumbnail_png(&frame_path, video, artifact_uid);
    // The extracted frame is an intermediate; drop it regardless of outcome.
    let _ = std::fs::remove_file(&frame_path);
    result
}

/// Extracts a single representative, downscaled frame from `video` to a sibling
/// `.frame.png` via ffmpeg. Uses the `thumbnail` filter for a representative
/// frame and `scale` to cap the width.
async fn extract_representative_frame(
    video: &Path,
    max_width: u32,
) -> Result<PathBuf, RecordingError> {
    let out_path = video.with_extension("frame.png");
    // `thumbnail` picks the frame closest to the average histogram (one frame
    // out); `scale` caps the width, preserving aspect ratio (`-1` keeps height
    // proportional). The whole chain runs inside `-filter_complex` so the comma
    // in the `scale` expression needs no shell escaping.
    let filter = format!("[0:v]thumbnail,scale='min({max_width},iw)':-1[out]");
    let output = tokio::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-i")
        .arg(video)
        .arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("[out]")
        .arg("-frames:v")
        .arg("1")
        .arg("-update")
        .arg("1")
        .arg(&out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to spawn ffmpeg for thumbnail extraction: {error}"),
        })?;
    if output.status.success() {
        return Ok(out_path);
    }
    let _ = std::fs::remove_file(&out_path);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(RecordingError::Finalize {
        reason: format!("ffmpeg thumbnail extraction failed: {}", stderr.trim_end()),
    })
}

/// Loads the extracted frame, composites the play-button glyph, and writes the
/// final PNG to the sibling `{artifact_uid}-thumb.png`.
fn build_thumbnail_png(
    frame_path: &Path,
    video: &Path,
    artifact_uid: &str,
) -> Result<PathBuf, RecordingError> {
    let frame = ImageReader::open(frame_path)
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to open extracted thumbnail frame: {error}"),
        })?
        .decode()
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to decode extracted thumbnail frame: {error}"),
        })?;
    let mut rgba = frame.to_rgba8();
    composite_play_button(&mut rgba);
    let out_path = thumbnail_path(video, artifact_uid);
    rgba.save(&out_path)
        .map_err(|error| RecordingError::Finalize {
            reason: format!("failed to write thumbnail PNG: {error}"),
        })?;
    Ok(out_path)
}

/// The path the finalized thumbnail PNG is written to: a sibling of `video`
/// whose basename is `{artifact_uid}-thumb.png`. The server links a thumbnail
/// to its video by this filename convention.
fn thumbnail_path(video: &Path, artifact_uid: &str) -> PathBuf {
    let file_name = format!("{artifact_uid}-thumb.png");
    match video.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

/// Composites a centered, semi-transparent play-button glyph (a dark disc with
/// an opaque white triangle) onto `frame` in place.
fn composite_play_button(frame: &mut RgbaImage) {
    let (width, height) = frame.dimensions();
    let overlay = render_play_button_overlay(width, height);
    blend_overlay(frame, &overlay);
}

/// Renders the play-button glyph as an RGBA overlay sized to the frame:
/// transparent everywhere except a centered disc with an inscribed triangle.
///
/// Edges are anti-aliased via supersampling: a binary in-shape coverage mask is
/// rendered at `PLAY_BUTTON_SUPER`× the target diameter and Lanczos-downscaled
/// to produce a smooth alpha mask. Color is then applied at target resolution
/// (white inside the triangle, dark inside the rest of the disc) multiplied by
/// the mask coverage, so the color boundary stays hard while only the alpha
/// edge is soft — avoiding a gray fringe between the triangle and the disc.
fn render_play_button_overlay(width: u32, height: u32) -> RgbaImage {
    let diameter = play_button_diameter(width, height);
    let mask = render_shape_coverage_mask(diameter);
    color_play_button(&mask, diameter)
}

/// The play-button disc diameter, as a fraction of the frame's smaller
/// dimension, with a floor so the glyph stays visible on small frames.
fn play_button_diameter(width: u32, height: u32) -> u32 {
    const FRACTION: f32 = 0.22;
    const MIN_DIAMETER: u32 = 48;
    let smaller = width.min(height) as f32;
    let diameter = (FRACTION * smaller).round();
    diameter.max(MIN_DIAMETER as f32) as u32
}

/// Renders a binary in-shape coverage mask (in-shape = white, out = black) for
/// the disc ∪ triangle at supersampled resolution, then Lanczos-downscales it
/// to `diameter` × `diameter` to get a smooth anti-aliased alpha mask.
fn render_shape_coverage_mask(diameter: u32) -> GrayImage {
    let ss_size = diameter.saturating_mul(PLAY_BUTTON_SUPER).max(1);
    let mut mask = GrayImage::new(ss_size, ss_size);
    let center = ss_size as f32 / 2.0;
    let radius = (diameter as f32 * PLAY_BUTTON_SUPER as f32) / 2.0;
    let triangle = play_button_triangle(center, center, radius);
    for y in 0..ss_size {
        for x in 0..ss_size {
            // Sample at pixel centers so the downscale stays symmetric.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let in_disc = (px - center).powi(2) + (py - center).powi(2) <= radius * radius;
            let in_triangle = point_in_triangle(px, py, triangle[0], triangle[1], triangle[2]);
            if in_disc || in_triangle {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
    }
    image::imageops::resize(
        &mask,
        diameter,
        diameter,
        image::imageops::FilterType::Lanczos3,
    )
}

/// Applies color to the coverage mask at target resolution: white inside the
/// triangle, dark inside the rest of the shape, transparent outside. Alpha is
/// the mask coverage (scaled to the glyph's per-region opacity).
fn color_play_button(mask: &GrayImage, diameter: u32) -> RgbaImage {
    const DISC_ALPHA: u32 = 180;
    const TRIANGLE_ALPHA: u32 = 255;
    let center = diameter as f32 / 2.0;
    let radius = diameter as f32 / 2.0;
    let triangle = play_button_triangle(center, center, radius);
    let mut overlay = RgbaImage::new(diameter, diameter);
    for y in 0..diameter {
        for x in 0..diameter {
            let coverage = mask.get_pixel(x, y).0[0] as u32;
            if coverage == 0 {
                continue;
            }
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if point_in_triangle(px, py, triangle[0], triangle[1], triangle[2]) {
                let alpha = (coverage * TRIANGLE_ALPHA / 255).min(255) as u8;
                overlay.put_pixel(x, y, Rgba([255, 255, 255, alpha]));
            } else {
                let alpha = (coverage * DISC_ALPHA / 255).min(255) as u8;
                overlay.put_pixel(x, y, Rgba([0, 0, 0, alpha]));
            }
        }
    }
    overlay
}

/// The play-button triangle vertices (pointing right) inscribed in a disc of
/// the given `radius` centered at `(cx, cy)`.
fn play_button_triangle(cx: f32, cy: f32, radius: f32) -> [(f32, f32); 3] {
    [
        (cx + radius * 0.55, cy),
        (cx - radius * 0.30, cy - radius * 0.50),
        (cx - radius * 0.30, cy + radius * 0.50),
    ]
}

/// Standard barycentric sign test for whether `(px, py)` is inside the triangle
/// `a`, `b`, `c` (edges included).
fn point_in_triangle(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let sign = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| -> f32 {
        (p.0 - r.0) * (q.1 - r.1) - (q.0 - r.0) * (p.1 - r.1)
    };
    let d1 = sign((px, py), a, b);
    let d2 = sign((px, py), b, c);
    let d3 = sign((px, py), c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Alpha-composites `overlay` (positioned at the center of `frame`) onto
/// `frame` in place. The frame is treated as fully opaque.
fn blend_overlay(frame: &mut RgbaImage, overlay: &RgbaImage) {
    let (frame_w, frame_h) = frame.dimensions();
    let (overlay_w, overlay_h) = overlay.dimensions();
    let origin_x = (frame_w as i64 - overlay_w as i64) / 2;
    let origin_y = (frame_h as i64 - overlay_h as i64) / 2;
    for y in 0..overlay_h {
        for x in 0..overlay_w {
            let dst_x = origin_x + x as i64;
            let dst_y = origin_y + y as i64;
            if !(0..frame_w as i64).contains(&dst_x) || !(0..frame_h as i64).contains(&dst_y) {
                continue;
            }
            let src = overlay.get_pixel(x, y);
            let alpha = u32::from(src.0[3]);
            if alpha == 0 {
                continue;
            }
            let dst = frame.get_pixel_mut(dst_x as u32, dst_y as u32);
            let inv = 255 - alpha;
            for channel in 0..3 {
                dst.0[channel] = ((u32::from(src.0[channel]) * alpha
                    + u32::from(dst.0[channel]) * inv)
                    / 255) as u8;
            }
            // The frame is opaque; keep the destination alpha at 255.
            dst.0[3] = 255;
        }
    }
}

#[cfg(test)]
#[path = "thumbnail_tests.rs"]
mod tests;
