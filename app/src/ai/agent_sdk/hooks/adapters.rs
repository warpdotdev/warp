use super::MAX_TOOL_INPUT_BYTES;
use super::redaction::{HookRedactor, RedactedText, RedactedValue};
use crate::ai::agent::{
    AIAgentActionResultType, AIAgentActionType, AIAgentActionTypeDiscriminants,
};
const MAX_METADATA_TEXT_BYTES: usize = 8 * 1024;
const MAX_METADATA_ITEMS: usize = 128;

pub(crate) fn local_action_payload(
    action: &AIAgentActionType,
    redactor: &HookRedactor,
) -> (&'static str, RedactedValue) {
    let tool_name = match AIAgentActionTypeDiscriminants::from(action) {
        AIAgentActionTypeDiscriminants::RequestCommandOutput => "run_shell_command",
        AIAgentActionTypeDiscriminants::WriteToLongRunningShellCommand => {
            "write_to_long_running_shell_command"
        }
        AIAgentActionTypeDiscriminants::ReadFiles => "read_files",
        AIAgentActionTypeDiscriminants::UploadArtifact => "upload_artifact",
        AIAgentActionTypeDiscriminants::SearchCodebase => "search_codebase",
        AIAgentActionTypeDiscriminants::RequestFileEdits => "request_file_edits",
        AIAgentActionTypeDiscriminants::Grep => "grep",
        AIAgentActionTypeDiscriminants::FileGlob => "file_glob",
        AIAgentActionTypeDiscriminants::FileGlobV2 => "file_glob",
        AIAgentActionTypeDiscriminants::ReadMCPResource => "read_mcp_resource",
        AIAgentActionTypeDiscriminants::CallMCPTool => "call_mcp_tool",
        AIAgentActionTypeDiscriminants::SuggestNewConversation => "suggest_new_conversation",
        AIAgentActionTypeDiscriminants::SuggestPrompt => "suggest_prompt",
        AIAgentActionTypeDiscriminants::InitProject => "init_project",
        AIAgentActionTypeDiscriminants::OpenCodeReview => "open_code_review",
        AIAgentActionTypeDiscriminants::ReadDocuments => "read_documents",
        AIAgentActionTypeDiscriminants::EditDocuments => "edit_documents",
        AIAgentActionTypeDiscriminants::CreateDocuments => "create_documents",
        AIAgentActionTypeDiscriminants::ReadShellCommandOutput => "read_shell_command_output",
        AIAgentActionTypeDiscriminants::UseComputer => "use_computer",
        AIAgentActionTypeDiscriminants::InsertCodeReviewComments => "insert_code_review_comments",
        AIAgentActionTypeDiscriminants::RequestComputerUse => "request_computer_use",
        AIAgentActionTypeDiscriminants::StartRecording => "start_recording",
        AIAgentActionTypeDiscriminants::StopRecording => "stop_recording",
        AIAgentActionTypeDiscriminants::ReadSkill => "read_skill",
        AIAgentActionTypeDiscriminants::FetchConversation => "fetch_conversation",
        AIAgentActionTypeDiscriminants::SendMessageToAgent => "send_message_to_agent",
        AIAgentActionTypeDiscriminants::TransferShellCommandControlToUser => {
            "transfer_shell_command_control_to_user"
        }
        AIAgentActionTypeDiscriminants::AskUserQuestion => "ask_user_question",
        AIAgentActionTypeDiscriminants::RunAgents => "run_agents",
        AIAgentActionTypeDiscriminants::WaitForEvents => "wait_for_events",
    };
    let input = match action {
        AIAgentActionType::RequestCommandOutput {
            command,
            is_read_only,
            is_risky,
            wait_until_completion,
            uses_pager,
            ..
        } => RedactedValue::object([
            ("command", redacted_text(redactor, command)),
            (
                "is_read_only",
                is_read_only.map_or(RedactedValue::Null, RedactedValue::Bool),
            ),
            (
                "is_risky",
                is_risky.map_or(RedactedValue::Null, RedactedValue::Bool),
            ),
            (
                "wait_until_completion",
                RedactedValue::Bool(*wait_until_completion),
            ),
            (
                "uses_pager",
                uses_pager.map_or(RedactedValue::Null, RedactedValue::Bool),
            ),
        ]),
        AIAgentActionType::WriteToLongRunningShellCommand { input, mode, .. } => {
            RedactedValue::object([
                (
                    "input",
                    redacted_text(redactor, &String::from_utf8_lossy(input)),
                ),
                (
                    "mode",
                    RedactedValue::from(format!("{mode:?}").to_lowercase()),
                ),
            ])
        }
        AIAgentActionType::ReadFiles(request) => RedactedValue::object([
            (
                "paths",
                string_array(
                    request
                        .locations
                        .iter()
                        .map(|location| location.name.as_str()),
                    redactor,
                ),
            ),
            (
                "path_count",
                RedactedValue::from(request.locations.len() as u64),
            ),
        ]),
        AIAgentActionType::UploadArtifact(request) => RedactedValue::object([
            ("path", redacted_text(redactor, &request.file_path)),
            (
                "description",
                request
                    .description
                    .as_deref()
                    .map_or(RedactedValue::Null, |value| redacted_text(redactor, value)),
            ),
        ]),
        AIAgentActionType::SearchCodebase(request) => RedactedValue::object([
            ("query", redacted_text(redactor, &request.query)),
            (
                "paths",
                request
                    .partial_paths
                    .as_ref()
                    .map_or(RedactedValue::Null, |paths| {
                        string_array(paths.iter().map(String::as_str), redactor)
                    }),
            ),
            (
                "codebase_path",
                request
                    .codebase_path
                    .as_deref()
                    .map_or(RedactedValue::Null, |value| redacted_text(redactor, value)),
            ),
        ]),
        AIAgentActionType::RequestFileEdits { file_edits, .. } => RedactedValue::object([
            (
                "paths",
                string_array(file_edits.iter().filter_map(|edit| edit.file()), redactor),
            ),
            ("file_count", RedactedValue::from(file_edits.len() as u64)),
        ]),
        AIAgentActionType::Grep { queries, path } => RedactedValue::object([
            (
                "queries",
                string_array(queries.iter().map(String::as_str), redactor),
            ),
            ("path", redacted_text(redactor, path)),
            ("query_count", RedactedValue::from(queries.len() as u64)),
        ]),
        AIAgentActionType::FileGlob { patterns, path } => RedactedValue::object([
            (
                "patterns",
                string_array(patterns.iter().map(String::as_str), redactor),
            ),
            (
                "path",
                path.as_deref()
                    .map_or(RedactedValue::Null, |value| redacted_text(redactor, value)),
            ),
        ]),
        AIAgentActionType::FileGlobV2 {
            patterns,
            search_dir,
        } => RedactedValue::object([
            (
                "patterns",
                string_array(patterns.iter().map(String::as_str), redactor),
            ),
            (
                "search_dir",
                search_dir
                    .as_deref()
                    .map_or(RedactedValue::Null, |value| redacted_text(redactor, value)),
            ),
        ]),
        AIAgentActionType::CallMCPTool { name, input, .. } => {
            let argument_keys = input
                .as_object()
                .map(|object| object.keys().map(String::as_str))
                .map_or_else(
                    || RedactedValue::Array(Vec::new()),
                    |keys| string_array(keys, redactor),
                );
            RedactedValue::object([
                ("name", redacted_text(redactor, name)),
                ("argument_keys", argument_keys),
            ])
        }
        AIAgentActionType::ReadMCPResource { name, uri, .. } => RedactedValue::object([
            ("name", redacted_text(redactor, name)),
            (
                "uri",
                uri.as_deref()
                    .map_or(RedactedValue::Null, |value| redacted_text(redactor, value)),
            ),
        ]),
        AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
            RedactedValue::object([("comment_count", RedactedValue::from(comments.len() as u64))])
        }
        AIAgentActionType::AskUserQuestion { questions } => RedactedValue::object([(
            "question_count",
            RedactedValue::from(questions.len() as u64),
        )]),
        AIAgentActionType::RunAgents(request) => RedactedValue::object([
            (
                "agent_count",
                RedactedValue::from(request.agent_run_configs.len() as u64),
            ),
            ("summary", redacted_text(redactor, &request.summary)),
            ("model", redacted_text(redactor, &request.model_id)),
            ("harness", redacted_text(redactor, &request.harness_type)),
            (
                "execution_mode",
                RedactedValue::from(if request.execution_mode.is_remote() {
                    "remote"
                } else {
                    "local"
                }),
            ),
        ]),
        AIAgentActionType::SuggestNewConversation { .. }
        | AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview
        | AIAgentActionType::ReadDocuments(_)
        | AIAgentActionType::EditDocuments(_)
        | AIAgentActionType::CreateDocuments(_)
        | AIAgentActionType::ReadShellCommandOutput { .. }
        | AIAgentActionType::UseComputer(_)
        | AIAgentActionType::RequestComputerUse(_)
        | AIAgentActionType::StartRecording { .. }
        | AIAgentActionType::StopRecording { .. }
        | AIAgentActionType::ReadSkill(_)
        | AIAgentActionType::FetchConversation { .. }
        | AIAgentActionType::SendMessageToAgent { .. }
        | AIAgentActionType::TransferShellCommandControlToUser { .. }
        | AIAgentActionType::WaitForEvents { .. } => {
            RedactedValue::redacted("sensitive_tool_input", 0)
        }
    };
    let input_bytes = input.serialized_len();
    let input = if input_bytes > MAX_TOOL_INPUT_BYTES {
        RedactedValue::redacted("tool_input_size_limit", input_bytes)
    } else {
        input
    };
    (tool_name, input)
}

