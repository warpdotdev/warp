use std::time::SystemTime;

use ai::agent::action_result::{AIAgentActionResultType, RecordingStarted, StartRecordingResult};
use futures::FutureExt;
use futures::future::BoxFuture;
use uuid::Uuid;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
use crate::ai::blocklist::action_model::RecordingTelemetryEvent;
use crate::ai::blocklist::action_model::recording_controller::RecordingController;
#[cfg(not(target_family = "wasm"))]
use crate::ai::blocklist::action_model::recording_finalize::spawn_recording_exit_watcher;
use crate::send_telemetry_from_ctx;

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
    ) -> impl Into<AnyActionExecution> + use<> {
        let ExecuteActionInput {
            action,
            conversation_id,
        } = input;
        let AIAgentActionType::StartRecording {
            frame_rate,
            max_duration,
            max_size_bytes,
            summary,
            description,
            playback_speed_multiplier,
            window,
        } = action.action.clone()
        else {
            return ActionExecution::InvalidAction;
        };
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
                let playback_speed_multiplier = resolve_playback_speed_multiplier(
                    playback_speed_multiplier,
                    defaults.playback_speed_multiplier,
                );
                let resolved_frame_rate = if frame_rate > 0 {
                    frame_rate
                } else {
                    defaults.frame_rate
                };
                let config = computer_use::RecordingConfig {
                    frame_rate: resolved_frame_rate,
                    max_duration: max_duration.unwrap_or(defaults.max_duration),
                    max_size_bytes: max_size_bytes.unwrap_or(defaults.max_size_bytes),
                    playback_speed_multiplier,
                    target,
                };
                // Carry the resolved frame rate and speed multiplier to the
                // completion callback: the controller stores the frame rate for
                // the post-stop smart cut's one-frame minimum, and the speed
                // multiplier for the post-stop speed pass (Linux) / as a record
                // of what was already applied live during capture (macOS).
                // Neither is echoed back to the server.
                (
                    recorder.start(config).await,
                    resolved_frame_rate,
                    playback_speed_multiplier,
                )
            },
            move |(result, frame_rate, playback_speed_multiplier), ctx| match result {
                Ok(handle) => {
                    let recording_id = Uuid::new_v4().to_string();
                    let started_at = SystemTime::now();
                    let width_px = handle.width() as i32;
                    let height_px = handle.height() as i32;
                    send_telemetry_from_ctx!(
                        RecordingTelemetryEvent::Started {
                            recording_id: recording_id.clone(),
                            capture_target: match target {
                                computer_use::Target::Screen => "screen",
                                computer_use::Target::Window { .. } => "window",
                            }
                            .to_string(),
                        },
                        ctx
                    );
                    let controller = RecordingController::handle(ctx);
                    controller.update(ctx, |controller, _| {
                        controller.finish_start(
                            recording_id.clone(),
                            conversation_id,
                            handle,
                            frame_rate,
                            playback_speed_multiplier,
                            summary,
                            description,
                            target,
                        );
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

/// Resolves the playback speed multiplier to actually pass to the recorder
/// from the wire-presence-preserving value carried on the action.
///
/// `None` means the server never specified a value at all (e.g. an old
/// server build), so the client's own `default` applies. `Some(raw)` is an
/// explicit server request -- including a value <= 1.0, which explicitly
/// asks for real-time -- and is validated through
/// `computer_use::sanitize_playback_speed_multiplier` rather than being
/// coerced back to `default`. Collapsing an explicit real-time request to
/// `default` was a real regression: a server configured for real-time
/// (`playback_speed_multiplier <= 1.0` in warp-server) would otherwise still
/// produce a sped-up recording.
fn resolve_playback_speed_multiplier(playback_speed_multiplier: Option<f32>, default: f32) -> f32 {
    match playback_speed_multiplier {
        None => default,
        Some(raw) => computer_use::sanitize_playback_speed_multiplier(raw),
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "start_recording_tests.rs"]
mod tests;
