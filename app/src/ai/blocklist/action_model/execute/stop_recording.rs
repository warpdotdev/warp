#[cfg(not(target_family = "wasm"))]
use ai::agent::action_result::StopRecordingResult;
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
            recording_controller::{RecordingController, StopRecordingControllerError},
            recording_finalize::{FinalizeReason, finalize_recording_by_id},
        },
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
            let ExecuteActionInput {
                action,
                conversation_id,
            } = input;
            let AIAgentActionType::StopRecording { recording_id } = &action.action else {
                return ActionExecution::<()>::InvalidAction.into();
            };
            // Explicit stop remains retry-safe while the conversation is
            // syncing: do not claim the handle until it can be associated with
            // the conversation. Automatic terminal paths may instead use the
            // ambient run association so they can upload before teardown.
            let conversation_is_synced = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_conversation_token())
                .is_some();
            if !conversation_is_synced {
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

            // Consume `Finalized` only from the completion callback, after the
            // result is delivered through this action. If the action is
            // cancelled, the callback is skipped while controller-owned
            // finalization continues and retains its result for a later stop.
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
