mod convert;

use std::{fmt::Display, ops::Range, path::PathBuf, time::Duration};

use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use strum_macros::EnumDiscriminants;
use uuid::Uuid;
use warp_terminal::model::BlockId;

use crate::{
    agent::{
        action_result::{
            AIAgentActionResultType, AskUserQuestionResult, CallMCPToolResult,
            CreateDocumentsResult, EditDocumentsResult, FetchConversationResult, FileGlobResult,
            FileGlobV2Result, GrepResult, InsertReviewCommentsResult, ReadDocumentsResult,
            ReadFilesResult, ReadMCPResourceResult, ReadShellCommandOutputResult, ReadSkillResult,
            RequestCommandOutputResult, RequestComputerUseResult, RequestFileEditsResult,
            RunAgentsResult, SearchCodebaseResult, SendMessageToAgentResult, StartAgentResult,
            StartAgentVersion, SuggestNewConversationResult, SuggestPromptResult,
            TransferShellCommandControlToUserResult, UploadArtifactResult, UseComputerResult,
            WriteToLongRunningShellCommandResult,
        },
        AIAgentCitation, FileLocations,
    },
    diff_validation::ParsedDiff,
    document::AIDocumentId,
    skills::SkillReference,
};
pub use warp_multi_agent_api::LifecycleEventType;

#[derive(Debug, Clone, Eq, PartialEq, EnumDiscriminants)]
pub enum AIAgentActionType {
    /// The AI requested the output for a given command to be retrieved as context in responding to
    /// a user's query.
    RequestCommandOutput {
        command: String,

        /// [`Some(true)`] iff the LLM thinks that the `command` is readonly and doesn't produce side-effects.
        is_read_only: Option<bool>,

        /// [`Some(true)`] iff the LLM thinks that the `command` is risky and should require user confirmation.
        is_risky: Option<bool>,

        /// `true` if the client should wait until the command is completed and report the finish output as the result.
        ///
        /// If `false` _and_ the command is long-running, a snapshot of the command output is taken and reported as the
        /// result instead.
        wait_until_completion: bool,

        /// [`Some(true)`] iff the LLM thinks that the `command` might invoke a pager.
        uses_pager: Option<bool>,

        /// The AI's rationale for requesting a command.
        rationale: Option<String>,

        /// The citations for the command.
        citations: Vec<AIAgentCitation>,
    },

    WriteToLongRunningShellCommand {
        block_id: BlockId,
        input: bytes::Bytes,
        mode: AIAgentPtyWriteMode,
    },

    /// AI requested getting the content of some files.
    ReadFiles(ReadFilesRequest),

    /// AI requested uploading a local file as a conversation artifact.
    UploadArtifact(UploadArtifactRequest),

    SearchCodebase(SearchCodebaseRequest),

    /// AI requested a vector of edits. Each edit holds a list of diffs on a single code file.
    RequestFileEdits {
        file_edits: Vec<FileEdit>,
        title: Option<String>,
    },

    Grep {
        queries: Vec<String>,
        path: String,
    },

    FileGlob {
        patterns: Vec<String>,
        path: Option<String>,
    },

    FileGlobV2 {
        patterns: Vec<String>,
        search_dir: Option<String>,
        // TODO(matthew): Maybe implement client side depth and result limits.
    },

    ReadMCPResource {
        server_id: Option<Uuid>,
        name: String,
        /// The unique URI for the resource. Prefer using this to identify
        /// a resource over [`ReadMCPResource::name`], when available.
        ///
        /// We should phase out `name` eventually and make this non-optional.
        uri: Option<String>,
    },

    CallMCPTool {
        server_id: Option<Uuid>,
        name: String,
        input: serde_json::Value,
    },

    SuggestNewConversation {
        message_id: String,
    },

    SuggestPrompt(SuggestPromptRequest),

    InitProject,
    OpenCodeReview,

