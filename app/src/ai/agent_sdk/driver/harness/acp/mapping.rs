//! Translates ACP `session/update` notifications for one prompt turn into the MAA
//! `ClientAction`s the native conversation model understands.
use std::collections::HashMap;
use std::time::SystemTime;

use prost_types::FieldMask;
use serde_json::Value;
use uuid::Uuid;
use warp_multi_agent_api::client_action::{AddMessagesToTask, AppendToMessageContent, CreateTask};
use warp_multi_agent_api::message::tool_call::apply_file_diffs::{FileDiff, NewFile};
use warp_multi_agent_api::message::tool_call::read_files::File as ReadFilesFile;
use warp_multi_agent_api::message::tool_call::{
    ApplyFileDiffs, CallMcpTool, ReadFiles, RunShellCommand, Tool,
};
use warp_multi_agent_api::message::tool_call_result::Result as ToolResult;
use warp_multi_agent_api::message::update_todos::Operation as TodoOperation;
use warp_multi_agent_api::message::{
    AgentOutput, AgentReasoning, Message as MessageKind, ToolCall, ToolCallResult, UpdateTodos,
    UserQuery,
};
use warp_multi_agent_api::{
    ApplyFileDiffsResult, CallMcpToolResult, ClientAction, CreateTodoList, FileContent,
    FileContentLineRange, MarkTodosCompleted, Message, ReadFilesResult, RunShellCommandResult,
    ShellCommandFinished, Task, TodoItem, apply_file_diffs_result, call_mcp_tool_result,
    client_action, read_files_result, run_shell_command_result,
};

use super::protocol::{
    PlanEntry, SessionUpdate, ToolCallContent, ToolCallFields, ToolCallStatus, ToolKind,
};
use crate::ai::agent::api::serde_json_to_prost;

const AGENT_OUTPUT_TEXT_PATH: &str = "agent_output.text";
const AGENT_REASONING_TEXT_PATH: &str = "agent_reasoning.reasoning";

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Output,
    Thought,
}

struct OpenMessage {
    kind: StreamKind,
    acp_message_id: Option<String>,
    message_id: String,
}

/// The MAA tool an ACP tool call is presented as. Decided once when the call is announced, so
/// the call and its result always agree; see [`ToolProjection::for_call`].
#[derive(Clone, Debug, PartialEq)]
enum ToolProjection {
    Shell {
        command: String,
    },
    /// One file edit, rendered through the conversation's file-edit UI. `old_text` is `None`
    /// for a newly created file.
    FileEdits {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    ReadFiles {
        path: String,
        line_range: Option<FileContentLineRange>,
    },
    /// Everything else is shown as an opaque tool call with the agent's raw input as its
    /// arguments. `search` deliberately lands here: `GrepResult` needs structured per-line
    /// matches, and ACP search output is free text whose shape differs per agent.
    Generic {
        name: String,
        args: Option<prost_types::Struct>,
    },
}

impl ToolProjection {
    fn for_call(
        kind: Option<ToolKind>,
        title: &str,
        raw_input: Option<&Value>,
        content: Option<&[ToolCallContent]>,
    ) -> Self {
        if let Some(command) = command_from_raw_input(raw_input, kind, title) {
            return Self::Shell { command };
        }
        if kind == Some(ToolKind::Edit)
            && let Some((path, old_text, new_text)) = first_diff(content)
        {
            return Self::FileEdits {
                path,
                old_text,
                new_text,
            };
        }
        if kind == Some(ToolKind::Read)
            && let Some(path) = path_from_raw_input(raw_input)
        {
            return Self::ReadFiles {
                path,
                line_range: line_range_from_raw_input(raw_input),
            };
        }
        Self::Generic {
            name: mcp_tool_name(kind, title),
            args: raw_input
                .cloned()
                .and_then(|input| serde_json_to_prost(input).ok())
                .and_then(|value| match value.kind {
                    Some(prost_types::value::Kind::StructValue(fields)) => Some(fields),
                    _ => None,
                }),
        }
    }

