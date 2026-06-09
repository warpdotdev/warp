use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, DocumentContext, ReadDocumentsRequest, ReadDocumentsResult,
};
use crate::ai::blocklist::orchestration_topology::conversation_participates_in_orchestration;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel};

pub struct ReadDocumentsExecutor;

impl ReadDocumentsExecutor {
    pub fn new() -> Self {
        Self
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
        ctx: &mut ModelContext<Self>,
    ) -> impl Into<AnyActionExecution> {
        let ExecuteActionInput {
            action,
            conversation_id,
        } = input;
        let AIAgentAction {
            action: AIAgentActionType::ReadDocuments(ReadDocumentsRequest { document_ids }),
            ..
        } = action
        else {
            return ActionExecution::<ReadDocumentsResult>::InvalidAction;
        };

        let (mut documents, mut missing_documents) = read_documents(document_ids, ctx);
        // Orchestrated children may reference plans owned by their parent conversation, so their
        // local document model can legitimately be missing a requested plan.
        let participates_in_orchestration = conversation_participates_in_orchestration(
            BlocklistAIHistoryModel::as_ref(ctx),
            conversation_id,
        );
        if !missing_documents.is_empty() && participates_in_orchestration {
            AIDocumentModel::handle(ctx).update(ctx, |model, ctx| {
                for document_id in &missing_documents {
                    if let Err(error) = model.hydrate_saved_plan_from_warp_drive(
                        *document_id,
                        conversation_id,
                        ctx,
                    ) {
                        log::warn!(
                            "Failed to hydrate requested plan document {document_id} from Warp Drive: {error}"
                        );
                    }
                }
            });
            (documents, missing_documents) = read_documents(document_ids, ctx);
        }

        if !missing_documents.is_empty() {
            let missing_list = missing_documents
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return ActionExecution::Sync(
                ReadDocumentsResult::Error(format!("Document(s) not found: {missing_list}")).into(),
            );
        }

        ActionExecution::Sync(ReadDocumentsResult::Success { documents }.into())
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for ReadDocumentsExecutor {
    type Event = ();
}

/// Reads requested documents and returns the IDs that are not loaded locally.
fn read_documents(
    document_ids: &[AIDocumentId],
    ctx: &AppContext,
) -> (Vec<DocumentContext>, Vec<AIDocumentId>) {
    let model = AIDocumentModel::as_ref(ctx);
    let mut documents = Vec::with_capacity(document_ids.len());
    let mut missing_documents = Vec::new();
    for id in document_ids {
        let Some(content) = model.get_document_content(id, ctx) else {
            missing_documents.push(*id);
            continue;
        };
        let Some(current_document) = model.get_current_document(id) else {
            missing_documents.push(*id);
            continue;
        };
        documents.push(DocumentContext {
            document_id: *id,
            content,
            line_ranges: vec![],
            document_version: current_document.version,
        });
    }
    (documents, missing_documents)
}

#[cfg(test)]
#[path = "read_documents_tests.rs"]
mod tests;
