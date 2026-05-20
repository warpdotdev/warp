//! Conversions from MAA API types to application types, for loading and restoring conversations.
//! Contains logic necessary for converting proto tasks to application exchanges and conversations.
//! Whenever adding new MAA types, conversions here must be updated for restoration and loading to work.
//! If some UI state is stored in the client, it needs to also be represented in the proto tasks somehow so it can be restored.
//! Some conversions may be lossy if it's not important to recover that UI state.

use crate::ai::agent::api::convert_from::{
    convert_user_query_mode, ConversionParams, ConvertAPIMessageToClientOutputMessage,
    MaybeAIAgentOutputMessage,
};
use crate::ai::agent::conversation::update_todo_list_from_todo_op;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentContext, AIAgentExchange,
    AIAgentExchangeId, AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentOutputStatus,
    CallMCPToolResult, CancellationReason, CloneRepositoryURL, CreateDocumentsResult,
    DocumentContext, EditDocumentsResult, FileContext, FileGlobResult, FileGlobV2Match,
    FileGlobV2Result, FinishedAIAgentOutput, GrepFileMatch, GrepLineMatch, GrepResult,
    ImageContext, InsertReviewCommentsResult, OutputModelInfo, PassiveCodeDiffEntry,
    PassiveSuggestionResultType, PassiveSuggestionTrigger, ReadDocumentsResult, ReadFilesResult,
    ReadMCPResourceResult, ReadShellCommandOutputResult, RequestCommandOutputResult,
    RequestFileEditsResult, SearchCodebaseFailureReason, SearchCodebaseResult, ServerOutputId,
    Shared, ShellCommandCompletedTrigger, ShellCommandError, SuggestNewConversationResult,
    SuggestPromptResult, TransferShellCommandControlToUserResult, UpdatedFileContext,
    UploadArtifactResult, WriteToLongRunningShellCommandResult,
};
use crate::ai::block_context::BlockContext;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};
use crate::ai::llms::LLMId;
use crate::ai_assistant::execution_context::{WarpAiExecutionContext, WarpAiOsContext};
use crate::terminal::model::block::BlockId;
use crate::terminal::model::terminal_model::BlockIndex;
use ai::agent::action_result::{
    AskUserQuestionAnswerItem, AskUserQuestionResult, FetchConversationResult, ReadSkillResult,
    RequestComputerUseResult, SendMessageToAgentResult, StartAgentResult, StartAgentVersion,
    UseComputerResult,
};
use ai::skills::ParsedSkill;
use chrono::{DateTime, Local, TimeZone};
use persistence::model::AgentConversationData;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use warp_core::command::ExitCode;
use warp_multi_agent_api as api;
use warp_multi_agent_api::ask_user_question_result::answer_item::Answer as AskUserQuestionAnswer;

use crate::ai::agent::conversation::ServerAIConversationMetadata;
use crate::ai::agent::UserQueryMode;

/// How to restore a conversation from the cloud.
pub enum RestorationMode {
    /// Continue the same conversation (use the same server ID).
    Continue,
    /// Fork from the original conversation.
    #[allow(dead_code)]
    Fork,
}

/// Converts a cloud ConversationData to an AIConversation.
/// The `metadata` contains all server-side information about the conversation, including usage data.
/// `restoration_mode` controls how the server metadata is handled - we should only keep the metadata when continuing, not forking
pub fn convert_conversation_data_to_ai_conversation(
    conversation_id: AIConversationId,
    conversation_data: &api::ConversationData,
    metadata: ServerAIConversationMetadata,
    restoration_mode: RestorationMode,
) -> Option<AIConversation> {
    let usage_metadata = Some(metadata.usage.clone());

    let agent_conversation_data = match restoration_mode {
        RestorationMode::Fork => AgentConversationData {
            server_conversation_token: None,
            conversation_usage_metadata: usage_metadata,
            reverted_action_ids: None,
            forked_from_server_conversation_token: Some(
                metadata.server_conversation_token.as_str().to_string(),
            ),
            // If we fork, new conversation, artifacts don't carry over
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        },
        RestorationMode::Continue => AgentConversationData {
            server_conversation_token: Some(
                metadata.server_conversation_token.as_str().to_string(),
            ),
            conversation_usage_metadata: usage_metadata,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: serde_json::to_string(&metadata.artifacts).ok(),
            parent_agent_id: None,
            agent_name: None,
            orchestration_harness_type: None,
            parent_conversation_id: None,
            is_remote_child: false,
            run_id: metadata
                .ambient_agent_task_id
                .map(|task_id| task_id.to_string()),
            autoexecute_override: None,
            last_event_sequence: None,
            pinned: false,
        },
    };

    match AIConversation::new_restored(
        conversation_id,
        conversation_data.tasks.clone(),
        Some(agent_conversation_data),
    ) {
        Ok(mut conversation) => {
            // Set the server metadata only if we're continuing
            // If we're forking, this should be treated as a brand new conversation that doesn't have server metadata yet.
            // After the first request, server metadata will be populated.
            if matches!(restoration_mode, RestorationMode::Continue) {
                conversation.set_server_metadata(metadata);
            }
            Some(conversation)
        }
        Err(e) => {
            log::warn!("Failed to convert ConversationData to AIConversation: {e:?}");
            None
        }
    }
}

/// Converts InputContext from the API to the application type `Arc<[AIAgentContext]>`
#[allow(clippy::single_range_in_vec_init)]
pub(crate) fn convert_input_context(context: Option<&api::InputContext>) -> Arc<[AIAgentContext]> {
    let Some(context) = context else {
        return Arc::new([]);
    };

    let mut result = Vec::new();

    // Convert executed shell commands
    #[allow(deprecated)]
    for executed_shell_command in &context.executed_shell_commands {
        if !executed_shell_command.command.is_empty() {
            result.push(AIAgentContext::Block(Box::new(BlockContext {
                id: BlockId::default(),
                index: BlockIndex::from(0),
                command: executed_shell_command.command.clone(),
                output: executed_shell_command.output.clone(),
                exit_code: ExitCode::from(executed_shell_command.exit_code),
                is_auto_attached: executed_shell_command.is_auto_attached,
                started_ts: executed_shell_command
                    .started_ts
                    .as_ref()
                    .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                finished_ts: executed_shell_command
                    .finished_ts
                    .as_ref()
                    .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                pwd: None,
                shell: None,
                username: None,
                hostname: None,
                git_branch: None,
                os: None,
                session_id: None,
            })));
        }
    }

    // Convert directory context
    if let Some(directory) = &context.directory {
        result.push(AIAgentContext::Directory {
            pwd: if directory.pwd.is_empty() {
                None
            } else {
                Some(directory.pwd.clone())
            },
            home_dir: if directory.home.is_empty() {
                None
            } else {
                Some(directory.home.clone())
            },
            are_file_symbols_indexed: directory.pwd_file_symbols_indexed,
        });
    }

    // Convert operating system and shell to execution environment
    if let (Some(os), Some(shell)) = (&context.operating_system, &context.shell) {
        result.push(AIAgentContext::ExecutionEnvironment(
            WarpAiExecutionContext {
                os: WarpAiOsContext {
                    category: if os.platform.is_empty() {
                        None
                    } else {
                        Some(os.platform.clone())
                    },
                    distribution: if os.distribution.is_empty() {
                        None
                    } else {
                        Some(os.distribution.clone())
                    },
                },
                shell_name: shell.name.clone(),
                shell_version: if shell.version.is_empty() {
                    None
                } else {
                    Some(shell.version.clone())
                },
            },
        ));
    }

    // Convert current time
    if let Some(current_time) = &context.current_time {
        let datetime = proto_timestamp_to_local_datetime(current_time.seconds, current_time.nanos);
        result.push(AIAgentContext::CurrentTime {
            current_time: datetime,
        });
    }

    // Convert selected text
    for selected_text in &context.selected_text {
        if !selected_text.text.is_empty() {
            result.push(AIAgentContext::SelectedText(selected_text.text.clone()));
        }
    }

    // Convert images
    for image in &context.images {
        if !image.data.is_empty() {
            let mime_type = if image.mime_type.is_empty() {
                "image/jpeg".to_string() // Default MIME type
            } else {
                image.mime_type.clone()
            };

            // Convert binary data to base64
            use base64::{engine::general_purpose, Engine};
            let data = general_purpose::STANDARD.encode(&image.data);

            result.push(AIAgentContext::Image(ImageContext {
                data,
                mime_type,
                file_name: "image".to_string(), // Default file name since proto doesn't have it
                is_figma: false, // This field is only used for detecting Figma pngs in the input and is not meaningful for restored conversations.
            }));
        }
    }

    // Convert files
    for file in &context.files {
        if let Some(content) = &file.content {
            let file_context = FileContext::from(content.clone());

            if !file_context.file_name.is_empty() && !file_context.content.is_empty() {
                result.push(AIAgentContext::File(file_context.clone()));
            }
        }
    }

    // Convert project rules
    for project_rules in &context.project_rules {
        if !project_rules.root_path.is_empty()
            && (!project_rules.active_rule_files.is_empty()
                || !project_rules.additional_rule_file_paths.is_empty())
        {
            result.push(AIAgentContext::ProjectRules {
                root_path: project_rules.root_path.clone(),
                active_rules: project_rules
                    .active_rule_files
                    .iter()
                    .map(|file_content| FileContext::from(file_content.clone()))
                    .collect(),
                additional_rule_paths: project_rules.additional_rule_file_paths.clone(),
            });
        }
    }

    // Convert codebases
    for codebase in &context.codebases {
        if !codebase.path.is_empty() && !codebase.name.is_empty() {
            result.push(AIAgentContext::Codebase {
                path: codebase.path.clone(),
                name: codebase.name.clone(),
            });
        }
    }

    result.into()
}

