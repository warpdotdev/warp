//! The tool name a Cursor-format hook matches on and sees as `tool_name`. Cursor's own names are
//! used where a Warp action maps onto one of Cursor's tools; Warp-only tools get Warp-specific
//! names, which a `"*"` matcher still matches. Loading a skill from disk is a `Read` of its
//! `SKILL.md`, because that is how a Cursor agent loads a skill and what Cursor scripts look for;
//! a bundled skill has no file to read, so it keeps a Warp name.

use std::borrow::Cow;

use crate::agent::action::{AIAgentActionType, FileEdit};
use crate::skills::SkillReference;

pub(super) fn tool_name(action: &AIAgentActionType) -> Cow<'static, str> {
    let name = match action {
        AIAgentActionType::RequestCommandOutput { .. } => "Shell",
        AIAgentActionType::ReadFiles(_) => "Read",
        AIAgentActionType::RequestFileEdits { file_edits, .. } => edits_tool_name(file_edits),
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
        AIAgentActionType::ReadSkill(request) => match &request.skill {
            SkillReference::Path(_) => "Read",
            SkillReference::BundledSkillId(_) => "ReadSkill",
        },
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

/// `Delete` for a batch made only of deletes, `Write` otherwise.
pub(super) fn edits_tool_name<'a>(edits: impl IntoIterator<Item = &'a FileEdit>) -> &'static str {
    let mut edits = edits.into_iter().peekable();
    let non_empty = edits.peek().is_some();
    if non_empty && edits.all(|edit| matches!(edit, FileEdit::Delete { .. })) {
        "Delete"
    } else {
        "Write"
    }
}

#[cfg(test)]
#[path = "tool_names_tests.rs"]
mod tests;
