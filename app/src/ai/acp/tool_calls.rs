use agent_client_protocol as acp;

use crate::ai::agent::local_acp_tool_call::{
    LocalAcpToolCallMessage, LocalAcpToolCallStatus, LocalAcpToolKind,
};
use crate::ai::agent::{AIAgentText, AIAgentTextSection, AgentOutputText};

pub(crate) fn message_from_tool_call(tool_call: acp::ToolCall) -> LocalAcpToolCallMessage {
    let body = body_from_tool_call(&tool_call);
    LocalAcpToolCallMessage {
        tool_call_id: tool_call.tool_call_id.to_string(),
        title: tool_call.title,
        kind: map_tool_kind(tool_call.kind),
        status: map_tool_status(tool_call.status),
        body,
        locations: tool_call
            .locations
            .iter()
            .map(|location| location.path.display().to_string())
            .collect(),
    }
}

pub(crate) fn apply_tool_call_update(
    message: &mut LocalAcpToolCallMessage,
    update: &acp::ToolCallUpdate,
) {
    let fields = &update.fields;
    if let Some(title) = &fields.title {
        message.title = title.clone();
    }
    if let Some(kind) = fields.kind {
        message.kind = map_tool_kind(kind);
    }
    if let Some(status) = fields.status {
        message.status = map_tool_status(status);
    }
    if let Some(locations) = &fields.locations {
        message.locations = locations
            .iter()
            .map(|location| location.path.display().to_string())
            .collect();
    }
    if let Some(content) = &fields.content {
        message.body = body_from_content(content);
    } else if fields.raw_output.is_some() || fields.raw_input.is_some() {
        message.body = body_from_tool_fields(fields.raw_input.as_ref(), fields.raw_output.as_ref());
    }
}

fn map_tool_kind(kind: acp::ToolKind) -> LocalAcpToolKind {
    match kind {
        acp::ToolKind::Read => LocalAcpToolKind::Read,
        acp::ToolKind::Edit => LocalAcpToolKind::Edit,
        acp::ToolKind::Delete => LocalAcpToolKind::Delete,
        acp::ToolKind::Move => LocalAcpToolKind::Move,
        acp::ToolKind::Search => LocalAcpToolKind::Search,
        acp::ToolKind::Execute => LocalAcpToolKind::Execute,
        acp::ToolKind::Think => LocalAcpToolKind::Think,
        acp::ToolKind::Fetch => LocalAcpToolKind::Fetch,
        acp::ToolKind::SwitchMode => LocalAcpToolKind::SwitchMode,
        acp::ToolKind::Other => LocalAcpToolKind::Other,
        _ => LocalAcpToolKind::Other,
    }
}

fn map_tool_status(status: acp::ToolCallStatus) -> LocalAcpToolCallStatus {
    match status {
        acp::ToolCallStatus::Pending => LocalAcpToolCallStatus::Pending,
        acp::ToolCallStatus::InProgress => LocalAcpToolCallStatus::InProgress,
        acp::ToolCallStatus::Completed => LocalAcpToolCallStatus::Completed,
        acp::ToolCallStatus::Failed => LocalAcpToolCallStatus::Failed,
        _ => LocalAcpToolCallStatus::Pending,
    }
}

fn body_from_tool_call(tool_call: &acp::ToolCall) -> AIAgentText {
    if !tool_call.content.is_empty() {
        return body_from_content(&tool_call.content);
    }

    body_from_tool_fields(tool_call.raw_input.as_ref(), tool_call.raw_output.as_ref())
}

fn body_from_tool_fields(
    raw_input: Option<&serde_json::Value>,
    raw_output: Option<&serde_json::Value>,
) -> AIAgentText {
    let mut sections = Vec::new();
    if let Some(input) = raw_input.and_then(format_json_value) {
        sections.push(text_section(format!("**Input**\n\n{}", fenced_code_block("json", &input))));
    }
    if let Some(output) = raw_output.and_then(format_json_value) {
        sections.push(text_section(format!(
            "**Output**\n\n{}",
            fenced_code_block("json", &output)
        )));
    }
    AIAgentText { sections }
}

fn body_from_content(content: &[acp::ToolCallContent]) -> AIAgentText {
    let mut sections = Vec::new();
    for item in content {
        match item {
            acp::ToolCallContent::Content(content) => match &content.content {
                acp::ContentBlock::Text(text) if !text.text.is_empty() => {
                    sections.push(text_section(format_tool_body_text(text.text.clone())));
                }
                _ => {}
            },
            acp::ToolCallContent::Diff(diff) => {
                let path = diff.path.display();
                if let Some(old_text) = &diff.old_text {
                    sections.push(text_section(format!(
                        "**{path}**\n```diff\n{old_text}\n---\n{}\n```",
                        diff.new_text
                    )));
                } else {
                    sections.push(text_section(format!(
                        "**{path}**\n```\n{}\n```",
                        diff.new_text
                    )));
                }
            }
            acp::ToolCallContent::Terminal(_) => {}
            _ => {}
        }
    }
    AIAgentText { sections }
}

fn text_section(text: String) -> AIAgentTextSection {
    AIAgentTextSection::PlainText {
        text: AgentOutputText::from(text),
    }
}

fn format_tool_body_text(text: String) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        return text;
    }
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return fenced_code_block("json", trimmed);
    }
    text
}

fn fenced_code_block(language: &str, content: &str) -> String {
    format!("```{language}\n{content}\n```")
}

fn format_json_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.is_empty() => Some(text.clone()),
        serde_json::Value::Null => None,
        other => serde_json::to_string_pretty(other).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tool_call_with_text_content() {
        let tool_call = acp::ToolCall::new("tool-1", "Read file")
            .kind(acp::ToolKind::Read)
            .status(acp::ToolCallStatus::Completed)
            .content(vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                acp::TextContent::new("hello"),
            ))]);

        let message = message_from_tool_call(tool_call);
        assert_eq!(message.tool_call_id, "tool-1");
        assert_eq!(message.title, "Read file");
        assert_eq!(message.kind, LocalAcpToolKind::Read);
        assert_eq!(message.status, LocalAcpToolCallStatus::Completed);
        assert!(message.body_plain_text().contains("hello"));
    }

    #[test]
    fn wraps_json_tool_output_in_code_fence() {
        let tool_call = acp::ToolCall::new("tool-2", "grep")
            .content(vec![acp::ToolCallContent::from(acp::ContentBlock::Text(
                acp::TextContent::new(r#"{"success":true}"#),
            ))]);

        let message = message_from_tool_call(tool_call);
        assert!(message.body_plain_text().contains("```json"));
        assert!(message.body_plain_text().contains(r#""success":true"#));
    }
}