/// Trait for converting task messages into AIAgentExchange objects
/// for display in the UI as restored AI blocks.
pub trait ConvertToExchanges {
    fn into_exchanges(self) -> Vec<AIAgentExchange>;
}

impl ConvertToExchanges for &api::Task {
    /// Converts a list of tasks into AIAgentExchange objects.
    ///
    /// Note: for now, we only restore messages from the root task (task with no parent).
    fn into_exchanges(self) -> Vec<AIAgentExchange> {
        let mut exchanges = Vec::new();
        let mut todo_lists: Vec<AIAgentTodoList> = Vec::new();

        // Build a map of message_id -> message for quick lookup
        let mut message_map: HashMap<&str, &api::Message> = HashMap::new();
        // Build a map of tool_call_id -> tool_call for cancelled results
        let mut tool_call_map: HashMap<String, &api::message::ToolCall> = HashMap::new();
        for message in &self.messages {
            message_map.insert(message.id.as_str(), message);
            // If this is a tool call message, add it to the tool call map
            if let Some(api::message::Message::ToolCall(tool_call)) = &message.message {
                tool_call_map.insert(tool_call.tool_call_id.clone(), tool_call);
            }
        }

        // Process messages in chronological order
        let mut current_inputs = Vec::new();
        let mut current_outputs = Vec::new();
        let mut current_message_ids = HashSet::new();
        let mut document_versions: HashMap<AIDocumentId, AIDocumentVersion> = HashMap::new();
        let mut current_request_id: Option<String> = None;

        // Almost all messages should be ingested as outputs, except for some special cases:
        // 1. User queries
        // 2. System queries, but only if they are displayed as user queries/initiate conversations, like queries:
        //   * from the new project flow (displayed as user queries)
        //   * from the clone repository flow (displayed as user queries)
        //   * from auto code diff queries (initiate new conversations)
        // 3. tool call results (as we also render these like inputs)
        for api_message in self.messages.iter() {
            let Some(message) = &api_message.message else {
                continue;
            };

            let task_id = TaskId::new(api_message.task_id.clone());
            // Check if request_id has changed - if so, create an exchange from accumulated messages
            let message_request_id = if api_message.request_id.is_empty() {
                None
            } else {
                Some(api_message.request_id.clone())
            };

            // Create exchange if request_id changed and we have accumulated messages
            if message_request_id != current_request_id
                && current_request_id.is_some()
                && (!current_inputs.is_empty() || !current_outputs.is_empty())
            {
                let is_output_tool_call_canceled = current_inputs
                    .last()
                    .and_then(|input: &AIAgentInput| input.action_result())
                    .map(|result| result.result.is_cancelled())
                    .unwrap_or(false);

                if let Some(exchange) = create_exchange_from_messages(
                    &current_inputs,
                    &current_outputs,
                    is_output_tool_call_canceled,
                    &current_message_ids,
                    &message_map,
                    current_request_id.as_deref(),
                ) {
                    exchanges.push(exchange);
                }
                current_inputs.clear();
                current_outputs.clear();
                current_message_ids.clear();
            }

            // Update current_request_id
            current_request_id = message_request_id;

            // Track this message ID for the current exchange
            current_message_ids.insert(api_message.id.clone());

            let added_message_as_exchange_input = match message {
                api::message::Message::UserQuery(user_query) => {
                    // Add user query as input
                    current_inputs.push(AIAgentInput::UserQuery {
                        query: user_query.query.clone(),
                        context: convert_input_context(user_query.context.as_ref()),
                        static_query_type: None,
                        referenced_attachments: HashMap::new(),
                        user_query_mode: convert_user_query_mode(user_query.mode.as_ref()),
                        running_command: None,
                        intended_agent: Some(user_query.intended_agent()),
                    });
                    true
                }
                api::message::Message::SystemQuery(query) => {
                    let Some(query_type) = &query.r#type else {
                        continue;
                    };

                    match query_type {
                        api::message::system_query::Type::CreateNewProject(auto_code_diff) => {
                            current_inputs.push(AIAgentInput::UserQuery {
                                query: auto_code_diff.query.clone(),
                                context: convert_input_context(query.context.as_ref()),
                                static_query_type: None,
                                referenced_attachments: HashMap::new(),
                                user_query_mode: UserQueryMode::default(), // SystemQuery doesn't have mode field
                                running_command: None,
                                intended_agent: None,
                            });
                            true
                        }
                        api::message::system_query::Type::CloneRepository(clone_repo) => {
                            current_inputs.push(AIAgentInput::CloneRepository {
                                clone_repo_url: CloneRepositoryURL::new(clone_repo.url.clone()),
                                context: convert_input_context(query.context.as_ref()),
                            });
                            true
                        }
                        api::message::system_query::Type::AutoCodeDiff(auto_code_diff) => {
                            current_inputs.push(AIAgentInput::AutoCodeDiffQuery {
                                query: auto_code_diff.query.clone(),
                                context: convert_input_context(query.context.as_ref()),
                            });
                            true
                        }
                        api::message::system_query::Type::FetchReviewComments(fetch) => {
                            current_inputs.push(AIAgentInput::FetchReviewComments {
                                repo_path: fetch.repo_path.clone(),
                                context: convert_input_context(query.context.as_ref()),
                            });
                            true
                        }
                        // TriggerSuggestPrompt is not rendered as user input, so we don't want to include it as an input in the exchange.
                        // ResumeConversation is actually added to the task's messages as a plain UserQuery, so we don't expect to encounter it in the task's messages.
                        api::message::system_query::Type::ResumeConversation(_)
                        | api::message::system_query::Type::GeneratePassiveSuggestions(_)
                        // TODO: Implement this for real. ZB adding this to bump proto version for unrelated API changes.
                        | api::message::system_query::Type::SummarizeConversation(_)
                        // HandoffRehydration is injected by the server for agent-only
                        // context; the client must never render it as user input.
                        | api::message::system_query::Type::HandoffRehydration(_) => false,
                    }
                }
                api::message::Message::ToolCallResult(tool_call_result) => {
                    // Try to convert tool call result - returns None for ServerToolCalls
                    if let Some(input) = convert_tool_call_result_to_input(
                        &task_id,
                        tool_call_result,
                        &tool_call_map,
                        &mut document_versions,
                    ) {
                        // Add tool call result as input
                        current_inputs.push(input);
                    }

                    true
                }
                api::message::Message::UpdateTodos(update) => {
                    if let Some(operation) = &update.operation {
                        update_todo_list_from_todo_op(&mut todo_lists, operation.clone());
                    }

                    false
                }
                api::message::Message::InvokeSkill(invoke_skill) => {
                    if let Some(api_skill) = invoke_skill.skill.clone() {
                        if let Ok(parsed_skill) = ParsedSkill::try_from(api_skill) {
                            let user_query = invoke_skill
                                .user_query
                                .clone()
                                .map(|user_query| crate::ai::agent::InvokeSkillUserQuery {
                                    query: user_query.query,
                                    // Restored conversations currently do not hydrate invoke-skill
                                    // inline attachments back into client-side attachment structs.
                                    // TODO(APP-3101): support rehydration of attachments.
                                    referenced_attachments: HashMap::new(),
                                });
                            let input = AIAgentInput::InvokeSkill {
                                context: Arc::new([]),
                                skill: parsed_skill,
                                user_query,
                            };
                            current_inputs.push(input);
                        };
                    };

                    true
                }
                // Preserve EventsFromAgents as an explicit input in restored conversations
                // so orchestration state (including lifecycle timestamps) survives roundtrip.
                api::message::Message::EventsFromAgents(events) => {
                    current_inputs.push(AIAgentInput::EventsFromAgents {
                        events: events.agent_events.clone(),
                    });
                    true
                }
                api::message::Message::PassiveSuggestionResult(passive_result) => {
                    if let Some(input) =
                        convert_passive_suggestion_result_to_input(passive_result)
                    {
                        current_inputs.push(input);
                    }
                    true
                }
                api::message::Message::AgentOutput(_)
                | api::message::Message::AgentReasoning(_)
                | api::message::Message::Summarization(_)
                | api::message::Message::ToolCall(_)
                | api::message::Message::ServerEvent(_)
                | api::message::Message::UpdateReviewComments(_)
                | api::message::Message::CodeReview(_)
                // TODO(advait): Handle this for restored + forked conversations w/ web searches/fetches.
                | api::message::Message::WebSearch(_)
                | api::message::Message::WebFetch(_)
                | api::message::Message::DebugOutput(_)
                | api::message::Message::ArtifactEvent(_)
                | api::message::Message::MessagesReceivedFromAgents(_)
                | api::message::Message::ModelUsed(_)
                | api::message::Message::OrchestrationConfigSnapshot(_) => false,
            };

            if !added_message_as_exchange_input {
                if let Ok(MaybeAIAgentOutputMessage::Message(output_msg)) = (*api_message)
                    .clone()
                    .to_client_output_message(ConversionParams {
                        current_todo_list: todo_lists.last(),
                        // TODO(alokedesai): Support persistence for the code review state.
                        active_code_review: None,
                        task_id: &TaskId::new(api_message.task_id.clone()),
                    })
                {
                    current_outputs.push(output_msg);
                }
            }
        }

