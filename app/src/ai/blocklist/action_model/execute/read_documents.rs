use futures::future::BoxFuture;
use futures::FutureExt;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, DocumentContext, ReadDocumentsRequest, ReadDocumentsResult,
};
use crate::ai::document::ai_document_model::AIDocumentModel;

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
        let ExecuteActionInput { action, .. } = input;
        let AIAgentAction {
            action: AIAgentActionType::ReadDocuments(ReadDocumentsRequest { document_ids }),
            ..
        } = action
        else {
            return ActionExecution::<ReadDocumentsResult>::InvalidAction;
        };

        // Access the model synchronously before the async block
        let model = AIDocumentModel::handle(ctx);
        let documents: Vec<DocumentContext> = document_ids
            .iter()
            .filter_map(|id| {
                let model = model.as_ref(ctx);
                let content = model.get_document_content(id, ctx)?;
                let version = model.get_current_document(id)?.version;
                Some(DocumentContext {
                    document_id: *id,
                    content,
                    line_ranges: vec![],
                    document_version: version,
                })
            })
            .collect();

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
