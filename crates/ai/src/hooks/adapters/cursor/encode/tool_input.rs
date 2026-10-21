//! The `tool_input` object for the tool events: the action's arguments as the model gave them, one
//! hand-written JSON view per `AIAgentActionType`.

use serde_json::{Map, Value, json};

use super::{insert_some, range_view, serialized};
use crate::agent::action::{
    AIAgentActionType, AIAgentPtyWriteMode, CommentSide, FileEdit, InsertReviewComment,
    RunAgentsExecutionMode, ShellCommandDelay, SuggestPromptRequest,
};
use crate::diff_validation::ParsedDiff;
use crate::hooks::model::HookEnvelope;

/// The `tool_input` object for `preToolUse` and the post-tool events: the action's arguments as
/// the model gave them. Shell commands carry the working directory beside the command, as
/// Cursor's example does.
pub(super) fn tool_input(action: &AIAgentActionType, envelope: &HookEnvelope) -> Value {
    match action {
        AIAgentActionType::RequestCommandOutput {
            command,
            is_read_only,
            is_risky,
            wait_until_completion,
            uses_pager,
            rationale,
            citations: _,
        } => {
            let mut fields = Map::new();
            fields.insert("command".to_owned(), json!(command));
            if let Some(cwd) = &envelope.cwd {
                fields.insert("cwd".to_owned(), json!(cwd.to_string_lossy()));
            }
            insert_some(&mut fields, "is_read_only", *is_read_only);
            insert_some(&mut fields, "is_risky", *is_risky);
            fields.insert(
                "wait_until_completion".to_owned(),
                json!(wait_until_completion),
            );
            insert_some(&mut fields, "uses_pager", *uses_pager);
            insert_some(&mut fields, "rationale", rationale.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::WriteToLongRunningShellCommand {
            block_id,
            input,
            mode,
        } => json!({
            "block_id": serialized("block_id", block_id),
            "input": String::from_utf8_lossy(input),
            "mode": match mode {
                AIAgentPtyWriteMode::Raw => "raw",
                AIAgentPtyWriteMode::Line => "line",
                AIAgentPtyWriteMode::Block => "block",
            },
        }),
        AIAgentActionType::ReadFiles(request) => json!({
            "files": request
                .locations
                .iter()
                .map(|location| json!({
                    "path": location.name,
                    "lines": location.lines.iter().map(range_view).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::UploadArtifact(request) => {
            let mut fields = Map::new();
            fields.insert("file_path".to_owned(), json!(request.file_path));
            insert_some(&mut fields, "description", request.description.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::SearchCodebase(request) => {
            let mut fields = Map::new();
            fields.insert("query".to_owned(), json!(request.query));
            insert_some(
                &mut fields,
                "partial_paths",
                request.partial_paths.as_deref(),
            );
            insert_some(
                &mut fields,
                "codebase_path",
                request.codebase_path.as_deref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::RequestFileEdits { file_edits, title } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "title", title.as_deref());
            fields.insert(
                "edits".to_owned(),
                Value::Array(file_edits.iter().map(file_edit_view).collect()),
            );
            Value::Object(fields)
        }
        AIAgentActionType::Grep { queries, path } => json!({ "queries": queries, "path": path }),
        AIAgentActionType::FileGlob { patterns, path } => {
            let mut fields = Map::new();
            fields.insert("patterns".to_owned(), json!(patterns));
            insert_some(&mut fields, "path", path.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::FileGlobV2 {
            patterns,
            search_dir,
        } => {
            let mut fields = Map::new();
            fields.insert("patterns".to_owned(), json!(patterns));
            insert_some(&mut fields, "search_dir", search_dir.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::ReadMCPResource {
            server_id,
            name,
            uri,
        } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "server_id", server_id.map(|id| id.to_string()));
            fields.insert("name".to_owned(), json!(name));
            insert_some(&mut fields, "uri", uri.as_deref());
            Value::Object(fields)
        }
        AIAgentActionType::CallMCPTool {
            server_id,
            name,
            input,
        } => {
            let mut fields = Map::new();
            insert_some(&mut fields, "server_id", server_id.map(|id| id.to_string()));
            fields.insert("name".to_owned(), json!(name));
            fields.insert("input".to_owned(), input.clone());
            Value::Object(fields)
        }
        AIAgentActionType::SuggestNewConversation { message_id } => {
            json!({ "message_id": message_id })
        }
        AIAgentActionType::SuggestPrompt(request) => match request {
            SuggestPromptRequest::UnitTestsSuggestion {
                query,
                title,
                description,
            } => json!({
                "kind": "unit_tests_suggestion",
                "query": query,
                "title": title,
                "description": description,
            }),
            SuggestPromptRequest::PromptSuggestion { prompt, label } => {
                let mut fields = Map::new();
                fields.insert("kind".to_owned(), json!("prompt_suggestion"));
                fields.insert("prompt".to_owned(), json!(prompt));
                insert_some(&mut fields, "label", label.as_deref());
                Value::Object(fields)
            }
        },
        AIAgentActionType::InitProject | AIAgentActionType::OpenCodeReview => json!({}),
        AIAgentActionType::ReadDocuments(request) => json!({
            "document_ids": request
                .document_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::EditDocuments(request) => json!({
            "diffs": request
                .diffs
                .iter()
                .map(|diff| json!({
                    "document_id": diff.document_id.to_string(),
                    "search": diff.search,
                    "replace": diff.replace,
                }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::CreateDocuments(request) => json!({
            "documents": request
                .documents
                .iter()
                .map(|document| json!({ "title": document.title, "content": document.content }))
                .collect::<Vec<_>>(),
        }),
        AIAgentActionType::ReadShellCommandOutput { block_id, delay } => {
            let mut fields = Map::new();
            fields.insert("block_id".to_owned(), serialized("block_id", block_id));
            let delay = delay.as_ref().map(|delay| match delay {
                ShellCommandDelay::Duration(duration) => {
                    json!({ "seconds": duration.as_secs_f64() })
                }
                ShellCommandDelay::OnCompletion => json!("on_completion"),
            });
            insert_some(&mut fields, "delay", delay);
            Value::Object(fields)
        }
        AIAgentActionType::UseComputer(request) => {
            let mut fields = Map::new();
            fields.insert("action_summary".to_owned(), json!(request.action_summary));
            fields.insert(
                "actions".to_owned(),
                serialized("actions", &request.actions),
            );
            insert_some(
                &mut fields,
                "screenshot_params",
                request.screenshot_params.as_ref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::InsertCodeReviewComments {
            repo_path,
            comments,
            base_branch,
        } => {
            let mut fields = Map::new();
            fields.insert("repo_path".to_owned(), json!(repo_path.to_string_lossy()));
            insert_some(&mut fields, "base_branch", base_branch.as_deref());
            fields.insert(
                "comments".to_owned(),
                Value::Array(comments.iter().map(review_comment_view).collect()),
            );
            Value::Object(fields)
        }
        AIAgentActionType::RequestComputerUse(request) => {
            let mut fields = Map::new();
            fields.insert("task_summary".to_owned(), json!(request.task_summary));
            insert_some(
                &mut fields,
                "screenshot_params",
                request.screenshot_params.as_ref(),
            );
            Value::Object(fields)
        }
        AIAgentActionType::StartRecording {
            frame_rate,
            max_duration,
            max_size_bytes,
            summary,
            description,
            playback_speed_multiplier,
            window,
        } => {
            let mut fields = Map::new();
            fields.insert("frame_rate".to_owned(), json!(frame_rate));
            insert_some(
                &mut fields,
                "max_duration_ms",
                max_duration.map(|duration| duration.as_millis() as u64),
            );
            insert_some(&mut fields, "max_size_bytes", *max_size_bytes);
            insert_some(&mut fields, "summary", summary.as_deref());
            insert_some(&mut fields, "description", description.as_deref());
            insert_some(
                &mut fields,
                "playback_speed_multiplier",
                *playback_speed_multiplier,
            );
            insert_some(&mut fields, "window", window.as_ref());
            Value::Object(fields)
        }
        AIAgentActionType::StopRecording {
            recording_id,
            should_persist,
        } => json!({ "recording_id": recording_id, "should_persist": should_persist }),
        AIAgentActionType::ReadSkill(request) => {
            json!({ "skill": serialized("skill", &request.skill) })
        }
        AIAgentActionType::FetchConversation { conversation_id } => {
            json!({ "conversation_id": conversation_id })
        }
        AIAgentActionType::SendMessageToAgent {
            addresses,
            subject,
            message,
        } => json!({ "addresses": addresses, "subject": subject, "message": message }),
        AIAgentActionType::TransferShellCommandControlToUser { reason } => {
            json!({ "reason": reason })
        }
        AIAgentActionType::AskUserQuestion { questions } => {
            json!({ "questions": serialized("questions", questions) })
        }
        AIAgentActionType::RunAgents(request) => {
            let execution_mode = match &request.execution_mode {
                RunAgentsExecutionMode::Local => json!("local"),
                RunAgentsExecutionMode::Remote {
                    environment_id,
                    worker_host,
                    computer_use_enabled,
                    runner_id,
                } => json!({
                    "remote": {
                        "environment_id": environment_id,
                        "worker_host": worker_host,
                        "computer_use_enabled": computer_use_enabled,
                        "runner_id": runner_id,
                    }
                }),
            };
            json!({
                "summary": request.summary,
                "base_prompt": request.base_prompt,
                "model_id": request.model_id,
                "harness_type": request.harness_type,
                "execution_mode": execution_mode,
                "plan_id": request.plan_id,
                "skills": serialized("skills", &request.skills),
                "metadata": request.metadata,
                "agents": request
                    .agent_run_configs
                    .iter()
                    .map(|config| json!({
                        "name": config.name,
                        "title": config.title,
                        "prompt": config.prompt,
                        "model_id": config.model_id,
                        "agent_identity_uid": config.agent_identity_uid,
                        "metadata": config.metadata,
                    }))
                    .collect::<Vec<_>>(),
            })
        }
        AIAgentActionType::WaitForEvents {
            tool_call_id,
            idle_timeout_seconds,
        } => json!({
            "tool_call_id": tool_call_id,
            "idle_timeout_seconds": idle_timeout_seconds,
        }),
    }
}

fn file_edit_view(edit: &FileEdit) -> Value {
    match edit {
        FileEdit::Edit(ParsedDiff::StrReplaceEdit {
            file,
            search,
            replace,
        }) => json!({ "kind": "edit", "file": file, "search": search, "replace": replace }),
        FileEdit::Edit(ParsedDiff::V4AEdit {
            file,
            move_to,
            hunks,
        }) => json!({
            "kind": "edit",
            "file": file,
            "move_to": move_to,
            "hunks": serialized("hunks", hunks),
        }),
        FileEdit::Create {
            file,
            content,
            allow_overwrite,
        } => json!({
            "kind": "create",
            "file": file,
            "content": content,
            "allow_overwrite": allow_overwrite,
        }),
        FileEdit::Delete { file } => json!({ "kind": "delete", "file": file }),
    }
}

fn review_comment_view(comment: &InsertReviewComment) -> Value {
    let mut fields = Map::new();
    fields.insert("comment_id".to_owned(), json!(comment.comment_id));
    fields.insert("author".to_owned(), json!(comment.author));
    fields.insert(
        "last_modified_timestamp".to_owned(),
        json!(comment.last_modified_timestamp),
    );
    fields.insert("comment_body".to_owned(), json!(comment.comment_body));
    insert_some(
        &mut fields,
        "parent_comment_id",
        comment.parent_comment_id.as_deref(),
    );
    insert_some(&mut fields, "html_url", comment.html_url.as_deref());
    if let Some(location) = &comment.comment_location {
        let mut location_fields = Map::new();
        location_fields.insert(
            "relative_file_path".to_owned(),
            json!(location.relative_file_path),
        );
        if let Some(line) = &location.line {
            let side = line.side.map(|side| match side {
                CommentSide::Right => "right",
                CommentSide::Left => "left",
            });
            let mut line_fields = Map::new();
            line_fields.insert(
                "comment_line_range".to_owned(),
                range_view(&line.comment_line_range),
            );
            line_fields.insert(
                "diff_hunk_line_range".to_owned(),
                range_view(&line.diff_hunk_line_range),
            );
            line_fields.insert("diff_hunk_text".to_owned(), json!(line.diff_hunk_text));
            insert_some(&mut line_fields, "side", side);
            location_fields.insert("line".to_owned(), Value::Object(line_fields));
        }
        fields.insert("location".to_owned(), Value::Object(location_fields));
    }
    Value::Object(fields)
}
