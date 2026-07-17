//! Per-tool, per-state one-line labels for tool-call rows in the TUI
//! transcript, modeled on the GUI's inline action text.

use std::path::Path;

use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionResultType, AIAgentActionType,
    AskUserQuestionResult, FileGlobV2Result, GrepResult, RequestCommandOutputResult,
    RunAgentsAgentOutcomeKind, RunAgentsResult, SearchCodebaseFailureReason, SearchCodebaseResult,
    StartAgentExecutionMode, SuggestNewConversationResult,
};
use warp_core::command::ExitCode;
use warpui_core::elements::tui::TuiStyle;

use self::ToolCallDisplayState as State;
use crate::tui_builder::TuiUiBuilder;

/// Ground-truth state of the terminal block backing a shell-command tool
/// call, resolved by the caller. When a block exists, its state supersedes
/// the stored action status/result for execution states (mirroring the GUI's
/// `RequestedCommandView`, which derives icon and expandability from the
/// block whenever one exists). Notably, an agent-monitored command's stored
/// result stays a `LongRunningCommandSnapshot` forever, so without the block
/// its row could never leave the "still running" state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandBlockState {
    Running,
    Finished { exit_code: ExitCode },
}

/// A shell-command tool call's terminal block as resolved by the caller: its
/// execution state plus the command it actually ran. The block's command
/// supersedes the streamed one, which the user may have edited before
/// accepting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedCommandBlock {
    /// The block's command, when it has one; `None` while the block's
    /// command grid is still empty.
    pub(crate) command: Option<String>,
    pub(crate) state: CommandBlockState,
}

/// Longest rendered length for interpolated values (commands, queries, paths)
/// so tool-call rows stay scannable one-liners.
const MAX_INLINE_LEN: usize = 80;

/// Coarse presentation state for a tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolCallDisplayState {
    /// The tool call's arguments are still streaming and may be incomplete.
    Constructing,
    /// The tool call is waiting to begin execution.
    Pending,
    /// The tool call is blocked on user confirmation.
    Blocked,
    /// The tool call is executing asynchronously.
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl ToolCallDisplayState {
    /// The compact leading glyph for this state.
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Constructing | Self::Pending => "○",
            Self::Blocked | Self::Cancelled => "■",
            Self::Running => "●",
            Self::Succeeded => "✓",
            Self::Failed => "×",
        }
    }

    /// The semantic theme style for this state's glyph.
    pub(crate) fn glyph_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running => builder.attention_glyph_style(),
            Self::Succeeded => builder.success_glyph_style(),
            Self::Failed => builder.error_text_style(),
            Self::Cancelled => builder.muted_text_style(),
        }
    }

    /// The semantic text style paired with this state.
    pub(crate) fn label_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running | Self::Succeeded | Self::Failed | Self::Cancelled => {
                builder.primary_text_style()
            }
        }
    }
}

/// Collapses an optional action status into the coarse display state.
/// `output_streaming` is whether the exchange output is still streaming;
/// a status-less action in a streaming output is still being constructed
/// (mirroring the GUI's `status.is_none() && is_streaming()` gating).
/// A resolved `block_state` supersedes the status for execution states
/// (see [`CommandBlockState`]).
pub(crate) fn tool_call_display_state(
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block_state: Option<CommandBlockState>,
) -> ToolCallDisplayState {
    // A block existing means the command actually started executing, so its
    // state is authoritative over the action status/result.
    match block_state {
        Some(CommandBlockState::Running) => return State::Running,
        Some(CommandBlockState::Finished { exit_code }) => {
            return if exit_code.is_sigint() {
                State::Cancelled
            } else if exit_code.was_successful() {
                State::Succeeded
            } else {
                State::Failed
            };
        }
        None => {}
    }
    match status {
        None if output_streaming => State::Constructing,
        None | Some(AIActionStatus::Preprocessing | AIActionStatus::Queued) => State::Pending,
        Some(AIActionStatus::Blocked) => State::Blocked,
        Some(AIActionStatus::RunningAsync) => State::Running,
        Some(finished @ AIActionStatus::Finished(_)) => {
            if finished.is_cancelled() {
                State::Cancelled
            } else if finished.is_failed() {
                State::Failed
            } else {
                State::Succeeded
            }
        }
    }
}

