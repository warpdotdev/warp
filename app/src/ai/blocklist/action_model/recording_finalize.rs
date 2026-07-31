use std::future::Future;
use std::path::Path;
use std::time::Duration;

use ai::agent::action_result::{RecordingStopped, StopRecordingResult};
use futures::channel::oneshot;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

pub(crate) use super::recording_controller::FinalizeReason;
use super::recording_controller::{
    ActiveRecording, FinalizationClaim, FinalizedRecording, RecordingController,
    StopRecordingControllerError,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::artifact_upload::{FileArtifactUploadRequest, FileArtifactUploader};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A handle to the canonical result owned by `RecordingController`.
///
/// `Pending` subscribes to work already owned by the controller; dropping the
/// receiver does not cancel stop or upload. `Ready` exposes the retained result
/// after that work has completed. Both variants carry the *actual*
/// [`FinalizeReason`] that drove the work — callers that only joined an
/// in-progress finalization still learn why it ran, rather than the reason
/// they claimed when joining.
pub(crate) enum RecordingFinalization {
    Pending(oneshot::Receiver<FinalizedRecording>),
    Ready(FinalizedRecording),
}

impl RecordingFinalization {
    pub(crate) async fn resolve(self) -> FinalizedRecording {
        match self {
            RecordingFinalization::Pending(receiver) => receiver.await.unwrap_or_else(|_| {
                (
                    StopRecordingResult::Error(
                        "Recording finalization ended without producing a result.".to_string(),
                    ),
                    // The result channel closed before delivering a result, so the
                    // real trigger is unknown and distinct from an ffmpeg crash.
                    FinalizeReason::FinalizationDropped,
                )
            }),
            RecordingFinalization::Ready(ready) => ready,
        }
    }
}

fn format_upload_error(err: &anyhow::Error) -> String {
    let error_chain = format!("{err:#}");
    if error_chain != err.to_string() {
        format!("Recording upload failed: {error_chain}")
    } else {
        error_chain
    }
}

/// Best-effort PR video thumbnail upload.
///
/// Generates a thumbnail PNG from the finalized `video_path` (a representative,
/// downscaled frame with a play-button glyph burned in), then uploads it as a
/// separate `FILE` artifact. The server links the thumbnail to its video by the
/// `{video_artifact_uid}-thumb.png` filename convention rather than a schema
/// back-reference. The local thumbnail file is removed whether the upload
/// succeeds or fails.
///
/// Any error is returned to the caller, which logs and discards it so the video
/// upload and PR creation are never blocked by a missing or failed thumbnail.
async fn upload_recording_thumbnail(
    video_path: &Path,
    video_artifact_uid: &str,
    uploader: &FileArtifactUploader,
    conversation_id: Option<ServerConversationToken>,
) -> anyhow::Result<()> {
    let thumbnail_path =
        computer_use::generate_video_thumbnail(video_path, video_artifact_uid).await?;
    let upload_result = async {
        let request = FileArtifactUploadRequest {
            path: thumbnail_path.clone(),
            run_id: None,
            conversation_id,
            title: None,
            description: None,
        };
        let association = uploader.resolve_upload_association(&request).await?;
        uploader.upload_with_association(request, association).await
    }
    .await;
    // The thumbnail file is ephemeral regardless of upload outcome.
    let _ = std::fs::remove_file(&thumbnail_path);
    upload_result?;
    Ok(())
}

/// Stops capture, uploads the finalized file, and produces the result retained
/// by the controller for all current and future callers.
async fn finalize_recording(
    recording: ActiveRecording,
    reason: FinalizeReason,
    should_upload: bool,
    uploader: FileArtifactUploader,
    server_conversation_token: Option<crate::ai::agent::api::ServerConversationToken>,
) -> StopRecordingResult {
    // A no-upload finalization discards the recording without publishing: it
    // drops the whole `ActiveRecording` (kill-on-drops ffmpeg, removes the
    // partial capture). The reason distinguishes an agent-requested discard
    // (`Discarded`) from a conversation cancellation (`Cancelled`), which the
    // agent turn treats differently. This check comes first so it holds even
    // when no action group was committed.
    if !should_upload {
        drop(recording);
        return match reason {
            FinalizeReason::StoppedByAgent => StopRecordingResult::Discarded,
            _ => StopRecordingResult::Cancelled,
        };
    }
    if recording.actions.is_empty() {
        drop(recording);
        return StopRecordingResult::Error(
            "Recording contained no committed actions; no video artifact was published."
                .to_string(),
        );
    }
    let ActiveRecording {
        handle,
        actions,
        frame_rate,
        ..
    } = recording;
    let recorder = computer_use::create_recorder();
    let output = match recorder.stop(handle).await {
        Ok(output) => output,
        Err(error) => return StopRecordingResult::Error(error.to_string()),
    };

    let local_path = output.path.clone();

    // Apply the post-stop smart cut (keep real action windows at 1x, drop
    // blocked/thinking gaps) and burn the remapped overlay pills into the video
    // before upload. Best-effort: on any failure the original 1x capture is
    // uploaded unannotated (a no-cut video beats no video). The cut/overlay
    // file, when produced, is a sibling of the mp4.
    let mut upload_path = local_path.clone();
    let mut overlay_path: Option<std::path::PathBuf> = None;
    match computer_use::post_process_recording(
        &local_path,
        &actions,
        (output.width, output.height),
        output.duration,
        frame_rate,
    )
    .await
    {
        Ok(path) if path != local_path => {
            overlay_path = Some(path.clone());
            upload_path = path;
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!("Recording cut/overlay burn-in failed; uploading original: {error}");
        }
    }
    let duration = match computer_use::finalized_video_duration(&upload_path).await {
        Ok(duration) => duration,
        Err(error) => {
            log::warn!(
                "Failed to inspect finalized recording duration; using capture duration: {error}"
            );
            output.duration
        }
    };

    // Keep a handle on the finalized video path for thumbnail extraction before
    // it is moved into the upload request; the thumbnail reads this file with
    // ffmpeg after the video upload resolves.
    let thumbnail_source_path = upload_path.clone();
    let request = FileArtifactUploadRequest {
        path: upload_path,
        run_id: None,
        conversation_id: server_conversation_token.clone(),
        title: recording.summary.clone(),
        description: recording.description.clone(),
    };
    let upload_result = async {
        let association = uploader.resolve_upload_association(&request).await?;
        uploader.upload_with_association(request, association).await
    }
    .await;
    // Best-effort PR video thumbnail: after a successful non-discard video
    // upload, extract a representative frame, composite a play-button glyph, and
    // upload it as a separate PNG file artifact linked to the video by the
    // `{video_uuid}-thumb.png` filename convention. Capture is unconditional; the
    // team setting and feature flag are consulted only at server render time. A
    // missing/failed thumbnail must never block the video upload or PR creation,
    // so any error is logged and dropped.
    if let Ok(upload) = &upload_result
        && let Err(error) = upload_recording_thumbnail(
            &thumbnail_source_path,
            &upload.artifact.artifact_uid,
            &uploader,
            server_conversation_token.clone(),
        )
        .await
    {
        log::warn!("PR video thumbnail capture failed; video upload unaffected: {error}");
    }
    // Local files are ephemeral regardless of upload outcome. Retrying failed
    // uploads or retaining their files requires a separate persistence policy.
    let _ = std::fs::remove_file(&local_path);
    let _ = std::fs::remove_file(local_path.with_extension("log"));
    if let Some(overlay_path) = overlay_path.as_ref() {
        let _ = std::fs::remove_file(overlay_path);
    }

    match upload_result {
        Ok(upload) => StopRecordingResult::Success(RecordingStopped {
            artifact_uid: upload.artifact.artifact_uid,
            duration,
            width_px: output.width as i32,
            height_px: output.height as i32,
            size_bytes: upload.size_bytes,
            completion_status: output.completion_status,
            termination_reason: reason.termination_reason(output.completion_status),
        }),
        Err(error) => StopRecordingResult::Error(format_upload_error(&error)),
    }
}

/// Captures the upload association and clients while the app models are still
/// available, before stop/upload work moves onto the controller-owned task.
fn build_finalize_future(
    recording: ActiveRecording,
    reason: FinalizeReason,
    should_upload: bool,
    ctx: &AppContext,
) -> (
    String,
    impl Future<Output = StopRecordingResult> + Send + 'static + use<>,
) {
    let server_conversation_token = BlocklistAIHistoryModel::as_ref(ctx)
        .conversation(&recording.conversation_id)
        .and_then(|conversation| conversation.server_conversation_token())
        .cloned();
    let uploader = FileArtifactUploader::new(
        ServerApiProvider::as_ref(ctx).get_ai_client(),
        ServerApiProvider::as_ref(ctx).get(),
    );
    let id = recording.id.clone();
    (
        id,
        finalize_recording(
            recording,
            reason,
            should_upload,
            uploader,
            server_conversation_token,
        ),
    )
}

/// Runs finalization independently of any action future and stores its result
/// on the controller before waking subscribers. The `reason` is forwarded to
/// [`RecordingController::complete_finalization`] so waiters that only joined
/// this work receive the actual reason it ran, not the reason they claimed.
fn spawn_finalize(
    recording: ActiveRecording,
    reason: FinalizeReason,
    should_upload: bool,
    ctx: &mut ModelContext<RecordingController>,
) {
    let (recording_id, future) = build_finalize_future(recording, reason, should_upload, ctx);
    ctx.spawn(future, move |controller, result, _ctx| {
        controller.complete_finalization(&recording_id, result, reason);
    });
}

/// Converts an atomic controller claim into a result handle. Only the caller
/// that receives `Claimed` starts work; concurrent and later callers subscribe
/// to the in-flight operation or receive its retained result.
fn start_or_join_finalization<T: Entity>(
    claim: FinalizationClaim,
    reason: FinalizeReason,
    should_upload: bool,
    ctx: &mut ModelContext<T>,
) -> Option<RecordingFinalization> {
    match claim {
        FinalizationClaim::Claimed {
            recording,
            result_receiver,
        } => {
            RecordingController::handle(ctx).update(ctx, |_controller, ctx| {
                spawn_finalize(*recording, reason, should_upload, ctx);
            });
            Some(RecordingFinalization::Pending(result_receiver))
        }
        FinalizationClaim::InProgress(receiver) => Some(RecordingFinalization::Pending(receiver)),
        FinalizationClaim::Finished(result) => Some(RecordingFinalization::Ready(result)),
        FinalizationClaim::NotFound => None,
    }
}

/// Starts or joins finalization for an explicit `StopRecording` request.
///
/// The returned handle only observes controller-owned work. The stop executor
/// decides when a retained result has been delivered and can be consumed.
pub(crate) fn finalize_recording_by_id<T: Entity>(
    recording_id: &str,
    reason: FinalizeReason,
    should_persist: bool,
    ctx: &mut ModelContext<T>,
) -> Result<RecordingFinalization, StopRecordingControllerError> {
    let claim = RecordingController::handle(ctx).update(ctx, |controller, _| {
        controller.claim_finalization_by_id(recording_id)
    });
    start_or_join_finalization(claim, reason, should_persist, ctx).ok_or_else(|| {
        StopRecordingControllerError::RecordingNotFound {
            recording_id: recording_id.to_string(),
        }
    })
}
/// Starts or joins finalization for this conversation.
///
/// Finalization itself is spawned on the recording controller, so dropping the
/// returned handle does not cancel stop/upload work. The driver awaits the
/// handle before teardown; conversation cancellation only observes it for
/// logging because cancellation must remain synchronous.
pub(crate) fn finalize_recording_for_conversation<T: Entity>(
    conversation_id: AIConversationId,
    reason: FinalizeReason,
    should_upload: bool,
    ctx: &mut ModelContext<T>,
) -> Option<RecordingFinalization> {
    // The recording controller is always registered in production
    // (`app/src/lib.rs`). Guard here so the conversation-cancellation and
    // driver-teardown paths never panic in test harnesses that don't register
    // the singleton — there is simply nothing to finalize in that case.
    if !ctx.has_singleton_model::<RecordingController>() {
        return None;
    }
    let claim = RecordingController::handle(ctx).update(ctx, |controller, _| {
        controller.claim_finalization_for_conversation(conversation_id)
    })?;
    start_or_join_finalization(claim, reason, should_upload, ctx)
}

/// Polls the active ffmpeg process until it exits or another path claims it.
///
/// Each timer schedules the next one only while this recording remains active.
/// Stop, cancellation, and driver teardown move it to `Finalizing`, at which
/// point this watcher observes that it is no longer active and ends.
pub(crate) fn spawn_recording_exit_watcher(
    recording_id: String,
    ctx: &mut ModelContext<RecordingController>,
) {
    ctx.spawn(
        async move {
            Timer::after(EXIT_POLL_INTERVAL).await;
        },
        move |controller, (), ctx| match controller.poll_active_exit(&recording_id) {
            Some(exit_kind) => {
                if let FinalizationClaim::Claimed { recording, .. } =
                    controller.claim_finalization_by_id(&recording_id)
                {
                    let reason = match exit_kind {
                        computer_use::RecordingExitKind::LimitReached => {
                            FinalizeReason::LimitReached
                        }
                        computer_use::RecordingExitKind::Crashed => FinalizeReason::FfmpegExited,
                    };
                    spawn_finalize(*recording, reason, true, ctx);
                }
            }
            None if controller.active_recording_id() == Some(recording_id.as_str()) => {
                spawn_recording_exit_watcher(recording_id, ctx);
            }
            None => {}
        },
    );
}

#[cfg(test)]
#[path = "recording_finalize_tests.rs"]
mod tests;
