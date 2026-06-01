#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;

#[cfg(test)]
#[path = "upload_artifact_tests.rs"]
mod tests;

use futures::future::BoxFuture;
use futures::FutureExt;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Entity, EntityId, ModelContext, ModelHandle};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::terminal::model::session::active_session::ActiveSession;
#[cfg(not(target_family = "wasm"))]
use crate::{
    ai::{
        agent::{AIAgentAction, AIAgentActionResultType, AIAgentActionType, UploadArtifactResult},
        agent_sdk::artifact_upload::{FileArtifactUploadRequest, FileArtifactUploader},
        blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions},
        paths::host_native_absolute_path,
    },
    server::server_api::ServerApiProvider,
};

#[cfg(not(target_family = "wasm"))]
fn format_upload_artifact_error(err: &anyhow::Error) -> String {
    let error_chain = format!("{err:#}");

    if error_chain != err.to_string() {
        format!("Artifact upload failed: {error_chain}")
    } else {
        error_chain
    }
}

pub struct UploadArtifactExecutor {
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    active_session: ModelHandle<ActiveSession>,
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    terminal_view_id: EntityId,
}

impl UploadArtifactExecutor {
    pub fn new(active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            active_session,
            terminal_view_id,
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables), allow(dead_code))]
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        #[cfg(target_family = "wasm")]
        {
            false
        }

        #[cfg(not(target_family = "wasm"))]
        {
            let ExecuteActionInput {
                action:
                    AIAgentAction {
                        action: AIAgentActionType::UploadArtifact(request),
                        ..
                    },
                conversation_id,
            } = input
            else {
                return false;
            };

            let resolved_path = self.resolve_path(&request.file_path, ctx);
            BlocklistAIPermissions::as_ref(ctx)
                .can_read_files_with_conversation(
                    &conversation_id,
                    vec![resolved_path],
                    Some(self.terminal_view_id),
                    ctx,
                )
                .is_allowed()
        }
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables), allow(dead_code))]
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
                ..
            } = input;
            let AIAgentAction {
                action: AIAgentActionType::UploadArtifact(request),
                ..
            } = action
            else {
                return ActionExecution::<()>::InvalidAction.into();
            };

            let resolved_path = self.resolve_path(&request.file_path, ctx);
            let server_conversation_token = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .and_then(|conversation| conversation.server_conversation_token())
                .cloned();

            let Some(server_conversation_token) = server_conversation_token else {
                return ActionExecution::<()>::Sync(AIAgentActionResultType::UploadArtifact(
                    UploadArtifactResult::Error(
                        "Current conversation has not been synced to the server yet".to_string(),
                    ),
                ))
                .into();
            };

            BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
                model.add_temporary_file_read_permissions(conversation_id, [resolved_path.clone()]);
            });

            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
            let server_api = ServerApiProvider::as_ref(ctx).get();
            let description = request.description.clone();

            ActionExecution::new_async(
                async move {
                    let uploader = FileArtifactUploader::new(ai_client, server_api);
                    let request = FileArtifactUploadRequest {
                        path: resolved_path,
                        run_id: None,
                        conversation_id: Some(server_conversation_token),
                        description,
                    };
                    let association = uploader.resolve_upload_association(&request).await?;
                    uploader.upload_with_association(request, association).await
                },
                |result, _ctx| match result {
                    Ok(upload) => {
                        AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Success {
                            artifact_uid: upload.artifact.artifact_uid,
                            filepath: Some(upload.artifact.filepath),
                            mime_type: upload.artifact.mime_type,
                            description: upload.artifact.description,
                            size_bytes: upload.size_bytes,
                        })
                    }
                    Err(err) => AIAgentActionResultType::UploadArtifact(
                        UploadArtifactResult::Error(format_upload_artifact_error(&err)),
                    ),
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

    #[cfg(not(target_family = "wasm"))]
    fn resolve_path(&self, file_path: &str, ctx: &ModelContext<Self>) -> PathBuf {
        let current_working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned();
        let shell = self.active_session.as_ref(ctx).shell_launch_data(ctx);

        PathBuf::from(host_native_absolute_path(
            file_path,
            &shell,
            &current_working_directory,
        ))
    }
}

impl Entity for UploadArtifactExecutor {
    type Event = ();
}
