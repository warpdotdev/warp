use std::collections::HashMap;

use super::Task;
use crate::ai::agent::task::TaskId;

/// Keeps track of the state of tasks before they are modified.
/// Messages are assumed to be only updated during the same transaction
/// in which they were added, so we can clean up message by simply
/// deleting them.
#[derive(Debug, Clone)]
pub struct Transaction {
    saved_tasks: HashMap<TaskId, SavedTask>,
}

/// Saves state for either a newly added task or a pre-existing task
/// modified during a transaction.
#[derive(Debug, Clone)]
pub enum SavedTask {
    New(TaskId),
    Existing(Box<Task>),
}

impl Transaction {
    pub fn new() -> Self {
        Self {
            saved_tasks: HashMap::new(),
        }
    }

    /// A map of the tasks modified in this transaction.
    pub fn saved_tasks(self) -> HashMap<TaskId, SavedTask> {
        self.saved_tasks
    }

    /// Saves a SavedTask::New to the transaction, representing a newly added task.
    pub fn checkpoint_new_task(&mut self, task_id: &TaskId) {
        if !self.saved_tasks.contains_key(task_id) {
            let task = SavedTask::New(task_id.clone());
            self.saved_tasks.insert(task_id.clone(), task);
        }
    }

    /// Saves a SavedTask::Existing to the transaction, representing an existing
    /// task which is being modified.
    pub fn checkpoint_task(&mut self, task: &Task) {
        if !self.saved_tasks.contains_key(task.id()) {
            self.saved_tasks.insert(
                task.id().clone(),
                SavedTask::Existing(Box::new(task.clone())),
            );
        }
    }
}
