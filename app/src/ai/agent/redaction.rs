use std::sync::Arc;

use crate::ai::agent::{
    AIAgentActionResultType, AIAgentAttachment, AIAgentContext, AIAgentInput, AnyFileContent,
    AskUserQuestionAnswerItem, AskUserQuestionResult, BlockContext, PassiveSuggestionResultType,
    PassiveSuggestionTrigger, RequestCommandOutputResult, TransferShellCommandControlToUserResult,
};

use super::super::blocklist::block::secret_redaction::{
    find_secrets_in_text, SECRET_REDACTION_REPLACEMENT_CHARACTER,
};

/// Redact all detected secrets in-place within the given string.
pub(crate) fn redact_secrets(input: &mut String) {
    let mut secrets: Vec<_> = find_secrets_in_text(input)
        .into_iter()
        .map(|r| r.byte_range)
        .collect();
    // Replace from the end to preserve indices
    secrets.sort_by_key(|range| range.start);
    for range in secrets.into_iter().rev() {
        let replacement =
            SECRET_REDACTION_REPLACEMENT_CHARACTER.repeat(range.end.saturating_sub(range.start));
        input.replace_range(range.start..range.end, &replacement);
    }
}

/// Redact secrets in-place for all user-provided text fields inside the inputs that will be
/// sent to the server.
pub(crate) fn redact_inputs(inputs: &mut [AIAgentInput]) {
    for input in inputs.iter_mut() {
        match input {
            AIAgentInput::UserQuery {
                query,
                context,
                referenced_attachments,
                ..
            } => {
                redact_secrets(query);
                redact_context(Arc::make_mut(context));
                referenced_attachments
                    .values_mut()
                    .for_each(redact_attachment);
            }
            AIAgentInput::AutoCodeDiffQuery { query, context, .. } => {
                redact_secrets(query);
                redact_context(Arc::make_mut(context));
            }
            AIAgentInput::CreateNewProject { context, .. }
            | AIAgentInput::CloneRepository { context, .. }
            | AIAgentInput::ResumeConversation { context }
            | AIAgentInput::InitProjectRules { context, .. }
            | AIAgentInput::StartFromAmbientRunPrompt { context, .. } => {
                redact_context(Arc::make_mut(context));
            }
            AIAgentInput::SummarizeConversation { prompt, context } => {
                if let Some(p) = prompt {
                    redact_secrets(p);
                }
                redact_context(Arc::make_mut(context));
            }
            AIAgentInput::CreateEnvironment { context, .. } => {
                redact_context(Arc::make_mut(context));
            }
            AIAgentInput::TriggerPassiveSuggestion {
                context,
                attachments,
                trigger,
            } => {
                redact_context(Arc::make_mut(context));
                attachments.iter_mut().for_each(redact_attachment);
                if let PassiveSuggestionTrigger::ShellCommandCompleted(shell_trigger) = trigger {
                    redact_secrets(&mut shell_trigger.executed_shell_command.command);
                    redact_secrets(&mut shell_trigger.executed_shell_command.output);
                    for file in shell_trigger.relevant_files.iter_mut() {
                        if let AnyFileContent::StringContent(content) = &mut file.content {
                            redact_secrets(content);
                        }
                    }
                }
            }
            AIAgentInput::CodeReview {
                context,
                review_comments,
            } => {
                redact_context(Arc::make_mut(context));
                for comment in review_comments.comments.iter_mut() {
                    redact_secrets(&mut comment.content);
                    match &mut comment.target {
                        crate::code_review::comments::AttachedReviewCommentTarget::Line {
                            content,
                            ..
                        } => {
                            redact_secrets(&mut content.content);
                        }
                        crate::code_review::comments::AttachedReviewCommentTarget::File {
                            ..
                        }
                        | crate::code_review::comments::AttachedReviewCommentTarget::General => {}
                    }
                }

                for diff in review_comments.diff_set.values_mut().flatten() {
                    redact_secrets(&mut diff.diff_content);
                }
            }
            // No user-provided text to redact in inter-agent relay inputs.
            AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => {}
            AIAgentInput::ActionResult { result, context } => {
                redact_context(Arc::make_mut(context));
                match &mut result.result {
                    AIAgentActionResultType::RequestCommandOutput(output) => {
                        if let RequestCommandOutputResult::Completed { output, .. } = output {
                            redact_secrets(output);
                        }
                    }
                    AIAgentActionResultType::WriteToLongRunningShellCommand(result) => {
                        use crate::ai::agent::WriteToLongRunningShellCommandResult::*;
                        match result {
                            Snapshot { grid_contents, .. } => redact_secrets(grid_contents),
                            CommandFinished { output, .. } => redact_secrets(output),
                            Error(_) | Cancelled => {}
                        }
                    }
                    AIAgentActionResultType::ReadShellCommandOutput(result) => {
                        use crate::ai::agent::ReadShellCommandOutputResult::*;
                        match result {
                            CommandFinished { output, .. } => redact_secrets(output),
                            LongRunningCommandSnapshot { grid_contents, .. } => {
                                redact_secrets(grid_contents)
                            }
                            Error(_) | Cancelled => {}
                        }
                    }
                    AIAgentActionResultType::ReadFiles(read_files_result) => {
                        if let crate::ai::agent::ReadFilesResult::Success { files } =
                            read_files_result
                        {
                            for file in files {
                                if let AnyFileContent::StringContent(content) = &mut file.content {
                                    redact_secrets(content);
                                }
                            }
                        }
                    }
                    AIAgentActionResultType::UploadArtifact(upload_result) => {
                        use crate::ai::agent::UploadArtifactResult;
                        match upload_result {
                            UploadArtifactResult::Success {
                                filepath,
                                description,
                                ..
                            } => {
                                if let Some(filepath) = filepath {
                                    redact_secrets(filepath);
                                }
                                if let Some(description) = description {
                                    redact_secrets(description);
                                }
                            }
                            UploadArtifactResult::Error(error) => redact_secrets(error),
                            UploadArtifactResult::Cancelled => {}
                        }
                    }
                    AIAgentActionResultType::SearchCodebase(search_codebase_result) => {
                        if let crate::ai::agent::SearchCodebaseResult::Success { files } =
                            search_codebase_result
                        {
                            for file in files {
                                if let AnyFileContent::StringContent(content) = &mut file.content {
                                    redact_secrets(content);
                                }
                            }
                        }
                    }
                    AIAgentActionResultType::RequestFileEdits(request_file_edits_result) => {
                        if let crate::ai::agent::RequestFileEditsResult::Success {
                            diff,
                            updated_files,
                            deleted_files,
                            ..
                        } = request_file_edits_result
                        {
                            redact_secrets(diff);
                            for file in updated_files {
                                if let AnyFileContent::StringContent(content) =
                                    &mut file.file_context.content
                                {
                                    redact_secrets(content);
                                }
                            }
                            for file_path in deleted_files {
                                redact_secrets(file_path);
                            }
                        }
                    }
                    AIAgentActionResultType::InsertReviewComments(result) => {
                        use crate::ai::agent::InsertReviewCommentsResult::*;
                        match result {
                            Success { repo_path } => redact_secrets(repo_path),
                            Error { repo_path, message } => {
                                redact_secrets(repo_path);
                                redact_secrets(message);
                            }
                            Cancelled => {}
                        }
                    }

                    // These are effectively flow control and don't contain secrets
                    AIAgentActionResultType::SuggestNewConversation { .. }
                    | AIAgentActionResultType::OpenCodeReview
                    | AIAgentActionResultType::InitProject => {}

                    // Contains only file path/line number information
                    AIAgentActionResultType::Grep(_)
                    | AIAgentActionResultType::FileGlob(_)
                    | AIAgentActionResultType::FileGlobV2(_) => {}

                    // TODO: Redact MCP-related results
                    AIAgentActionResultType::CallMCPTool { .. }
                    | AIAgentActionResultType::ReadSkill { .. }
                    | AIAgentActionResultType::ReadMCPResource { .. }
                    | AIAgentActionResultType::SuggestPrompt { .. }
                    | AIAgentActionResultType::ReadDocuments(_)
                    | AIAgentActionResultType::EditDocuments(_)
                    | AIAgentActionResultType::CreateDocuments(_) => {}

                    // TODO(AGENT-2282): figure out whether there's any reasonable way to
                    // do redaction here (probably not).
                    AIAgentActionResultType::UseComputer(_) => {}

                    // Request computer use just contains screen dimensions, no secrets
                    AIAgentActionResultType::RequestComputerUse(_) => {}

                    // FetchConversation results contain tasks returned from the server,
                    // which were already redacted before being sent as client inputs.
                    // (client inputs -> redaction -> server request -> task messages)
                    AIAgentActionResultType::FetchConversation(_) => {}

                    // StartAgent results contain only an agent ID string, no secrets
                    AIAgentActionResultType::StartAgent(_) => {}

                    // SendMessageToAgent results contain only a message ID or error string, no secrets
                    AIAgentActionResultType::SendMessageToAgent(_) => {}
                    // TransferShellCommandControlToUser result - similar to WriteToLongRunningShellCommand
                    AIAgentActionResultType::TransferShellCommandControlToUser(result) => {
                        match result {
                            TransferShellCommandControlToUserResult::Snapshot {
                                grid_contents,
                                ..
                            } => redact_secrets(grid_contents),
                            TransferShellCommandControlToUserResult::CommandFinished {
                                output,
                                ..
                            } => redact_secrets(output),
                            TransferShellCommandControlToUserResult::Error(_)
                            | TransferShellCommandControlToUserResult::Cancelled => {}
                        }
                    }
                    AIAgentActionResultType::AskUserQuestion(result) => {
                        redact_ask_user_question_result(result);
                    }
                    // Orchestrate results contain agent IDs / canonical error
                    // strings only; no user-provided text to redact.
                    AIAgentActionResultType::RunAgents(_) => {}
                }
            }
            AIAgentInput::FetchReviewComments { repo_path, context } => {
                redact_secrets(repo_path);
                redact_context(Arc::make_mut(context));
            }
            AIAgentInput::InvokeSkill {
                context,
                skill,
                user_query,
            } => {
                redact_context(Arc::make_mut(context));
                redact_secrets(&mut skill.content);
                if let Some(user_query) = user_query {
                    redact_secrets(&mut user_query.query);
                    for attachment in user_query.referenced_attachments.values_mut() {
                        redact_attachment(attachment);
                    }
                }
            }
            AIAgentInput::PassiveSuggestionResult {
                trigger,
                suggestion,
                context,
            } => {
                redact_context(Arc::make_mut(context));
                match suggestion {
                    PassiveSuggestionResultType::Prompt { prompt } => redact_secrets(prompt),
                    PassiveSuggestionResultType::CodeDiff { diffs, .. } => {
                        for diff in diffs {
                            redact_secrets(&mut diff.file_path);
                            redact_secrets(&mut diff.search);
                            redact_secrets(&mut diff.replace);
                        }
                    }
                }
                if let Some(PassiveSuggestionTrigger::ShellCommandCompleted(shell_trigger)) =
                    trigger
                {
                    redact_secrets(&mut shell_trigger.executed_shell_command.command);
                    redact_secrets(&mut shell_trigger.executed_shell_command.output);
                    for file in shell_trigger.relevant_files.iter_mut() {
                        if let AnyFileContent::StringContent(content) = &mut file.content {
                            redact_secrets(content);
                        }
                    }
                }
            }
        }
    }
}

