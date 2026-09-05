//! X11/Xvfb-gated tests for the Linux recorder.
//!
//! These exercise the real ffmpeg-backed recorder against the live X display (`$DISPLAY`,
//! typically `:99` under Xvfb in CI). If no display or ffmpeg is available the tests skip rather
//! than fail, so they are a no-op in environments that can't run them.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, Rectangle, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::Recorder;
// The `Recorder` trait provides `start`/`stop` on the concrete `super::Recorder` struct.
use crate::{Recorder as _, RecordingConfig, Target};

// 24-bit TrueColor pixel values (0xRRGGBB) for the two solid-color test windows.
const RED_PIXEL: u32 = 0x00FF_0000;
const BLUE_PIXEL: u32 = 0x0000_00FF;

/// Returns whether the environment can run the ffmpeg + X11 recorder tests.
async fn recorder_env_available() -> bool {
    if std::env::var("DISPLAY").is_err() {
        return false;
    }
    let ffmpeg_ok = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ffmpeg_ok {
        return false;
    }
    RustConnection::connect(None).is_ok()
}

/// Creates a mapped, solid-color, borderless top-level window and paints it, returning its id.
fn create_solid_window(
    conn: &RustConnection,
    screen: &xproto::Screen,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    color: u32,
) -> xproto::Window {
    let window = conn.generate_id().expect("generate window id");
    conn.create_window(
        screen.root_depth,
        window,
        screen.root,
        x,
        y,
        width,
        height,
        0, // border_width
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(color)
            .event_mask(EventMask::EXPOSURE),
    )
    .expect("create window")
    .check()
    .expect("create window check");
    conn.map_window(window).expect("map window");
    conn.flush().expect("flush");
    window
}

/// Paints `window` a solid `color` via a graphics-context fill.
fn paint_window(
    conn: &RustConnection,
    window: xproto::Window,
    width: u16,
    height: u16,
    color: u32,
) {
    let gc = conn.generate_id().expect("generate gc id");
    conn.create_gc(gc, window, &CreateGCAux::new().foreground(color))
        .expect("create gc");
    conn.poly_fill_rectangle(
        window,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width,
            height,
        }],
    )
    .expect("fill rectangle");
    let _ = conn.free_gc(gc);
    let _ = conn.flush();
}

