use chrono::{DateTime, Local};
use warp_multi_agent_api::{
    self as api,
    apply_file_diffs_result::success::UpdatedFileContent,
    ask_user_question_result::answer_item::{self, Answer as AskUserQuestionAnswer},
};

use crate::agent::{action_result::ShellCommandError, convert::ConvertToAPITypeError};

use super::*;

fn local_datetime_to_timestamp(timestamp: DateTime<Local>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: timestamp.timestamp(),
        nanos: timestamp.timestamp_subsec_nanos() as i32,
    }
}

impl TryFrom<RequestCommandOutputResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: RequestCommandOutputResult) -> Result<Self, Self::Error> {
        match result {
            RequestCommandOutputResult::Completed {
                command,
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => Ok(
                api::request::input::tool_call_result::Result::RunShellCommand(
                    #[allow(deprecated)]
                    api::RunShellCommandResult {
                        command,
                        output: Default::default(),
                        exit_code: Default::default(),
                        result: Some(api::run_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                command_id: block_id.to_string(),
                                output,
                                exit_code: exit_code.value(),
                                start_ts: start_ts.map(local_datetime_to_timestamp),
                                finish_ts: completed_ts.map(local_datetime_to_timestamp),
                            },
                        )),
                    },
                ),
            ),
            RequestCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
            } => Ok(
                api::request::input::tool_call_result::Result::RunShellCommand(
                    #[allow(deprecated)]
                    api::RunShellCommandResult {
                        command,
                        output: Default::default(),
                        exit_code: Default::default(),
                        result: Some(
                            api::run_shell_command_result::Result::LongRunningCommandSnapshot(
                                api::LongRunningShellCommandSnapshot {
                                    command_id: block_id.to_string(),
                                    output: grid_contents,
                                    cursor: cursor.to_owned(),
                                    is_alt_screen_active,
                                    is_preempted: false,
                                },
                            ),
                        ),
                    },
                ),
            ),
            RequestCommandOutputResult::CancelledBeforeExecution => {
                Err(ConvertToAPITypeError::Ignore)
            }
            RequestCommandOutputResult::Denylisted { command } =>
            {
                #[allow(deprecated)]
                Ok(
                    api::request::input::tool_call_result::Result::RunShellCommand(
                        api::RunShellCommandResult {
                            command,
                            output: Default::default(),
                            exit_code: Default::default(),
                            result: Some(api::run_shell_command_result::Result::PermissionDenied(
                                api::PermissionDenied {
                                    reason: Some(
                                        api::permission_denied::Reason::DenylistedCommand(()),
                                    ),
                                },
                            )),
                        },
                    ),
                )
            }
        }
    }
}

impl TryFrom<WriteToLongRunningShellCommandResult>
    for api::request::input::tool_call_result::Result
{
    type Error = ConvertToAPITypeError;

    fn try_from(result: WriteToLongRunningShellCommandResult) -> Result<Self, Self::Error> {
        match result {
            WriteToLongRunningShellCommandResult::Snapshot { block_id, grid_contents, cursor, is_alt_screen_active, is_preempted } => Ok(
                api::request::input::tool_call_result::Result::WriteToLongRunningShellCommand(
                    api::WriteToLongRunningShellCommandResult {
                        result: Some(api::write_to_long_running_shell_command_result::Result::LongRunningCommandSnapshot(
                            api::LongRunningShellCommandSnapshot {
                                command_id: block_id.to_string(),
                                output: grid_contents,
                                cursor: cursor.to_owned(),
                                is_alt_screen_active,
                                is_preempted,
                            }
                        ))
                    },
                ),
            ),
            WriteToLongRunningShellCommandResult::CommandFinished { block_id, output, exit_code, start_ts, completed_ts } => Ok(
                api::request::input::tool_call_result::Result::WriteToLongRunningShellCommand(
                    api::WriteToLongRunningShellCommandResult {
                        result: Some(api::write_to_long_running_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                command_id: block_id.to_string(),
                                output,
                                exit_code: exit_code.value(),
                                start_ts: start_ts.map(local_datetime_to_timestamp),
                                finish_ts: completed_ts.map(local_datetime_to_timestamp),
                            }
                        ))
                    },
                ),
            ),
            WriteToLongRunningShellCommandResult::Cancelled =>
                Err(ConvertToAPITypeError::Ignore),
            WriteToLongRunningShellCommandResult::Error(ShellCommandError::BlockNotFound) => {
                Ok(api::request::input::tool_call_result::Result::WriteToLongRunningShellCommand(
                        api::WriteToLongRunningShellCommandResult {
                            result: Some(
                                api::write_to_long_running_shell_command_result::Result::Error(
                                    api::ShellCommandError {
                                        r#type: Some(api::shell_command_error::Type::CommandNotFound(())),
                                    },
                                ),
                            ),
                        },
                    ),
                )
            }
        }
    }
}

