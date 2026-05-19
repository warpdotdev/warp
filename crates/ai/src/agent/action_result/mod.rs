mod convert;

use std::{fmt::Display, ops::Range, time::SystemTime};

use chrono::{DateTime, Local};
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use warp_core::command::ExitCode;
use warp_multi_agent_api::apply_file_diffs_result::success::UpdatedFileContent;
use warp_terminal::model::BlockId;

use crate::{
    agent::FileLocations,
    document::{AIDocumentId, AIDocumentVersion},
};

#[derive(Debug, Clone, PartialEq)]
pub enum AIAgentActionResultType {
    /// The output of a requested command.
    RequestCommandOutput(RequestCommandOutputResult),

    /// The result of sending some input to a long-running command.
    WriteToLongRunningShellCommand(WriteToLongRunningShellCommandResult),

    /// The output of a requested file edits.
    RequestFileEdits(RequestFileEditsResult),

    /// The output of a read files action.
    ReadFiles(ReadFilesResult),

    /// The output of an upload artifact action.
    UploadArtifact(UploadArtifactResult),

    /// The output of a search codebase action.
    SearchCodebase(SearchCodebaseResult),

    /// The output of a grep action.
    Grep(GrepResult),

    /// The output of a file glob action.
    FileGlob(FileGlobResult),

    /// The output of a file glob V2 action.
    FileGlobV2(FileGlobV2Result),

    /// The output of reading an MCP resource.
    ReadMCPResource(ReadMCPResourceResult),

    /// The output of calling an MCP tool.
    CallMCPTool(CallMCPToolResult),

    /// The output of reading a skill.
    ReadSkill(ReadSkillResult),

    /// The output of suggesting a new conversation.
    SuggestNewConversation(SuggestNewConversationResult),

    /// The result of suggesting a prompt.
    SuggestPrompt(SuggestPromptResult),

    OpenCodeReview,

    InitProject,

    /// The output of a read documents action.
    ReadDocuments(ReadDocumentsResult),

    /// The output of an edit documents action.
    EditDocuments(EditDocumentsResult),

    /// The output of a create documents action.
    CreateDocuments(CreateDocumentsResult),

    /// The output of reading shell command output.
    ReadShellCommandOutput(ReadShellCommandOutputResult),

    /// The output of using computer.
    UseComputer(UseComputerResult),

    /// The result of inserting code review comments.
    InsertReviewComments(InsertReviewCommentsResult),

    /// The output of requesting computer use.
    RequestComputerUse(RequestComputerUseResult),

    /// The result of fetching a conversation's tasks.
    FetchConversation(FetchConversationResult),

    /// The result of starting a child agent.
    StartAgent(StartAgentResult),

    /// The result of sending a message to another agent.
    SendMessageToAgent(SendMessageToAgentResult),

    /// The output of transferring shell command control to the user.
    TransferShellCommandControlToUser(TransferShellCommandControlToUserResult),
    /// The result of asking the user a question.
    AskUserQuestion(AskUserQuestionResult),

