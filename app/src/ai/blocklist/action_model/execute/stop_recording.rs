#[cfg(not(target_family = "wasm"))]
use ai::agent::action_result::{RecordingStopped, StopRecordingResult};
use futures::FutureExt;
use futures::future::BoxFuture;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
#[cfg(not(target_family = "wasm"))]
use crate::ai::{
    agent::AIAgentActionResultType,
    blocklist::{
        BlocklistAIHistoryModel,
        action_model::{
            RecordingTelemetryEvent,
            recording_controller::{RecordingController, StopRecordingControllerError},
            recording_finalize::{FinalizeReason, finalize_recording_by_id},
        },
    },
};
#[cfg(not(target_family = "wasm"))]
use crate::send_telemetry_from_ctx;

pub struct StopRecordingExecutor;

impl StopRecordingExecutor {
    pub fn new() -> Self {
        Self
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput { action, .. } = input;
        matches!(action.action, AIAgentActionType::StopRecording { .. })
            && warp_core::features::FeatureFlag::VideoRecording.is_enabled()
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> AnyActionExecution {
        #[cfg(target_family = "wasm")]
        {
            ActionExecution::<()>::InvalidAction.into()
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let ExecuteActionInput {
                action,
                conversation_id,
            } = input;
            let AIAgentActionType::StopRecording {
                recording_id,
                should_persist,
            } = &action.action
            else {
                return ActionExecution::<()>::InvalidAction.into();
            };
            // A persisting stop remains retry-safe while the conversation is
            // syncing: do not claim the handle until it can be associated with
            // the conversation for upload. A discard needs no upload
            // association or server token, so it proceeds regardless of sync
            // state.
            let conversation_is_synced = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_conversation_token())
                .is_some();
            if *should_persist && !conversation_is_synced {
                return ActionExecution::<()>::Sync(AIAgentActionResultType::StopRecording(
                    StopRecordingResult::Error(
                        StopRecordingControllerError::ConversationNotSynced.to_string(),
                    ),
                ))
                .into();
            }

            // Atomically claim an active recording, join an upload another
            // terminal path already started, or read the retained result. The
            // controller owns the actual stop/upload task in every case.
            //
            // `claimed_reason` is the stop action's own view of why
            // finalization ran; it is used only when this call actually starts
            // the work. When the call joins an in-progress finalization
            // started by another path (the exit watcher or conversation
            // cancellation), the resolved `RecordingFinalization` carries the
            // *actual* reason back so telemetry reports the true trigger.
            let claimed_reason = FinalizeReason::StoppedByAgent;
            let finalization = match finalize_recording_by_id(
                recording_id,
                claimed_reason,
                *should_persist,
                ctx,
            ) {
                Ok(finalization) => finalization,
                Err(error) => {
                    return ActionExecution::<()>::Sync(AIAgentActionResultType::StopRecording(
                        StopRecordingResult::Error(error.to_string()),
                    ))
                    .into();
                }
            };
            let recording_id = recording_id.clone();

            // Consume `Finalized` only from the completion callback, after the
            // result is delivered through this action. If the action is
            // cancelled, the callback is skipped while controller-owned
            // finalization continues and retains its result for a later stop.
            ActionExecution::new_async(
                async move { finalization.resolve().await },
                move |(result, actual_reason), ctx| {
                    RecordingController::handle(ctx).update(ctx, |controller, _| {
                        controller.consume_finalized(&recording_id);
                    });
                    send_telemetry_from_ctx!(
                        recording_stopped_telemetry(&recording_id, actual_reason, &result),
                        ctx
                    );
                    AIAgentActionResultType::StopRecording(result)
                },
            )
            .into()
        }
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for StopRecordingExecutor {
    type Event = ();
}

/// Builds the `Recording.Stopped` telemetry event from a resolved
/// [`StopRecordingResult`] and the *actual* [`FinalizeReason`] that drove
/// finalization to completion (as reported by the controller through
/// [`RecordingFinalization::resolve`]).
///
/// `outcome` is `"success"` only when an artifact was published; deliberate
/// discards and cancellations map to `"cancelled"`, and errors to `"error"`.
/// `artifact_uid_present` is `true` only for the `Success` variant, so
/// `outcome == "success"` coincides with a published artifact. The
/// `termination_reason` key comes directly from the actual `reason`, so a
/// stop action that only joined an in-progress finalization reports the
/// trigger that actually ran (`limit_reached`, `encoding_failed`,
/// `agent_finished`, `cancelled`) rather than its own claimed reason.
///
/// [`RecordingFinalization::resolve`]: super::super::recording_finalize::RecordingFinalization::resolve
#[cfg(not(target_family = "wasm"))]
fn recording_stopped_telemetry(
    recording_id: &str,
    reason: FinalizeReason,
    result: &StopRecordingResult,
) -> RecordingTelemetryEvent {
    let termination_reason = reason.telemetry_key().to_string();
    match result {
        StopRecordingResult::Success(RecordingStopped {
            duration,
            size_bytes,
            completion_status,
            ..
        }) => RecordingTelemetryEvent::Stopped {
            recording_id: recording_id.to_string(),
            outcome: "success".to_string(),
            duration_secs: Some(duration.as_secs_f64()),
            size_bytes: Some(*size_bytes),
            completion_status: match completion_status {
                computer_use::RecordingCompletionStatus::Completed => "complete",
                computer_use::RecordingCompletionStatus::StoppedEarly => "incomplete",
            }
            .to_string(),
            termination_reason,
            artifact_uid_present: true,
        },
        StopRecordingResult::Discarded => RecordingTelemetryEvent::Stopped {
            recording_id: recording_id.to_string(),
            outcome: "cancelled".to_string(),
            duration_secs: None,
            size_bytes: None,
            completion_status: "unknown".to_string(),
            termination_reason,
            artifact_uid_present: false,
        },
        StopRecordingResult::Cancelled => RecordingTelemetryEvent::Stopped {
            recording_id: recording_id.to_string(),
            outcome: "cancelled".to_string(),
            duration_secs: None,
            size_bytes: None,
            completion_status: "unknown".to_string(),
            termination_reason,
            artifact_uid_present: false,
        },
        StopRecordingResult::Error(_) => RecordingTelemetryEvent::Stopped {
            recording_id: recording_id.to_string(),
            outcome: "error".to_string(),
            duration_secs: None,
            size_bytes: None,
            completion_status: "unknown".to_string(),
            termination_reason,
            artifact_uid_present: false,
        },
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "stop_recording_tests.rs"]
mod tests;
