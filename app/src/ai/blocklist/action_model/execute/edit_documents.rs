use std::collections::HashMap;

use ai::diff_validation::DiffDelta;
use futures::FutureExt;
use futures::future::BoxFuture;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{ActionExecution, AnyActionExecution, ExecuteActionInput, PreprocessActionInput};
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, DocumentContext, EditDocumentsRequest, EditDocumentsResult,
};
use crate::ai::document::ai_document_model::{
    AIDocumentId, AIDocumentModel, AIDocumentUpdateSource,
};

pub struct EditDocumentsExecutor;

impl EditDocumentsExecutor {
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
    ) -> impl Into<AnyActionExecution> + use<> {
        let ExecuteActionInput { action, .. } = input;
        let AIAgentAction {
            action: AIAgentActionType::EditDocuments(EditDocumentsRequest { diffs }),
            ..
        } = action
        else {
            return ActionExecution::<EditDocumentsResult>::InvalidAction;
        };

        let model = AIDocumentModel::handle(ctx);

        let mut updated_documents = Vec::new();
        let mut error_messages = Vec::new();
        let mut document_deltas: HashMap<AIDocumentId, Vec<DiffDelta>> = HashMap::new();

        // First pass: validate all diffs and accumulate deltas
        for diff in diffs.iter() {
            // Get current document content
            let current_content = match model
                .as_ref(ctx)
                .get_document_content(&diff.document_id, ctx)
            {
                Some(content) => content,
                None => {
                    error_messages.push(format!("Document {} does not exist.", diff.document_id));
                    continue;
                }
            };

            // Apply the diff using fuzzy matching logic
            let search_replace = ai::diff_validation::SearchAndReplace {
                search: diff.search.clone(),
                replace: diff.replace.clone(),
            };

            let content_name = format!("document_{}", diff.document_id);
            let fuzzy_result = ai::diff_validation::fuzzy_match_diffs(
                &content_name,
                &[search_replace],
                current_content,
            );

            // Check if diff application failed
            if fuzzy_result.warrants_failure() {
                let error_msg = if let Some(failures) = &fuzzy_result.failures {
                    if failures.fuzzy_match_failures > 0 {
                        format!(
                            "Could not apply diff to document {}: content mismatch",
                            diff.document_id
                        )
                    } else if failures.noop_deltas > 0 {
                        format!("Changes to document {} were already made", diff.document_id)
                    } else {
                        format!("Failed to apply diff to document {}", diff.document_id)
                    }
                } else {
                    format!("Unknown diff failure for document {}", diff.document_id)
                };
                error_messages.push(error_msg);
                continue;
            }

            // Accumulate deltas for this document
            if let ai::diff_validation::DiffType::Update { deltas, .. } = fuzzy_result.diff_type {
                document_deltas
                    .entry(diff.document_id)
                    .or_default()
                    .extend(deltas);
            }
        }

        // If any diffs failed, don't apply any deltas.
        // This result will be sent to the LLM as an error, so we don't want partial applications since they agent won't know about them.
        if !error_messages.is_empty() {
            let combined_errors = error_messages.join("\n");
            return ActionExecution::Sync(EditDocumentsResult::Error(combined_errors).into());
        }

        // For every document, apply all deltas at once and collect updated documents for response
        model.update(ctx, |model, model_ctx| {
            for (document_id, deltas) in document_deltas {
                if let Some(version) = model.create_new_version_and_apply_diffs(
                    &document_id,
                    deltas,
                    AIDocumentUpdateSource::Agent,
                    model_ctx,
                ) {
                    let new_content = model
                        .get_document_content(&document_id, model_ctx)
                        .unwrap_or_default();

                    updated_documents.push(DocumentContext {
                        document_id,
                        document_version: version,
                        content: new_content,
                        line_ranges: vec![],
                    });
                }
            }
        });

        // All diffs succeeded, return success
        ActionExecution::Sync(EditDocumentsResult::Success { updated_documents }.into())
    }

    pub(super) fn preprocess_action(
        &mut self,
        _input: PreprocessActionInput,
        _ctx: &mut ModelContext<Self>,
    ) -> BoxFuture<'static, ()> {
        futures::future::ready(()).boxed()
    }
}

impl Entity for EditDocumentsExecutor {
    type Event = ();
}
