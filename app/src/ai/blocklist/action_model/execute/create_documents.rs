use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, CreateDocumentsRequest, CreateDocumentsResult,
    DocumentContext,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentModel, AIDocumentVersion};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::notebooks::editor::model::FileLinkResolutionContext;
use crate::terminal::model::session::active_session::ActiveSession;

pub struct CreateDocumentsExecutor {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: warpui::EntityId,
}

impl CreateDocumentsExecutor {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: warpui::EntityId,
    ) -> Self {
        Self {
            active_session,
            terminal_view_id,
        }
    }

    pub(super) fn should_autoexecute(
        &self,
        _input: ExecuteActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        // Document operations are always auto-executed
        true
    }

    pub(super) fn execute(
        &mut self,
        input: ExecuteActionInput,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> + use<> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentAction {
            id: action_id,
            action: AIAgentActionType::CreateDocuments(CreateDocumentsRequest { documents }),
            ..
        } = action
        else {
            return ActionExecution::<CreateDocumentsResult>::InvalidAction;
        };

        // Access the model synchronously before the async block
        let model = AIDocumentModel::handle(ctx);

        let created_documents: Vec<DocumentContext> = documents
            .iter()
            .enumerate()
            .map(|(index, document)| {
                // If we have streamed updates for this plan, just apply the last streamed update.
                // A full reset would cause syntax highlighting in code blocks to flicker.
                let existing_id = model
                    .as_ref(ctx)
                    .streaming_document_id_for_create_documents(&conversation_id, action_id, index);
                let id = if let Some(existing_id) = existing_id {
                    model.update(ctx, |model, model_ctx| {
                        model.apply_streamed_agent_update(
                            &existing_id,
                            &document.title,
                            &document.content,
                            model_ctx,
                        );
                    });
                    existing_id
                } else {
                    // If we weren't streaming updates for this document before, create the whole document now.
                    let session = self.active_session.as_ref(ctx);
                    let working_directory = session.current_working_directory().cloned();
                    let shell_launch_data = session.shell_launch_data(ctx);
                    let file_link_resolution_context =
                        working_directory.map(|working_directory| FileLinkResolutionContext {
                            working_directory,
                            shell_launch_data,
                        });
                    model.update(ctx, |model, model_ctx| {
                        model.create_document(
                            &document.title,
                            document.content.clone(),
                            conversation_id,
                            file_link_resolution_context.clone(),
                            model_ctx,
                        )
                    })
                };

                let profile = AIExecutionProfilesModel::as_ref(ctx)
                    .active_profile(Some(self.terminal_view_id), ctx);
                let should_autosync = profile.data().autosync_plans_to_warp_drive;

                if should_autosync {
                    model.update(ctx, |model, model_ctx| {
                        model.sync_to_warp_drive(id, model_ctx);
                    });
                }

                // Add plan artifact to the conversation.
                let artifact = Artifact::Plan {
                    document_uid: id.to_string(),
                    notebook_uid: None, // Will be updated when synced to Warp Drive
                    title: Some(document.title.clone()),
                };
                let terminal_view_id = self.terminal_view_id;
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    if let Some(conversation) = history.conversation_mut(&conversation_id) {
                        conversation.add_artifact(artifact, terminal_view_id, ctx);
                    }
                });

                // Read the actual content from the created document.
                // The AIDocumentModel does some processing to remove additional newlines, since our rich text editor
                // renders every newline as a linebreak.
                let actual_content = model
                    .as_ref(ctx)
                    .get_document_content(&id, ctx)
                    .unwrap_or_else(|| document.content.clone());

                DocumentContext {
                    document_id: id,
                    document_version: AIDocumentVersion::default(),
                    content: actual_content,
                    line_ranges: vec![],
                }
            })
            .collect();

        // Clear any streaming mappings for this action now that we've finalized results.
        model.update(ctx, |model, ctx| {
            model.clear_streaming_documents_for_action(&conversation_id, action_id, ctx);
        });

        ActionExecution::Sync(CreateDocumentsResult::Success { created_documents }.into())
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for CreateDocumentsExecutor {
    type Event = ();
}
