use std::time::SystemTime;

use ai::agent::action_result::{AIAgentActionResultType, RecordingStarted, StartRecordingResult};
use futures::future::BoxFuture;
use futures::FutureExt;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::action_model::recording_finalize::spawn_recording_exit_watcher;

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
        matches!(action.action, AIAgentActionType::StartRecording { .. })
            && FeatureFlag::VideoRecording.is_enabled()
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput {
            action,
            conversation_id,
        } = input;
        let AIAgentActionType::StartRecording {
            frame_rate,
            max_duration,
            max_size_bytes,
            playback_speed_multiplier,
            window,
            ..
        } = &action.action
        else {
            return ActionExecution::InvalidAction;
        };
        let frame_rate = *frame_rate;
        let max_duration = *max_duration;
        let max_size_bytes = *max_size_bytes;
        let playback_speed_multiplier = *playback_speed_multiplier;
        // Only honor a window target when background computer use is enabled; otherwise fall back
        // to whole-screen capture, keeping behavior byte-identical to the pre-existing path.
        let target = if FeatureFlag::BackgroundComputerUse.is_enabled() {
            window.unwrap_or(computer_use::Target::Screen)
        } else {
            computer_use::Target::Screen
        };

        // Reserve the single runtime slot up front so a concurrent start can't
        // race past the guard while ffmpeg is spinning up.
        let controller = RecordingController::handle(ctx);
        if let Err(error) = controller.update(ctx, |controller, _| {
            controller.try_begin_start(conversation_id)
        }) {
            return ActionExecution::Sync(AIAgentActionResultType::StartRecording(
                StartRecordingResult::Error(error.to_string()),
            ));
        }

        ActionExecution::new_async(
            async move {
                let recorder = computer_use::create_recorder();
                // Fall back to the recorder's defaults when the server omits a value:
                // frame rate 0 means unspecified, and absent limits would otherwise
                // leave the capture unbounded.
                let defaults = computer_use::RecordingConfig::default();
                // Use server-provided integer speed multiplier (> 1 means faster
                // playback); fall back to the client default (4x) when absent.
                let playback_speed_multiplier = playback_speed_multiplier
                    .filter(|&s| s > 1)
                    .map(|s| s as f32)
                    .unwrap_or(defaults.playback_speed_multiplier);
                let config = computer_use::RecordingConfig {
                    frame_rate: if frame_rate > 0 {
                        frame_rate
                    } else {
                        defaults.frame_rate
                    },
                    max_duration: max_duration.unwrap_or(defaults.max_duration),
                    max_size_bytes: max_size_bytes.unwrap_or(defaults.max_size_bytes),
                    playback_speed_multiplier,
                    target,
                };
                recorder.start(config).await
            },
            move |result, ctx| match result {
                Ok(handle) => {
                    let recording_id = Uuid::new_v4().to_string();
                    let started_at = SystemTime::now();
                    let width_px = handle.width() as i32;
                    let height_px = handle.height() as i32;
                    let controller = RecordingController::handle(ctx);
                    controller.update(ctx, |controller, _| {
                        controller.finish_start(recording_id.clone(), conversation_id, handle);
                    });
                    #[cfg(not(target_family = "wasm"))]
                    controller.update(ctx, |_controller, ctx| {
                        spawn_recording_exit_watcher(recording_id.clone(), ctx);
                    });
                    AIAgentActionResultType::StartRecording(StartRecordingResult::Success(
                        RecordingStarted {
                            recording_id,
                            started_at,
                            width_px,
                            height_px,
                        },
                    ))
                }
                Err(error) => {
                    RecordingController::handle(ctx).update(ctx, |controller, _| {
                        controller.abort_start(conversation_id);
                    });
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
