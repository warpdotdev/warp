//! Turns an envelope into the JSON object Cursor documents for the event: the common fields every
//! hook receives, then the event's own fields. Values Warp does not know are left out rather than
//! invented; the two exceptions are `transcript_path`, which Cursor's schema makes `null`, and
//! `model_params`, which is an empty list because Warp has no structured model parameters.

use std::ops::Range;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;
use warp_terminal::model::BlockId;

use super::events::{CursorEvent, CursorEventSpec};
use super::facets::{finished_shell_command, mcp_tool_call, pending_shell_command};
use crate::agent::action::{
    AIAgentActionType, AIAgentPtyWriteMode, CommentSide, FileEdit, InsertReviewComment,
    RunAgentsExecutionMode, ShellCommandDelay, SuggestPromptRequest,
};
use crate::agent::action_result::{
    AIAgentActionResultType, AnyFileContent, AskUserQuestionAnswerItem, AskUserQuestionResult,
    CallMCPToolResult, CreateDocumentsResult, DocumentContext, EditDocumentsResult,
    FetchConversationResult, FileContext, FileGlobResult, FileGlobV2Result, GrepResult,
    InsertReviewCommentsResult, ReadDocumentsResult, ReadFilesResult, ReadMCPResourceResult,
    ReadShellCommandOutputResult, ReadSkillResult, RequestCommandOutputResult,
    RequestComputerUseResult, RequestFileEditsResult, RunAgentsResult, ScreenshotSource,
    SearchCodebaseResult, SendMessageToAgentResult, StartRecordingResult, StopRecordingResult,
    SuggestNewConversationResult, SuggestPromptResult, TransferShellCommandControlToUserResult,
    UploadArtifactResult, UseComputerResult, WaitForEventsResult,
    WriteToLongRunningShellCommandResult,
};
use crate::diff_validation::ParsedDiff;
use crate::hooks::model::{HookEnvelope, HookPayload, TurnStopStatus};

/// The JSON object a hook on `spec`'s event receives on stdin for `envelope`.
pub(super) fn encode_input(spec: &CursorEventSpec, envelope: &HookEnvelope) -> Value {
    let mut input = common_fields(spec, envelope);
    match spec.event {
        CursorEvent::SessionStart => encode_session_start(envelope, &mut input),
        CursorEvent::PreToolUse => encode_pre_tool_use(envelope, &mut input),
        CursorEvent::PostToolUse => encode_post_tool_use(envelope, &mut input),
        CursorEvent::PostToolUseFailure => encode_post_tool_use_failure(envelope, &mut input),
        CursorEvent::BeforeShellExecution => encode_before_shell_execution(envelope, &mut input),
        CursorEvent::AfterShellExecution => encode_after_shell_execution(envelope, &mut input),
        CursorEvent::BeforeMcpExecution => encode_before_mcp_execution(envelope, &mut input),
        CursorEvent::AfterMcpExecution => encode_after_mcp_execution(envelope, &mut input),
        CursorEvent::BeforeSubmitPrompt => encode_before_submit_prompt(envelope, &mut input),
        CursorEvent::Stop => encode_stop(envelope, &mut input),
        CursorEvent::AfterAgentResponse => encode_after_agent_response(envelope, &mut input),
        CursorEvent::AfterAgentThought => encode_after_agent_thought(envelope, &mut input),
        // Warp does not fire these yet, so there is no payload to encode.
        CursorEvent::SessionEnd
        | CursorEvent::SubagentStart
        | CursorEvent::SubagentStop
        | CursorEvent::BeforeReadFile
        | CursorEvent::AfterFileEdit
        | CursorEvent::PreCompact
        | CursorEvent::BeforeTabFileRead
        | CursorEvent::AfterTabFileEdit
        | CursorEvent::WorkspaceOpen => {}
    }
    Value::Object(input)
}

/// The fields every Cursor hook receives. `generation_id` is the turn id; until a turn id is
/// allocated it is the conversation id, a real identifier that is stable for the conversation.
/// `cursor_version` carries Warp's version because Cursor-format scripts read it; `warp_version`
/// says the same thing under its own name.
fn common_fields(spec: &CursorEventSpec, envelope: &HookEnvelope) -> Map<String, Value> {
    let conversation_id = envelope.conversation_id.to_string();
    let generation_id = envelope
        .turn_id
        .map(|turn| turn.as_uuid().to_string())
        .unwrap_or_else(|| conversation_id.clone());
    let mut fields = Map::new();
    fields.insert("conversation_id".to_owned(), json!(conversation_id));
    fields.insert("generation_id".to_owned(), json!(generation_id));
    if let Some(model) = &envelope.model_id {
        let model = model.to_string();
        fields.insert("model".to_owned(), json!(model));
        fields.insert("model_id".to_owned(), json!(model));
    }
    fields.insert("model_params".to_owned(), json!([]));
    fields.insert("hook_event_name".to_owned(), json!(spec.name));
    fields.insert("cursor_version".to_owned(), json!(envelope.warp_version));
    fields.insert("warp_version".to_owned(), json!(envelope.warp_version));
    fields.insert(
        "workspace_roots".to_owned(),
        json!(workspace_roots(envelope)),
    );
    fields.insert("user_email".to_owned(), json!(envelope.user_email));
    fields.insert("transcript_path".to_owned(), Value::Null);
    fields
}

