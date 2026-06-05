use serde::Deserialize;

use super::{CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType};
use crate::terminal::CLIAgent;

/// Resolves a CLI agent from the `"agent"` string in a CLI agent event.
/// Returns `None` if the string doesn't match any known agent.
fn resolve_agent(agent: &str) -> Option<CLIAgent> {
    enum_iterator::all::<CLIAgent>()
        .find(|a| !matches!(a, CLIAgent::Unknown) && a.command_prefix() == agent)
}

pub(super) fn parse(body: &str) -> Option<CLIAgentEvent> {
    let raw: RawEvent = serde_json::from_str(body).ok()?;

    let event = match raw.event.as_str() {
        "session_start" => CLIAgentEventType::SessionStart,
        "prompt_submit" => CLIAgentEventType::PromptSubmit,
        "tool_complete" => CLIAgentEventType::ToolComplete,
        "stop" => CLIAgentEventType::Stop,
        "permission_request" => CLIAgentEventType::PermissionRequest,
        "permission_replied" => CLIAgentEventType::PermissionReplied,
        "question_asked" => CLIAgentEventType::QuestionAsked,
        "idle_prompt" => CLIAgentEventType::IdlePrompt,
        other => CLIAgentEventType::Unknown(other.to_string()),
    };

    let tool_input_preview = raw.tool_input.as_ref().and_then(|val| {
        val.get("command")
            .or_else(|| val.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });

    let agent = raw
        .agent
        .as_deref()
        .and_then(resolve_agent)
        .unwrap_or(CLIAgent::Unknown);

    Some(CLIAgentEvent {
        v: raw.v.unwrap_or(1),
        agent,
        event,
        session_id: raw.session_id,
        cwd: raw.cwd,
        project: raw.project,
        payload: CLIAgentEventPayload {
            query: raw.query,
            response: raw.response,
            transcript_path: raw.transcript_path,
            summary: raw.summary,
            tool_name: raw.tool_name,
            tool_input_preview,
            plugin_version: raw.plugin_version,
        },
        source: CLIAgentEventSource::RichPlugin,
    })
}

#[derive(Deserialize)]
struct RawEvent {
    v: Option<u32>,
    agent: Option<String>,
    event: String,
    session_id: Option<String>,
    cwd: Option<String>,
    project: Option<String>,
    query: Option<String>,
    response: Option<String>,
    transcript_path: Option<String>,
    summary: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    plugin_version: Option<String>,
}
