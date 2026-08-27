use std::sync::Arc;

use ai::skills::SkillPathOrigin;
use prost_types::FieldMask;
use warp_multi_agent_api as api;

use super::{ExtractMessagesError, Task, TaskMessageContext, UpdateTaskError};
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentExchange, AIAgentExchangeId, AIAgentInput,
    AIAgentOutput, AIAgentOutputStatus, FinishedAIAgentOutput, MessageId, Shared, TaskId,
    UseComputerResult,
};
use crate::ai::llms::LLMId;
use crate::test_util::ai_agent_tasks::{
    create_api_subtask, create_api_task, create_message, create_subagent_tool_call_message,
};

/// Creates a Task backed by server data from the given api::Task.
fn create_server_task(api_task: api::Task) -> Task {
    Task::new_restored_root(api_task, std::iter::empty())
}

// =============================================================================
// Tests for Task::splice_messages()
// =============================================================================

#[test]
fn test_splice_messages_happy_path() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
            create_message("m4", task_id),
            create_message("m5", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract m2, m3, m4 (middle 3 messages).
    let replacement = vec![create_message("replacement", task_id)];
    let result = task.splice_messages("m2", "m4", 3, replacement);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 3);
    assert_eq!(extracted[0].id, "m2");
    assert_eq!(extracted[1].id, "m3");
    assert_eq!(extracted[2].id, "m4");

    // Verify the task now has: m1, replacement, m5.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["m1", "replacement", "m5"]);
}

#[test]
fn test_splice_messages_single_message() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract just m2.
    let replacement = vec![create_message("replacement", task_id)];
    let result = task.splice_messages("m2", "m2", 1, replacement);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].id, "m2");

    // Verify the task now has: m1, replacement, m3.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["m1", "replacement", "m3"]);
}

#[test]
fn test_splice_messages_all_messages() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract all messages.
    let replacement = vec![create_message("replacement", task_id)];
    let result = task.splice_messages("m1", "m3", 3, replacement);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 3);

    // Verify the task now only has the replacement.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["replacement"]);
}

#[test]
fn test_splice_messages_empty_replacement() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract m2 with no replacement (pure deletion).
    let result = task.splice_messages("m2", "m2", 1, vec![]);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].id, "m2");

    // Verify the task now has: m1, m3.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["m1", "m3"]);
}

#[test]
fn test_splice_messages_multiple_replacements() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract m2 and replace with two messages.
    let replacement = vec![create_message("r1", task_id), create_message("r2", task_id)];
    let result = task.splice_messages("m2", "m2", 1, replacement);

    assert!(result.is_ok());

    // Verify the task now has: m1, r1, r2, m3.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["m1", "r1", "r2", "m3"]);
}

#[test]
fn test_splice_messages_first_message_not_found() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![create_message("m1", task_id), create_message("m2", task_id)],
    );
    let mut task = create_server_task(api_task);

    let result = task.splice_messages("nonexistent", "m2", 1, vec![]);

    assert!(matches!(
        result,
        Err(ExtractMessagesError::FirstMessageNotFound(id)) if id == "nonexistent"
    ));
}

#[test]
fn test_splice_messages_last_message_not_found() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![create_message("m1", task_id), create_message("m2", task_id)],
    );
    let mut task = create_server_task(api_task);

    let result = task.splice_messages("m1", "nonexistent", 1, vec![]);

    assert!(matches!(
        result,
        Err(ExtractMessagesError::LastMessageNotFound(id)) if id == "nonexistent"
    ));
}

#[test]
fn test_splice_messages_invalid_range() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // first_message_id appears after last_message_id.
    let result = task.splice_messages("m3", "m1", 3, vec![]);

    assert!(matches!(result, Err(ExtractMessagesError::InvalidRange)));
}