/// The detected roots, or the working directory when no root was detected.
fn workspace_roots(envelope: &HookEnvelope) -> Vec<String> {
    if envelope.roots.is_empty() {
        return envelope
            .cwd
            .as_deref()
            .map(|cwd| vec![cwd.to_string_lossy().into_owned()])
            .unwrap_or_default();
    }
    envelope
        .roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect()
}

fn insert_cwd(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    if let Some(cwd) = &envelope.cwd {
        input.insert("cwd".to_owned(), json!(cwd.to_string_lossy()));
    }
}

fn insert_tool_use_id(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    if let Some(action_id) = &envelope.action_id {
        input.insert("tool_use_id".to_owned(), json!(action_id.to_string()));
    }
}

fn insert_duration_ms(key: &str, duration: Option<Duration>, input: &mut Map<String, Value>) {
    if let Some(duration) = duration {
        input.insert(key.to_owned(), json!(duration.as_millis() as u64));
    }
}

fn encode_session_start(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::SessionStart {
        is_background_agent,
    } = &envelope.payload
    else {
        return;
    };
    input.insert(
        "session_id".to_owned(),
        json!(envelope.conversation_id.to_string()),
    );
    input.insert("is_background_agent".to_owned(), json!(is_background_agent));
    // Warp's agent has one mode at conversation creation; Cursor's `ask` and `edit` have no
    // equivalent.
    input.insert("composer_mode".to_owned(), json!("agent"));
}

fn encode_pre_tool_use(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::ToolPre { action, tool_name } = &envelope.payload else {
        return;
    };
    input.insert("tool_name".to_owned(), json!(tool_name.as_str()));
    input.insert("tool_input".to_owned(), tool_input(action, envelope));
    insert_tool_use_id(envelope, input);
    insert_cwd(envelope, input);
}

fn encode_post_tool_use(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::ToolResult {
        action,
        tool_name,
        result,
        duration,
    } = &envelope.payload
    else {
        return;
    };
    input.insert("tool_name".to_owned(), json!(tool_name.as_str()));
    input.insert("tool_input".to_owned(), tool_input(action, envelope));
    input.insert(
        "tool_output".to_owned(),
        json!(result_view(result).to_string()),
    );
    insert_tool_use_id(envelope, input);
    insert_cwd(envelope, input);
    insert_duration_ms("duration", *duration, input);
}

fn encode_post_tool_use_failure(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::ToolFailure {
        action,
        tool_name,
        result,
        cancellation_reason,
        duration,
    } = &envelope.payload
    else {
        return;
    };
    input.insert("tool_name".to_owned(), json!(tool_name.as_str()));
    input.insert("tool_input".to_owned(), tool_input(action, envelope));
    insert_tool_use_id(envelope, input);
    insert_cwd(envelope, input);
    input.insert("error_message".to_owned(), json!(result.to_string()));
    input.insert("failure_type".to_owned(), json!(failure_type(result)));
    insert_duration_ms("duration", *duration, input);
    let is_interrupt = cancellation_reason.is_some() || result.is_cancelled();
    input.insert("is_interrupt".to_owned(), json!(is_interrupt));
    if let Some(reason) = cancellation_reason {
        input.insert("cancellation_reason".to_owned(), json!(reason));
    }
}

/// Cursor's `failure_type` vocabulary is `error`, `timeout`, and `permission_denied`. Warp tools
/// do not time out, so only the first and last occur.
fn failure_type(result: &AIAgentActionResultType) -> &'static str {
    let permission_denied = matches!(
        result,
        AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::Denylisted { .. }
                | RequestCommandOutputResult::HookDenied { .. }
        ) | AIAgentActionResultType::RunAgents(RunAgentsResult::Denied { .. })
    );
    if permission_denied {
        "permission_denied"
    } else {
        "error"
    }
}

fn encode_before_shell_execution(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let Some(command) = pending_shell_command(&envelope.payload) else {
        return;
    };
    input.insert("command".to_owned(), json!(command));
    insert_cwd(envelope, input);
    insert_tool_use_id(envelope, input);
}

fn encode_after_shell_execution(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let Some(finished) = finished_shell_command(&envelope.payload) else {
        return;
    };
    input.insert("command".to_owned(), json!(finished.command));
    input.insert("output".to_owned(), json!(finished.output));
    let duration = payload_duration(&envelope.payload)
        .or_else(|| elapsed_between(finished.start_ts, finished.completed_ts));
    insert_duration_ms("duration", duration, input);
    if let Some(exit_code) = finished.exit_code {
        input.insert("exit_code".to_owned(), json!(exit_code));
    }
    insert_tool_use_id(envelope, input);
}

fn encode_before_mcp_execution(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let Some(call) = mcp_tool_call(&envelope.payload) else {
        return;
    };
    insert_mcp_call_fields(call.name, call.input, call.server_id, input);
    insert_tool_use_id(envelope, input);
}

fn encode_after_mcp_execution(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let Some(call) = mcp_tool_call(&envelope.payload) else {
        return;
    };
    insert_mcp_call_fields(call.name, call.input, call.server_id, input);
    if let Some(result) = call.result {
        let result_json = match result {
            CallMCPToolResult::Success { result } => serialized(result),
            CallMCPToolResult::Error(message) => json!({ "error": message }),
            CallMCPToolResult::Cancelled => Value::Null,
        };
        input.insert("result_json".to_owned(), json!(result_json.to_string()));
    }
    insert_duration_ms("duration", payload_duration(&envelope.payload), input);
    insert_tool_use_id(envelope, input);
}

