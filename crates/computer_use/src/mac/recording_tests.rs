//! macOS-gated unit tests for the avfoundation recorder's ffmpeg argv.
//!
//! These inspect the ffmpeg command built by [`super::new_ffmpeg_capture_command`]
//! without spawning ffmpeg or opening a display, so they run anywhere a macOS
//! build runs. The live start/stop capture tests live in the crate-level
//! `recording_tests.rs` and require a Mac runner with a display.

use super::{CapturePlan, diagnose_start_failure, new_ffmpeg_capture_command};
use crate::RecordingConfig;
use crate::recording::window_crop::CaptureCrop;

const DISPLAY_FRAME: (u32, u32) = (1920, 1080);

/// Builds the ffmpeg argv (after the program name) for a 1920x1080 capture.
///
/// Inspecting the command's args (rather than spawning it) keeps the test
/// hermetic: no display, no ffmpeg process, no temp files.
fn argv(config: &RecordingConfig, plan: &CapturePlan) -> Vec<String> {
    let command = new_ffmpeg_capture_command(config, plan);
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn screen_plan() -> CapturePlan {
    CapturePlan {
        input: DISPLAY_FRAME,
        crop: None,
    }
}

fn window_plan(crop: CaptureCrop) -> CapturePlan {
    CapturePlan {
        input: DISPLAY_FRAME,
        crop: Some(crop),
    }
}

/// The macOS master is captured at 1x: there is no live `setpts` speed filter and
/// no speed-only `-vf`, while `-t` stays an input option before `-i` and the
/// codec, pixel format, `-fs`, and movflags settings are preserved.
#[test]
fn mac_capture_command_captures_at_1x_without_setpts() {
    let config = RecordingConfig {
        playback_speed_multiplier: 4.0,
        ..RecordingConfig::default()
    };
    let args = argv(&config, &screen_plan());

    assert!(
        !args.iter().any(|arg| arg.starts_with("setpts=")),
        "argv should not contain a live setpts speed filter, got {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg == "-vf"),
        "a screen capture needs no output filter at all, got {args:?}"
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

    for [flag, value] in [
        ["-c:v", "libx264"],
        ["-pix_fmt", "yuv420p"],
        ["-movflags", "+faststart"],
    ] {
        let index = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("argv should contain {flag}, got {args:?}"));
        assert_eq!(args.get(index + 1), Some(&value.to_string()));
    }
}

/// avfoundation must not composite its own cursor or click flashes: the shared
/// burn-in draws a synthetic cursor and click rings from the recorded pointer
/// events, and background PID-targeted actions never move the real cursor.
#[test]
fn capture_command_disables_native_cursor_compositing() {
    let config = RecordingConfig::default();
    let crop = CaptureCrop {
        x: 100,
        y: 50,
        width: 800,
        height: 600,
    };
    for plan in [screen_plan(), window_plan(crop)] {
        let args = argv(&config, &plan);
        let i_index = args
            .iter()
            .position(|arg| arg == "-i")
            .expect("argv should contain -i");
        for flag in ["-capture_cursor", "-capture_mouse_clicks"] {
            let index = args
                .iter()
                .position(|arg| arg == flag)
                .unwrap_or_else(|| panic!("argv should contain {flag}, got {args:?}"));
            assert_eq!(args.get(index + 1), Some(&"0".to_string()), "{args:?}");
            assert!(
                index < i_index,
                "{flag} must be an avfoundation input option (before -i), got {args:?}"
            );
        }
    }
}

/// A window target crops the display capture to the target's rectangle and
/// reports the cropped dimensions, while a screen target is uncropped.
#[test]
fn window_target_crops_the_display_capture() {
    let config = RecordingConfig::default();
    let crop = CaptureCrop {
        x: 200,
        y: 100,
        width: 800,
        height: 600,
    };
    let plan = window_plan(crop);
    assert_eq!(plan.encoded_dimensions(), (800, 600));

    let args = argv(&config, &plan);
    let vf_index = args
        .iter()
        .position(|arg| arg == "-vf")
        .expect("a window capture should pass a crop filter");
    assert_eq!(
        args.get(vf_index + 1),
        Some(&"crop=800:600:200:100".to_string()),
        "{args:?}"
    );
    // The input still covers the whole display; only the encoded frame is narrowed.
    let video_size_index = args
        .iter()
        .position(|arg| arg == "-video_size")
        .expect("argv should contain -video_size");
    assert_eq!(
        args.get(video_size_index + 1),
        Some(&"1920x1080".to_string())
    );

    let screen = screen_plan();
    assert_eq!(screen.encoded_dimensions(), DISPLAY_FRAME);
    assert!(
        !argv(&config, &screen).iter().any(|arg| arg == "-vf"),
        "a screen capture should not be cropped"
    );
}

/// A permission-class start failure is annotated with the setting the user has
/// to change; anything else is left to ffmpeg's own message.
#[test]
fn start_diagnosis_flags_a_denied_screen_recording_grant() {
    assert!(
        diagnose_start_failure("[AVFoundation indev] Operation not permitted")
            .is_some_and(|hint| hint.contains("Screen Recording"))
    );
    assert_eq!(
        diagnose_start_failure("[AVFoundation indev] Selected framerate is not supported"),
        None
    );
}