    /// The result of an orchestrate tool call: launched (with per-agent
    /// outcomes), launch denied (Stage 2), failure, or cancelled.
    RunAgents(RunAgentsResult),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum StartAgentVersion {
    #[default]
    V1,
    V2,
}

impl AIAgentActionResultType {
    /// Returns the effective command string for command-related results, if any.
    ///
    /// This is used by UIs (e.g. requested command views) that want to display the
    /// final executed command rather than the original suggestion.
    pub fn command_str(&self) -> Option<&str> {
        match self {
            AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::Completed { command, .. },
            )
            | AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { command, .. },
            )
            | AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::CommandFinished { command, .. },
            )
            | AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::LongRunningCommandSnapshot { command, .. },
            ) => Some(command.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

impl Display for AIAgentActionResultType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIAgentActionResultType::RequestCommandOutput(result) => result.fmt(f),
            AIAgentActionResultType::WriteToLongRunningShellCommand(result) => result.fmt(f),
            AIAgentActionResultType::RequestFileEdits(result) => result.fmt(f),
            AIAgentActionResultType::ReadFiles(result) => result.fmt(f),
            AIAgentActionResultType::UploadArtifact(result) => result.fmt(f),
            AIAgentActionResultType::SearchCodebase(result) => result.fmt(f),
            AIAgentActionResultType::Grep(result) => result.fmt(f),
            AIAgentActionResultType::FileGlob(result) => result.fmt(f),
            AIAgentActionResultType::FileGlobV2(result) => result.fmt(f),
            AIAgentActionResultType::ReadMCPResource(result) => result.fmt(f),
            AIAgentActionResultType::CallMCPTool(result) => result.fmt(f),
            AIAgentActionResultType::ReadSkill(result) => result.fmt(f),
            AIAgentActionResultType::SuggestNewConversation(result) => result.fmt(f),
            AIAgentActionResultType::SuggestPrompt(result) => result.fmt(f),
            AIAgentActionResultType::ReadDocuments(result) => result.fmt(f),
            AIAgentActionResultType::EditDocuments(result) => result.fmt(f),
            AIAgentActionResultType::CreateDocuments(result) => result.fmt(f),
            AIAgentActionResultType::ReadShellCommandOutput(result) => result.fmt(f),
            AIAgentActionResultType::UseComputer(result) => result.fmt(f),
            AIAgentActionResultType::InsertReviewComments(result) => result.fmt(f),
            AIAgentActionResultType::RequestComputerUse(result) => result.fmt(f),
            AIAgentActionResultType::FetchConversation(result) => result.fmt(f),
            AIAgentActionResultType::StartAgent(result) => result.fmt(f),
            AIAgentActionResultType::SendMessageToAgent(result) => result.fmt(f),
            AIAgentActionResultType::TransferShellCommandControlToUser(result) => result.fmt(f),
            AIAgentActionResultType::AskUserQuestion(result) => result.fmt(f),
            AIAgentActionResultType::RunAgents(result) => result.fmt(f),
            AIAgentActionResultType::OpenCodeReview | AIAgentActionResultType::InitProject => {
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RequestCommandOutputResult {
    Completed {
        block_id: BlockId,
        command: String,
        output: String,
        exit_code: ExitCode,
        start_ts: Option<DateTime<Local>>,
        completed_ts: Option<DateTime<Local>>,
    },
    LongRunningCommandSnapshot {
        block_id: BlockId,
        command: String,
        grid_contents: String,
        cursor: String,
        is_alt_screen_active: bool,
    },
    /// A running command canceled via ctrl-c
    /// would have Completed result with exit code 130.
    CancelledBeforeExecution,
    /// The command was denied because it was present on the denylist.
    Denylisted { command: String },
}

impl RequestCommandOutputResult {
    pub fn is_successful(&self) -> bool {
        match self {
            Self::Completed { exit_code, .. } => exit_code.was_successful(),
            Self::LongRunningCommandSnapshot { .. } => true,
            Self::CancelledBeforeExecution | Self::Denylisted { .. } => false,
        }
    }

    pub fn failed(&self) -> bool {
        match self {
            Self::Completed { exit_code, .. } => !exit_code.was_successful(),
            Self::Denylisted { .. } => true,
            Self::CancelledBeforeExecution | Self::LongRunningCommandSnapshot { .. } => false,
        }
    }
}

impl Display for RequestCommandOutputResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestCommandOutputResult::Completed {
                command,
                output,
                exit_code,
                ..
            } => {
                write!(
                    f,
                    "Command '{}' completed with exit code {}:\n{}",
                    command,
                    exit_code.value(),
                    output
                )
            }
            RequestCommandOutputResult::LongRunningCommandSnapshot { command, .. } => {
                write!(f, "Command '{command}' is long-running")
            }
            RequestCommandOutputResult::CancelledBeforeExecution => {
                write!(f, "Command output cancelled")
            }
            RequestCommandOutputResult::Denylisted { .. } => {
                write!(f, "Command output was on denylist")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum ShellCommandError {
    BlockNotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum WriteToLongRunningShellCommandResult {
    Snapshot {
        block_id: BlockId,
        grid_contents: String,
        cursor: String,
        is_alt_screen_active: bool,
        is_preempted: bool,
    },
    CommandFinished {
        block_id: BlockId,
        output: String,
        exit_code: ExitCode,
        start_ts: Option<DateTime<Local>>,
        completed_ts: Option<DateTime<Local>>,
    },
    Cancelled,
    Error(ShellCommandError),
}

impl Display for WriteToLongRunningShellCommandResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot { .. } => {
                write!(f, "Sent snapshot of long-running shell command to agent")
            }
            Self::CommandFinished {
                output, exit_code, ..
            } => write!(
                f,
                "Long-running shell command finished with exit code {}:\n{output}",
                exit_code.value()
            ),
            Self::Cancelled => write!(f, "Writing to long-running shell command cancelled"),
            Self::Error(e) => write!(f, "Write to long-running shell command failed: {e:?}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum SuggestNewConversationResult {
    Accepted { message_id: String },
    Rejected,
    Cancelled,
}

impl Display for SuggestNewConversationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accepted { message_id } => {
                write!(
                    f,
                    "Suggest new conversation accepted for message {message_id}"
                )
            }
            Self::Rejected => {
                write!(f, "Suggest new conversation rejected for message")
            }
            Self::Cancelled => write!(f, "Suggest new conversation cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum AnyFileContent {
    StringContent(String),
    BinaryContent(Vec<u8>),
}

impl AnyFileContent {
    pub fn len(&self) -> usize {
        match self {
            Self::StringContent(content) => content.len(),
            Self::BinaryContent(content) => content.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        match self {
            Self::StringContent(content) => content.lines().count(),
            Self::BinaryContent(_) => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::StringContent(content) => content.is_empty(),
            Self::BinaryContent(content) => content.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct FileContext {
    pub file_name: String,
    pub content: AnyFileContent,
    pub line_range: Option<Range<usize>>,
    pub last_modified: Option<SystemTime>,
    pub line_count: usize,
}

impl FileContext {
    // create a new FileContext and autocalculate number of lines in the given file
    pub fn new(
        file_name: String,
        content: AnyFileContent,
        line_range: Option<Range<usize>>,
        last_modified: Option<SystemTime>,
    ) -> Self {
        let string_content = if let AnyFileContent::StringContent(content) = content.clone() {
            content
        } else {
            return Self {
                file_name,
                content,
                line_range,
                last_modified,
                line_count: 0,
            };
        };

        let line_count = string_content.lines().count();

        Self {
            file_name,
            content: AnyFileContent::StringContent(string_content),
            line_range,
            last_modified,
            line_count,
        }
    }
}

impl Display for FileContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.line_range {
            None => write!(f, "{}", self.file_name),
            Some(range) => write!(f, "{} ({}-{})", self.file_name, range.start, range.end),
        }
    }
}

impl From<&FileContext> for FileLocations {
    fn from(context: &FileContext) -> Self {
        FileLocations {
            name: context.file_name.clone(),
            lines: context.line_range.clone().into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReadFilesResult {
    Success { files: Vec<FileContext> },
    Error(String),
    Cancelled,
}

impl Display for ReadFilesResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadFilesResult::Success { files } => {
                write!(f, "Read files: {}", files.iter().format(", "))
            }
            ReadFilesResult::Error(error) => write!(f, "Read files error: {error}"),
            ReadFilesResult::Cancelled => write!(f, "Read files cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum UploadArtifactResult {
    Success {
        artifact_uid: String,
        filepath: Option<String>,
        mime_type: String,
        description: Option<String>,
        size_bytes: i64,
    },
    Error(String),
    Cancelled,
}

impl Display for UploadArtifactResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UploadArtifactResult::Success {
                artifact_uid,
                filepath,
                ..
            } => match filepath {
                Some(filepath) => write!(f, "Uploaded artifact {artifact_uid} from {filepath}"),
                None => write!(f, "Uploaded artifact {artifact_uid}"),
            },
            UploadArtifactResult::Error(error) => write!(f, "Upload artifact error: {error}"),
            UploadArtifactResult::Cancelled => write!(f, "Upload artifact cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentContext {
    pub document_id: AIDocumentId,
    pub document_version: AIDocumentVersion,
    pub content: String,
    pub line_ranges: Vec<Range<usize>>,
}

impl Display for DocumentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line_ranges.is_empty() {
            return write!(f, "Document {}", self.document_id);
        }

        let line_ranges = self
            .line_ranges
            .iter()
            .map(|range| format!("{}-{}", range.start, range.end))
            .collect_vec();
        write!(
            f,
            "Document {} ({})",
            self.document_id,
            line_ranges.join(", ")
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReadDocumentsResult {
    Success { documents: Vec<DocumentContext> },
    Error(String),
    Cancelled,
}

impl Display for ReadDocumentsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadDocumentsResult::Success { documents } => {
                write!(f, "Read documents: {}", documents.iter().format(", "))
            }
            ReadDocumentsResult::Error(error) => write!(f, "Read documents error: {error}"),
            ReadDocumentsResult::Cancelled => write!(f, "Read documents cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EditDocumentsResult {
    Success {
        updated_documents: Vec<DocumentContext>,
    },
    Error(String),
    Cancelled,
}

impl Display for EditDocumentsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditDocumentsResult::Success { updated_documents } => {
                write!(
                    f,
                    "Edited documents: {}",
                    updated_documents.iter().format(", ")
                )
            }
            EditDocumentsResult::Error(error) => write!(f, "Edit documents error: {error}"),
            EditDocumentsResult::Cancelled => write!(f, "Edit documents cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CreateDocumentsResult {
    Success {
        created_documents: Vec<DocumentContext>,
    },
    Error(String),
    Cancelled,
}

impl Display for CreateDocumentsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateDocumentsResult::Success { created_documents } => {
                write!(
                    f,
                    "Created documents: {}",
                    created_documents.iter().format(", ")
                )
            }
            CreateDocumentsResult::Error(error) => write!(f, "Create documents error: {error}"),
            CreateDocumentsResult::Cancelled => write!(f, "Create documents cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ReadShellCommandOutputResult {
    CommandFinished {
        command: String,
        block_id: BlockId,
        output: String,
        exit_code: ExitCode,
        start_ts: Option<DateTime<Local>>,
        completed_ts: Option<DateTime<Local>>,
    },
    LongRunningCommandSnapshot {
        command: String,
        block_id: BlockId,
        grid_contents: String,
        cursor: String,
        is_alt_screen_active: bool,
        is_preempted: bool,
    },
    Cancelled,
    Error(ShellCommandError),
}

impl Display for ReadShellCommandOutputResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadShellCommandOutputResult::CommandFinished {
                output, exit_code, ..
            } => {
                write!(
                    f,
                    "Shell command output finished with exit code{}:\n{output}",
                    exit_code.value()
                )
            }
            ReadShellCommandOutputResult::LongRunningCommandSnapshot { .. } => {
                write!(f, "Sent snapshot of long-running shell command to agent")
            }
            ReadShellCommandOutputResult::Cancelled => {
                write!(f, "Read shell command output cancelled")
            }
            ReadShellCommandOutputResult::Error(e) => {
                write!(f, "Read shell command output failed: {e:?}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SearchCodebaseFailureReason {
    CodebaseNotIndexed,
    InvalidFilePaths,
    GetRelevantFilesError,
    ClientError,
    MissingCurrentWorkingDirectory,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SearchCodebaseResult {
    Success {
        files: Vec<FileContext>,
    },
    Failed {
        reason: SearchCodebaseFailureReason,

        /// The message to be sent back to the LLM to inform it why the search failed.
        message: String,
    },
    Cancelled,
}

impl Display for SearchCodebaseResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchCodebaseResult::Success { files } => {
                write!(f, "Codebase search found: {}", files.iter().format(", "))
            }
            SearchCodebaseResult::Failed { reason, message } => {
                write!(f, "Codebase search failed ({reason:?}): {message}")
            }
            SearchCodebaseResult::Cancelled => write!(f, "Codebase search cancelled"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RequestFileEditsResult {
    Success {
        diff: String,
        updated_files: Vec<UpdatedFileContext>,
        deleted_files: Vec<String>,
        lines_added: usize,
        lines_removed: usize,
    },
    Cancelled,
    /// Diff application failed.
    DiffApplicationFailed {
        error: String,
    },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UpdatedFileContext {
    pub was_edited_by_user: bool,
    pub file_context: FileContext,
}

impl Display for UpdatedFileContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "user_edited {}, file {}",
            self.was_edited_by_user, self.file_context
        )
    }
}

impl From<UpdatedFileContext> for Vec<UpdatedFileContent> {
    fn from(value: UpdatedFileContext) -> Self {
        // Note: This method only makes sense for FileContexts that have a string content.
        // TODO: How do we gracefully fail binary files here?
        let file_content: Vec<warp_multi_agent_api::FileContent> = value.file_context.into();

        file_content
            .into_iter()
            .map(|content| UpdatedFileContent {
                was_edited_by_user: value.was_edited_by_user,
                file: Some(content),
            })
            .collect()
    }
}

impl Display for RequestFileEditsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestFileEditsResult::Success {
                diff,
                updated_files,
                ..
            } => {
                write!(
                    f,
                    "File edits completed:\n\tDiff:\n{diff}\n\tUpdatedFiles: [{}]",
                    updated_files.iter().format(", ")
                )
            }
            RequestFileEditsResult::Cancelled => write!(f, "File edits cancelled"),
            RequestFileEditsResult::DiffApplicationFailed { error } => {
                write!(f, "File edits failed: {error}")
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SuggestPromptResult {
    Accepted { query: String },
    Cancelled,
}

impl Display for SuggestPromptResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuggestPromptResult::Accepted { query } => {
                write!(f, "Suggest prompt accepted: {query}")
            }
            SuggestPromptResult::Cancelled => write!(f, "Suggest prompt cancelled"),
        }
    }
}

impl AIAgentActionResultType {
    /// A user visible description of what the result contains.
    /// Used to display error messages when the content is too large for
    /// the LLM context window.
    pub fn description(&self) -> &str {
        match self {
            AIAgentActionResultType::RequestCommandOutput(_) => {
                "The output of your last command executed by Agent Mode"
            }
            AIAgentActionResultType::WriteToLongRunningShellCommand(_) => {
                "A snapshot of the command currently being executed by Agent Mode"
            }
            AIAgentActionResultType::RequestFileEdits(_) => {
                "The diff from editing the last file in Agent Mode"
            }
            AIAgentActionResultType::ReadFiles(_) => "The requested file content",
            AIAgentActionResultType::UploadArtifact(_) => "The uploaded artifact metadata",
            AIAgentActionResultType::SearchCodebase(_) => "The codebase search results",
            AIAgentActionResultType::Grep(_) => "The results of the grep operation",
            AIAgentActionResultType::FileGlob(_) => "The results of the file glob operation",
            AIAgentActionResultType::FileGlobV2(_) => "The results of the file glob operation",
            AIAgentActionResultType::CallMCPTool(_) => "The MCP tool call",
            AIAgentActionResultType::ReadSkill(_) => "The results of reading a skill from file",
            AIAgentActionResultType::ReadMCPResource(_) => "The MCP resource",
            AIAgentActionResultType::SuggestNewConversation(_) => {
                "Your decision on whether to start a new conversation"
            }
            AIAgentActionResultType::SuggestPrompt(_) => "The suggested prompt",
            AIAgentActionResultType::OpenCodeReview => "Open code review",
            AIAgentActionResultType::InsertReviewComments(_) => "Insert code review comments",
            AIAgentActionResultType::InitProject => "Initialize project",
            AIAgentActionResultType::ReadDocuments(_) => "The requested document content",
            AIAgentActionResultType::EditDocuments(_) => "The edited document content",
            AIAgentActionResultType::CreateDocuments(_) => "The newly created documents",
            AIAgentActionResultType::ReadShellCommandOutput(_) => "The shell command output",
            AIAgentActionResultType::UseComputer(_) => "The computer use result",
            AIAgentActionResultType::RequestComputerUse(_) => "The computer use request result",
            AIAgentActionResultType::FetchConversation(_) => "The fetched conversation tasks",
            AIAgentActionResultType::StartAgent(_) => "The result of starting a child agent",
            AIAgentActionResultType::SendMessageToAgent(_) => "The result of sending a message",
            AIAgentActionResultType::TransferShellCommandControlToUser(_) => {
                "The result of transferring shell command control to user"
            }
            AIAgentActionResultType::AskUserQuestion(_) => {
                "The user's answers to clarifying questions"
            }
            AIAgentActionResultType::RunAgents(_) => {
                "The result of an orchestrate batch of child agents"
            }
        }
    }

    pub fn is_successful(&self) -> bool {
        match self {
            Self::RequestCommandOutput(r) => r.is_successful(),
            Self::RequestFileEdits(RequestFileEditsResult::Success { .. })
            | Self::ReadFiles(ReadFilesResult::Success { .. })
            | Self::UploadArtifact(UploadArtifactResult::Success { .. })
            | Self::SearchCodebase(SearchCodebaseResult::Success { .. })
            | Self::Grep(GrepResult::Success { .. })
            | Self::FileGlob(FileGlobResult::Success { .. })
            | Self::FileGlobV2(FileGlobV2Result::Success { .. })
            | Self::ReadMCPResource(ReadMCPResourceResult::Success { .. })
            | Self::CallMCPTool(CallMCPToolResult::Success { .. })
            | Self::SuggestNewConversation(SuggestNewConversationResult::Accepted { .. })
            | Self::SuggestPrompt(SuggestPromptResult::Accepted { .. })
            | Self::ReadDocuments(ReadDocumentsResult::Success { .. })
            | Self::EditDocuments(EditDocumentsResult::Success { .. })
            | Self::CreateDocuments(CreateDocumentsResult::Success { .. })
            | Self::ReadShellCommandOutput(
                ReadShellCommandOutputResult::CommandFinished { .. }
                | ReadShellCommandOutputResult::LongRunningCommandSnapshot { .. },
            )
            | Self::UseComputer(UseComputerResult::Success(_))
            | Self::InsertReviewComments(InsertReviewCommentsResult::Success { .. })
            | Self::RequestComputerUse(RequestComputerUseResult::Approved { .. })
            | Self::OpenCodeReview
            | Self::ReadSkill(ReadSkillResult::Success { .. })
            | Self::FetchConversation(FetchConversationResult::Success { .. })
            | Self::StartAgent(StartAgentResult::Success { .. })
            | Self::SendMessageToAgent(SendMessageToAgentResult::Success { .. })
            | Self::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Snapshot { .. }
                | TransferShellCommandControlToUserResult::CommandFinished { .. },
            ) => true,
            Self::AskUserQuestion(AskUserQuestionResult::Success { .. }) => true,
            Self::RunAgents(RunAgentsResult::Launched { .. }) => true,
            _ => false,
        }
    }

    pub fn is_failed(&self) -> bool {
        match self {
            Self::RequestCommandOutput(r) => r.failed(),
            Self::RequestFileEdits(RequestFileEditsResult::DiffApplicationFailed { .. })
            | Self::ReadFiles(ReadFilesResult::Error(_))
            | Self::UploadArtifact(UploadArtifactResult::Error(_))
            | Self::SearchCodebase(SearchCodebaseResult::Failed { .. })
            | Self::Grep(GrepResult::Error(_))
            | Self::FileGlob(FileGlobResult::Error(_))
            | Self::FileGlobV2(FileGlobV2Result::Error(_))
            | Self::ReadMCPResource(ReadMCPResourceResult::Error(_))
            | Self::CallMCPTool(CallMCPToolResult::Error(_))
            | Self::ReadDocuments(ReadDocumentsResult::Error(_))
            | Self::EditDocuments(EditDocumentsResult::Error(_))
            | Self::CreateDocuments(CreateDocumentsResult::Error(_))
            | Self::UseComputer(UseComputerResult::Error(_))
            | Self::InsertReviewComments(InsertReviewCommentsResult::Error { .. })
            | Self::RequestComputerUse(RequestComputerUseResult::Error(_))
            | Self::FetchConversation(FetchConversationResult::Error(_))
            | Self::StartAgent(StartAgentResult::Error { .. })
            | Self::SendMessageToAgent(SendMessageToAgentResult::Error(_))
            | Self::AskUserQuestion(AskUserQuestionResult::Error(_))
            | Self::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Error(_),
            )
            | Self::RunAgents(RunAgentsResult::Failure { .. } | RunAgentsResult::Denied { .. }) => {
                true
            }
            _ => false,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::RequestCommandOutput(RequestCommandOutputResult::CancelledBeforeExecution) => {
                true
            }
            Self::RequestCommandOutput(RequestCommandOutputResult::Completed {
                exit_code, ..
            }) if exit_code.value() == 130 => true,
            Self::RequestFileEdits(RequestFileEditsResult::Cancelled)
            | Self::ReadFiles(ReadFilesResult::Cancelled)
            | Self::UploadArtifact(UploadArtifactResult::Cancelled)
            | Self::SearchCodebase(SearchCodebaseResult::Cancelled)
            | Self::Grep(GrepResult::Cancelled)
            | Self::FileGlob(FileGlobResult::Cancelled)
            | Self::FileGlobV2(FileGlobV2Result::Cancelled)
            | Self::ReadMCPResource(ReadMCPResourceResult::Cancelled)
            | Self::CallMCPTool(CallMCPToolResult::Cancelled)
            | Self::SuggestNewConversation(SuggestNewConversationResult::Cancelled)
            | Self::SuggestPrompt(SuggestPromptResult::Cancelled)
            | Self::ReadDocuments(ReadDocumentsResult::Cancelled)
            | Self::EditDocuments(EditDocumentsResult::Cancelled)
            | Self::CreateDocuments(CreateDocumentsResult::Cancelled)
            | Self::ReadShellCommandOutput(ReadShellCommandOutputResult::Cancelled)
            | Self::UseComputer(UseComputerResult::Cancelled)
            | Self::InsertReviewComments(InsertReviewCommentsResult::Cancelled)
            | Self::RequestComputerUse(RequestComputerUseResult::Cancelled)
            | Self::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Cancelled,
            )
            | Self::WriteToLongRunningShellCommand(
                WriteToLongRunningShellCommandResult::Cancelled,
            )
            | Self::ReadSkill(ReadSkillResult::Cancelled)
            | Self::FetchConversation(FetchConversationResult::Cancelled)
            | Self::StartAgent(StartAgentResult::Cancelled { .. })
            | Self::SendMessageToAgent(SendMessageToAgentResult::Cancelled)
            // SkippedByAutoApprove is intentionally excluded: the agent should continue.
            | Self::AskUserQuestion(AskUserQuestionResult::Cancelled)
            | Self::RunAgents(RunAgentsResult::Cancelled) => true,
            _ => false,
        }
    }

    pub fn is_cancelled_during_requested_command_execution(&self) -> bool {
        matches!(self, Self::RequestCommandOutput(RequestCommandOutputResult::Completed {
            exit_code, ..
        }) if exit_code.value() == 130)
    }

    /// Returns `true` if this completion of this action result should trigger a follow-up request.
    pub fn should_trigger_request_upon_completion(&self) -> bool {
        !self.is_cancelled()
    }

    pub fn is_requested_command(&self) -> bool {
        matches!(self, AIAgentActionResultType::RequestCommandOutput(_))
    }

    pub fn is_call_mcp_tool(&self) -> bool {
        matches!(self, AIAgentActionResultType::CallMCPTool(_))
    }

    /// Returns `true` if this result will cause the server to route the next
    /// turn to a subagent (e.g. the CLI subagent) rather than the orchestrator.
    /// LRC snapshot variants are the current indicators of this.
    pub fn triggers_server_subagent(&self) -> bool {
        matches!(
            self,
            Self::RequestCommandOutput(
                RequestCommandOutputResult::LongRunningCommandSnapshot { .. }
            ) | Self::WriteToLongRunningShellCommand(
                WriteToLongRunningShellCommandResult::Snapshot { .. }
            ) | Self::ReadShellCommandOutput(
                ReadShellCommandOutputResult::LongRunningCommandSnapshot { .. }
            ) | Self::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Snapshot { .. }
            )
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrepResult {
    Success { matched_files: Vec<GrepFileMatch> },
    Error(String),
    Cancelled,
}

impl Display for GrepResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GrepResult::Success { matched_files } => {
                write!(
                    f,
                    "Grep found matches in: [{}]",
                    matched_files.iter().format(", ")
                )
            }
            GrepResult::Error(error) => write!(f, "Grep error: {error}"),
            GrepResult::Cancelled => write!(f, "Grep cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepFileMatch {
    /// The absolute path to the file that was matched.
    pub file_path: String,
    /// The lines that matched the query.
    pub matched_lines: Vec<GrepLineMatch>,
}

impl Display for GrepFileMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} on lines [{}]",
            self.file_path,
            self.matched_lines.iter().format(", ")
        )
    }
}

/// Info about a line that matched the grep query. This only contains the line
/// number for now, but can be extended in the future to include more info, e.g.
/// line contents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepLineMatch {
    /// The line number of the line that matched the query.
    pub line_number: usize,
}

impl Display for GrepLineMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.line_number)
    }
}

