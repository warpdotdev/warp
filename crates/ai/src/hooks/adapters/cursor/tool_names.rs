//! The tool name a Cursor-format hook matches on and sees as `tool_name`. Cursor's own names are
//! used where a Warp action maps onto one of Cursor's tools; Warp-only tools get Warp-specific
//! names, which a `"*"` matcher still matches.

use std::borrow::Cow;

use crate::agent::action::{AIAgentActionType, FileEdit};

pub(super) fn tool_name(action: &AIAgentActionType) -> Cow<'static, str> {
    let name = match action {
        AIAgentActionType::RequestCommandOutput { .. } => "Shell",
        AIAgentActionType::ReadFiles(_) => "Read",
        AIAgentActionType::RequestFileEdits { file_edits, .. } => {
            let all_deletes = !file_edits.is_empty()
                && file_edits
                    .iter()
                    .all(|edit| matches!(edit, FileEdit::Delete { .. }));
            if all_deletes { "Delete" } else { "Write" }
        }
        AIAgentActionType::Grep { .. } => "Grep",
        AIAgentActionType::RunAgents(_) => "Task",
        AIAgentActionType::CallMCPTool { name, .. } => return Cow::Owned(format!("MCP:{name}")),
        AIAgentActionType::FileGlob { .. } | AIAgentActionType::FileGlobV2 { .. } => "Glob",
        AIAgentActionType::SearchCodebase(_) => "SearchCodebase",
        AIAgentActionType::ReadMCPResource { name, .. } => {
            return Cow::Owned(format!("MCPResource:{name}"));
        }
        AIAgentActionType::WriteToLongRunningShellCommand { .. } => "ShellWrite",
        AIAgentActionType::ReadShellCommandOutput { .. } => "ShellRead",
        AIAgentActionType::TransferShellCommandControlToUser { .. } => "ShellHandoff",
        AIAgentActionType::ReadSkill(_) => "ReadSkill",
        AIAgentActionType::UseComputer(_) => "UseComputer",
        AIAgentActionType::RequestComputerUse(_) => "RequestComputerUse",
        AIAgentActionType::StartRecording { .. } => "StartRecording",
        AIAgentActionType::StopRecording { .. } => "StopRecording",
        AIAgentActionType::ReadDocuments(_) => "ReadDocuments",
        AIAgentActionType::EditDocuments(_) => "EditDocuments",
        AIAgentActionType::CreateDocuments(_) => "CreateDocuments",
        AIAgentActionType::AskUserQuestion { .. } => "AskUserQuestion",
        AIAgentActionType::SendMessageToAgent { .. } => "SendMessageToAgent",
        AIAgentActionType::FetchConversation { .. } => "FetchConversation",
        AIAgentActionType::WaitForEvents { .. } => "WaitForEvents",
        AIAgentActionType::UploadArtifact(_) => "UploadArtifact",
        AIAgentActionType::SuggestNewConversation { .. } => "SuggestNewConversation",
        AIAgentActionType::SuggestPrompt(_) => "SuggestPrompt",
        AIAgentActionType::InitProject => "InitProject",
        AIAgentActionType::OpenCodeReview => "OpenCodeReview",
        AIAgentActionType::InsertCodeReviewComments { .. } => "InsertCodeReviewComments",
    };
    Cow::Borrowed(name)
}

#[cfg(test)]
#[path = "tool_names_tests.rs"]
mod tests;