/// Cursor sends the MCP tool's parameters as a JSON string on these events, unlike `preToolUse`,
/// which sends an object. The server name is not known in this crate, so the server id stands in
/// under its own name.
fn insert_mcp_call_fields(
    name: &str,
    tool_input: &Value,
    server_id: Option<Uuid>,
    input: &mut Map<String, Value>,
) {
    input.insert("tool_name".to_owned(), json!(name));
    input.insert("tool_input".to_owned(), json!(tool_input.to_string()));
    if let Some(server_id) = server_id {
        input.insert("mcp_server_id".to_owned(), json!(server_id.to_string()));
    }
}

fn encode_before_submit_prompt(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::PromptSubmit {
        text,
        attachments_summary,
    } = &envelope.payload
    else {
        return;
    };
    input.insert("prompt".to_owned(), json!(text));
    let attachments = attachments_summary
        .iter()
        .map(|path| json!({ "type": "file", "file_path": path }))
        .collect::<Vec<_>>();
    input.insert("attachments".to_owned(), Value::Array(attachments));
}

fn encode_stop(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::TurnStop {
        status,
        continuation_count,
    } = &envelope.payload
    else {
        return;
    };
    let status = match status {
        TurnStopStatus::Completed => "completed",
        TurnStopStatus::Cancelled => "aborted",
        TurnStopStatus::Error => "error",
    };
    input.insert("status".to_owned(), json!(status));
    input.insert("loop_count".to_owned(), json!(continuation_count));
}

fn encode_after_agent_response(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::ResponseComplete { text, .. } = &envelope.payload else {
        return;
    };
    input.insert("text".to_owned(), json!(text));
}

fn encode_after_agent_thought(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let HookPayload::ReasoningComplete { text, duration, .. } = &envelope.payload else {
        return;
    };
    input.insert("text".to_owned(), json!(text));
    insert_duration_ms("duration_ms", *duration, input);
}

fn payload_duration(payload: &HookPayload) -> Option<Duration> {
    match payload {
        HookPayload::ToolResult { duration, .. } | HookPayload::ToolFailure { duration, .. } => {
            *duration
        }
        HookPayload::ToolPre { .. }
        | HookPayload::SessionStart { .. }
        | HookPayload::PromptSubmit { .. }
        | HookPayload::TurnStop { .. }
        | HookPayload::ResponseComplete { .. }
        | HookPayload::ReasoningComplete { .. } => None,
    }
}

fn elapsed_between(
    start: Option<DateTime<Local>>,
    end: Option<DateTime<Local>>,
) -> Option<Duration> {
    let (start, end) = (start?, end?);
    end.signed_duration_since(start).to_std().ok()
}

fn serialized<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn insert_some(fields: &mut Map<String, Value>, key: &str, value: Option<impl Serialize>) {
    if let Some(value) = value {
        fields.insert(key.to_owned(), serialized(&value));
    }
}

fn timestamp(ts: Option<DateTime<Local>>) -> Option<String> {
    ts.map(|ts| ts.to_rfc3339())
}

fn range_view(range: &Range<usize>) -> Value {
    json!([range.start, range.end])
}

