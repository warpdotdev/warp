use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use warp_multi_agent_api as api;

use super::task::helper::{MessageExt, ToolCallExt};
use super::task::{Task, TaskId};
use super::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputMessageType, MessageId};
use crate::ai::agent::{AIAgentContext, AIAgentInput};
use crate::ai::skills::SkillDescriptor;

#[derive(Debug, Clone)]
struct ExchangeRef {
    task_id: TaskId,
    exchange_index: usize,
}

/// Task storage with a combined ordered exchange index.
///
/// Exchanges are stored in a single [`IndexMap`] keyed by [`AIAgentExchangeId`].
/// The map preserves DFS traversal insertion order, providing both O(1)
/// positional access (first/last) and O(1) lookup by ID in one structure.
/// This eliminates the previous two-field design (`linearized_refs` +
/// `exchange_id_index`) whose synchronization was an invariant that callers
/// had to manually maintain.
#[derive(Debug, Clone)]
pub struct TaskStore {
    root_task_id: TaskId,
    tasks: HashMap<TaskId, Task>,
    /// Linearized (DFS-ordered) map from exchange ID to its location in `tasks`.
    /// Serves as both the ordered exchange list and the O(1) ID lookup index.
    exchanges: IndexMap<AIAgentExchangeId, ExchangeRef>,
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
            exchanges: IndexMap::new(),
            root_task_id: root_task_id.clone(),
            optimistic_root_task_id: None,
        };
        store.tasks.insert(root_task_id, root_task);
        store.rebuild_exchange_index();
        store
    }

    /// Creates a TaskStore from an existing HashMap of tasks.
    /// Rebuilds the exchange index after construction.
    pub fn from_tasks(tasks: HashMap<TaskId, Task>, root_task_id: TaskId) -> Self {
        let mut store = Self {
            tasks,
            exchanges: IndexMap::new(),
            root_task_id,
            optimistic_root_task_id: None,
        };
        store.rebuild_exchange_index();
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

    /// Appends an exchange to a task and rebuilds the exchange index.
    /// Returns true if the task was found and the exchange was appended.
    pub fn append_exchange(&mut self, task_id: &TaskId, exchange: AIAgentExchange) -> bool {
        let Some(task) = self.tasks.get_mut(task_id) else {
            return false;
        };
        task.append_exchange(exchange);
        self.rebuild_exchange_index();
        true
    }

    /// Removes an exchange from a task and rebuilds the exchange index.
    /// Returns the removed exchange if found.
    pub fn remove_task_exchange(
        &mut self,
        task_id: &TaskId,
        exchange_id: AIAgentExchangeId,
    ) -> Option<AIAgentExchange> {
        let task = self.tasks.get_mut(task_id)?;
        let exchange = task.remove_exchange(exchange_id)?;
        self.rebuild_exchange_index();
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

    /// Modifies a task via the provided closure and rebuilds the exchange index if the exchange
    /// count changes.
    pub fn modify_task<R>(
        &mut self,
        task_id: &TaskId,
        f: impl FnOnce(&mut Task) -> R,
    ) -> Option<R> {
        let task = self.tasks.get_mut(task_id)?;
        let exchange_count_before = task.exchanges_len();
        let result = f(task);
        let exchange_count_after = self
            .tasks
            .get(task_id)
            .map(|t| t.exchanges_len())
            .unwrap_or(0);
        if exchange_count_before != exchange_count_after {
            self.rebuild_exchange_index();
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

    pub fn exchange_by_id(&self, exchange_id: AIAgentExchangeId) -> Option<&AIAgentExchange> {
        let exchange_ref = self.exchanges.get(&exchange_id)?;
        self.lookup_exchange(exchange_ref)
    }

    /// Rebuilds the exchange index from scratch. Call this after any mutation
    /// that changes exchange IDs without changing exchange count (e.g.
    /// `reassign_exchange_ids`).
    pub(super) fn rebuild_exchange_index(&mut self) {
        self.exchanges = Self::build_exchange_index(&self.tasks, &self.root_task_id);
    }

    pub fn first_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges
            .get_index(0)
            .and_then(|(_, r)| self.lookup_exchange(r))
    }

    pub fn latest_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges
            .last()
            .and_then(|(_, r)| self.lookup_exchange(r))
    }

    pub fn exchange_count(&self) -> usize {
        self.exchanges.len()
    }

    pub fn all_exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges
            .values()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_rev(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges
            .values()
            .rev()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_by_task(&self) -> Vec<(TaskId, Vec<&AIAgentExchange>)> {
        let mut result: Vec<(TaskId, Vec<&AIAgentExchange>)> = Vec::new();

        for (_, exchange_ref) in &self.exchanges {
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
        self.exchanges.values().rev().find_map(|exchange_ref| {
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
        self.rebuild_exchange_index();
    }

    pub fn remove(&mut self, task_id: &TaskId) -> Option<Task> {
        let task = self.tasks.remove(task_id)?;
        self.rebuild_exchange_index();
        Some(task)
    }

    /// Removes the given message ids from the source of every non-root task.
    ///
    /// Used by conversation rewind: summarization (`MoveMessagesToNewTask`)
    /// relocates rewound root messages into a subtask while the root exchange's
    /// `added_message_ids` still reference them, so a root-only removal would
    /// leave them to be re-sent. Removing source messages does not change the
    /// exchange index, so no rebuild is required.
    pub fn remove_messages_from_non_root_tasks(&mut self, message_ids: &HashSet<MessageId>) {
        if message_ids.is_empty() {
            return;
        }
        let root_task_id = self.root_task_id.clone();
        for (task_id, task) in self.tasks.iter_mut() {
            if *task_id == root_task_id {
                continue;
            }
            task.remove_messages(message_ids);
        }
    }

    /// Removes every non-root task that is no longer reachable from the root by
    /// following sub-agent tool calls (transitively).
    ///
    /// Used after a rewind truncation to drop orphaned subtasks (whose spawning
    /// sub-agent tool call was removed) and straddle subtasks (whose call we
    /// stripped because its result was rewound). Sub-agents whose call survives
    /// in the root remain reachable and are kept, preserving valid history.
    pub fn prune_unreachable_subtasks(&mut self) {
        let reachable = self.reachable_task_ids();
        let to_remove: Vec<TaskId> = self
            .tasks
            .keys()
            .filter(|id| **id != self.root_task_id && !reachable.contains(*id))
            .cloned()
            .collect();
        if to_remove.is_empty() {
            return;
        }
        for id in &to_remove {
            self.tasks.remove(id);
        }
        self.rebuild_exchange_index();
    }

    /// Computes the set of task ids reachable from the root task by following
    /// sub-agent tool calls in task sources (transitively). Unlike
    /// `compute_active_task_ids`, a sub-agent's subtask stays reachable even
    /// after its result arrives, so finished sub-agents remain part of history.
    fn reachable_task_ids(&self) -> HashSet<TaskId> {
        let mut reachable: HashSet<TaskId> = HashSet::new();
        let mut queue = vec![self.root_task_id.clone()];
        while let Some(task_id) = queue.pop() {
            if !reachable.insert(task_id.clone()) {
                continue;
            }
            let Some(task) = self.tasks.get(&task_id) else {
                continue;
            };
            for message in task.messages() {
                if let Some(subagent) = message.tool_call().and_then(|tc| tc.subagent()) {
                    if !subagent.task_id.is_empty() {
                        queue.push(TaskId::new(subagent.task_id.clone()));
                    }
                }
            }
        }
        reachable
    }

    fn lookup_exchange(&self, r: &ExchangeRef) -> Option<&AIAgentExchange> {
        self.tasks
            .get(&r.task_id)?
            .exchanges()
            .nth(r.exchange_index)
    }

    /// Builds a DFS-ordered [`IndexMap`] from exchange ID to its location in `tasks`.
    ///
    /// By building into an `IndexMap` rather than separate `Vec` + `HashMap` fields,
    /// the ordering and the ID index are a single structure with one source of
    /// truth — callers can never observe them out of sync.
    fn build_exchange_index(
        tasks: &HashMap<TaskId, Task>,
        root_task_id: &TaskId,
    ) -> IndexMap<AIAgentExchangeId, ExchangeRef> {
        let mut index = IndexMap::new();

        fn append_refs_for_task(
            tasks: &HashMap<TaskId, Task>,
            index: &mut IndexMap<AIAgentExchangeId, ExchangeRef>,
            task: &Task,
        ) {
            let task_id = task.id().clone();

            for (exchange_index, exchange) in task.exchanges().enumerate() {
                index.insert(
                    exchange.id,
                    ExchangeRef {
                        task_id: task_id.clone(),
                        exchange_index,
                    },
                );

                // Check for subagent calls in the exchange output.
                if let Some(output) = exchange.output_status.output() {
                    for output_message in output.get().messages.iter() {
                        if let AIAgentOutputMessageType::Subagent(subagent_call) =
                            &output_message.message
                        {
                            if let Some(subtask) =
                                tasks.get(&TaskId::new(subagent_call.task_id.clone()))
                            {
                                append_refs_for_task(tasks, index, subtask);
                            }
                        }
                    }
                }
            }
        }

        if let Some(root_task) = tasks.get(root_task_id) {
            append_refs_for_task(tasks, &mut index, root_task);
        }

        index
    }
}

#[cfg(test)]
mod testing {
    use super::TaskStore;
    use crate::ai::agent::task::TaskId;

    impl TaskStore {
        pub fn contains(&self, task_id: &TaskId) -> bool {
            self.tasks.contains_key(task_id)
        }
    }
}

#[cfg(test)]
#[path = "task_store_tests.rs"]
mod tests;