/// The result of a file globbing operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileGlobResult {
    Success { matched_files: String },
    Error(String),
    Cancelled,
}

impl Display for FileGlobResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileGlobResult::Success { matched_files } => {
                write!(f, "File glob completed: {matched_files}")
            }
            FileGlobResult::Error(error) => write!(f, "File glob error: {error}"),
            FileGlobResult::Cancelled => write!(f, "File glob cancelled"),
        }
    }
}

// The result of a v2 file globbing operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileGlobV2Result {
    Success {
        matched_files: Vec<FileGlobV2Match>,
        warnings: Option<String>,
    },
    Error(String),
    Cancelled,
}

impl Display for FileGlobV2Result {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileGlobV2Result::Success {
                matched_files,
                warnings,
            } => {
                write!(
                    f,
                    "File glob V2 completed: [{}] warnings: {:?}",
                    matched_files.iter().format(", "),
                    warnings
                )
            }
            FileGlobV2Result::Error(error) => write!(f, "File glob V2 error: {error}"),
            FileGlobV2Result::Cancelled => write!(f, "File glob V2 cancelled"),
        }
    }
}

// A match of a single file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileGlobV2Match {
    pub file_path: String,
}

impl Display for FileGlobV2Match {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.file_path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallMCPToolResult {
    Success { result: rmcp::model::CallToolResult },
    Error(String),
    Cancelled,
}

impl Display for CallMCPToolResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallMCPToolResult::Success { result } => {
                write!(
                    f,
                    "MCP tool call completed: [{result:?}]",
                    // results.iter().format(", ")
                )
            }
            CallMCPToolResult::Error(error) => write!(f, "MCP tool call error: {error}"),
            CallMCPToolResult::Cancelled => write!(f, "MCP tool call cancelled"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadMCPResourceResult {
    Success {
        resource_contents: Vec<rmcp::model::ResourceContents>,
    },
    Error(String),
    Cancelled,
}

impl Display for ReadMCPResourceResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadMCPResourceResult::Success { resource_contents } => {
                write!(f, "MCP resource read completed: [{resource_contents:?}]",)
            }
            ReadMCPResourceResult::Error(error) => write!(f, "MCP resource error: {error}"),
            ReadMCPResourceResult::Cancelled => write!(f, "MCP resource read cancelled"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadSkillResult {
    Success { content: FileContext },
    Error(String),
    Cancelled,
}

impl Display for ReadSkillResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadSkillResult::Success { content } => {
                write!(f, "Skill read successfully: {}", content.file_name)
            }
            ReadSkillResult::Error(error) => write!(f, "Skill read error: {error}"),
            ReadSkillResult::Cancelled => write!(f, "Skill read cancelled"),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum UseComputerResult {
    /// Computer use succeeded, with one result per requested action.
    Success(computer_use::ActionResult),
    Error(String),
    Cancelled,
}

impl Display for UseComputerResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UseComputerResult::Success(_) => write!(f, "Use computer completed"),
            UseComputerResult::Error(error) => write!(f, "Use computer error: {error}"),
            UseComputerResult::Cancelled => write!(f, "Use computer cancelled"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertReviewCommentsResult {
    Success { repo_path: String },
    Error { repo_path: String, message: String },
    Cancelled,
}

impl Display for InsertReviewCommentsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsertReviewCommentsResult::Success { repo_path } => {
                write!(f, "Inserted code review comments for {repo_path}")
            }
            InsertReviewCommentsResult::Error { repo_path, message } => {
                write!(
                    f,
                    "Error inserting code review comments for {repo_path}: {message}"
                )
            }
            InsertReviewCommentsResult::Cancelled => {
                write!(f, "Cancelled inserting code review comments")
            }
        }
    }
}

/// Screen dimensions for computer use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenDimensions {
    pub width_px: i32,
    pub height_px: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestComputerUseResult {
    /// Request was accepted, with the screen dimensions.
    Approved {
        screenshot: computer_use::Screenshot,
        platform: computer_use::Platform,
    },
    /// Request errored.
    Error(String),
    /// Request was cancelled or rejected by the user.
    Cancelled,
}

impl Display for RequestComputerUseResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestComputerUseResult::Approved { screenshot, .. } => {
                write!(
                    f,
                    "Request computer use accepted ({}x{})",
                    screenshot.original_width, screenshot.original_height
                )
            }
            RequestComputerUseResult::Error(error) => {
                write!(f, "Request computer use error: {error}")
            }
            RequestComputerUseResult::Cancelled => write!(f, "Request computer use cancelled"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchConversationResult {
    Success { directory_path: String },
    Error(String),
    Cancelled,
}

impl Display for FetchConversationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchConversationResult::Success { directory_path } => {
                write!(f, "Fetched conversation to {directory_path}")
            }
            FetchConversationResult::Error(error) => {
                write!(f, "Fetch conversation error: {error}")
            }
            FetchConversationResult::Cancelled => write!(f, "Fetch conversation cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StartAgentResult {
    Success {
        agent_id: String,
        #[serde(default)]
        version: StartAgentVersion,
    },
    Error {
        error: String,
        #[serde(default)]
        version: StartAgentVersion,
    },
    Cancelled {
        #[serde(default)]
        version: StartAgentVersion,
    },
}

impl StartAgentResult {
    /// Returns which start-agent tool schema version produced this result.
    pub fn version(&self) -> StartAgentVersion {
        match self {
            StartAgentResult::Success { version, .. }
            | StartAgentResult::Error { version, .. }
            | StartAgentResult::Cancelled { version } => *version,
        }
    }
}

impl Display for StartAgentResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartAgentResult::Success { agent_id, .. } => {
                write!(f, "Started agent with id {agent_id}")
            }
            StartAgentResult::Error { error, .. } => write!(f, "Start agent error: {error}"),
            StartAgentResult::Cancelled { .. } => write!(f, "Start agent cancelled"),
        }
    }
}