/// Decodes the recorded video to raw RGB and returns its final full frame (`width * height * 3`
/// bytes). Using the last frame avoids any race with the first frames captured before the test
/// finished painting the window.
async fn decode_last_frame_rgb(path: &Path, width: u32, height: u32) -> Vec<u8> {
    let raw_path = path.with_extension("raw");
    let output = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
        .arg(&raw_path)
        .output()
        .await
        .expect("run ffmpeg decode");
    assert!(
        output.status.success(),
        "ffmpeg decode failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data = std::fs::read(&raw_path).expect("read decoded rawvideo");
    let _ = std::fs::remove_file(&raw_path);
    let frame_len = (width as usize) * (height as usize) * 3;
    assert!(
        data.len() >= frame_len,
        "decoded output ({} bytes) smaller than one {width}x{height} frame ({frame_len} bytes)",
        data.len(),
    );
    // Return the last complete frame.
    let start = (data.len() / frame_len - 1) * frame_len;
    data[start..start + frame_len].to_vec()
}

/// Parses the encoded video's pixel dimensions from `ffmpeg -i` stderr (no ffprobe available).
async fn probe_dimensions(path: &Path) -> (u32, u32) {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .output()
        .await
        .expect("run ffmpeg probe");
    // `ffmpeg -i` with no output file "fails" (exit code 1) but still prints stream info.
    let stderr = String::from_utf8_lossy(&output.stderr);
    for token in stderr.split([' ', ',', '\n']) {
        if let Some((w, h)) = token.split_once('x')
            && let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
            && w > 0
            && h > 0
        {
            return (w, h);
        }
    }
    panic!("could not parse dimensions from ffmpeg output:\n{stderr}");
}

/// Records a window that starts fully covered, asserting recording start raises it and native
/// x11grab captures the target window's dimensions and pixels.
#[tokio::test]
async fn records_window_target_via_native_x11grab_after_raise() {
    if !recorder_env_available().await {
        eprintln!(
            "skipping records_window_target_via_native_x11grab_after_raise: no X11/ffmpeg \
             environment"
        );
        return;
    }

    let (conn, screen_index) = RustConnection::connect(None).expect("connect X11");
    let screen = conn.setup().roots[screen_index].clone();

    // A red target window, initially fully covered by a blue window stacked on top of it.
    let width: u16 = 200;
    let height: u16 = 200;
    let target = create_solid_window(&conn, &screen, 100, 100, width, height, RED_PIXEL);
    let cover = create_solid_window(&conn, &screen, 100, 100, width, height, BLUE_PIXEL);
    // The later-mapped `cover` window is already on top; paint both to be safe.
    paint_window(&conn, target, width, height, RED_PIXEL);
    paint_window(&conn, cover, width, height, BLUE_PIXEL);
    conn.flush().expect("flush");

    let recorder = Recorder::new();
    let config = RecordingConfig {
        frame_rate: 15,
        target: Target::Window {
            window_id: target,
            pid: 0,
        },
        ..RecordingConfig::default()
    };
    let handle = recorder
        .start(config)
        .await
        .expect("start window recording");
    let out_width = handle.width();
    let out_height = handle.height();

    // Once recording starts, the target should have been raised. Repaint it while capture is
    // live so the final frames are unambiguously target-red.
    for _ in 0..12 {
        paint_window(&conn, target, width, height, RED_PIXEL);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let output = recorder.stop(handle).await.expect("stop window recording");
    assert_eq!(output.width, u32::from(width));
    assert_eq!(output.height, u32::from(height));
    let (probed_width, probed_height) = probe_dimensions(&output.path).await;
    assert_eq!(
        (probed_width, probed_height),
        (u32::from(width), u32::from(height)),
        "encoded window recording should match the target window size"
    );

    let frame = decode_last_frame_rgb(&output.path, out_width, out_height).await;
    // Sample the center of the window. If the cover were recorded instead, this would be blue.
    let (px, py) = (out_width / 2, out_height / 2);
    let offset = ((py * out_width + px) * 3) as usize;
    let (r, g, b) = (frame[offset], frame[offset + 1], frame[offset + 2]);
    assert!(
        r > g && r > b && r > 100,
        "recorded pixel at ({px},{py}) should be the target's red, got rgb=({r},{g},{b}) \
         (blue cover would give a dominant blue channel)"
    );

    // Optionally preserve the recording as a visual-evidence artifact when requested.
    if let Ok(dir) = std::env::var("WARP_RECORDING_TEST_OUTPUT_DIR") {
        let dest = Path::new(&dir).join("window_target_recording.mp4");
        let _ = std::fs::copy(&output.path, &dest);
    }

    // Cleanup.
    let _ = std::fs::remove_file(&output.path);
    let _ = conn.destroy_window(cover);
    let _ = conn.destroy_window(target);
    let _ = conn.flush();
}

#[test]
fn visibility_samples_stay_inside_window() {
    let geometry = super::windows::WindowGeometry {
        x: 10,
        y: 20,
        width: 100,
        height: 50,
        border_width: 0,
    };
    let samples = super::visibility_sample_points(geometry);

    assert_eq!(samples.len(), 5);
    for sample in samples {
        assert!(sample.x() >= 10 && sample.x() < 110);
        assert!(sample.y() >= 20 && sample.y() < 70);
    }
}

/// Records with a `Screen` target and asserts the encoded video is the full (even-rounded)
/// display size — i.e. the fallback path is unchanged.
#[tokio::test]
async fn records_full_display_for_screen_target() {
    if !recorder_env_available().await {
        eprintln!("skipping records_full_display_for_screen_target: no X11/ffmpeg environment");
        return;
    }

    let (conn, screen_index) = RustConnection::connect(None).expect("connect X11");
    let screen = &conn.setup().roots[screen_index];
    let expected_width = u32::from(screen.width_in_pixels) & !1;
    let expected_height = u32::from(screen.height_in_pixels) & !1;

    let recorder = Recorder::new();
    let config = RecordingConfig {
        frame_rate: 15,
        target: Target::Screen,
        ..RecordingConfig::default()
    };
    let handle = recorder
        .start(config)
        .await
        .expect("start screen recording");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let output = recorder.stop(handle).await.expect("stop screen recording");

    assert_eq!(output.width, expected_width);
    assert_eq!(output.height, expected_height);
    let (probed_width, probed_height) = probe_dimensions(&output.path).await;
    assert_eq!(
        (probed_width, probed_height),
        (expected_width, expected_height),
        "encoded screen recording should match the full display size"
    );

    let _ = std::fs::remove_file(&output.path);
}

/// Builds the ffmpeg argv (after the program name) for a 1920x1080 screen capture.
fn capture_argv(config: &RecordingConfig) -> Vec<String> {
    let command = super::new_ffmpeg_capture_command(config, ":99", 1920, 1080, None);
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// The Linux master is captured at 1x: the live `setpts` speed filter is gone,
/// `-t` stays an input option before `-i`, and `-fs` stays present with the
/// configured byte limit.
#[test]
fn linux_capture_command_captures_at_1x_without_setpts() {
    let config = RecordingConfig {
        playback_speed_multiplier: 4.0,
        ..RecordingConfig::default()
    };
    let args = capture_argv(&config);

    assert!(
        !args.iter().any(|arg| arg.starts_with("setpts=")),
        "argv should not contain a live setpts speed filter, got {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg == "-vf"),
        "argv should not contain -vf, got {args:?}"
    );

    let t_index = args
        .iter()
        .position(|arg| arg == "-t")
        .expect("argv should contain -t");
    let i_index = args
        .iter()
        .position(|arg| arg == "-i")
        .expect("argv should contain -i");
    assert!(
        t_index < i_index,
        "-t should precede -i (input option), got {args:?}"
    );
    assert_eq!(
        args.get(t_index + 1),
        Some(&format!("{:.3}", config.max_duration.as_secs_f64())),
        "duration after -t should match max_duration, got {args:?}"
    );

    let fs_index = args
        .iter()
        .position(|arg| arg == "-fs")
        .expect("argv should contain -fs");
    assert_eq!(
        args.get(fs_index + 1),
        Some(&config.max_size_bytes.to_string()),
        "-fs value should match max_size_bytes, got {args:?}"
    );
}

/// The capture composites no X11 cursor: XFixes only reports the user's core
/// pointer (never the background agent seat's), so the post-stop burn-in
/// synthesizes the cursor from recorded pointer events instead — identically
/// for screen and window scopes.
#[test]
fn capture_command_disables_cursor_compositing_for_screen_and_window() {
    let config = RecordingConfig::default();
    for window in [None, Some(0x0060_0011)] {
        let command = super::new_ffmpeg_capture_command(&config, ":99", 1920, 1080, window);
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let index = args
            .iter()
            .position(|arg| arg == "-draw_mouse")
            .expect("argv should contain -draw_mouse");
        assert_eq!(args.get(index + 1), Some(&"0".to_string()), "{args:?}");
        let i_index = args
            .iter()
            .position(|arg| arg == "-i")
            .expect("argv should contain -i");
        assert!(
            index < i_index,
            "-draw_mouse must be an x11grab input option (before -i), got {args:?}"
        );
    }
}
