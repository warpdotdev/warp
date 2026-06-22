use std::collections::HashMap;

use warp_multi_agent_api as api;

use super::*;

// Helper function to create a basic message
fn create_message(id: &str, task_id: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: "server_data".to_string(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: format!("Message content for {id}"),
            },
        )),
        request_id: String::new(),
        timestamp: None,
    }
}

fn create_subagent_tool_call_message(id: &str, task_id: &str, subtask_id: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: "server_data".to_string(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: format!("{id}_tool_call"),
            tool: Some(api::message::tool_call::Tool::Subagent(
                api::message::tool_call::Subagent {
                    task_id: subtask_id.to_string(),
                    payload: String::new(),
                    metadata: None,
                },
            )),
        })),
        request_id: String::new(),
        timestamp: None,
    }
}

// Helper function to create a tool call result message.
fn create_tool_call_result_message(id: &str, task_id: &str, tool_call_id: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: task_id.to_string(),
        server_message_data: "server_data".to_string(),
        citations: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: tool_call_id.to_string(),
                context: None,
                result: None,
            },
        )),
        request_id: String::new(),
        timestamp: None,
    }
}

// Helper function to create a task with dependencies
fn create_task(id: &str, messages: Vec<api::Message>, parent_task_id: Option<String>) -> api::Task {
    let dependencies = parent_task_id.map(|parent_id| api::task::Dependencies {
        parent_task_id: parent_id,
    });

    api::Task {
        id: id.to_string(),
        messages,
        dependencies,
        description: format!("Task {id}"),
        summary: format!("Summary for task {id}"),
        server_data: "".to_string(),
    }
}

// Helper function to create a root task (no parent)
fn create_root_task(id: &str, messages: Vec<api::Message>) -> api::Task {
    create_task(id, messages, None)
}

// Helper function to create a child task
fn create_child_task(id: &str, messages: Vec<api::Message>, parent_id: &str) -> api::Task {
    create_task(id, messages, Some(parent_id.to_string()))
}

// Helper to build a task map from a slice of tasks.
fn make_task_map(tasks: &[api::Task]) -> HashMap<&str, &api::Task> {
    tasks.iter().map(|t| (t.id.as_str(), t)).collect()
}

#[test]
fn test_compute_active_task_ids_single_root() {
    let root = create_root_task("root", vec![create_message("m1", "root")]);
    let tasks = vec![root];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    assert_eq!(active.len(), 1);
    assert!(active.contains("root"));
}

#[test]
fn test_compute_active_task_ids_subagent_in_progress() {
    // Root calls a subagent but has not received the result yet.
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child"),
    ];
    let child_messages = vec![create_message("child_m1", "child")];

    let root = create_root_task("root", root_messages);
    let child = create_child_task("child", child_messages, "root");
    let tasks = vec![root, child];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // Both root and child are active.
    assert_eq!(active.len(), 2);
    assert!(active.contains("root"));
    assert!(active.contains("child"));
}

#[test]
fn test_compute_active_task_ids_subagent_completed() {
    // Root calls a subagent and has received the result.
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child"),
        create_tool_call_result_message("result1", "root", "call1_tool_call"),
        create_message("m2", "root"),
    ];
    let child_messages = vec![create_message("child_m1", "child")];

    let root = create_root_task("root", root_messages);
    let child = create_child_task("child", child_messages, "root");
    let tasks = vec![root, child];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // Only root is active - child completed.
    assert_eq!(active.len(), 1);
    assert!(active.contains("root"));
}

#[test]
fn test_compute_active_task_ids_nested_subagents() {
    // root -> child (in progress) -> grandchild (in progress)
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child"),
    ];
    let child_messages = vec![
        create_message("child_m1", "child"),
        create_subagent_tool_call_message("call2", "child", "grandchild"),
    ];
    let grandchild_messages = vec![create_message("gc_m1", "grandchild")];

    let root = create_root_task("root", root_messages);
    let child = create_child_task("child", child_messages, "root");
    let grandchild = create_child_task("grandchild", grandchild_messages, "child");
    let tasks = vec![root, child, grandchild];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // All three are active.
    assert_eq!(active.len(), 3);
    assert!(active.contains("root"));
    assert!(active.contains("child"));
    assert!(active.contains("grandchild"));
}

