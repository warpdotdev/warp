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

use self::tool_input::tool_input;
use self::tool_output::result_view;
use super::events::{CursorEvent, CursorEventSpec};
use super::facets::{McpCallOutcome, finished_shell_command, mcp_tool_call, pending_shell_command};
use crate::agent::action_result::{
    AIAgentActionResultType, RequestCommandOutputResult, RunAgentsResult,
};
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
        attachments_summary,
        provenance: _,
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

/// Inserts `value` only when present. Optional fields go through this rather than `json!`, which
/// would write `null` for a missing value; an arm with no optional field can use `json!` directly.
fn insert_some(fields: &mut Map<String, Value>, key: &str, value: Option<impl Serialize>) {
    if let Some(value) = value {
        fields.insert(key.to_owned(), serialized(key, &value));
    }
}

fn range_view(range: &Range<usize>) -> Value {
    json!([range.start, range.end])
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