fn redacted_text(redactor: &HookRedactor, value: &str) -> RedactedValue {
    let RedactedText { value, truncation } = redactor.redact_text(value, MAX_METADATA_TEXT_BYTES);
    match truncation {
        Some(truncation) => RedactedValue::object([
            ("value", RedactedValue::from(value)),
            ("truncated", RedactedValue::Bool(truncation.truncated)),
            (
                "original_bytes",
                RedactedValue::from(truncation.original_bytes as u64),
            ),
            (
                "included_bytes",
                RedactedValue::from(truncation.included_bytes as u64),
            ),
        ]),
        None => RedactedValue::from(value),
    }
}

fn string_array<'a>(
    values: impl IntoIterator<Item = &'a str>,
    redactor: &HookRedactor,
) -> RedactedValue {
    RedactedValue::Array(
        values
            .into_iter()
            .take(MAX_METADATA_ITEMS)
            .map(|value| redacted_text(redactor, value))
            .collect(),
    )
}

pub(crate) fn local_action_result_payload(result: &AIAgentActionResultType) -> RedactedValue {
    let status = if result.is_cancelled() {
        "cancelled"
    } else if result.is_failed() {
        "failed"
    } else if result.is_successful() {
        "succeeded"
    } else {
        "completed"
    };
    RedactedValue::object([("status", RedactedValue::from(status))])
}

#[cfg(test)]
#[path = "adapters_tests.rs"]
mod tests;
