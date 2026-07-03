#[cfg(not(target_family = "wasm"))]
use ai::agent::action_result::{RecordingStopped, StopRecordingResult};
use futures::future::BoxFuture;
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Entity, ModelContext};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::AIAgentActionType;
#[cfg(not(target_family = "wasm"))]
use crate::{
    ai::{
        agent::AIAgentActionResultType,
        agent_sdk::artifact_upload::{FileArtifactUploadRequest, FileArtifactUploader},
        blocklist::action_model::recording_controller::{
            RecordingController, StopRecordingControllerError,
        },
        blocklist::BlocklistAIHistoryModel,
    },
    server::server_api::ServerApiProvider,
};

#[cfg(not(target_family = "wasm"))]
fn format_stop_recording_error(err: &anyhow::Error) -> String {
    let error_chain = format!("{err:#}");
    if error_chain != err.to_string() {
        format!("Recording upload failed: {error_chain}")
    } else {
        error_chain
    }
}

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
            let server_conversation_token = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_conversation_token())
                .cloned();
            let Some(server_conversation_token) = server_conversation_token else {
                return ActionExecution::<()>::Sync(AIAgentActionResultType::StopRecording(
                    StopRecordingResult::Error(
                        StopRecordingControllerError::ConversationNotSynced.to_string(),
                    ),
                ))
                .into();
            };

            let handle = RecordingController::handle(ctx).update(ctx, |controller, _| {
                controller.take_handle_or_err(recording_id)
            });
            let handle = match handle {
                Ok(handle) => handle,
                Err(error) => {
                    return ActionExecution::<()>::Sync(AIAgentActionResultType::StopRecording(
                        StopRecordingResult::Error(error.to_string()),
                    ))
                    .into();
                }
            };

            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
            let server_api = ServerApiProvider::as_ref(ctx).get();

            ActionExecution::new_async(
                async move {
                    let recorder = computer_use::create_recorder();
                    let output = match recorder.stop(handle).await {
                        Ok(output) => output,
                        Err(error) => return StopRecordingResult::Error(error.to_string()),
                    };

                    // The local file is an implementation detail; publish it and
                    // delete it so results only ever carry the artifact ref.
                    let local_path = output.path.clone();
                    let uploader = FileArtifactUploader::new(ai_client, server_api);
                    let request = FileArtifactUploadRequest {
                        path: output.path,
                        run_id: None,
                        conversation_id: Some(server_conversation_token),
                        description: None,
                    };
                    let upload_result = async {
                        let association = uploader.resolve_upload_association(&request).await?;
                        uploader.upload_with_association(request, association).await
                    }
                    .await;
                    // TODO(vkodithala): Retain or retry the local file if upload fails.
                    let _ = std::fs::remove_file(&local_path);

                    match upload_result {
                        Ok(upload) => {
                            let completion_status = output.completion_status;
                            let termination_reason = match completion_status {
                                computer_use::RecordingCompletionStatus::Completed => {
                                    "Stopped by agent".to_string()
                                }
                                computer_use::RecordingCompletionStatus::StoppedEarly => {
                                    "Recording stopped before the agent requested it".to_string()
                                }
                            };
                            StopRecordingResult::Success(RecordingStopped {
                                artifact_uid: upload.artifact.artifact_uid,
                                duration: output.duration,
                                width_px: output.width as i32,
                                height_px: output.height as i32,
                                size_bytes: output.size_bytes as i64,
                                completion_status,
                                termination_reason,
                            })
                        }
                        Err(err) => StopRecordingResult::Error(format_stop_recording_error(&err)),
                    }
                },
                |result, _ctx| AIAgentActionResultType::StopRecording(result),
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
