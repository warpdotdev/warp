//! macOS-gated unit tests for the avfoundation recorder's ffmpeg argv.
//!
//! These inspect the ffmpeg command built by [`super::new_ffmpeg_capture_command`]
//! without spawning ffmpeg or opening a display, so they run anywhere a macOS
//! build runs. The live start/stop capture tests live in the crate-level
//! `recording_tests.rs` and require a Mac runner with a display.

use super::new_ffmpeg_capture_command;
use crate::{RecordingConfig, Target};

/// Builds the ffmpeg argv (after the program name) for a 1920x1080 capture.
///
/// Inspecting the command's args (rather than spawning it) keeps the test
/// hermetic: no display, no ffmpeg process, no temp files.
fn argv(config: &RecordingConfig) -> Vec<String> {
    let command = new_ffmpeg_capture_command(config, 1920, 1080);
    command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn applies_setpts_filter_when_playback_speed_exceeds_one() {
    let config = RecordingConfig {
        playback_speed_multiplier: 4.0,
        ..RecordingConfig::default()
    };
    let args = argv(&config);

    // 1.0 / 4.0 = 0.25, formatted to six decimals — matches Linux's setpts format.
    let setpts = args
        .iter()
        .find(|arg| arg.starts_with("setpts="))
        .expect("argv should contain a setpts filter when multiplier > 1.0");
    assert_eq!(setpts, "setpts=0.250000*PTS");

    // The filter is passed via the `-vf` output video-filter option.
    assert!(
        args.iter().any(|arg| arg == "-vf"),
        "argv should pass setpts via -vf, got {args:?}"
    );
}

#[test]
fn applies_setpts_filter_for_the_default_1_5x_speed() {
    // The universal default is 1.5x, sent by the server on every
    // StartRecording call and mirrored by the client-side fallback default.
    let config = RecordingConfig::default();
    assert_eq!(config.playback_speed_multiplier, 1.5);
    let args = argv(&config);

    // 1.0 / 1.5 = 0.666667, formatted to six decimals.
    let setpts = args
        .iter()
        .find(|arg| arg.starts_with("setpts="))
        .expect("argv should contain a setpts filter at the default 1.5x speed");
    assert_eq!(setpts, "setpts=0.666667*PTS");
}

#[test]
fn omits_setpts_filter_for_non_finite_playback_speed() {
    // NaN or +Infinity must never reach the ffmpeg argv as a corrupt or
    // zero-duration `setpts` filter; the command builder sanitizes them to
    // real-time instead.
    for multiplier in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let config = RecordingConfig {
            playback_speed_multiplier: multiplier,
            ..RecordingConfig::default()
        };
        let args = argv(&config);

        assert!(
            !args.iter().any(|arg| arg.starts_with("setpts=")),
            "argv should omit setpts for non-finite multiplier {multiplier}, got {args:?}"
        );
    }
}

#[test]
fn clamps_absurdly_large_playback_speed_to_the_maximum() {
    let config = RecordingConfig {
        playback_speed_multiplier: f32::MAX,
        ..RecordingConfig::default()
    };
    let args = argv(&config);

    let setpts = args
        .iter()
        .find(|arg| arg.starts_with("setpts="))
        .expect("argv should still contain a setpts filter, clamped to the maximum");
    let expected = format!(
        "setpts={:.6}*PTS",
        1.0 / crate::MAX_PLAYBACK_SPEED_MULTIPLIER
    );
    assert_eq!(setpts, &expected);
}

#[test]
fn omits_setpts_filter_when_playback_speed_is_real_time() {
    for multiplier in [0.0_f32, 1.0] {
        let config = RecordingConfig {
            playback_speed_multiplier: multiplier,
            ..RecordingConfig::default()
        };
        let args = argv(&config);

        assert!(
            !args.iter().any(|arg| arg.starts_with("setpts=")),
            "argv should omit setpts at multiplier {multiplier}, got {args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg == "-vf"),
            "argv should omit -vf at multiplier {multiplier}, got {args:?}"
        );
    }
}

#[test]
fn limits_duration_as_an_input_option_before_i() {
    // `-t` must precede `-i` so `max_duration` is an input option independent of
    // the setpts speedup (mirrors Linux). As an output option, a 4x multiplier
    // would stretch the effective duration to ~40 min.
    let config = RecordingConfig::default();
    let args = argv(&config);

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

    // The duration value follows `-t` and matches `max_duration` formatted to
    // three decimals, exactly as the recorder emits it.
    let expected_duration = format!("{:.3}", config.max_duration.as_secs_f64());
    assert_eq!(
        args.get(t_index + 1),
        Some(&expected_duration),
        "duration after -t should be {expected_duration}, got {args:?}"
    );
}

#[test]
fn ignores_window_target_until_window_scoped_recording_lands() {
    // Window-scoped recording is deferred (see the TODO in `Recorder::start`);
    // a `Window` target must not alter the argv and must still record the whole
    // main display via avfoundation.
    let window_config = RecordingConfig {
        target: Target::Window {
            window_id: 42,
            pid: 1234,
        },
        ..RecordingConfig::default()
    };
    let window_args = argv(&window_config);
    let screen_args = argv(&RecordingConfig::default());

    assert_eq!(
        window_args, screen_args,
        "a Window target must not alter the ffmpeg argv until window-scoped recording is implemented"
    );
}