/// Returns the one-line transcript label for a tool call in its current state.
pub(crate) fn tool_call_label(
    action: &AIAgentAction,
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block: Option<&ResolvedCommandBlock>,
) -> String {
    let state = tool_call_display_state(status, output_streaming, block.map(|block| block.state));
    let result = status
        .and_then(AIActionStatus::finished_result)
        .map(|result| &result.result);
    let label = label_for_action(&action.action, state, result, block);
    match state {
        State::Blocked => format!("{label} (awaiting approval)"),
        State::Constructing
        | State::Pending
        | State::Running
        | State::Succeeded
        | State::Failed
        | State::Cancelled => label,
    }
}

/// Builds the per-tool label body; the awaiting-approval suffix is applied by
/// [`tool_call_label`]. `result` is the finished result, when there is one.
///
/// `Constructing` arms never interpolate argument fields (they may be empty
/// or partial while streaming); their copy is indexed on the GUI's loading
/// messages (`common.rs` `LOAD_OUTPUT_MESSAGE_*` and the requested-command
/// view's "Generating command...").
fn label_for_action(
    action: &AIAgentActionType,
    state: State,
    result: Option<&AIAgentActionResultType>,
    block: Option<&ResolvedCommandBlock>,
) -> String {
    let block_state = block.map(|block| block.state);
    match action {
        AIAgentActionType::RequestCommandOutput { command, .. } => {
            // The streamed command can be edited before acceptance, so
            // prefer the executed command from the finished result or the
            // resolved block over the original suggestion.
            let executed = result
                .and_then(AIAgentActionResultType::command_str)
                .or_else(|| block.and_then(|block| block.command.as_deref()));
            let cmd = single_line(executed.unwrap_or(command));
            match state {
                State::Constructing => "Generating command…".to_owned(),
                State::Pending | State::Blocked => format!("Run `{cmd}`"),
                State::Running => format!("Running `{cmd}`"),
                State::Succeeded => match block_state {
                    Some(CommandBlockState::Finished { .. }) => format!("Ran `{cmd}`"),
                    // No local block: fall back to the stored result. A
                    // snapshot result means the command was still running at
                    // the last point we could observe it.
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::LongRunningCommandSnapshot { .. },
                        )) => format!("`{cmd}` is still running"),
                        _ => format!("Ran `{cmd}`"),
                    },
                },
                State::Failed => match block_state {
                    Some(CommandBlockState::Finished { exit_code }) => {
                        format!("`{cmd}` exited with code {}", exit_code.value())
                    }
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Completed { exit_code, .. },
                        )) => format!("`{cmd}` exited with code {}", exit_code.value()),
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Denylisted { .. },
                        )) => format!("`{cmd}` denied (denylisted)"),
                        _ => format!("`{cmd}` failed"),
                    },
                },
                State::Cancelled => format!("Cancelled `{cmd}`"),
            }
        }
        AIAgentActionType::WriteToLongRunningShellCommand { .. } => match state {
            State::Constructing => "Writing command input…".to_owned(),
            State::Pending | State::Blocked => "Write input to running command".to_owned(),
            State::Running => "Writing input to running command…".to_owned(),
            State::Succeeded => "Wrote input to running command".to_owned(),
            State::Failed => "Failed to write to running command".to_owned(),
            State::Cancelled => "Write to running command cancelled".to_owned(),
        },
        AIAgentActionType::ReadFiles(request) => {
            let files = files_summary(request.locations.iter().map(|location| &location.name));
            match state {
                State::Constructing => "Reading files…".to_owned(),
                State::Pending | State::Blocked | State::Succeeded => {
                    format!("Read {files}")
                }
                State::Running => format!("Reading {files}"),
                State::Failed => format!("Failed to read {files}"),
                State::Cancelled => format!("Cancelled reading {files}"),
            }
        }
        AIAgentActionType::UploadArtifact(request) => {
            let file = single_line(&request.file_path);
            match state {
                State::Constructing => "Preparing upload…".to_owned(),
                State::Pending | State::Blocked => format!("Upload {file}"),
                State::Running => format!("Uploading {file}"),
                State::Succeeded => format!("Uploaded {file}"),
                State::Failed => format!("Upload of {file} failed"),
                State::Cancelled => format!("Upload of {file} cancelled"),
            }
        }
        AIAgentActionType::SearchCodebase(request) => {
            let query = single_line(&request.query);
            let scope = request
                .codebase_path
                .as_deref()
                .map(|path| format!(" in {}", base_name(path)))
                .unwrap_or_default();
            match state {
                State::Constructing => "Searching codebase…".to_owned(),
                State::Pending | State::Blocked => {
                    format!("Search for \"{query}\"{scope}")
                }
                State::Running => format!("Searching for \"{query}\"{scope}"),
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Success { files },
                    )) if files.is_empty() => {
                        format!("Searched for \"{query}\"{scope}, no results")
                    }
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Success { files },
                    )) => format!(
                        "Searched for \"{query}\"{scope}, {}",
                        count_label(files.len(), "result", "results")
                    ),
                    _ => format!("Searched for \"{query}\"{scope}"),
                },
                State::Failed => match result {
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Failed {
                            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                            ..
                        },
                    )) => format!(
                        "Search for \"{query}\"{scope} failed because the codebase isn't indexed"
                    ),
                    _ => format!("Search for \"{query}\"{scope} failed"),
                },
                State::Cancelled => format!("Search for \"{query}\"{scope} cancelled"),
            }
        }
        // Rendered by its own stateful child view (`TuiFileEditsView`); the
        // label path should never be reached for it.
        AIAgentActionType::RequestFileEdits { .. } => {
            log::warn!("tool_call_label called for RequestFileEdits, which has custom rendering");
            String::new()
        }
        AIAgentActionType::Grep { queries, path } => {
            let queries = single_line(&queries.join(", "));
            let path = display_path(path);
            match state {
                State::Constructing => "Grepping…".to_owned(),
                State::Pending | State::Blocked => {
                    format!("Grep for {queries} in {path}")
                }
                State::Running => format!("Grepping for {queries} in {path}"),
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::Grep(GrepResult::Success { matched_files })) => {
                        format!(
                            "Grepped for {queries} in {path}, {}",
                            count_label(matched_files.len(), "matching file", "matching files")
                        )
                    }
                    _ => format!("Grepped for {queries} in {path}"),
                },
                State::Failed => format!("Grep for {queries} failed"),
                State::Cancelled => format!("Grep for {queries} cancelled"),
            }
        }
        AIAgentActionType::FileGlob { patterns, path } => {
            file_glob_label(patterns, path.as_deref(), state, None)
        }
        AIAgentActionType::FileGlobV2 {
            patterns,
            search_dir,
        } => {
            let matched_count = match result {
                Some(AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Success {
                    matched_files,
                    ..
                })) => Some(matched_files.len()),
                _ => None,
            };
            file_glob_label(patterns, search_dir.as_deref(), state, matched_count)
        }
        AIAgentActionType::ReadMCPResource { name, uri, .. } => {
            let resource = single_line(uri.as_deref().unwrap_or(name));
            match state {
                // The resource name arrives with the tool-call header (not
                // the streamed args), so include it when present, like the
                // GUI's "Reading \"{name}\" MCP resource..." loading text.
                State::Constructing if name.is_empty() => "Reading MCP resource…".to_owned(),
                State::Constructing => format!("Reading \"{name}\" MCP resource…"),
                State::Pending | State::Blocked | State::Succeeded => {
                    format!("Read MCP resource {resource}")
                }
                State::Running => format!("Reading MCP resource {resource}"),
                State::Failed => format!("MCP resource {resource} failed"),
                State::Cancelled => format!("MCP resource {resource} cancelled"),
            }
        }
        AIAgentActionType::CallMCPTool { name, .. } => {
            let name = single_line(name);
            match state {
                // Like the GUI's "Calling \"{name}\" MCP tool..." loading
                // text; the tool name is available before its args finish.
                State::Constructing if name.is_empty() => "Calling MCP tool…".to_owned(),
                State::Constructing => format!("Calling \"{name}\" MCP tool…"),
                State::Pending | State::Blocked => format!("Call MCP tool {name}"),
                State::Running => format!("Calling MCP tool {name}"),
                State::Succeeded => format!("Called MCP tool {name}"),
                State::Failed => format!("MCP tool {name} failed"),
                State::Cancelled => format!("MCP tool {name} cancelled"),
            }
        }
        AIAgentActionType::SuggestNewConversation { .. } => match state {
            State::Constructing => "Suggesting a new conversation…".to_owned(),
            State::Pending | State::Blocked | State::Running | State::Failed => {
                "Suggested starting a new conversation".to_owned()
            }
            State::Succeeded => match result {
                Some(AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Rejected,
                )) => "Continuing current conversation".to_owned(),
                _ => "New conversation started".to_owned(),
            },
            State::Cancelled => "New conversation suggestion cancelled".to_owned(),
        },
        AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview => fallback_label(action, state),
        AIAgentActionType::ReadDocuments(request) => {
            let documents = count_label(request.document_ids.len(), "document", "documents");
            match state {
                State::Constructing => "Reading documents…".to_owned(),
                State::Pending | State::Blocked | State::Succeeded => {
                    format!("Read {documents}")
                }
                State::Running => format!("Reading {documents}"),
                State::Failed => "Failed to read documents".to_owned(),
                State::Cancelled => "Cancelled reading documents".to_owned(),
            }
        }
        AIAgentActionType::EditDocuments(request) => match state {
            State::Pending | State::Blocked => "Update plan".to_owned(),
            State::Constructing | State::Running => "Updating plan…".to_owned(),
            State::Succeeded => format!(
                "Updated plan ({})",
                count_label(request.diffs.len(), "edit", "edits")
            ),
            State::Failed => "Failed to update plan".to_owned(),
            State::Cancelled => "Update plan cancelled".to_owned(),
        },
        AIAgentActionType::CreateDocuments(request) => match state {
            State::Pending | State::Blocked => "Create plan".to_owned(),
            State::Constructing | State::Running => "Generating plan…".to_owned(),
            State::Succeeded => {
                let count = request.documents.len();
                if count > 1 {
                    format!("Created {count} documents")
                } else {
                    "Created plan".to_owned()
                }
            }
            State::Failed => "Failed to create plan".to_owned(),
            State::Cancelled => "Create plan cancelled".to_owned(),
        },
        AIAgentActionType::ReadShellCommandOutput { .. } => match state {
            State::Pending | State::Blocked | State::Succeeded => "Read command output".to_owned(),
            State::Constructing | State::Running => "Reading command output…".to_owned(),
            State::Failed => "Failed to read command output".to_owned(),
            State::Cancelled => "Read command output cancelled".to_owned(),
        },
        AIAgentActionType::UseComputer(request) => summary_label(&request.action_summary, state),
        AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
            let comments = count_label(comments.len(), "review comment", "review comments");
            match state {
                State::Constructing => "Preparing review comments…".to_owned(),
                State::Pending | State::Blocked => format!("Insert {comments}"),
                State::Running => format!("Inserting {comments}…"),
                State::Succeeded => format!("Inserted {comments}"),
                State::Failed => "Failed to insert review comments".to_owned(),
                State::Cancelled => "Insert review comments cancelled".to_owned(),
            }
        }
        AIAgentActionType::RequestComputerUse(request) => {
            summary_label(&request.task_summary, state)
        }
        AIAgentActionType::StartRecording { .. } => match state {
            State::Pending | State::Blocked => "Start recording".to_owned(),
            State::Constructing | State::Running => "Starting recording…".to_owned(),
            State::Succeeded => "Started screen recording".to_owned(),
            State::Failed => "Recording failed to start".to_owned(),
            State::Cancelled => "Start recording cancelled".to_owned(),
        },
        AIAgentActionType::StopRecording { .. } => match state {
            State::Pending | State::Blocked => "Stop recording".to_owned(),
            State::Constructing | State::Running => "Stopping recording…".to_owned(),
            State::Succeeded => "Saved screen recording".to_owned(),
            State::Failed => "Failed to save recording".to_owned(),
            State::Cancelled => "Stop recording cancelled".to_owned(),
        },
        AIAgentActionType::ReadSkill(request) => {
            let skill = single_line(&request.skill.display_label());
            match state {
                State::Constructing => "Reading skill…".to_owned(),
                State::Pending | State::Blocked | State::Succeeded => {
                    format!("Read skill {skill}")
                }
                State::Running => format!("Reading skill {skill}"),
                State::Failed => format!("Failed to read skill {skill}"),
                State::Cancelled => format!("Cancelled reading skill {skill}"),
            }
        }
        AIAgentActionType::FetchConversation { .. } => match state {
            State::Pending | State::Blocked => "Fetch conversation".to_owned(),
            State::Constructing | State::Running => "Fetching conversation…".to_owned(),
            State::Succeeded => "Fetched conversation".to_owned(),
            State::Failed => "Fetch conversation failed".to_owned(),
            State::Cancelled => "Fetch conversation cancelled".to_owned(),
        },
        AIAgentActionType::StartAgent {
            name,
            execution_mode,
            ..
        } => {
            let agent = if matches!(execution_mode, StartAgentExecutionMode::Remote { .. }) {
                format!("remote agent {name}")
            } else {
                format!("agent {name}")
            };
            match state {
                State::Constructing => "Configuring agent…".to_owned(),
                State::Pending | State::Blocked => format!("Start {agent}"),
                State::Running => format!("Starting {agent}…"),
                State::Succeeded => format!("Started agent {name}"),
                State::Failed => format!("Failed to start agent {name}"),
                State::Cancelled => format!("Start agent {name} cancelled"),
            }
        }
        AIAgentActionType::SendMessageToAgent {
            addresses, subject, ..
        } => {
            let subject = single_line(subject);
            match state {
                State::Constructing => "Composing message…".to_owned(),
                State::Pending | State::Blocked => format!("Send message: {subject}"),
                State::Running => format!(
                    "Sending message to {}: {subject}",
                    count_label(addresses.len(), "agent", "agents")
                ),
                State::Succeeded => format!("Sent message: {subject}"),
                State::Failed => format!("Failed to send message: {subject}"),
                State::Cancelled => "Send message cancelled".to_owned(),
            }
        }
        AIAgentActionType::TransferShellCommandControlToUser { reason } => match state {
            State::Constructing => "Handing control to you…".to_owned(),
            State::Pending | State::Blocked | State::Running => {
                format!("Handing control to you: {}", single_line(reason))
            }
            State::Succeeded => "You are in control".to_owned(),
            State::Failed => "Control transfer failed".to_owned(),
            State::Cancelled => "Control transfer cancelled".to_owned(),
        },
        AIAgentActionType::AskUserQuestion { questions } => match state {
            State::Constructing => "Preparing question…".to_owned(),
            State::Pending | State::Blocked | State::Running => format!(
                "Asking {}",
                count_label(questions.len(), "question", "questions")
            ),
            State::Succeeded => match result {
                Some(AIAgentActionResultType::AskUserQuestion(
                    AskUserQuestionResult::Success { answers },
                )) => {
                    let total = answers.len();
                    let answered = answers.iter().filter(|answer| !answer.is_skipped()).count();
                    if answered == 0 {
                        "Questions skipped".to_owned()
                    } else if answered == total && total == 1 {
                        "Answered question".to_owned()
                    } else if answered == total {
                        format!("Answered all {total} questions")
                    } else {
                        format!("Answered {answered} of {total} questions")
                    }
                }
                Some(AIAgentActionResultType::AskUserQuestion(
                    AskUserQuestionResult::SkippedByAutoApprove { .. },
                )) => "Questions skipped".to_owned(),
                _ => "Answered questions".to_owned(),
            },
            State::Failed => "Questions failed".to_owned(),
            State::Cancelled => "Questions cancelled".to_owned(),
        },
        AIAgentActionType::RunAgents(request) => {
            let total = request.agent_run_configs.len();
            match state {
                State::Constructing | State::Pending | State::Blocked => {
                    "Configuring agents…".to_owned()
                }
                State::Running => {
                    format!("Spawning {}…", count_label(total, "agent", "agents"))
                }
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                        agents,
                        ..
                    })) => {
                        let launched = agents
                            .iter()
                            .filter(|agent| {
                                matches!(agent.kind, RunAgentsAgentOutcomeKind::Launched { .. })
                            })
                            .count();
                        let total = agents.len();
                        if launched == total {
                            format!("Spawned {}", count_label(total, "agent", "agents"))
                        } else if launched == 0 {
                            format!("Failed to spawn {}", count_label(total, "agent", "agents"))
                        } else {
                            format!("Spawned {launched} of {total} agents")
                        }
                    }
                    _ => format!("Spawned {}", count_label(total, "agent", "agents")),
                },
                State::Failed => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
                        ..
                    })) => "Orchestration disabled — agents not launched".to_owned(),
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Failure {
                        error,
                    })) if !error.is_empty() => {
                        format!("Failed to start orchestration: {}", single_line(error))
                    }
                    _ => "Failed to start orchestration".to_owned(),
                },
                State::Cancelled => "Spawn agents cancelled".to_owned(),
            }
        }
        AIAgentActionType::WaitForEvents { .. } => match state {
            State::Constructing | State::Pending | State::Blocked | State::Running => {
                "Waiting for agent events…".to_owned()
            }
            State::Succeeded => "Done waiting for agent events".to_owned(),
            State::Failed => "Waiting for agent events failed".to_owned(),
            State::Cancelled => "Wait for events cancelled".to_owned(),
        },
    }
}

