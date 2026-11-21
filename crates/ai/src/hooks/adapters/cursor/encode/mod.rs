//! Turns an envelope into the JSON object Cursor documents for the event: the common fields every
//! hook receives, then the event's own fields. Values Warp does not know are left out rather than
//! invented; the two exceptions are `transcript_path`, which Cursor's schema makes `null`, and
//! `model_params`, which is an empty list because Warp has no structured model parameters.

mod tool_input;
mod tool_output;

use std::ops::Range;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use self::tool_input::{mcp_server_id, part_tool_input};
use self::tool_output::part_result_view;
use super::events::{CursorEvent, CursorEventSpec, SUBAGENT_TYPE};
use super::facets::{
    McpCallOutcome, finished_shell_command, mcp_tool_call, pending_shell_command, subagent_launch,
};
use super::parts::{ToolPart, tool_parts};
use crate::agent::action_result::{
    AIAgentActionResultType, RequestCommandOutputResult, RunAgentsResult,
};
use crate::hooks::adapters::HookInput;
use crate::hooks::model::{
    HookEnvelope, HookPayload, PromptAttachmentKind, SubagentExecutionMode, TurnStopStatus,
};

/// The stdin payloads for one firing of `spec`'s event: one per part of a tool action that
/// `part_matches` accepts (a Cursor script sees a batched read or edit as one event per file), and
/// one for every other event. Inputs are labeled by their file only when the action was split.
pub(super) fn encode_inputs(
    spec: &CursorEventSpec,
    envelope: &HookEnvelope,
    part_matches: impl Fn(&ToolPart<'_>) -> bool,
) -> Vec<HookInput> {
    let action = match spec.event {
        CursorEvent::PreToolUse | CursorEvent::PostToolUse | CursorEvent::PostToolUseFailure => {
            envelope.payload.action()
        }
        CursorEvent::SessionStart
        | CursorEvent::BeforeShellExecution
        | CursorEvent::AfterShellExecution
        | CursorEvent::BeforeMcpExecution
        | CursorEvent::AfterMcpExecution
        | CursorEvent::BeforeSubmitPrompt
        | CursorEvent::Stop
        | CursorEvent::AfterAgentResponse
        | CursorEvent::AfterAgentThought
        | CursorEvent::SessionEnd
        | CursorEvent::SubagentStart
        | CursorEvent::SubagentStop
        | CursorEvent::BeforeReadFile
        | CursorEvent::AfterFileEdit
        | CursorEvent::PreCompact
        | CursorEvent::BeforeTabFileRead
        | CursorEvent::AfterTabFileEdit
        | CursorEvent::WorkspaceOpen => None,
    };
    let Some(action) = action else {
        return vec![HookInput::whole(encode_part(spec, envelope, None))];
    };
    let parts = tool_parts(action);
    let split = parts.len() > 1;
    parts
        .iter()
        .filter(|part| part_matches(part))
        .map(|part| HookInput {
            label: split.then(|| part.file().map(str::to_owned)).flatten(),
            payload: encode_part(spec, envelope, Some(part)),
        })
        .collect()
}

/// The one stdin payload for a firing that has one part, which is every firing of an event other
/// than the generic tool events and every tool event whose action is not split.
#[cfg(test)]
pub(super) fn encode_input(spec: &CursorEventSpec, envelope: &HookEnvelope) -> Value {
    let mut inputs = encode_inputs(spec, envelope, |_| true);
    assert_eq!(inputs.len(), 1, "{} has one part", spec.name);
    inputs.remove(0).payload
}

/// The payload for `spec`'s event, about `part` when the event is one of the generic tool
/// events (and the envelope carries a tool action).
fn encode_part(
    spec: &CursorEventSpec,
    envelope: &HookEnvelope,
    part: Option<&ToolPart<'_>>,
) -> Value {
    let mut input = common_fields(spec, envelope);
    match spec.event {
        CursorEvent::SessionStart => encode_session_start(envelope, &mut input),
        CursorEvent::PreToolUse => encode_pre_tool_use(envelope, part, &mut input),
        CursorEvent::PostToolUse => encode_post_tool_use(envelope, part, &mut input),
        CursorEvent::PostToolUseFailure => encode_post_tool_use_failure(envelope, part, &mut input),
        CursorEvent::BeforeShellExecution => encode_before_shell_execution(envelope, &mut input),
        CursorEvent::AfterShellExecution => encode_after_shell_execution(envelope, &mut input),
        CursorEvent::BeforeMcpExecution => encode_before_mcp_execution(envelope, &mut input),
        CursorEvent::AfterMcpExecution => encode_after_mcp_execution(envelope, &mut input),
        CursorEvent::BeforeSubmitPrompt => encode_before_submit_prompt(envelope, &mut input),
        CursorEvent::SubagentStart => encode_subagent_start(envelope, &mut input),
        CursorEvent::Stop => encode_stop(envelope, &mut input),
        CursorEvent::AfterAgentResponse => encode_after_agent_response(envelope, &mut input),
        CursorEvent::AfterAgentThought => encode_after_agent_thought(envelope, &mut input),
        // Warp does not fire these yet, so there is no payload to encode.
        CursorEvent::SessionEnd
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

/// The generic tool events' fields for one part of the action: Cursor's `tool_name` and
/// `tool_input`, plus the MCP server for an MCP action, named beside the input as the MCP events
/// name it.
fn insert_tool_fields(
    part: &ToolPart<'_>,
    envelope: &HookEnvelope,
    input: &mut Map<String, Value>,
) {
    input.insert("tool_name".to_owned(), json!(part.tool_name()));
    input.insert("tool_input".to_owned(), part_tool_input(part, envelope));
    if let ToolPart::Whole(action) = part
        && let Some(server_id) = mcp_server_id(action)
    {
        input.insert("mcp_server_id".to_owned(), json!(server_id.to_string()));
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

fn encode_pre_tool_use(
    envelope: &HookEnvelope,
    part: Option<&ToolPart<'_>>,
    input: &mut Map<String, Value>,
) {
    let (HookPayload::ToolPre { .. }, Some(part)) = (&envelope.payload, part) else {
        return;
    };
    insert_tool_fields(part, envelope, input);
    insert_tool_use_id(envelope, input);
    insert_cwd(envelope, input);
}

fn encode_post_tool_use(
    envelope: &HookEnvelope,
    part: Option<&ToolPart<'_>>,
    input: &mut Map<String, Value>,
) {
    let (
        HookPayload::ToolResult {
            result, duration, ..
        },
        Some(part),
    ) = (&envelope.payload, part)
    else {
        return;
    };
    insert_tool_fields(part, envelope, input);
    input.insert(
        "tool_output".to_owned(),
        json!(part_result_view(part, result).to_string()),
    );
    insert_tool_use_id(envelope, input);
    insert_cwd(envelope, input);
    insert_duration_ms("duration", *duration, input);
}

fn encode_post_tool_use_failure(
    envelope: &HookEnvelope,
    part: Option<&ToolPart<'_>>,
    input: &mut Map<String, Value>,
) {
    let (
        HookPayload::ToolFailure {
            result,
            cancellation_reason,
            duration,
            ..
        },
        Some(part),
    ) = (&envelope.payload, part)
    else {
        return;
    };
    insert_tool_fields(part, envelope, input);
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
        ) | AIAgentActionResultType::RunAgents(RunAgentsResult::Denied { .. })
            | AIAgentActionResultType::DeniedByHook { .. }
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
    if let Some(outcome) = call.outcome {
        let result_json = match outcome {
            McpCallOutcome::Returned(result) => serialized("result_json", result),
            McpCallOutcome::Failed(message) => json!({ "error": message }),
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
        attachments,
        provenance: _,
    } = &envelope.payload
    else {
        return;
    };
    input.insert("prompt".to_owned(), json!(text));
    let attachments = attachments
        .iter()
        .map(|attachment| {
            let kind = match attachment.kind {
                PromptAttachmentKind::File => "file",
                PromptAttachmentKind::Rule => "rule",
            };
            json!({ "type": kind, "file_path": attachment.path.to_string_lossy() })
        })
        .collect::<Vec<_>>();
    input.insert("attachments".to_owned(), Value::Array(attachments));
}

/// Cursor's fields for a subagent about to start, plus Warp's own: the child's name, its harness,
/// and whether it runs here or as a cloud run. `git_branch` is left out because Warp makes no
/// branch for a child at launch.
fn encode_subagent_start(envelope: &HookEnvelope, input: &mut Map<String, Value>) {
    let Some(launch) = subagent_launch(&envelope.payload) else {
        return;
    };
    input.insert(
        "subagent_id".to_owned(),
        json!(launch.subagent_id.to_string()),
    );
    input.insert("subagent_type".to_owned(), json!(SUBAGENT_TYPE));
    input.insert("task".to_owned(), json!(launch.task));
    input.insert(
        "parent_conversation_id".to_owned(),
        json!(envelope.conversation_id.to_string()),
    );
    if let Some(action_id) = &envelope.action_id {
        input.insert("tool_call_id".to_owned(), json!(action_id.to_string()));
    }
    insert_some(input, "subagent_model", launch.model_id.as_deref());
    input.insert(
        "is_parallel_worker".to_owned(),
        json!(launch.is_parallel_worker),
    );
    input.insert("subagent_name".to_owned(), json!(launch.name));
    input.insert("harness".to_owned(), json!(launch.harness));
    let execution_mode = match launch.execution_mode {
        SubagentExecutionMode::Local => "local",
        SubagentExecutionMode::Remote => "remote",
    };
    input.insert("execution_mode".to_owned(), json!(execution_mode));
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
        | HookPayload::SubagentStart { .. }
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

/// `value` as JSON, for the payload field `field`. The types that reach this all serialize, so a
/// failure is a bug; it is logged and the field becomes `null` rather than failing the hook.
fn serialized<T: Serialize>(field: &str, value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|error| {
        log::warn!(
            "hook payload field {field} could not be serialized from {}: {error}",
            std::any::type_name::<T>()
        );
        Value::Null
    })
}

/// Inserts `value` only when present, so an optional field is omitted rather than written as
/// `null`.
fn insert_some(fields: &mut Map<String, Value>, key: &str, value: Option<impl Serialize>) {
    if let Some(value) = value {
        fields.insert(key.to_owned(), serialized(key, &value));
    }
}

fn range_view(range: &Range<usize>) -> Value {
    json!([range.start, range.end])
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
