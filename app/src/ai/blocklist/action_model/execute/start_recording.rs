use std::time::{Duration, SystemTime};

use ai::agent::action_result::{AIAgentActionResultType, RecordingStarted, StartRecordingResult};
use futures::future::BoxFuture;
use futures::FutureExt;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
use crate::ai::blocklist::action_model::recording_controller::{
    RecordingController, RecordingSession,
};

/// Enforced maximum recording duration. Bounds orphaned captures while leaving
/// enough room for a typical computer-use task.
const MAX_RECORDING_DURATION: Duration = Duration::from_secs(10 * 60);
/// Enforced maximum recording size in bytes. Caps local disk growth if display
/// content produces a large video before the time limit.
const MAX_RECORDING_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

pub struct StartRecordingExecutor;

impl StartRecordingExecutor {
    pub fn new() -> Self {
        Self
    }

    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ExecuteActionInput { action, .. } = input;
        // Recording is only offered within an already-approved computer-use
        // subagent, so approval extends to it. Still require the feature flag.
        matches!(action.action, AIAgentActionType::StartRecording)
            && FeatureFlag::VideoRecording.is_enabled()
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentActionType::StartRecording = &action.action else {
            return ActionExecution::InvalidAction;
        };

        // Reserve the single runtime slot up front so a concurrent start can't
        // race past the guard while ffmpeg is spinning up.
        let controller = RecordingController::handle(ctx);
        if let Err(error) = controller.update(ctx, |controller, _| controller.try_begin_start()) {
            return ActionExecution::Sync(AIAgentActionResultType::StartRecording(
                StartRecordingResult::Error(error.to_string()),
            ));
        }

        ActionExecution::new_async(
            async move {
                let recorder = computer_use::create_recorder();
                let config = computer_use::RecordingConfig {
                    max_duration: Some(MAX_RECORDING_DURATION),
                    max_size_bytes: Some(MAX_RECORDING_SIZE_BYTES),
                    ..Default::default()
                };
                recorder.start(config).await
            },
            |result, ctx| match result {
                Ok(handle) => {
                    let recording_id = Uuid::new_v4().to_string();
                    let started_at = SystemTime::now();
                    let width_px = handle.width() as i32;
                    let height_px = handle.height() as i32;
                    let frame_rate = handle.frame_rate() as i32;
                    let max_duration = handle.max_duration();
                    let max_size_bytes = handle.max_size_bytes().map(|bytes| bytes as i64);
                    RecordingController::handle(ctx).update(ctx, |controller, _| {
                        controller
                            .finish_start(recording_id.clone(), RecordingSession::new(handle));
                    });
                    AIAgentActionResultType::StartRecording(StartRecordingResult::Success(
                        RecordingStarted {
                            recording_id,
                            started_at,
                            width_px,
                            height_px,
                            frame_rate,
                            max_duration,
                            max_size_bytes,
                        },
                    ))
                }
                Err(error) => {
                    RecordingController::handle(ctx)
                        .update(ctx, |controller, _| controller.abort_start());
                    AIAgentActionResultType::StartRecording(StartRecordingResult::Error(
                        error.to_string(),
                    ))
                }
            },
        )
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for StartRecordingExecutor {
    type Event = ();
}