    fn tool(&self, title: &str) -> Tool {
        match self {
            Self::Shell { command } => Tool::RunShellCommand(RunShellCommand {
                command: command.clone(),
                ..Default::default()
            }),
            Self::FileEdits {
                path,
                old_text,
                new_text,
            } => {
                let mut diffs = ApplyFileDiffs {
                    summary: title.to_owned(),
                    ..Default::default()
                };
                match old_text {
                    Some(old_text) => diffs.diffs.push(FileDiff {
                        file_path: path.clone(),
                        search: old_text.clone(),
                        replace: new_text.clone(),
                    }),
                    None => diffs.new_files.push(NewFile {
                        file_path: path.clone(),
                        content: new_text.clone(),
                        allow_overwrite: true,
                    }),
                }
                Tool::ApplyFileDiffs(diffs)
            }
            Self::ReadFiles { path, line_range } => Tool::ReadFiles(ReadFiles {
                files: vec![ReadFilesFile {
                    name: path.clone(),
                    line_ranges: line_range.iter().copied().collect(),
                }],
            }),
            Self::Generic { name, args } => Tool::CallMcpTool(CallMcpTool {
                name: name.clone(),
                args: args.clone(),
                ..Default::default()
            }),
        }
    }

    fn result(&self, action_id: &str, output: String, failed: bool) -> ToolResult {
        match self {
            Self::Shell { command } => {
                let exit_code = i32::from(failed);
                #[allow(deprecated)]
                ToolResult::RunShellCommand(RunShellCommandResult {
                    command: command.clone(),
                    output: output.clone(),
                    exit_code,
                    result: Some(run_shell_command_result::Result::CommandFinished(
                        ShellCommandFinished {
                            output,
                            exit_code,
                            command_id: action_id.to_owned(),
                            start_ts: None,
                            finish_ts: None,
                        },
                    )),
                })
            }
            Self::FileEdits { path, new_text, .. } => {
                let result = if failed {
                    apply_file_diffs_result::Result::Error(apply_file_diffs_result::Error {
                        message: output,
                    })
                } else {
                    apply_file_diffs_result::Result::Success(apply_file_diffs_result::Success {
                        updated_files_v2: vec![
                            apply_file_diffs_result::success::UpdatedFileContent {
                                file: Some(FileContent {
                                    file_path: path.clone(),
                                    content: new_text.clone(),
                                    line_range: None,
                                }),
                                was_edited_by_user: false,
                            },
                        ],
                        ..Default::default()
                    })
                };
                ToolResult::ApplyFileDiffs(ApplyFileDiffsResult {
                    result: Some(result),
                })
            }
            Self::ReadFiles { path, line_range } => {
                let result = if failed {
                    read_files_result::Result::Error(read_files_result::Error { message: output })
                } else {
                    read_files_result::Result::TextFilesSuccess(
                        read_files_result::TextFilesSuccess {
                            files: vec![FileContent {
                                file_path: path.clone(),
                                content: output,
                                line_range: *line_range,
                            }],
                            failed_reads: Vec::new(),
                        },
                    )
                };
                ToolResult::ReadFiles(ReadFilesResult {
                    result: Some(result),
                })
            }
            Self::Generic { .. } if failed => ToolResult::CallMcpTool(CallMcpToolResult {
                result: Some(call_mcp_tool_result::Result::Error(
                    call_mcp_tool_result::Error { message: output },
                )),
            }),
            Self::Generic { .. } => ToolResult::CallMcpTool(CallMcpToolResult {
                result: Some(call_mcp_tool_result::Result::Success(
                    call_mcp_tool_result::Success {
                        results: vec![call_mcp_tool_result::success::Result {
                            result: Some(call_mcp_tool_result::success::result::Result::Text(
                                call_mcp_tool_result::success::result::Text { text: output },
                            )),
                        }],
                    },
                )),
            }),
        }
    }
}

struct ToolCallState {
    /// The id the conversation sees. Minted here rather than reusing the agent's `toolCallId`:
    /// the action model indexes results by id across conversations, and agents are free to
    /// reuse their ids across turns.
    action_id: String,
    kind: Option<ToolKind>,
    title: String,
    projection: ToolProjection,
    output: String,
    has_result: bool,
}

/// A `think` tool call streamed as reasoning rather than shown as a tool. Agents tend to resend
/// the whole thought on each update, so the text already emitted is kept to append only what is
/// new.
struct ThoughtState {
    emitted: String,
    finished: bool,
}

/// Output of the mapper for one ACP update, in order.
#[derive(Debug)]
pub(super) enum TurnEvent {
    Actions(Vec<ClientAction>),
    /// The current request stream must be finished and a new one opened before the following
    /// events are applied. Mirrors MAA, where tool calls end a stream and their results arrive
    /// in the next request, so a stream that ends without actions settles the conversation.
    SegmentBoundary,
}

pub(super) struct AcpTurnMapper {
    task_id: String,
    request_id: String,
    open_message: Option<OpenMessage>,
    tool_calls: HashMap<String, ToolCallState>,
    thoughts: HashMap<String, ThoughtState>,
    todo_list_created: bool,
    /// Whether a tool call was announced in the current request stream, so the next result
    /// must be preceded by a segment boundary.
    tool_call_announced_in_segment: bool,
}

impl AcpTurnMapper {
    pub(super) fn new(task_id: String, request_id: String) -> Self {
        Self {
            task_id,
            request_id,
            open_message: None,
            tool_calls: HashMap::new(),
            thoughts: HashMap::new(),
            todo_list_created: false,
            tool_call_announced_in_segment: false,
        }
    }

