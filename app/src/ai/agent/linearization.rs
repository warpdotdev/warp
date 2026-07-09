//! Linearization utilities for task messages.
//!
//! This module provides pure functions for linearizing task messages in a conversation,
//! following a DFS traversal that interleaves subtask messages at subagent tool calls.

use std::collections::{HashMap, HashSet};

use warp_multi_agent_api as api;

use crate::ai::agent::task::helper::TaskExt as _;
use crate::report_error;

/// Computes the set of "active" task IDs in a task tree.
///
/// An active task is one that is still in progress. The algorithm:
/// 1. Start with a queue containing the root task ID
/// 2. For each task in the queue, walk through its messages:
///    - When encountering a subagent ToolCall, add the subtask to the queue
///    - When encountering a ToolCallResult matching a subagent call, remove from queue
/// 3. After processing all messages, add the task to the active set
/// 4. Repeat until the queue is empty
pub fn compute_active_task_ids<'a>(
    root_task_id: &str,
    tasks: &HashMap<&str, &'a api::Task>,
) -> HashSet<&'a str> {
    let mut active_tasks = HashSet::new();
    let mut visited = HashSet::new();
    let mut queue = vec![root_task_id];

    while let Some(task_id) = queue.pop() {
        // Cycle protection: skip tasks we've already processed.
        if !visited.insert(task_id) {
            report_error!(
                "Cycle detected in active task computation",
                extra: { "task_id" => %task_id }
            );
            continue;
        }

        let Some(task) = tasks.get(task_id) else {
            // Task not found - skip it.
            continue;
        };

        // Track subagent tool calls: tool_call_id -> subtask_id.
        let mut pending_subagents: HashMap<&str, &str> = HashMap::new();

        for message in &task.messages {
            match &message.message {
                Some(api::message::Message::ToolCall(tool_call)) => {
                    // Check if this is a subagent call.
                    if let Some(api::message::tool_call::Tool::Subagent(subagent)) = &tool_call.tool
                    {
                        if !subagent.task_id.is_empty() {
                            // Add subtask to the queue.
                            queue.push(subagent.task_id.as_str());
                            // Track this subagent call so we can remove it when we see the result.
                            pending_subagents
                                .insert(tool_call.tool_call_id.as_str(), subagent.task_id.as_str());
                        }
                    }
                }
                Some(api::message::Message::ToolCallResult(result)) => {
                    // If this result matches a pending subagent call, remove from queue.
                    if let Some(subtask_id) = pending_subagents.remove(result.tool_call_id.as_str())
                    {
                        queue.retain(|id| *id != subtask_id);
                    }
                }
                _ => {}
            }
        }

        // After processing all messages, add this task to the active set.
        active_tasks.insert(task.id.as_str());
    }

    active_tasks
}

/// Computes the depth (distance from root) for each task in the map.
///
/// Tasks with no parent have depth 0. Tasks whose parent chain contains a cycle or leads to a
/// missing task are assigned depth 0.
pub fn compute_task_depths(tasks: &HashMap<String, api::Task>) -> HashMap<&str, usize> {
    let mut depths = HashMap::new();
    for (task_id, _) in tasks.iter() {
        let mut depth = 0;
        let mut current_id: &str = task_id;
        let mut visited = HashSet::new();
        while let Some(task) = tasks.get(current_id) {
            if !visited.insert(current_id) {
                // Cycle detected; treat as depth 0.
                report_error!(
                    "Cycle detected in task parent chain",
                    extra: { "task_id" => %task_id }
                );
                depth = 0;
                break;
            }
            if let Some(parent_id) = task.parent_id() {
                depth += 1;
                current_id = parent_id;
            } else {
                break;
            }
        }
        depths.insert(task_id.as_str(), depth);
    }
    depths
}

#[cfg(test)]
#[path = "linearization_tests.rs"]
mod tests;