#[test]
fn test_compute_active_task_ids_nested_subagents_partial_completion() {
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child"),
    ];
    let child_messages = vec![
        create_message("child_m1", "child"),
        create_subagent_tool_call_message("call2", "child", "grandchild"),
        create_tool_call_result_message("result2", "child", "call2_tool_call"),
    ];
    let grandchild_messages = vec![create_message("gc_m1", "grandchild")];

    let root = create_root_task("root", root_messages);
    let child = create_child_task("child", child_messages, "root");
    let grandchild = create_child_task("grandchild", grandchild_messages, "child");
    let tasks = vec![root, child, grandchild];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // Grandchild completed, but root and its child are still running.
    assert_eq!(active.len(), 2);
    assert!(active.contains("root"));
    assert!(active.contains("child"));
}

#[test]
fn test_compute_active_task_ids_multiple_parallel_subagents() {
    // Root calls two subagents, one completed and one in progress.
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child1"),
        create_subagent_tool_call_message("call2", "root", "child2"),
        create_tool_call_result_message("result1", "root", "call1_tool_call"),
    ];
    let child1_messages = vec![create_message("c1_m1", "child1")];
    let child2_messages = vec![create_message("c2_m1", "child2")];

    let root = create_root_task("root", root_messages);
    let child1 = create_child_task("child1", child1_messages, "root");
    let child2 = create_child_task("child2", child2_messages, "root");
    let tasks = vec![root, child1, child2];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // Root and child2 are active, child1 is completed.
    assert_eq!(active.len(), 2);
    assert!(active.contains("root"));
    assert!(active.contains("child2"));
}

#[test]
fn test_compute_active_task_ids_missing_subtask() {
    // Root calls a subagent that doesn't exist in the task list.
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "nonexistent"),
    ];

    let root = create_root_task("root", root_messages);
    let tasks = vec![root];
    let tasks = make_task_map(&tasks);

    let active = compute_active_task_ids("root", &tasks);

    // Only root is active - the missing subtask is skipped.
    assert_eq!(active.len(), 1);
    assert!(active.contains("root"));
}

#[test]
fn test_compute_active_task_ids_missing_root() {
    // Root task ID doesn't exist in the map.
    let tasks: HashMap<&str, &api::Task> = HashMap::new();

    let active = compute_active_task_ids("nonexistent", &tasks);

    assert!(active.is_empty());
}

#[test]
fn test_compute_active_task_ids_cycle_protection() {
    // Create a cycle: root -> child -> root (via subagent call).
    // This should not cause an infinite loop.
    let root_messages = vec![
        create_message("m1", "root"),
        create_subagent_tool_call_message("call1", "root", "child"),
    ];
    let child_messages = vec![
        create_message("child_m1", "child"),
        // Child calls back to root, creating a cycle.
        create_subagent_tool_call_message("call2", "child", "root"),
    ];

    let root = create_root_task("root", root_messages);
    let child = create_child_task("child", child_messages, "root");
    let tasks = vec![root, child];
    let tasks = make_task_map(&tasks);

    // This should complete without infinite looping.
    let active = compute_active_task_ids("root", &tasks);

    // Both root and child are active (cycle is broken by visited check).
    assert_eq!(active.len(), 2);
    assert!(active.contains("root"));
    assert!(active.contains("child"));
}

// ============================================================================
// compute_task_depths tests
// ============================================================================

/// Creates a task with the given ID and optional parent, without any messages.
fn create_task_for_depth(id: &str, parent_id: Option<&str>) -> api::Task {
    create_task(id, vec![], parent_id.map(str::to_string))
}

#[test]
fn test_compute_task_depths_empty() {
    let tasks = HashMap::new();
    let depths = compute_task_depths(&tasks);
    assert!(depths.is_empty());
}