    /// Switches message stamping to the request stream opened after a [`TurnEvent::SegmentBoundary`].
    pub(super) fn start_segment(&mut self, request_id: String) {
        self.request_id = request_id;
        self.tool_call_announced_in_segment = false;
        self.open_message = None;
    }

    /// Actions that open the conversation: create the root task and echo the user's prompt,
    /// mirroring what the MAA server streams first.
    pub(super) fn initial_actions(&self, prompt: &str) -> Vec<ClientAction> {
        vec![
            action(client_action::Action::CreateTask(CreateTask {
                task: Some(Task {
                    id: self.task_id.clone(),
                    ..Default::default()
                }),
            })),
            self.user_query_action(prompt),
        ]
    }

    /// Echoes a user prompt into the task, as the MAA server does for each request.
    pub(super) fn user_query_action(&self, prompt: &str) -> ClientAction {
        self.add_messages(vec![self.new_message(MessageKind::UserQuery(UserQuery {
            query: prompt.to_owned(),
            ..Default::default()
        }))])
    }

    pub(super) fn map_update(&mut self, update: SessionUpdate) -> Vec<TurnEvent> {
        match update {
            SessionUpdate::AgentMessageChunk {
                content,
                message_id,
            } => actions(self.chunk(StreamKind::Output, content.as_text(), message_id)),
            SessionUpdate::AgentThoughtChunk {
                content,
                message_id,
            } => actions(self.chunk(StreamKind::Thought, content.as_text(), message_id)),
            // The prompt was already echoed by `initial_actions`.
            SessionUpdate::UserMessageChunk { .. } => Vec::new(),
            SessionUpdate::ToolCall { call } => self.tool_call(call),
            SessionUpdate::ToolCallUpdate { call } => self.tool_call_update(call),
            SessionUpdate::Plan { entries } => actions(self.plan(&entries)),
            SessionUpdate::UsageUpdate { .. } | SessionUpdate::Unsupported => Vec::new(),
        }
    }

    /// Events that close the turn: a cancelled result for every tool call the agent never
    /// finished, so nothing is left pending in the conversation.
    pub(super) fn finish_events(&mut self) -> Vec<TurnEvent> {
        self.open_message = None;
        let mut unfinished: Vec<String> = self
            .tool_calls
            .iter()
            .filter(|(_, state)| !state.has_result)
            .map(|(id, _)| id.clone())
            .collect();
        unfinished.sort();
        if unfinished.is_empty() {
            return Vec::new();
        }
        let mut events = self.boundary_before_results();
        let messages: Vec<Message> = unfinished
            .into_iter()
            .filter_map(|tool_call_id| {
                let state = self.tool_calls.get_mut(&tool_call_id)?;
                state.has_result = true;
                let action_id = state.action_id.clone();
                Some(self.cancel_message(&action_id))
            })
            .collect();
        events.push(TurnEvent::Actions(vec![self.add_messages(messages)]));
        events
    }

