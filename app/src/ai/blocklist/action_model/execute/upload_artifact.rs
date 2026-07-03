use std::path::PathBuf;

use crate::ai::{
    agent::{AIAgentAction, AIAgentActionResultType, AIAgentActionType, UploadArtifactResult},
    blocklist::{BlocklistAIHistoryModel, BlocklistAIPermissions},
    paths::host_native_absolute_path,
};
use crate::terminal::model::session::active_session::ActiveSession;
use futures::{future::BoxFuture, FutureExt};
use warpui::SingletonEntity;
use warpui::{Entity, EntityId, ModelContext, ModelHandle};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};

pub struct UploadArtifactExecutor {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
}

impl UploadArtifactExecutor {
    pub fn new(active_session: ModelHandle<ActiveSession>, terminal_view_id: EntityId) -> Self {
        Self {
            active_session,
            terminal_view_id,
        }
    }
    pub(super) fn should_autoexecute(
        &self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
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
    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        ctx: &mut ModelContext<Self>,
    ) -> AnyActionExecution {
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
            let _ = BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id);

            BlocklistAIPermissions::handle(ctx).update(ctx, |model, _ctx| {
                model.add_temporary_file_read_permissions(conversation_id, [resolved_path.clone()]);
            });

            let _ = resolved_path;
            let _ = request;
            ActionExecution::<()>::Sync(AIAgentActionResultType::UploadArtifact(
                UploadArtifactResult::Error(
                    "Hosted artifact uploads are unavailable in local-only Warper".to_string(),
                ),
            ))
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