#[test]
fn test_compute_task_depths_single_root() {
    let tasks: HashMap<String, _> =
        [("root".to_string(), create_task_for_depth("root", None))].into();

    let depths = compute_task_depths(&tasks);

    assert_eq!(depths.get("root"), Some(&0));
}

#[test]
fn test_compute_task_depths_linear_chain() {
    // root -> child -> grandchild
    let tasks: HashMap<String, _> = [
        ("root".to_string(), create_task_for_depth("root", None)),
        (
            "child".to_string(),
            create_task_for_depth("child", Some("root")),
        ),
        (
            "grandchild".to_string(),
            create_task_for_depth("grandchild", Some("child")),
        ),
    ]
    .into();

    let depths = compute_task_depths(&tasks);

    assert_eq!(depths.get("root"), Some(&0));
    assert_eq!(depths.get("child"), Some(&1));
    assert_eq!(depths.get("grandchild"), Some(&2));
}

#[test]
fn test_compute_task_depths_tree_structure() {
    // root -> child1 -> grandchild1
    //      -> child2
    let tasks: HashMap<String, _> = [
        ("root".to_string(), create_task_for_depth("root", None)),
        (
            "child1".to_string(),
            create_task_for_depth("child1", Some("root")),
        ),
        (
            "child2".to_string(),
            create_task_for_depth("child2", Some("root")),
        ),
        (
            "grandchild1".to_string(),
            create_task_for_depth("grandchild1", Some("child1")),
        ),
    ]
    .into();

    let depths = compute_task_depths(&tasks);

    assert_eq!(depths.get("root"), Some(&0));
    assert_eq!(depths.get("child1"), Some(&1));
    assert_eq!(depths.get("child2"), Some(&1));
    assert_eq!(depths.get("grandchild1"), Some(&2));
}

#[test]
fn test_compute_task_depths_orphan() {
    // Task with parent that doesn't exist in the map.
    let tasks: HashMap<String, _> = [(
        "orphan".to_string(),
        create_task_for_depth("orphan", Some("missing_parent")),
    )]
    .into();

    let depths = compute_task_depths(&tasks);

    // Orphan's parent is missing, so the chain breaks immediately after computing depth 1.
    assert_eq!(depths.get("orphan"), Some(&1));
}

#[test]
fn test_compute_task_depths_cycle_two_tasks() {
    // a -> b -> a (cycle)
    let tasks: HashMap<String, _> = [
        ("a".to_string(), create_task_for_depth("a", Some("b"))),
        ("b".to_string(), create_task_for_depth("b", Some("a"))),
    ]
    .into();

    let depths = compute_task_depths(&tasks);

    // Both tasks are in a cycle, so they should get depth 0.
    assert_eq!(depths.get("a"), Some(&0));
    assert_eq!(depths.get("b"), Some(&0));
}

#[test]
fn test_compute_task_depths_self_referential_cycle() {
    // a -> a (self-referential)
    let tasks: HashMap<String, _> =
        [("a".to_string(), create_task_for_depth("a", Some("a")))].into();

    let depths = compute_task_depths(&tasks);

    // Self-referential task should get depth 0.
    assert_eq!(depths.get("a"), Some(&0));
}

#[test]
fn test_compute_task_depths_cycle_with_tail() {
    // Task c points to a cycle: c -> a -> b -> a
    let tasks: HashMap<String, _> = [
        ("a".to_string(), create_task_for_depth("a", Some("b"))),
        ("b".to_string(), create_task_for_depth("b", Some("a"))),
        ("c".to_string(), create_task_for_depth("c", Some("a"))),
    ]
    .into();

    let depths = compute_task_depths(&tasks);

    // a and b are in a cycle, so depth 0.
    assert_eq!(depths.get("a"), Some(&0));
    assert_eq!(depths.get("b"), Some(&0));
    // c's parent chain leads into a cycle, so it also gets depth 0.
    assert_eq!(depths.get("c"), Some(&0));
}
