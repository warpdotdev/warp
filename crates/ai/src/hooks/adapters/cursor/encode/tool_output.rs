//! A `kind`-tagged JSON view of every `AIAgentActionResultType`, carried as a string in
//! `tool_output`.

use std::time::SystemTime;

use chrono::{DateTime, Local};
use serde_json::{Map, Value, json};
use warp_terminal::model::BlockId;

use super::{insert_some, range_view, serialized};
use crate::agent::action_result::{
    AIAgentActionResultType, AnyFileContent, AskUserQuestionAnswerItem, AskUserQuestionResult,
    CallMCPToolResult, CreateDocumentsResult, DocumentContext, EditDocumentsResult,
    FetchConversationResult, FileContext, FileGlobResult, FileGlobV2Result, GrepResult,
    InsertReviewCommentsResult, LrcActivity, ReadDocumentsResult, ReadFilesResult,
    ReadMCPResourceResult, ReadShellCommandOutputResult, ReadSkillResult,
    RequestCommandOutputResult, RequestComputerUseResult, RequestFileEditsResult, RunAgentsResult,
    ScreenshotSource, SearchCodebaseResult, SendMessageToAgentResult, StartRecordingResult,
    StopRecordingResult, SuggestNewConversationResult, SuggestPromptResult,
    TransferShellCommandControlToUserResult, UploadArtifactResult, UseComputerResult,
    WaitForEventsResult, WriteToLongRunningShellCommandResult,
};

