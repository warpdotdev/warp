use ai::agent::action_result::StopRecordingResult;
use computer_use::RecordingHandle;
use instant::Instant;
use warpui::{App, SingletonEntity};

use super::super::recording_controller::ActiveRecording;
use super::*;
use crate::test_util::terminal::initialize_app_for_terminal_view;
#[tokio::test]
async fn invalid_processed_recording_falls_back_to_validated_raw() {
    if !tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .is_ok_and(|output| output.status.success())
    {
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "warp-recording-validation-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let raw_path = root.join("raw.mp4");
    let processed_path = root.join("processed.mp4");
    let generated = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=10:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&raw_path)
        .output()
        .await
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    std::fs::write(&processed_path, b"not a video").unwrap();

    let (upload_path, duration, retained_processed_path) =
        validated_recording_for_upload(&raw_path, Some(processed_path.clone()))
            .await
            .unwrap();

    assert_eq!(upload_path, raw_path);
    assert!(duration > Duration::ZERO);
    assert_eq!(retained_processed_path, None);
    assert!(!processed_path.exists());
    std::fs::remove_dir_all(root).unwrap();
}

/// Conversation cancellation must not upload the recording: it kills ffmpeg (by
/// dropping the handle) and resolves as `Cancelled` without touching the
/// uploader.
#[test]
fn cancellation_finalization_skips_upload_even_without_actions() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let uploader = app.update(|ctx| {
            FileArtifactUploader::new(
                ServerApiProvider::as_ref(ctx).get_ai_client(),
                ServerApiProvider::as_ref(ctx).get(),
            )
        });

        let (handle, _exit_state) = RecordingHandle::new_test(1, 1);
        let geometry = handle.geometry();
        let recording = ActiveRecording {
            id: "recording".to_string(),
            conversation_id: AIConversationId::new(),
            handle,
            started_at: Instant::now(),
            frame_rate: 15,
            target: computer_use::Target::Screen,
            geometry,
            pointer_session: computer_use::PointerSession::new(),
            actions: Vec::new(),
            summary: None,
            description: None,
            pending_group: None,
        };

        let result = finalize_recording(
            recording,
            FinalizeReason::RunCancelled,
            false,
            uploader,
            None,
        )
        .await;

        assert_eq!(result, StopRecordingResult::Cancelled);
    });
}

/// An agent-initiated stop with `should_persist=false` discards the recording:
/// it kills ffmpeg (by dropping the handle) and resolves as `Discarded` without
/// touching the uploader, so the agent's turn continues.
#[test]
fn agent_discard_finalization_skips_upload() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let uploader = app.update(|ctx| {
            FileArtifactUploader::new(
                ServerApiProvider::as_ref(ctx).get_ai_client(),
                ServerApiProvider::as_ref(ctx).get(),
            )
        });

        let (handle, _exit_state) = RecordingHandle::new_test(1, 1);
        let geometry = handle.geometry();
        let recording = ActiveRecording {
            id: "recording".to_string(),
            conversation_id: AIConversationId::new(),
            handle,
            started_at: Instant::now(),
            frame_rate: 15,
            target: computer_use::Target::Screen,
            geometry,
            pointer_session: computer_use::PointerSession::new(),
            actions: Vec::new(),
            summary: None,
            description: None,
            pending_group: None,
        };

        let result = finalize_recording(
            recording,
            FinalizeReason::StoppedByAgent,
            false,
            uploader,
            None,
        )
        .await;

        assert_eq!(result, StopRecordingResult::Discarded);
    });
}

/// A recording with no committed meaningful action group is an explicit
/// finalization error: it is not uploaded, and the handle is dropped (which
/// kill-on-drops ffmpeg and removes the partial capture). This returns before
/// `recorder.stop`, so it needs no live recorder or uploader.
#[test]
fn empty_actions_finalization_is_an_error_without_upload() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let uploader = app.update(|ctx| {
            FileArtifactUploader::new(
                ServerApiProvider::as_ref(ctx).get_ai_client(),
                ServerApiProvider::as_ref(ctx).get(),
            )
        });

        let (handle, _exit_state) = RecordingHandle::new_test(1, 1);
        let geometry = handle.geometry();
        let recording = ActiveRecording {
            id: "recording".to_string(),
            conversation_id: AIConversationId::new(),
            handle,
            started_at: Instant::now(),
            frame_rate: 15,
            target: computer_use::Target::Screen,
            geometry,
            pointer_session: computer_use::PointerSession::new(),
            actions: Vec::new(),
            summary: None,
            description: None,
            pending_group: None,
        };

        let result = finalize_recording(
            recording,
            FinalizeReason::StoppedByAgent,
            true,
            uploader,
            None,
        )
        .await;

        assert!(
            matches!(result, StopRecordingResult::Error(ref msg) if msg.contains("no committed actions")),
            "empty actions should finalize as an error, got {result:?}"
        );
    });
}
