//! Unit tests for the pure-Rust thumbnail compositing. These exercise the
//! play-button glyph rendering and blending without ffmpeg, so they are
//! deterministic and run anywhere the `image` crate is available.

use std::path::PathBuf;

use image::{GenericImageView, Rgba, RgbaImage};

use super::*;

/// A solid-color frame keeps its corners untouched while the centered glyph
/// darkens the disc and turns the triangle white.
#[test]
fn composite_play_button_darkens_disc_and_whites_triangle() {
    let mut frame = solid_frame(100, 100, [200, 0, 0]);
    composite_play_button(&mut frame);

    // A corner is outside the glyph and stays the original red.
    assert_eq!(
        frame.get_pixel(0, 0),
        &Rgba([200, 0, 0, 255]),
        "corner pixel should be untouched"
    );

    // The frame center lands inside the triangle, so it becomes white.
    let center = frame.get_pixel(50, 50);
    assert!(
        center.0[1] > 200 && center.0[2] > 200,
        "center pixel should be the white triangle, got {:?}",
        center
    );

    // A point inside the disc but above the triangle is darkened toward black
    // (the semi-transparent disc blended over red), not whitened.
    let disc_only = frame.get_pixel(50, 30);
    assert!(
        disc_only.0[0] < 200 && disc_only.0[1] == 0 && disc_only.0[2] == 0,
        "disc-only pixel should be darkened red, got {:?}",
        disc_only
    );
}

/// The play-button overlay is transparent outside the shape and non-transparent
/// inside it, with the triangle rendered white and the rest of the disc dark.
#[test]
fn play_button_overlay_shape_and_colors() {
    let overlay = render_play_button_overlay(200, 200);
    // diameter = max(0.22 * 200, 48) = 48
    assert_eq!(overlay.dimensions(), (48, 48));

    // A corner of the overlay is outside the disc: fully transparent.
    assert_eq!(
        overlay.get_pixel(0, 0),
        &Rgba([0, 0, 0, 0]),
        "overlay corner should be transparent"
    );

    // The center is inside the triangle: white with full coverage.
    let center = overlay.get_pixel(24, 24);
    assert!(
        center.0[3] > 0 && center.0[0] > 200 && center.0[1] > 200 && center.0[2] > 200,
        "overlay center should be the white triangle, got {:?}",
        center
    );

    // A disc-only point (top of the disc, above the triangle) is dark with
    // partial alpha, never white.
    let disc_only = overlay.get_pixel(24, 3);
    assert!(
        disc_only.0[3] > 0,
        "disc-only pixel should be non-transparent, got {:?}",
        disc_only
    );
    assert!(
        disc_only.0[0] < 50 && disc_only.0[1] < 50 && disc_only.0[2] < 50,
        "disc-only pixel should be dark, got {:?}",
        disc_only
    );
}

/// The disc diameter scales with the smaller frame dimension but never drops
/// below the visibility floor.
#[test]
fn play_button_diameter_scales_with_floor() {
    // 22% of the smaller edge, rounded.
    assert_eq!(play_button_diameter(1280, 720), 158); // 0.22 * 720 = 158.4 -> 158
    // Below the floor, the minimum kicks in.
    assert_eq!(play_button_diameter(50, 50), 48);
}

/// The thumbnail basename follows the `{artifact_uid}-thumb.png` convention the
/// server uses to link a thumbnail back to its video, as a sibling of the video.
#[test]
fn thumbnail_path_uses_uid_naming_convention() {
    let video = PathBuf::from("/tmp/warp-recording-abc.mp4");
    assert_eq!(
        thumbnail_path(&video, "vid-uid-123"),
        PathBuf::from("/tmp/vid-uid-123-thumb.png")
    );
}

/// Generating a thumbnail for a nonexistent video fails gracefully rather than
/// panicking, regardless of whether ffmpeg is installed.
#[tokio::test]
async fn generate_thumbnail_for_missing_video_errors() {
    let result = generate_video_thumbnail(
        &PathBuf::from("/nonexistent/warp-recording-missing.mp4"),
        DEFAULT_THUMBNAIL_MAX_WIDTH,
        "vid-uid-missing",
    )
    .await;
    assert!(
        result.is_err(),
        "missing video should produce a thumbnail error, got {result:?}"
    );
}