    ReadDocuments(ReadDocumentsRequest),
    EditDocuments(EditDocumentsRequest),
    CreateDocuments(CreateDocumentsRequest),

    ReadShellCommandOutput {
        block_id: BlockId,
        delay: Option<ShellCommandDelay>,
    },

    UseComputer(UseComputerRequest),

    InsertCodeReviewComments {
        repo_path: PathBuf,
        comments: Vec<InsertReviewComment>,
        base_branch: Option<String>,
    },

    RequestComputerUse(RequestComputerUseRequest),

    // AI requested to read a skill.
    ReadSkill(ReadSkillRequest),

    FetchConversation {
        conversation_id: String,
    },

    StartAgent {
        version: StartAgentVersion,
        name: String,
        prompt: String,
        execution_mode: StartAgentExecutionMode,
        lifecycle_subscription: Option<Vec<LifecycleEventType>>,
    },

    SendMessageToAgent {
        addresses: Vec<String>,
        subject: String,
        message: String,
    },
    /// Transfer control of a running shell command to the user.
    TransferShellCommandControlToUser {
        /// The reason provided by the agent for transferring control.
        reason: String,
    },

    AskUserQuestion {
        questions: Vec<AskUserQuestionItem>,
    },

    /// AI requested batched orchestration of one-or-more child agents that
    /// share run-wide configuration (model, harness, execution mode).
    /// The full per-child prompt is computed at dispatch time as
    /// `base_prompt + "\n\n" + agent_run_configs[i].prompt` (or just
    /// `base_prompt` when the per-agent `prompt` is empty).
    RunAgents(RunAgentsRequest),
}

/// Run-wide + per-agent configuration for a `RunAgents` tool call.
///
/// Mirrors the proto `RunAgents` message. Server-resolved fields
/// (`model_id`, `harness_type`, `execution_mode`'s remote details) are
/// folded in by the server's final tool-call re-emission once the
/// payload is complete; the client renders the full layout from a
/// fully-resolved instance only.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RunAgentsRequest {
    pub summary: String,
    pub base_prompt: String,
    pub skills: Vec<SkillReference>,
    pub model_id: String,
    pub harness_type: String,
    pub execution_mode: RunAgentsExecutionMode,
    pub agent_run_configs: Vec<RunAgentsAgentRunConfig>,
    pub plan_id: String,
    /// Resolved client-side at dispatch time; not serialized to the wire.
    pub harness_auth_secret_name: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RunAgentsExecutionMode {
    Local,
    Remote {
        environment_id: String,
        worker_host: String,
        computer_use_enabled: bool,
    },
}

