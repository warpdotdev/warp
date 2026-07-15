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
