use std::io::Read as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;
use tokio::process::{Child, Command};

use super::*;
use crate::{
    Action, ActionLogEntry, Actor as _, MouseButton, Options, PointerEventKind, PointerSession,
    PointerSink, Recorder as _, RecordingGeometry, ScrollDirection, ScrollDistance, Target,
    TargetedAction, Vector2I,
};
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;

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

fn recording_process(mode: &str, path: &Path, stdin: Stdio) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["recording_process_helper", "--ignored", "--test-threads=1"])
        .env("WARP_RECORDING_PROCESS_MODE", mode)
        .env("WARP_RECORDING_PROCESS_PATH", path)
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
        capture_origin: Vector2I::new(0, 0),
        exit_state: Arc::new(Mutex::new(None)),
        path,
        started_at: Instant::now(),
        process: Some(process),
        cleanup_on_drop: true,
    }
}
async fn decoded_frame(path: &Path, seek_from_end: bool) -> Vec<u8> {
    let mut command = Command::new("ffmpeg");
    command.args(["-v", "error"]);
    if seek_from_end {
        command.args(["-sseof", "-0.5"]);
    }
    let output = command
        .arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

async fn validate_recording(path: &Path, width: u32, height: u32) {
    let expected_frame_bytes = width as usize * height as usize * 3;
    let first_frame = decoded_frame(path, false).await;
    let last_frame = decoded_frame(path, true).await;
    assert_eq!(first_frame.len(), expected_frame_bytes);
    assert_eq!(last_frame.len(), expected_frame_bytes);
    assert_ne!(first_frame, last_frame);
    let decode = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .await
        .unwrap();
    assert!(
        decode.status.success(),
        "{}",
        String::from_utf8_lossy(&decode.stderr)
    );
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
            "0",
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

#[test]
fn maps_virtual_screen_points_into_even_recording_frame() {
    let geometry = normalize_virtual_screen_geometry(-1921, -1079, 3839, 2159).unwrap();
    let recording_geometry = RecordingGeometry::new(
        Vector2I::new(geometry.origin_x, geometry.origin_y),
        geometry.width,
        geometry.height,
    );

    assert_eq!(
        recording_geometry.frame_point(Vector2I::new(-1921, -1079)),
        Vector2I::new(0, 0)
    );
    assert_eq!(
        recording_geometry.frame_point(Vector2I::new(-900, 21)),
        Vector2I::new(1021, 1100)
    );
    assert_eq!(
        recording_geometry.frame_point(Vector2I::new(-5000, -5000)),
        Vector2I::new(0, 0)
    );
    assert_eq!(
        recording_geometry.frame_point(Vector2I::new(5000, 5000)),
        Vector2I::new(3837, 2157)
    );
}

#[test]
fn records_pointer_events_using_capture_start_geometry() {
    let captured = RecordingGeometry::new(Vector2I::new(-1921, -1079), 3838, 2158);
    let current = RecordingGeometry::new(Vector2I::new(0, 0), 1920, 1080);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = PointerSink {
        started_at: Instant::now(),
        recording_target: Target::Screen,
        recording_geometry: captured,
        events: events.clone(),
        session: PointerSession::new(),
    };

    super::super::record_positioned_event(
        Some(&sink),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(-1900, -1000),
    );
    super::super::record_positioned_event(
        Some(&sink),
        PointerEventKind::Move,
        None,
        Vector2I::new(5000, 5000),
    );
    super::super::record_up(Some(&sink), MouseButton::Left);

    let events = events.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].kind, PointerEventKind::Down);
    assert_eq!(events[0].point, Vector2I::new(21, 79));
    assert_ne!(
        events[0].point,
        current.frame_point(Vector2I::new(-1900, -1000))
    );
    assert_eq!(events[1].kind, PointerEventKind::Move);
    assert_eq!(events[1].point, Vector2I::new(3837, 2157));
    assert_eq!(events[2].kind, PointerEventKind::Up);
    assert_eq!(events[2].point, Vector2I::new(3837, 2157));
}