impl RunAgentsExecutionMode {
    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RunAgentsAgentRunConfig {
    pub name: String,
    pub prompt: String,
    pub title: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StartAgentExecutionMode {
    Local {
        /// `None` selects the legacy embedded local child-agent flow.
        /// `Some(...)` selects a third-party CLI harness to launch locally.
        harness_type: Option<String>,
        /// `None` inherits the parent agent's preferred LLM (legacy behavior).
        /// `Some(_)` overrides the child's preferred LLM with the supplied
        /// model id (used by the orchestrate confirmation card so the user's
        /// model selection is honored on local launches).
        model_id: Option<String>,
    },
    Remote {
        environment_id: String,
        skill_references: Vec<SkillReference>,
        model_id: String,
        computer_use_enabled: bool,
        worker_host: String,
        harness_type: String,
        title: String,
        /// Name of a managed secret to forward as the authentication
        /// credential for the remote child when running a non-Oz harness.
        /// `None` means no client-side secret was selected — the remote
        /// environment falls back to its own ambient credentials.
        auth_secret_name: Option<String>,
    },
}

impl StartAgentExecutionMode {
    /// Constructs a local execution mode using the legacy v1 default harness.
    pub fn local_with_defaults() -> Self {
        Self::Local {
            harness_type: None,
            model_id: None,
        }
    }
    /// Constructs a local execution mode for a specific third-party harness.
    pub fn local_harness(harness_type: String) -> Self {
        Self::Local {
            harness_type: Some(harness_type),
            model_id: None,
        }
    }
    /// Constructs a remote execution mode using the legacy v1 defaults for
    /// fields that were added later in StartAgentV2.
    pub fn remote_with_defaults(environment_id: String) -> Self {
        Self::Remote {
            environment_id,
            skill_references: Vec::new(),
            model_id: String::new(),
            computer_use_enabled: false,
            worker_host: String::new(),
            harness_type: String::new(),
            title: String::new(),
            auth_secret_name: None,
        }
    }
}
impl AIAgentActionType {
    pub fn is_request_command_output(&self) -> bool {
        matches!(self, Self::RequestCommandOutput { .. })
    }

    pub fn is_read_files(&self) -> bool {
        matches!(self, Self::ReadFiles(..))
    }

    pub fn is_search_codebase(&self) -> bool {
        matches!(self, Self::SearchCodebase(..))
    }

    pub fn is_grep(&self) -> bool {
        matches!(self, Self::Grep { .. })
    }

    pub fn is_file_glob(&self) -> bool {
        matches!(self, Self::FileGlob { .. } | Self::FileGlobV2 { .. })
    }

    pub fn is_write_to_shell_command(&self) -> bool {
        matches!(self, Self::WriteToLongRunningShellCommand { .. })
    }

    pub fn cancelled_result(&self) -> AIAgentActionResultType {
        match self {
            Self::RequestCommandOutput { .. } => AIAgentActionResultType::RequestCommandOutput(
                RequestCommandOutputResult::CancelledBeforeExecution,
            ),
            Self::RequestFileEdits { .. } => {
                AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Cancelled)
            }
            Self::ReadFiles(..) => AIAgentActionResultType::ReadFiles(ReadFilesResult::Cancelled),
            Self::UploadArtifact(..) => {
                AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Cancelled)
            }
            Self::SearchCodebase(..) => {
                AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Cancelled)
            }
            Self::Grep { .. } => AIAgentActionResultType::Grep(GrepResult::Cancelled),
            Self::FileGlob { .. } => AIAgentActionResultType::FileGlob(FileGlobResult::Cancelled),
            Self::FileGlobV2 { .. } => {
                AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Cancelled)
            }
            Self::WriteToLongRunningShellCommand { .. } => {
                AIAgentActionResultType::WriteToLongRunningShellCommand(
                    WriteToLongRunningShellCommandResult::Cancelled,
                )
            }
            Self::CallMCPTool { .. } => {
                AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Cancelled)
            }
            Self::ReadMCPResource { .. } => {
                AIAgentActionResultType::ReadMCPResource(ReadMCPResourceResult::Cancelled)
            }
            Self::SuggestNewConversation { .. } => AIAgentActionResultType::SuggestNewConversation(
                SuggestNewConversationResult::Cancelled,
            ),
            Self::SuggestPrompt { .. } => {
                AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled)
            }
            Self::OpenCodeReview => AIAgentActionResultType::OpenCodeReview,
            Self::InitProject => AIAgentActionResultType::InitProject,
            Self::ReadDocuments(_) => {
                AIAgentActionResultType::ReadDocuments(ReadDocumentsResult::Cancelled)
            }
            Self::EditDocuments(_) => {
                AIAgentActionResultType::EditDocuments(EditDocumentsResult::Cancelled)
            }
            Self::CreateDocuments(_) => {
                AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Cancelled)
            }
            Self::ReadShellCommandOutput { .. } => AIAgentActionResultType::ReadShellCommandOutput(
                ReadShellCommandOutputResult::Cancelled,
            ),
            Self::UseComputer(_) => {
                AIAgentActionResultType::UseComputer(UseComputerResult::Cancelled)
            }
            Self::InsertCodeReviewComments { .. } => {
                AIAgentActionResultType::InsertReviewComments(InsertReviewCommentsResult::Cancelled)
            }
            Self::RequestComputerUse(_) => {
                AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Cancelled)
            }
            Self::ReadSkill(_) => AIAgentActionResultType::ReadSkill(ReadSkillResult::Cancelled),
            Self::FetchConversation { .. } => {
                AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled)
            }
            Self::StartAgent { version, .. } => {
                AIAgentActionResultType::StartAgent(StartAgentResult::Cancelled {
                    version: *version,
                })
            }
            Self::SendMessageToAgent { .. } => {
                AIAgentActionResultType::SendMessageToAgent(SendMessageToAgentResult::Cancelled)
            }
            Self::TransferShellCommandControlToUser { .. } => {
                AIAgentActionResultType::TransferShellCommandControlToUser(
                    TransferShellCommandControlToUserResult::Cancelled,
                )
            }
            Self::AskUserQuestion { .. } => {
                AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled)
            }
            Self::RunAgents(_) => AIAgentActionResultType::RunAgents(RunAgentsResult::Cancelled),
        }
    }

    pub fn user_friendly_name(&self) -> String {
        match self {
            Self::RequestCommandOutput { command, .. } => {
                format!("Run command: {command}")
            }
            Self::WriteToLongRunningShellCommand { .. } => {
                "Write to long running shell command".to_string()
            }
            Self::ReadFiles(_) => "Read files".to_string(),
            Self::UploadArtifact(_) => "Upload artifact".to_string(),
            Self::SearchCodebase(_) => "Search codebase".to_string(),
            Self::RequestFileEdits { file_edits, .. } => {
                let file_names = file_edits.iter().filter_map(|edit| edit.file()).join(", ");
                format!("Edit {file_names}")
            }
            Self::Grep { .. } => "Grep".to_string(),
            Self::FileGlob { .. } | Self::FileGlobV2 { .. } => "File glob".to_string(),
            Self::ReadMCPResource { .. } => "Read mcp resource".to_string(),
            Self::CallMCPTool { .. } => "Call mcp tool".to_string(),
            Self::SuggestNewConversation { .. } => "Suggest new conversation".to_string(),
            Self::SuggestPrompt { .. } => "Suggest prompt".to_string(),
            Self::InitProject => "Init project".to_string(),
            Self::OpenCodeReview => "Open code review".to_string(),
            Self::ReadDocuments(_) => "Read documents".to_string(),
            Self::EditDocuments(_) => "Edit documents".to_string(),
            Self::CreateDocuments(_) => "Create documents".to_string(),
            Self::ReadShellCommandOutput { .. } => "Read shell command output".to_string(),
            Self::UseComputer(_) => "Use computer".to_string(),
            Self::InsertCodeReviewComments { comments, .. } => {
                format!("Insert {} code review comments", comments.len())
            }
            Self::RequestComputerUse(_) => "Request computer use".to_string(),
            Self::ReadSkill(_) => "Read skill".to_string(),
            Self::FetchConversation { .. } => "Fetch conversation".to_string(),
            Self::StartAgent { name, .. } => format!("Start agent: {name}"),
            Self::SendMessageToAgent { subject, .. } => format!("Send message: {subject}"),
            Self::TransferShellCommandControlToUser { .. } => {
                "Transfer shell command control to user".to_string()
            }
            Self::AskUserQuestion { questions } => {
                format!("Ask user {} question(s)", questions.len())
            }
            Self::RunAgents(req) => {
                format!("Orchestrate {} agent(s)", req.agent_run_configs.len())
            }
        }
    }
}

