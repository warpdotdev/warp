use std::collections::HashSet;

use chrono::Local;
use uuid::Uuid;

use crate::ai::{
    agent::{
        task::{Task, TaskId},
        AIAgentExchange, AIAgentExchangeId, AIAgentOutput, AIAgentOutputMessage,
        AIAgentOutputMessageType, AIAgentOutputStatus, FinishedAIAgentOutput, MessageId, Shared,
        SubagentCall,
    },
    llms::LLMId,
};

use super::TaskStore;

fn create_test_exchange() -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status: AIAgentOutputStatus::Streaming { output: None },
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

fn create_test_task_with_exchanges(exchange_count: usize) -> Task {
    let mut task = Task::new_optimistic_root();
    for _ in 0..exchange_count {
        task.append_exchange(create_test_exchange());
    }
    task
}

fn create_test_subtask_with_exchanges(exchange_count: usize) -> Task {
    use crate::terminal::model::block::BlockId;
    let mut task = Task::new_optimistic_cli_agent_subtask(BlockId::new());
    for _ in 0..exchange_count {
        task.append_exchange(create_test_exchange());
    }
    task
}

/// Creates an exchange with a finished output containing a subagent call to the given task_id.
fn create_exchange_with_subagent_call(subtask_id: &TaskId) -> AIAgentExchange {
    let output = AIAgentOutput {
        messages: vec![AIAgentOutputMessage {
            id: MessageId::new(Uuid::new_v4().to_string()),
            message: AIAgentOutputMessageType::Subagent(SubagentCall {
                task_id: subtask_id.to_string(),
                subagent_type: crate::ai::agent::SubagentType::Unknown,
            }),
            citations: vec![],
        }],
        citations: vec![],
        server_output_id: None,
        api_metadata_bytes: None,
        suggestions: None,
        telemetry_events: vec![],
        model_info: None,
        request_cost: None,
    };

    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status: AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(output),
            },
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

#[test]
fn test_with_root_task() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();

    let store = TaskStore::with_root_task(task);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);
    assert_eq!(store.root_task_id(), &task_id);
    assert_eq!(store.root_task().expect("task exists").id(), &task_id);
    assert_eq!(store.first_exchange().map(|e| e.id), Some(exchange_ids[0]));
    assert_eq!(store.latest_exchange().map(|e| e.id), Some(exchange_ids[2]));
}

#[test]
fn test_first_and_latest_exchange_o1() {
    let task = create_test_task_with_exchanges(5);
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let store = TaskStore::with_root_task(task);

    // These should be O(1) operations
    let first = store.first_exchange().expect("has exchanges");
    let latest = store.latest_exchange().expect("has exchanges");

    assert_eq!(first.id, exchange_ids[0]);
    assert_eq!(latest.id, exchange_ids[4]);
}

#[test]
fn test_insert_subtask() {
    // Create root task with 1 exchange, then we'll add a subagent call exchange
    let root_task = create_test_task_with_exchanges(1);
    let root_task_id = root_task.id().clone();
    let mut store = TaskStore::with_root_task(root_task);

    // Create subtask with 1 exchange
    let subtask = create_test_subtask_with_exchanges(1);
    let subtask_id = subtask.id().clone();

    // Add exchange with subagent call to root task BEFORE inserting subtask
    let subagent_exchange = create_exchange_with_subagent_call(&subtask_id);
    store.append_exchange(&root_task_id, subagent_exchange);

    // Now insert the subtask - its exchanges should be included via the subagent call
    store.insert(subtask);

    assert_eq!(store.task_count(), 2);
    // 1 root exchange + 1 subagent call exchange + 1 subtask exchange = 3
    assert_eq!(store.exchange_count(), 3);
    assert!(store.get(&root_task_id).is_some());
    assert!(store.get(&subtask_id).is_some());
    assert!(store.contains(&subtask_id));
}

#[test]
fn test_remove_task() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let mut store = TaskStore::with_root_task(task);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);

    let removed = store.remove(&task_id);
    assert!(removed.is_some());
    assert_eq!(store.task_count(), 0);
    assert_eq!(store.exchange_count(), 0);
    assert!(store.get(&task_id).is_none());
}