#[tokio::test]
async fn readiness_waits_for_output_growth() {
    let path = temp_path("delayed-output", "mp4");
    let mut process = recording_process("delayed-output", &path, Stdio::null());

    wait_for_first_output(&path, &mut process, Duration::from_secs(2))
        .await
        .unwrap();

    kill_and_reap(&mut process).await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn readiness_reports_early_exit_and_timeout() {
    let early_path = temp_path("early-exit", "mp4");
    let mut early = recording_process("exit-23", &early_path, Stdio::null());
    let error = wait_for_first_output(&early_path, &mut early, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(error.contains("status"));
    let _ = early.wait().await;

    let timeout_path = temp_path("timeout", "mp4");
    let mut stalled = recording_process("stalled", &timeout_path, Stdio::null());
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
    let mut process = recording_process("read-stdin", &marker, Stdio::piped());

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
    let mut early = recording_process("exit-0", &early_path, Stdio::piped());
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
    let mut no_stdin = recording_process("stalled", &no_stdin_path, Stdio::null());
    let error = finalize_capture(&mut no_stdin, &no_stdin_path, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(matches!(error, RecordingError::Finalize { .. }));
    assert!(!no_stdin_path.exists());

    let timeout_path = temp_path("finalize-timeout", "mp4");
    std::fs::write(&timeout_path, b"video").unwrap();
    let mut stalled = recording_process("read-stdin-stall", &timeout_path, Stdio::piped());
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
    let mut empty_process = recording_process("exit-0", &empty_path, Stdio::piped());
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
    let mut invalid_process = recording_process("exit-0", &invalid_path, Stdio::piped());
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
#[ignore = "requires ffmpeg/gdigrab and an interactive Windows desktop"]
async fn records_real_virtual_desktop_when_requested() {
    let output_dir = std::env::var("WARP_RECORDING_TEST_OUTPUT_DIR")
        .expect("set WARP_RECORDING_TEST_OUTPUT_DIR to run the live recording test");
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
    let started_at = Instant::now();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let group_offset = started_at.elapsed();
    let mut actor = super::super::Actor::new();
    let width = i32::try_from(geometry.width).unwrap();
    let height = i32::try_from(geometry.height).unwrap();
    let first = Vector2I::new(
        geometry.origin_x + width / 4,
        geometry.origin_y + height / 4,
    );
    let second = Vector2I::new(
        geometry.origin_x + width * 3 / 4,
        geometry.origin_y + height * 3 / 4,
    );
    let click = Vector2I::new(
        geometry.origin_x + width / 2,
        geometry.origin_y + height / 4,
    );
    let actions = vec![
        TargetedAction::screen(Action::MouseMove { to: first }),
        TargetedAction::screen(Action::MouseDown {
            button: MouseButton::Left,
            at: first,
        }),
        TargetedAction::screen(Action::Wait(Duration::from_millis(500))),
        TargetedAction::screen(Action::MouseMove { to: second }),
        TargetedAction::screen(Action::Wait(Duration::from_millis(500))),
        TargetedAction::screen(Action::MouseUp {
            button: MouseButton::Left,
        }),
        TargetedAction::screen(Action::MouseDown {
            button: MouseButton::Left,
            at: click,
        }),
        TargetedAction::screen(Action::Wait(Duration::from_millis(250))),
        TargetedAction::screen(Action::MouseUp {
            button: MouseButton::Left,
        }),
        TargetedAction::screen(Action::MouseWheel {
            at: second,
            direction: ScrollDirection::Down,
            distance: ScrollDistance::Clicks(1),
        }),
        TargetedAction::screen(Action::TypeText {
            text: "overlay verification".to_string(),
        }),
    ];
    let events = Arc::new(Mutex::new(Vec::new()));
    actor
        .perform_actions(
            &actions,
            Options {
                screenshot_params: None,
                background_enabled: false,
                pointer_sink: Some(PointerSink {
                    started_at,
                    recording_target: Target::Screen,
                    recording_geometry: handle.geometry(),
                    events: events.clone(),
                    session: PointerSession::new(),
                }),
            },
        )
        .await
        .unwrap();
    let finish_offset = started_at.elapsed();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let output = recorder.stop(handle).await.unwrap();
    assert_eq!(
        (output.width, output.height),
        (geometry.width, geometry.height)
    );
    validate_recording(&output.path, output.width, output.height).await;
    let pointer_events = std::mem::take(&mut *events.lock().unwrap());
    let entries = [ActionLogEntry {
        offset: group_offset,
        finish_offset,
        labels: crate::overlay_labels_for(&actions, "Windows overlay verification"),
        pointer_events,
    }];
    let overlay_path = crate::recording_post_process::post_process_recording(
        &output.path,
        &entries,
        (output.width, output.height),
        output.duration,
        15,
    )
    .await
    .unwrap();
    validate_recording(&overlay_path, output.width, output.height).await;
    std::fs::create_dir_all(&output_dir).unwrap();
    let raw_artifact = Path::new(&output_dir).join("windows_raw_recording.mp4");
    let overlay_artifact = Path::new(&output_dir).join("windows_overlay_recording.mp4");
    std::fs::copy(&output.path, &raw_artifact).unwrap();
    std::fs::copy(&overlay_path, &overlay_artifact).unwrap();
    eprintln!(
        "origin=({}, {}) dimensions={}x{} duration={:?} size={} completion={:?} raw={} overlay={}",
        geometry.origin_x,
        geometry.origin_y,
        output.width,
        output.height,
        output.duration,
        output.size_bytes,
        output.completion_status,
        raw_artifact.display(),
        overlay_artifact.display()
    );
    let log_path = output.path.with_extension("log");
    std::fs::remove_file(&output.path).unwrap();
    std::fs::remove_file(&overlay_path).unwrap();
    assert!(!output.path.exists());
    assert!(!overlay_path.exists());
    assert!(!log_path.exists());
}
#[test]
fn dropping_non_cooperative_live_handle_is_non_blocking_and_cleans_after_exit() {
    let path = temp_path("drop-live", "mp4");
    let log_path = path.with_extension("log");
    let lock_ready_path = path.with_extension("lock-ready");
    std::fs::write(&path, b"video").unwrap();
    std::fs::write(&log_path, b"log").unwrap();
    let process = recording_process("stalled", &path, Stdio::null());
    let mut lock_process = recording_process("lock-output", &path, Stdio::null());
    let lock_deadline = Instant::now() + Duration::from_secs(5);
    while !lock_ready_path.exists() && Instant::now() < lock_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(lock_ready_path.exists());

    let started = Instant::now();
    drop(handle_for(process, path.clone()));

    assert!(started.elapsed() < Duration::from_millis(500));
    let deadline = Instant::now() + Duration::from_secs(5);
    while (path.exists() || log_path.exists()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!path.exists());
    assert!(!log_path.exists());
    let lock_deadline = Instant::now() + Duration::from_secs(5);
    while lock_process.try_wait().unwrap().is_none() && Instant::now() < lock_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(lock_process.try_wait().unwrap().is_some());
    std::fs::remove_file(lock_ready_path).unwrap();
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

#[test]
fn identifies_desktop_session_access_denial() {
    let reason = capture_start_failure_reason(
        "ffmpeg exited early with status exit code: 1",
        "Failed to capture image (error 5)\nOutput file does not contain any stream",
    );

    assert!(reason.contains("denied access to the current Windows desktop session"));
    assert!(reason.contains("error 5"));
}

#[test]
#[ignore]
fn recording_process_helper() {
    let mode = std::env::var("WARP_RECORDING_PROCESS_MODE").unwrap();
    let path = PathBuf::from(std::env::var_os("WARP_RECORDING_PROCESS_PATH").unwrap());

    match mode.as_str() {
        "delayed-output" => {
            std::thread::sleep(Duration::from_millis(150));
            std::fs::write(path, [1]).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "exit-23" => std::process::exit(23),
        "exit-0" => {}
        "stalled" => std::thread::sleep(Duration::from_secs(30)),
        "lock-output" => {
            let _file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(&path)
                .unwrap();
            std::fs::write(path.with_extension("lock-ready"), []).unwrap();
            std::thread::sleep(Duration::from_millis(500));
        }
        "read-stdin" => {
            let mut bytes = Vec::new();
            std::io::stdin().read_to_end(&mut bytes).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        "read-stdin-stall" => {
            std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => panic!("unknown recording process mode: {mode}"),
    }
}
