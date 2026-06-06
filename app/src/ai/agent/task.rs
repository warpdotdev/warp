pub mod helper;
pub mod transaction;

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::ops::Deref;

use ai::skills::SkillPathOrigin;
use field_mask::{FieldMaskError, FieldMaskOperation};
use helper::{MessageExt, SubagentExt, ToolCallExt};
use itertools::Itertools;
use prost_types::FieldMask;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_multi_agent_api::message::tool_call::subagent::Metadata;
use warp_multi_agent_api::message::Message;
use warp_multi_agent_api::{self as api};

use super::api::convert_conversation::convert_tool_call_result_to_input;
use super::api::{
    user_inputs_from_messages, ConversionParams, ConvertAPIMessageToClientOutputMessage,
};
use super::comment::CodeReview;
use super::conversation::{context_in_exchanges, update_todo_list_from_todo_op};
use super::{
    AIAgentContext, AIAgentExchange, AIAgentExchangeId, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputStatus, MaybeAIAgentOutputMessage, MessageId, MessageToAIAgentOutputMessageError,
    Shared,
};
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::terminal::model::block::BlockId;
use crate::AIAgentTodoList;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(id: String) -> Self {
        TaskId(id)
    }
}

impl From<TaskId> for String {
    fn from(id: TaskId) -> Self {
        id.0
    }
}