#[test]
fn test_splice_messages_checksum_mismatch_too_few() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Claim there are 5 messages when there are only 3 in the range.
    let result = task.splice_messages("m1", "m3", 5, vec![]);

    assert!(matches!(
        result,
        Err(ExtractMessagesError::ChecksumMismatch {
            expected: 5,
            actual: 3
        })
    ));
}

#[test]
fn test_splice_messages_checksum_mismatch_too_many() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Claim there is 1 message when there are 3 in the range.
    let result = task.splice_messages("m1", "m3", 1, vec![]);

    assert!(matches!(
        result,
        Err(ExtractMessagesError::ChecksumMismatch {
            expected: 1,
            actual: 3
        })
    ));
}

#[test]
fn test_splice_messages_optimistic_task_not_initialized() {
    let mut task = Task::new_optimistic_root();

    let result = task.splice_messages("m1", "m2", 2, vec![]);

    assert!(matches!(
        result,
        Err(ExtractMessagesError::TaskNotInitialized)
    ));
}

#[test]
fn test_splice_messages_from_beginning() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
            create_message("m4", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract from the beginning.
    let replacement = vec![create_message("replacement", task_id)];
    let result = task.splice_messages("m1", "m2", 2, replacement);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 2);

    // Verify the task now has: replacement, m3, m4.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["replacement", "m3", "m4"]);
}

#[test]
fn test_splice_messages_from_end() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![
            create_message("m1", task_id),
            create_message("m2", task_id),
            create_message("m3", task_id),
            create_message("m4", task_id),
        ],
    );
    let mut task = create_server_task(api_task);

    // Extract from the end.
    let replacement = vec![create_message("replacement", task_id)];
    let result = task.splice_messages("m3", "m4", 2, replacement);

    assert!(result.is_ok());
    let extracted = result.unwrap();
    assert_eq!(extracted.len(), 2);

    // Verify the task now has: m1, m2, replacement.
    let remaining_ids: Vec<_> = task.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(remaining_ids, vec!["m1", "m2", "replacement"]);
}

// =============================================================================
// Tests for Task::new_moved_messages_subtask()
// =============================================================================

#[test]
fn test_new_moved_messages_subtask_basic() {
    let parent_id = "parent";
    let subtask_id = "subtask";

    // Create parent task with a subagent call referencing the subtask.
    let parent_api_task = create_api_task(
        parent_id,
        vec![
            create_message("m1", parent_id),
            create_subagent_tool_call_message("subagent_call", parent_id, subtask_id, None),
            create_message("m2", parent_id),
        ],
    );

    // Create the subtask api::Task with some messages.
    let subtask_api_task = create_api_task(
        subtask_id,
        vec![
            create_message("s1", subtask_id),
            create_message("s2", subtask_id),
        ],
    );

    let subtask = Task::new_moved_messages_subtask(subtask_api_task, &parent_api_task);

    assert_eq!(subtask.id().to_string(), subtask_id);
    assert!(subtask.exchanges().next().is_none()); // No exchanges.
    assert_eq!(subtask.messages().count(), 2);

    // Should have subagent_params extracted from parent.
    let subagent_params = subtask.subagent_params();
    assert!(subagent_params.is_some());
    assert_eq!(
        subagent_params.unwrap().tool_call_id,
        "subagent_call_tool_call"
    );
}

#[test]
fn test_new_moved_messages_subtask_with_summarization_metadata() {
    let parent_id = "parent";
    let subtask_id = "subtask";

    // Create parent task with a summarization subagent call.
    let parent_api_task = create_api_task(
        parent_id,
        vec![create_subagent_tool_call_message(
            "summary_call",
            parent_id,
            subtask_id,
            Some(api::message::tool_call::subagent::Metadata::Summarization(
                (),
            )),
        )],
    );

    let subtask_api_task = create_api_task(subtask_id, vec![create_message("s1", subtask_id)]);

    let subtask = Task::new_moved_messages_subtask(subtask_api_task, &parent_api_task);

    // Check that subagent_params has the summarization metadata.
    let subagent_params = subtask.subagent_params();
    assert!(subagent_params.is_some());

    let call = &subagent_params.unwrap().call;
    assert!(matches!(
        call.metadata,
        Some(api::message::tool_call::subagent::Metadata::Summarization(
            _
        ))
    ));
}