    fn cancel_message(&self, action_id: &str) -> Message {
        self.new_message(MessageKind::ToolCallResult(ToolCallResult {
            tool_call_id: action_id.to_owned(),
            context: None,
            result: Some(ToolResult::Cancel(())),
        }))
    }

    /// A boundary is required when results would otherwise land in the same stream as the tool
    /// calls they answer.
    fn boundary_before_results(&mut self) -> Vec<TurnEvent> {
        if self.tool_call_announced_in_segment {
            self.tool_call_announced_in_segment = false;
            self.open_message = None;
            vec![TurnEvent::SegmentBoundary]
        } else {
            Vec::new()
        }
    }

    fn chunk(
        &mut self,
        kind: StreamKind,
        text: String,
        acp_message_id: Option<String>,
    ) -> Vec<ClientAction> {
        if text.is_empty() {
            return Vec::new();
        }
        let continues_open_message = self.open_message.as_ref().is_some_and(|open| {
            open.kind == kind && (acp_message_id.is_none() || open.acp_message_id == acp_message_id)
        });
        let payload = |text: String| match kind {
            StreamKind::Output => MessageKind::AgentOutput(AgentOutput { text }),
            StreamKind::Thought => MessageKind::AgentReasoning(AgentReasoning {
                reasoning: text,
                finished_duration: None,
            }),
        };
        if continues_open_message {
            let message_id = self
                .open_message
                .as_ref()
                .map(|open| open.message_id.clone())
                .unwrap_or_default();
            let path = match kind {
                StreamKind::Output => AGENT_OUTPUT_TEXT_PATH,
                StreamKind::Thought => AGENT_REASONING_TEXT_PATH,
            };
            return vec![action(client_action::Action::AppendToMessageContent(
                AppendToMessageContent {
                    task_id: self.task_id.clone(),
                    message: Some(Message {
                        id: message_id,
                        task_id: self.task_id.clone(),
                        request_id: self.request_id.clone(),
                        message: Some(payload(text)),
                        ..Default::default()
                    }),
                    mask: Some(FieldMask {
                        paths: vec![path.to_owned()],
                    }),
                },
            ))];
        }
        let message = self.new_message(payload(text));
        self.open_message = Some(OpenMessage {
            kind,
            acp_message_id,
            message_id: message.id.clone(),
        });
        vec![self.add_messages(vec![message])]
    }

    fn tool_call(&mut self, call: ToolCallFields) -> Vec<TurnEvent> {
        // A finished call's id may be reused by the agent in a later turn; only an unfinished
        // one is the same call.
        if self
            .tool_calls
            .get(&call.tool_call_id)
            .is_some_and(|state| !state.has_result)
            || self
                .thoughts
                .get(&call.tool_call_id)
                .is_some_and(|thought| !thought.finished)
        {
            return self.tool_call_update(call);
        }
        if call.kind == Some(ToolKind::Think) {
            return self.thought_call(call);
        }
        self.open_message = None;
        let title = call.title.clone().unwrap_or_else(|| "Tool call".to_owned());
        let projection = ToolProjection::for_call(
            call.kind,
            &title,
            call.raw_input.as_ref(),
            call.content.as_deref(),
        );
        let action_id = Uuid::new_v4().to_string();
        let tool = projection.tool(&title);
        let state = ToolCallState {
            action_id: action_id.clone(),
            kind: call.kind,
            title,
            projection,
            output: tool_output_text(call.content.as_deref(), call.raw_output.as_ref()),
            has_result: false,
        };
        self.tool_calls.insert(call.tool_call_id.clone(), state);
        self.tool_call_announced_in_segment = true;

        let mut events = vec![TurnEvent::Actions(vec![self.add_messages(vec![
            self.new_message(MessageKind::ToolCall(ToolCall {
                tool_call_id: action_id,
                tool: Some(tool),
            })),
        ])])];
        if let Some(result) = self.result_if_finished(&call.tool_call_id, call.status) {
            events.extend(self.boundary_before_results());
            events.push(TurnEvent::Actions(vec![self.add_messages(vec![result])]));
        }
        events
    }

