use std::collections::HashMap;

use warp_multi_agent_api as api;

use crate::ai::{
    agent::{AIAgentContext, AIAgentInput},
    skills::SkillDescriptor,
};

use super::{
    task::{
        helper::{MessageExt, ToolCallExt},
        Task, TaskId,
    },
    AIAgentExchange, AIAgentExchangeId, AIAgentOutputMessageType,
};

#[derive(Debug, Clone)]
struct ExchangeRef {
    task_id: TaskId,
    exchange_index: usize,
}

/// Task storage with a linearized exchange index for O(1) first/last access.
#[derive(Debug, Clone)]
pub struct TaskStore {
    root_task_id: TaskId,
    tasks: HashMap<TaskId, Task>,
    linearized_refs: Vec<ExchangeRef>,
    /// If the root task was upgraded from an optimistic (client-generated) ID
    /// to a server-assigned ID, stores the original optimistic ID so that
    /// deferred event handlers referencing the stale ID can still resolve
    /// the task via `root_task_id`.
    optimistic_root_task_id: Option<TaskId>,
}

impl TaskStore {
    pub fn with_root_task(root_task: Task) -> Self {
        let root_task_id = root_task.id().clone();
        let mut store = Self {
            tasks: HashMap::new(),
            linearized_refs: Vec::new(),
            root_task_id: root_task_id.clone(),
            optimistic_root_task_id: None,
        };
        store.tasks.insert(root_task_id, root_task);
        store.rebuild_linearized_refs_index();
        store
    }

    /// Creates a TaskStore from an existing HashMap of tasks.
    /// Rebuilds the linearized index after construction.
    pub fn from_tasks(tasks: HashMap<TaskId, Task>, root_task_id: TaskId) -> Self {
        let mut store = Self {
            tasks,
            linearized_refs: Vec::new(),
            root_task_id,
            optimistic_root_task_id: None,
        };
        store.rebuild_linearized_refs_index();
        store
    }

    pub fn root_task_id(&self) -> &TaskId {
        &self.root_task_id
    }

