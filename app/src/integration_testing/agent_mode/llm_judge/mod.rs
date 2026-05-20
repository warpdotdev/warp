pub mod llm_generate;

use anyhow::Result;
use llm_generate::LLMGenerateRequest;
use reqwest::blocking::Client;
use serde::Deserialize;
use warp_multi_agent_api::{
    apply_file_diffs_result::success::UpdatedFileContent, message, Message,
};

use crate::ai::agent::conversation::AIConversation;

#[derive(Clone, Debug)]
pub struct LLMJudgeConfig {
    pub prompt: String,
    pub model: &'static str,
}

/// The LLM judge's response content will be directly unmarshalled into this struct.
/// Each judge's prompt must instruct the LLM to output its response in this format.
#[derive(Debug, Deserialize)]
pub struct LLMJudgeResult {
    pub pass: bool,
    pub critique: String,
}

pub struct LLMJudge {
    config: LLMJudgeConfig,
    client: Client,
}

impl LLMJudge {
    pub fn new(config: LLMJudgeConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    /// Format the conversation history to pass to the LLM judge.
    /// TODO: support optional different formatting for different judges
    fn format_conversation_history_for_llm_judge(conversation: &AIConversation) -> Result<String> {
        let messages = conversation.all_linearized_messages();

        // Filter out tool call result contents from the task's messages.
        let filtered_messages = filter_tool_call_results_from_messages(messages.into_iter());

        // Use debug format for now since we can't serialize protos to JSON.
        let task_json = format!("{filtered_messages:?}");

        Ok(task_json)
    }

    pub fn judge(&self, conversation: &AIConversation) -> Result<LLMJudgeResult> {
        let formatted_conversation_history =
            Self::format_conversation_history_for_llm_judge(conversation)?;
        let request = LLMGenerateRequest {
            prompt: self.config.prompt.to_owned(),
            user_messages: vec![formatted_conversation_history],
            model_id: self.config.model.to_owned(),
        };

        let response = llm_generate::generate_llm_response(&self.client, request)?;

        // Parse the response content as JSON
        let result: LLMJudgeResult = serde_json::from_str(&response.content)?;
        Ok(result)
    }
}

/// Filter out tool call result contents while preserving structure and success/error status
pub fn filter_tool_call_result(result: &message::ToolCallResult) -> message::ToolCallResult {
    use message::tool_call_result::Result as ToolResult;
    use warp_multi_agent_api::*;

    let filtered_result = match &result.result {
        Some(ToolResult::RunShellCommand(cmd_result)) =>
        {
            #[allow(deprecated)]
            Some(ToolResult::RunShellCommand(RunShellCommandResult {
                command: cmd_result.command.clone(),
                output: Default::default(),
                exit_code: Default::default(),
                result: Some(
                    warp_multi_agent_api::run_shell_command_result::Result::CommandFinished(
                        warp_multi_agent_api::ShellCommandFinished {
                            command_id: "command_id".to_string(),
                            output: "[OUTPUT OMITTED]".to_string(),
                            exit_code: cmd_result.exit_code,
                            start_ts: None,
                            finish_ts: None,
                        },
                    ),
                ),
            }))
        }
        Some(ToolResult::ReadFiles(read_result)) => {
            use read_files_result::Result as ReadResult;
            let filtered_read_result = match &read_result.result {
                Some(ReadResult::TextFilesSuccess(success)) => {
                    // Keep file paths and line ranges but remove content
                    let filtered_files = success
                        .files
                        .iter()
                        .map(|file| FileContent {
                            file_path: file.file_path.clone(),
                            line_range: file.line_range,
                            content: "[CONTENT OMITTED]".to_string(),
                        })
                        .collect();

                    Some(ReadResult::TextFilesSuccess(
                        read_files_result::TextFilesSuccess {
                            files: filtered_files,
                        },
                    ))
                }
                Some(ReadResult::AnyFilesSuccess(success)) => {
                    // Keep file paths and line ranges but remove content
                    let filtered_files = success
                        .files
                        .iter()
                        .map(|file| match &file.content {
                            Some(any_file_content::Content::TextContent(text_content)) => {
                                AnyFileContent {
                                    content: Some(any_file_content::Content::TextContent(
                                        warp_multi_agent_api::FileContent {
                                            file_path: text_content.file_path.clone(),
                                            content: "[CONTENT OMITTED]".to_string(),
                                            line_range: text_content.line_range,
                                        },
                                    )),
                                }
                            }
                            Some(any_file_content::Content::BinaryContent(binary_content)) => {
                                AnyFileContent {
                                    content: Some(any_file_content::Content::BinaryContent(
                                        warp_multi_agent_api::BinaryFileContent {
                                            file_path: binary_content.file_path.clone(),
                                            data: vec![],
                                        },
                                    )),
                                }
                            }
                            None => unreachable!(
                                "AnyFileContent should always contain TextContent or BinaryContent"
                            ),
                        })
                        .collect();
                    Some(ReadResult::AnyFilesSuccess(
                        read_files_result::AnyFilesSuccess {
                            files: filtered_files,
                        },
                    ))
                }
                Some(ReadResult::Error(err)) => Some(ReadResult::Error(err.clone())),
                None => None,
            };
            Some(ToolResult::ReadFiles(ReadFilesResult {
                result: filtered_read_result,
            }))
        }
        Some(ToolResult::SearchCodebase(search_result)) => {
            use search_codebase_result::Result as SearchResult;
            let filtered_search_result = match &search_result.result {
                Some(SearchResult::Success(success)) => {
                    // Keep file paths and line ranges but remove file contents
                    let filtered_files = success
                        .files
                        .iter()
                        .map(|file| FileContent {
                            file_path: file.file_path.clone(),
                            line_range: file.line_range,
                            content: "[CONTENT OMITTED]".to_string(),
                        })
                        .collect();

                    Some(SearchResult::Success(search_codebase_result::Success {
                        files: filtered_files,
                    }))
                }
                Some(SearchResult::Error(err)) => Some(SearchResult::Error(err.clone())),
                None => None,
            };
            Some(ToolResult::SearchCodebase(SearchCodebaseResult {
                result: filtered_search_result,
            }))
        }
        Some(ToolResult::ApplyFileDiffs(diff_result)) => {
            use apply_file_diffs_result::Result as DiffResult;
            let filtered_diff_result = match &diff_result.result {
                Some(DiffResult::Success(success)) => {
                    // Keep file paths and line ranges but remove file contents
                    let filtered_files = success
                        .updated_files_v2
                        .iter()
                        .map(|file| UpdatedFileContent {
                            file: file.file.as_ref().map(|file_content| FileContent {
                                file_path: file_content.file_path.clone(),
                                line_range: file_content.line_range,
                                content: "[CONTENT OMITTED]".to_string(),
                            }),
                            was_edited_by_user: file.was_edited_by_user,
                        })
                        .collect();

                    Some(DiffResult::Success(apply_file_diffs_result::Success {
                        updated_files_v2: filtered_files,
                        ..Default::default()
                    }))
                }
                Some(DiffResult::Error(err)) => Some(DiffResult::Error(err.clone())),
                None => None,
            };
            Some(ToolResult::ApplyFileDiffs(ApplyFileDiffsResult {
                result: filtered_diff_result,
            }))
        }
        Some(ToolResult::Grep(grep_result)) => {
            use grep_result::Result as GrepResult;
            let filtered_grep_result = match &grep_result.result {
                Some(GrepResult::Success(success)) => {
                    // Keep only file paths, remove all matched content and line numbers
                    let filtered_files = success
                        .matched_files
                        .iter()
                        .map(|file| {
                            grep_result::success::GrepFileMatch {
                                file_path: file.file_path.clone(),
                                // These can be very long so they're filtered, but we can't replace with a placeholder.
                                // The prompt should address this.
                                matched_lines: vec![],
                            }
                        })
                        .collect();

                    Some(GrepResult::Success(grep_result::Success {
                        matched_files: filtered_files,
                    }))
                }
                Some(GrepResult::Error(err)) => Some(GrepResult::Error(err.clone())),
                None => None,
            };
            Some(ToolResult::Grep(warp_multi_agent_api::GrepResult {
                result: filtered_grep_result,
            }))
        }
        other => other.clone(),
    };

    message::ToolCallResult {
        tool_call_id: result.tool_call_id.clone(),
        context: None, // Remove context to reduce noise
        result: filtered_result,
    }
}

/// Filter out tool call result contents while preserving structure
pub fn filter_tool_call_results_from_messages<'a>(
    messages: impl Iterator<Item = &'a Message>,
) -> Vec<Message> {
    messages
        .map(|msg| {
            let mut filtered_msg = msg.clone();
            if let Some(message::Message::ToolCallResult(ref tool_result)) = &msg.message {
                filtered_msg.message = Some(message::Message::ToolCallResult(
                    filter_tool_call_result(tool_result),
                ));
            }
            filtered_msg
        })
        .collect()
}