/// The terminal outcome of an orchestrate tool call.
///
/// Mirrors the proto `RunAgentsResult` oneof, with an additional
/// `Cancelled` variant used internally by the action machinery when the
/// user clicks Reject. The proto wire form for cancellation is the
/// generic `ToolCallResult.Cancel` marker; the conversion code emits
/// `ConvertToAPITypeError::Ignore` for `Cancelled` so the input
/// interceptor can synthesize the marker on the next outbound input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunAgentsResult {
    /// Orchestration launched. Carries the resolved configuration and one
    /// `AgentOutcome` per `agent_run_configs[]` entry, in input order.
    Launched {
        model_id: String,
        harness_type: String,
        execution_mode: RunAgentsLaunchedExecutionMode,
        agents: Vec<RunAgentsAgentOutcome>,
    },
    /// Declined for a non-error reason (currently disapproval).
    Denied { reason: String },
    /// Actual error path: server-side validation rejected the call, or the
    /// client could not begin the launch sequence at all.
    Failure { error: String },
    /// User rejected via the Reject button. Wire form is the generic
    /// `ToolCallResult.Cancel` marker, synthesized by the server's input
    /// interceptor on the next user input.
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunAgentsLaunchedExecutionMode {
    Local,
    Remote {
        environment_id: String,
        worker_host: String,
        computer_use_enabled: bool,
    },
}

