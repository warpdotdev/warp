use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;
use tokio::process::{Child, Command};

use super::*;
use crate::{Action, Actor as _, Options, Recorder as _, Target, TargetedAction, Vector2I};

fn temp_path(name: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "warp-windows-recording-{name}-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ))
}

fn write_batch(name: &str, body: &str) -> PathBuf {
    let path = temp_path(name, "cmd");
    std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
    path
}

fn powershell(script: &str, argument: &Path, stdin: Stdio) -> Child {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .arg(argument)
        .stdin(stdin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.spawn().unwrap()
}

fn handle_for(process: Child, path: PathBuf) -> RecordingHandle {
    RecordingHandle {
        width: 320,
        height: 240,
        exit_state: Arc::new(Mutex::new(None)),
        path,
        started_at: Instant::now(),
        process: Some(process),
        cleanup_on_drop: true,
    }
}

#[test]
fn normalizes_negative_origin_and_odd_dimensions() {
    let geometry = normalize_virtual_screen_geometry(-1921, -1079, 3839, 2159).unwrap();
    assert_eq!(
        geometry,
        VirtualScreenGeometry {
            origin_x: -1921,
            origin_y: -1079,
            width: 3838,
            height: 2158,
        }
    );
}

#[test]
fn rejects_non_positive_or_zero_even_dimensions() {
    for (width, height) in [(0, 1080), (-1, 1080), (1920, 0), (1920, -1), (1, 1)] {
        assert!(matches!(
            normalize_virtual_screen_geometry(0, 0, width, height),
            Err(RecordingError::Environment { .. })
        ));
    }
}

#[test]
fn builds_full_virtual_desktop_capture_command() {
    let config = RecordingConfig {
        frame_rate: 24,
        max_duration: Duration::from_millis(12_345),
        max_size_bytes: 123_456,
        playback_speed_multiplier: 8.0,
        target: Target::Window {
            window_id: 42,
            pid: 7,
        },
    };
    let geometry = normalize_virtual_screen_geometry(-1921, -1079, 3839, 2159).unwrap();
    let command = new_ffmpeg_capture_command(Path::new("ffmpeg"), &config, geometry);
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        args,
        [
            "-y",
            "-f",
            "gdigrab",
            "-framerate",
            "24",
            "-offset_x",
            "-1921",
            "-offset_y",
            "-1079",
            "-video_size",
            "3838x2158",
            "-draw_mouse",
            "1",
            "-t",
            "12.345",
            "-i",
            "desktop",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-fs",
            "123456",
        ]
    );
    assert!(!args.iter().any(|arg| arg.contains("setpts")));
}

#[tokio::test]
async fn probe_accepts_only_successful_gdigrab_help() {
    let usable = write_batch(
        "probe-usable",
        "echo Demuxer gdigrab [GDI API Windows frame grabber]\r\nexit /b 0",
    );
    verify_gdigrab_available(&usable).await.unwrap();

    let missing = write_batch(
        "probe-missing",
        "echo No matching demuxer is available\r\nexit /b 0",
    );
    let error = verify_gdigrab_available(&missing).await.unwrap_err();
    assert!(matches!(error, RecordingError::Environment { .. }));
    assert!(error.to_string().contains("gdigrab"));

    let failed = write_batch("probe-failed", "echo probe failed 1>&2\r\nexit /b 7");
    let error = verify_gdigrab_available(&failed).await.unwrap_err();
    assert!(matches!(error, RecordingError::Environment { .. }));
    assert!(error.to_string().contains("status"));

    for path in [usable, missing, failed] {
        let _ = std::fs::remove_file(path);
    }
}

#[tokio::test]
async fn probe_reports_missing_ffmpeg_as_environment_error() {
    let error = verify_gdigrab_available(&temp_path("missing", "exe"))
        .await
        .unwrap_err();
    assert!(matches!(error, RecordingError::Environment { .. }));
    assert!(error.to_string().contains("ffmpeg"));
}