/// Shared label body for both file-glob action versions; only V2 results
/// carry a match count.
fn file_glob_label(
    patterns: &[String],
    path: Option<&str>,
    state: State,
    matched_count: Option<usize>,
) -> String {
    let patterns = single_line(&patterns.join(", "));
    let path = display_path(path.unwrap_or("."));
    match state {
        State::Constructing => "Finding files…".to_owned(),
        State::Pending | State::Blocked => {
            format!("Find files matching {patterns} in {path}")
        }
        State::Running => format!("Finding files matching {patterns} in {path}"),
        State::Succeeded => match matched_count {
            Some(count) => format!(
                "Found {} matching {patterns}",
                count_label(count, "file", "files")
            ),
            None => format!("Found files matching {patterns}"),
        },
        State::Failed => format!("File search for {patterns} failed"),
        State::Cancelled => format!("File search for {patterns} cancelled"),
    }
}

/// Labels computer-use calls with their agent-supplied summary, marking only
/// terminal non-success states (matching the GUI, which shows the summary
/// verbatim).
fn summary_label(summary: &str, state: State) -> String {
    let summary = single_line(summary);
    match state {
        State::Constructing => "Preparing computer use…".to_owned(),
        State::Pending | State::Blocked | State::Running | State::Succeeded => summary,
        State::Failed => format!("{summary} — failed"),
        State::Cancelled => format!("{summary} — cancelled"),
    }
}