impl Deref for TaskId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateTaskError {
    #[error("Task never initialized with CreateTask client action.")]
    TaskNotInitialized,
    #[error("Message not found")]
    MessageNotFound,
    #[error("Field mask operation failed: {0:#}")]
    FieldMask(#[from] FieldMaskError),
    #[error("Exchange not found.")]
    ExchangeNotFound,
    #[error("Attempted to update already-finished output.")]
    OutputAlreadyFinished,
    #[error("Attempted to update output that was never initialized.")]
    OutputNeverInitialized,
    #[error("Failed to convert API message to client type: {0}")]
    ConversionError(#[from] MessageToAIAgentOutputMessageError),
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractMessagesError {
    #[error("Task never initialized with CreateTask client action.")]
    TaskNotInitialized,
    #[error("First message not found: {0}")]
    FirstMessageNotFound(String),
    #[error("Last message not found: {0}")]
    LastMessageNotFound(String),
    #[error("Invalid range: first message appears after last message")]
    InvalidRange,
    #[error("Checksum mismatch: expected {expected} messages, found {actual}")]
    ChecksumMismatch { expected: u32, actual: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum UpgradeOptimisticTaskError {
    #[error("Attempted to upgrade optimistic root task with parent.")]
    RootWithUnexpectedParent,
    #[error("Attempted to upgrade optimistic CLI subagent task with no parent.")]
    CLISubagentMissingParent,
    #[error(
        "Attempted to upgrade optimistic CLI subagent task for subtask with no CLI subagent call."
    )]
    CLISubagentMissingSubagentCall,
    #[error("Attempted to upgrade task with server data.")]
    UnexpectedUpgrade,
}

#[derive(Debug, Clone)]
pub(super) struct SubagentParams {
    pub(super) tool_call_id: String,
    pub(super) call: api::message::tool_call::Subagent,
}

#[derive(Debug, Clone)]
struct ServerTask {
    source: api::Task,
    subagent_params: Option<SubagentParams>,
}

mod optimistic {
    use crate::terminal::model::block::BlockId;

    #[derive(Debug, Clone)]
    pub(super) struct CLIAgentSubtask {
        pub(super) block_id: BlockId,
    }

    #[derive(Debug, Clone)]
    pub(super) enum Task {
        Root,
        CLIAgent(CLIAgentSubtask),
    }

    impl Task {
        pub(super) fn is_root(&self) -> bool {
            matches!(self, Task::Root)
        }

        pub(super) fn is_cli_subagent(&self) -> bool {
            matches!(self, Task::CLIAgent(..))
        }
    }
}
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum TaskImpl {
    Server(ServerTask),
    Optimistic(optimistic::Task),
}

impl TaskImpl {
    fn server_data(&self) -> Option<&ServerTask> {
        match &self {
            TaskImpl::Server(data) => Some(data),
            TaskImpl::Optimistic(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    id: TaskId,
    data: TaskImpl,
    /// List of `AIAgentExchange`s corresponding to messages contained in this task.
    exchanges: Vec<AIAgentExchange>,
}
#[derive(Clone, Copy)]
pub(super) struct TaskMessageContext<'a> {
    pub(super) current_todo_list: Option<&'a AIAgentTodoList>,
    pub(super) active_code_review: Option<&'a CodeReview>,
    pub(super) skill_path_origin: &'a SkillPathOrigin,
}

impl Task {
    pub(super) fn new_optimistic_root() -> Self {
        Self {
            id: TaskId::new(Uuid::new_v4().to_string()),
            data: TaskImpl::Optimistic(optimistic::Task::Root),
            exchanges: vec![],
        }
    }

    pub(super) fn new_optimistic_cli_agent_subtask(block_id: BlockId) -> Self {
        Self {
            id: TaskId::new(Uuid::new_v4().to_string()),
            data: TaskImpl::Optimistic(optimistic::Task::CLIAgent(optimistic::CLIAgentSubtask {
                block_id,
            })),
            exchanges: vec![],
        }
    }

    #[allow(clippy::unwrap_in_result)]
    pub(super) fn into_server_created_task(
        mut self,
        task: api::Task,
        parent_task: Option<&api::Task>,
        current_todo_list: Option<&AIAgentTodoList>,
        active_code_review: Option<&CodeReview>,
        skill_path_origin: &SkillPathOrigin,
    ) -> Result<Self, UpgradeOptimisticTaskError> {
        match self.data {
            TaskImpl::Optimistic(optimistic::Task::Root) => {
                if parent_task.is_some() {
                    return Err(UpgradeOptimisticTaskError::RootWithUnexpectedParent);
                }
                self.id = TaskId::new(task.id.clone());
                self.data = TaskImpl::Server(ServerTask {
                    source: task,
                    subagent_params: None,
                })
            }
            TaskImpl::Optimistic(optimistic::Task::CLIAgent(_)) => {
                let Some(parent_task) = parent_task else {
                    return Err(UpgradeOptimisticTaskError::CLISubagentMissingParent);
                };

                let Some((subagent_call, subagent_tool_call_id)) =
                    parent_task.messages.iter().find_map(|message| {
                        let tool_call = message.tool_call()?;
                        let subagent_call = tool_call.subagent()?;
                        (subagent_call.task_id == task.id && subagent_call.is_cli())
                            .then(|| (subagent_call.clone(), tool_call.tool_call_id.clone()))
                    })
                else {
                    return Err(UpgradeOptimisticTaskError::CLISubagentMissingSubagentCall);
                };

                self.id = TaskId::new(task.id.clone());
                self.data = TaskImpl::Server(ServerTask {
                    source: task,
                    subagent_params: Some(SubagentParams {
                        call: subagent_call,
                        tool_call_id: subagent_tool_call_id,
                    }),
                })
            }
            TaskImpl::Server(_) => return Err(UpgradeOptimisticTaskError::UnexpectedUpgrade),
        };

        let messages = self.source().expect("exists").messages.clone();
        if let Some(exchange_id) = self.exchanges.last().map(|exchange| exchange.id) {
            if let Err(e) = self.update_exchange_from_messages(
                messages,
                exchange_id,
                TaskMessageContext {
                    current_todo_list,
                    active_code_review,
                    skill_path_origin,
                },
                false,
            ) {
                log::error!(
                    "Failed to update last exchange from messages upon converting to a server created task: {e:?}"
                );
            }
        }
        Ok(self)
    }

    pub(super) fn new_restored_root(
        task: api::Task,
        restored_exchanges: impl Iterator<Item = AIAgentExchange>,
    ) -> Self {
        let mut restored_exchanges = restored_exchanges.collect_vec();
        restored_exchanges.sort_by_key(|exchange| exchange.start_time);

        Self {
            id: TaskId(task.id.clone()),
            data: TaskImpl::Server(ServerTask {
                source: task,
                subagent_params: None,
            }),
            exchanges: restored_exchanges,
        }
    }

    pub(super) fn new_subtask(
        subtask: api::Task,
        parent_task: &api::Task,
        existing_exchange: &AIAgentExchange,
        current_todo_list: Option<&AIAgentTodoList>,
        active_code_review: Option<&CodeReview>,
        skill_path_origin: &SkillPathOrigin,
        should_convert_input_messages: bool,
    ) -> Self {
        let subagent_call_and_id = parent_task.messages.iter().find_map(|message| {
            let tool_call = message.tool_call()?;
            let subagent_call = tool_call.subagent()?;
            (subagent_call.task_id == subtask.id)
                .then(|| (subagent_call.clone(), tool_call.tool_call_id.clone()))
        });

        let mut new_exchange = AIAgentExchange {
            id: AIAgentExchangeId::new(),
            input: vec![],
            output_status: AIAgentOutputStatus::Streaming { output: None },
            added_message_ids: Default::default(),
            start_time: chrono::Local::now(),
            finish_time: None,
            time_to_first_token_ms: None,
            working_directory: existing_exchange.working_directory.clone(),
            model_id: existing_exchange.model_id.clone(),
            coding_model_id: existing_exchange.coding_model_id.clone(),
            cli_agent_model_id: existing_exchange.cli_agent_model_id.clone(),
            computer_use_model_id: existing_exchange.computer_use_model_id.clone(),
            request_cost: None,
            response_initiator: existing_exchange.response_initiator.clone(),
        };
        new_exchange
            .init_output(
                existing_exchange
                    .output_status
                    .output()
                    .expect("exists")
                    .get()
                    .server_output_id
                    .clone()
                    .expect("has output id"),
            )
            .expect("Exchange output is in streaming state.");

        let messages_clone = subtask.messages.clone();
        let new_exchange_id = new_exchange.id;
        let mut me = Self {
            id: TaskId(subtask.id.clone()),
            exchanges: vec![new_exchange],
            data: TaskImpl::Server(ServerTask {
                source: subtask,
                subagent_params: subagent_call_and_id
                    .map(|(call, tool_call_id)| SubagentParams { call, tool_call_id }),
            }),
        };
        me.update_exchange_from_messages(
            messages_clone,
            new_exchange_id,
            TaskMessageContext {
                current_todo_list,
                active_code_review,
                skill_path_origin,
            },
            should_convert_input_messages,
        )
        .expect("Exchange exists and output is in 'streaming' state.");
        me
    }

    pub(super) fn new_restored_subtask(
        subtask: api::Task,
        parent_task: &api::Task,
        restored_exchanges: Vec<AIAgentExchange>,
    ) -> Self {
        let subagent_call_and_id = parent_task.messages.iter().find_map(|message| {
            let tool_call = message.tool_call()?;
            let subagent_call = tool_call.subagent()?;
            (subagent_call.task_id == subtask.id)
                .then(|| (subagent_call.clone(), tool_call.tool_call_id.clone()))
        });

        Self {
            id: TaskId(subtask.id.clone()),
            exchanges: restored_exchanges,
            data: TaskImpl::Server(ServerTask {
                source: subtask,
                subagent_params: subagent_call_and_id
                    .map(|(call, tool_call_id)| SubagentParams { call, tool_call_id }),
            }),
        }
    }

    /// Creates a new subtask from an api::Task and the parent task for moved messages.
    ///
    /// This is used by `MoveMessagesToNewTask` to create a task for holding moved
    /// messages. The parent_task should already contain the replacement messages
    /// (including the subagent call referencing this subtask) so that we can look
    /// up the subagent_params.
    pub(super) fn new_moved_messages_subtask(subtask: api::Task, parent_task: &api::Task) -> Self {
        let subagent_call_and_id = parent_task.messages.iter().find_map(|message| {
            let tool_call = message.tool_call()?;
            let subagent_call = tool_call.subagent()?;
            (subagent_call.task_id == subtask.id)
                .then(|| (subagent_call.clone(), tool_call.tool_call_id.clone()))
        });

        Self {
            id: TaskId(subtask.id.clone()),
            exchanges: vec![],
            data: TaskImpl::Server(ServerTask {
                source: subtask,
                subagent_params: subagent_call_and_id
                    .map(|(call, tool_call_id)| SubagentParams { call, tool_call_id }),
            }),
        }
    }

    pub(super) fn subagent_params(&self) -> Option<&SubagentParams> {
        self.data
            .server_data()
            .and_then(|data| data.subagent_params.as_ref())
    }

    pub fn cli_subagent_block_id(&self) -> Option<BlockId> {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .map(|params| &params.call)
                .and_then(|call| match &call.metadata {
                    Some(Metadata::Cli(call)) => Some(call.command_id.clone().into()),
                    Some(Metadata::Research(_)) => None,
                    Some(Metadata::Advice(_)) => None,
                    Some(Metadata::ComputerUse(_)) => None,
                    Some(Metadata::Summarization(_)) => None,
                    Some(Metadata::ConversationSearch(_)) => None,
                    Some(Metadata::WarpDocumentationSearch(_)) => None,
                    None => None,
                }),
            TaskImpl::Optimistic(optimistic::Task::CLIAgent(subtask)) => {
                Some(subtask.block_id.clone())
            }
            TaskImpl::Optimistic(optimistic::Task::Root) => None,
        }
    }

    pub(super) fn append_new_exchange(
        &mut self,
        existing_exchange: &AIAgentExchange,
    ) -> AIAgentExchangeId {
        let mut new_exchange = AIAgentExchange {
            id: AIAgentExchangeId::new(),
            input: vec![],
            output_status: AIAgentOutputStatus::Streaming { output: None },
            added_message_ids: Default::default(),
            start_time: chrono::Local::now(),
            finish_time: None,
            time_to_first_token_ms: None,
            working_directory: existing_exchange.working_directory.clone(),
            model_id: existing_exchange.model_id.clone(),
            coding_model_id: existing_exchange.coding_model_id.clone(),
            cli_agent_model_id: existing_exchange.cli_agent_model_id.clone(),
            computer_use_model_id: existing_exchange.computer_use_model_id.clone(),
            request_cost: None,
            response_initiator: existing_exchange.response_initiator.clone(),
        };
        new_exchange
            .init_output(
                existing_exchange
                    .output_status
                    .output()
                    .expect("exists")
                    .get()
                    .server_output_id
                    .clone()
                    .expect("has output id"),
            )
            .expect("Output is initialized as streaming.");

        let new_exchange_id = new_exchange.id;
        self.exchanges.push(new_exchange);
        new_exchange_id
    }

    pub fn id(&self) -> &TaskId {
        &self.id
    }

    pub fn parent_id(&self) -> Option<TaskId> {
        self.source()
            .and_then(|source| source.dependencies.as_ref())
            .map(|dependencies| TaskId(dependencies.parent_task_id.clone()))
    }

    pub fn is_root_task(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .source
                .dependencies
                .as_ref()
                .is_none_or(|deps| deps.parent_task_id.is_empty()),
            TaskImpl::Optimistic(task) => task.is_root(),
        }
    }

    pub fn is_cli_subagent(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .is_some_and(|params| params.call.is_cli()),
            TaskImpl::Optimistic(task) => task.is_cli_subagent(),
        }
    }

    pub fn is_advice_subagent(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .is_some_and(|params| params.call.is_advice()),
            TaskImpl::Optimistic(_) => false,
        }
    }

    pub fn is_computer_use_subagent(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .is_some_and(|params| params.call.is_computer_use()),
            TaskImpl::Optimistic(_) => false,
        }
    }

