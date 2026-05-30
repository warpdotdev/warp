use std::collections::HashSet;

use chrono::Local;
use prost_types::FieldMask;
use warp_multi_agent_api as api;

use super::{ExtractMessagesError, Task};
use crate::ai::agent::{
    AIAgentActionType, AIAgentExchange, AIAgentOutput, AIAgentOutputMessageType,
    AIAgentOutputStatus, MessageId, Shared,
};
use crate::ai::llms::LLMId;
use crate::test_util::ai_agent_tasks::{
    create_api_subtask, create_api_task, create_message, create_subagent_tool_call_message,
};

/// Creates a Task backed by server data from the given api::Task.
fn create_server_task(api_task: api::Task) -> Task {
    Task::new_restored_root(api_task, std::iter::empty())
}

fn create_streaming_exchange_with_output() -> AIAgentExchange {
    AIAgentExchange {
        id: Default::default(),
        input: vec![],
        output_status: AIAgentOutputStatus::Streaming {
            output: Some(Shared::new(AIAgentOutput::default())),
        },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from(""),
        request_cost: None,
        coding_model_id: LLMId::from(""),
        cli_agent_model_id: LLMId::from(""),
        computer_use_model_id: LLMId::from(""),
        response_initiator: None,
    }
}

fn create_start_agent_tool_call_message(
    id: &str,
    task_id: &str,
    name: &str,
    prompt: &str,
) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: format!("{id}_tool_call"),
            tool: Some(api::message::tool_call::Tool::StartAgent(api::StartAgent {
                name: name.to_string(),
                prompt: prompt.to_string(),
                execution_mode: None,
                lifecycle_subscription: None,
            })),
        })),
        request_id: String::new(),
        timestamp: None,
    }
}

fn assert_start_agent_prompt(
    task: &Task,
    exchange_id: crate::ai::agent::AIAgentExchangeId,
    prompt: &str,
) {
    let exchange = task.exchange(exchange_id).expect("exchange should exist");
    let output = exchange
        .output_status
        .output()
        .expect("output should be initialized");
    let output = output.get();
    let output_message = output
        .messages
        .iter()
        .find(|message| message.id == MessageId::new("start_agent_message".to_string()))
        .expect("start agent output message should exist");

    let AIAgentOutputMessageType::Action(action) = &output_message.message else {
        panic!("expected action output message");
    };
    let AIAgentActionType::StartAgent {
        prompt: current_prompt,
        ..
    } = &action.action
    else {
        panic!("expected StartAgent action");
    };

    assert_eq!(current_prompt, prompt);
}

#[test]
fn test_upsert_message_adds_start_agent_prompt_to_output() {
    let task_id = "task1";
    let mut task = create_server_task(create_api_task(task_id, vec![]));

    let exchange = create_streaming_exchange_with_output();
    let exchange_id = exchange.id;
    task.append_exchange(exchange);

    task.upsert_message(
        create_start_agent_tool_call_message(
            "start_agent_message",
            task_id,
            "Agent 1",
            "run tests",
        ),
        exchange_id,
        None,
        None,
        FieldMask {
            paths: vec!["message.tool_call".to_string()],
        },
        false,
    )
    .expect("initial upsert should succeed");
    assert_start_agent_prompt(&task, exchange_id, "run tests");
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