/// Per-agent outcome reported in `RunAgentsResult::Launched.agents`.
/// Order mirrors the input order of `RunAgents.agent_run_configs[]`,
/// regardless of which `CreateAgentTask` call returned first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunAgentsAgentOutcome {
    pub name: String,
    pub kind: RunAgentsAgentOutcomeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunAgentsAgentOutcomeKind {
    Launched { agent_id: String },
    Failed { error: String },
}

impl Display for RunAgentsResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunAgentsResult::Launched { agents, .. } => {
                let launched = agents
                    .iter()
                    .filter(|a| matches!(a.kind, RunAgentsAgentOutcomeKind::Launched { .. }))
                    .count();
                write!(
                    f,
                    "Orchestrate launched ({launched}/{} agents started)",
                    agents.len()
                )
            }
            RunAgentsResult::Denied { reason } => {
                write!(f, "Orchestrate launch denied: {reason}")
            }
            RunAgentsResult::Failure { error } => write!(f, "Orchestrate failure: {error}"),
            RunAgentsResult::Cancelled => write!(f, "Orchestrate cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SendMessageToAgentResult {
    Success { message_id: String },
    Error(String),
    Cancelled,
}

impl Display for SendMessageToAgentResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendMessageToAgentResult::Success { message_id } => {
                write!(f, "Sent message with id {message_id}")
            }
            SendMessageToAgentResult::Error(error) => write!(f, "Send message error: {error}"),
            SendMessageToAgentResult::Cancelled => write!(f, "Send message cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum TransferShellCommandControlToUserResult {
    Snapshot {
        block_id: BlockId,
        grid_contents: String,
        cursor: String,
        is_alt_screen_active: bool,
        is_preempted: bool,
    },
    CommandFinished {
        block_id: BlockId,
        output: String,
        exit_code: ExitCode,
        start_ts: Option<DateTime<Local>>,
        completed_ts: Option<DateTime<Local>>,
    },
    Cancelled,
    Error(ShellCommandError),
}

impl Display for TransferShellCommandControlToUserResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Snapshot { .. } => {
                write!(f, "Transferred control to user, sent snapshot")
            }
            Self::CommandFinished {
                output, exit_code, ..
            } => write!(
                f,
                "Command finished while user had control, exit code {}:\n{output}",
                exit_code.value()
            ),
            Self::Cancelled => write!(f, "Transfer shell command control to user cancelled"),
            Self::Error(e) => write!(f, "Transfer shell command control to user failed: {e:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskUserQuestionAnswerItem {
    Answered {
        question_id: String,
        selected_options: Vec<String>,
        other_text: String,
    },
    Skipped {
        question_id: String,
    },
}

impl AskUserQuestionAnswerItem {
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped { .. })
    }

    pub fn display_text(&self) -> String {
        match self {
            Self::Answered {
                selected_options,
                other_text,
                ..
            } => {
                let mut parts = selected_options.clone();
                if !other_text.is_empty() {
                    parts.push(format!("Other: {other_text}"));
                }
                parts.join(", ")
            }
            Self::Skipped { .. } => "Skipped".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskUserQuestionResult {
    Success {
        answers: Vec<AskUserQuestionAnswerItem>,
    },
    Error(String),
    Cancelled,
    /// The question was skipped automatically because the conversation is in auto-approve mode.
    SkippedByAutoApprove {
        question_ids: Vec<String>,
    },
}

impl Display for AskUserQuestionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskUserQuestionResult::Success { answers } => {
                write!(
                    f,
                    "Ask user question completed with {} answer(s)",
                    answers.len()
                )
            }
            AskUserQuestionResult::Error(msg) => write!(f, "Ask user question error: {msg}"),
            AskUserQuestionResult::Cancelled => write!(f, "Ask user question cancelled"),
            AskUserQuestionResult::SkippedByAutoApprove { question_ids } => {
                write!(
                    f,
                    "Ask user question skipped (auto-approve) with {} skipped question(s)",
                    question_ids.len()
                )
            }
        }
    }
}