/// The `tool_input` object for `preToolUse` and the post-tool events: the action's arguments as
/// the model gave them. Shell commands carry the working directory beside the command, as
/// Cursor's example does.
fn tool_input(action: &AIAgentActionType, envelope: &HookEnvelope) -> Value {
    match action {
        AIAgentActionType::RequestCommandOutput {
            command,
            is_read_only,
            is_risky,
            wait_until_completion,
            uses_pager,
            rationale,
            citations: _,
        } => {
            let mut fields = Map::new();
            fields.insert("command".to_owned(), json!(command));
            if let Some(cwd) = &envelope.cwd {
                fields.insert("cwd".to_owned(), json!(cwd.to_string_lossy()));
            }
            insert_some(&mut fields, "is_read_only", *is_read_only);
            insert_some(&mut fields, "is_risky", *is_risky);
            fields.insert(
                "wait_until_completion".to_owned(),
                json!(wait_until_completion),
            );
            insert_some(&mut fields, "uses_pager", *uses_pager);
            insert_some(&mut fields, "rationale", rationale.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::WriteToLongRunningShellCommand {
            block_id,
            input,
            mode,
        } => json!({
            "block_id": serialized(block_id),
            "input": String::from_utf8_lossy(input),
            "mode": match mode {
                AIAgentPtyWriteMode::Raw => "raw",
                AIAgentPtyWriteMode::Line => "line",
                AIAgentPtyWriteMode::Block => "block",
            },
        }),
        AIAgentActionType::ReadFiles(request) => json!({
            "files": request
                .locations
                .iter()
                .map(|location| json!({
                    "path": location.name,
                    "lines": location.lines.iter().map(range_view).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::UploadArtifact(request) => {
            let mut fields = Map::new();
            fields.insert("file_path".to_owned(), json!(request.file_path));
            insert_some(&mut fields, "description", request.description.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::SearchCodebase(request) => {
            let mut fields = Map::new();
            fields.insert("query".to_owned(), json!(request.query));
            insert_some(
                &mut fields,
                "partial_paths",
                request.partial_paths.as_deref(),
            );
            insert_some(
                &mut fields,
                "codebase_path",
                request.codebase_path.as_deref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::RequestFileEdits { file_edits, title } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "title", title.as_deref());
            fields.insert(
                "edits".to_owned(),
                Value::Array(file_edits.iter().map(file_edit_view).collect()),
            );
            Value::Object(fields)
        }
        AIAgentActionType::Grep { queries, path } => json!({ "queries": queries, "path": path }),
        AIAgentActionType::FileGlob { patterns, path } => {
            let mut fields = Map::new();
            fields.insert("patterns".to_owned(), json!(patterns));
            insert_some(&mut fields, "path", path.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::FileGlobV2 {
            patterns,
            search_dir,
        } => {
            let mut fields = Map::new();
            fields.insert("patterns".to_owned(), json!(patterns));
            insert_some(&mut fields, "search_dir", search_dir.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::ReadMCPResource {
            server_id,
            name,
            uri,
        } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "server_id", server_id.map(|id| id.to_string()));
            fields.insert("name".to_owned(), json!(name));
            insert_some(&mut fields, "uri", uri.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::CallMCPTool {
            server_id,
            name,
            input,
        } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "server_id", server_id.map(|id| id.to_string()));
            fields.insert("name".to_owned(), json!(name));
            fields.insert("input".to_owned(), input.clone());
            Value::Object(fields)
        }
        AIAgentActionType::SuggestNewConversation { message_id } => {
            json!({ "message_id": message_id })
        }
        AIAgentActionType::SuggestPrompt(request) => match request {
            SuggestPromptRequest::UnitTestsSuggestion {
                query,
                title,
                description,
            } => json!({
                "kind": "unit_tests_suggestion",
                "query": query,
                "title": title,
                "description": description,
            }),
            SuggestPromptRequest::PromptSuggestion { prompt, label } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("prompt_suggestion"));
                fields.insert("prompt".to_owned(), json!(prompt));
                insert_some(&mut fields, "label", label.as_deref());
                Value::Object(fields)
            }
        },
        AIAgentActionType::InitProject | AIAgentActionType::OpenCodeReview => json!({}),
        AIAgentActionType::ReadDocuments(request) => json!({
            "document_ids": request
                .document_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::EditDocuments(request) => json!({
            "diffs": request
                .diffs
                .iter()
                .map(|diff| json!({
                    "document_id": diff.document_id.to_string(),
                    "search": diff.search,
                    "replace": diff.replace,
                }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::CreateDocuments(request) => json!({
            "documents": request
                .documents
                .iter()
                .map(|document| json!({ "title": document.title, "content": document.content }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::ReadShellCommandOutput { block_id, delay } => {
            let mut fields = Map::new();
            fields.insert("block_id".to_owned(), serialized(block_id));
            let delay = delay.as_ref().map(|delay| match delay {
                ShellCommandDelay::Duration(duration) => {
                    json!({ "seconds": duration.as_secs_f64() })
                }
                ShellCommandDelay::OnCompletion => json!("on_completion"),
            });
            insert_some(&mut fields, "delay", delay);
            Value::Object(fields)
        }
        AIAgentActionType::UseComputer(request) => {
            let mut fields = Map::new();
            fields.insert("action_summary".to_owned(), json!(request.action_summary));
            fields.insert("actions".to_owned(), serialized(&request.actions));
            insert_some(
                &mut fields,
                "screenshot_params",
                request.screenshot_params.as_ref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::InsertCodeReviewComments {
            repo_path,
            comments,
            base_branch,
        } => {
            let mut fields = Map::new();
            fields.insert("repo_path".to_owned(), json!(repo_path.to_string_lossy()));
            insert_some(&mut fields, "base_branch", base_branch.as_deref());
            fields.insert(
                "comments".to_owned(),
                Value::Array(comments.iter().map(review_comment_view).collect()),
            );
            Value::Object(fields)
        }
        AIAgentActionType::RequestComputerUse(request) => {
            let mut fields = Map::new();
            fields.insert("task_summary".to_owned(), json!(request.task_summary));
            insert_some(
                &mut fields,
                "screenshot_params",
                request.screenshot_params.as_ref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::StartRecording {
            frame_rate,
            max_duration,
            max_size_bytes,
            summary,
            description,
            playback_speed_multiplier,
            window,
        } => {
            let mut fields = Map::new();
            fields.insert("frame_rate".to_owned(), json!(frame_rate));
            insert_some(
                &mut fields,
                "max_duration_ms",
                max_duration.map(|duration| duration.as_millis() as u64),
            );
            insert_some(&mut fields, "max_size_bytes", *max_size_bytes);
            insert_some(&mut fields, "summary", summary.as_deref());
            insert_some(&mut fields, "description", description.as_deref());
            insert_some(
                &mut fields,
                "playback_speed_multiplier",
                *playback_speed_multiplier,
            );
            insert_some(&mut fields, "window", window.as_ref());
            Value::Object(fields)
        }
        AIAgentActionType::StopRecording {
            recording_id,
            should_persist,
        } => json!({ "recording_id": recording_id, "should_persist": should_persist }),
        AIAgentActionType::ReadSkill(request) => json!({ "skill": serialized(&request.skill) }),
        AIAgentActionType::FetchConversation { conversation_id } => {
            json!({ "conversation_id": conversation_id })
        }
        AIAgentActionType::SendMessageToAgent {
            addresses,
            subject,
            message,
        } => json!({ "addresses": addresses, "subject": subject, "message": message }),
        AIAgentActionType::TransferShellCommandControlToUser { reason } => {
            json!({ "reason": reason })
        }
        AIAgentActionType::AskUserQuestion { questions } => {
            json!({ "questions": serialized(questions) })
        }
        AIAgentActionType::RunAgents(request) => {
            let execution_mode = match &request.execution_mode {
                RunAgentsExecutionMode::Local => json!("local"),
                RunAgentsExecutionMode::Remote {
                    environment_id,
                    worker_host,
                    computer_use_enabled,
                    runner_id,
                } => json!({
                    "remote": {
                        "environment_id": environment_id,
                        "worker_host": worker_host,
                        "computer_use_enabled": computer_use_enabled,
                        "runner_id": runner_id,
                    }
                }),
            };
            json!({
                "summary": request.summary,
                "base_prompt": request.base_prompt,
                "model_id": request.model_id,
                "harness_type": request.harness_type,
                "execution_mode": execution_mode,
                "plan_id": request.plan_id,
                "skills": serialized(&request.skills),
                "metadata": request.metadata,
                "agents": request
                    .agent_run_configs
                    .iter()
                    .map(|config| json!({
                        "name": config.name,
                        "title": config.title,
                        "prompt": config.prompt,
                        "model_id": config.model_id,
                        "agent_identity_uid": config.agent_identity_uid,
                        "metadata": config.metadata,
                    }))
                    .collect::<Vec<_>>(),
            })
        }
        AIAgentActionType::WaitForEvents {
            tool_call_id,
            idle_timeout_seconds,
        } => json!({
            "tool_call_id": tool_call_id,
            "idle_timeout_seconds": idle_timeout_seconds,
        }),
    }
}

fn file_edit_view(edit: &FileEdit) -> Value {
    match edit {
        FileEdit::Edit(ParsedDiff::StrReplaceEdit {
            file,
            search,
            replace,
        }) => json!({ "kind": "edit", "file": file, "search": search, "replace": replace }),
        FileEdit::Edit(ParsedDiff::V4AEdit {
            file,
            move_to,
            hunks,
        }) => json!({
            "kind": "edit",
            "file": file,
            "move_to": move_to,
            "hunks": serialized(hunks),
        }),
        FileEdit::Create {
            file,
            content,
            allow_overwrite,
        } => json!({
            "kind": "create",
            "file": file,
            "content": content,
            "allow_overwrite": allow_overwrite,
        }),
        FileEdit::Delete { file } => json!({ "kind": "delete", "file": file }),
    }
}

fn review_comment_view(comment: &InsertReviewComment) -> Value {
    let mut fields = Map::new();
    fields.insert("comment_id".to_owned(), json!(comment.comment_id));
    fields.insert("author".to_owned(), json!(comment.author));
    fields.insert(
        "last_modified_timestamp".to_owned(),
        json!(comment.last_modified_timestamp),
    );
    fields.insert("comment_body".to_owned(), json!(comment.comment_body));
    insert_some(
        &mut fields,
        "parent_comment_id",
        comment.parent_comment_id.as_deref(),
    );
    insert_some(&mut fields, "html_url", comment.html_url.as_deref());
    if let Some(location) = &comment.comment_location {
        let mut location_fields = Map::new();
        location_fields.insert(
            "relative_file_path".to_owned(),
            json!(location.relative_file_path),
        );
        if let Some(line) = &location.line {
            let side = line.side.map(|side| match side {
                CommentSide::Right => "right",
                CommentSide::Left => "left",
            });
            let mut line_fields = Map::new();
            line_fields.insert(
                "comment_line_range".to_owned(),
                range_view(&line.comment_line_range),
            );
            line_fields.insert(
                "diff_hunk_line_range".to_owned(),
                range_view(&line.diff_hunk_line_range),
            );
            line_fields.insert("diff_hunk_text".to_owned(), json!(line.diff_hunk_text));
            insert_some(&mut line_fields, "side", side);
            location_fields.insert("line".to_owned(), Value::Object(line_fields));
        }
        fields.insert("location".to_owned(), Value::Object(location_fields));
    }
    Value::Object(fields)
}

fn file_context_view(context: &FileContext) -> Value {
    let mut fields = Map::new();
    fields.insert("file_name".to_owned(), json!(context.file_name));
    match &context.content {
        AnyFileContent::StringContent(content) => {
            fields.insert("content".to_owned(), json!(content));
        }
        AnyFileContent::BinaryContent(bytes) => {
            fields.insert("is_binary".to_owned(), json!(true));
            fields.insert("size_bytes".to_owned(), json!(bytes.len()));
        }
    }
    insert_some(
        &mut fields,
        "line_range",
        context.line_range.as_ref().map(range_view),
    );
    fields.insert("line_count".to_owned(), json!(context.line_count));
    Value::Object(fields)
}

fn file_context_views(contexts: &[FileContext]) -> Value {
    Value::Array(contexts.iter().map(file_context_view).collect())
}

fn document_view(document: &DocumentContext) -> Value {
    json!({
        "document_id": document.document_id.to_string(),
        "version": document.document_version.0,
        "content": document.content,
        "line_ranges": document.line_ranges.iter().map(range_view).collect::<Vec<_>>(),
    })
}

fn document_views(documents: &[DocumentContext]) -> Value {
    Value::Array(documents.iter().map(document_view).collect())
}

fn screenshot_view(screenshot: &computer_use::Screenshot) -> Value {
    json!({
        "width": screenshot.width,
        "height": screenshot.height,
        "original_width": screenshot.original_width,
        "original_height": screenshot.original_height,
        "mime_type": screenshot.mime_type,
        "size_bytes": screenshot.data.len(),
    })
}

fn screenshot_source_view(source: &ScreenshotSource) -> Value {
    match source {
        ScreenshotSource::Inline(screenshot) => screenshot_view(screenshot),
        ScreenshotSource::Stored {
            stored_ref: _,
            mime_type,
            width,
            height,
        } => json!({
            "stored": true,
            "mime_type": mime_type,
            "width": width,
            "height": height,
        }),
    }
}

fn unix_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_millis() as u64)
}

/// A JSON object describing a result, tagged with the variant it came from. Binary payloads are
/// summarized by size; everything else is carried as the result holds it.
fn result_view(result: &AIAgentActionResultType) -> Value {
    match result {
        AIAgentActionResultType::RequestCommandOutput(result) => match result {
            RequestCommandOutputResult::Completed {
                block_id,
                command,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("completed"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("exit_code".to_owned(), json!(exit_code.value()));
                fields.insert("block_id".to_owned(), serialized(block_id));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            RequestCommandOutputResult::LongRunningCommandSnapshot {
                block_id,
                command,
                grid_contents,
                cursor,
                is_alt_screen_active,
                activity,
            } => json!({
                "kind": "long_running_snapshot",
                "command": command,
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "block_id": serialized(block_id),
                "activity": serialized(activity),
            }),
            RequestCommandOutputResult::ShellRecovered {
                block_id,
                command,
                output,
                status,
                restored_working_directory,
                used_fallback_directory,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("shell_recovered"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("status".to_owned(), serialized(status));
                fields.insert(
                    "restored_working_directory".to_owned(),
                    json!(restored_working_directory),
                );
                fields.insert(
                    "used_fallback_directory".to_owned(),
                    json!(used_fallback_directory),
                );
                fields.insert("block_id".to_owned(), serialized(block_id));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            RequestCommandOutputResult::CancelledBeforeExecution => {
                json!({ "kind": "cancelled_before_execution" })
            }
            RequestCommandOutputResult::Denylisted { command } => {
                json!({ "kind": "denylisted", "command": command })
            }
            RequestCommandOutputResult::HookDenied { command, reason } => {
                json!({ "kind": "hook_denied", "command": command, "reason": reason })
            }
            RequestCommandOutputResult::TerminalBusy { command, block_id } => json!({
                "kind": "terminal_busy",
                "command": command,
                "block_id": serialized(block_id),
            }),
        },
        AIAgentActionResultType::WriteToLongRunningShellCommand(result) => match result {
            WriteToLongRunningShellCommandResult::Snapshot {
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => json!({
                "kind": "snapshot",
                "block_id": serialized(block_id),
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "is_preempted": is_preempted,
                "activity": serialized(activity),
            }),
            WriteToLongRunningShellCommandResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                None,
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            WriteToLongRunningShellCommandResult::Cancelled => json!({ "kind": "cancelled" }),
            WriteToLongRunningShellCommandResult::Error(error) => {
                json!({ "kind": "error", "error": serialized(error) })
            }
        },
        AIAgentActionResultType::RequestFileEdits(result) => match result {
            RequestFileEditsResult::Success {
                diff,
                updated_files,
                deleted_files,
                lines_added,
                lines_removed,
            } => json!({
                "kind": "success",
                "diff": diff,
                "updated_files": updated_files
                    .iter()
                    .map(|updated| json!({
                        "was_edited_by_user": updated.was_edited_by_user,
                        "file": file_context_view(&updated.file_context),
                    }))
                    .collect::<Vec<_>>(),
                "deleted_files": deleted_files,
                "lines_added": lines_added,
                "lines_removed": lines_removed,
            }),
            RequestFileEditsResult::Cancelled => json!({ "kind": "cancelled" }),
            RequestFileEditsResult::DiffApplicationFailed { error } => {
                json!({ "kind": "diff_application_failed", "error": error })
            }
        },
        AIAgentActionResultType::ReadFiles(result) => match result {
            ReadFilesResult::Success {
                files,
                failed_files,
            } => json!({
                "kind": "success",
                "files": file_context_views(files),
                "failed_files": serialized(failed_files),
            }),
            ReadFilesResult::Error(error) => error_view(error),
            ReadFilesResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::UploadArtifact(result) => match result {
            UploadArtifactResult::Success {
                artifact_uid,
                filepath,
                mime_type,
                description,
                size_bytes,
            } => json!({
                "kind": "success",
                "artifact_uid": artifact_uid,
                "filepath": filepath,
                "mime_type": mime_type,
                "description": description,
                "size_bytes": size_bytes,
            }),
            UploadArtifactResult::Error(error) => error_view(error),
            UploadArtifactResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SearchCodebase(result) => match result {
            SearchCodebaseResult::Success { files } => {
                json!({ "kind": "success", "files": file_context_views(files) })
            }
            SearchCodebaseResult::Failed { reason, message } => json!({
                "kind": "failed",
                "reason": format!("{reason:?}"),
                "message": message,
            }),
            SearchCodebaseResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::Grep(result) => match result {
            GrepResult::Success { matched_files } => {
                json!({ "kind": "success", "matched_files": serialized(matched_files) })
            }
            GrepResult::Error(error) => error_view(error),
            GrepResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::FileGlob(result) => match result {
            FileGlobResult::Success { matched_files } => {
                json!({ "kind": "success", "matched_files": matched_files })
            }
            FileGlobResult::Error(error) => error_view(error),
            FileGlobResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::FileGlobV2(result) => match result {
            FileGlobV2Result::Success {
                matched_files,
                warnings,
            } => json!({
                "kind": "success",
                "matched_files": serialized(matched_files),
                "warnings": warnings,
            }),
            FileGlobV2Result::Error(error) => error_view(error),
            FileGlobV2Result::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadMCPResource(result) => match result {
            ReadMCPResourceResult::Success { resource_contents } => json!({
                "kind": "success",
                "resource_contents": serialized(resource_contents),
            }),
            ReadMCPResourceResult::Error(error) => error_view(error),
            ReadMCPResourceResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::CallMCPTool(result) => match result {
            CallMCPToolResult::Success { result } => {
                json!({ "kind": "success", "result": serialized(result) })
            }
            CallMCPToolResult::Error(error) => error_view(error),
            CallMCPToolResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadSkill(result) => match result {
            ReadSkillResult::Success { content } => {
                json!({ "kind": "success", "content": file_context_view(content) })
            }
            ReadSkillResult::Error(error) => error_view(error),
            ReadSkillResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SuggestNewConversation(result) => match result {
            SuggestNewConversationResult::Accepted { message_id } => {
                json!({ "kind": "accepted", "message_id": message_id })
            }
            SuggestNewConversationResult::Rejected => json!({ "kind": "rejected" }),
            SuggestNewConversationResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SuggestPrompt(result) => match result {
            SuggestPromptResult::Accepted { query } => {
                json!({ "kind": "accepted", "query": query })
            }
            SuggestPromptResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::OpenCodeReview | AIAgentActionResultType::InitProject => {
            json!({ "kind": "completed" })
        }
        AIAgentActionResultType::ReadDocuments(result) => match result {
            ReadDocumentsResult::Success { documents } => {
                json!({ "kind": "success", "documents": document_views(documents) })
            }
            ReadDocumentsResult::Error(error) => error_view(error),
            ReadDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::EditDocuments(result) => match result {
            EditDocumentsResult::Success { updated_documents } => json!({
                "kind": "success",
                "updated_documents": document_views(updated_documents),
            }),
            EditDocumentsResult::Error(error) => error_view(error),
            EditDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::CreateDocuments(result) => match result {
            CreateDocumentsResult::Success { created_documents } => json!({
                "kind": "success",
                "created_documents": document_views(created_documents),
            }),
            CreateDocumentsResult::Error(error) => error_view(error),
            CreateDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadShellCommandOutput(result) => match result {
            ReadShellCommandOutputResult::CommandFinished {
                command,
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                Some(command),
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => json!({
                "kind": "long_running_snapshot",
                "command": command,
                "block_id": serialized(block_id),
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "is_preempted": is_preempted,
                "activity": serialized(activity),
            }),
            ReadShellCommandOutputResult::ShellRecovered {
                command,
                block_id,
                output,
                status,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("shell_recovered"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("block_id".to_owned(), serialized(block_id));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("status".to_owned(), serialized(status));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            ReadShellCommandOutputResult::Cancelled => json!({ "kind": "cancelled" }),
            ReadShellCommandOutputResult::Error(error) => {
                json!({ "kind": "error", "error": serialized(error) })
            }
        },
        AIAgentActionResultType::UseComputer(result) => match result {
            UseComputerResult::Success {
                screenshot,
                cursor_position,
                windows,
                captured_window,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("success"));
                insert_some(
                    &mut fields,
                    "screenshot",
                    screenshot.as_ref().map(screenshot_source_view),
                );
                insert_some(
                    &mut fields,
                    "cursor_position",
                    cursor_position.map(|position| json!([position.x(), position.y()])),
                );
                fields.insert("windows".to_owned(), serialized(windows));
                insert_some(
                    &mut fields,
                    "captured_window",
                    captured_window.map(|window| {
                        json!({
                            "window_id": window.window_id,
                            "width_px": window.width_px,
                            "height_px": window.height_px,
                        })
                    }),
                );
                Value::Object(fields)
            }
            UseComputerResult::Error(error) => error_view(error),
            UseComputerResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::InsertReviewComments(result) => match result {
            InsertReviewCommentsResult::Success { repo_path } => {
                json!({ "kind": "success", "repo_path": repo_path })
            }
            InsertReviewCommentsResult::Error { repo_path, message } => {
                json!({ "kind": "error", "repo_path": repo_path, "error": message })
            }
            InsertReviewCommentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::RequestComputerUse(result) => match result {
            RequestComputerUseResult::Approved {
                screenshot,
                platform,
                windows,
            } => json!({
                "kind": "approved",
                "screenshot": screenshot_view(screenshot),
                "platform": format!("{platform:?}"),
                "windows": serialized(windows),
            }),
            RequestComputerUseResult::Error(error) => error_view(error),
            RequestComputerUseResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::StartRecording(result) => match result {
            StartRecordingResult::Success(started) => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("success"));
                fields.insert("recording_id".to_owned(), json!(started.recording_id));
                insert_some(
                    &mut fields,
                    "started_at_ms",
                    unix_millis(started.started_at),
                );
                fields.insert("width_px".to_owned(), json!(started.width_px));
                fields.insert("height_px".to_owned(), json!(started.height_px));
                Value::Object(fields)
            }
            StartRecordingResult::Error(error) => error_view(error),
            StartRecordingResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::StopRecording(result) => match result {
            StopRecordingResult::Success(stopped) => json!({
                "kind": "success",
                "artifact_uid": stopped.artifact_uid,
                "duration_ms": stopped.duration.as_millis() as u64,
                "width_px": stopped.width_px,
                "height_px": stopped.height_px,
                "size_bytes": stopped.size_bytes,
                "completion_status": format!("{:?}", stopped.completion_status),
                "termination_reason": stopped.termination_reason,
            }),
            StopRecordingResult::Error(error) => error_view(error),
            StopRecordingResult::Cancelled => json!({ "kind": "cancelled" }),
            StopRecordingResult::Discarded => json!({ "kind": "discarded" }),
        },
        AIAgentActionResultType::FetchConversation(result) => match result {
            FetchConversationResult::Success { directory_path } => {
                json!({ "kind": "success", "directory_path": directory_path })
            }
            FetchConversationResult::Error(error) => error_view(error),
            FetchConversationResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SendMessageToAgent(result) => match result {
            SendMessageToAgentResult::Success { message_id } => {
                json!({ "kind": "success", "message_id": message_id })
            }
            SendMessageToAgentResult::Error(error) => error_view(error),
            SendMessageToAgentResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::TransferShellCommandControlToUser(result) => match result {
            TransferShellCommandControlToUserResult::Snapshot {
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => json!({
                "kind": "snapshot",
                "block_id": serialized(block_id),
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "is_preempted": is_preempted,
                "activity": serialized(activity),
            }),
            TransferShellCommandControlToUserResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                None,
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            TransferShellCommandControlToUserResult::Cancelled => json!({ "kind": "cancelled" }),
            TransferShellCommandControlToUserResult::Error(error) => {
                json!({ "kind": "error", "error": serialized(error) })
            }
        },
        AIAgentActionResultType::AskUserQuestion(result) => match result {
            AskUserQuestionResult::Success { answers } => json!({
                "kind": "success",
                "answers": answers
                    .iter()
                    .map(|answer| match answer {
                        AskUserQuestionAnswerItem::Answered {
                            question_id,
                            selected_options,
                            other_text,
                        } => json!({
                            "question_id": question_id,
                            "selected_options": selected_options,
                            "other_text": other_text,
                        }),
                        AskUserQuestionAnswerItem::Skipped { question_id } => {
                            json!({ "question_id": question_id, "skipped": true })
                        }
                    })
                    .collect::<Vec<_>>(),
            }),
            AskUserQuestionResult::Error(error) => error_view(error),
            AskUserQuestionResult::Cancelled => json!({ "kind": "cancelled" }),
            AskUserQuestionResult::SkippedByAutoApprove { question_ids } => json!({
                "kind": "skipped_by_auto_approve",
                "question_ids": question_ids,
            }),
        },
        AIAgentActionResultType::RunAgents(result) => match result {
            RunAgentsResult::Launched {
                model_id,
                harness_type,
                execution_mode,
                agents,
            } => json!({
                "kind": "launched",
                "model_id": model_id,
                "harness_type": harness_type,
                "execution_mode": serialized(execution_mode),
                "agents": serialized(agents),
            }),
            RunAgentsResult::Denied { reason } => json!({ "kind": "denied", "reason": reason }),
            RunAgentsResult::Failure { error } => json!({ "kind": "failure", "error": error }),
            RunAgentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::WaitForEvents(result) => match result {
            WaitForEventsResult::Completed => json!({ "kind": "completed" }),
            WaitForEventsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
    }
}

fn error_view(error: &str) -> Value {
    json!({ "kind": "error", "error": error })
}

fn finished_command_view(
    block_id: &BlockId,
    command: Option<&str>,
    output: &str,
    exit_code: i32,
    start_ts: Option<DateTime<Local>>,
    completed_ts: Option<DateTime<Local>>,
) -> Value {
    let mut fields = Map::new();
    fields.insert("kind".to_owned(), json!("command_finished"));
    insert_some(&mut fields, "command", command);
    fields.insert("block_id".to_owned(), serialized(block_id));
    fields.insert("output".to_owned(), json!(output));
    fields.insert("exit_code".to_owned(), json!(exit_code));
    insert_some(&mut fields, "start_ts", timestamp(start_ts));
    insert_some(&mut fields, "completed_ts", timestamp(completed_ts));
    Value::Object(fields)
}

#[cfg(test)]
#[path = "encode_tests.rs"]
mod tests;