    pub fn is_conversation_search_subagent(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .is_some_and(|params| params.call.is_conversation_search()),
            TaskImpl::Optimistic(_) => false,
        }
    }

    pub fn is_warp_documentation_search_subagent(&self) -> bool {
        match &self.data {
            TaskImpl::Server(server_data) => server_data
                .subagent_params
                .as_ref()
                .is_some_and(|params| params.call.is_warp_documentation_search()),
            TaskImpl::Optimistic(_) => false,
        }
    }

    pub fn description(&self) -> &str {
        self.source()
            .map(|source| source.description.as_str())
            .unwrap_or("")
    }

    pub fn exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges.iter()
    }

    pub fn exchange(&self, exchange_id: AIAgentExchangeId) -> Option<&AIAgentExchange> {
        self.exchanges
            .iter()
            .find(|exchange| exchange.id == exchange_id)
    }

    pub(super) fn exchange_mut(
        &mut self,
        exchange_id: AIAgentExchangeId,
    ) -> Option<&mut AIAgentExchange> {
        self.exchanges
            .iter_mut()
            .find(|exchange| exchange.id == exchange_id)
    }

    pub fn last_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges.last()
    }

    pub fn exchanges_len(&self) -> usize {
        self.exchanges.len()
    }

    pub fn exchanges_reversed(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges.iter().rev()
    }

    pub fn source(&self) -> Option<&api::Task> {
        self.try_get_source().ok()
    }

    pub(super) fn source_for_persistence(&self) -> Option<api::Task> {
        match &self.data {
            TaskImpl::Server(server_data) => Some(server_data.source.clone()),
            // Optimistic root tasks have a client-generated UUID and no
            // server-side identity yet. Persisting a stub `api::Task` for them
            // produces an orphan row in `agent_tasks` that survives the later
            // server-side upgrade and breaks restore by competing with the
            // real server root for parentless-task selection. See QUALITY-774.
            TaskImpl::Optimistic(optimistic::Task::Root) => None,
            TaskImpl::Optimistic(optimistic::Task::CLIAgent(_)) => None,
        }
    }

    pub fn messages(&self) -> impl Iterator<Item = &api::Message> {
        self.source()
            .into_iter()
            .flat_map(|source| source.messages.iter())
    }

    /// Returns all the `AIAgentContext` objects attached messages in this conversation.
    pub fn all_contexts(&self) -> impl Iterator<Item = &AIAgentContext> {
        context_in_exchanges(self.exchanges())
    }

    pub fn initial_working_directory(&self) -> Option<String> {
        self.source()
            .and_then(Self::api_task_initial_working_directory)
    }

    pub fn api_task_initial_working_directory(task: &api::Task) -> Option<String> {
        task.messages
            .iter()
            .find_map(|message| {
                message.message.as_ref().and_then(|content| {
                    let context = match content {
                        Message::UserQuery(user_query) => user_query.context.as_ref(),
                        Message::ToolCallResult(tool_call_result) => {
                            tool_call_result.context.as_ref()
                        }
                        Message::SystemQuery(system_query) => system_query.context.as_ref(),
                        _ => None,
                    };

                    context
                        .and_then(|ctx| ctx.directory.as_ref())
                        .map(|dir| dir.pwd.clone())
                })
            })
            .filter(|pwd| !pwd.is_empty())
    }

    pub(super) fn update_description(&mut self, description: String) {
        let Ok(source) = self.try_get_source_mut() else {
            return;
        };
        source.description = description;
    }

    pub(super) fn update_task_server_data(&mut self, new_server_data: String) {
        let Ok(source) = self.try_get_source_mut() else {
            return;
        };
        source.server_data = new_server_data;
    }

    pub(super) fn append_exchange(&mut self, exchange: AIAgentExchange) {
        self.exchanges.push(exchange);
    }

    fn try_get_source(&self) -> Result<&api::Task, UpdateTaskError> {
        let TaskImpl::Server(ServerTask { source, .. }) = &self.data else {
            return Err(UpdateTaskError::TaskNotInitialized);
        };
        Ok(source)
    }

    fn try_get_source_mut(&mut self) -> Result<&mut api::Task, UpdateTaskError> {
        let TaskImpl::Server(ServerTask { source, .. }) = &mut self.data else {
            return Err(UpdateTaskError::TaskNotInitialized);
        };
        Ok(source)
    }

    pub(super) fn add_messages(
        &mut self,
        messages: Vec<api::Message>,
        exchange_id: AIAgentExchangeId,
        message_context: TaskMessageContext<'_>,
        should_convert_input_messages: bool,
    ) -> Result<(), UpdateTaskError> {
        if self.source().is_none() {
            return Err(UpdateTaskError::TaskNotInitialized);
        }
        self.update_exchange_from_messages(
            messages.clone(),
            exchange_id,
            message_context,
            should_convert_input_messages,
        )?;
        self.try_get_source_mut()?.messages.extend(messages);
        Ok(())
    }

    pub(super) fn upsert_message(
        &mut self,
        message: api::Message,
        exchange_id: AIAgentExchangeId,
        message_context: TaskMessageContext<'_>,
        mask: FieldMask,
        should_convert_input_messages: bool,
    ) -> Result<&api::Message, UpdateTaskError> {
        let Some((idx, existing_message)) = self
            .try_get_source()?
            .messages
            .iter()
            .enumerate()
            .find(|(_, m)| message.id == m.id)
        else {
            self.add_messages(
                vec![message.clone()],
                exchange_id,
                message_context,
                should_convert_input_messages,
            )?;
            return self
                .try_get_source()?
                .messages
                .last()
                .ok_or(UpdateTaskError::MessageNotFound);
        };
        let updated_message =
            FieldMaskOperation::update(&api::MESSAGE_DESCRIPTOR, existing_message, &message, mask)
                .apply()
                .map_err(UpdateTaskError::from)?;

        let id = self.id.clone();
        let exchange_to_update = self
            .exchange_mut(exchange_id)
            .ok_or(UpdateTaskError::ExchangeNotFound)?;
        exchange_to_update.upsert_output_for_message(
            &updated_message,
            ConversionParams {
                task_id: &id,
                current_todo_list: message_context.current_todo_list,
                active_code_review: message_context.active_code_review,
                skill_path_origin: message_context.skill_path_origin,
            },
        )?;

        // Task message updates can carry tool call result updates with them,
        // so we need to convert any tool call results and update the exchange accordingly
        // (this is necessary for session sharing, where the tool call input has not already been
        // optimistically inserted into the exchange)
        if should_convert_input_messages {
            if let Some(tool_call_result) = message.tool_call_result() {
                let mut document_versions: HashMap<AIDocumentId, AIDocumentVersion> =
                    HashMap::new();
                if let Some(input) = convert_tool_call_result_to_input(
                    &id,
                    tool_call_result,
                    &HashMap::new(),
                    &mut document_versions,
                ) {
                    if let Some(action_result) = input.action_result() {
                        if let Some(existing_result) =
                            exchange_to_update.input.iter_mut().find(|existing_input| {
                                existing_input
                                    .action_result()
                                    .is_some_and(|existing_result| {
                                        existing_result.id == action_result.id
                                    })
                            })
                        {
                            *existing_result = input;
                        }
                    } else {
                        exchange_to_update.input.push(input)
                    }
                }
            }
        }

        let source = self.try_get_source_mut()?;
        source.messages[idx] = updated_message;
        Ok(&source.messages[idx])
    }

    pub(super) fn append_to_message_content(
        &mut self,
        message: api::Message,
        exchange_id: AIAgentExchangeId,
        message_context: TaskMessageContext<'_>,
        mask: FieldMask,
    ) -> Result<&api::Message, UpdateTaskError> {
        let Some((idx, existing_message)) = self
            .try_get_source()?
            .messages
            .iter()
            .enumerate()
            .find(|(_, m)| message.id == m.id)
        else {
            log::error!("Message not found for append client action.");
            return Err(UpdateTaskError::MessageNotFound);
        };
        let updated_message =
            FieldMaskOperation::append(&api::MESSAGE_DESCRIPTOR, existing_message, &message, mask)
                .apply()
                .map_err(UpdateTaskError::from)?;

        let id = self.id.clone();
        let exchange_to_update = self
            .exchange_mut(exchange_id)
            .ok_or(UpdateTaskError::ExchangeNotFound)?;
        exchange_to_update.upsert_output_for_message(
            &updated_message,
            ConversionParams {
                task_id: &id,
                current_todo_list: message_context.current_todo_list,
                active_code_review: message_context.active_code_review,
                skill_path_origin: message_context.skill_path_origin,
            },
        )?;

        let source = self.try_get_source_mut()?;
        source.messages[idx] = updated_message;
        Ok(&source.messages[idx])
    }

    pub(super) fn remove_exchange(
        &mut self,
        exchange_id: AIAgentExchangeId,
    ) -> Option<AIAgentExchange> {
        if let Some(index) = self
            .exchanges
            .iter()
            .position(|exchange| exchange.id == exchange_id)
        {
            Some(self.exchanges.remove(index))
        } else {
            None
        }
    }

    /// Truncates all exchanges starting from the given exchange ID (inclusive).
    pub(super) fn truncate_exchanges_from(&mut self, from_exchange_id: AIAgentExchangeId) {
        if let Some(index) = self
            .exchanges
            .iter()
            .position(|exchange| exchange.id == from_exchange_id)
        {
            self.exchanges.truncate(index);
        }
    }

    /// Assigns fresh exchange IDs to all exchanges in this task.
    /// Used when forking conversations to avoid ID collisions with persisted blocks.
    pub(super) fn reassign_exchange_ids(&mut self) {
        for exchange in &mut self.exchanges {
            exchange.id = AIAgentExchangeId::new();
        }
    }

    /// Removes messages with the given IDs from the task source.
    pub(super) fn remove_messages(&mut self, message_ids: &HashSet<MessageId>) {
        match self.try_get_source_mut() {
            Ok(source) => {
                source
                    .messages
                    .retain(|m| !message_ids.contains(&MessageId::new(m.id.clone())));
            }
            Err(e) => {
                log::warn!("Failed to get mutable source for removing messages: {e:?}");
            }
        }
    }

    /// Splices a range of messages into the task, returning the replaced messages.
    ///
    /// This finds the range from `first_message_id` to `last_message_id` (inclusive),
    /// validates that the range contains `expected_message_count` messages, removes
    /// those messages, inserts `replacement_messages` at the same position, and
    /// returns the extracted messages.
    ///
    /// Note: This only modifies the task's proto message list. It does NOT modify
    /// the exchange's client representation, so the UI remains unchanged during a
    /// live session.
    pub(super) fn splice_messages(
        &mut self,
        first_message_id: &str,
        last_message_id: &str,
        expected_message_count: u32,
        replacement_messages: Vec<api::Message>,
    ) -> Result<Vec<api::Message>, ExtractMessagesError> {
        let source = match self.try_get_source_mut() {
            Ok(s) => s,
            Err(_) => return Err(ExtractMessagesError::TaskNotInitialized),
        };

        // Find the index of the first message.
        let first_idx = source
            .messages
            .iter()
            .position(|m| m.id == first_message_id)
            .ok_or_else(|| {
                ExtractMessagesError::FirstMessageNotFound(first_message_id.to_string())
            })?;

        // Find the index of the last message.
        let last_idx = source
            .messages
            .iter()
            .position(|m| m.id == last_message_id)
            .ok_or_else(|| {
                ExtractMessagesError::LastMessageNotFound(last_message_id.to_string())
            })?;

        // Validate that first comes before or equals last.
        if first_idx > last_idx {
            return Err(ExtractMessagesError::InvalidRange);
        }

        // Calculate the actual message count in the range (inclusive).
        let actual_count = (last_idx - first_idx + 1) as u32;
        if actual_count != expected_message_count {
            return Err(ExtractMessagesError::ChecksumMismatch {
                expected: expected_message_count,
                actual: actual_count,
            });
        }

        // Drain the messages from the range.
        let extracted: Vec<api::Message> = source.messages.drain(first_idx..=last_idx).collect();

        // Insert the replacement messages at the same position.
        source
            .messages
            .splice(first_idx..first_idx, replacement_messages);

        Ok(extracted)
    }

    fn update_exchange_from_messages(
        &mut self,
        messages: Vec<api::Message>,
        exchange_id: AIAgentExchangeId,
        message_context: TaskMessageContext<'_>,
        should_convert_input_messages: bool,
    ) -> Result<(), UpdateTaskError> {
        let exchange = self
            .exchange_mut(exchange_id)
            .ok_or(UpdateTaskError::ExchangeNotFound)?;
        exchange
            .added_message_ids
            .extend(messages.iter().map(|m| MessageId::new(m.id.clone())));

        if should_convert_input_messages {
            let user_inputs = user_inputs_from_messages(&messages);

            for input in user_inputs.into_iter() {
                // If the input is an ActionResult with an action ID that already exists,
                // replace the existing one (to handle updates to long-running commands).
                if let Some(action_result) = input.action_result() {
                    if let Some(existing_result) =
                        exchange.input.iter_mut().find(|existing_input| {
                            existing_input
                                .action_result()
                                .is_some_and(|existing_result| {
                                    existing_result.id == action_result.id
                                })
                        })
                    {
                        *existing_result = input;
                        continue;
                    }
                }

                exchange.input.push(input);
            }
        }

        let output = exchange.get_streaming_output()?;
        let output_messages: Result<Vec<AIAgentOutputMessage>, MessageToAIAgentOutputMessageError> =
            messages
                .into_iter()
                .filter_map(|m| {
                    match m.to_client_output_message(ConversionParams {
                        task_id: &self.id,
                        current_todo_list: message_context.current_todo_list,
                        active_code_review: message_context.active_code_review,
                        skill_path_origin: message_context.skill_path_origin,
                    }) {
                        Ok(MaybeAIAgentOutputMessage::Message(m)) => Some(Ok(m)),
                        Ok(MaybeAIAgentOutputMessage::NoClientRepresentation) => None,
                        Err(e) => Some(Err(e)),
                    }
                })
                .collect();
        output.get_mut().messages.extend(output_messages?);
        Ok(())
    }
}