/// Generic label for action types without bespoke text, derived from the
/// action's user-friendly name.
fn fallback_label(action: &AIAgentActionType, state: State) -> String {
    let name = action.user_friendly_name();
    match state {
        State::Pending | State::Blocked => name,
        State::Constructing | State::Running => format!("{name}…"),
        State::Succeeded => format!("{name} — done"),
        State::Failed => format!("{name} — failed"),
        State::Cancelled => format!("{name} — cancelled"),
    }
}

/// Collapses text to its first line, capped at [`MAX_INLINE_LEN`] chars, with
/// a trailing `…` when anything was trimmed.
fn single_line(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim_end();
    let mut out: String = first_line.chars().take(MAX_INLINE_LEN).collect();
    if first_line.chars().count() > MAX_INLINE_LEN || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

/// Renders a search path for display, mirroring the GUI's treatment of `.`.
fn display_path(path: &str) -> String {
    if path == "." {
        "the current directory".to_owned()
    } else {
        single_line(path)
    }
}

/// Returns the final path component, falling back to the input when there is none.
fn base_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Summarizes file paths as comma-joined base names for up to 3 files, else a count.
fn files_summary<'a>(paths: impl ExactSizeIterator<Item = &'a String>) -> String {
    if paths.len() > 3 {
        return count_label(paths.len(), "file", "files");
    }
    let names: Vec<String> = paths.map(|path| base_name(path)).collect();
    if names.is_empty() {
        "files".to_owned()
    } else {
        names.join(", ")
    }
}

/// Pluralizes a counted noun, e.g. `count_label(2, "file", "files")` → "2 files".
fn count_label(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{count} {noun}")
}

#[cfg(test)]
#[path = "tool_call_labels_tests.rs"]
mod tests;
