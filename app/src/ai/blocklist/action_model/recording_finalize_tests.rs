use ai::agent::action_result::StopRecordingResult;
use computer_use::RecordingHandle;
use instant::Instant;
use warpui::{App, SingletonEntity};

use super::super::recording_controller::ActiveRecording;
use super::*;
use crate::test_util::terminal::initialize_app_for_terminal_view;

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
        let recording = ActiveRecording {
            id: "recording".to_string(),
            conversation_id: AIConversationId::new(),
            handle,
            started_at: Instant::now(),
            frame_rate: 15,
            target: computer_use::Target::Screen,
            actions: Vec::new(),
            summary: None,
            description: None,
            pending_group: None,
        };

        let result =
            finalize_recording(recording, FinalizeReason::Cancelled, false, uploader, None).await;

        assert_eq!(result, StopRecordingResult::Cancelled);
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
        let recording = ActiveRecording {
            id: "recording".to_string(),
            conversation_id: AIConversationId::new(),
            handle,
            started_at: Instant::now(),
            frame_rate: 15,
            target: computer_use::Target::Screen,
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