/// A JSON object describing a result, tagged with the variant it came from. Binary payloads are
/// summarized by size; everything else is carried as the result holds it.
pub(super) fn result_view(result: &AIAgentActionResultType) -> Value {
    match result {
        AIAgentActionResultType::RequestCommandOutput(result) => match result {
            RequestCommandOutputResult::Completed {
                block_id,
                command,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("completed"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("exit_code".to_owned(), json!(exit_code.value()));
                fields.insert("block_id".to_owned(), serialized("block_id", block_id));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            RequestCommandOutputResult::LongRunningCommandSnapshot {
                block_id,
                command,
                grid_contents,
                cursor,
                is_alt_screen_active,
                activity,
            } => json!({
                "kind": "long_running_snapshot",
                "command": command,
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "block_id": serialized("block_id", block_id),
                "activity": serialized("activity", activity),
            }),
            RequestCommandOutputResult::ShellRecovered {
                block_id,
                command,
                output,
                status,
                restored_working_directory,
                used_fallback_directory,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("shell_recovered"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("status".to_owned(), serialized("status", status));
                fields.insert(
                    "restored_working_directory".to_owned(),
                    json!(restored_working_directory),
                );
                fields.insert(
                    "used_fallback_directory".to_owned(),
                    json!(used_fallback_directory),
                );
                fields.insert("block_id".to_owned(), serialized("block_id", block_id));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            RequestCommandOutputResult::CancelledBeforeExecution => {
                json!({ "kind": "cancelled_before_execution" })
            }
            RequestCommandOutputResult::Denylisted { command } => {
                json!({ "kind": "denylisted", "command": command })
            }
            RequestCommandOutputResult::HookDenied { command, reason } => {
                json!({ "kind": "hook_denied", "command": command, "reason": reason })
            }
            RequestCommandOutputResult::TerminalBusy { command, block_id } => json!({
                "kind": "terminal_busy",
                "command": command,
                "block_id": serialized("block_id", block_id),
            }),
        },
        AIAgentActionResultType::WriteToLongRunningShellCommand(result) => match result {
            WriteToLongRunningShellCommandResult::Snapshot {
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => pty_snapshot_view(
                block_id,
                grid_contents,
                cursor,
                *is_alt_screen_active,
                *is_preempted,
                activity,
            ),
            WriteToLongRunningShellCommandResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                None,
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            WriteToLongRunningShellCommandResult::Cancelled => json!({ "kind": "cancelled" }),
            WriteToLongRunningShellCommandResult::Error(error) => {
                json!({ "kind": "error", "error": serialized("error", error) })
            }
        },
        AIAgentActionResultType::RequestFileEdits(result) => match result {
            RequestFileEditsResult::Success {
                diff,
                updated_files,
                deleted_files,
                lines_added,
                lines_removed,
            } => json!({
                "kind": "success",
                "diff": diff,
                "updated_files": updated_files
                    .iter()
                    .map(|updated| json!({
                        "was_edited_by_user": updated.was_edited_by_user,
                        "file": file_context_view(&updated.file_context),
                    }))
                    .collect::<Vec<_>>(),
                "deleted_files": deleted_files,
                "lines_added": lines_added,
                "lines_removed": lines_removed,
            }),
            RequestFileEditsResult::Cancelled => json!({ "kind": "cancelled" }),
            RequestFileEditsResult::DiffApplicationFailed { error } => {
                json!({ "kind": "diff_application_failed", "error": error })
            }
        },
        AIAgentActionResultType::ReadFiles(result) => match result {
            ReadFilesResult::Success {
                files,
                failed_files,
            } => json!({
                "kind": "success",
                "files": file_context_views(files),
                "failed_files": serialized("failed_files", failed_files),
            }),
            ReadFilesResult::Error(error) => error_view(error),
            ReadFilesResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::UploadArtifact(result) => match result {
            UploadArtifactResult::Success {
                artifact_uid,
                filepath,
                mime_type,
                description,
                size_bytes,
            } => json!({
                "kind": "success",
                "artifact_uid": artifact_uid,
                "filepath": filepath,
                "mime_type": mime_type,
                "description": description,
                "size_bytes": size_bytes,
            }),
            UploadArtifactResult::Error(error) => error_view(error),
            UploadArtifactResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SearchCodebase(result) => match result {
            SearchCodebaseResult::Success { files } => {
                json!({ "kind": "success", "files": file_context_views(files) })
            }
            SearchCodebaseResult::Failed { reason, message } => json!({
                "kind": "failed",
                "reason": format!("{reason:?}"),
                "message": message,
            }),
            SearchCodebaseResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::Grep(result) => match result {
            GrepResult::Success { matched_files } => {
                json!({ "kind": "success", "matched_files": serialized("matched_files", matched_files) })
            }
            GrepResult::Error(error) => error_view(error),
            GrepResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::FileGlob(result) => match result {
            FileGlobResult::Success { matched_files } => {
                json!({ "kind": "success", "matched_files": matched_files })
            }
            FileGlobResult::Error(error) => error_view(error),
            FileGlobResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::FileGlobV2(result) => match result {
            FileGlobV2Result::Success {
                matched_files,
                warnings,
            } => json!({
                "kind": "success",
                "matched_files": serialized("matched_files", matched_files),
                "warnings": warnings,
            }),
            FileGlobV2Result::Error(error) => error_view(error),
            FileGlobV2Result::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadMCPResource(result) => match result {
            ReadMCPResourceResult::Success { resource_contents } => json!({
                "kind": "success",
                "resource_contents": serialized("resource_contents", resource_contents),
            }),
            ReadMCPResourceResult::Error(error) => error_view(error),
            ReadMCPResourceResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::CallMCPTool(result) => match result {
            CallMCPToolResult::Success { result } => {
                json!({ "kind": "success", "result": serialized("result", result) })
            }
            CallMCPToolResult::Error(error) => error_view(error),
            CallMCPToolResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadSkill(result) => match result {
            ReadSkillResult::Success { content } => {
                json!({ "kind": "success", "content": file_context_view(content) })
            }
            ReadSkillResult::Error(error) => error_view(error),
            ReadSkillResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SuggestNewConversation(result) => match result {
            SuggestNewConversationResult::Accepted { message_id } => {
                json!({ "kind": "accepted", "message_id": message_id })
            }
            SuggestNewConversationResult::Rejected => json!({ "kind": "rejected" }),
            SuggestNewConversationResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SuggestPrompt(result) => match result {
            SuggestPromptResult::Accepted { query } => {
                json!({ "kind": "accepted", "query": query })
            }
            SuggestPromptResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::OpenCodeReview | AIAgentActionResultType::InitProject => {
            json!({ "kind": "completed" })
        }
        AIAgentActionResultType::ReadDocuments(result) => match result {
            ReadDocumentsResult::Success { documents } => {
                json!({ "kind": "success", "documents": document_views(documents) })
            }
            ReadDocumentsResult::Error(error) => error_view(error),
            ReadDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::EditDocuments(result) => match result {
            EditDocumentsResult::Success { updated_documents } => json!({
                "kind": "success",
                "updated_documents": document_views(updated_documents),
            }),
            EditDocumentsResult::Error(error) => error_view(error),
            EditDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::CreateDocuments(result) => match result {
            CreateDocumentsResult::Success { created_documents } => json!({
                "kind": "success",
                "created_documents": document_views(created_documents),
            }),
            CreateDocumentsResult::Error(error) => error_view(error),
            CreateDocumentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::ReadShellCommandOutput(result) => match result {
            ReadShellCommandOutputResult::CommandFinished {
                command,
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                Some(command),
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => json!({
                "kind": "long_running_snapshot",
                "command": command,
                "block_id": serialized("block_id", block_id),
                "grid_contents": grid_contents,
                "cursor": cursor,
                "is_alt_screen_active": is_alt_screen_active,
                "is_preempted": is_preempted,
                "activity": serialized("activity", activity),
            }),
            ReadShellCommandOutputResult::ShellRecovered {
                command,
                block_id,
                output,
                status,
                start_ts,
                completed_ts,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("shell_recovered"));
                fields.insert("command".to_owned(), json!(command));
                fields.insert("block_id".to_owned(), serialized("block_id", block_id));
                fields.insert("output".to_owned(), json!(output));
                fields.insert("status".to_owned(), serialized("status", status));
                insert_some(&mut fields, "start_ts", timestamp(*start_ts));
                insert_some(&mut fields, "completed_ts", timestamp(*completed_ts));
                Value::Object(fields)
            }
            ReadShellCommandOutputResult::Cancelled => json!({ "kind": "cancelled" }),
            ReadShellCommandOutputResult::Error(error) => {
                json!({ "kind": "error", "error": serialized("error", error) })
            }
        },
        AIAgentActionResultType::UseComputer(result) => match result {
            UseComputerResult::Success {
                screenshot,
                cursor_position,
                windows,
                captured_window,
            } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("success"));
                insert_some(
                    &mut fields,
                    "screenshot",
                    screenshot.as_ref().map(screenshot_source_view),
                );
                insert_some(
                    &mut fields,
                    "cursor_position",
                    cursor_position.map(|position| json!([position.x(), position.y()])),
                );
                fields.insert("windows".to_owned(), serialized("windows", windows));
                insert_some(
                    &mut fields,
                    "captured_window",
                    captured_window.map(|window| {
                        json!({
                            "window_id": window.window_id,
                            "width_px": window.width_px,
                            "height_px": window.height_px,
                        })
                    }),
                );
                Value::Object(fields)
            }
            UseComputerResult::Error(error) => error_view(error),
            UseComputerResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::InsertReviewComments(result) => match result {
            InsertReviewCommentsResult::Success { repo_path } => {
                json!({ "kind": "success", "repo_path": repo_path })
            }
            InsertReviewCommentsResult::Error { repo_path, message } => {
                json!({ "kind": "error", "repo_path": repo_path, "error": message })
            }
            InsertReviewCommentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::RequestComputerUse(result) => match result {
            RequestComputerUseResult::Approved {
                screenshot,
                platform,
                windows,
            } => json!({
                "kind": "approved",
                "screenshot": screenshot_view(screenshot),
                "platform": format!("{platform:?}"),
                "windows": serialized("windows", windows),
            }),
            RequestComputerUseResult::Error(error) => error_view(error),
            RequestComputerUseResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::StartRecording(result) => match result {
            StartRecordingResult::Success(started) => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("success"));
                fields.insert("recording_id".to_owned(), json!(started.recording_id));
                insert_some(
                    &mut fields,
                    "started_at_ms",
                    unix_millis(started.started_at),
                );
                fields.insert("width_px".to_owned(), json!(started.width_px));
                fields.insert("height_px".to_owned(), json!(started.height_px));
                Value::Object(fields)
            }
            StartRecordingResult::Error(error) => error_view(error),
            StartRecordingResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::StopRecording(result) => match result {
            StopRecordingResult::Success(stopped) => json!({
                "kind": "success",
                "artifact_uid": stopped.artifact_uid,
                "duration_ms": stopped.duration.as_millis() as u64,
                "width_px": stopped.width_px,
                "height_px": stopped.height_px,
                "size_bytes": stopped.size_bytes,
                "completion_status": format!("{:?}", stopped.completion_status),
                "termination_reason": stopped.termination_reason,
            }),
            StopRecordingResult::Error(error) => error_view(error),
            StopRecordingResult::Cancelled => json!({ "kind": "cancelled" }),
            StopRecordingResult::Discarded => json!({ "kind": "discarded" }),
        },
        AIAgentActionResultType::FetchConversation(result) => match result {
            FetchConversationResult::Success { directory_path } => {
                json!({ "kind": "success", "directory_path": directory_path })
            }
            FetchConversationResult::Error(error) => error_view(error),
            FetchConversationResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::SendMessageToAgent(result) => match result {
            SendMessageToAgentResult::Success { message_id } => {
                json!({ "kind": "success", "message_id": message_id })
            }
            SendMessageToAgentResult::Error(error) => error_view(error),
            SendMessageToAgentResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::TransferShellCommandControlToUser(result) => match result {
            TransferShellCommandControlToUserResult::Snapshot {
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
                activity,
            } => pty_snapshot_view(
                block_id,
                grid_contents,
                cursor,
                *is_alt_screen_active,
                *is_preempted,
                activity,
            ),
            TransferShellCommandControlToUserResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => finished_command_view(
                block_id,
                None,
                output,
                exit_code.value(),
                *start_ts,
                *completed_ts,
            ),
            TransferShellCommandControlToUserResult::Cancelled => json!({ "kind": "cancelled" }),
            TransferShellCommandControlToUserResult::Error(error) => {
                json!({ "kind": "error", "error": serialized("error", error) })
            }
        },
        AIAgentActionResultType::AskUserQuestion(result) => match result {
            AskUserQuestionResult::Success { answers } => json!({
                "kind": "success",
                "answers": answers
                    .iter()
                    .map(|answer| match answer {
                        AskUserQuestionAnswerItem::Answered {
                            question_id,
                            selected_options,
                            other_text,
                        } => json!({
                            "question_id": question_id,
                            "selected_options": selected_options,
                            "other_text": other_text,
                        }),
                        AskUserQuestionAnswerItem::Skipped { question_id } => {
                            json!({ "question_id": question_id, "skipped": true })
                        }
                    })
                    .collect::<Vec<_>>(),
            }),
            AskUserQuestionResult::Error(error) => error_view(error),
            AskUserQuestionResult::Cancelled => json!({ "kind": "cancelled" }),
            AskUserQuestionResult::SkippedByAutoApprove { question_ids } => json!({
                "kind": "skipped_by_auto_approve",
                "question_ids": question_ids,
            }),
        },
        AIAgentActionResultType::RunAgents(result) => match result {
            RunAgentsResult::Launched {
                model_id,
                harness_type,
                execution_mode,
                agents,
            } => json!({
                "kind": "launched",
                "model_id": model_id,
                "harness_type": harness_type,
                "execution_mode": serialized("execution_mode", execution_mode),
                "agents": serialized("agents", agents),
            }),
            RunAgentsResult::Denied { reason } => json!({ "kind": "denied", "reason": reason }),
            RunAgentsResult::Failure { error } => json!({ "kind": "failure", "error": error }),
            RunAgentsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
        AIAgentActionResultType::WaitForEvents(result) => match result {
            WaitForEventsResult::Completed => json!({ "kind": "completed" }),
            WaitForEventsResult::Cancelled => json!({ "kind": "cancelled" }),
        },
    }
}

fn error_view(error: &str) -> Value {
    json!({ "kind": "error", "error": error })
}

fn pty_snapshot_view(
    block_id: &BlockId,
    grid_contents: &str,
    cursor: &str,
    is_alt_screen_active: bool,
    is_preempted: bool,
    activity: &Option<LrcActivity>,
) -> Value {
    json!({
        "kind": "snapshot",
        "block_id": serialized("block_id", block_id),
        "grid_contents": grid_contents,
        "cursor": cursor,
        "is_alt_screen_active": is_alt_screen_active,
        "is_preempted": is_preempted,
        "activity": serialized("activity", activity),
    })
}

fn finished_command_view(
    block_id: &BlockId,
    command: Option<&str>,
    output: &str,
    exit_code: i32,
    start_ts: Option<DateTime<Local>>,
    completed_ts: Option<DateTime<Local>>,
) -> Value {
    let mut fields = Map::new();
    fields.insert("kind".to_owned(), json!("command_finished"));
    insert_some(&mut fields, "command", command);
    fields.insert("block_id".to_owned(), serialized("block_id", block_id));
    fields.insert("output".to_owned(), json!(output));
    fields.insert("exit_code".to_owned(), json!(exit_code));
    insert_some(&mut fields, "start_ts", timestamp(start_ts));
    insert_some(&mut fields, "completed_ts", timestamp(completed_ts));
    Value::Object(fields)
}

fn file_context_view(context: &FileContext) -> Value {
    let mut fields = Map::new();
    fields.insert("file_name".to_owned(), json!(context.file_name));
    match &context.content {
        AnyFileContent::StringContent(content) => {
            fields.insert("content".to_owned(), json!(content));
        }
        AnyFileContent::BinaryContent(bytes) => {
            fields.insert("is_binary".to_owned(), json!(true));
            fields.insert("size_bytes".to_owned(), json!(bytes.len()));
        }
    }
    insert_some(
        &mut fields,
        "line_range",
        context.line_range.as_ref().map(range_view),
    );
    fields.insert("line_count".to_owned(), json!(context.line_count));
    Value::Object(fields)
}

fn file_context_views(contexts: &[FileContext]) -> Value {
    Value::Array(contexts.iter().map(file_context_view).collect())
}

fn document_view(document: &DocumentContext) -> Value {
    json!({
        "document_id": document.document_id.to_string(),
        "version": document.document_version.0,
        "content": document.content,
        "line_ranges": document.line_ranges.iter().map(range_view).collect::<Vec<_>>(),
    })
}

fn document_views(documents: &[DocumentContext]) -> Value {
    Value::Array(documents.iter().map(document_view).collect())
}

fn screenshot_view(screenshot: &computer_use::Screenshot) -> Value {
    json!({
        "width": screenshot.width,
        "height": screenshot.height,
        "original_width": screenshot.original_width,
        "original_height": screenshot.original_height,
        "mime_type": screenshot.mime_type,
        "size_bytes": screenshot.data.len(),
    })
}

fn screenshot_source_view(source: &ScreenshotSource) -> Value {
    match source {
        ScreenshotSource::Inline(screenshot) => screenshot_view(screenshot),
        ScreenshotSource::Stored {
            stored_ref: _,
            mime_type,
            width,
            height,
        } => json!({
            "stored": true,
            "mime_type": mime_type,
            "width": width,
            "height": height,
        }),
    }
}

fn unix_millis(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_millis() as u64)
}

fn timestamp(ts: Option<DateTime<Local>>) -> Option<String> {
    ts.map(|ts| ts.to_rfc3339())
}