    fn tool_call_update(&mut self, call: ToolCallFields) -> Vec<TurnEvent> {
        if self.thoughts.contains_key(&call.tool_call_id) {
            return self.thought_call(call);
        }
        if !self.tool_calls.contains_key(&call.tool_call_id) {
            return self.tool_call(call);
        }
        if let Some(events) = self.reproject_as_file_edits(&call) {
            return events;
        }
        self.open_message = None;
        let Some(state) = self.tool_calls.get_mut(&call.tool_call_id) else {
            return Vec::new();
        };
        if let Some(title) = call.title.clone() {
            state.title = title;
        }
        if call.kind.is_some() {
            state.kind = call.kind;
        }
        match &mut state.projection {
            ToolProjection::Shell { command } => {
                if call.raw_input.is_some()
                    && let Some(updated) =
                        command_from_raw_input(call.raw_input.as_ref(), state.kind, "")
                {
                    *command = updated;
                }
            }
            // The announced call already carries the diff; a later one (e.g. the applied
            // result) is the authoritative final content.
            ToolProjection::FileEdits {
                old_text, new_text, ..
            } => {
                if let Some((_, updated_old, updated_new)) = first_diff(call.content.as_deref()) {
                    if updated_old.is_some() {
                        *old_text = updated_old;
                    }
                    *new_text = updated_new;
                }
            }
            ToolProjection::ReadFiles { .. } | ToolProjection::Generic { .. } => {}
        }
        let output = tool_output_text(call.content.as_deref(), call.raw_output.as_ref());
        if !output.is_empty() {
            state.output = output;
        }
        match self.result_if_finished(&call.tool_call_id, call.status) {
            Some(result) => {
                let mut events = self.boundary_before_results();
                events.push(TurnEvent::Actions(vec![self.add_messages(vec![result])]));
                events
            }
            None => Vec::new(),
        }
    }

    /// Some agents announce an edit before they know what it changes and attach the diff only
    /// on a later update. The announced generic call cannot be rewritten in place, so it is
    /// cancelled and the edit is re-announced as a file edit under a fresh action id.
    fn reproject_as_file_edits(&mut self, call: &ToolCallFields) -> Option<Vec<TurnEvent>> {
        let state = self.tool_calls.get(&call.tool_call_id)?;
        let kind = call.kind.or(state.kind);
        if state.has_result
            || kind != Some(ToolKind::Edit)
            || !matches!(state.projection, ToolProjection::Generic { .. })
            || first_diff(call.content.as_deref()).is_none()
        {
            return None;
        }
        let previous = self.tool_calls.remove(&call.tool_call_id)?;
        let mut events = self.boundary_before_results();
        events.push(TurnEvent::Actions(vec![
            self.add_messages(vec![self.cancel_message(&previous.action_id)]),
        ]));
        let mut merged = call.clone();
        merged.title = call.title.clone().or(Some(previous.title));
        merged.kind = kind;
        events.extend(self.tool_call(merged));
        Some(events)
    }

    /// Streams a `think` tool call as reasoning. The thought's text may arrive on the
    /// announcement, on updates, or both.
    fn thought_call(&mut self, call: ToolCallFields) -> Vec<TurnEvent> {
        let text = thought_text(&call);
        let finished = matches!(
            call.status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        );
        let new_text = {
            let thought = self
                .thoughts
                .entry(call.tool_call_id.clone())
                .or_insert_with(|| ThoughtState {
                    emitted: String::new(),
                    finished: false,
                });
            let new_text = match text.strip_prefix(thought.emitted.as_str()) {
                Some(suffix) => suffix.to_owned(),
                None => text,
            };
            thought.emitted.push_str(&new_text);
            thought.finished |= finished;
            new_text
        };
        let acp_message_id = Some(format!("think:{}", call.tool_call_id));
        let events = actions(self.chunk(StreamKind::Thought, new_text, acp_message_id));
        if finished {
            self.open_message = None;
        }
        events
    }