        // At the end, if we have remaining inputs or outputs, create the last exchange.
        if !current_inputs.is_empty() || !current_outputs.is_empty() {
            // If the last message is a tool call (i.e. we have no corresponding result)
            // we will assume the tool call was cancelled.
            let is_output_tool_call_canceled = self.messages.last().is_some_and(|message| {
                matches!(message.message, Some(api::message::Message::ToolCall(_)))
            });

            if let Some(exchange) = create_exchange_from_messages(
                &current_inputs,
                &current_outputs,
                is_output_tool_call_canceled,
                &current_message_ids,
                &message_map,
                current_request_id.as_deref(),
            ) {
                exchanges.push(exchange);
            }
        }

        exchanges
    }
}

/// Convert a ToolCallResult to an AIAgentInput::ActionResult
/// Returns None if the tool call result is a ServerToolCallResult
/// `document_versions` tracks the latest version per document for CreateDocuments and EditDocuments results.
/// Each new document (CreateDocuments) starts at the default version; edits increment the specific document's version.
#[allow(clippy::single_range_in_vec_init)]
pub(crate) fn convert_tool_call_result_to_input(
    task_id: &TaskId,
    tool_call_result: &api::message::ToolCallResult,
    tool_call_map: &HashMap<String, &api::message::ToolCall>,
    document_versions: &mut HashMap<AIDocumentId, AIDocumentVersion>,
) -> Option<AIAgentInput> {
    use warp_multi_agent_api::message::tool_call_result::Result as ToolCallResultType;

    let tool_call_id = tool_call_result.tool_call_id.clone();
    let context = convert_input_context(tool_call_result.context.as_ref());

    match tool_call_result.result.as_ref() {
        Some(ToolCallResultType::RunShellCommand(result)) => {
            // Convert RunShellCommand result to RequestCommandOutputResult
            let command_output_result = match &result.result {
                Some(api::run_shell_command_result::Result::CommandFinished(finished)) => {
                    RequestCommandOutputResult::Completed {
                        block_id: finished.command_id.clone().into(),
                        command: result.command.clone(),
                        output: finished.output.clone(),
                        exit_code: ExitCode::from(finished.exit_code),
                        start_ts: finished
                            .start_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                        completed_ts: finished
                            .finish_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                    }
                }
                Some(api::run_shell_command_result::Result::LongRunningCommandSnapshot(
                    snapshot,
                )) => RequestCommandOutputResult::LongRunningCommandSnapshot {
                    command: result.command.clone(),
                    block_id: snapshot.command_id.clone().into(),
                    grid_contents: snapshot.output.clone(),
                    cursor: snapshot.cursor.clone(),
                    is_alt_screen_active: snapshot.is_alt_screen_active,
                },
                Some(api::run_shell_command_result::Result::PermissionDenied(
                    api::PermissionDenied { .. },
                ))
                | None => {
                    // If no result is present, treat as cancelled
                    RequestCommandOutputResult::CancelledBeforeExecution
                }
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::RequestCommandOutput(command_output_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::WriteToLongRunningShellCommand(result)) => {
            let write_result = match &result.result {
                    Some(api::write_to_long_running_shell_command_result::Result::LongRunningCommandSnapshot(
                        snapshot,
                    )) => WriteToLongRunningShellCommandResult::Snapshot {
                        block_id: snapshot.command_id.clone().into(),
                        grid_contents: snapshot.output.clone(),
                        cursor: snapshot.cursor.clone(),
                        is_alt_screen_active: snapshot.is_alt_screen_active,
                        is_preempted: snapshot.is_preempted,
                    },
                    Some(api::write_to_long_running_shell_command_result::Result::CommandFinished(
                        finished,
                    )) => WriteToLongRunningShellCommandResult::CommandFinished {
                        block_id: finished.command_id.clone().into(),
                        output: finished.output.clone(),
                        exit_code: ExitCode::from(finished.exit_code),
                        start_ts: finished.start_ts.as_ref().map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                        completed_ts: finished.finish_ts.as_ref().map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                    },
                    Some(api::write_to_long_running_shell_command_result::Result::Error(api::ShellCommandError{
                        r#type: Some(api::shell_command_error::Type::CommandNotFound(()))
                    })) => WriteToLongRunningShellCommandResult::Error(ShellCommandError::BlockNotFound),
                    Some(api::write_to_long_running_shell_command_result::Result::Error(_)) | None => WriteToLongRunningShellCommandResult::Cancelled,
                };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::WriteToLongRunningShellCommand(write_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::ReadFiles(result)) => {
            let read_result = match &result.result {
                Some(api::read_files_result::Result::AnyFilesSuccess(success)) => {
                    let files = success
                        .files
                        .iter()
                        .map(|file| FileContext::from(file.clone()))
                        .collect();

                    ReadFilesResult::Success { files }
                }
                Some(api::read_files_result::Result::TextFilesSuccess(success)) => {
                    let files = success
                        .files
                        .iter()
                        .map(|file| FileContext::from(file.clone()))
                        .collect();
                    ReadFilesResult::Success { files }
                }
                Some(api::read_files_result::Result::Error(error)) => {
                    ReadFilesResult::Error(error.message.clone())
                }
                None => ReadFilesResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::ReadFiles(read_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::UploadFileArtifact(result)) => {
            let upload_result = match &result.result {
                Some(api::upload_file_artifact_result::Result::Success(success)) => {
                    UploadArtifactResult::Success {
                        artifact_uid: success.artifact_uid.clone(),
                        filepath: None,
                        mime_type: success.mime_type.clone(),
                        description: None,
                        size_bytes: success.size_bytes,
                    }
                }
                Some(api::upload_file_artifact_result::Result::Error(error)) => {
                    UploadArtifactResult::Error(error.message.clone())
                }
                None => UploadArtifactResult::Error(
                    "Upload artifact tool call returned no result".to_string(),
                ),
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::UploadArtifact(upload_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::SearchCodebase(result)) => {
            let search_result = match &result.result {
                Some(api::search_codebase_result::Result::Success(success)) => {
                    let files = success
                        .files
                        .iter()
                        .map(|file| FileContext::from(file.clone()))
                        .collect();

                    SearchCodebaseResult::Success { files }
                }
                Some(api::search_codebase_result::Result::Error(error)) => {
                    SearchCodebaseResult::Failed {
                        reason: SearchCodebaseFailureReason::ClientError,
                        message: error.message.clone(),
                    }
                }
                None => SearchCodebaseResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::SearchCodebase(search_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::ApplyFileDiffs(result)) => {
            let edit_result = match &result.result {
                Some(api::apply_file_diffs_result::Result::Success(success)) => {
                    let updated_files = success
                        .updated_files_v2
                        .iter()
                        .filter_map(|updated_file| {
                            updated_file.file.as_ref().map(|file| UpdatedFileContext {
                                was_edited_by_user: updated_file.was_edited_by_user,
                                file_context: FileContext::from(file.clone()),
                            })
                        })
                        .collect();

                    RequestFileEditsResult::Success {
                        diff: "".to_string(), // This is a legacy-only field that should be removed
                        updated_files,
                        deleted_files: success
                            .deleted_files
                            .iter()
                            .map(|f| f.file_path.clone())
                            .collect(),
                        // Line counts are not available in legacy persisted data
                        lines_added: 0,
                        lines_removed: 0,
                    }
                }
                Some(api::apply_file_diffs_result::Result::Error(error)) => {
                    RequestFileEditsResult::DiffApplicationFailed {
                        error: error.message.clone(),
                    }
                }
                None => RequestFileEditsResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::RequestFileEdits(edit_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::Grep(result)) => {
            let grep_result = match &result.result {
                Some(api::grep_result::Result::Success(success)) => {
                    let matched_files = success
                        .matched_files
                        .iter()
                        .map(|file| {
                            let matched_lines = file
                                .matched_lines
                                .iter()
                                .map(|line| GrepLineMatch {
                                    line_number: line.line_number as usize,
                                })
                                .collect();

                            GrepFileMatch {
                                file_path: file.file_path.clone(),
                                matched_lines,
                            }
                        })
                        .collect();

                    GrepResult::Success { matched_files }
                }
                Some(api::grep_result::Result::Error(error)) => {
                    GrepResult::Error(error.message.clone())
                }
                None => GrepResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::Grep(grep_result),
                },
                context,
            })
        }
        #[allow(deprecated)]
        Some(ToolCallResultType::FileGlob(result)) => {
            let glob_result = match &result.result {
                Some(api::file_glob_result::Result::Success(success)) => FileGlobResult::Success {
                    matched_files: success.matched_files.clone(),
                },
                Some(api::file_glob_result::Result::Error(error)) => {
                    FileGlobResult::Error(error.message.clone())
                }
                None => FileGlobResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::FileGlob(glob_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::FileGlobV2(result)) => {
            let glob_result = match &result.result {
                Some(api::file_glob_v2_result::Result::Success(success)) => {
                    let matched_files = success
                        .matched_files
                        .iter()
                        .map(|file| FileGlobV2Match {
                            file_path: file.file_path.clone(),
                        })
                        .collect();

                    FileGlobV2Result::Success {
                        matched_files,
                        warnings: Some(success.warnings.clone()).filter(|s| !s.is_empty()),
                    }
                }
                Some(api::file_glob_v2_result::Result::Error(error)) => {
                    FileGlobV2Result::Error(error.message.clone())
                }
                None => FileGlobV2Result::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::FileGlobV2(glob_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::ReadMcpResource(result)) => {
            let mcp_result = match &result.result {
                Some(api::read_mcp_resource_result::Result::Success(success)) => {
                    let resource_contents = success
                        .contents
                        .iter()
                        .map(|content| {
                            match &content.content_type {
                                Some(api::mcp_resource_content::ContentType::Text(text)) => {
                                    rmcp::model::ResourceContents::TextResourceContents {
                                        uri: content.uri.clone(),
                                        mime_type: Some(text.mime_type.clone()),
                                        text: text.content.clone(),
                                        meta: None,
                                    }
                                }
                                Some(api::mcp_resource_content::ContentType::Binary(binary)) => {
                                    rmcp::model::ResourceContents::BlobResourceContents {
                                        uri: content.uri.clone(),
                                        mime_type: Some(binary.mime_type.clone()),
                                        blob: String::from_utf8_lossy(&binary.data).into_owned(),
                                        meta: None,
                                    }
                                }
                                None => {
                                    // Default to text if no content type is specified
                                    rmcp::model::ResourceContents::TextResourceContents {
                                        uri: content.uri.clone(),
                                        mime_type: None,
                                        text: "".to_string(),
                                        meta: None,
                                    }
                                }
                            }
                        })
                        .collect();

                    ReadMCPResourceResult::Success { resource_contents }
                }
                Some(api::read_mcp_resource_result::Result::Error(error)) => {
                    ReadMCPResourceResult::Error(error.message.clone())
                }
                None => ReadMCPResourceResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::ReadMCPResource(mcp_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::CallMcpTool(result)) => {
            let mcp_tool_result = match &result.result {
                Some(api::call_mcp_tool_result::Result::Success(success)) => {
                    let results = success
                        .results
                        .iter()
                        .map(|api_result| match &api_result.result {
                            Some(api::call_mcp_tool_result::success::result::Result::Text(
                                text,
                            )) => rmcp::model::Content::text(text.text.clone()),
                            Some(api::call_mcp_tool_result::success::result::Result::Image(
                                image,
                            )) => rmcp::model::Content::image(
                                String::from_utf8_lossy(&image.data).to_string(),
                                image.mime_type.clone(),
                            ),
                            Some(api::call_mcp_tool_result::success::result::Result::Resource(
                                resource,
                            )) => match &resource.content_type {
                                Some(api::mcp_resource_content::ContentType::Text(text)) => {
                                    rmcp::model::Content::resource(
                                        rmcp::model::ResourceContents::text(
                                            text.content.clone(),
                                            resource.uri.clone(),
                                        ),
                                    )
                                }
                                Some(api::mcp_resource_content::ContentType::Binary(binary)) => {
                                    rmcp::model::Content::resource(
                                        rmcp::model::ResourceContents::BlobResourceContents {
                                            uri: resource.uri.clone(),
                                            mime_type: Some(binary.mime_type.clone()),
                                            blob: String::from_utf8_lossy(&binary.data).to_string(),
                                            meta: None,
                                        },
                                    )
                                }
                                None => rmcp::model::Content::resource(
                                    rmcp::model::ResourceContents::text(
                                        String::new(),
                                        resource.uri.clone(),
                                    ),
                                ),
                            },
                            None => rmcp::model::Content::text(String::new()),
                        })
                        .collect();

                    let result = rmcp::model::CallToolResult::success(results);

                    CallMCPToolResult::Success { result }
                }
                Some(api::call_mcp_tool_result::Result::Error(error)) => {
                    CallMCPToolResult::Error(error.message.clone())
                }
                None => CallMCPToolResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::CallMCPTool(mcp_tool_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::ReadSkill(result)) => {
            let read_skill_result = match &result.result {
                Some(api::read_skill_result::Result::Success(success)) => {
                    if let Some(content) = &success.content {
                        let context = FileContext::from(content.clone());
                        ReadSkillResult::Success { content: context }
                    } else {
                        ReadSkillResult::Error("FileContent is None".to_string())
                    }
                }
                Some(api::read_skill_result::Result::Error(error)) => {
                    ReadSkillResult::Error(error.message.clone())
                }
                None => ReadSkillResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::ReadSkill(read_skill_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::SuggestNewConversation(result)) => {
            let conversation_result = match &result.result {
                Some(api::suggest_new_conversation_result::Result::Accepted(accepted)) => {
                    SuggestNewConversationResult::Accepted {
                        message_id: accepted.message_id.clone(),
                    }
                }
                Some(api::suggest_new_conversation_result::Result::Rejected(_)) => {
                    SuggestNewConversationResult::Rejected
                }
                None => SuggestNewConversationResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::SuggestNewConversation(conversation_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::SuggestPrompt(result)) => {
            let prompt_result = match &result.result {
                Some(api::suggest_prompt_result::Result::Accepted(_)) => {
                    // Find the accepted query from the original SuggestPrompt tool call
                    let query = tool_call_map.get(&tool_call_id)
                        .and_then(|tool_call| {
                            if let Some(api::message::tool_call::Tool::SuggestPrompt(suggest_prompt)) = &tool_call.tool {
                                match &suggest_prompt.display_mode {
                                    Some(api::message::tool_call::suggest_prompt::DisplayMode::InlineQueryBanner(
                                        inline_query_banner,
                                    )) => Some(inline_query_banner.query.clone()),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();

                    SuggestPromptResult::Accepted { query }
                }
                _ => SuggestPromptResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::SuggestPrompt(prompt_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::OpenCodeReview(_)) => Some(AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: tool_call_id.into(),
                task_id: task_id.clone(),
                result: AIAgentActionResultType::OpenCodeReview,
            },
            context,
        }),
        Some(ToolCallResultType::InitProject(_)) => Some(AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: tool_call_id.into(),
                task_id: task_id.clone(),
                result: AIAgentActionResultType::InitProject,
            },
            context,
        }),
        Some(ToolCallResultType::ReadDocuments(result)) => {
            let read_result = match &result.result {
                Some(api::read_documents_result::Result::Success(success)) => {
                    let documents = success
                        .documents
                        .iter()
                        .filter_map(|doc| {
                            AIDocumentId::try_from(doc.document_id.clone())
                                .ok()
                                .map(|id| {
                                    DocumentContext {
                                        document_id: id,
                                        // Version is a placeholder here - the actual current version
                                        // will be determined when the document is accessed from AIDocumentModel
                                        document_version: AIDocumentVersion(1),
                                        content: doc.content.clone(),
                                        line_ranges: vec![],
                                    }
                                })
                        })
                        .collect();

                    ReadDocumentsResult::Success { documents }
                }
                Some(api::read_documents_result::Result::Error(error)) => {
                    ReadDocumentsResult::Error(error.message.clone())
                }
                None => ReadDocumentsResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::ReadDocuments(read_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::EditDocuments(result)) => {
            let edit_result = match &result.result {
                Some(api::edit_documents_result::Result::Success(success)) => {
                    let updated_documents = success
                        .updated_documents
                        .iter()
                        .map(|doc| {
                            AIDocumentId::try_from(doc.document_id.clone())
                                .ok()
                                .map(|id| {
                                    // Each edit produces a new version. Increment the
                                    // tracked version for this document and use the result.
                                    let entry = document_versions.entry(id).or_default();
                                    let version = entry.next();
                                    *entry = version;
                                    DocumentContext {
                                        document_id: id,
                                        document_version: version,
                                        content: doc.content.clone(),
                                        line_ranges: vec![],
                                    }
                                })
                        })
                        .collect::<Option<Vec<_>>>()
                        .unwrap_or_default();

                    EditDocumentsResult::Success { updated_documents }
                }
                Some(api::edit_documents_result::Result::Error(error)) => {
                    EditDocumentsResult::Error(error.message.clone())
                }
                None => EditDocumentsResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::EditDocuments(edit_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::CreateDocuments(result)) => {
            let create_result = match &result.result {
                Some(api::create_documents_result::Result::Success(success)) => {
                    let created_documents = success
                        .created_documents
                        .iter()
                        .map(|doc| {
                            AIDocumentId::try_from(doc.document_id.clone())
                                .ok()
                                .map(|id| {
                                    // New documents always start at the default version.
                                    let version = AIDocumentVersion::default();
                                    document_versions.insert(id, version);
                                    DocumentContext {
                                        document_id: id,
                                        document_version: version,
                                        content: doc.content.clone(),
                                        line_ranges: vec![],
                                    }
                                })
                        })
                        .collect::<Option<Vec<_>>>()
                        .unwrap_or_default();

                    CreateDocumentsResult::Success { created_documents }
                }
                Some(api::create_documents_result::Result::Error(error)) => {
                    CreateDocumentsResult::Error(error.message.clone())
                }
                None => CreateDocumentsResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::CreateDocuments(create_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::ReadShellCommandOutput(result)) => {
            let read_result = match &result.result {
                Some(api::read_shell_command_output_result::Result::CommandFinished(finished)) => {
                    ReadShellCommandOutputResult::CommandFinished {
                        command: result.command.clone(),
                        block_id: finished.command_id.clone().into(),
                        output: finished.output.clone(),
                        exit_code: ExitCode::from(finished.exit_code),
                        start_ts: finished
                            .start_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                        completed_ts: finished
                            .finish_ts
                            .as_ref()
                            .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                    }
                }
                Some(
                    api::read_shell_command_output_result::Result::LongRunningCommandSnapshot(
                        snapshot,
                    ),
                ) => ReadShellCommandOutputResult::LongRunningCommandSnapshot {
                    command: result.command.clone(),
                    block_id: snapshot.command_id.clone().into(),
                    grid_contents: snapshot.output.clone(),
                    cursor: snapshot.cursor.clone(),
                    is_alt_screen_active: snapshot.is_alt_screen_active,
                    is_preempted: snapshot.is_preempted,
                },
                Some(api::read_shell_command_output_result::Result::Error(
                    api::ShellCommandError {
                        r#type: Some(api::shell_command_error::Type::CommandNotFound(())),
                    },
                )) => ReadShellCommandOutputResult::Error(ShellCommandError::BlockNotFound),
                Some(api::read_shell_command_output_result::Result::Error(_)) | None => {
                    ReadShellCommandOutputResult::Cancelled
                }
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::ReadShellCommandOutput(read_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::TransferShellCommandControlToUser(result)) => {
            let transfer_result = match &result.result {
                Some(
                    api::transfer_shell_command_control_to_user_result::Result::LongRunningCommandSnapshot(snapshot),
                ) => TransferShellCommandControlToUserResult::Snapshot {
                    block_id: snapshot.command_id.clone().into(),
                    grid_contents: snapshot.output.clone(),
                    cursor: snapshot.cursor.clone(),
                    is_alt_screen_active: snapshot.is_alt_screen_active,
                    is_preempted: snapshot.is_preempted,
                },
                Some(
                    api::transfer_shell_command_control_to_user_result::Result::CommandFinished(finished),
                ) => TransferShellCommandControlToUserResult::CommandFinished {
                    block_id: finished.command_id.clone().into(),
                    output: finished.output.clone(),
                    exit_code: ExitCode::from(finished.exit_code),
                    start_ts: finished.start_ts.as_ref().map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                    completed_ts: finished.finish_ts.as_ref().map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
                },
                Some(api::transfer_shell_command_control_to_user_result::Result::Error(
                    api::ShellCommandError {
                        r#type: Some(api::shell_command_error::Type::CommandNotFound(())),
                    },
                )) => TransferShellCommandControlToUserResult::Error(
                    ShellCommandError::BlockNotFound,
                ),
                Some(api::transfer_shell_command_control_to_user_result::Result::Error(_))
                | None => TransferShellCommandControlToUserResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::TransferShellCommandControlToUser(
                        transfer_result,
                    ),
                },
                context,
            })
        }
        Some(ToolCallResultType::InsertReviewComments(api_result)) => {
            let res = AIAgentActionResultType::InsertReviewComments(match &api_result.result {
                Some(api::insert_review_comments_result::Result::Success(_)) => {
                    InsertReviewCommentsResult::Success {
                        repo_path: api_result.repo_path.clone(),
                    }
                }
                Some(api::insert_review_comments_result::Result::Error(err)) => {
                    InsertReviewCommentsResult::Error {
                        repo_path: api_result.repo_path.clone(),
                        message: err.message.clone(),
                    }
                }
                None => InsertReviewCommentsResult::Cancelled,
            });
            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: res,
                },
                context,
            })
        }
        Some(ToolCallResultType::UseComputer(result)) => {
            let use_computer_result = match &result.result {
                Some(api::use_computer_result::Result::Success(success)) => {
                    let screenshot = success.screenshot.as_ref().map(|s| {
                        // The original dimensions are not preserved through the API, so we use
                        // the current dimensions for both.
                        computer_use::Screenshot {
                            width: s.width as usize,
                            height: s.height as usize,
                            original_width: s.width as usize,
                            original_height: s.height as usize,
                            data: s.data.clone(),
                            mime_type: s.mime_type.clone().into(),
                        }
                    });
                    let cursor_position = success
                        .cursor_position
                        .as_ref()
                        .map(|c| computer_use::Vector2I::new(c.x, c.y));
                    UseComputerResult::Success(computer_use::ActionResult {
                        screenshot,
                        cursor_position,
                    })
                }
                Some(api::use_computer_result::Result::Error(error)) => {
                    UseComputerResult::Error(error.message.clone())
                }
                None => UseComputerResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::UseComputer(use_computer_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::RequestComputerUseResult(result)) => {
            let request_result = match &result.result {
                Some(api::request_computer_use_result::Result::Approved(approved)) => {
                    match (approved, convert_api_platform(approved.platform)) {
                        (
                            api::request_computer_use_result::Approved {
                                screen_dimensions: Some(screen_dimensions),
                                initial_screenshot: Some(initial_screenshot),
                                ..
                            },
                            Some(platform),
                        ) => RequestComputerUseResult::Approved {
                            screenshot: computer_use::Screenshot {
                                width: initial_screenshot.width as usize,
                                height: initial_screenshot.height as usize,
                                original_width: screen_dimensions.width_px as usize,
                                original_height: screen_dimensions.height_px as usize,
                                data: initial_screenshot.data.clone(),
                                mime_type: initial_screenshot.mime_type.clone().into(),
                            },
                            platform,
                        },
                        _ => RequestComputerUseResult::Error(
                            "Missing screen dimensions, initial screenshot, or valid platform"
                                .to_string(),
                        ),
                    }
                }
                Some(api::request_computer_use_result::Result::Rejected(_)) => {
                    RequestComputerUseResult::Cancelled
                }
                Some(api::request_computer_use_result::Result::Error(error)) => {
                    RequestComputerUseResult::Error(error.message.clone())
                }
                None => RequestComputerUseResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::RequestComputerUse(request_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::FetchConversation(result)) => {
            let fetch_result = match &result.result {
                Some(api::fetch_conversation_result::Result::Success(success)) => {
                    FetchConversationResult::Success {
                        directory_path: success.directory_path.clone(),
                    }
                }
                Some(api::fetch_conversation_result::Result::Error(error)) => {
                    FetchConversationResult::Error(error.message.clone())
                }
                None => FetchConversationResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::FetchConversation(fetch_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::Server(_)) => {
            // Server results should not create exchanges - return None
            None
        }
        Some(ToolCallResultType::Cancel(_)) => {
            // Cancel results indicate the tool call was explicitly cancelled
            // Look up the original tool call to determine the correct result type
            create_cancelled_result_for_tool_call(task_id, &tool_call_id, tool_call_map, context)
        }
        Some(ToolCallResultType::Subagent(_)) => None,
        Some(ToolCallResultType::StartAgent(result)) => {
            let start_agent_result = match &result.result {
                Some(api::start_agent_result::Result::Success(success)) => {
                    StartAgentResult::Success {
                        agent_id: success.agent_id.clone(),
                        version: StartAgentVersion::V1,
                    }
                }
                Some(api::start_agent_result::Result::Error(error)) => StartAgentResult::Error {
                    error: error.error.clone(),
                    version: StartAgentVersion::V1,
                },
                None => StartAgentResult::Cancelled {
                    version: StartAgentVersion::V1,
                },
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::StartAgent(start_agent_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::StartAgentV2(result)) => {
            let start_agent_result = match &result.result {
                Some(api::start_agent_v2_result::Result::Success(success)) => {
                    StartAgentResult::Success {
                        agent_id: success.agent_id.clone(),
                        version: StartAgentVersion::V2,
                    }
                }
                Some(api::start_agent_v2_result::Result::Error(error)) => StartAgentResult::Error {
                    error: error.error.clone(),
                    version: StartAgentVersion::V2,
                },
                None => StartAgentResult::Cancelled {
                    version: StartAgentVersion::V2,
                },
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::StartAgent(start_agent_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::AskUserQuestion(result)) => {
            let ask_result = match &result.result {
                Some(warp_multi_agent_api::ask_user_question_result::Result::Success(success)) => {
                    AskUserQuestionResult::Success {
                        answers: success
                            .answers
                            .iter()
                            .map(|a| match &a.answer {
                                Some(AskUserQuestionAnswer::MultipleChoice(mc)) => {
                                    AskUserQuestionAnswerItem::Answered {
                                        question_id: a.question_id.clone(),
                                        selected_options: mc.selected_options.clone(),
                                        other_text: mc.other_text.clone(),
                                    }
                                }
                                Some(AskUserQuestionAnswer::Skipped(())) | None => {
                                    AskUserQuestionAnswerItem::Skipped {
                                        question_id: a.question_id.clone(),
                                    }
                                }
                            })
                            .collect(),
                    }
                }
                Some(warp_multi_agent_api::ask_user_question_result::Result::Error(err)) => {
                    AskUserQuestionResult::Error(err.message.clone())
                }
                None => AskUserQuestionResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::AskUserQuestion(ask_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::SendMessageToAgent(result)) => {
            let send_message_result = match &result.result {
                Some(api::send_message_to_agent_result::Result::Success(success)) => {
                    SendMessageToAgentResult::Success {
                        message_id: success.message_id.clone(),
                    }
                }
                Some(api::send_message_to_agent_result::Result::Error(error)) => {
                    SendMessageToAgentResult::Error(error.message.clone())
                }
                None => SendMessageToAgentResult::Cancelled,
            };

            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::SendMessageToAgent(send_message_result),
                },
                context,
            })
        }
        Some(ToolCallResultType::RunAgentsResult(result)) => {
            use ai::agent::action_result::{
                RunAgentsAgentOutcome, RunAgentsAgentOutcomeKind, RunAgentsLaunchedExecutionMode,
                RunAgentsResult,
            };
            let run_agents_result = match &result.outcome {
                Some(api::run_agents_result::Outcome::Launched(launched)) => {
                    let execution_mode = match &launched.resolved_execution_mode {
                        Some(api::run_agents_result::launched::ResolvedExecutionMode::Remote(
                            remote,
                        )) => RunAgentsLaunchedExecutionMode::Remote {
                            environment_id: remote.environment_id.clone(),
                            worker_host: remote.worker_host.clone(),
                            computer_use_enabled: remote.computer_use_enabled,
                        },
                        Some(api::run_agents_result::launched::ResolvedExecutionMode::Local(_))
                        | None => RunAgentsLaunchedExecutionMode::Local,
                    };
                    let agents = launched
                        .agents
                        .iter()
                        .map(|outcome| RunAgentsAgentOutcome {
                            name: outcome.name.clone(),
                            kind: match &outcome.result {
                                Some(api::run_agents_result::agent_outcome::Result::Launched(
                                    launched_agent,
                                )) => RunAgentsAgentOutcomeKind::Launched {
                                    agent_id: launched_agent.agent_id.clone(),
                                },
                                Some(api::run_agents_result::agent_outcome::Result::Failed(
                                    failed,
                                )) => RunAgentsAgentOutcomeKind::Failed {
                                    error: failed.error.clone(),
                                },
                                None => RunAgentsAgentOutcomeKind::Failed {
                                    error: String::new(),
                                },
                            },
                        })
                        .collect();
                    RunAgentsResult::Launched {
                        model_id: launched.resolved_model_id.clone(),
                        harness_type:
                            crate::ai::agent::api::convert_from::convert_run_agents_harness(
                                launched.resolved_harness.as_ref(),
                            )
                            .unwrap_or_default(),
                        execution_mode,
                        agents,
                    }
                }
                Some(api::run_agents_result::Outcome::Denied(denied)) => RunAgentsResult::Denied {
                    reason: denied.reason.clone(),
                },
                Some(api::run_agents_result::Outcome::Failure(failure)) => {
                    RunAgentsResult::Failure {
                        error: failure.error.clone(),
                    }
                }
                None => RunAgentsResult::Cancelled,
            };
            Some(AIAgentInput::ActionResult {
                result: AIAgentActionResult {
                    id: tool_call_id.into(),
                    task_id: task_id.clone(),
                    result: AIAgentActionResultType::RunAgents(run_agents_result),
                },
                context,
            })
        }
        // Deprecated/unused result types or absent result.
        Some(ToolCallResultType::SuggestCreatePlan(..))
        | Some(ToolCallResultType::SuggestPlan(..))
        | None => {
            log::warn!("No result present for tool call ID: {tool_call_id}");
            None
        }
    }
}

/// Create a cancelled result for a tool call based on the original tool call type
fn create_cancelled_result_for_tool_call(
    task_id: &TaskId,
    tool_call_id: &str,
    tool_call_map: &HashMap<String, &api::message::ToolCall>,
    context: Arc<[AIAgentContext]>,
) -> Option<AIAgentInput> {
    use api::message::tool_call::Tool as ToolType;

    let Some(original_tool_call) = tool_call_map.get(tool_call_id) else {
        log::warn!("No original tool call found for cancelled tool call ID: {tool_call_id}");
        // Default to RequestCommandOutput if we can't find the original tool call
        let cancelled_result = RequestCommandOutputResult::CancelledBeforeExecution;
        return Some(AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: tool_call_id.to_string().into(),
                task_id: task_id.clone(),
                result: AIAgentActionResultType::RequestCommandOutput(cancelled_result),
            },
            context,
        });
    };

    let Some(tool) = &original_tool_call.tool else {
        log::warn!("No tool found in original tool call for ID: {tool_call_id}");
        // Default to RequestCommandOutput if we can't find the tool type
        let cancelled_result = RequestCommandOutputResult::CancelledBeforeExecution;
        return Some(AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: tool_call_id.to_string().into(),
                task_id: task_id.clone(),
                result: AIAgentActionResultType::RequestCommandOutput(cancelled_result),
            },
            context,
        });
    };

    let result_type = match tool {
        ToolType::RunShellCommand(_) => AIAgentActionResultType::RequestCommandOutput(
            RequestCommandOutputResult::CancelledBeforeExecution,
        ),
        ToolType::WriteToLongRunningShellCommand(_) => {
            AIAgentActionResultType::WriteToLongRunningShellCommand(
                WriteToLongRunningShellCommandResult::Cancelled,
            )
        }
        ToolType::ReadFiles(_) => AIAgentActionResultType::ReadFiles(ReadFilesResult::Cancelled),
        ToolType::UploadFileArtifact(_) => {
            AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Cancelled)
        }
        ToolType::SearchCodebase(_) => {
            AIAgentActionResultType::SearchCodebase(SearchCodebaseResult::Cancelled)
        }
        ToolType::ApplyFileDiffs(_) => {
            AIAgentActionResultType::RequestFileEdits(RequestFileEditsResult::Cancelled)
        }
        ToolType::Grep(_) => AIAgentActionResultType::Grep(GrepResult::Cancelled),
        #[allow(deprecated)]
        ToolType::FileGlob(_) => AIAgentActionResultType::FileGlob(FileGlobResult::Cancelled),
        ToolType::FileGlobV2(_) => AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Cancelled),
        ToolType::ReadMcpResource(_) => {
            AIAgentActionResultType::ReadMCPResource(ReadMCPResourceResult::Cancelled)
        }
        ToolType::CallMcpTool(_) => {
            AIAgentActionResultType::CallMCPTool(CallMCPToolResult::Cancelled)
        }
        ToolType::ReadSkill(_) => AIAgentActionResultType::ReadSkill(ReadSkillResult::Cancelled),
        ToolType::SuggestNewConversation(_) => {
            AIAgentActionResultType::SuggestNewConversation(SuggestNewConversationResult::Cancelled)
        }
        ToolType::SuggestPrompt(_) => {
            AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled)
        }
        ToolType::OpenCodeReview(_) => AIAgentActionResultType::OpenCodeReview,
        ToolType::InsertReviewComments(_) => {
            AIAgentActionResultType::InsertReviewComments(InsertReviewCommentsResult::Cancelled)
        }
        ToolType::InitProject(_) => AIAgentActionResultType::InitProject,
        ToolType::ReadDocuments(_) => {
            AIAgentActionResultType::ReadDocuments(ReadDocumentsResult::Cancelled)
        }
        ToolType::EditDocuments(_) => {
            AIAgentActionResultType::EditDocuments(EditDocumentsResult::Cancelled)
        }
        ToolType::CreateDocuments(_) => {
            AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Cancelled)
        }
        ToolType::ReadShellCommandOutput(_) => {
            AIAgentActionResultType::ReadShellCommandOutput(ReadShellCommandOutputResult::Cancelled)
        }
        ToolType::TransferShellCommandControlToUser(_) => {
            AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Cancelled,
            )
        }
        ToolType::UseComputer(_) => {
            AIAgentActionResultType::UseComputer(UseComputerResult::Cancelled)
        }
        ToolType::RequestComputerUse(_) => {
            AIAgentActionResultType::RequestComputerUse(RequestComputerUseResult::Cancelled)
        }
        ToolType::FetchConversation(_) => {
            AIAgentActionResultType::FetchConversation(FetchConversationResult::Cancelled)
        }
        ToolType::Server(_) => {
            return None;
        }
        ToolType::Subagent(_) => return None,
        ToolType::StartAgent(_) => {
            AIAgentActionResultType::StartAgent(StartAgentResult::Cancelled {
                version: StartAgentVersion::V1,
            })
        }
        ToolType::StartAgentV2(_) => {
            AIAgentActionResultType::StartAgent(StartAgentResult::Cancelled {
                version: StartAgentVersion::V2,
            })
        }
        ToolType::AskUserQuestion(_) => {
            AIAgentActionResultType::AskUserQuestion(AskUserQuestionResult::Cancelled)
        }
        ToolType::SendMessageToAgent(_) => {
            AIAgentActionResultType::SendMessageToAgent(SendMessageToAgentResult::Cancelled)
        }
        ToolType::RunAgents(_) => {
            AIAgentActionResultType::RunAgents(ai::agent::action_result::RunAgentsResult::Cancelled)
        }
        // These tools are deprecated.
        ToolType::SuggestCreatePlan(_) | ToolType::SuggestPlan(_) => return None,
    };

    Some(AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: tool_call_id.to_string().into(),
            task_id: task_id.clone(),
            result: result_type,
        },
        context,
    })
}

fn create_exchange_from_messages(
    inputs: &[AIAgentInput],
    outputs: &[AIAgentOutputMessage],
    is_output_tool_call_canceled: bool,
    message_ids: &HashSet<String>,
    message_map: &HashMap<&str, &api::Message>,
    server_output_id: Option<&str>,
) -> Option<AIAgentExchange> {
    // Allow exchanges with only outputs (e.g., when returning from a subtask).
    if inputs.is_empty() && outputs.is_empty() {
        return None;
    }

    let exchange_id = AIAgentExchangeId::new();

    // get the exchange's start time from the latest input's context
    let start_time = inputs
        .last()
        .and_then(|input| input.context())
        .and_then(|contexts| {
            contexts.iter().find_map(|context| match context {
                AIAgentContext::CurrentTime { current_time } => Some(*current_time),
                _ => None,
            })
        })
        // Fall back to any timestamp from the messages in this exchange
        .or_else(|| {
            message_ids.iter().find_map(|message_id| {
                message_map.get(message_id.as_str()).and_then(|message| {
                    message.timestamp.as_ref().map(|timestamp| {
                        proto_timestamp_to_local_datetime(timestamp.seconds, timestamp.nanos)
                    })
                })
            })
        })
        .unwrap_or_default();

    // Get the exchange's finish time from the last message timestamp in this exchange.
    let finish_time = message_ids
        .iter()
        .filter_map(|message_id| {
            message_map.get(message_id.as_str()).and_then(|message| {
                message.timestamp.as_ref().map(|timestamp| {
                    proto_timestamp_to_local_datetime(timestamp.seconds, timestamp.nanos)
                })
            })
        })
        .max();

    // Compute time to first token from the first non-input message in this exchange,
    // reusing the same logic we expose for restored/forked conversations.
    let time_to_first_token_ms = compute_time_to_first_token_ms_from_messages(
        start_time,
        message_ids
            .iter()
            .filter_map(|message_id| message_map.get(message_id.as_str()).copied()),
    );

    // Collect all citations from the output messages.
    let mut citations = Vec::new();
    for output in outputs {
        citations.extend(output.citations.clone());
    }
    let model_used = message_ids
        .iter()
        .filter_map(|message_id| message_map.get(message_id.as_str()))
        .find_map(|message| {
            message.message.as_ref().and_then(|message| match message {
                api::message::Message::ModelUsed(model_used) => Some(model_used),
                _ => None,
            })
        });

    // Create AIAgentOutput from the output messages
    let ai_output = AIAgentOutput {
        messages: outputs.to_vec(),
        citations,
        api_metadata_bytes: None,
        server_output_id: server_output_id.map(|id| ServerOutputId::new(id.to_owned())),
        suggestions: None,
        telemetry_events: vec![],
        model_info: model_used.map(|model| OutputModelInfo {
            model_id: model.model_id.clone().into(),
            display_name: model.model_display_name.clone(),
            is_fallback: model.is_fallback,
        }),
        request_cost: None,
    };

    // There is a special case where an exchange consists of only ActionResults with no outputs
    // (i.e. if a passive code diff is accepted and the agent doesn't follow up, which it shouldn't).
    // In this case, we should not mark the exchange as cancelled.
    let last_input_is_action_result = inputs
        .last()
        .map(|input| matches!(input, AIAgentInput::ActionResult { .. }))
        .unwrap_or(false);

    // Create exchange with finished status
    let output_status = if outputs.is_empty() && !last_input_is_action_result {
        AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Cancelled {
                output: None,
                reason: CancellationReason::ManuallyCancelled,
            },
        }
    } else if is_output_tool_call_canceled {
        AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Cancelled {
                output: Some(Shared::new(ai_output)),
                reason: CancellationReason::ManuallyCancelled,
            },
        }
    } else {
        AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(ai_output),
            },
        }
    };

    // Extract working directory from the first input that has a Directory context
    let working_directory = inputs.iter().find_map(|input| {
        input.context().and_then(|contexts| {
            contexts.iter().find_map(|context| {
                if let AIAgentContext::Directory { pwd, .. } = context {
                    pwd.clone()
                } else {
                    None
                }
            })
        })
    });

    // Use a default LLM ID for restored exchanges
    let default_model_id = LLMId::from("auto");

    Some(AIAgentExchange {
        id: exchange_id,
        input: inputs.to_vec(),
        output_status,
        start_time,
        finish_time,
        time_to_first_token_ms,
        working_directory,
        model_id: default_model_id.clone(),
        request_cost: None,
        coding_model_id: default_model_id.clone(),
        cli_agent_model_id: default_model_id.clone(),
        computer_use_model_id: default_model_id,
        response_initiator: None,
        added_message_ids: message_ids.iter().map(|s| s.clone().into()).collect(),
    })
}

/// Compute the time to first token for an exchange from a sequence of server messages.
pub(crate) fn compute_time_to_first_token_ms_from_messages<'a, I>(
    start_time: DateTime<Local>,
    messages: I,
) -> Option<i64>
where
    I: Iterator<Item = &'a api::Message>,
{
    let first_output_time = messages
        .filter_map(|message| {
            let msg = message.message.as_ref()?;
            match msg {
                // Messages treated as inputs in create_exchange_from_messages
                api::message::Message::UserQuery(_)
                | api::message::Message::SystemQuery(_)
                | api::message::Message::ToolCallResult(_)
                | api::message::Message::UpdateTodos(_)
                | api::message::Message::MessagesReceivedFromAgents(_)
                | api::message::Message::EventsFromAgents(_)
                | api::message::Message::PassiveSuggestionResult(_) => None,
                // Anything else is considered agent/stream activity we want to measure
                api::message::Message::AgentOutput(_)
                | api::message::Message::AgentReasoning(_)
                | api::message::Message::Summarization(_)
                | api::message::Message::ToolCall(_)
                | api::message::Message::ServerEvent(_)
                | api::message::Message::UpdateReviewComments(_)
                | api::message::Message::CodeReview(_)
                | api::message::Message::WebSearch(_)
                | api::message::Message::WebFetch(_)
                | api::message::Message::DebugOutput(_)
                | api::message::Message::ArtifactEvent(_)
                | api::message::Message::InvokeSkill(_)
                | api::message::Message::ModelUsed(_)
                | api::message::Message::OrchestrationConfigSnapshot(_) => {
                    message.timestamp.as_ref().map(|timestamp| {
                        proto_timestamp_to_local_datetime(timestamp.seconds, timestamp.nanos)
                    })
                }
            }
        })
        .min()?;

    if first_output_time < start_time {
        return None;
    }

    Some(
        first_output_time
            .signed_duration_since(start_time)
            .num_milliseconds()
            .max(0),
    )
}

impl From<api::ExecutedShellCommand> for BlockContext {
    fn from(cmd: api::ExecutedShellCommand) -> Self {
        BlockContext {
            id: cmd.command_id.into(),
            index: BlockIndex::from(0),
            command: cmd.command,
            output: cmd.output,
            exit_code: ExitCode::from(cmd.exit_code),
            is_auto_attached: cmd.is_auto_attached,
            started_ts: cmd
                .started_ts
                .as_ref()
                .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
            finished_ts: cmd
                .finished_ts
                .as_ref()
                .map(|ts| proto_timestamp_to_local_datetime(ts.seconds, ts.nanos)),
            pwd: None,
            shell: None,
            username: None,
            hostname: None,
            git_branch: None,
            os: None,
            session_id: None,
        }
    }
}

/// Converts a persisted `PassiveSuggestionResult` message back to an
/// [`AIAgentInput::PassiveSuggestionResult`] so the trigger block information
/// is available after conversation restoration.
fn convert_passive_suggestion_result_to_input(
    passive_result: &api::message::PassiveSuggestionResult,
) -> Option<AIAgentInput> {
    let api_result = passive_result.result.as_ref()?;

    let trigger = match &api_result.trigger {
        Some(api::passive_suggestion_result_type::Trigger::ExecutedShellCommand(cmd)) => {
            PassiveSuggestionTrigger::ShellCommandCompleted(ShellCommandCompletedTrigger {
                executed_shell_command: Box::new(cmd.clone().into()),
                // Relevant files are not persisted in the proto message.
                relevant_files: vec![],
            })
        }
        Some(api::passive_suggestion_result_type::Trigger::AgentResponseCompleted(_)) => {
            // The exchange_id is not stored in the proto; use a placeholder.
            PassiveSuggestionTrigger::AgentResponseCompleted {
                exchange_id: AIAgentExchangeId::default(),
            }
        }
        None => return None,
    };

    let suggestion = match &api_result.suggestion {
        Some(api::passive_suggestion_result_type::Suggestion::Prompt(p)) => {
            PassiveSuggestionResultType::Prompt {
                prompt: p.prompt.clone(),
            }
        }
        Some(api::passive_suggestion_result_type::Suggestion::CodeDiff(cd)) => {
            PassiveSuggestionResultType::CodeDiff {
                diffs: cd
                    .diffs
                    .iter()
                    .map(|d| PassiveCodeDiffEntry {
                        file_path: d.file_path.clone(),
                        search: d.search.clone(),
                        replace: d.replace.clone(),
                    })
                    .collect(),
                summary: cd.summary.clone(),
                accepted: cd.accepted,
            }
        }
        None => return None,
    };

    let context = convert_input_context(passive_result.context.as_ref());

    Some(AIAgentInput::PassiveSuggestionResult {
        trigger: Some(trigger),
        suggestion,
        context,
    })
}
pub(crate) fn proto_timestamp_to_local_datetime(seconds: i64, nanos: i32) -> DateTime<Local> {
    let nanos = if nanos < 0 { 0 } else { nanos as u32 };

    Local
        .timestamp_opt(seconds, nanos)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap())
}

impl From<String> for crate::ai::agent::MessageId {
    fn from(s: String) -> Self {
        crate::ai::agent::MessageId(s)
    }
}

fn convert_api_platform(platform: i32) -> Option<computer_use::Platform> {
    use api::request_computer_use_result::approved::Platform;
    match Platform::try_from(platform) {
        Ok(Platform::Macos) => Some(computer_use::Platform::Mac),
        Ok(Platform::Windows) => Some(computer_use::Platform::Windows),
        Ok(Platform::LinuxX11) => Some(computer_use::Platform::LinuxX11),
        Ok(Platform::LinuxWayland) => Some(computer_use::Platform::LinuxWayland),
        Err(_) => {
            log::warn!("Unknown platform value: {platform}");
            None
        }
    }
}

#[cfg(test)]
#[path = "convert_conversation_tests.rs"]
mod tests;