impl Display for AIAgentActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIAgentActionType::RequestCommandOutput {
                command,
                is_read_only,
                uses_pager,
                ..
            } => {
                write!(
                    f,
                    "RequestCommandOutput: {command} (read_only: {is_read_only:?}, pager: {uses_pager:?})"
                )
            }
            AIAgentActionType::WriteToLongRunningShellCommand {
                block_id,
                input,
                mode,
            } => {
                write!(
                    f,
                    "WriteToLongRunningShellCommand (block id: {block_id}): {input:?}, {mode:?}",
                )
            }
            AIAgentActionType::ReadFiles(request) => {
                write!(f, "{request}")
            }
            AIAgentActionType::UploadArtifact(request) => {
                write!(f, "{request}")
            }
            AIAgentActionType::SearchCodebase(request) => {
                write!(f, "{request}")
            }
            AIAgentActionType::RequestFileEdits { file_edits, title } => {
                let file_names = file_edits
                    .iter()
                    .filter_map(|edit| edit.file())
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(title) = title {
                    write!(f, "RequestFileEdits '{title}': [{file_names}]")
                } else {
                    write!(f, "RequestFileEdits: [{file_names}]")
                }
            }
            AIAgentActionType::Grep { queries, path } => {
                write!(f, "Grep: [{}] in {}", queries.join(", "), path)
            }
            AIAgentActionType::FileGlob { patterns, path } => {
                let path_str = path.as_deref().unwrap_or(".");
                write!(f, "FileGlob: [{}] in {}", patterns.join(", "), path_str)
            }
            AIAgentActionType::FileGlobV2 {
                patterns,
                search_dir,
            } => {
                let path_str = search_dir.as_deref().unwrap_or(".");
                write!(f, "FileGlobV2: [{}] in {}", patterns.join(", "), path_str)
            }
            AIAgentActionType::ReadMCPResource {
                server_id: _,
                name,
                uri,
            } => {
                if let Some(uri) = uri {
                    write!(f, "ReadMCPResource: {name} ({uri})")
                } else {
                    write!(f, "ReadMCPResource: {name}")
                }
            }
            AIAgentActionType::CallMCPTool {
                server_id: _,
                name,
                input,
            } => {
                write!(f, "CallMCPTool: {name} with input {input:?}")
            }
            AIAgentActionType::SuggestNewConversation { message_id } => {
                write!(f, "SuggestNewConversation: {message_id}")
            }
            AIAgentActionType::SuggestPrompt(request) => {
                write!(f, "SuggestPrompt: {request:?}")
            }
            AIAgentActionType::InitProject => {
                write!(f, "InitProject")
            }
            AIAgentActionType::OpenCodeReview => {
                write!(f, "OpenCodeReview")
            }
            AIAgentActionType::ReadDocuments(request) => {
                let ids: Vec<String> = request
                    .document_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect();
                write!(f, "ReadDocuments: [{}]", ids.join(", "))
            }
            AIAgentActionType::EditDocuments(request) => {
                write!(f, "EditDocuments: {} diffs", request.diffs.len())
            }
            AIAgentActionType::CreateDocuments(request) => {
                write!(f, "CreateDocuments: {} documents", request.documents.len())
            }
            AIAgentActionType::ReadShellCommandOutput { delay, block_id } => {
                let delay = match delay {
                    Some(ShellCommandDelay::Duration(duration)) => {
                        format!("{} seconds", duration.as_secs())
                    }
                    Some(ShellCommandDelay::OnCompletion) => "on completion".to_string(),
                    None => "no".to_string(),
                };
                write!(
                    f,
                    "ReadShellCommandOutput (block id: {block_id}): with {delay} delay"
                )
            }
            AIAgentActionType::UseComputer(req) => {
                write!(
                    f,
                    "UseComputer: {} actions, screenshot_params={:?}",
                    req.actions.len(),
                    req.screenshot_params
                )
            }
            AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
                let file_paths = comments
                    .iter()
                    .filter_map(|c| {
                        c.comment_location
                            .as_ref()
                            .map(|loc| loc.relative_file_path.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "InsertCodeReviewComments: {} comments on [{}]",
                    comments.len(),
                    file_paths
                )
            }
            AIAgentActionType::RequestComputerUse(req) => {
                write!(f, "RequestComputerUse: {}", req.task_summary)
            }
            AIAgentActionType::ReadSkill(req) => {
                write!(f, "ReadSkill: {}", req.skill)
            }
            AIAgentActionType::FetchConversation { conversation_id } => {
                write!(f, "FetchConversation: {conversation_id}")
            }
            AIAgentActionType::StartAgent { name, .. } => {
                write!(f, "StartAgent: {name}")
            }
            AIAgentActionType::SendMessageToAgent {
                addresses, subject, ..
            } => {
                write!(
                    f,
                    "SendMessageToAgent: to=[{}] subject={subject}",
                    addresses.join(", ")
                )
            }
            AIAgentActionType::TransferShellCommandControlToUser { reason } => {
                write!(f, "TransferShellCommandControlToUser: {reason}")
            }
            AIAgentActionType::AskUserQuestion { questions } => {
                write!(f, "AskUserQuestion: {} question(s)", questions.len())
            }
            AIAgentActionType::RunAgents(req) => {
                let names = req
                    .agent_run_configs
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "Orchestrate: summary='{}' agents=[{names}]", req.summary,)
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum AskUserQuestionType {
    MultipleChoice {
        is_multiselect: bool,
        options: Vec<AskUserQuestionOption>,
        supports_other: bool,
    },
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AskUserQuestionOption {
    pub label: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AskUserQuestionItem {
    pub question_id: String,
    pub question: String,
    pub question_type: AskUserQuestionType,
}

impl AskUserQuestionItem {
    pub fn is_multiselect(&self) -> bool {
        match &self.question_type {
            AskUserQuestionType::MultipleChoice { is_multiselect, .. } => *is_multiselect,
        }
    }

    pub fn multiple_choice_options(&self) -> Option<&[AskUserQuestionOption]> {
        match &self.question_type {
            AskUserQuestionType::MultipleChoice { options, .. } => Some(options),
        }
    }

    pub fn supports_other(&self) -> bool {
        match &self.question_type {
            AskUserQuestionType::MultipleChoice { supports_other, .. } => *supports_other,
        }
    }

    pub fn numbered_option_count(&self) -> usize {
        self.multiple_choice_options()
            .map_or(0, |options| options.len())
            + usize::from(self.supports_other())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReadFilesRequest {
    pub locations: Vec<FileLocations>,
}

impl Display for ReadFilesRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file_names = self
            .locations
            .iter()
            .map(|loc| loc.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write!(f, "ReadFiles: [{file_names}]")
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UploadArtifactRequest {
    pub file_path: String,
    pub description: Option<String>,
}

impl Display for UploadArtifactRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UploadArtifact: {}", self.file_path)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SearchCodebaseRequest {
    pub query: String,

    /// Optional list of file paths to search through.  This is used to narrow down the search scope.
    /// Files are searched if any of the partial paths are a substring of the file path.
    pub partial_paths: Option<Vec<String>>,

    /// Optional absolute path to the codebase that we want to search. If not
    /// provided, we will use the codebase in the user's current directory.
    pub codebase_path: Option<String>,
}

impl Display for SearchCodebaseRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SearchCodebase: {}", self.query)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReadDocumentsRequest {
    pub document_ids: Vec<AIDocumentId>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentDiff {
    pub document_id: AIDocumentId,
    pub search: String,
    pub replace: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EditDocumentsRequest {
    pub diffs: Vec<DocumentDiff>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DocumentToCreate {
    pub content: String,
    pub title: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CreateDocumentsRequest {
    pub documents: Vec<DocumentToCreate>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UseComputerRequest {
    pub action_summary: String,
    pub actions: Vec<computer_use::Action>,
    /// If set, a screenshot will be captured after the actions are executed.
    pub screenshot_params: Option<computer_use::ScreenshotParams>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RequestComputerUseRequest {
    /// A short summary of the task.
    pub task_summary: String,
    /// If set, a screenshot will be captured after the actions are executed.
    pub screenshot_params: Option<computer_use::ScreenshotParams>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReadSkillRequest {
    pub skill: SkillReference,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ShellCommandDelay {
    Duration(Duration),
    OnCompletion,
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, EnumDiscriminants)]
pub enum AIAgentPtyWriteMode {
    #[default]
    Raw,
    Line,
    Block,
}

impl AIAgentPtyWriteMode {
    /// Decorates input bytes according to the write mode.
    pub fn decorate_bytes(
        self,
        bytes: impl Into<Vec<u8>>,
        is_bracketed_paste_enabled: bool,
    ) -> Vec<u8> {
        use warp_terminal::model::escape_sequences;

        let bytes = bytes.into();
        match self {
            AIAgentPtyWriteMode::Raw => bytes,
            AIAgentPtyWriteMode::Line => {
                // Move to beginning of line, write input, then submit (Enter).
                let mut v = Vec::with_capacity(bytes.len() + 2);
                // ^A (SOH) is "beginning of line" for readline/prompt-toolkit style editors.
                v.push(escape_sequences::C0::SOH);
                v.extend_from_slice(&bytes);
                cfg_if::cfg_if! {
                    if #[cfg(target_os = "windows")] {
                        // Use CR to submit on Windows hosts.
                        v.push(escape_sequences::C0::CR);
                    } else {
                        // Use LF to submit on POSIX.
                        v.push(escape_sequences::C0::LF);
                    }
                }
                v
            }
            AIAgentPtyWriteMode::Block => {
                if is_bracketed_paste_enabled {
                    escape_sequences::BRACKETED_PASTE_START
                        .iter()
                        .copied()
                        .chain(bytes)
                        .chain(escape_sequences::BRACKETED_PASTE_END.iter().copied())
                        .collect()
                } else {
                    bytes
                }
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertReviewComment {
    pub comment_id: String,
    pub author: String,
    pub last_modified_timestamp: String,
    pub comment_body: String,
    pub parent_comment_id: Option<String>,
    /// The file and line range the comment is attached to.
    /// If None, the comment applies to the whole diff set.
    pub comment_location: Option<InsertedCommentLocation>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertedCommentLocation {
    /// Repo-relative path of the file the comment is attached to.
    pub relative_file_path: String,
    /// The specific line range the comment is attached to.
    /// If None, the comment applies to the whole file.
    pub line: Option<InsertedCommentLine>,
}

/// The side of a diff that a comment is attached to.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CommentSide {
    /// The right side of the diff (new file / additions).
    Right,
    /// The left side of the diff (old file / deletions).
    Left,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertedCommentLine {
    pub comment_line_range: Range<usize>,
    /// The diff hunk line range overlaps with the comment line range
    /// but may not match it exactly. We need this in order to be able
    /// to find the full diff hunk this comment is attached to.
    pub diff_hunk_line_range: Range<usize>,
    /// The diff hunk text is needed to find where to attach comments
    /// when line numbers on the local and remote branches have diverged.
    pub diff_hunk_text: String,
    /// The side of the diff the comment is attached to.
    pub side: Option<CommentSide>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SuggestPromptRequest {
    UnitTestsSuggestion {
        query: String,
        title: String,
        description: String,
    },
    PromptSuggestion {
        prompt: String,
        label: Option<String>,
    },
}

/// A file-editing request from the agent.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FileEdit {
    /// Edit an existing file by applying a diff.
    Edit(ParsedDiff),
    /// Create a new file.
    Create {
        file: Option<String>,
        content: Option<String>,
    },
    /// Delete an existing file.
    Delete { file: Option<String> },
}

impl FileEdit {
    /// The path to the file this edit applies to.
    pub fn file(&self) -> Option<&str> {
        match self {
            Self::Edit(diff) => diff.file().map(|s| s.as_str()),
            Self::Create { file, .. } => file.as_deref(),
            Self::Delete { file } => file.as_deref(),
        }
    }
}
