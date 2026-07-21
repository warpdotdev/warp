//! Per-tool, per-state one-line labels for tool-call rows in the TUI
//! transcript, modeled on the GUI's inline action text.

use std::path::Path;

use ai::agent::action_result::RunAgentsAgentOutcome;
use warp::tui_export::{
    menu_label, AIActionStatus, AIAgentAction, AIAgentActionResultType, AIAgentActionType,
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
        State::Blocked => format!(
            "{label}{}",
            menu_label(
                "tui.common.awaiting_approval_suffix",
                " (awaiting approval)"
            )
        ),
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
                State::Constructing => menu_label(
                    "tui.request_command_output.constructing",
                    "Generating command…",
                )
                .to_owned(),
                State::Pending | State::Blocked => {
                    menu_label("tui.request_command_output.pending", "Run `{cmd}`")
                        .replace("{cmd}", &cmd)
                }
                State::Running => {
                    menu_label("tui.request_command_output.running", "Running `{cmd}`")
                        .replace("{cmd}", &cmd)
                }
                State::Succeeded => match block_state {
                    Some(CommandBlockState::Finished { .. }) => {
                        menu_label("tui.request_command_output.succeeded", "Ran `{cmd}`")
                            .replace("{cmd}", &cmd)
                    }
                    // No local block: fall back to the stored result. A
                    // snapshot result means the command was still running at
                    // the last point we could observe it.
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::LongRunningCommandSnapshot { .. },
                        )) => menu_label(
                            "tui.request_command_output.still_running",
                            "`{cmd}` is still running",
                        )
                        .replace("{cmd}", &cmd),
                        _ => menu_label("tui.request_command_output.succeeded", "Ran `{cmd}`")
                            .replace("{cmd}", &cmd),
                    },
                },
                State::Failed => match block_state {
                    Some(CommandBlockState::Finished { exit_code }) => menu_label(
                        "tui.request_command_output.exited",
                        "`{cmd}` exited with code {code}",
                    )
                    .replace("{cmd}", &cmd)
                    .replace("{code}", &exit_code.value().to_string()),
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Completed { exit_code, .. },
                        )) => menu_label(
                            "tui.request_command_output.exited",
                            "`{cmd}` exited with code {code}",
                        )
                        .replace("{cmd}", &cmd)
                        .replace("{code}", &exit_code.value().to_string()),
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Denylisted { .. },
                        )) => menu_label(
                            "tui.request_command_output.denied_denylisted",
                            "`{cmd}` denied (denylisted)",
                        )
                        .replace("{cmd}", &cmd),
                        _ => menu_label("tui.request_command_output.failed", "`{cmd}` failed")
                            .replace("{cmd}", &cmd),
                    },
                },
                State::Cancelled => {
                    menu_label("tui.request_command_output.cancelled", "Cancelled `{cmd}`")
                        .replace("{cmd}", &cmd)
                }
            }
        }
        AIAgentActionType::WriteToLongRunningShellCommand { .. } => match state {
            State::Constructing => menu_label(
                "tui.write_to_long_running_shell_command.constructing",
                "Writing command input…",
            )
            .to_owned(),
            State::Pending | State::Blocked => menu_label(
                "tui.write_to_long_running_shell_command.pending",
                "Write input to running command",
            )
            .to_owned(),
            State::Running => menu_label(
                "tui.write_to_long_running_shell_command.running",
                "Writing input to running command…",
            )
            .to_owned(),
            State::Succeeded => menu_label(
                "tui.write_to_long_running_shell_command.succeeded",
                "Wrote input to running command",
            )
            .to_owned(),
            State::Failed => menu_label(
                "tui.write_to_long_running_shell_command.failed",
                "Failed to write to running command",
            )
            .to_owned(),
            State::Cancelled => menu_label(
                "tui.write_to_long_running_shell_command.cancelled",
                "Write to running command cancelled",
            )
            .to_owned(),
        },
        AIAgentActionType::ReadFiles(request) => {
            let files = files_summary(request.locations.iter().map(|location| &location.name));
            match state {
                State::Constructing => {
                    menu_label("tui.read_files.constructing", "Reading files…").to_owned()
                }
                State::Pending | State::Blocked | State::Succeeded => {
                    menu_label("tui.read_files.succeeded", "Read {files}")
                        .replace("{files}", &files)
                }
                State::Running => menu_label("tui.read_files.running", "Reading {files}")
                    .replace("{files}", &files),
                State::Failed => menu_label("tui.read_files.failed", "Failed to read {files}")
                    .replace("{files}", &files),
                State::Cancelled => {
                    menu_label("tui.read_files.cancelled", "Cancelled reading {files}")
                        .replace("{files}", &files)
                }
            }
        }
        AIAgentActionType::UploadArtifact(request) => {
            let file = single_line(&request.file_path);
            match state {
                State::Constructing => {
                    menu_label("tui.upload_artifact.constructing", "Preparing upload…").to_owned()
                }
                State::Pending | State::Blocked => {
                    menu_label("tui.upload_artifact.pending", "Upload {file}")
                        .replace("{file}", &file)
                }
                State::Running => menu_label("tui.upload_artifact.running", "Uploading {file}")
                    .replace("{file}", &file),
                State::Succeeded => menu_label("tui.upload_artifact.succeeded", "Uploaded {file}")
                    .replace("{file}", &file),
                State::Failed => {
                    menu_label("tui.upload_artifact.failed", "Upload of {file} failed")
                        .replace("{file}", &file)
                }
                State::Cancelled => menu_label(
                    "tui.upload_artifact.cancelled",
                    "Upload of {file} cancelled",
                )
                .replace("{file}", &file),
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
                State::Constructing => {
                    menu_label("tui.search_codebase.constructing", "Searching codebase…").to_owned()
                }
                State::Pending | State::Blocked => menu_label(
                    "tui.search_codebase.pending",
                    "Search for \"{query}\"{scope}",
                )
                .replace("{query}", &query)
                .replace("{scope}", &scope),
                State::Running => menu_label(
                    "tui.search_codebase.running",
                    "Searching for \"{query}\"{scope}",
                )
                .replace("{query}", &query)
                .replace("{scope}", &scope),
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Success { files },
                    )) if files.is_empty() => menu_label(
                        "tui.search_codebase.succeeded_no_results",
                        "Searched for \"{query}\"{scope}, no results",
                    )
                    .replace("{query}", &query)
                    .replace("{scope}", &scope),
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Success { files },
                    )) => {
                        let count_noun = count_label(files.len(), "result", "results");
                        menu_label(
                            "tui.search_codebase.succeeded_with_count",
                            "Searched for \"{query}\"{scope}, {count} {noun}",
                        )
                        .replace("{query}", &query)
                        .replace("{scope}", &scope)
                        .replace("{count} {noun}", &count_noun)
                    }
                    _ => menu_label(
                        "tui.search_codebase.succeeded_generic",
                        "Searched for \"{query}\"{scope}",
                    )
                    .replace("{query}", &query)
                    .replace("{scope}", &scope),
                },
                State::Failed => match result {
                    Some(AIAgentActionResultType::SearchCodebase(
                        SearchCodebaseResult::Failed {
                            reason: SearchCodebaseFailureReason::CodebaseNotIndexed,
                            ..
                        },
                    )) => menu_label(
                        "tui.search_codebase.failed_not_indexed",
                        "Search for \"{query}\"{scope} failed because the codebase isn't indexed",
                    )
                    .replace("{query}", &query)
                    .replace("{scope}", &scope),
                    _ => menu_label(
                        "tui.search_codebase.failed",
                        "Search for \"{query}\"{scope} failed",
                    )
                    .replace("{query}", &query)
                    .replace("{scope}", &scope),
                },
                State::Cancelled => menu_label(
                    "tui.search_codebase.cancelled",
                    "Search for \"{query}\"{scope} cancelled",
                )
                .replace("{query}", &query)
                .replace("{scope}", &scope),
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
                State::Constructing => menu_label("tui.grep.constructing", "Grepping…").to_owned(),
                State::Pending | State::Blocked => {
                    menu_label("tui.grep.pending", "Grep for {queries} in {path}")
                        .replace("{queries}", &queries)
                        .replace("{path}", &path)
                }
                State::Running => {
                    menu_label("tui.grep.running", "Grepping for {queries} in {path}")
                        .replace("{queries}", &queries)
                        .replace("{path}", &path)
                }
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::Grep(GrepResult::Success { matched_files })) => {
                        let count_noun =
                            count_label(matched_files.len(), "matching file", "matching files");
                        menu_label(
                            "tui.grep.succeeded_with_count",
                            "Grepped for {queries} in {path}, {count} {noun}",
                        )
                        .replace("{queries}", &queries)
                        .replace("{path}", &path)
                        .replace("{count} {noun}", &count_noun)
                    }
                    _ => menu_label(
                        "tui.grep.succeeded_generic",
                        "Grepped for {queries} in {path}",
                    )
                    .replace("{queries}", &queries)
                    .replace("{path}", &path),
                },
                State::Failed => menu_label("tui.grep.failed", "Grep for {queries} failed")
                    .replace("{queries}", &queries),
                State::Cancelled => {
                    menu_label("tui.grep.cancelled", "Grep for {queries} cancelled")
                        .replace("{queries}", &queries)
                }
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
                State::Constructing if name.is_empty() => menu_label(
                    "tui.read_mcp_resource.constructing_no_name",
                    "Reading MCP resource…",
                )
                .to_owned(),
                State::Constructing => menu_label(
                    "tui.read_mcp_resource.constructing",
                    "Reading \"{name}\" MCP resource…",
                )
                .replace("{name}", name),
                State::Pending | State::Blocked | State::Succeeded => menu_label(
                    "tui.read_mcp_resource.succeeded",
                    "Read MCP resource {resource}",
                )
                .replace("{resource}", &resource),
                State::Running => menu_label(
                    "tui.read_mcp_resource.running",
                    "Reading MCP resource {resource}",
                )
                .replace("{resource}", &resource),
                State::Failed => menu_label(
                    "tui.read_mcp_resource.failed",
                    "MCP resource {resource} failed",
                )
                .replace("{resource}", &resource),
                State::Cancelled => menu_label(
                    "tui.read_mcp_resource.cancelled",
                    "MCP resource {resource} cancelled",
                )
                .replace("{resource}", &resource),
            }
        }
        AIAgentActionType::CallMCPTool { name, .. } => {
            let name = single_line(name);
            match state {
                // Like the GUI's "Calling \"{name}\" MCP tool..." loading
                // text; the tool name is available before its args finish.
                State::Constructing if name.is_empty() => menu_label(
                    "tui.call_mcp_tool.constructing_no_name",
                    "Calling MCP tool…",
                )
                .to_owned(),
                State::Constructing => menu_label(
                    "tui.call_mcp_tool.constructing",
                    "Calling \"{name}\" MCP tool…",
                )
                .replace("{name}", &name),
                State::Pending | State::Blocked => {
                    menu_label("tui.call_mcp_tool.pending", "Call MCP tool {name}")
                        .replace("{name}", &name)
                }
                State::Running => {
                    menu_label("tui.call_mcp_tool.running", "Calling MCP tool {name}")
                        .replace("{name}", &name)
                }
                State::Succeeded => {
                    menu_label("tui.call_mcp_tool.succeeded", "Called MCP tool {name}")
                        .replace("{name}", &name)
                }
                State::Failed => menu_label("tui.call_mcp_tool.failed", "MCP tool {name} failed")
                    .replace("{name}", &name),
                State::Cancelled => {
                    menu_label("tui.call_mcp_tool.cancelled", "MCP tool {name} cancelled")
                        .replace("{name}", &name)
                }
            }
        }
        AIAgentActionType::SuggestNewConversation { .. } => match state {
            State::Constructing => menu_label(
                "tui.suggest_new_conversation.constructing",
                "Suggesting a new conversation…",
            )
            .to_owned(),
            State::Pending | State::Blocked | State::Running | State::Failed => menu_label(
                "tui.suggest_new_conversation.pending",
                "Suggested starting a new conversation",
            )
            .to_owned(),
            State::Succeeded => match result {
                Some(AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Rejected,
                )) => menu_label(
                    "tui.suggest_new_conversation.succeeded_rejected",
                    "Continuing current conversation",
                )
                .to_owned(),
                _ => menu_label(
                    "tui.suggest_new_conversation.succeeded",
                    "New conversation started",
                )
                .to_owned(),
            },
            State::Cancelled => menu_label(
                "tui.suggest_new_conversation.cancelled",
                "New conversation suggestion cancelled",
            )
            .to_owned(),
        },
        AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview => fallback_label(action, state),
        AIAgentActionType::ReadDocuments(request) => {
            let documents = count_label(request.document_ids.len(), "document", "documents");
            match state {
                State::Constructing => {
                    menu_label("tui.read_documents.constructing", "Reading documents…").to_owned()
                }
                State::Pending | State::Blocked | State::Succeeded => {
                    menu_label("tui.read_documents.succeeded", "Read {documents}")
                        .replace("{documents}", &documents)
                }
                State::Running => menu_label("tui.read_documents.running", "Reading {documents}")
                    .replace("{documents}", &documents),
                State::Failed => {
                    menu_label("tui.read_documents.failed", "Failed to read documents").to_owned()
                }
                State::Cancelled => menu_label(
                    "tui.read_documents.cancelled",
                    "Cancelled reading documents",
                )
                .to_owned(),
            }
        }
        AIAgentActionType::EditDocuments(request) => match state {
            State::Pending | State::Blocked => {
                menu_label("tui.edit_documents.pending", "Update plan").to_owned()
            }
            State::Constructing | State::Running => {
                menu_label("tui.edit_documents.running", "Updating plan…").to_owned()
            }
            State::Succeeded => {
                let count_noun = count_label(request.diffs.len(), "edit", "edits");
                menu_label(
                    "tui.edit_documents.succeeded",
                    "Updated plan ({count} {noun})",
                )
                .replace("{count} {noun}", &count_noun)
            }
            State::Failed => {
                menu_label("tui.edit_documents.failed", "Failed to update plan").to_owned()
            }
            State::Cancelled => {
                menu_label("tui.edit_documents.cancelled", "Update plan cancelled").to_owned()
            }
        },
        AIAgentActionType::CreateDocuments(request) => match state {
            State::Pending | State::Blocked => {
                menu_label("tui.create_documents.pending", "Create plan").to_owned()
            }
            State::Constructing | State::Running => {
                menu_label("tui.create_documents.running", "Generating plan…").to_owned()
            }
            State::Succeeded => {
                let count = request.documents.len();
                if count > 1 {
                    menu_label(
                        "tui.create_documents.succeeded_multi",
                        "Created {count} documents",
                    )
                    .replace("{count}", &count.to_string())
                } else {
                    menu_label("tui.create_documents.succeeded_single", "Created plan").to_owned()
                }
            }
            State::Failed => {
                menu_label("tui.create_documents.failed", "Failed to create plan").to_owned()
            }
            State::Cancelled => {
                menu_label("tui.create_documents.cancelled", "Create plan cancelled").to_owned()
            }
        },
        AIAgentActionType::ReadShellCommandOutput { .. } => match state {
            State::Pending | State::Blocked | State::Succeeded => menu_label(
                "tui.read_shell_command_output.succeeded",
                "Read command output",
            )
            .to_owned(),
            State::Constructing | State::Running => menu_label(
                "tui.read_shell_command_output.running",
                "Reading command output…",
            )
            .to_owned(),
            State::Failed => menu_label(
                "tui.read_shell_command_output.failed",
                "Failed to read command output",
            )
            .to_owned(),
            State::Cancelled => menu_label(
                "tui.read_shell_command_output.cancelled",
                "Read command output cancelled",
            )
            .to_owned(),
        },
        AIAgentActionType::UseComputer(request) => summary_label(&request.action_summary, state),
        AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
            let comments_count = count_label(comments.len(), "review comment", "review comments");
            match state {
                State::Constructing => menu_label(
                    "tui.insert_code_review_comments.constructing",
                    "Preparing review comments…",
                )
                .to_owned(),
                State::Pending | State::Blocked => menu_label(
                    "tui.insert_code_review_comments.pending",
                    "Insert {comments}",
                )
                .replace("{comments}", &comments_count),
                State::Running => menu_label(
                    "tui.insert_code_review_comments.running",
                    "Inserting {comments}…",
                )
                .replace("{comments}", &comments_count),
                State::Succeeded => menu_label(
                    "tui.insert_code_review_comments.succeeded",
                    "Inserted {comments}",
                )
                .replace("{comments}", &comments_count),
                State::Failed => menu_label(
                    "tui.insert_code_review_comments.failed",
                    "Failed to insert review comments",
                )
                .to_owned(),
                State::Cancelled => menu_label(
                    "tui.insert_code_review_comments.cancelled",
                    "Insert review comments cancelled",
                )
                .to_owned(),
            }
        }
        AIAgentActionType::RequestComputerUse(request) => {
            summary_label(&request.task_summary, state)
        }
        AIAgentActionType::StartRecording { .. } => match state {
            State::Pending | State::Blocked => {
                menu_label("tui.start_recording.pending", "Start recording").to_owned()
            }
            State::Constructing | State::Running => {
                menu_label("tui.start_recording.running", "Starting recording…").to_owned()
            }
            State::Succeeded => {
                menu_label("tui.start_recording.succeeded", "Started screen recording").to_owned()
            }
            State::Failed => {
                menu_label("tui.start_recording.failed", "Recording failed to start").to_owned()
            }
            State::Cancelled => {
                menu_label("tui.start_recording.cancelled", "Start recording cancelled").to_owned()
            }
        },
        AIAgentActionType::StopRecording { .. } => match state {
            State::Pending | State::Blocked => {
                menu_label("tui.stop_recording.pending", "Stop recording").to_owned()
            }
            State::Constructing | State::Running => {
                menu_label("tui.stop_recording.running", "Stopping recording…").to_owned()
            }
            State::Succeeded => {
                menu_label("tui.stop_recording.succeeded", "Saved screen recording").to_owned()
            }
            State::Failed => {
                menu_label("tui.stop_recording.failed", "Failed to save recording").to_owned()
            }
            State::Cancelled => {
                menu_label("tui.stop_recording.cancelled", "Stop recording cancelled").to_owned()
            }
        },
        AIAgentActionType::ReadSkill(request) => {
            let skill = single_line(&request.skill.display_label());
            match state {
                State::Constructing => {
                    menu_label("tui.read_skill.constructing", "Reading skill…").to_owned()
                }
                State::Pending | State::Blocked | State::Succeeded => {
                    menu_label("tui.read_skill.succeeded", "Read skill {skill}")
                        .replace("{skill}", &skill)
                }
                State::Running => menu_label("tui.read_skill.running", "Reading skill {skill}")
                    .replace("{skill}", &skill),
                State::Failed => {
                    menu_label("tui.read_skill.failed", "Failed to read skill {skill}")
                        .replace("{skill}", &skill)
                }
                State::Cancelled => menu_label(
                    "tui.read_skill.cancelled",
                    "Cancelled reading skill {skill}",
                )
                .replace("{skill}", &skill),
            }
        }
        AIAgentActionType::FetchConversation { .. } => match state {
            State::Pending | State::Blocked => {
                menu_label("tui.fetch_conversation.pending", "Fetch conversation").to_owned()
            }
            State::Constructing | State::Running => {
                menu_label("tui.fetch_conversation.running", "Fetching conversation…").to_owned()
            }
            State::Succeeded => {
                menu_label("tui.fetch_conversation.succeeded", "Fetched conversation").to_owned()
            }
            State::Failed => {
                menu_label("tui.fetch_conversation.failed", "Fetch conversation failed").to_owned()
            }
            State::Cancelled => menu_label(
                "tui.fetch_conversation.cancelled",
                "Fetch conversation cancelled",
            )
            .to_owned(),
        },
        AIAgentActionType::StartAgent {
            name,
            execution_mode,
            ..
        } => {
            let agent_label = if matches!(execution_mode, StartAgentExecutionMode::Remote { .. }) {
                menu_label("tui.start_agent.remote_agent_label", "remote agent {name}")
                    .replace("{name}", name)
            } else {
                menu_label("tui.start_agent.local_agent_label", "agent {name}")
                    .replace("{name}", name)
            };
            match state {
                State::Constructing => {
                    menu_label("tui.start_agent.constructing", "Configuring agent…").to_owned()
                }
                State::Pending | State::Blocked => {
                    menu_label("tui.start_agent.pending", "Start {agent}")
                        .replace("{agent}", &agent_label)
                }
                State::Running => menu_label("tui.start_agent.running", "Starting {agent}…")
                    .replace("{agent}", &agent_label),
                State::Succeeded => menu_label("tui.start_agent.succeeded", "Started agent {name}")
                    .replace("{name}", name),
                State::Failed => {
                    menu_label("tui.start_agent.failed", "Failed to start agent {name}")
                        .replace("{name}", name)
                }
                State::Cancelled => {
                    menu_label("tui.start_agent.cancelled", "Start agent {name} cancelled")
                        .replace("{name}", name)
                }
            }
        }
        AIAgentActionType::SendMessageToAgent {
            addresses, subject, ..
        } => {
            let subject = single_line(subject);
            match state {
                State::Constructing => menu_label(
                    "tui.send_message_to_agent.constructing",
                    "Composing message…",
                )
                .to_owned(),
                State::Pending | State::Blocked => menu_label(
                    "tui.send_message_to_agent.pending",
                    "Send message: {subject}",
                )
                .replace("{subject}", &subject),
                State::Running => {
                    let count_noun = count_label(addresses.len(), "agent", "agents");
                    menu_label(
                        "tui.send_message_to_agent.running",
                        "Sending message to {count} {noun}: {subject}",
                    )
                    .replace("{count} {noun}", &count_noun)
                    .replace("{subject}", &subject)
                }
                State::Succeeded => menu_label(
                    "tui.send_message_to_agent.succeeded",
                    "Sent message: {subject}",
                )
                .replace("{subject}", &subject),
                State::Failed => menu_label(
                    "tui.send_message_to_agent.failed",
                    "Failed to send message: {subject}",
                )
                .replace("{subject}", &subject),
                State::Cancelled => menu_label(
                    "tui.send_message_to_agent.cancelled",
                    "Send message cancelled",
                )
                .to_owned(),
            }
        }
        AIAgentActionType::TransferShellCommandControlToUser { reason } => match state {
            State::Constructing => menu_label(
                "tui.transfer_shell_command_control_to_user.constructing",
                "Handing control to you…",
            )
            .to_owned(),
            State::Pending | State::Blocked | State::Running => menu_label(
                "tui.transfer_shell_command_control_to_user.running",
                "Handing control to you: {reason}",
            )
            .replace("{reason}", &single_line(reason)),
            State::Succeeded => menu_label(
                "tui.transfer_shell_command_control_to_user.succeeded",
                "You are in control",
            )
            .to_owned(),
            State::Failed => menu_label(
                "tui.transfer_shell_command_control_to_user.failed",
                "Control transfer failed",
            )
            .to_owned(),
            State::Cancelled => menu_label(
                "tui.transfer_shell_command_control_to_user.cancelled",
                "Control transfer cancelled",
            )
            .to_owned(),
        },
        AIAgentActionType::AskUserQuestion { questions } => {
            let total = questions.len();
            match state {
                State::Constructing => {
                    menu_label("tui.ask_user_question.constructing", "Preparing question…")
                        .to_owned()
                }
                State::Pending | State::Blocked | State::Running => {
                    let count_noun = count_label(total, "question", "questions");
                    menu_label("tui.ask_user_question.running", "Asking {count} {noun}")
                        .replace("{count} {noun}", &count_noun)
                }
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::AskUserQuestion(
                        AskUserQuestionResult::Success { answers },
                    )) => {
                        let total = answers.len();
                        let answered = answers.iter().filter(|answer| !answer.is_skipped()).count();
                        if answered == 0 {
                            menu_label("tui.ask_user_question.skipped", "Questions skipped")
                                .to_owned()
                        } else if answered == total && total == 1 {
                            menu_label("tui.ask_user_question.single_answered", "Answered question")
                                .to_owned()
                        } else if answered == total {
                            menu_label(
                                "tui.ask_user_question.all_answered",
                                "Answered all {total} questions",
                            )
                            .replace("{total}", &total.to_string())
                        } else {
                            menu_label(
                                "tui.ask_user_question.partial_answered",
                                "Answered {answered} of {total} questions",
                            )
                            .replace("{answered}", &answered.to_string())
                            .replace("{total}", &total.to_string())
                        }
                    }
                    Some(AIAgentActionResultType::AskUserQuestion(
                        AskUserQuestionResult::SkippedByAutoApprove { .. },
                    )) => {
                        menu_label("tui.ask_user_question.skipped", "Questions skipped").to_owned()
                    }
                    _ => menu_label(
                        "tui.ask_user_question.answered_generic",
                        "Answered questions",
                    )
                    .to_owned(),
                },
                State::Failed => {
                    menu_label("tui.ask_user_question.failed", "Questions failed").to_owned()
                }
                State::Cancelled => {
                    menu_label("tui.ask_user_question.cancelled", "Questions cancelled").to_owned()
                }
            }
        }
        AIAgentActionType::RunAgents(request) => {
            let total = request.agent_run_configs.len();
            let count_noun = count_label(total, "agent", "agents");
            match state {
                State::Constructing | State::Pending | State::Blocked => {
                    menu_label("tui.run_agents.constructing", "Configuring agents…").to_owned()
                }
                State::Running => menu_label("tui.run_agents.running", "Spawning {count} {noun}…")
                    .replace("{count} {noun}", &count_noun),
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                        agents,
                        ..
                    })) => launched_agents_label(agents),
                    _ => menu_label("tui.run_agents.succeeded_all", "Spawned {count} {noun}")
                        .replace("{count} {noun}", &count_noun),
                },
                State::Failed => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                        agents,
                        ..
                    })) => launched_agents_label(agents),
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
                        ..
                    })) => menu_label(
                        "tui.run_agents.denied",
                        "Orchestration disabled — agents not launched",
                    )
                    .to_owned(),
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Failure {
                        error,
                    })) if !error.is_empty() => menu_label(
                        "tui.run_agents.failed_with_error",
                        "Failed to start orchestration: {error}",
                    )
                    .replace("{error}", &single_line(error)),
                    _ => menu_label(
                        "tui.run_agents.failed_generic",
                        "Failed to start orchestration",
                    )
                    .to_owned(),
                },
                State::Cancelled => {
                    menu_label("tui.run_agents.cancelled", "Spawn agents cancelled").to_owned()
                }
            }
        }
        AIAgentActionType::WaitForEvents { .. } => match state {
            State::Constructing | State::Pending | State::Blocked | State::Running => {
                menu_label("tui.wait_for_events.running", "Waiting for agent events…").to_owned()
            }
            State::Succeeded => menu_label(
                "tui.wait_for_events.succeeded",
                "Done waiting for agent events",
            )
            .to_owned(),
            State::Failed => menu_label(
                "tui.wait_for_events.failed",
                "Waiting for agent events failed",
            )
            .to_owned(),
            State::Cancelled => {
                menu_label("tui.wait_for_events.cancelled", "Wait for events cancelled").to_owned()
            }
        },
    }
}