fn redact_ask_user_question_result(result: &mut AskUserQuestionResult) {
    match result {
        AskUserQuestionResult::Success { answers } => {
            for answer in answers {
                if let AskUserQuestionAnswerItem::Answered { other_text, .. } = answer {
                    redact_secrets(other_text);
                }
            }
        }
        AskUserQuestionResult::SkippedByAutoApprove { .. } => {}
        AskUserQuestionResult::Error(message) => redact_secrets(message),
        AskUserQuestionResult::Cancelled => {}
    }
}
fn redact_context(context: &mut [AIAgentContext]) {
    for context_item in context {
        match context_item {
            AIAgentContext::Block(context) => {
                redact_secrets(&mut context.command);
                redact_secrets(&mut context.output);
            }
            AIAgentContext::SelectedText(text) => {
                redact_secrets(text);
            }
            // Other context types don't contain user-provided text that needs redaction
            AIAgentContext::Directory { .. }
            | AIAgentContext::ExecutionEnvironment(_)
            | AIAgentContext::CurrentTime { .. }
            | AIAgentContext::Image(_)
            | AIAgentContext::Codebase { .. }
            | AIAgentContext::ProjectRules { .. }
            | AIAgentContext::Git { .. }
            | AIAgentContext::File(_)
            | AIAgentContext::Skills { .. } => {}
        }
    }
}

