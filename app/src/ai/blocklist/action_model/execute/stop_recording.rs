#[cfg(not(target_family = "wasm"))]
use ai::agent::action_result::StopRecordingResult;
use futures::future::BoxFuture;
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
#[cfg(not(target_family = "wasm"))]
use crate::ai::{
    agent::AIAgentActionResultType,
    blocklist::action_model::{
        recording_controller::RecordingController,
        recording_finalize::{finalize_recording_by_id, FinalizeReason},
    },
};

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
            let ExecuteActionInput { action, .. } = input;
            let AIAgentActionType::StopRecording { recording_id } = &action.action else {
                return ActionExecution::<()>::InvalidAction.into();
            };
            let finalization =
                match finalize_recording_by_id(recording_id, FinalizeReason::StoppedByAgent, ctx) {
                    Ok(finalization) => finalization,
                    Err(error) => {
                        return ActionExecution::<()>::Sync(
                            AIAgentActionResultType::StopRecording(StopRecordingResult::Error(
                                error.to_string(),
                            )),
                        )
                        .into();
                    }
                };
            let recording_id = recording_id.clone();

            ActionExecution::new_async(
                async move { finalization.resolve().await },
                move |result, ctx| {
                    RecordingController::handle(ctx).update(ctx, |controller, _| {
                        controller.consume_finalized(&recording_id);
                    });
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