    pub fn get(&self, task_id: &TaskId) -> Option<&Task> {
        self.tasks.get(task_id).or_else(|| {
            let old_id = self.optimistic_root_task_id.as_ref()?;
            (old_id == task_id).then(|| self.tasks.get(&self.root_task_id))?
        })
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Appends an exchange to a task and rebuilds the index.
    /// Returns true if the task was found and the exchange was appended.
    pub fn append_exchange(&mut self, task_id: &TaskId, exchange: AIAgentExchange) -> bool {
        let Some(task) = self.tasks.get_mut(task_id) else {
            return false;
        };
        task.append_exchange(exchange);
        self.rebuild_linearized_refs_index();
        true
    }

    /// Removes an exchange from a task and rebuilds the index.
    /// Returns the removed exchange if found.
    pub fn remove_task_exchange(
        &mut self,
        task_id: &TaskId,
        exchange_id: AIAgentExchangeId,
    ) -> Option<AIAgentExchange> {
        let task = self.tasks.get_mut(task_id)?;
        let exchange = task.remove_exchange(exchange_id)?;
        self.rebuild_linearized_refs_index();
        Some(exchange)
    }

    /// Returns a mutable reference to an exchange by its ID, searching all tasks.
    pub fn exchange_mut(&mut self, exchange_id: AIAgentExchangeId) -> Option<&mut AIAgentExchange> {
        for task in self.tasks.values_mut() {
            if let Some(exchange) = task.exchange_mut(exchange_id) {
                return Some(exchange);
            }
        }
        None
    }

    /// Modifies a task via the provided closure and rebuilds the exchange index
    /// if exchanges changed.
    pub fn modify_task<R>(
        &mut self,
        task_id: &TaskId,
        f: impl FnOnce(&mut Task) -> R,
    ) -> Option<R> {
        let exchange_count_before = self.tasks.get(task_id)?.exchanges_len();
        let task = self.tasks.get_mut(task_id)?;
        let result = f(task);
        let exchange_count_after = self
            .tasks
            .get(task_id)
            .map(|t| t.exchanges_len())
            .unwrap_or(0);
        if exchange_count_before != exchange_count_after {
            self.rebuild_linearized_refs_index();
        }
        Some(result)
    }

    /// Modifies the root task via the provided closure and rebuilds the exchange index if exchanges changed.
    pub fn modify_root_task<R>(&mut self, f: impl FnOnce(&mut Task) -> R) -> Option<R> {
        let root_task_id = self.root_task_id.clone();
        self.modify_task(&root_task_id, f)
    }

    pub fn root_task(&self) -> Option<&Task> {
        self.tasks.get(&self.root_task_id)
    }

    /// Sets or replaces the root task, removing any previous root if it exists.
    pub fn set_root_task(&mut self, root_task: Task) {
        // Remove the old root task and its exchange refs
        let old_root_id = self.root_task_id.clone();
        self.remove(&old_root_id);

        let new_root_id = root_task.id().clone();
        if old_root_id != new_root_id {
            self.optimistic_root_task_id = Some(old_root_id);
        }
        self.root_task_id = new_root_id;
        self.insert(root_task);
    }

    pub fn first_exchange(&self) -> Option<&AIAgentExchange> {
        self.linearized_refs
            .first()
            .and_then(|r| self.lookup_exchange(r))
    }

    pub fn latest_exchange(&self) -> Option<&AIAgentExchange> {
        self.linearized_refs
            .last()
            .and_then(|r| self.lookup_exchange(r))
    }

    pub fn exchange_count(&self) -> usize {
        self.linearized_refs.len()
    }

    pub fn all_exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.linearized_refs
            .iter()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_rev(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.linearized_refs
            .iter()
            .rev()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_by_task(&self) -> Vec<(TaskId, Vec<&AIAgentExchange>)> {
        let mut result: Vec<(TaskId, Vec<&AIAgentExchange>)> = Vec::new();

        for exchange_ref in &self.linearized_refs {
            let Some(exchange) = self.lookup_exchange(exchange_ref) else {
                continue;
            };

            // Check if we should append to the last group or start a new one
            if let Some((last_task_id, exchanges)) = result.last_mut() {
                if last_task_id == &exchange_ref.task_id {
                    exchanges.push(exchange);
                    continue;
                }
            }

            // Start a new group
            result.push((exchange_ref.task_id.clone(), vec![exchange]));
        }

        result
    }

    pub fn latest_skills(&self) -> Option<Vec<SkillDescriptor>> {
        self.linearized_refs.iter().rev().find_map(|exchange_ref| {
            let exchange = self.lookup_exchange(exchange_ref);

            if let Some(exchange) = exchange {
                let skills = exchange.input.iter().find_map(|input| {
                    let context = match input {
                        AIAgentInput::UserQuery { context, .. } => Some(context),
                        AIAgentInput::ResumeConversation { context, .. } => Some(context),
                        AIAgentInput::ActionResult { context, .. } => Some(context),
                        AIAgentInput::TriggerPassiveSuggestion { context, .. } => Some(context),
                        _ => None,
                    };

                    context.and_then(|ctx| {
                        ctx.iter().find_map(|context| {
                            if let AIAgentContext::Skills { skills } = context {
                                Some(skills)
                            } else {
                                None
                            }
                        })
                    })
                });

                skills.cloned()
            } else {
                None
            }
        })
    }

    /// Returns all messages in linearized DFS order, interleaving subtask messages
    /// immediately after their parent subagent call messages.
    pub fn all_linearized_messages(&self) -> Vec<&api::Message> {
        fn collect_messages_dfs<'a>(
            me: &'a TaskStore,
            messages: &mut Vec<&'a api::Message>,
            task: &'a Task,
        ) {
            for message in task.messages() {
                messages.push(message);
                // If this message is a subagent call, recursively add subtask messages
                if let Some(subagent_call) = message
                    .tool_call()
                    .and_then(|tc: &api::message::ToolCall| tc.subagent())
                {
                    if let Some(subtask) = me.get(&TaskId::new(subagent_call.task_id.clone())) {
                        collect_messages_dfs(me, messages, subtask);
                    }
                }
            }
        }

        let mut messages = Vec::new();
        if let Some(root_task) = self.root_task() {
            collect_messages_dfs(self, &mut messages, root_task);
        }
        messages
    }

    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id().clone(), task);
        self.rebuild_linearized_refs_index();
    }

    pub fn remove(&mut self, task_id: &TaskId) -> Option<Task> {
        let task = self.tasks.remove(task_id)?;
        self.linearized_refs.retain(|r| &r.task_id != task_id);
        Some(task)
    }

    fn lookup_exchange(&self, r: &ExchangeRef) -> Option<&AIAgentExchange> {
        self.tasks
            .get(&r.task_id)?
            .exchanges()
            .nth(r.exchange_index)
    }

    /// Rebuilds the linearized index from scratch using DFS traversal.
    fn rebuild_linearized_refs_index(&mut self) {
        self.linearized_refs = Self::build_linearized_refs(&self.tasks, &self.root_task_id);
    }

    /// Builds linearized exchange refs via DFS traversal without mutating self.
    /// This allows us to borrow `tasks` immutably throughout the traversal.
    fn build_linearized_refs(
        tasks: &HashMap<TaskId, Task>,
        root_task_id: &TaskId,
    ) -> Vec<ExchangeRef> {
        let mut refs = Vec::new();

        fn append_refs_for_task(
            tasks: &HashMap<TaskId, Task>,
            refs: &mut Vec<ExchangeRef>,
            task: &Task,
        ) {
            let task_id = task.id().clone();

            for (exchange_index, exchange) in task.exchanges().enumerate() {
                refs.push(ExchangeRef {
                    task_id: task_id.clone(),
                    exchange_index,
                });

                // Check for subagent calls in the exchange output.
                if let Some(output) = exchange.output_status.output() {
                    for output_message in output.get().messages.iter() {
                        if let AIAgentOutputMessageType::Subagent(subagent_call) =
                            &output_message.message
                        {
                            if let Some(subtask) =
                                tasks.get(&TaskId::new(subagent_call.task_id.clone()))
                            {
                                append_refs_for_task(tasks, refs, subtask);
                            }
                        }
                    }
                }
            }
        }

        if let Some(root_task) = tasks.get(root_task_id) {
            append_refs_for_task(tasks, &mut refs, root_task);
        }

        refs
    }
}

#[cfg(test)]
mod testing {
    use crate::ai::agent::task::TaskId;

    use super::TaskStore;

    impl TaskStore {
        pub fn contains(&self, task_id: &TaskId) -> bool {
            self.tasks.contains_key(task_id)
        }
    }
}

#[cfg(test)]
#[path = "task_store_tests.rs"]
mod tests;