#[tokio::test]
async fn readiness_waits_for_output_growth() {
    let path = temp_path("delayed-output", "mp4");
    let script = concat!(
        "[Threading.Thread]::Sleep(150);",
        "[IO.File]::WriteAllBytes($args[0], [byte[]](1));",
        "[Threading.Thread]::Sleep(30000)"
    );
    let mut process = powershell(script, &path, Stdio::null());

    wait_for_first_output(&path, &mut process, Duration::from_secs(2))
        .await
        .unwrap();

    kill_and_reap(&mut process).await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn readiness_reports_early_exit_and_timeout() {
    let early_path = temp_path("early-exit", "mp4");
    let mut early = powershell("exit 23", &early_path, Stdio::null());
    let error = wait_for_first_output(&early_path, &mut early, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(error.contains("status"));
    let _ = early.wait().await;

    let timeout_path = temp_path("timeout", "mp4");
    let mut stalled = powershell(
        "[Threading.Thread]::Sleep(30000)",
        &timeout_path,
        Stdio::null(),
    );
    let error = wait_for_first_output(&timeout_path, &mut stalled, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(error.contains("timed out"));
    kill_and_reap(&mut stalled).await;
}

#[tokio::test]
async fn graceful_finalization_writes_q_and_closes_stdin() {
    let path = temp_path("graceful-output", "mp4");
    let marker = temp_path("graceful-stdin", "bin");
    std::fs::write(&path, b"video").unwrap();
    let script = concat!(
        "$bytes=[Text.Encoding]::UTF8.GetBytes([Console]::In.ReadToEnd());",
        "[IO.File]::WriteAllBytes($args[0], $bytes)"
    );
    let mut process = powershell(script, &marker, Stdio::piped());

    let status = finalize_capture(&mut process, &path, Duration::from_secs(2))
        .await
        .unwrap();

    assert_eq!(status, RecordingCompletionStatus::Completed);
    assert_eq!(std::fs::read(&marker).unwrap(), b"q\n");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(marker);
}

#[tokio::test]
async fn finalization_handles_early_exit_missing_stdin_and_timeout() {
    let early_path = temp_path("finalize-early", "mp4");
    std::fs::write(&early_path, b"video").unwrap();
    let mut early = powershell("exit 0", &early_path, Stdio::piped());
    let _ = early.wait().await;
    assert_eq!(
        finalize_capture(&mut early, &early_path, Duration::from_secs(1))
            .await
            .unwrap(),
        RecordingCompletionStatus::StoppedEarly
    );
    let _ = std::fs::remove_file(early_path);

    let no_stdin_path = temp_path("finalize-no-stdin", "mp4");
    std::fs::write(&no_stdin_path, b"video").unwrap();
    let mut no_stdin = powershell(
        "[Threading.Thread]::Sleep(30000)",
        &no_stdin_path,
        Stdio::null(),
    );
    let error = finalize_capture(&mut no_stdin, &no_stdin_path, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(matches!(error, RecordingError::Finalize { .. }));
    assert!(!no_stdin_path.exists());

    let timeout_path = temp_path("finalize-timeout", "mp4");
    std::fs::write(&timeout_path, b"video").unwrap();
    let mut stalled = powershell(
        "$null=[Console]::In.ReadToEnd(); [Threading.Thread]::Sleep(30000)",
        &timeout_path,
        Stdio::piped(),
    );
    let error = finalize_capture(&mut stalled, &timeout_path, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(matches!(error, RecordingError::Finalize { .. }));
    assert!(!timeout_path.exists());
}

#[tokio::test]
async fn stop_rejects_empty_and_invalid_media() {
    let empty_path = temp_path("empty", "mp4");
    std::fs::write(&empty_path, b"").unwrap();
    let mut empty_process = powershell("exit 0", &empty_path, Stdio::piped());
    let _ = empty_process.wait().await;
    let recorder = Recorder::with_ffmpeg(PathBuf::from("ffmpeg"));
    let error = recorder
        .stop(handle_for(empty_process, empty_path.clone()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("empty"));
    assert!(!empty_path.exists());

    let invalid_path = temp_path("invalid", "mp4");
    std::fs::write(&invalid_path, b"not an mp4").unwrap();
    let mut invalid_process = powershell("exit 0", &invalid_path, Stdio::piped());
    let _ = invalid_process.wait().await;
    let invalid_probe = write_batch("invalid-probe", "echo invalid media 1>&2\r\nexit /b 1");
    let recorder = Recorder::with_ffmpeg(invalid_probe.clone());
    let error = recorder
        .stop(handle_for(invalid_process, invalid_path.clone()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("valid finalized video duration"));
    assert!(!invalid_path.exists());
    let _ = std::fs::remove_file(invalid_probe);
}

#[tokio::test]
async fn records_real_virtual_desktop_when_requested() {
    let Ok(output_dir) = std::env::var("WARP_RECORDING_TEST_OUTPUT_DIR") else {
        return;
    };
    let geometry = query_virtual_screen_geometry().unwrap();
    let recorder = Recorder::new();
    let handle = recorder
        .start(RecordingConfig {
            frame_rate: 15,
            max_duration: Duration::from_secs(10),
            max_size_bytes: 100 * 1024 * 1024,
            playback_speed_multiplier: 1.0,
            target: Target::Screen,
        })
        .await
        .unwrap();
    let mut actor = super::super::Actor::new();
    let width = i32::try_from(geometry.width).unwrap();
    let height = i32::try_from(geometry.height).unwrap();
    for point in [
        Vector2I::new(
            geometry.origin_x + width / 4,
            geometry.origin_y + height / 4,
        ),
        Vector2I::new(
            geometry.origin_x + width * 3 / 4,
            geometry.origin_y + height * 3 / 4,
        ),
    ] {
        actor
            .perform_actions(
                &[TargetedAction::screen(Action::MouseMove { to: point })],
                Options {
                    screenshot_params: None,
                    background_enabled: false,
                    pointer_sink: None,
                },
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let output = recorder.stop(handle).await.unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();
    let artifact = Path::new(&output_dir).join("windows_raw_recording.mp4");
    std::fs::copy(&output.path, &artifact).unwrap();
    eprintln!(
        "origin=({}, {}) dimensions={}x{} duration={:?} size={} completion={:?} artifact={}",
        geometry.origin_x,
        geometry.origin_y,
        output.width,
        output.height,
        output.duration,
        output.size_bytes,
        output.completion_status,
        artifact.display()
    );
    let _ = std::fs::remove_file(output.path);
}
#[test]
fn dropping_live_handle_reaps_process_and_removes_partial_files() {
    let path = temp_path("drop-live", "mp4");
    let log_path = path.with_extension("log");
    std::fs::write(&path, b"video").unwrap();
    std::fs::write(&log_path, b"log").unwrap();
    let process = powershell("[Threading.Thread]::Sleep(30000)", &path, Stdio::null());

    drop(handle_for(process, path.clone()));

    assert!(!path.exists());
    assert!(!log_path.exists());
}

#[test]
fn diagnostics_are_bounded_to_three_lines_and_512_characters() {
    let text = format!("ignored\n{}\nsecond\nthird", "x".repeat(700));
    let diagnostic = diagnostic_tail(&text);
    assert!(!diagnostic.contains("ignored"));
    assert!(diagnostic.chars().count() <= 515);
    assert!(diagnostic.contains("second"));
    assert!(diagnostic.contains("third"));
}