fn launched_agents_label(agents: &[RunAgentsAgentOutcome]) -> String {
    let launched = agents
        .iter()
        .filter(|agent| matches!(agent.kind, RunAgentsAgentOutcomeKind::Launched { .. }))
        .count();
    let total = agents.len();
    if launched == total {
        menu_label("tui.run_agents.succeeded_all", "Spawned {count} {noun}")
            .replace("{count} {noun}", &count_label(total, "agent", "agents"))
    } else if launched == 0 {
        menu_label(
            "tui.run_agents.failed_spawn",
            "Failed to spawn {count} {noun}",
        )
        .replace("{count} {noun}", &count_label(total, "agent", "agents"))
    } else {
        menu_label(
            "tui.run_agents.succeeded_partial",
            "Spawned {launched} of {total} agents",
        )
        .replace("{launched}", &launched.to_string())
        .replace("{total}", &total.to_string())
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
        State::Constructing => {
            menu_label("tui.file_glob.constructing", "Finding files…").to_owned()
        }
        State::Pending | State::Blocked => menu_label(
            "tui.file_glob.pending",
            "Find files matching {patterns} in {path}",
        )
        .replace("{patterns}", &patterns)
        .replace("{path}", &path),
        State::Running => menu_label(
            "tui.file_glob.running",
            "Finding files matching {patterns} in {path}",
        )
        .replace("{patterns}", &patterns)
        .replace("{path}", &path),
        State::Succeeded => match matched_count {
            Some(count) => {
                let count_noun = count_label(count, "file", "files");
                menu_label(
                    "tui.file_glob.succeeded_with_count",
                    "Found {count} {noun} matching {patterns}",
                )
                .replace("{count} {noun}", &count_noun)
                .replace("{patterns}", &patterns)
            }
            None => menu_label(
                "tui.file_glob.succeeded_generic",
                "Found files matching {patterns}",
            )
            .replace("{patterns}", &patterns),
        },
        State::Failed => menu_label("tui.file_glob.failed", "File search for {patterns} failed")
            .replace("{patterns}", &patterns),
        State::Cancelled => menu_label(
            "tui.file_glob.cancelled",
            "File search for {patterns} cancelled",
        )
        .replace("{patterns}", &patterns),
    }
}