#[test]
fn test_remove_nonexistent_task() {
    let task = create_test_task_with_exchanges(2);
    let mut store = TaskStore::with_root_task(task);

    let nonexistent_id = TaskId::new(Uuid::new_v4().to_string());
    let removed = store.remove(&nonexistent_id);
    assert!(removed.is_none());
    assert_eq!(store.task_count(), 1);
}

#[test]
fn test_all_exchanges_iteration() {
    let task = create_test_task_with_exchanges(4);
    let expected_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let store = TaskStore::with_root_task(task);

    let actual_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();

    assert_eq!(actual_ids, expected_ids);
}

#[test]
fn test_all_exchanges_by_task() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let store = TaskStore::with_root_task(task);

    let by_task = store.all_exchanges_by_task();
    assert_eq!(by_task.len(), 1);
    assert_eq!(by_task[0].0, task_id);
    assert_eq!(by_task[0].1.len(), 3);

    let actual_ids: Vec<_> = by_task[0].1.iter().map(|e| e.id).collect();
    assert_eq!(actual_ids, exchange_ids);
}

#[test]
fn test_set_root_task_replaces_old() {
    let task1 = create_test_task_with_exchanges(2);
    let task1_id = task1.id().clone();
    let mut store = TaskStore::with_root_task(task1);

    let task2 = create_test_task_with_exchanges(3);
    let task2_id = task2.id().clone();
    let task2_exchange_ids: Vec<_> = task2.exchanges().map(|e| e.id).collect();
    store.set_root_task(task2);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);
    // task1_id is now aliased to the new root task via optimistic_root_task_id
    assert!(store.get(&task1_id).is_some());
    assert!(store.get(&task2_id).is_some());
    assert_eq!(store.root_task_id(), &task2_id);

    let actual_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(actual_ids, task2_exchange_ids);
}

#[test]
fn test_append_exchange() {
    let task = create_test_task_with_exchanges(2);
    let task_id = task.id().clone();
    let mut store = TaskStore::with_root_task(task);

    let new_exchange = create_test_exchange();
    let new_exchange_id = new_exchange.id;

    let result = store.append_exchange(&task_id, new_exchange);
    assert!(result);
    assert_eq!(store.exchange_count(), 3);

    // Verify the new exchange is accessible
    assert_eq!(store.latest_exchange().map(|e| e.id), Some(new_exchange_id));
}

#[test]
fn test_remove_task_exchange() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let mut store = TaskStore::with_root_task(task);

    // First verify initial state
    assert_eq!(store.exchange_count(), 3);

    // Remove the middle exchange
    let removed = store.remove_task_exchange(&task_id, exchange_ids[1]);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, exchange_ids[1]);

    // After removal, we should have 2 exchanges
    assert_eq!(store.exchange_count(), 2);

    // Verify the remaining exchanges are correct
    let remaining_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(remaining_ids, vec![exchange_ids[0], exchange_ids[2]]);
}

#[test]
fn test_append_exchange_to_nonexistent_task() {
    let task = create_test_task_with_exchanges(1);
    let mut store = TaskStore::with_root_task(task);

    let nonexistent_id = TaskId::new(Uuid::new_v4().to_string());
    let result = store.append_exchange(&nonexistent_id, create_test_exchange());
    assert!(!result);
    assert_eq!(store.exchange_count(), 1);
}

#[test]
fn test_remove_exchange_from_nonexistent_task() {
    let task = create_test_task_with_exchanges(1);
    let exchange_id = task.exchanges().next().unwrap().id;
    let mut store = TaskStore::with_root_task(task);

    let nonexistent_task_id = TaskId::new(Uuid::new_v4().to_string());
    let result = store.remove_task_exchange(&nonexistent_task_id, exchange_id);
    assert!(result.is_none());
    assert_eq!(store.exchange_count(), 1);
}

#[test]
fn test_tasks_iteration() {
    let task1 = create_test_task_with_exchanges(2);
    let task1_id = task1.id().clone();
    let mut store = TaskStore::with_root_task(task1);

    let task2 = create_test_subtask_with_exchanges(1);
    let task2_id = task2.id().clone();
    store.insert(task2);

    let task_ids: HashSet<_> = store.tasks().map(|t| t.id().clone()).collect();
    assert_eq!(task_ids.len(), 2);
    assert!(task_ids.contains(&task1_id));
    assert!(task_ids.contains(&task2_id));
}

