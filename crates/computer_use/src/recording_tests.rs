use super::*;

#[test]
fn observes_synthetic_recording_exit() {
    let (mut handle, exit_state) = RecordingHandle::new_test(1, 1);
    assert_eq!(handle.poll_exit(), None);

    *exit_state.lock().unwrap() = Some(RecordingExitKind::LimitReached);

    assert_eq!(handle.poll_exit(), Some(RecordingExitKind::LimitReached));
}

#[cfg(linux)]
#[test]
fn removes_unclaimed_output_when_handle_is_dropped() {
    let path =
        std::env::temp_dir().join(format!("warp-recording-drop-test-{}", std::process::id()));
    std::fs::write(&path, b"video").unwrap();
    let handle = RecordingHandle {
        width: 1,
        height: 1,
        exit_state: Arc::new(Mutex::new(None)),
        path: path.clone(),
        started_at: instant::Instant::now(),
        process: None,
        cleanup_on_drop: true,
    };

    drop(handle);

    assert!(!path.exists());
}

#[cfg(macos)]
#[test]
fn removes_unclaimed_output_when_handle_is_dropped_macos() {
    // Mirrors the Linux drop test: the macOS `Drop` impl (widened to
    // `any(linux, macos)`) must remove a handle's partial output when it is
    // abandoned without `Recorder::stop`.
    let path =
        std::env::temp_dir().join(format!("warp-recording-drop-test-{}", std::process::id()));
    std::fs::write(&path, b"video").unwrap();
    let handle = RecordingHandle {
        width: 1,
        height: 1,
        exit_state: Arc::new(Mutex::new(None)),
        path: path.clone(),
        started_at: instant::Instant::now(),
        process: None,
        cleanup_on_drop: true,
    };

    drop(handle);

    assert!(!path.exists());
}

#[cfg(macos)]
#[tokio::test]
async fn start_reports_unsupported_when_ffmpeg_absent() {
    // The stable macOS sidecar ships without ffmpeg, so `Recorder::start` must
    // fail fast with `RecordingError::Environment` naming ffmpeg rather than
    // silently producing nothing. Force ffmpeg to be unresolvable for this spawn
    // by pointing PATH at an empty directory; `main_display_dimensions` uses
    // CoreGraphics (not PATH), so it is unaffected. Mutates the process env, so
    // run under `cargo nextest` (process-per-test) or in isolation.
    std::env::remove_var("WARP_MOCK_RECORDER");
    let saved_path = std::env::var_os("PATH");
    std::env::set_var("PATH", "/var/empty");

    let result = create_recorder().start(RecordingConfig::default()).await;

    if let Some(path) = saved_path {
        std::env::set_var("PATH", path);
    }

    let err = result.expect_err("expected RecordingError::Environment when ffmpeg is not on PATH");
    match err {
        RecordingError::Environment { reason } => {
            assert!(
                reason.contains("ffmpeg"),
                "expected the error to name ffmpeg, got: {reason}"
            );
        }
        other => panic!("expected RecordingError::Environment, got {other:?}"),
    }
}