#[test]
fn test_new_moved_messages_subtask_no_matching_subagent_call() {
    let parent_id = "parent";
    let subtask_id = "subtask";

    // Parent task has no subagent call to this subtask.
    let parent_api_task = create_api_task(
        parent_id,
        vec![
            create_message("m1", parent_id),
            // Subagent call references a different task.
            create_subagent_tool_call_message("other_call", parent_id, "other_task", None),
        ],
    );

    let subtask_api_task = create_api_task(subtask_id, vec![create_message("s1", subtask_id)]);

    let subtask = Task::new_moved_messages_subtask(subtask_api_task, &parent_api_task);

    // No subagent_params since no matching call was found.
    assert!(subtask.subagent_params().is_none());
}

#[test]
fn test_new_moved_messages_subtask_preserves_messages() {
    let parent_id = "parent";
    let subtask_id = "subtask";

    let parent_api_task = create_api_task(
        parent_id,
        vec![create_subagent_tool_call_message(
            "call", parent_id, subtask_id, None,
        )],
    );

    // Subtask with multiple messages.
    let subtask_api_task = create_api_task(
        subtask_id,
        vec![
            create_message("s1", subtask_id),
            create_message("s2", subtask_id),
            create_message("s3", subtask_id),
        ],
    );

    let subtask = Task::new_moved_messages_subtask(subtask_api_task, &parent_api_task);

    // All messages should be preserved.
    let message_ids: Vec<_> = subtask.messages().map(|m| m.id.as_str()).collect();
    assert_eq!(message_ids, vec!["s1", "s2", "s3"]);
}

// =============================================================================
// Tests for Task::upsert_message()
// =============================================================================

fn create_exchange(
    owned_message_ids: &[&str],
    output_status: AIAgentOutputStatus,
    start_time: chrono::DateTime<chrono::Local>,
) -> AIAgentExchange {
    let model_id = LLMId::from("auto");
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status,
        added_message_ids: owned_message_ids
            .iter()
            .map(|id| MessageId::new(id.to_string()))
            .collect(),
        start_time,
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: model_id.clone(),
        request_cost: None,
        coding_model_id: model_id.clone(),
        cli_agent_model_id: model_id.clone(),
        computer_use_model_id: model_id,
        response_initiator: None,
    }
}

fn streaming_output_status() -> AIAgentOutputStatus {
    AIAgentOutputStatus::Streaming {
        output: Some(Shared::new(AIAgentOutput::default())),
    }
}

fn finished_output_status() -> AIAgentOutputStatus {
    AIAgentOutputStatus::Finished {
        finished_output: FinishedAIAgentOutput::Success {
            output: Shared::new(AIAgentOutput::default()),
        },
    }
}

fn message_context() -> TaskMessageContext<'static> {
    TaskMessageContext {
        current_todo_list: None,
        active_code_review: None,
        skill_path_origin: &SkillPathOrigin::Unavailable,
    }
}

fn agent_output_message(id: &str, task_id: &str, text: &str) -> api::Message {
    api::Message {
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: text.to_string(),
            },
        )),
        ..create_message(id, task_id)
    }
}

fn use_computer_result_message(
    id: &str,
    task_id: &str,
    tool_call_id: &str,
    screenshot: api::RawImage,
) -> api::Message {
    api::Message {
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: tool_call_id.to_string(),
                context: None,
                result: Some(api::message::tool_call_result::Result::UseComputer(
                    api::UseComputerResult {
                        result: Some(api::use_computer_result::Result::Success(
                            api::use_computer_result::Success {
                                screenshot: Some(screenshot),
                                cursor_position: None,
                                windows: vec![],
                                captured_window: None,
                            },
                        )),
                    },
                )),
            },
        )),
        ..create_message(id, task_id)
    }
}