#[test]
fn test_multiple_tasks_exchange_order() {
    // Create root task with 1 exchange, then add subagent call exchange
    let root_task = create_test_task_with_exchanges(1);
    let root_task_id = root_task.id().clone();
    let first_root_exchange_id = root_task.exchanges().next().unwrap().id;
    let mut store = TaskStore::with_root_task(root_task);

    // Create subtask with 2 exchanges
    let subtask = create_test_subtask_with_exchanges(2);
    let subtask_id = subtask.id().clone();
    let subtask_exchange_ids: Vec<_> = subtask.exchanges().map(|e| e.id).collect();

    // Add subagent call exchange to root
    let subagent_exchange = create_exchange_with_subagent_call(&subtask_id);
    let subagent_exchange_id = subagent_exchange.id;
    store.append_exchange(&root_task_id, subagent_exchange);

    // Insert subtask - now linked via subagent call
    store.insert(subtask);

    // 1 root + 1 subagent call + 2 subtask = 4 exchanges
    assert_eq!(store.exchange_count(), 4);

    // First/last should still work
    assert_eq!(
        store.first_exchange().map(|e| e.id),
        Some(first_root_exchange_id)
    );
    assert_eq!(
        store.latest_exchange().map(|e| e.id),
        Some(subtask_exchange_ids[1])
    );

    // Order: root[0], root[subagent_call], subtask[0], subtask[1]
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 4);
    assert_eq!(all_ids[0], first_root_exchange_id);
    assert_eq!(all_ids[1], subagent_exchange_id);
    assert_eq!(all_ids[2], subtask_exchange_ids[0]);
    assert_eq!(all_ids[3], subtask_exchange_ids[1]);
}

#[test]
fn test_empty_task_handling() {
    let task = create_test_task_with_exchanges(0);
    let task_id = task.id().clone();
    let store = TaskStore::with_root_task(task);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 0);
    assert!(store.first_exchange().is_none());
    assert!(store.latest_exchange().is_none());
    assert!(store.get(&task_id).is_some());
}

#[test]
fn test_from_tasks() {
    use std::collections::HashMap;

    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();

    let mut tasks = HashMap::new();
    tasks.insert(task_id.clone(), task);

    let store = TaskStore::from_tasks(tasks, task_id.clone());

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);
    assert_eq!(store.root_task_id(), &task_id);

    let actual_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(actual_ids, exchange_ids);
}

#[test]
fn test_exchange_mut() {
    let task = create_test_task_with_exchanges(2);
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let mut store = TaskStore::with_root_task(task);

    // Can find existing exchange
    let exchange = store.exchange_mut(exchange_ids[0]);
    assert!(exchange.is_some());
    assert_eq!(exchange.unwrap().id, exchange_ids[0]);

    // Returns None for non-existent exchange
    let fake_id = AIAgentExchangeId::new();
    assert!(store.exchange_mut(fake_id).is_none());
}

#[test]
fn test_modify_task_conditional_rebuild() {
    let task = create_test_task_with_exchanges(2);
    let task_id = task.id().clone();
    let original_exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let mut store = TaskStore::with_root_task(task);

    // Verify initial state
    assert_eq!(store.exchange_count(), 2);

    // modify_task with no exchange change should still work
    let result = store.modify_task(&task_id, |task| {
        assert_eq!(task.exchanges_len(), 2);
        "no change"
    });
    assert_eq!(result, Some("no change"));
    assert_eq!(store.exchange_count(), 2);

    // modify_task that adds an exchange should update the index
    let new_exchange = create_test_exchange();
    let new_exchange_id = new_exchange.id;
    store.modify_task(&task_id, |task| {
        task.append_exchange(new_exchange);
    });
    assert_eq!(store.exchange_count(), 3);
    assert_eq!(store.latest_exchange().map(|e| e.id), Some(new_exchange_id));

    // Verify all exchanges are in the index
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 3);
    assert_eq!(all_ids[0], original_exchange_ids[0]);
    assert_eq!(all_ids[1], original_exchange_ids[1]);
    assert_eq!(all_ids[2], new_exchange_id);
}

