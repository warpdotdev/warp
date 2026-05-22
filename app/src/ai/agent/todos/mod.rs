use super::AIAgentTodoId;
use crate::ai::agent::AIAgentTodo;
pub(crate) mod popup;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AIAgentTodoList {
    completed_items: Vec<AIAgentTodo>,
    pending_items: Vec<AIAgentTodo>,
}

impl AIAgentTodoList {
    pub fn with_pending_items(mut self, pending_items: Vec<AIAgentTodo>) -> Self {
        self.pending_items = pending_items;
        self
    }

    pub fn with_completed_items(mut self, completed_items: Vec<AIAgentTodo>) -> Self {
        self.completed_items = completed_items;
        self
    }

    pub fn update_pending_items(&mut self, pending_items: Vec<AIAgentTodo>) {
        self.pending_items = pending_items;
    }

    pub fn clear_pending_items(&mut self) {
        self.pending_items.clear();
    }

    pub fn len(&self) -> usize {
        self.pending_items.len() + self.completed_items.len()
    }

    pub fn is_finished(&self) -> bool {
        self.pending_items.is_empty() && !self.completed_items.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.pending_items.is_empty() && self.completed_items.is_empty()
    }

    pub fn in_progress_item(&self) -> Option<&AIAgentTodo> {
        self.pending_items.first()
    }

    pub fn pending_items(&self) -> &[AIAgentTodo] {
        &self.pending_items
    }

    pub fn completed_items(&self) -> &[AIAgentTodo] {
        &self.completed_items
    }

    pub fn is_pending(&self, todo_id: &AIAgentTodoId) -> bool {
        self.pending_items.iter().any(|item| &item.id == todo_id)
    }

    pub fn is_completed(&self, todo_id: &AIAgentTodoId) -> bool {
        self.completed_items.iter().any(|item| &item.id == todo_id)
    }

    pub fn get_item(&self, todo_id: &AIAgentTodoId) -> Option<&AIAgentTodo> {
        self.items().find(|item| &item.id == todo_id)
    }

    pub fn get_item_index(&self, todo_id: &AIAgentTodoId) -> Option<usize> {
        self.items().position(|item| &item.id == todo_id)
    }

    fn items(&self) -> impl Iterator<Item = &AIAgentTodo> {
        self.completed_items.iter().chain(self.pending_items.iter())
    }

    pub fn update_pending_todos(&mut self, todos: Vec<AIAgentTodo>) {
        self.pending_items = todos;
    }

    pub fn mark_todos_complete(&mut self, completed_todo_ids: Vec<String>) {
        for completed_todo_id in completed_todo_ids.into_iter() {
            if let Some(item) = self
                .pending_items
                .iter()
                .position(|item| item.id == completed_todo_id.clone().into())
                .map(|i| self.pending_items.remove(i))
            {
                self.completed_items.push(item);
            }
        }
    }
}