/// Labels computer-use calls with their agent-supplied summary, marking only
/// terminal non-success states (matching the GUI, which shows the summary
/// verbatim).
fn summary_label(summary: &str, state: State) -> String {
    let summary = single_line(summary);
    match state {
        State::Constructing => {
            menu_label("tui.summary.constructing", "Preparing computer use…").to_owned()
        }
        State::Pending | State::Blocked | State::Running | State::Succeeded => {
            menu_label("tui.summary.succeeded", "{summary}").replace("{summary}", &summary)
        }
        State::Failed => {
            menu_label("tui.summary.failed", "{summary} — failed").replace("{summary}", &summary)
        }
        State::Cancelled => menu_label("tui.summary.cancelled", "{summary} — cancelled")
            .replace("{summary}", &summary),
    }
}

/// Generic label for action types without bespoke text, derived from the
/// action's user-friendly name.
fn fallback_label(action: &AIAgentActionType, state: State) -> String {
    let name = action.user_friendly_name();
    match state {
        State::Pending | State::Blocked => {
            menu_label("tui.fallback.pending", "{name}").replace("{name}", &name)
        }
        State::Constructing | State::Running => {
            menu_label("tui.fallback.running", "{name}…").replace("{name}", &name)
        }
        State::Succeeded => {
            menu_label("tui.fallback.succeeded", "{name} — done").replace("{name}", &name)
        }
        State::Failed => {
            menu_label("tui.fallback.failed", "{name} — failed").replace("{name}", &name)
        }
        State::Cancelled => {
            menu_label("tui.fallback.cancelled", "{name} — cancelled").replace("{name}", &name)
        }
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
        menu_label(
            "tui.helper.current_directory_label",
            "the current directory",
        )
        .to_owned()
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
        menu_label("tui.helper.empty_files_fallback", "files").to_owned()
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