// =============================================================================
// Subtask Linearization Tests
// =============================================================================

#[test]
fn test_linearization_parent_with_one_subtask() {
    // Create a subtask first so we have its ID
    let subtask = create_test_subtask_with_exchanges(2);
    let subtask_id = subtask.id().clone();
    let subtask_exchange_ids: Vec<_> = subtask.exchanges().map(|e| e.id).collect();

    // Create root task: exchange1, exchange_with_subagent_call, exchange3
    let mut root_task = Task::new_optimistic_root();
    let exchange1 = create_test_exchange();
    let exchange1_id = exchange1.id;
    root_task.append_exchange(exchange1);

    let exchange_with_call = create_exchange_with_subagent_call(&subtask_id);
    let exchange_with_call_id = exchange_with_call.id;
    root_task.append_exchange(exchange_with_call);

    let exchange3 = create_test_exchange();
    let exchange3_id = exchange3.id;
    root_task.append_exchange(exchange3);

    // Build the store
    let mut store = TaskStore::with_root_task(root_task);
    store.insert(subtask);

    // Total exchanges: 3 root + 2 subtask = 5
    assert_eq!(store.exchange_count(), 5);

    // Expected order: root[0], root[1] (with subagent call), subtask[0], subtask[1], root[2]
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 5);
    assert_eq!(all_ids[0], exchange1_id);
    assert_eq!(all_ids[1], exchange_with_call_id);
    assert_eq!(all_ids[2], subtask_exchange_ids[0]);
    assert_eq!(all_ids[3], subtask_exchange_ids[1]);
    assert_eq!(all_ids[4], exchange3_id);

    // Verify first and last
    assert_eq!(store.first_exchange().map(|e| e.id), Some(exchange1_id));
    assert_eq!(store.latest_exchange().map(|e| e.id), Some(exchange3_id));
}

#[test]
fn test_linearization_nested_subtasks() {
    // Create nested subtask (grandchild) first
    let grandchild_subtask = create_test_subtask_with_exchanges(1);
    let grandchild_id = grandchild_subtask.id().clone();
    let grandchild_exchange_id = grandchild_subtask.exchanges().next().unwrap().id;

    // Create child subtask with a call to grandchild
    use crate::terminal::model::block::BlockId;
    let mut child_subtask = Task::new_optimistic_cli_agent_subtask(BlockId::new());
    let child_id = child_subtask.id().clone();

    let child_exchange1 = create_test_exchange();
    let child_exchange1_id = child_exchange1.id;
    child_subtask.append_exchange(child_exchange1);

    let child_call_to_grandchild = create_exchange_with_subagent_call(&grandchild_id);
    let child_call_exchange_id = child_call_to_grandchild.id;
    child_subtask.append_exchange(child_call_to_grandchild);

    // Create root task with a call to child
    let mut root_task = Task::new_optimistic_root();

    let root_exchange1 = create_test_exchange();
    let root_exchange1_id = root_exchange1.id;
    root_task.append_exchange(root_exchange1);

    let root_call_to_child = create_exchange_with_subagent_call(&child_id);
    let root_call_exchange_id = root_call_to_child.id;
    root_task.append_exchange(root_call_to_child);

    // Build the store
    let mut store = TaskStore::with_root_task(root_task);
    store.insert(child_subtask);
    store.insert(grandchild_subtask);

    // Total: 2 root + 2 child + 1 grandchild = 5
    assert_eq!(store.exchange_count(), 5);

    // Expected DFS order:
    // root[0], root[1] (calls child) -> child[0], child[1] (calls grandchild) -> grandchild[0]
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 5);
    assert_eq!(all_ids[0], root_exchange1_id);
    assert_eq!(all_ids[1], root_call_exchange_id);
    assert_eq!(all_ids[2], child_exchange1_id);
    assert_eq!(all_ids[3], child_call_exchange_id);
    assert_eq!(all_ids[4], grandchild_exchange_id);
}