impl TryFrom<ReadFilesResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: ReadFilesResult) -> Result<Self, Self::Error> {
        match result {
            ReadFilesResult::Success { files } => Ok(
                api::request::input::tool_call_result::Result::ReadFiles(api::ReadFilesResult {
                    result: Some(api::read_files_result::Result::AnyFilesSuccess(
                        api::read_files_result::AnyFilesSuccess {
                            files: files
                                .into_iter()
                                .flat_map(Into::<Vec<api::AnyFileContent>>::into)
                                .collect(),
                        },
                    )),
                }),
            ),
            ReadFilesResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::ReadFiles(api::ReadFilesResult {
                    result: Some(api::read_files_result::Result::Error(
                        api::read_files_result::Error { message: error },
                    )),
                }),
            ),
            ReadFilesResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<UploadArtifactResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: UploadArtifactResult) -> Result<Self, Self::Error> {
        match result {
            UploadArtifactResult::Success {
                artifact_uid,
                mime_type,
                size_bytes,
                ..
            } => Ok(
                api::request::input::tool_call_result::Result::UploadFileArtifact(
                    api::UploadFileArtifactResult {
                        result: Some(api::upload_file_artifact_result::Result::Success(
                            api::upload_file_artifact_result::Success {
                                artifact_uid,
                                mime_type,
                                size_bytes,
                            },
                        )),
                    },
                ),
            ),
            UploadArtifactResult::Error(message) => Ok(
                api::request::input::tool_call_result::Result::UploadFileArtifact(
                    api::UploadFileArtifactResult {
                        result: Some(api::upload_file_artifact_result::Result::Error(
                            api::upload_file_artifact_result::Error { message },
                        )),
                    },
                ),
            ),
            UploadArtifactResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<SearchCodebaseResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: SearchCodebaseResult) -> Result<Self, Self::Error> {
        match result {
            SearchCodebaseResult::Success { files } => Ok(
                api::request::input::tool_call_result::Result::SearchCodebase(
                    api::SearchCodebaseResult {
                        result: Some(api::search_codebase_result::Result::Success(
                            api::search_codebase_result::Success {
                                files: files
                                    .into_iter()
                                    .flat_map(Into::<Vec<api::FileContent>>::into)
                                    .collect(),
                            },
                        )),
                    },
                ),
            ),
            SearchCodebaseResult::Failed { message, .. } => Ok(
                api::request::input::tool_call_result::Result::SearchCodebase(
                    api::SearchCodebaseResult {
                        result: Some(api::search_codebase_result::Result::Error(
                            api::search_codebase_result::Error { message },
                        )),
                    },
                ),
            ),
            SearchCodebaseResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<RequestFileEditsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: RequestFileEditsResult) -> Result<Self, Self::Error> {
        match result {
            RequestFileEditsResult::Success {
                updated_files,
                deleted_files,
                ..
            } => Ok(
                api::request::input::tool_call_result::Result::ApplyFileDiffs(
                    api::ApplyFileDiffsResult {
                        result: Some(api::apply_file_diffs_result::Result::Success(
                            api::apply_file_diffs_result::Success {
                                updated_files_v2: updated_files
                                    .into_iter()
                                    .flat_map(Into::<Vec<UpdatedFileContent>>::into)
                                    .collect(),
                                deleted_files: deleted_files
                                    .into_iter()
                                    .map(|file_path| {
                                        api::apply_file_diffs_result::success::DeletedFile {
                                            file_path,
                                        }
                                    })
                                    .collect(),
                                ..Default::default()
                            },
                        )),
                    },
                ),
            ),
            RequestFileEditsResult::DiffApplicationFailed { error } => Ok(
                api::request::input::tool_call_result::Result::ApplyFileDiffs(
                    api::ApplyFileDiffsResult {
                        result: Some(api::apply_file_diffs_result::Result::Error(
                            api::apply_file_diffs_result::Error { message: error },
                        )),
                    },
                ),
            ),
            RequestFileEditsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<SuggestNewConversationResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: SuggestNewConversationResult) -> Result<Self, Self::Error> {
        match result {
            SuggestNewConversationResult::Accepted { message_id } => Ok(
                api::request::input::tool_call_result::Result::SuggestNewConversation(
                    api::SuggestNewConversationResult {
                        result: Some(api::suggest_new_conversation_result::Result::Accepted(
                            api::suggest_new_conversation_result::Accepted { message_id },
                        )),
                    },
                ),
            ),
            SuggestNewConversationResult::Rejected => Ok(
                api::request::input::tool_call_result::Result::SuggestNewConversation(
                    api::SuggestNewConversationResult {
                        result: Some(api::suggest_new_conversation_result::Result::Rejected(
                            api::suggest_new_conversation_result::Rejected {},
                        )),
                    },
                ),
            ),
            SuggestNewConversationResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<SuggestPromptResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: SuggestPromptResult) -> Result<Self, Self::Error> {
        match result {
            SuggestPromptResult::Accepted { .. } => Ok(
                api::request::input::tool_call_result::Result::SuggestPrompt(
                    api::SuggestPromptResult {
                        result: Some(api::suggest_prompt_result::Result::Accepted(())),
                    },
                ),
            ),
            SuggestPromptResult::Cancelled => Ok(
                api::request::input::tool_call_result::Result::SuggestPrompt(
                    api::SuggestPromptResult {
                        result: Some(api::suggest_prompt_result::Result::Rejected(())),
                    },
                ),
            ),
        }
    }
}

impl TryFrom<GrepResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: GrepResult) -> Result<Self, Self::Error> {
        match result {
            GrepResult::Success { matched_files } => Ok(
                api::request::input::tool_call_result::Result::Grep(api::GrepResult {
                    result: Some(api::grep_result::Result::Success(
                        api::grep_result::Success {
                            matched_files: matched_files.into_iter().map(Into::into).collect(),
                        },
                    )),
                }),
            ),
            GrepResult::Error(error) => Ok(api::request::input::tool_call_result::Result::Grep(
                api::GrepResult {
                    result: Some(api::grep_result::Result::Error(api::grep_result::Error {
                        message: error,
                    })),
                },
            )),
            GrepResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<FileGlobResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: FileGlobResult) -> Result<Self, Self::Error> {
        match result {
            FileGlobResult::Success { matched_files } => Ok(
                api::request::input::tool_call_result::Result::FileGlob(api::FileGlobResult {
                    result: Some(api::file_glob_result::Result::Success(
                        api::file_glob_result::Success { matched_files },
                    )),
                }),
            ),
            FileGlobResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::FileGlob(api::FileGlobResult {
                    result: Some(api::file_glob_result::Result::Error(
                        api::file_glob_result::Error { message: error },
                    )),
                }),
            ),
            FileGlobResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<FileGlobV2Result> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: FileGlobV2Result) -> Result<Self, Self::Error> {
        match result {
            FileGlobV2Result::Success {
                matched_files,
                warnings,
            } => Ok(api::request::input::tool_call_result::Result::FileGlobV2(
                api::FileGlobV2Result {
                    result: Some(api::file_glob_v2_result::Result::Success(
                        api::file_glob_v2_result::Success {
                            matched_files: matched_files.into_iter().map(Into::into).collect(),
                            warnings: warnings.unwrap_or_default(),
                        },
                    )),
                },
            )),
            FileGlobV2Result::Error(error) => Ok(
                api::request::input::tool_call_result::Result::FileGlobV2(api::FileGlobV2Result {
                    result: Some(api::file_glob_v2_result::Result::Error(
                        api::file_glob_v2_result::Error { message: error },
                    )),
                }),
            ),
            FileGlobV2Result::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

// Convert FileGlobV2Result to FileGlobResult.
impl From<FileGlobV2Result> for FileGlobResult {
    fn from(value: FileGlobV2Result) -> Self {
        match value {
            FileGlobV2Result::Success {
                matched_files,
                warnings: _,
            } => FileGlobResult::Success {
                matched_files: matched_files
                    .into_iter()
                    .map(|matched_file| matched_file.file_path)
                    .join("\n"),
            },
            FileGlobV2Result::Error(e) => FileGlobResult::Error(e),
            FileGlobV2Result::Cancelled => FileGlobResult::Cancelled,
        }
    }
}

impl TryFrom<ReadMCPResourceResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: ReadMCPResourceResult) -> Result<Self, Self::Error> {
        match result {
            ReadMCPResourceResult::Success { resource_contents } => Ok(
                api::request::input::tool_call_result::Result::ReadMcpResource(
                    api::ReadMcpResourceResult {
                        result: Some(api::read_mcp_resource_result::Result::Success(
                            api::read_mcp_resource_result::Success {
                                contents: resource_contents
                                    .into_iter()
                                    .map(convert_mcp_resource_content)
                                    .collect(),
                            },
                        )),
                    },
                ),
            ),
            ReadMCPResourceResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::ReadMcpResource(
                    api::ReadMcpResourceResult {
                        result: Some(api::read_mcp_resource_result::Result::Error(
                            api::read_mcp_resource_result::Error { message: error },
                        )),
                    },
                ),
            ),
            ReadMCPResourceResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<CallMCPToolResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: CallMCPToolResult) -> Result<Self, Self::Error> {
        match result {
            CallMCPToolResult::Success { result } => {
                Ok(api::request::input::tool_call_result::Result::CallMcpTool(
                    api::CallMcpToolResult {
                        result: Some(convert_mcp_tool_call_result(result)),
                    },
                ))
            }
            CallMCPToolResult::Error(error) => {
                Ok(api::request::input::tool_call_result::Result::CallMcpTool(
                    api::CallMcpToolResult {
                        result: Some(api::call_mcp_tool_result::Result::Error(
                            api::call_mcp_tool_result::Error { message: error },
                        )),
                    },
                ))
            }
            CallMCPToolResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<ReadSkillResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: ReadSkillResult) -> Result<Self, Self::Error> {
        match result {
            ReadSkillResult::Success { content } => {
                let file_contents: Vec<api::FileContent> = content.into();

                // There should only be one file content

                if file_contents.len() != 1 {
                    return Err(ConvertToAPITypeError::Ignore);
                }

                Ok(api::request::input::tool_call_result::Result::ReadSkill(
                    api::ReadSkillResult {
                        result: Some(api::read_skill_result::Result::Success(
                            api::read_skill_result::Success {
                                content: Some(file_contents[0].clone()),
                            },
                        )),
                    },
                ))
            }
            ReadSkillResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::ReadSkill(api::ReadSkillResult {
                    result: Some(api::read_skill_result::Result::Error(
                        api::read_skill_result::Error { message: error },
                    )),
                }),
            ),
            ReadSkillResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<ReadDocumentsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: ReadDocumentsResult) -> Result<Self, Self::Error> {
        match result {
            ReadDocumentsResult::Success { documents } => {
                let docs: Vec<api::DocumentContent> = documents
                    .into_iter()
                    .flat_map(Into::<Vec<api::DocumentContent>>::into)
                    .collect();
                Ok(
                    api::request::input::tool_call_result::Result::ReadDocuments(
                        api::ReadDocumentsResult {
                            result: Some(api::read_documents_result::Result::Success(
                                api::read_documents_result::Success { documents: docs },
                            )),
                        },
                    ),
                )
            }
            ReadDocumentsResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::ReadDocuments(
                    api::ReadDocumentsResult {
                        result: Some(api::read_documents_result::Result::Error(
                            api::read_documents_result::Error { message: error },
                        )),
                    },
                ),
            ),
            ReadDocumentsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<EditDocumentsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: EditDocumentsResult) -> Result<Self, Self::Error> {
        match result {
            EditDocumentsResult::Success { updated_documents } => {
                let docs: Vec<api::DocumentContent> = updated_documents
                    .into_iter()
                    .flat_map(Into::<Vec<api::DocumentContent>>::into)
                    .collect();
                Ok(
                    api::request::input::tool_call_result::Result::EditDocuments(
                        api::EditDocumentsResult {
                            result: Some(api::edit_documents_result::Result::Success(
                                api::edit_documents_result::Success {
                                    updated_documents: docs,
                                },
                            )),
                        },
                    ),
                )
            }
            EditDocumentsResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::EditDocuments(
                    api::EditDocumentsResult {
                        result: Some(api::edit_documents_result::Result::Error(
                            api::edit_documents_result::Error { message: error },
                        )),
                    },
                ),
            ),
            EditDocumentsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<CreateDocumentsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: CreateDocumentsResult) -> Result<Self, Self::Error> {
        match result {
            CreateDocumentsResult::Success { created_documents } => {
                let docs: Vec<api::DocumentContent> = created_documents
                    .into_iter()
                    .flat_map(Into::<Vec<api::DocumentContent>>::into)
                    .collect();
                Ok(
                    api::request::input::tool_call_result::Result::CreateDocuments(
                        api::CreateDocumentsResult {
                            result: Some(api::create_documents_result::Result::Success(
                                api::create_documents_result::Success {
                                    created_documents: docs,
                                },
                            )),
                        },
                    ),
                )
            }
            CreateDocumentsResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::CreateDocuments(
                    api::CreateDocumentsResult {
                        result: Some(api::create_documents_result::Result::Error(
                            api::create_documents_result::Error { message: error },
                        )),
                    },
                ),
            ),
            CreateDocumentsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<ReadShellCommandOutputResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: ReadShellCommandOutputResult) -> Result<Self, Self::Error> {
        match result {
            ReadShellCommandOutputResult::CommandFinished {
                command,
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => Ok(
                api::request::input::tool_call_result::Result::ReadShellCommandOutput(
                    api::ReadShellCommandOutputResult {
                        command,
                        result: Some(api::read_shell_command_output_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                command_id: block_id.to_string(),
                                output,
                                exit_code: exit_code.value(),
                                start_ts: start_ts.map(local_datetime_to_timestamp),
                                finish_ts: completed_ts.map(local_datetime_to_timestamp),
                            },
                        )),
                    },
                ),
            ),
            ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                command,
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
            } => Ok(
                api::request::input::tool_call_result::Result::ReadShellCommandOutput(
                    api::ReadShellCommandOutputResult {
                        command,
                        result: Some(
                            api::read_shell_command_output_result::Result::LongRunningCommandSnapshot(
                                api::LongRunningShellCommandSnapshot {
                                    command_id: block_id.to_string(),
                                    output: grid_contents,
                                    cursor: cursor.to_owned(),
                                    is_alt_screen_active,
                                    is_preempted,
                                },
                            ),
                        ),
                    },
                ),
            ),
            ReadShellCommandOutputResult::Error(ShellCommandError::BlockNotFound) => {
                Ok(api::request::input::tool_call_result::Result::ReadShellCommandOutput(
                        api::ReadShellCommandOutputResult {
                            command: "".to_owned(),
                            result: Some(
                                api::read_shell_command_output_result::Result::Error(
                                    api::ShellCommandError {
                                        r#type: Some(api::shell_command_error::Type::CommandNotFound(())),
                                    },
                                ),
                            ),
                        },
                    ),
                )
            }
            _ => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<TransferShellCommandControlToUserResult>
    for api::request::input::tool_call_result::Result
{
    type Error = ConvertToAPITypeError;

    fn try_from(result: TransferShellCommandControlToUserResult) -> Result<Self, Self::Error> {
        match result {
            TransferShellCommandControlToUserResult::Snapshot {
                block_id,
                grid_contents,
                cursor,
                is_alt_screen_active,
                is_preempted,
            } => Ok(
                api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api::TransferShellCommandControlToUserResult {
                        result: Some(
                            api::transfer_shell_command_control_to_user_result::Result::LongRunningCommandSnapshot(
                                api::LongRunningShellCommandSnapshot {
                                    command_id: block_id.to_string(),
                                    output: grid_contents,
                                    cursor,
                                    is_alt_screen_active,
                                    is_preempted,
                                },
                            ),
                        ),
                    },
                ),
            ),
            TransferShellCommandControlToUserResult::CommandFinished {
                block_id,
                output,
                exit_code,
                start_ts,
                completed_ts,
            } => Ok(
                api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api::TransferShellCommandControlToUserResult {
                        result: Some(
                            api::transfer_shell_command_control_to_user_result::Result::CommandFinished(
                                api::ShellCommandFinished {
                                    command_id: block_id.to_string(),
                                    output,
                                    exit_code: exit_code.value(),
                                    start_ts: start_ts.map(local_datetime_to_timestamp),
                                    finish_ts: completed_ts.map(local_datetime_to_timestamp),
                                },
                            ),
                        ),
                    },
                ),
            ),
            TransferShellCommandControlToUserResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
            TransferShellCommandControlToUserResult::Error(ShellCommandError::BlockNotFound) => {
                Ok(api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                        api::TransferShellCommandControlToUserResult {
                            result: Some(
                                api::transfer_shell_command_control_to_user_result::Result::Error(
                                    api::ShellCommandError {
                                        r#type: Some(api::shell_command_error::Type::CommandNotFound(())),
                                    },
                                ),
                            ),
                        },
                    ),
                )
            }
        }
    }
}

impl From<FileContext> for Vec<api::FileContent> {
    fn from(context: FileContext) -> Self {
        match context.content.clone() {
            AnyFileContent::StringContent(content) => {
                vec![api::FileContent {
                    file_path: context.file_name.clone(),
                    content,
                    line_range: context.line_range.map(|range| api::FileContentLineRange {
                        start: range.start as u32,
                        end: range.end as u32,
                    }),
                }]
            }
            // Ignore any binary context since they can't be converted to FileContent
            AnyFileContent::BinaryContent(_content) => vec![],
        }
    }
}

impl From<FileContext> for Vec<api::AnyFileContent> {
    fn from(context: FileContext) -> Self {
        match context.content.clone() {
            AnyFileContent::StringContent(content) => {
                vec![api::AnyFileContent {
                    content: Some(api::any_file_content::Content::TextContent(
                        api::FileContent {
                            file_path: context.file_name.clone(),
                            content,
                            line_range: context.line_range.map(|range| api::FileContentLineRange {
                                start: range.start as u32,
                                end: range.end as u32,
                            }),
                        },
                    )),
                }]
            }
            AnyFileContent::BinaryContent(content) => {
                // Binary content: drop any line range and return binary content as-is.
                vec![api::AnyFileContent {
                    content: Some(api::any_file_content::Content::BinaryContent(
                        api::BinaryFileContent {
                            file_path: context.file_name.clone(),
                            data: content,
                        },
                    )),
                }]
            }
        }
    }
}

impl From<GrepLineMatch> for api::grep_result::success::grep_file_match::GrepLineMatch {
    fn from(value: GrepLineMatch) -> Self {
        api::grep_result::success::grep_file_match::GrepLineMatch {
            line_number: value.line_number as u32,
        }
    }
}

impl From<GrepFileMatch> for api::grep_result::success::GrepFileMatch {
    fn from(value: GrepFileMatch) -> Self {
        api::grep_result::success::GrepFileMatch {
            file_path: value.file_path,
            matched_lines: value.matched_lines.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<FileGlobV2Match> for api::file_glob_v2_result::success::FileGlobMatch {
    fn from(value: FileGlobV2Match) -> Self {
        api::file_glob_v2_result::success::FileGlobMatch {
            file_path: value.file_path,
        }
    }
}

impl From<DocumentContext> for Vec<api::DocumentContent> {
    fn from(context: DocumentContext) -> Self {
        let content = context.content.clone();
        if context.line_ranges.is_empty() {
            return vec![api::DocumentContent {
                document_id: context.document_id.to_string(),
                content,
                line_range: None,
            }];
        }

        let lines: Vec<_> = content.lines().collect();
        context
            .line_ranges
            .iter()
            .filter_map(|range| {
                let start = range.start.saturating_sub(1).min(lines.len());
                let end = range.end.min(lines.len());
                if start >= end {
                    None
                } else {
                    let fragment = lines[start..end].join("\n");
                    Some(api::DocumentContent {
                        document_id: context.document_id.to_string(),
                        content: fragment,
                        line_range: Some(api::FileContentLineRange {
                            start: range.start as u32,
                            end: range.end as u32,
                        }),
                    })
                }
            })
            .collect()
    }
}

fn convert_mcp_resource_content(val: rmcp::model::ResourceContents) -> api::McpResourceContent {
    use api::mcp_resource_content::*;
    match val {
        rmcp::model::ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => api::McpResourceContent {
            uri,
            content_type: Some(ContentType::Text(Text {
                content: text,
                mime_type: mime_type.unwrap_or_default(),
            })),
        },
        rmcp::model::ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => api::McpResourceContent {
            uri,
            content_type: Some(ContentType::Binary(Binary {
                data: blob.into_bytes(),
                mime_type: mime_type.unwrap_or_default(),
            })),
        },
    }
}

impl From<CreateDocumentsResult> for AIAgentActionResultType {
    fn from(result: CreateDocumentsResult) -> Self {
        AIAgentActionResultType::CreateDocuments(result)
    }
}

impl From<EditDocumentsResult> for AIAgentActionResultType {
    fn from(result: EditDocumentsResult) -> Self {
        AIAgentActionResultType::EditDocuments(result)
    }
}

impl From<ReadDocumentsResult> for AIAgentActionResultType {
    fn from(result: ReadDocumentsResult) -> Self {
        AIAgentActionResultType::ReadDocuments(result)
    }
}

impl From<ReadSkillResult> for AIAgentActionResultType {
    fn from(result: ReadSkillResult) -> Self {
        AIAgentActionResultType::ReadSkill(result)
    }
}

impl TryFrom<RequestComputerUseResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: RequestComputerUseResult) -> Result<Self, Self::Error> {
        match result {
            RequestComputerUseResult::Approved {
                screenshot,
                platform,
            } => Ok(
                api::request::input::tool_call_result::Result::RequestComputerUse(
                    api::RequestComputerUseResult {
                        result: Some(api::request_computer_use_result::Result::Approved(
                            api::request_computer_use_result::Approved {
                                screen_dimensions: Some(api::ScreenDimensions {
                                    width_px: screenshot.original_width as i32,
                                    height_px: screenshot.original_height as i32,
                                }),
                                initial_screenshot: Some(api::RawImage {
                                    data: screenshot.data,
                                    mime_type: screenshot.mime_type.to_string(),
                                    width: screenshot.width as i32,
                                    height: screenshot.height as i32,
                                }),
                                platform: convert_platform(platform).into(),
                            },
                        )),
                    },
                ),
            ),
            RequestComputerUseResult::Cancelled => Ok(
                api::request::input::tool_call_result::Result::RequestComputerUse(
                    api::RequestComputerUseResult {
                        result: Some(api::request_computer_use_result::Result::Rejected(
                            api::request_computer_use_result::Rejected {},
                        )),
                    },
                ),
            ),
            RequestComputerUseResult::Error(error) => Ok(
                api::request::input::tool_call_result::Result::RequestComputerUse(
                    api::RequestComputerUseResult {
                        result: Some(api::request_computer_use_result::Result::Error(
                            api::request_computer_use_result::Error { message: error },
                        )),
                    },
                ),
            ),
        }
    }
}

impl TryFrom<UseComputerResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: UseComputerResult) -> Result<Self, Self::Error> {
        match result {
            UseComputerResult::Success(result) => {
                Ok(api::request::input::tool_call_result::Result::UseComputer(
                    api::UseComputerResult {
                        result: Some(api::use_computer_result::Result::Success(
                            api::use_computer_result::Success {
                                screenshot: result.screenshot.map(|s| api::RawImage {
                                    data: s.data,
                                    mime_type: s.mime_type.to_string(),
                                    width: s.width as i32,
                                    height: s.height as i32,
                                }),
                                cursor_position: result.cursor_position.map(vec_to_coordinates),
                            },
                        )),
                    },
                ))
            }
            UseComputerResult::Error(error) => {
                Ok(api::request::input::tool_call_result::Result::UseComputer(
                    api::UseComputerResult {
                        result: Some(api::use_computer_result::Result::Error(
                            api::use_computer_result::Error { message: error },
                        )),
                    },
                ))
            }
            UseComputerResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

fn vec_to_coordinates(vec: computer_use::Vector2I) -> api::Coordinates {
    api::Coordinates {
        x: vec.x(),
        y: vec.y(),
    }
}

fn convert_platform(
    platform: computer_use::Platform,
) -> api::request_computer_use_result::approved::Platform {
    use api::request_computer_use_result::approved::Platform;
    match platform {
        computer_use::Platform::Mac => Platform::Macos,
        computer_use::Platform::Windows => Platform::Windows,
        computer_use::Platform::LinuxX11 => Platform::LinuxX11,
        computer_use::Platform::LinuxWayland => Platform::LinuxWayland,
    }
}

fn convert_mcp_tool_call_result(
    val: rmcp::model::CallToolResult,
) -> api::call_mcp_tool_result::Result {
    if val.is_error.unwrap_or_default() {
        return api::call_mcp_tool_result::Result::Error(api::call_mcp_tool_result::Error {
            message: val
                .structured_content
                .map(|content| content.to_string())
                .unwrap_or_default(),
        });
    }

    use api::call_mcp_tool_result::success::{self, result};
    api::call_mcp_tool_result::Result::Success(api::call_mcp_tool_result::Success {
        results: val
            .content
            .into_iter()
            .filter_map(|content| {
                use rmcp::model::RawContent::*;
                match content.raw {
                    Text(raw_text_content) => Some(result::Result::Text(result::Text {
                        text: raw_text_content.text,
                    })),
                    Image(raw_image_content) => Some(result::Result::Image(result::Image {
                        data: raw_image_content.data.into_bytes(),
                        mime_type: raw_image_content.mime_type,
                    })),
                    Resource(raw_embedded_resource) => Some(result::Result::Resource(
                        convert_mcp_resource_content(raw_embedded_resource.resource),
                    )),
                    Audio(_) => {
                        log::warn!("Audio content not supported");
                        None
                    }
                    ResourceLink(_) => {
                        log::warn!("Resource link content not supported");
                        None
                    }
                }
            })
            .map(|result| success::Result {
                result: Some(result),
            })
            .collect(),
    })
}

impl TryFrom<FetchConversationResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: FetchConversationResult) -> Result<Self, Self::Error> {
        match result {
            FetchConversationResult::Success { directory_path } => Ok(
                api::request::input::tool_call_result::Result::FetchConversation(
                    api::FetchConversationResult {
                        result: Some(api::fetch_conversation_result::Result::Success(
                            api::fetch_conversation_result::Success { directory_path },
                        )),
                    },
                ),
            ),
            FetchConversationResult::Error(message) => Ok(
                api::request::input::tool_call_result::Result::FetchConversation(
                    api::FetchConversationResult {
                        result: Some(api::fetch_conversation_result::Result::Error(
                            api::fetch_conversation_result::Error { message },
                        )),
                    },
                ),
            ),
            FetchConversationResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl From<StartAgentResult> for api::request::input::tool_call_result::Result {
    fn from(result: StartAgentResult) -> Self {
        match result {
            StartAgentResult::Success {
                agent_id,
                version: StartAgentVersion::V1,
            } => api::request::input::tool_call_result::Result::StartAgent(api::StartAgentResult {
                result: Some(api::start_agent_result::Result::Success(
                    api::start_agent_result::Success { agent_id },
                )),
            }),
            StartAgentResult::Error {
                error,
                version: StartAgentVersion::V1,
            } => api::request::input::tool_call_result::Result::StartAgent(api::StartAgentResult {
                result: Some(api::start_agent_result::Result::Error(
                    api::start_agent_result::Error { error },
                )),
            }),
            StartAgentResult::Cancelled {
                version: StartAgentVersion::V1,
            } => api::request::input::tool_call_result::Result::StartAgent(api::StartAgentResult {
                result: Some(api::start_agent_result::Result::Error(
                    api::start_agent_result::Error {
                        error: "Cancelled by user".to_string(),
                    },
                )),
            }),
            // The remaining arms translate the v2 result schema back into the shared client
            // StartAgentResult so downstream UI/rendering code can stay version-agnostic.
            StartAgentResult::Success {
                agent_id,
                version: StartAgentVersion::V2,
            } => api::request::input::tool_call_result::Result::StartAgentV2(
                api::StartAgentV2Result {
                    result: Some(api::start_agent_v2_result::Result::Success(
                        api::start_agent_v2_result::Success { agent_id },
                    )),
                },
            ),
            StartAgentResult::Error {
                error,
                version: StartAgentVersion::V2,
            } => api::request::input::tool_call_result::Result::StartAgentV2(
                api::StartAgentV2Result {
                    result: Some(api::start_agent_v2_result::Result::Error(
                        api::start_agent_v2_result::Error { error },
                    )),
                },
            ),
            StartAgentResult::Cancelled {
                version: StartAgentVersion::V2,
            } => api::request::input::tool_call_result::Result::StartAgentV2(
                api::StartAgentV2Result {
                    result: Some(api::start_agent_v2_result::Result::Error(
                        api::start_agent_v2_result::Error {
                            error: "Cancelled by user".to_string(),
                        },
                    )),
                },
            ),
        }
    }
}

impl From<SendMessageToAgentResult> for api::request::input::tool_call_result::Result {
    fn from(result: SendMessageToAgentResult) -> Self {
        api::request::input::tool_call_result::Result::SendMessageToAgent(
            api::SendMessageToAgentResult {
                result: match result {
                    SendMessageToAgentResult::Success { message_id } => {
                        Some(api::send_message_to_agent_result::Result::Success(
                            api::send_message_to_agent_result::Success { message_id },
                        ))
                    }
                    SendMessageToAgentResult::Error(error) => {
                        Some(api::send_message_to_agent_result::Result::Error(
                            api::send_message_to_agent_result::Error { message: error },
                        ))
                    }
                    SendMessageToAgentResult::Cancelled => {
                        Some(api::send_message_to_agent_result::Result::Error(
                            api::send_message_to_agent_result::Error {
                                message: "Cancelled by user".to_string(),
                            },
                        ))
                    }
                },
            },
        )
    }
}

impl From<AskUserQuestionAnswerItem> for api::ask_user_question_result::AnswerItem {
    fn from(item: AskUserQuestionAnswerItem) -> Self {
        match item {
            AskUserQuestionAnswerItem::Answered {
                question_id,
                selected_options,
                other_text,
            } => api::ask_user_question_result::AnswerItem {
                question_id,
                answer: Some(AskUserQuestionAnswer::MultipleChoice(
                    answer_item::MultipleChoiceAnswer {
                        selected_options,
                        other_text,
                    },
                )),
            },
            AskUserQuestionAnswerItem::Skipped { question_id } => {
                api::ask_user_question_result::AnswerItem {
                    question_id,
                    answer: Some(AskUserQuestionAnswer::Skipped(())),
                }
            }
        }
    }
}

impl From<AskUserQuestionResult> for api::request::input::tool_call_result::Result {
    fn from(result: AskUserQuestionResult) -> Self {
        let api_result = match result {
            AskUserQuestionResult::Success { answers } => {
                let api_answers = answers.into_iter().map(Into::into).collect();
                Some(api::ask_user_question_result::Result::Success(
                    api::ask_user_question_result::Success {
                        answers: api_answers,
                    },
                ))
            }
            AskUserQuestionResult::SkippedByAutoApprove { question_ids } => {
                let api_answers = question_ids
                    .into_iter()
                    .map(|question_id| api::ask_user_question_result::AnswerItem {
                        question_id,
                        answer: Some(AskUserQuestionAnswer::Skipped(())),
                    })
                    .collect();
                Some(api::ask_user_question_result::Result::Success(
                    api::ask_user_question_result::Success {
                        answers: api_answers,
                    },
                ))
            }
            AskUserQuestionResult::Error(message) => {
                Some(api::ask_user_question_result::Result::Error(
                    api::ask_user_question_result::Error { message },
                ))
            }
            AskUserQuestionResult::Cancelled => Some(api::ask_user_question_result::Result::Error(
                api::ask_user_question_result::Error {
                    message: "Cancelled by user".to_string(),
                },
            )),
        };
        api::request::input::tool_call_result::Result::AskUserQuestion(api::AskUserQuestionResult {
            result: api_result,
        })
    }
}

impl From<RunAgentsLaunchedExecutionMode>
    for api::run_agents_result::launched::ResolvedExecutionMode
{
    fn from(mode: RunAgentsLaunchedExecutionMode) -> Self {
        match mode {
            RunAgentsLaunchedExecutionMode::Local => {
                api::run_agents_result::launched::ResolvedExecutionMode::Local(
                    api::run_agents::Local {},
                )
            }
            RunAgentsLaunchedExecutionMode::Remote {
                environment_id,
                worker_host,
                computer_use_enabled,
            } => api::run_agents_result::launched::ResolvedExecutionMode::Remote(
                api::run_agents::Remote {
                    environment_id,
                    worker_host,
                    computer_use_enabled,
                },
            ),
        }
    }
}

impl From<RunAgentsAgentOutcome> for api::run_agents_result::AgentOutcome {
    fn from(outcome: RunAgentsAgentOutcome) -> Self {
        let result = match outcome.kind {
            RunAgentsAgentOutcomeKind::Launched { agent_id } => {
                api::run_agents_result::agent_outcome::Result::Launched(
                    api::run_agents_result::LaunchedAgent { agent_id },
                )
            }
            RunAgentsAgentOutcomeKind::Failed { error } => {
                api::run_agents_result::agent_outcome::Result::Failed(
                    api::run_agents_result::FailedAgent { error },
                )
            }
        };
        api::run_agents_result::AgentOutcome {
            name: outcome.name,
            result: Some(result),
        }
    }
}

/// Maps a client-side harness string identifier (e.g. "oz", "claude")
/// to the new proto `Harness` oneof. Returns `None` for empty,
/// unrecognized, or `"unknown"` strings; callers leave
/// `resolved_harness` unset in that case.
pub(super) fn build_api_harness(harness_type: &str) -> Option<api::Harness> {
    let normalized = harness_type.trim().to_ascii_lowercase().replace('_', "-");
    let variant = match normalized.as_str() {
        "oz" => api::harness::Variant::Oz(api::harness::Oz {}),
        "claude" | "claude-code" => api::harness::Variant::ClaudeCode(api::harness::ClaudeCode {}),
        "opencode" | "open-code" => api::harness::Variant::OpenCode(api::harness::OpenCode {}),
        "gemini" => api::harness::Variant::Gemini(api::harness::Gemini {}),
        "codex" => api::harness::Variant::Codex(api::harness::Codex {}),
        _ => return None,
    };
    Some(api::Harness {
        variant: Some(variant),
    })
}

impl TryFrom<RunAgentsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: RunAgentsResult) -> Result<Self, Self::Error> {
        match result {
            RunAgentsResult::Launched {
                model_id,
                harness_type,
                execution_mode,
                agents,
            } => Ok(
                api::request::input::tool_call_result::Result::RunAgentsResult(
                    api::RunAgentsResult {
                        outcome: Some(api::run_agents_result::Outcome::Launched(
                            api::run_agents_result::Launched {
                                resolved_model_id: model_id,
                                resolved_harness: build_api_harness(&harness_type),
                                resolved_execution_mode: Some(execution_mode.into()),
                                agents: agents.into_iter().map(Into::into).collect(),
                            },
                        )),
                    },
                ),
            ),
            RunAgentsResult::Denied { reason } => Ok(
                api::request::input::tool_call_result::Result::RunAgentsResult(
                    api::RunAgentsResult {
                        outcome: Some(api::run_agents_result::Outcome::Denied(
                            api::run_agents_result::Denied { reason },
                        )),
                    },
                ),
            ),
            RunAgentsResult::Failure { error } => Ok(
                api::request::input::tool_call_result::Result::RunAgentsResult(
                    api::RunAgentsResult {
                        outcome: Some(api::run_agents_result::Outcome::Failure(
                            api::run_agents_result::Failure { error },
                        )),
                    },
                ),
            ),
            // Reject is conveyed by the generic ToolCallResult.Cancel marker
            // synthesized server-side on the next user input; nothing for the
            // client to send on the wire here.
            RunAgentsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

impl TryFrom<InsertReviewCommentsResult> for api::request::input::tool_call_result::Result {
    type Error = ConvertToAPITypeError;

    fn try_from(result: InsertReviewCommentsResult) -> Result<Self, Self::Error> {
        match result {
            InsertReviewCommentsResult::Success { repo_path } => Ok(
                api::request::input::tool_call_result::Result::InsertReviewComments(
                    api::InsertReviewCommentsResult {
                        repo_path,
                        result: Some(api::insert_review_comments_result::Result::Success(
                            api::insert_review_comments_result::Success {},
                        )),
                    },
                ),
            ),
            InsertReviewCommentsResult::Error { repo_path, message } => Ok(
                api::request::input::tool_call_result::Result::InsertReviewComments(
                    api::InsertReviewCommentsResult {
                        repo_path,
                        result: Some(api::insert_review_comments_result::Result::Error(
                            api::insert_review_comments_result::Error { message },
                        )),
                    },
                ),
            ),
            InsertReviewCommentsResult::Cancelled => Err(ConvertToAPITypeError::Ignore),
        }
    }
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
