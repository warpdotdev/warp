pub mod text {
    use std::collections::HashSet;
    use std::fmt;
    use std::io::{self, Write};

    const CANCELLED_MESSAGE: &str = "<cancelled>";

    use ai::agent::action_result::{FetchConversationResult, ReadSkillResult, UseComputerResult};
    use itertools::Itertools;

    use crate::ai::agent::{
        AIAgentActionType, AIAgentInput, AIAgentOutput, AIAgentOutputMessageType, AIAgentTodo,
        ArtifactCreatedData, CallMCPToolResult, FileGlobResult, FileGlobV2Result, GrepResult,
        ReadFilesResult, ReadMCPResourceResult, RequestCommandOutputResult, RequestFileEditsResult,
        SearchCodebaseResult, SuggestNewConversationResult, SuggestPromptResult, TodoOperation,
        UploadArtifactResult, WebFetchStatus, WebSearchStatus,
        WriteToLongRunningShellCommandResult,
    };
    use crate::AIAgentActionResultType;

    /// Format an agent input as a human-readable string. For action results, it's assumed that
    /// the action is shown immediately before this result.
    ///
    /// Unlike other contexts where we format agent inputs, this is a user-facing API. Consider
    /// what details are relevant and acceptable to expose.
    pub fn format_input<W: Write>(input: &AIAgentInput, w: &mut W) -> io::Result<()> {
        match input {
            AIAgentInput::UserQuery { .. }
            | AIAgentInput::AutoCodeDiffQuery { .. }
            | AIAgentInput::CreateNewProject { .. }
            | AIAgentInput::CloneRepository { .. }
            | AIAgentInput::InitProjectRules { .. }
            | AIAgentInput::CodeReview { .. }
            | AIAgentInput::FetchReviewComments { .. }
            | AIAgentInput::CreateEnvironment { .. }
            | AIAgentInput::SummarizeConversation { .. }
            | AIAgentInput::InvokeSkill { .. }
            | AIAgentInput::StartFromAmbientRunPrompt { .. }
            | AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::PassiveSuggestionResult { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => {
                // Do not include the user query, since it's already provided as input to the agent.
                Ok(())
            }
            // These input types should not occur in a SDK-run agent.
            AIAgentInput::ResumeConversation { .. }
            | AIAgentInput::TriggerPassiveSuggestion { .. } => Ok(()),
            AIAgentInput::ActionResult { result, .. } => match &result.result {
                AIAgentActionResultType::RequestCommandOutput(result) => match result {
                    RequestCommandOutputResult::Completed {
                        command,
                        output,
                        exit_code,
                        ..
                    } => writeln!(w, "{output}\n\n (`{command}` exited with code {exit_code})"),
                    RequestCommandOutputResult::LongRunningCommandSnapshot { command, .. } => {
                        writeln!(w, "`{command}` is still running...")
                    }
                    RequestCommandOutputResult::CancelledBeforeExecution => {
                        writeln!(w, "{CANCELLED_MESSAGE}")
                    }
                    RequestCommandOutputResult::Denylisted { .. } => {
                        writeln!(
                            w,
                            "Command was not allowed to run due to presence on denylist"
                        )
                    }
                },
                AIAgentActionResultType::WriteToLongRunningShellCommand(result) => match result {
                    WriteToLongRunningShellCommandResult::Snapshot { .. } => {
                        writeln!(w, "Command is still running...")
                    }
                    WriteToLongRunningShellCommandResult::CommandFinished {
                        output,
                        exit_code,
                        ..
                    } => writeln!(w, "{output}\n\n (exited with code {exit_code})"),
                    WriteToLongRunningShellCommandResult::Cancelled => {
                        writeln!(w, "{CANCELLED_MESSAGE}")
                    }
                    WriteToLongRunningShellCommandResult::Error(_) => {
                        writeln!(w, "Failed to write to command.")
                    }
                },
                AIAgentActionResultType::RequestFileEdits(result) => match result {
                    RequestFileEditsResult::Success {
                        diff,
                        updated_files,
                        deleted_files,
                        ..
                    } => {
                        writeln!(
                            w,
                            "Updated {} files, deleted {} files:\n```diff\n{diff}\n```",
                            updated_files.len(),
                            deleted_files.len()
                        )
                    }
                    RequestFileEditsResult::Cancelled => {
                        writeln!(w, "{CANCELLED_MESSAGE}")
                    }
                    RequestFileEditsResult::DiffApplicationFailed { error } => {
                        writeln!(w, "Editing files failed: {error}")
                    }
                },
                AIAgentActionResultType::ReadFiles(result) => match result {
                    ReadFilesResult::Success { .. } => Ok(()),
                    ReadFilesResult::Error(error) => writeln!(w, "Reading files failed: {error}"),
                    ReadFilesResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::UploadArtifact(result) => match result {
                    UploadArtifactResult::Success {
                        artifact_uid,
                        filepath,
                        ..
                    } => match filepath {
                        Some(filepath) => {
                            writeln!(w, "Uploaded artifact {artifact_uid} from {filepath}")
                        }
                        None => writeln!(w, "Uploaded artifact {artifact_uid}"),
                    },
                    UploadArtifactResult::Error(error) => {
                        writeln!(w, "Uploading artifact failed: {error}")
                    }
                    UploadArtifactResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::SearchCodebase(result) => match result {
                    SearchCodebaseResult::Success { files } => {
                        writeln!(w, "Codebase search results:")?;
                        for file in files {
                            writeln!(w, "- {file}")?;
                        }
                        Ok(())
                    }
                    SearchCodebaseResult::Failed { message, .. } => {
                        writeln!(w, "Searching codebase failed: {message}")
                    }
                    SearchCodebaseResult::Cancelled => todo!(),
                },
                AIAgentActionResultType::Grep(result) => match result {
                    GrepResult::Success { matched_files } => {
                        for file in matched_files {
                            writeln!(w, "- {file}")?;
                        }
                        Ok(())
                    }
                    GrepResult::Error(error) => writeln!(w, "grep failed: {error}"),
                    GrepResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::FileGlob(result) => match result {
                    FileGlobResult::Success { matched_files } => writeln!(w, "{matched_files}"),
                    FileGlobResult::Error(error) => writeln!(w, "find failed: {error}"),
                    FileGlobResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::FileGlobV2(result) => match result {
                    FileGlobV2Result::Success { matched_files, .. } => {
                        for file in matched_files {
                            writeln!(w, "- {file}")?;
                        }
                        Ok(())
                    }
                    FileGlobV2Result::Error(error) => writeln!(w, "find failed: {error}"),
                    FileGlobV2Result::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::ReadMCPResource(result) => match result {
                    ReadMCPResourceResult::Success { resource_contents } => {
                        for resource in resource_contents {
                            write!(w, "- ")?;
                            match resource {
                                rmcp::model::ResourceContents::TextResourceContents {
                                    uri,
                                    mime_type,
                                    text,
                                    ..
                                } => writeln!(
                                    w,
                                    "{uri} ({})\n{text}",
                                    mime_type.as_deref().unwrap_or("text/plain")
                                )?,
                                rmcp::model::ResourceContents::BlobResourceContents {
                                    uri,
                                    mime_type,
                                    ..
                                } => writeln!(
                                    w,
                                    "{uri} ({})",
                                    mime_type.as_deref().unwrap_or("text/plain")
                                )?,
                            }
                        }
                        Ok(())
                    }
                    ReadMCPResourceResult::Error(error) => {
                        writeln!(w, "Reading MCP resource failed: {error}")
                    }
                    ReadMCPResourceResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::CallMCPTool(result) => {
                    match result {
                        CallMCPToolResult::Success { result } => {
                            for content in &result.content {
                                write!(w, "- ")?;
                                match &content.raw {
                                    rmcp::model::RawContent::Text(text_content) => {
                                        writeln!(w, "{}", text_content.text)?;
                                    }
                                    rmcp::model::RawContent::Image(image_content) => {
                                        writeln!(w, "{} image", image_content.mime_type)?;
                                    }
                                    rmcp::model::RawContent::Resource(embedded_resource) => {
                                        match &embedded_resource.resource {
                                        rmcp::model::ResourceContents::TextResourceContents {
                                            uri,
                                            mime_type,
                                            text,
                                            ..
                                        } => {
                                            writeln!(w, "{uri} ({})\n{text}", mime_type.as_deref().unwrap_or("text/plain"))?;
                                        }
                                        rmcp::model::ResourceContents::BlobResourceContents {
                                            uri,
                                            mime_type,
                                            ..
                                        } => {
                                            writeln!(w, "{uri} ({})", mime_type.as_deref().unwrap_or("text/plain"))?;
                                        }
                                    };
                                    }
                                    rmcp::model::RawContent::Audio(audio_content) => {
                                        writeln!(w, "{} audio", audio_content.mime_type)?;
                                    }
                                    rmcp::model::RawContent::ResourceLink(raw_resource) => {
                                        let rmcp::model::RawResource {
                                            uri,
                                            mime_type,
                                            name,
                                            ..
                                        } = raw_resource;
                                        writeln!(
                                            w,
                                            "{name}: {uri} ({})",
                                            mime_type.as_deref().unwrap_or("unknown")
                                        )?;
                                    }
                                }
                            }
                            Ok(())
                        }
                        CallMCPToolResult::Error(error) => {
                            writeln!(w, "Calling MCP tool failed: {error}")
                        }
                        CallMCPToolResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                    }
                }
                AIAgentActionResultType::ReadSkill(result) => match result {
                    ReadSkillResult::Success { content } => {
                        writeln!(w, "Skill read successfully: {}", content.file_name)
                    }
                    ReadSkillResult::Error(error) => writeln!(w, "Skill read error: {error}"),
                    ReadSkillResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::SuggestNewConversation(result) => match result {
                    SuggestNewConversationResult::Accepted { .. }
                    | SuggestNewConversationResult::Rejected => Ok(()),
                    SuggestNewConversationResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::SuggestPrompt(result) => match result {
                    SuggestPromptResult::Accepted { .. } => Ok(()),
                    SuggestPromptResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                AIAgentActionResultType::OpenCodeReview => Ok(()),
                AIAgentActionResultType::InsertReviewComments(_) => Ok(()),
                AIAgentActionResultType::InitProject => Ok(()),
                // Document operations - not yet implemented for SDK
                AIAgentActionResultType::ReadDocuments(_)
                | AIAgentActionResultType::EditDocuments(_)
                | AIAgentActionResultType::CreateDocuments(_) => Ok(()),
                AIAgentActionResultType::ReadShellCommandOutput { .. } => Ok(()),
                AIAgentActionResultType::TransferShellCommandControlToUser { .. } => Ok(()),
                AIAgentActionResultType::UseComputer(result) => match result {
                    // TODO(AGENT-2281): implement
                    UseComputerResult::Success(_result) => Ok(()),
                    UseComputerResult::Error(error) => writeln!(w, "Use computer error: {error}"),
                    UseComputerResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                // TODO(AGENT-2281): implement
                AIAgentActionResultType::RequestComputerUse(_result) => Ok(()),
                AIAgentActionResultType::FetchConversation(result) => match result {
                    FetchConversationResult::Success { directory_path } => {
                        writeln!(w, "Fetched conversation to {directory_path}")
                    }
                    FetchConversationResult::Error(error) => {
                        writeln!(w, "Fetch conversation error: {error}")
                    }
                    FetchConversationResult::Cancelled => writeln!(w, "{CANCELLED_MESSAGE}"),
                },
                // StartAgent is a client-side orchestration action, not used in SDK
                AIAgentActionResultType::StartAgent(_) => Ok(()),
                // SendMessageToAgent is a client-side orchestration action, not used in SDK
                AIAgentActionResultType::SendMessageToAgent(_) => Ok(()),
                AIAgentActionResultType::AskUserQuestion(_) => Ok(()),
                // RunAgents is a desktop-client-only action; not used in the SDK.
                AIAgentActionResultType::RunAgents(_) => Ok(()),
                // No user-visible payload to emit.
                AIAgentActionResultType::WaitForEvents(_) => Ok(()),
            },
        }
    }

    pub fn format_output<W: Write>(output: &AIAgentOutput, w: &mut W) -> io::Result<()> {
        for message in output.messages.iter() {
            match &message.message {
                AIAgentOutputMessageType::Text(text)
                | AIAgentOutputMessageType::Reasoning { text, .. }
                | AIAgentOutputMessageType::Summarization { text, .. } => {
                    super::format_agent_text(text, w)?;
                }
                AIAgentOutputMessageType::Action(action) => match &action.action {
                    AIAgentActionType::RequestCommandOutput { command, .. } => {
                        writeln!(w, "Running `{command}`")?;
                    }
                    AIAgentActionType::WriteToLongRunningShellCommand { input, .. } => {
                        writeln!(w, "Write {} bytes to command", input.len())?;
                    }
                    AIAgentActionType::ReadFiles(request) => {
                        writeln!(
                            w,
                            "Reading {}",
                            request
                                .locations
                                .iter()
                                .format_with(", ", |loc, f| f(&format_args!("{}", loc.name)))
                        )?;
                        // TODO: Better formatting, need shell info.
                    }
                    AIAgentActionType::UploadArtifact(request) => {
                        writeln!(w, "Uploading artifact {}", request.file_path)?;
                    }
                    AIAgentActionType::SearchCodebase(request) => {
                        writeln!(
                            w,
                            "Searching {} for {}",
                            request.codebase_path.as_deref().unwrap_or("codebase"),
                            request.query
                        )?;
                    }
                    AIAgentActionType::RequestFileEdits { file_edits, title } => {
                        write!(w, "Editing files:")?;
                        if let Some(title) = title {
                            write!(w, " {title}")?;
                        }
                        writeln!(w)?;
                        let file_paths: HashSet<_> =
                            file_edits.iter().flat_map(|edit| edit.file()).collect();
                        for path in file_paths {
                            writeln!(w, "- {path}")?;
                        }
                    }
                    AIAgentActionType::Grep { queries, path } => {
                        writeln!(w, "Grepping for {} in {path}", format_queries(queries))?;
                    }
                    AIAgentActionType::FileGlob { patterns, path } => {
                        write!(w, "Finding files matching {}", format_queries(patterns))?;
                        if let Some(path) = path {
                            write!(w, " in {path}")?;
                        }
                        writeln!(w)?;
                    }
                    AIAgentActionType::FileGlobV2 {
                        patterns,
                        search_dir,
                    } => {
                        write!(w, "Finding files matching {}", format_queries(patterns))?;
                        if let Some(path) = search_dir {
                            write!(w, " in {path}")?;
                        }
                        writeln!(w)?;
                    }
                    AIAgentActionType::ReadMCPResource {
                        server_id: _,
                        name,
                        uri,
                    } => match uri {
                        Some(uri) => writeln!(w, "Reading MCP resource {uri}")?,
                        None => writeln!(w, "Reading MCP resource {name}")?,
                    },
                    AIAgentActionType::CallMCPTool {
                        server_id: _,
                        name,
                        input,
                    } => {
                        writeln!(w, "MCP tool call {name}({input:#})")?;
                    }
                    AIAgentActionType::SuggestNewConversation { .. } => (),
                    AIAgentActionType::SuggestPrompt { .. } => (),
                    AIAgentActionType::OpenCodeReview => (),
                    AIAgentActionType::InsertCodeReviewComments { .. } => (),
                    AIAgentActionType::InitProject => (),
                    // Document operations - not yet implemented for SDK
                    AIAgentActionType::ReadDocuments(_)
                    | AIAgentActionType::EditDocuments(_)
                    | AIAgentActionType::CreateDocuments(_)
                    | AIAgentActionType::ReadShellCommandOutput { .. }
                    | AIAgentActionType::TransferShellCommandControlToUser { .. } => (),
                    AIAgentActionType::UseComputer(request) => {
                        writeln!(w, "Computer use action: {}", request.action_summary)?;
                    }
                    AIAgentActionType::RequestComputerUse(request) => {
                        writeln!(w, "Requesting computer use: {}", request.task_summary)?;
                    }
                    AIAgentActionType::ReadSkill(request) => {
                        writeln!(w, "Reading skill: {}", request.skill)?;
                    }
                    AIAgentActionType::FetchConversation { conversation_id } => {
                        writeln!(w, "Fetching conversation {conversation_id}")?;
                    }
                    AIAgentActionType::StartAgent { name, .. } => {
                        writeln!(w, "Starting agent: {name}")?;
                    }
                    AIAgentActionType::SendMessageToAgent {
                        addresses, subject, ..
                    } => {
                        writeln!(
                            w,
                            "Sending message to [{}]: {subject}",
                            addresses.join(", ")
                        )?;
                    }
                    AIAgentActionType::AskUserQuestion { .. } => (),
                    // RunAgents is desktop-client-only; SDK driver renders nothing.
                    AIAgentActionType::RunAgents(_) => (),
                    AIAgentActionType::WaitForEvents { .. } => (),
                },
                AIAgentOutputMessageType::TodoOperation(operation) => match operation {
                    TodoOperation::UpdateTodos { todos } => {
                        writeln!(w, "Updated TODO list:")?;
                        format_todos(todos, w)?;
                    }
                    TodoOperation::MarkAsCompleted { completed_todos } => {
                        writeln!(w, "Completed TODOs:")?;
                        format_todos(completed_todos, w)?;
                    }
                },
                AIAgentOutputMessageType::Subagent(subagent) => {
                    writeln!(w, "{subagent}")?;
                }
                AIAgentOutputMessageType::WebSearch(status) => match status {
                    WebSearchStatus::Searching { query } => match query {
                        Some(q) => writeln!(w, "Searching web for: {q}")?,
                        None => writeln!(w, "Searching web")?,
                    },
                    WebSearchStatus::Success { query, pages } => {
                        writeln!(w, "Searched web for: {query} ({} results)", pages.len())?;
                    }
                    WebSearchStatus::Error { query } => {
                        writeln!(w, "Web search failed for: {query}")?;
                    }
                },
                AIAgentOutputMessageType::WebFetch(status) => match status {
                    WebFetchStatus::Fetching { urls } => {
                        writeln!(w, "Fetching {} web pages...", urls.len())?;
                    }
                    WebFetchStatus::Success { pages } => {
                        writeln!(w, "Fetched {} web pages", pages.len())?;
                    }
                    WebFetchStatus::Error => {
                        writeln!(w, "Web fetch failed")?;
                    }
                },
                AIAgentOutputMessageType::CommentsAddressed {
                    comments: comment_ids,
                } => {
                    writeln!(w, "Addressed {} comments", comment_ids.len())?;
                }
                AIAgentOutputMessageType::DebugOutput { text } => {
                    writeln!(w, "[DEBUG] {text}")?;
                }
                AIAgentOutputMessageType::ArtifactCreated(data) => match data {
                    ArtifactCreatedData::PullRequest { url, branch } => {
                        writeln!(w, "Created PR: {url} (branch: {branch})")?;
                    }
                    ArtifactCreatedData::Screenshot { artifact_uid, .. } => {
                        writeln!(w, "Screenshot captured (artifact: {artifact_uid})")?;
                    }
                    ArtifactCreatedData::File {
                        artifact_uid,
                        filepath,
                        ..
                    } => {
                        writeln!(
                            w,
                            "File artifact uploaded: {filepath} (artifact: {artifact_uid})"
                        )?;
                    }
                },
                AIAgentOutputMessageType::SkillInvoked(invoked_skill) => {
                    writeln!(w, "Skill Read: {}", invoked_skill.name)?;
                }
                AIAgentOutputMessageType::MessagesReceivedFromAgents { messages } => {
                    writeln!(w, "Received {} messages", messages.len())?;
                }
                AIAgentOutputMessageType::EventsFromAgents { event_ids } => {
                    writeln!(w, "Received {} agent events", event_ids.len())?;
                }
            }
        }

        // TODO(REMOTE-22): Format citations.

        Ok(())
    }

    /// Format a list of TODO items.
    fn format_todos<W: Write>(todos: &[AIAgentTodo], w: &mut W) -> io::Result<()> {
        for todo in todos {
            writeln!(w, "* {}", todo.title)?;
        }
        Ok(())
    }

    /// Report that the agent conversation has started. This debug ID can be reported to us for troubleshooting.
    pub fn conversation_started<W: Write>(conversation_id: &str, w: &mut W) -> io::Result<()> {
        writeln!(
            w,
            "New conversation started with debug ID: {conversation_id}\n"
        )
    }

    /// Report the run ID with a link to the Oz dashboard.
    pub fn run_started<W: Write>(run_id: &str, w: &mut W) -> io::Result<()> {
        let run_url = super::run_url(run_id);
        writeln!(w, "Run ID: {run_id}")?;
        writeln!(w, "Open in Oz: {run_url}\n")
    }

    /// Report that a shared session has been established.
    pub fn shared_session_established<W: Write>(join_url: &str, w: &mut W) -> io::Result<()> {
        writeln!(w, "Sharing session at: {join_url}")
    }

    /// Format a list of query patterns.
    fn format_queries<I: IntoIterator<Item = S>, S: fmt::Display>(queries: I) -> String {
        match queries.into_iter().exactly_one() {
            Ok(query) => query.to_string(),
            Err(queries) => format!("[{}]", queries.format(", ")),
        }
    }

    /// Write an artifact_created message for a plan to stdout. We have a separate function for
    /// this since we report creation on plan WD sync.
    pub fn plan_artifact_created<W: Write>(
        document_id: &str,
        notebook_link: &str,
        title: &str,
        w: &mut W,
    ) -> io::Result<()> {
        writeln!(
            w,
            "Created plan (title: {title}, id: {document_id}, notebook: {notebook_link})"
        )
    }
}

pub mod json {
    use std::borrow::Cow;
    use std::io::{self, Write};
    use std::ops::Range;

    use serde::Serialize;

    use crate::ai::agent::comment::ReviewComment;
    use crate::ai::agent::{
        AIAgentActionType, AIAgentInput, AIAgentOutput, AIAgentOutputMessage,
        AIAgentOutputMessageType, AIAgentTodo, ArtifactCreatedData, CallMCPToolResult, FileContext,
        FileGlobResult, FileGlobV2Result, GrepResult, ReadFilesResult, ReadMCPResourceResult,
        RequestCommandOutputResult, RequestFileEditsResult, SearchCodebaseResult, SubagentCall,
        TodoOperation, UploadArtifactResult, WriteToLongRunningShellCommandResult,
    };
    use crate::code::buffer_location::LocalOrRemotePath;
    use crate::AIAgentActionResultType;

    /// JSON representation of messages in an agent conversation. This is intentionally not 1:1 with our internal `AIAgent*` types - it's
    /// a stable interface for callers.
    #[derive(Serialize)]
    #[serde(tag = "type")]
    enum JsonMessage<'a> {
        #[serde(rename = "tool_result")]
        ToolResult(JsonToolResult<'a>),
        #[serde(rename = "tool_canceled")]
        ToolCanceled,
        #[serde(rename = "tool_error")]
        ToolError {
            error: Cow<'a, str>,
        },
        #[serde(rename = "tool_call")]
        ToolCall(JsonToolCall<'a>),
        #[serde(rename = "agent")]
        AgentOutput {
            text: String,
        },
        #[serde(rename = "agent_reasoning")]
        AgentReasoning {
            text: String,
        },
        #[serde(rename = "update_todos")]
        UpdateTodos {
            todo_list: Vec<JsonTodo<'a>>,
        },
        #[serde(rename = "complete_todos")]
        MarkTodosCompleted {
            completed_todos: Vec<JsonTodo<'a>>,
        },
        Subagent {
            task_id: &'a str,
        },
        #[serde(rename = "system")]
        System(JsonSystemEvent<'a>),
        #[serde(rename = "num_comments_addressed")]
        CommentsAddressed {
            addressed_comments: Vec<JsonComment<'a>>,
        },
        #[serde(rename = "artifact_created")]
        ArtifactCreated(JsonArtifact<'a>),
        SkillInvoked {
            name: &'a str,
        },
    }

    #[derive(Serialize)]
    #[serde(tag = "event_type", rename_all = "snake_case")]
    enum JsonSystemEvent<'a> {
        ConversationStarted { conversation_id: &'a str },
        RunStarted { run_id: &'a str, run_url: &'a str },
        SharedSessionEstablished { join_url: &'a str },
    }

    #[derive(Serialize)]
    #[serde(tag = "tool", rename_all = "snake_case")]
    enum JsonToolCall<'a> {
        RunCommand {
            command: &'a str,
        },
        WriteToCommand,
        ReadFiles {
            files: Vec<JsonFile<'a>>,
        },
        UploadArtifact {
            path: &'a str,
            description: Option<&'a str>,
        },
        SearchCodebase {
            query: &'a str,
            codebase: Option<&'a str>,
        },
        EditFiles {
            title: Option<&'a str>,
            file_paths: Vec<&'a str>,
        },
        Grep {
            queries: &'a [String],
            path: &'a str,
        },
        FileGlob {
            patterns: &'a [String],
            path: Option<&'a str>,
        },
        ReadMcpResource {
            name: &'a str,
            uri: Option<&'a str>,
        },
        CallMcpTool {
            name: &'a str,
            input: &'a serde_json::Value,
        },
    }

    #[derive(Serialize)]
    #[serde(tag = "tool", rename_all = "snake_case")]
    enum JsonToolResult<'a> {
        RunCommand(JsonRunCommandResult<'a>),
        EditFiles(JsonEditFilesResult<'a>),
        ReadFiles(JsonFileCollectionResult<'a>),
        UploadArtifact(JsonUploadArtifactResult<'a>),
        SearchCodebase(JsonFileCollectionResult<'a>),
        Grep(JsonFileCollectionResult<'a>),
        FileGlob(JsonFileCollectionResult<'a>),
        ReadMcpResource(JsonReadMcpResourceResult<'a>),
        CallMcpTool(JsonCallMcpToolResult<'a>),
    }

    #[derive(Serialize)]
    #[serde(tag = "status", rename_all = "snake_case")]
    enum JsonRunCommandResult<'a> {
        Complete { exit_code: i32, output: &'a str },
        Running,
    }

    #[derive(Serialize)]
    struct JsonEditFilesResult<'a> {
        diff: &'a str,
    }

    #[derive(Serialize)]
    struct JsonFileCollectionResult<'a> {
        files: Vec<JsonFile<'a>>,
    }

    #[derive(Serialize)]
    struct JsonUploadArtifactResult<'a> {
        artifact_uid: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        filepath: Option<&'a str>,
        mime_type: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<&'a str>,
        size_bytes: i64,
    }

    #[derive(Serialize)]
    struct JsonFile<'a> {
        path: &'a str,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        lines: Vec<Range<usize>>,
    }
    #[derive(Serialize)]
    struct JsonReadMcpResourceResult<'a> {
        resource_contents: &'a [rmcp::model::ResourceContents],
    }

    #[derive(Serialize)]
    struct JsonCallMcpToolResult<'a> {
        result: &'a rmcp::model::CallToolResult,
    }

    #[derive(Serialize)]
    struct JsonTodo<'a> {
        title: &'a str,
        description: &'a str,
    }

    #[derive(Serialize)]
    struct JsonComment<'a> {
        comment_text: &'a str,
        file_path: Option<String>,
        line_number: Option<usize>,
        head_title: Option<&'a str>,
    }

    #[derive(Serialize)]
    #[serde(tag = "artifact_type", rename_all = "snake_case")]
    enum JsonArtifact<'a> {
        PullRequest {
            url: &'a str,
            branch: &'a str,
        },
        Plan {
            document_id: &'a str,
            notebook_link: &'a str,
            title: &'a str,
        },
        Screenshot {
            artifact_uid: &'a str,
            mime_type: &'a str,
            description: Option<&'a str>,
        },
        File {
            artifact_uid: &'a str,
            filepath: &'a str,
            filename: &'a str,
            mime_type: &'a str,
            description: Option<&'a str>,
            size_bytes: i64,
        },
    }

    impl<'a> JsonMessage<'a> {
        fn from_input(input: &'a AIAgentInput) -> Option<Self> {
            match input {
                // Do not include the user query, since it's already provided as input to the agent.
                AIAgentInput::UserQuery { .. }
                | AIAgentInput::AutoCodeDiffQuery { .. }
                | AIAgentInput::CreateNewProject { .. }
                | AIAgentInput::CloneRepository { .. }
                | AIAgentInput::InitProjectRules { .. }
                | AIAgentInput::CodeReview { .. }
                | AIAgentInput::FetchReviewComments { .. }
                | AIAgentInput::CreateEnvironment { .. }
                | AIAgentInput::SummarizeConversation { .. }
                | AIAgentInput::InvokeSkill { .. }
                | AIAgentInput::StartFromAmbientRunPrompt { .. }
                | AIAgentInput::MessagesReceivedFromAgents { .. }
                | AIAgentInput::EventsFromAgents { .. }
                | AIAgentInput::PassiveSuggestionResult { .. }
                | AIAgentInput::OrchestrationConfigUpdate { .. } => None,
                // These input types should not occur in a SDK-run agent.
                AIAgentInput::ResumeConversation { .. }
                | AIAgentInput::TriggerPassiveSuggestion { .. } => None,
                AIAgentInput::ActionResult { result, .. } => {
                    Self::from_action_result(&result.result)
                }
            }
        }

        fn from_action_result(result: &'a AIAgentActionResultType) -> Option<Self> {
            match result {
                AIAgentActionResultType::RequestCommandOutput(result) => match result {
                    RequestCommandOutputResult::Completed {
                        output, exit_code, ..
                    } => Some(JsonMessage::ToolResult(JsonToolResult::RunCommand(
                        JsonRunCommandResult::Complete {
                            exit_code: exit_code.value(),
                            output,
                        },
                    ))),
                    RequestCommandOutputResult::LongRunningCommandSnapshot { .. } => {
                        Some(JsonMessage::ToolResult(JsonToolResult::RunCommand(
                            JsonRunCommandResult::Running,
                        )))
                    }
                    RequestCommandOutputResult::CancelledBeforeExecution => {
                        Some(JsonMessage::ToolCanceled)
                    }
                    RequestCommandOutputResult::Denylisted { .. } => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(
                            "Command was not allowed to run due to presence on denylist",
                        ),
                    }),
                },
                AIAgentActionResultType::WriteToLongRunningShellCommand(result) => match result {
                    WriteToLongRunningShellCommandResult::Snapshot { .. } => {
                        Some(JsonMessage::ToolResult(JsonToolResult::RunCommand(
                            JsonRunCommandResult::Running,
                        )))
                    }
                    WriteToLongRunningShellCommandResult::CommandFinished {
                        output,
                        exit_code,
                        ..
                    } => Some(JsonMessage::ToolResult(JsonToolResult::RunCommand(
                        JsonRunCommandResult::Complete {
                            exit_code: exit_code.value(),
                            output,
                        },
                    ))),
                    WriteToLongRunningShellCommandResult::Error(_) => {
                        Some(JsonMessage::ToolError {
                            error: "Failed to write to command.".into(),
                        })
                    }
                    WriteToLongRunningShellCommandResult::Cancelled => {
                        Some(JsonMessage::ToolCanceled)
                    }
                },
                AIAgentActionResultType::RequestFileEdits(result) => match result {
                    RequestFileEditsResult::Success { diff, .. } => Some(JsonMessage::ToolResult(
                        JsonToolResult::EditFiles(JsonEditFilesResult { diff }),
                    )),
                    RequestFileEditsResult::DiffApplicationFailed { error } => {
                        Some(JsonMessage::ToolError {
                            error: Cow::Borrowed(error.as_str()),
                        })
                    }
                    RequestFileEditsResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::ReadFiles(result) => match result {
                    ReadFilesResult::Success { files } => Some(JsonMessage::ToolResult(
                        JsonToolResult::ReadFiles(JsonFileCollectionResult {
                            files: JsonFile::from_file_contexts(files),
                        }),
                    )),
                    ReadFilesResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    ReadFilesResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::UploadArtifact(result) => match result {
                    UploadArtifactResult::Success {
                        artifact_uid,
                        filepath,
                        mime_type,
                        description,
                        size_bytes,
                    } => Some(JsonMessage::ToolResult(JsonToolResult::UploadArtifact(
                        JsonUploadArtifactResult {
                            artifact_uid,
                            filepath: filepath.as_deref(),
                            mime_type,
                            description: description.as_deref(),
                            size_bytes: *size_bytes,
                        },
                    ))),
                    UploadArtifactResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    UploadArtifactResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::SearchCodebase(result) => match result {
                    SearchCodebaseResult::Success { files } => Some(JsonMessage::ToolResult(
                        JsonToolResult::SearchCodebase(JsonFileCollectionResult {
                            files: JsonFile::from_file_contexts(files),
                        }),
                    )),
                    SearchCodebaseResult::Failed { message, .. } => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(message.as_str()),
                    }),
                    SearchCodebaseResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::Grep(result) => match result {
                    GrepResult::Success { matched_files } => {
                        use crate::ai::agent::GrepFileMatch;
                        let files: Vec<JsonFile> = matched_files
                            .iter()
                            .map(|m: &GrepFileMatch| JsonFile {
                                path: m.file_path.as_str(),
                                lines: m
                                    .matched_lines
                                    .iter()
                                    .map(|lm| lm.line_number..(lm.line_number.saturating_add(1)))
                                    .collect(),
                            })
                            .collect();
                        Some(JsonMessage::ToolResult(JsonToolResult::Grep(
                            JsonFileCollectionResult { files },
                        )))
                    }
                    GrepResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    GrepResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::FileGlobV2(result) => match result {
                    FileGlobV2Result::Success { matched_files, .. } => {
                        let files: Vec<JsonFile> = matched_files
                            .iter()
                            .map(|m| JsonFile {
                                path: m.file_path.as_str(),
                                lines: Vec::new(),
                            })
                            .collect();
                        Some(JsonMessage::ToolResult(JsonToolResult::FileGlob(
                            JsonFileCollectionResult { files },
                        )))
                    }
                    FileGlobV2Result::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    FileGlobV2Result::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::FileGlob(result) => match result {
                    FileGlobResult::Success { matched_files } => {
                        let files: Vec<JsonFile> = matched_files
                            .lines()
                            .filter_map(|line| {
                                let p = line.trim();
                                if p.is_empty() {
                                    None
                                } else {
                                    Some(JsonFile {
                                        path: p,
                                        lines: Vec::new(),
                                    })
                                }
                            })
                            .collect();
                        Some(JsonMessage::ToolResult(JsonToolResult::FileGlob(
                            JsonFileCollectionResult { files },
                        )))
                    }
                    FileGlobResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    FileGlobResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::ReadMCPResource(result) => match result {
                    ReadMCPResourceResult::Success { resource_contents } => {
                        Some(JsonMessage::ToolResult(JsonToolResult::ReadMcpResource(
                            JsonReadMcpResourceResult { resource_contents },
                        )))
                    }
                    ReadMCPResourceResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    ReadMCPResourceResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                AIAgentActionResultType::CallMCPTool(result) => match result {
                    CallMCPToolResult::Success { result } => Some(JsonMessage::ToolResult(
                        JsonToolResult::CallMcpTool(JsonCallMcpToolResult { result }),
                    )),
                    CallMCPToolResult::Error(error) => Some(JsonMessage::ToolError {
                        error: Cow::Borrowed(error.as_str()),
                    }),
                    CallMCPToolResult::Cancelled => Some(JsonMessage::ToolCanceled),
                },
                _ => None,
            }
        }

        fn from_output_message(output: &'a AIAgentOutputMessage) -> Option<Self> {
            match &output.message {
                AIAgentOutputMessageType::Text(text) => {
                    let mut buf = Vec::<u8>::new();
                    super::format_agent_text(text, &mut buf).ok()?;
                    let text = String::from_utf8(buf).ok()?;
                    Some(JsonMessage::AgentOutput { text })
                }
                AIAgentOutputMessageType::Reasoning { text, .. } => {
                    let mut buf = Vec::<u8>::new();
                    super::format_agent_text(text, &mut buf).ok()?;
                    let text = String::from_utf8(buf).ok()?;
                    Some(JsonMessage::AgentReasoning { text })
                }
                AIAgentOutputMessageType::Summarization { text, .. } => {
                    let mut buf = Vec::<u8>::new();
                    super::format_agent_text(text, &mut buf).ok()?;
                    let text = String::from_utf8(buf).ok()?;
                    Some(JsonMessage::AgentReasoning { text })
                }
                AIAgentOutputMessageType::Action(action) => match &action.action {
                    AIAgentActionType::RequestCommandOutput { command, .. } => {
                        Some(JsonMessage::ToolCall(JsonToolCall::RunCommand { command }))
                    }
                    AIAgentActionType::WriteToLongRunningShellCommand { .. } => {
                        Some(JsonMessage::ToolCall(JsonToolCall::WriteToCommand))
                    }
                    AIAgentActionType::ReadFiles(request) => {
                        let files = request
                            .locations
                            .iter()
                            .map(|loc| JsonFile {
                                path: loc.name.as_str(),
                                lines: loc.lines.clone(),
                            })
                            .collect();
                        Some(JsonMessage::ToolCall(JsonToolCall::ReadFiles { files }))
                    }
                    AIAgentActionType::UploadArtifact(request) => {
                        Some(JsonMessage::ToolCall(JsonToolCall::UploadArtifact {
                            path: request.file_path.as_str(),
                            description: request.description.as_deref(),
                        }))
                    }
                    AIAgentActionType::SearchCodebase(request) => {
                        Some(JsonMessage::ToolCall(JsonToolCall::SearchCodebase {
                            query: request.query.as_str(),
                            codebase: request.codebase_path.as_deref(),
                        }))
                    }
                    AIAgentActionType::RequestFileEdits { file_edits, title } => {
                        let file_paths: Vec<&str> =
                            file_edits.iter().filter_map(|edit| edit.file()).collect();
                        Some(JsonMessage::ToolCall(JsonToolCall::EditFiles {
                            title: title.as_deref(),
                            file_paths,
                        }))
                    }
                    AIAgentActionType::Grep { queries, path } => {
                        Some(JsonMessage::ToolCall(JsonToolCall::Grep {
                            queries,
                            path: path.as_str(),
                        }))
                    }
                    AIAgentActionType::FileGlob { patterns, path } => {
                        Some(JsonMessage::ToolCall(JsonToolCall::FileGlob {
                            patterns,
                            path: path.as_deref(),
                        }))
                    }
                    AIAgentActionType::FileGlobV2 {
                        patterns,
                        search_dir,
                    } => Some(JsonMessage::ToolCall(JsonToolCall::FileGlob {
                        patterns,
                        path: search_dir.as_deref(),
                    })),
                    AIAgentActionType::ReadMCPResource {
                        server_id: _,
                        name,
                        uri,
                    } => Some(JsonMessage::ToolCall(JsonToolCall::ReadMcpResource {
                        name,
                        uri: uri.as_deref(),
                    })),
                    AIAgentActionType::CallMCPTool {
                        server_id: _,
                        name,
                        input,
                    } => Some(JsonMessage::ToolCall(JsonToolCall::CallMcpTool {
                        name,
                        input,
                    })),
                    // TODO(AGENT-2281): implement
                    AIAgentActionType::UseComputer(_use_computer_request) => None,
                    // TODO(AGENT-2281): implement
                    AIAgentActionType::RequestComputerUse(_) => None,
                    // Internal or non-CLI tool calls: skip them
                    AIAgentActionType::SuggestNewConversation { .. }
                    | AIAgentActionType::SuggestPrompt { .. }
                    | AIAgentActionType::InitProject
                    | AIAgentActionType::OpenCodeReview
                    | AIAgentActionType::InsertCodeReviewComments { .. }
                    | AIAgentActionType::ReadDocuments(_)
                    | AIAgentActionType::EditDocuments(_)
                    | AIAgentActionType::CreateDocuments(_)
                    | AIAgentActionType::ReadShellCommandOutput { .. }
                    | AIAgentActionType::ReadSkill(_)
                    | AIAgentActionType::FetchConversation { .. }
                    | AIAgentActionType::StartAgent { .. }
                    | AIAgentActionType::SendMessageToAgent { .. }
                    | AIAgentActionType::TransferShellCommandControlToUser { .. } => None,
                    AIAgentActionType::AskUserQuestion { .. } => None,
                    // RunAgents is desktop-client-only; SDK has no JSON
                    // representation for it.
                    AIAgentActionType::RunAgents(_) => None,
                    AIAgentActionType::WaitForEvents { .. } => None,
                },
                AIAgentOutputMessageType::TodoOperation(operation) => match operation {
                    TodoOperation::UpdateTodos { todos } => Some(JsonMessage::UpdateTodos {
                        todo_list: JsonTodo::from_todos(todos),
                    }),
                    TodoOperation::MarkAsCompleted { completed_todos } => {
                        Some(JsonMessage::MarkTodosCompleted {
                            completed_todos: JsonTodo::from_todos(completed_todos),
                        })
                    }
                },
                AIAgentOutputMessageType::Subagent(SubagentCall { task_id, .. }) => {
                    Some(JsonMessage::Subagent { task_id })
                }
                AIAgentOutputMessageType::WebSearch(_) => None,
                AIAgentOutputMessageType::WebFetch(_) => None,
                AIAgentOutputMessageType::DebugOutput { .. } => None,
                AIAgentOutputMessageType::CommentsAddressed { comments } => {
                    Some(JsonMessage::CommentsAddressed {
                        addressed_comments: JsonComment::from_review_comments(comments),
                    })
                }
                AIAgentOutputMessageType::ArtifactCreated(data) => {
                    Some(JsonMessage::ArtifactCreated(JsonArtifact::from(data)))
                }
                AIAgentOutputMessageType::SkillInvoked(invoked_skill) => {
                    Some(JsonMessage::SkillInvoked {
                        name: &invoked_skill.name,
                    })
                }
                AIAgentOutputMessageType::MessagesReceivedFromAgents { .. }
                | AIAgentOutputMessageType::EventsFromAgents { .. } => None,
            }
        }
    }

    impl<'a> JsonFile<'a> {
        fn from_file_contexts(contexts: &'a [FileContext]) -> Vec<Self> {
            contexts.iter().map(Self::from).collect()
        }
    }

    impl<'a> From<&'a FileContext> for JsonFile<'a> {
        fn from(context: &'a FileContext) -> Self {
            Self {
                path: context.file_name.as_str(),
                lines: context.line_range.clone().into_iter().collect(),
            }
        }
    }

    impl<'a> JsonComment<'a> {
        fn from_review_comments(comments: &'a [ReviewComment]) -> Vec<Self> {
            comments.iter().map(Self::from).collect()
        }
    }

    impl<'a> From<&'a ReviewComment> for JsonComment<'a> {
        fn from(review_comment: &'a ReviewComment) -> Self {
            Self {
                comment_text: review_comment.content.as_str(),
                file_path: review_comment
                    .diff
                    .file_path
                    .as_ref()
                    .map(LocalOrRemotePath::display_path),
                line_number: review_comment.diff.line_number,
                head_title: review_comment.head_title.as_deref(),
            }
        }
    }

    impl<'a> JsonTodo<'a> {
        fn from_todos(todos: &'a [AIAgentTodo]) -> Vec<Self> {
            todos.iter().map(Self::from).collect()
        }
    }

    impl<'a> From<&'a AIAgentTodo> for JsonTodo<'a> {
        fn from(todo: &'a AIAgentTodo) -> Self {
            Self {
                title: todo.title.as_str(),
                description: todo.description.as_str(),
            }
        }
    }

    impl<'a> From<&'a ArtifactCreatedData> for JsonArtifact<'a> {
        fn from(data: &'a ArtifactCreatedData) -> Self {
            match data {
                ArtifactCreatedData::PullRequest { url, branch } => JsonArtifact::PullRequest {
                    url: url.as_str(),
                    branch: branch.as_str(),
                },
                ArtifactCreatedData::Screenshot {
                    artifact_uid,
                    mime_type,
                    description,
                } => JsonArtifact::Screenshot {
                    artifact_uid: artifact_uid.as_str(),
                    mime_type: mime_type.as_str(),
                    description: description.as_deref(),
                },
                ArtifactCreatedData::File {
                    artifact_uid,
                    filepath,
                    filename,
                    mime_type,
                    description,
                    size_bytes,
                } => JsonArtifact::File {
                    artifact_uid: artifact_uid.as_str(),
                    filepath: filepath.as_str(),
                    filename: filename.as_str(),
                    mime_type: mime_type.as_str(),
                    description: description.as_deref(),
                    size_bytes: *size_bytes,
                },
            }
        }
    }

    /// Write an artifact_created message for a plan to stdout.
    pub fn plan_artifact_created<W: Write>(
        document_id: &str,
        notebook_link: &str,
        title: &str,
        w: &mut W,
    ) -> io::Result<()> {
        let message = JsonMessage::ArtifactCreated(JsonArtifact::Plan {
            document_id,
            notebook_link,
            title,
        });
        write_message(&message, w)
    }

    fn write_message<W: Write>(message: &JsonMessage, w: &mut W) -> io::Result<()> {
        serde_json::to_writer(&mut *w, message).map_err(|e| io::Error::other(e.to_string()))?;
        writeln!(w)?;
        Ok(())
    }

    pub fn format_output<W: Write>(output: &AIAgentOutput, w: &mut W) -> io::Result<()> {
        for message in output.messages.iter() {
            if let Some(message) = JsonMessage::from_output_message(message) {
                write_message(&message, w)?;
            }
        }
        Ok(())
    }

    pub fn format_input<W: Write>(input: &AIAgentInput, w: &mut W) -> io::Result<()> {
        match JsonMessage::from_input(input) {
            Some(message) => write_message(&message, w),
            None => Ok(()),
        }
    }

    /// Write a conversation_started system event to stdout.
    pub fn conversation_started<W: Write>(conversation_id: &str, w: &mut W) -> io::Result<()> {
        let message = JsonMessage::System(JsonSystemEvent::ConversationStarted { conversation_id });
        write_message(&message, w)
    }

    /// Write a run_started system event to stdout.
    pub fn run_started<W: Write>(run_id: &str, w: &mut W) -> io::Result<()> {
        let run_url = super::run_url(run_id);
        let message = JsonMessage::System(JsonSystemEvent::RunStarted {
            run_id,
            run_url: &run_url,
        });
        write_message(&message, w)
    }

    /// Write a shared_session_established system event to stdout.
    pub fn shared_session_established<W: Write>(join_url: &str, w: &mut W) -> io::Result<()> {
        let message = JsonMessage::System(JsonSystemEvent::SharedSessionEstablished { join_url });
        write_message(&message, w)
    }
}

use std::io::{self, BufWriter, Write};

use warp_core::channel::ChannelState;

use crate::ai::agent::{AIAgentText, AIAgentTextSection};
use crate::code::editor_management::CodeSource;

/// Constructs the Oz dashboard URL for a given run ID.
fn run_url(run_id: &str) -> String {
    let oz_root_url = ChannelState::oz_root_url();
    format!("{oz_root_url}/runs/{run_id}")
}

/// Execute a closure with a buffered stdout writer and flush it afterwards.
pub fn with_stdout_buffered<F>(f: F) -> io::Result<()>
where
    F: FnOnce(&mut BufWriter<io::StdoutLock>) -> io::Result<()>,
{
    let stdout = io::stdout();
    let handle = stdout.lock();
    let mut buf = BufWriter::new(handle);
    f(&mut buf)?;
    buf.flush()
}

fn format_agent_text<W: Write>(text: &AIAgentText, w: &mut W) -> io::Result<()> {
    let mut wrote_newline = false;
    for section in &text.sections {
        match section {
            AIAgentTextSection::PlainText { text } => {
                write!(w, "{}", text.text())?;
                wrote_newline = text.text().ends_with('\n');
            }
            AIAgentTextSection::Code {
                code,
                language,
                source,
            } => {
                write!(w, "```")?;
                if let Some(language) = language {
                    write!(w, "{}", language.display_name())?;
                }

                match source {
                    Some(CodeSource::ProjectRules { location }) => {
                        writeln!(w, " rules_path={}", location.display_path())?;
                    }
                    Some(CodeSource::Link {
                        path,
                        range_start,
                        range_end,
                    }) => {
                        write!(w, " path={}", path.display())?;

                        if let Some(start) = range_start {
                            write!(w, " start={}", start.line_num)?;
                        }

                        if let Some(end) = range_end {
                            write!(w, " end={}", end.line_num)?;
                        }

                        writeln!(w)?;
                    }
                    Some(CodeSource::Skill { location, .. }) => {
                        writeln!(w, " skill_path={}", location.display_path())?;
                    }
                    Some(CodeSource::AIAction { .. })
                    | Some(CodeSource::New { .. })
                    | Some(CodeSource::FileTree { .. })
                    | Some(CodeSource::CommandPalette { .. })
                    | Some(CodeSource::Finder { .. })
                    | None => {}
                }

                writeln!(w, "{code}\n```",)?;
                wrote_newline = true;
            }
            AIAgentTextSection::Table { table } => {
                write!(w, "{}", table.markdown_source)?;
                wrote_newline = table.markdown_source.ends_with('\n');
            }
            AIAgentTextSection::Image { image } => {
                write!(w, "{}", image.markdown_source)?;
                wrote_newline = image.markdown_source.ends_with('\n');
            }
            AIAgentTextSection::MermaidDiagram { diagram } => {
                write!(w, "{}", diagram.markdown_source)?;
                wrote_newline = diagram.markdown_source.ends_with('\n');
            }
        }
    }
    if !wrote_newline {
        writeln!(w)?;
    }

    Ok(())
}