/// Derives todo lists from tasks by replaying UpdateTodos operations in message order.
pub fn derive_todo_lists_from_root_task(root_task: &Task) -> Vec<AIAgentTodoList> {
    let mut todo_lists = Vec::new();

    // Sort messages by their index in the task (messages are already in order within each task)
    // For simplicity, we'll iterate through messages and apply UpdateTodos operations
    for message in root_task.messages() {
        if let Some(api::message::Message::UpdateTodos(update)) = &message.message {
            if let Some(operation) = &update.operation {
                update_todo_list_from_todo_op(&mut todo_lists, operation.clone());
            }
        }
    }

    todo_lists
}

impl AIAgentExchange {
    /// Upserts the output for a specific message.
    /// Note: this means updates will insert a new entry after previously added entries.
    fn upsert_output_for_message(
        &self,
        task_message: &api::Message,
        conversion_params: super::api::ConversionParams<'_>,
    ) -> Result<(), UpdateTaskError> {
        if let AIAgentOutputStatus::Streaming {
            output: Some(output),
        } = &self.output_status
        {
            let mut output = output.get_mut();
            let message_idx = output
                .messages
                .iter()
                .position(|m| m.id.0 == task_message.id);

            match task_message
                .clone()
                .to_client_output_message(conversion_params)?
            {
                MaybeAIAgentOutputMessage::Message(m) => {
                    // Extract citations from the message and add to the output citations
                    output.extend_citations(m.citations.clone());
                    // Upsert behavior: update the message if it exists, otherwise add it to the end of the list.
                    if let Some(message_idx) = message_idx {
                        output.messages[message_idx] = m;
                    } else {
                        output.messages.push(m);
                    }
                }
                MaybeAIAgentOutputMessage::NoClientRepresentation => {
                    log::warn!(
                        "Tried to update output for message which no longer has a client representation"
                    );
                }
            }
        }

        Ok(())
    }

    /// Retrieves the output if it is currently being streamed.
    fn get_streaming_output(&self) -> Result<Shared<AIAgentOutput>, UpdateTaskError> {
        match &self.output_status {
            AIAgentOutputStatus::Streaming {
                output: Some(output),
            } => Ok(output.get_owned()),
            AIAgentOutputStatus::Streaming { output: None } => {
                Err(UpdateTaskError::OutputNeverInitialized)
            }
            _ => Err(UpdateTaskError::OutputAlreadyFinished),
        }
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