fn inline_raw_image() -> api::RawImage {
    api::RawImage {
        source: Some(api::raw_image::Source::Data(vec![1, 2, 3])),
        mime_type: "image/png".to_string(),
        width: 2,
        height: 2,
    }
}

fn stored_ref_raw_image() -> api::RawImage {
    api::RawImage {
        source: Some(api::raw_image::Source::StoredRef(
            api::StoredScreenshotRef {
                screenshot_uid: "shot-1".to_string(),
                conversation_id: "conv-1".to_string(),
                size_bytes: 3,
            },
        )),
        mime_type: "image/png".to_string(),
        width: 2,
        height: 2,
    }
}

#[test]
fn test_upsert_message_same_stream_updates_current_exchange() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![agent_output_message("m1", task_id, "original text")],
    );
    let exchange = create_exchange(&["m1"], streaming_output_status(), chrono::Local::now());
    let exchange_id = exchange.id;
    let mut task = Task::new_restored_root(api_task, std::iter::once(exchange));

    let (updated_exchange_id, updated_message) = task
        .upsert_message(
            agent_output_message("m1", task_id, "updated text"),
            Some(exchange_id),
            message_context(),
            FieldMask {
                paths: vec!["agent_output".to_string()],
            },
            false,
        )
        .expect("same-stream upsert should succeed");

    assert_eq!(updated_exchange_id, exchange_id);
    assert!(matches!(
        &updated_message.message,
        Some(api::message::Message::AgentOutput(output)) if output.text == "updated text"
    ));

    // The rendered output of the current stream's exchange picked up the update.
    let exchange = task.exchange(exchange_id).expect("exchange exists");
    let output = exchange.output_status.output().expect("output exists");
    let messages = &output.get().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, MessageId::new("m1".to_string()));
}

#[test]
fn test_upsert_message_cross_exchange_swaps_screenshot_bytes_for_stored_ref() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![use_computer_result_message(
            "m-shot",
            task_id,
            "tool-1",
            inline_raw_image(),
        )],
    );

    // The earlier exchange owns the screenshot message and already finished, and holds the
    // corresponding action-result input as reconstructed by shared-session viewers.
    let mut earlier_exchange = create_exchange(
        &["m-shot"],
        finished_output_status(),
        chrono::Local::now() - chrono::Duration::minutes(1),
    );
    earlier_exchange.input.push(AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: "tool-1".to_string().into(),
            task_id: TaskId::new(task_id.to_string()),
            result: AIAgentActionResultType::UseComputer(UseComputerResult::success(
                computer_use::ActionResult::legacy(
                    Some(computer_use::Screenshot {
                        width: 2,
                        height: 2,
                        original_width: 2,
                        original_height: 2,
                        data: vec![1, 2, 3],
                        mime_type: "image/png".into(),
                    }),
                    None,
                ),
            )),
        },
        context: Arc::from(Vec::new()),
    });
    let earlier_exchange_id = earlier_exchange.id;
    let current_exchange = create_exchange(&[], streaming_output_status(), chrono::Local::now());
    let current_exchange_id = current_exchange.id;
    let mut task =
        Task::new_restored_root(api_task, [earlier_exchange, current_exchange].into_iter());

    let (updated_exchange_id, updated_message) = task
        .upsert_message(
            use_computer_result_message("m-shot", task_id, "tool-1", stored_ref_raw_image()),
            Some(current_exchange_id),
            message_context(),
            FieldMask {
                paths: vec!["tool_call_result".to_string()],
            },
            true,
        )
        .expect("cross-exchange upsert should succeed");

    // The update lands on the exchange that owns the message, not the current stream's.
    assert_eq!(updated_exchange_id, earlier_exchange_id);

    // The task source (echoed back to the server) retains the stored ref in place of the
    // inline bytes.
    let Some(api::message::Message::ToolCallResult(result)) = &updated_message.message else {
        panic!("expected a tool call result message");
    };
    let Some(api::message::tool_call_result::Result::UseComputer(use_computer)) = &result.result
    else {
        panic!("expected a use computer result");
    };
    let Some(api::use_computer_result::Result::Success(success)) = &use_computer.result else {
        panic!("expected a success result");
    };
    let screenshot = success.screenshot.as_ref().expect("screenshot present");
    let Some(api::raw_image::Source::StoredRef(stored_ref)) = &screenshot.source else {
        panic!("expected a stored ref screenshot source");
    };
    assert_eq!(stored_ref.screenshot_uid, "shot-1");

    // The owning exchange's action-result input was swapped to the ref-only form.
    let earlier_exchange = task
        .exchange(earlier_exchange_id)
        .expect("earlier exchange exists");
    let AIAgentInput::ActionResult { result, .. } = &earlier_exchange.input[0] else {
        panic!("expected an action result input");
    };
    let AIAgentActionResultType::UseComputer(UseComputerResult::Success {
        result: action_result,
        stored_screenshot_ref,
    }) = &result.result
    else {
        panic!("expected a use computer success result");
    };
    assert!(action_result.screenshot.is_none());
    assert_eq!(
        stored_screenshot_ref
            .as_ref()
            .map(|stored_ref| stored_ref.screenshot_uid.as_str()),
        Some("shot-1")
    );
}