    fn result_if_finished(
        &mut self,
        tool_call_id: &str,
        status: Option<ToolCallStatus>,
    ) -> Option<Message> {
        let failed = match status {
            Some(ToolCallStatus::Completed) => false,
            Some(ToolCallStatus::Failed) => true,
            Some(
                ToolCallStatus::Pending | ToolCallStatus::InProgress | ToolCallStatus::Unknown,
            )
            | None => return None,
        };
        let state = self.tool_calls.get_mut(tool_call_id)?;
        if state.has_result {
            return None;
        }
        state.has_result = true;
        let action_id = state.action_id.clone();
        let output = if state.output.is_empty() && failed {
            format!("{} failed", state.title)
        } else {
            state.output.clone()
        };
        let result = state.projection.result(&action_id, output, failed);
        Some(
            self.new_message(MessageKind::ToolCallResult(ToolCallResult {
                tool_call_id: action_id,
                context: None,
                result: Some(result),
            })),
        )
    }

    fn plan(&mut self, entries: &[PlanEntry]) -> Vec<ClientAction> {
        self.open_message = None;
        let todo_id = |index: usize| format!("acp-todo-{index}");
        let operation = if self.todo_list_created {
            let completed: Vec<String> = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.status.as_deref() == Some("completed"))
                .map(|(index, _)| todo_id(index))
                .collect();
            if completed.is_empty() {
                return Vec::new();
            }
            TodoOperation::MarkTodosCompleted(MarkTodosCompleted {
                todo_ids: completed,
            })
        } else {
            self.todo_list_created = true;
            TodoOperation::CreateTodoList(CreateTodoList {
                initial_todos: entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| TodoItem {
                        id: todo_id(index),
                        title: entry.content.clone(),
                        description: entry.priority.clone().unwrap_or_default(),
                    })
                    .collect(),
            })
        };
        vec![
            self.add_messages(vec![self.new_message(MessageKind::UpdateTodos(
                UpdateTodos {
                    operation: Some(operation),
                },
            ))]),
        ]
    }

    fn add_messages(&self, messages: Vec<Message>) -> ClientAction {
        action(client_action::Action::AddMessagesToTask(
            AddMessagesToTask {
                task_id: self.task_id.clone(),
                messages,
            },
        ))
    }

    fn new_message(&self, kind: MessageKind) -> Message {
        Message {
            id: Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            request_id: self.request_id.clone(),
            timestamp: Some(prost_types::Timestamp::from(SystemTime::now())),
            message: Some(kind),
            ..Default::default()
        }
    }
}

fn action(action: client_action::Action) -> ClientAction {
    ClientAction {
        action: Some(action),
    }
}

fn actions(actions: Vec<ClientAction>) -> Vec<TurnEvent> {
    if actions.is_empty() {
        Vec::new()
    } else {
        vec![TurnEvent::Actions(actions)]
    }
}