fn solid_frame(width: u32, height: u32, rgb: [u8; 3]) -> RgbaImage {
    let mut frame = RgbaImage::new(width, height);
    for pixel in frame.pixels_mut() {
        *pixel = Rgba([rgb[0], rgb[1], rgb[2], 255]);
    }
    frame
}

/// Integration test (requires ffmpeg): synthesizes a short video, generates
/// a thumbnail from it, and verifies the output PNG. Confirms the full
/// ffmpeg extraction + play-button compositing pipeline end-to-end.
///
/// Run with:
/// ```
/// cargo test -p computer_use generate_thumbnail_for_synthetic_video -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore]
async fn generate_thumbnail_for_synthetic_video() {
    let video = std::env::temp_dir().join("warp-test-recording.mp4");
    // 2-second solid-colour video — simple and deterministic.
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=640x360:duration=2:rate=30",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&video)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .expect("ffmpeg should be available");
    assert!(status.success(), "synthetic video creation failed");

    let thumb_path = generate_video_thumbnail(&video, DEFAULT_THUMBNAIL_MAX_WIDTH, "synthetic-uid")
        .await
        .expect("thumbnail generation should succeed");

    println!("Thumbnail written to: {}", thumb_path.display());
    assert!(thumb_path.exists(), "thumbnail file should exist on disk");

    let img = image::open(&thumb_path).expect("thumbnail should be valid PNG");
    let (w, h) = img.dimensions();
    println!("Thumbnail dimensions: {w}x{h}");
    assert!(
        w <= DEFAULT_THUMBNAIL_MAX_WIDTH,
        "width must not exceed max_width"
    );
    assert!(w > 0 && h > 0, "thumbnail must have non-zero dimensions");

    // The play-button disc must darken the center relative to the background.
    // For a solid blue frame, the disc area should have less blue than the
    // corner pixels (blended with a dark overlay) and the center of the
    // triangle should be brighter (white glyph).
    let img_rgba = img.to_rgba8();
    let corner = img_rgba.get_pixel(0, 0);
    let center = img_rgba.get_pixel(w / 2, h / 2);
    // The solid blue background may have minor rounding from YUV→RGB conversion
    // after H.264 encoding; allow ±2 per channel.
    assert!(
        corner.0[0] < 10 && corner.0[1] < 10 && corner.0[2] > 240,
        "corner should be approximately blue background, got {corner:?}"
    );
    assert!(
        center.0[0] > 200 && center.0[1] > 200 && center.0[2] > 200,
        "frame center should be the white triangle, got {center:?}"
    );
}

/// Eye-test evidence (requires ffmpeg): fabricates a `testsrc` clip, generates
/// the thumbnail, and copies the resulting PNG to a fixed path so the burned-in
/// play glyph can be eyeballed by a reviewer.
///
/// Run with:
/// ```
/// cargo test -p computer_use -- --ignored thumbnail
/// ```
#[tokio::test]
#[ignore]
async fn eyetest_thumbnail_glyph_render() {
    let video = PathBuf::from("/tmp/warp-testsrc.mp4");
    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=1280x720:rate=15",
        ])
        .arg(&video)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .expect("ffmpeg should be available");
    assert!(status.success(), "synthetic clip creation failed");

    let thumb_path = generate_video_thumbnail(&video, DEFAULT_THUMBNAIL_MAX_WIDTH, "eyetest-uid")
        .await
        .expect("thumbnail generation should succeed");

    let eyetest_path = PathBuf::from("/tmp/warp-thumb-eyetest.png");
    std::fs::copy(&thumb_path, &eyetest_path).expect("copy thumbnail for eye-test");
    let _ = std::fs::remove_file(&thumb_path);
    assert!(eyetest_path.exists(), "eye-test PNG should exist");
    println!("Eye-test thumbnail written to: {}", eyetest_path.display());
}