#[test]
fn test_linearization_multiple_subtasks_same_parent() {
    // Create two subtasks
    let subtask1 = create_test_subtask_with_exchanges(1);
    let subtask1_id = subtask1.id().clone();
    let subtask1_exchange_id = subtask1.exchanges().next().unwrap().id;

    let subtask2 = create_test_subtask_with_exchanges(2);
    let subtask2_id = subtask2.id().clone();
    let subtask2_exchange_ids: Vec<_> = subtask2.exchanges().map(|e| e.id).collect();

    // Create root task with calls to both subtasks in separate exchanges
    let mut root_task = Task::new_optimistic_root();

    let call_to_subtask1 = create_exchange_with_subagent_call(&subtask1_id);
    let call_to_subtask1_id = call_to_subtask1.id;
    root_task.append_exchange(call_to_subtask1);

    let middle_exchange = create_test_exchange();
    let middle_exchange_id = middle_exchange.id;
    root_task.append_exchange(middle_exchange);

    let call_to_subtask2 = create_exchange_with_subagent_call(&subtask2_id);
    let call_to_subtask2_id = call_to_subtask2.id;
    root_task.append_exchange(call_to_subtask2);

    // Build the store
    let mut store = TaskStore::with_root_task(root_task);
    store.insert(subtask1);
    store.insert(subtask2);

    // Total: 3 root + 1 subtask1 + 2 subtask2 = 6
    assert_eq!(store.exchange_count(), 6);

    // Expected order:
    // root[0] (calls subtask1) -> subtask1[0], root[1], root[2] (calls subtask2) -> subtask2[0], subtask2[1]
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 6);
    assert_eq!(all_ids[0], call_to_subtask1_id);
    assert_eq!(all_ids[1], subtask1_exchange_id);
    assert_eq!(all_ids[2], middle_exchange_id);
    assert_eq!(all_ids[3], call_to_subtask2_id);
    assert_eq!(all_ids[4], subtask2_exchange_ids[0]);
    assert_eq!(all_ids[5], subtask2_exchange_ids[1]);
}

#[test]
fn test_all_exchanges_by_task_with_subtasks() {
    // Create a subtask
    let subtask = create_test_subtask_with_exchanges(2);
    let subtask_id = subtask.id().clone();
    let subtask_exchange_ids: Vec<_> = subtask.exchanges().map(|e| e.id).collect();

    // Create root task with a call to subtask
    let mut root_task = Task::new_optimistic_root();
    let root_id = root_task.id().clone();

    let root_exchange1 = create_test_exchange();
    let root_exchange1_id = root_exchange1.id;
    root_task.append_exchange(root_exchange1);

    let call_to_subtask = create_exchange_with_subagent_call(&subtask_id);
    let call_exchange_id = call_to_subtask.id;
    root_task.append_exchange(call_to_subtask);

    let root_exchange3 = create_test_exchange();
    let root_exchange3_id = root_exchange3.id;
    root_task.append_exchange(root_exchange3);

    // Build the store
    let mut store = TaskStore::with_root_task(root_task);
    store.insert(subtask);

    // Check all_exchanges_by_task grouping
    let by_task = store.all_exchanges_by_task();

    // Should have 3 groups: root[0-1], subtask[0-1], root[2]
    assert_eq!(by_task.len(), 3);

    // First group: root task's first two exchanges
    assert_eq!(by_task[0].0, root_id);
    assert_eq!(by_task[0].1.len(), 2);
    assert_eq!(by_task[0].1[0].id, root_exchange1_id);
    assert_eq!(by_task[0].1[1].id, call_exchange_id);

    // Second group: subtask's exchanges
    assert_eq!(by_task[1].0, subtask_id);
    assert_eq!(by_task[1].1.len(), 2);
    assert_eq!(by_task[1].1[0].id, subtask_exchange_ids[0]);
    assert_eq!(by_task[1].1[1].id, subtask_exchange_ids[1]);

    // Third group: root task's last exchange
    assert_eq!(by_task[2].0, root_id);
    assert_eq!(by_task[2].1.len(), 1);
    assert_eq!(by_task[2].1[0].id, root_exchange3_id);
}
