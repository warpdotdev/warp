//! The pieces of a tool action a Cursor script sees. Cursor's `Read`, `Write`, and `Delete` work
//! on one file at a time, so a Warp read or edit that covers several files is presented as one
//! part per file, each with Cursor's single-file shape; every other action is one part. The
//! matcher tests each part, and the adapter encodes one stdin payload per matching part.

use std::borrow::Cow;
use std::path::Path;

use super::tool_names::{edits_tool_name, tool_name};
use crate::agent::FileLocations;
use crate::agent::action::{AIAgentActionType, FileEdit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ToolPart<'a> {
    /// The action as one operation.
    Whole(&'a AIAgentActionType),
    /// One file of a read.
    ReadFile(&'a FileLocations),
    /// The edits of a batch that touch one file. `file` is `None` for edits that name no file.
    EditFile {
        file: Option<&'a str>,
        title: Option<&'a str>,
        edits: Vec<&'a FileEdit>,
    },
}

impl ToolPart<'_> {
    pub(super) fn tool_name(&self) -> Cow<'static, str> {
        match self {
            Self::Whole(action) => tool_name(action),
            Self::ReadFile(_) => Cow::Borrowed("Read"),
            Self::EditFile { edits, .. } => Cow::Borrowed(edits_tool_name(edits.iter().copied())),
        }
    }

    /// The file this part is about, which labels its input when the action was split.
    pub(super) fn file(&self) -> Option<&str> {
        match self {
            Self::Whole(_) => None,
            Self::ReadFile(location) => Some(&location.name),
            Self::EditFile { file, .. } => *file,
        }
    }
}

/// The parts of `action`, in the order the action lists them. A read is always presented per
/// file, so a single-file read has Cursor's `Read` shape too; an edit batch is split only when it
/// touches more than one file, so a single-file batch keeps its whole shape, which already names
/// the file.
pub(super) fn tool_parts(action: &AIAgentActionType) -> Vec<ToolPart<'_>> {
    match action {
        AIAgentActionType::ReadFiles(request) if !request.locations.is_empty() => {
            request.locations.iter().map(ToolPart::ReadFile).collect()
        }
        AIAgentActionType::RequestFileEdits { file_edits, title } => {
            let groups = edits_by_file(file_edits);
            if groups.len() > 1 {
                groups
                    .into_iter()
                    .map(|(file, edits)| ToolPart::EditFile {
                        file,
                        title: title.as_deref(),
                        edits,
                    })
                    .collect()
            } else {
                vec![ToolPart::Whole(action)]
            }
        }
        AIAgentActionType::ReadFiles(_)
        | AIAgentActionType::RequestCommandOutput { .. }
        | AIAgentActionType::WriteToLongRunningShellCommand { .. }
        | AIAgentActionType::ReadShellCommandOutput { .. }
        | AIAgentActionType::TransferShellCommandControlToUser { .. }
        | AIAgentActionType::UploadArtifact(_)
        | AIAgentActionType::SearchCodebase(_)
        | AIAgentActionType::Grep { .. }
        | AIAgentActionType::FileGlob { .. }
        | AIAgentActionType::FileGlobV2 { .. }
        | AIAgentActionType::ReadMCPResource { .. }
        | AIAgentActionType::CallMCPTool { .. }
        | AIAgentActionType::SuggestNewConversation { .. }
        | AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview
        | AIAgentActionType::ReadDocuments(_)
        | AIAgentActionType::EditDocuments(_)
        | AIAgentActionType::CreateDocuments(_)
        | AIAgentActionType::UseComputer(_)
        | AIAgentActionType::InsertCodeReviewComments { .. }
        | AIAgentActionType::RequestComputerUse(_)
        | AIAgentActionType::StartRecording { .. }
        | AIAgentActionType::StopRecording { .. }
        | AIAgentActionType::ReadSkill(_)
        | AIAgentActionType::FetchConversation { .. }
        | AIAgentActionType::SendMessageToAgent { .. }
        | AIAgentActionType::AskUserQuestion { .. }
        | AIAgentActionType::RunAgents(_)
        | AIAgentActionType::WaitForEvents { .. } => vec![ToolPart::Whole(action)],
    }
}

/// Groups edits by the file they name, keeping the order files first appear in.
fn edits_by_file(file_edits: &[FileEdit]) -> Vec<(Option<&str>, Vec<&FileEdit>)> {
    let mut groups: Vec<(Option<&str>, Vec<&FileEdit>)> = Vec::new();
    for edit in file_edits {
        let file = edit.file();
        match groups.iter_mut().find(|(group, _)| *group == file) {
            Some((_, edits)) => edits.push(edit),
            None => groups.push((file, vec![edit])),
        }
    }
    groups
}

/// Whether a path in a result names the file a part asked for. A result reports the host's
/// absolute path, while the model may have asked with a path relative to the working directory,
/// so the request is accepted as a trailing run of the result's path components.
pub(super) fn names_requested_file(result_path: &str, requested: &str) -> bool {
    if result_path == requested {
        return true;
    }
    let requested = requested.strip_prefix("./").unwrap_or(requested);
    Path::new(result_path).ends_with(Path::new(requested))
}

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;