#[test]
fn test_upsert_message_succeeds_without_current_stream_exchange() {
    let task_id = "task1";
    let api_task = create_api_task(
        task_id,
        vec![agent_output_message("m1", task_id, "original text")],
    );
    let exchange = create_exchange(&["m1"], finished_output_status(), chrono::Local::now());
    let exchange_id = exchange.id;
    let mut task = Task::new_restored_root(api_task, std::iter::once(exchange));

    // No exchange was added for this task in the current stream; the update still applies
    // to the (finished) exchange that owns the message.
    let (updated_exchange_id, _) = task
        .upsert_message(
            agent_output_message("m1", task_id, "updated text"),
            None,
            message_context(),
            FieldMask {
                paths: vec!["agent_output".to_string()],
            },
            false,
        )
        .expect("upsert without a current-stream exchange should succeed");

    assert_eq!(updated_exchange_id, exchange_id);

    // The finished exchange's rendered output picked up the update.
    let exchange = task.exchange(exchange_id).expect("exchange exists");
    let output = exchange.output_status.output().expect("output exists");
    assert_eq!(output.get().messages.len(), 1);
}

#[test]
fn test_upsert_message_new_message_requires_current_stream_exchange() {
    let task_id = "task1";
    let api_task = create_api_task(task_id, vec![]);
    let mut task = create_server_task(api_task);

    let result = task.upsert_message(
        agent_output_message("new-message", task_id, "text"),
        None,
        message_context(),
        FieldMask {
            paths: vec!["agent_output".to_string()],
        },
        false,
    );

    assert!(matches!(result, Err(UpdateTaskError::ExchangeNotFound)));
}

// =============================================================================
// Tests for Warp docs subagent classification
// =============================================================================

#[test]
fn test_is_warp_documentation_search_subagent() {
    let parent_id = "parent";
    let subtask_id = "subtask";
    let parent_api_task = create_api_task(
        parent_id,
        vec![create_subagent_tool_call_message(
            "docs_call",
            parent_id,
            subtask_id,
            Some(api::message::tool_call::subagent::Metadata::WarpDocumentationSearch(())),
        )],
    );
    let subtask_api_task = create_api_subtask(subtask_id, parent_id, vec![]);
    let subtask = Task::new_restored_subtask(subtask_api_task, &parent_api_task, vec![]);

    assert!(subtask.is_warp_documentation_search_subagent());
    assert!(!subtask.is_conversation_search_subagent());
}
