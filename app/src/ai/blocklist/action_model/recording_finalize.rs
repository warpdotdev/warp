use std::future::Future;
use std::time::Duration;

use ai::agent::action_result::{RecordingStopped, StopRecordingResult};
use futures::channel::oneshot;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::recording_controller::{
    ActiveRecording, FinalizationClaim, RecordingController, StopRecordingControllerError,
};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::artifact_upload::{FileArtifactUploadRequest, FileArtifactUploader};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::server::server_api::ServerApiProvider;

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FinalizeReason {
    StoppedByAgent,
    AgentFinished,
    LimitReached,
    FfmpegExited,
    Cancelled,
}

impl FinalizeReason {
    fn termination_reason(
        self,
        completion_status: computer_use::RecordingCompletionStatus,
    ) -> String {
        match self {
            FinalizeReason::StoppedByAgent => match completion_status {
                computer_use::RecordingCompletionStatus::Completed => {
                    "Stopped by agent".to_string()
                }
                computer_use::RecordingCompletionStatus::StoppedEarly => {
                    "Recording stopped before the agent requested it".to_string()
                }
            },
            FinalizeReason::AgentFinished => {
                "Finalized because the agent finished without stopping the recording".to_string()
            }
            FinalizeReason::LimitReached => {
                "Stopped at the configured duration or size limit".to_string()
            }
            FinalizeReason::FfmpegExited => {
                "Capture process exited before the recording was stopped".to_string()
            }
            FinalizeReason::Cancelled => {
                "Recording was interrupted when the conversation was cancelled".to_string()
            }
        }
    }
}

/// A handle to the canonical result owned by `RecordingController`.
///
/// `Pending` subscribes to work already owned by the controller; dropping the
/// receiver does not cancel stop or upload. `Ready` exposes the retained result
/// after that work has completed.
pub(crate) enum RecordingFinalization {
    Pending(oneshot::Receiver<StopRecordingResult>),
    Ready(StopRecordingResult),
}

impl RecordingFinalization {
    pub(crate) async fn resolve(self) -> StopRecordingResult {
        match self {
            RecordingFinalization::Pending(receiver) => receiver.await.unwrap_or_else(|_| {
                StopRecordingResult::Error(
                    "Recording finalization ended without producing a result.".to_string(),
                )
            }),
            RecordingFinalization::Ready(result) => result,
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

/// Stops capture, uploads the finalized file, and produces the result retained
/// by the controller for all current and future callers.
async fn finalize_recording(
    recording: ActiveRecording,
    reason: FinalizeReason,
    should_upload: bool,
    uploader: FileArtifactUploader,
    server_conversation_token: Option<crate::ai::agent::api::ServerConversationToken>,
) -> StopRecordingResult {
    // Both early exits discard the recording without uploading. They share the
    // same mechanism — dropping the whole `ActiveRecording` struct, which
    // kill-on-drops ffmpeg and removes the partial capture — but return
    // different results to distinguish a user-initiated cancellation from a
    // genuine finalization error. The cancellation check intentionally comes
    // first so its `Cancelled` contract holds even when no action group was
    // committed.
    if !should_upload {
        drop(recording);
        return StopRecordingResult::Cancelled;
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

    let request = FileArtifactUploadRequest {
        path: upload_path,
        run_id: None,
        conversation_id: server_conversation_token,
        title: recording.summary.clone(),
        description: recording.description.clone(),
    };
    let upload_result = async {
        let association = uploader.resolve_upload_association(&request).await?;
        uploader.upload_with_association(request, association).await
    }
    .await;
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
/// on the controller before waking subscribers.
fn spawn_finalize(
    recording: ActiveRecording,
    reason: FinalizeReason,
    should_upload: bool,
    ctx: &mut ModelContext<RecordingController>,
) {
    let (recording_id, future) = build_finalize_future(recording, reason, should_upload, ctx);
    ctx.spawn(future, move |controller, result, _ctx| {
        controller.complete_finalization(&recording_id, result);
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
    ctx: &mut ModelContext<T>,
) -> Result<RecordingFinalization, StopRecordingControllerError> {
    let claim = RecordingController::handle(ctx).update(ctx, |controller, _| {
        controller.claim_finalization_by_id(recording_id)
    });
    start_or_join_finalization(claim, reason, true, ctx).ok_or_else(|| {
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
