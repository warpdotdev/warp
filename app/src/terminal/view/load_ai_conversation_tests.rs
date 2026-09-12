use std::collections::HashSet;

use chrono::Local;
use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResult, AIAgentInput, AIAgentOutputStatus, DocumentContext,
    EditDocumentsRequest, FinishedAIAgentOutput, MessageId, Shared,
};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel};
use crate::ai::llms::LLMId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn edit_documents_result(
    action_id: &AIAgentActionId,
    task_id: &TaskId,
    document_id: AIDocumentId,
    content: &str,
) -> AIAgentActionResult {
    AIAgentActionResult {
        id: action_id.clone(),
        task_id: task_id.clone(),
        result: AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success {
            updated_documents: vec![DocumentContext {
                document_id,
                document_version: Default::default(),
                content: content.to_owned(),
                line_ranges: vec![],
            }],
        }),
    }
}

fn edit_documents_exchange(
    action_id: &AIAgentActionId,
    task_id: &TaskId,
    document_id: AIDocumentId,
    content: &str,
) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![AIAgentInput::ActionResult {
            result: edit_documents_result(action_id, task_id, document_id, content),
            context: Default::default(),
        }],
        output_status: AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(AIAgentOutput {
                    messages: vec![AIAgentOutputMessage {
                        id: MessageId::new("edit-documents".to_owned()),
                        message: AIAgentOutputMessageType::Action(AIAgentAction {
                            id: action_id.clone(),
                            task_id: task_id.clone(),
                            action: AIAgentActionType::EditDocuments(EditDocumentsRequest {
                                diffs: vec![],
                            }),
                            requires_result: true,
                        }),
                        citations: vec![],
                    }],
                    ..Default::default()
                }),
            },
        },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn restoring_edit_documents_uses_result_from_target_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let first_conversation_id = AIConversationId::new();
        let second_conversation_id = AIConversationId::new();
        let action_id = AIAgentActionId::from("shared-edit-id".to_owned());
        let task_id = TaskId::new("task".to_owned());
        let document_id = AIDocumentId::new();
        let first_exchange =
            edit_documents_exchange(&action_id, &task_id, document_id, "wrong conversation");
        let second_exchange =
            edit_documents_exchange(&action_id, &task_id, document_id, "target conversation");

        terminal.update(&mut app, |view, ctx| {
            AIDocumentModel::handle(ctx).update(ctx, |documents, ctx| {
                documents.restore_document(
                    document_id,
                    second_conversation_id,
                    "Plan",
                    "initial",
                    Local::now(),
                    ctx,
                );
            });
            view.ai_action_model.update(ctx, |actions, ctx| {
                actions.restore_action_results_from_exchanges(
                    first_conversation_id,
                    vec![&first_exchange],
                );
                actions.restore_action_results_from_exchanges(
                    second_conversation_id,
                    vec![&second_exchange],
                );
                actions.apply_finished_action_result(
                    first_conversation_id,
                    edit_documents_result(&action_id, &task_id, document_id, "wrong conversation"),
                    ctx,
                );
            });
            view.restore_ai_documents_from_exchanges(
                &[&second_exchange],
                second_conversation_id,
                ctx,
            );
        });

        AIDocumentModel::handle(&app).read(&app, |documents, ctx| {
            assert_eq!(
                documents.get_document_content(&document_id, ctx).as_deref(),
                Some("target conversation")
            );
        });
    });
}