fn redact_attachment(attachment: &mut AIAgentAttachment) {
    match attachment {
        AIAgentAttachment::PlainText(text) => {
            redact_secrets(text);
        }
        AIAgentAttachment::Block(BlockContext {
            command, output, ..
        }) => {
            redact_secrets(command);
            redact_secrets(output);
        }
        AIAgentAttachment::DriveObject { payload, .. } => {
            if let Some(drive_payload) = payload {
                match drive_payload {
                    crate::ai::agent::DriveObjectPayload::Workflow {
                        name,
                        description,
                        command,
                    } => {
                        redact_secrets(name);
                        redact_secrets(description);
                        redact_secrets(command);
                    }
                    crate::ai::agent::DriveObjectPayload::Notebook { title, content } => {
                        redact_secrets(title);
                        redact_secrets(content);
                    }
                    crate::ai::agent::DriveObjectPayload::GenericStringObject {
                        payload, ..
                    } => {
                        redact_secrets(payload);
                    }
                }
            }
        }
        AIAgentAttachment::DiffHunk {
            file_path,
            diff_content,
            ..
        } => {
            redact_secrets(file_path);
            redact_secrets(diff_content);
        }
        AIAgentAttachment::DiffSet { file_diffs, .. } => {
            for hunks in file_diffs.values_mut() {
                for hunk in hunks {
                    redact_secrets(&mut hunk.diff_content);
                }
            }
        }
        AIAgentAttachment::DocumentContent { content, .. } => {
            redact_secrets(content);
        }
        // FilePathReference only contains a file ID and filename, no user secrets.
        AIAgentAttachment::FilePathReference { .. } => {}
    }
}