/// Extracts the shell command an `execute` tool call runs, trying the common `rawInput` shapes
/// before falling back to the title.
pub(super) fn command_from_raw_input(
    raw_input: Option<&Value>,
    kind: Option<ToolKind>,
    title: &str,
) -> Option<String> {
    if kind != Some(ToolKind::Execute) {
        return None;
    }
    let from_input = raw_input.and_then(|input| {
        ["command", "cmd", "commandLine"]
            .iter()
            .find_map(|key| input.get(key))
            .and_then(|value| match value {
                Value::String(command) => Some(command.clone()),
                Value::Array(parts) => Some(
                    parts
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                _ => None,
            })
    });
    from_input
        .filter(|command| !command.is_empty())
        .or_else(|| (!title.is_empty()).then(|| title.to_owned()))
}

/// The file a `read`, `edit`, `delete`, or `move` tool call targets, under the key each agent's
/// file tools use.
pub(super) fn path_from_raw_input(raw_input: Option<&Value>) -> Option<String> {
    ["file_path", "path", "absolute_path", "filePath"]
        .iter()
        .find_map(|key| raw_input?.get(key)?.as_str())
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

/// Read tools express a window as a 1-based start line (`offset` / `line`) and a `limit`; both
/// are needed for a closed range.
fn line_range_from_raw_input(raw_input: Option<&Value>) -> Option<FileContentLineRange> {
    let input = raw_input?;
    let start = ["offset", "line"]
        .iter()
        .find_map(|key| input.get(key)?.as_u64())?;
    let limit = input.get("limit")?.as_u64()?;
    let start = u32::try_from(start.max(1)).ok()?;
    let end = u32::try_from(u64::from(start) + limit.saturating_sub(1)).ok()?;
    Some(FileContentLineRange { start, end })
}

pub(super) fn first_diff(
    content: Option<&[ToolCallContent]>,
) -> Option<(String, Option<String>, String)> {
    content?.iter().find_map(|item| match item {
        ToolCallContent::Diff {
            path,
            old_text,
            new_text,
        } => Some((path.clone(), old_text.clone(), new_text.clone())),
        ToolCallContent::Content { .. }
        | ToolCallContent::Terminal { .. }
        | ToolCallContent::Unsupported => None,
    })
}

/// The text of a `think` call: its content blocks, else the thought carried in its input, else
/// its title.
fn thought_text(call: &ToolCallFields) -> String {
    let from_content = tool_output_text(call.content.as_deref(), None);
    if !from_content.is_empty() {
        return from_content;
    }
    let from_input = call.raw_input.as_ref().and_then(|input| {
        ["thought", "thinking", "text", "content"]
            .iter()
            .find_map(|key| input.get(key)?.as_str())
            .map(str::to_owned)
    });
    from_input
        .filter(|text| !text.is_empty())
        .or_else(|| call.title.clone())
        .unwrap_or_default()
}

fn mcp_tool_name(kind: Option<ToolKind>, title: &str) -> String {
    let kind = match kind {
        Some(ToolKind::Read) => "read",
        Some(ToolKind::Edit) => "edit",
        Some(ToolKind::Delete) => "delete",
        Some(ToolKind::Move) => "move",
        Some(ToolKind::Search) => "search",
        Some(ToolKind::Execute) => "execute",
        Some(ToolKind::Think) => "think",
        Some(ToolKind::Fetch) => "fetch",
        Some(ToolKind::SwitchMode) => "switch_mode",
        Some(ToolKind::Other | ToolKind::Unknown) | None => "tool",
    };
    if title.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind}: {title}")
    }
}

fn tool_output_text(content: Option<&[ToolCallContent]>, raw_output: Option<&Value>) -> String {
    let mut parts: Vec<String> = content
        .unwrap_or_default()
        .iter()
        .filter_map(|item| match item {
            ToolCallContent::Content { content } => {
                let text = content.as_text();
                (!text.is_empty()).then_some(text)
            }
            ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } => Some(render_diff(path, old_text.as_deref(), new_text)),
            ToolCallContent::Terminal { terminal_id } => Some(format!("[terminal {terminal_id}]")),
            ToolCallContent::Unsupported => None,
        })
        .collect();
    if parts.is_empty()
        && let Some(raw_output) = raw_output
    {
        parts.push(match raw_output {
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        });
    }
    parts.join("\n")
}

fn render_diff(path: &str, old_text: Option<&str>, new_text: &str) -> String {
    let mut rendered = format!("--- {path}\n+++ {path}\n");
    if let Some(old_text) = old_text {
        for line in old_text.lines() {
            rendered.push('-');
            rendered.push_str(line);
            rendered.push('\n');
        }
    }
    for line in new_text.lines() {
        rendered.push('+');
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;
