//! Conversions from MAA API types to application types.
use std::collections::HashMap;
use std::time::Duration;

use ai::agent::action::{LifecycleEventType as StartAgentLifecycleEventType, ReadSkillRequest};
use ai::agent::action_result::StartAgentVersion;
use ai::agent::convert::ToolToAIAgentActionError;
use ai::agent::UnknownCitationTypeError;
use ai::skills::{
    skill_reference_from_api_skill_ref, skill_reference_from_read_skill_ref, SkillPathOrigin,
};
use api::ask_user_question::question::QuestionType;
use warp_core::channel::ChannelState;
use warp_multi_agent_api as api;

use crate::ai::agent::api::convert_conversation::{
    convert_input_context, convert_tool_call_result_to_input,
};
use crate::ai::agent::comment::CodeReview;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::todos::AIAgentTodoList;
use crate::ai::agent::util::parse_markdown_into_text_and_code_sections;
use crate::ai::agent::{
    AIAgentAction, AIAgentActionType, AIAgentAttachment, AIAgentCitation, AIAgentInput,
    AIAgentOutputMessage, AIAgentText, AIAgentTodo, ArtifactCreatedData, CloneRepositoryURL,
    MessageId, RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest,
    StartAgentExecutionMode, SubagentCall, SubagentType, SuggestedAgentModeWorkflow, SuggestedRule,
    Suggestions, SummarizationType, TodoOperation, UserQueryMode, WebFetchStatus, WebSearchStatus,
};
use crate::ai::artifact_download::sanitized_basename;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentVersion};

impl TryFrom<api::Attachment> for AIAgentAttachment {
    type Error = anyhow::Error;

    fn try_from(attachment: api::Attachment) -> Result<Self, Self::Error> {
        match attachment.value {
            Some(api::attachment::Value::FilePathReference(fpr)) => {
                Ok(AIAgentAttachment::FilePathReference {
                    file_id: String::new(),
                    file_name: fpr
                        .file_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&fpr.file_path)
                        .to_string(),
                    file_path: fpr.file_path,
                })
            }
            _ => anyhow::bail!("Unsupported attachment type for conversion"),
        }
    }
}

fn convert_read_skill(
    read_skill: api::message::tool_call::ReadSkill,
    skill_path_origin: &SkillPathOrigin,
) -> Result<AIAgentActionType, ToolToAIAgentActionError> {
    let Some(reference) = read_skill.skill_reference else {
        return Err(ToolToAIAgentActionError::MissingSkillReference);
    };
    let skill = skill_reference_from_read_skill_ref(reference, skill_path_origin)
        .map_err(|_| ToolToAIAgentActionError::MissingSkillReference)?;
    Ok(AIAgentActionType::ReadSkill(ReadSkillRequest { skill }))
}

/// Converts proto UserQueryMode to the internal UserQueryMode type
pub(crate) fn convert_user_query_mode(mode: Option<&api::UserQueryMode>) -> UserQueryMode {
    let Some(mode) = mode else {
        return UserQueryMode::default();
    };

    match &mode.r#type {
        Some(api::user_query_mode::Type::Plan(_)) => UserQueryMode::Plan,
        Some(api::user_query_mode::Type::Orchestrate(_)) => UserQueryMode::Orchestrate,
        None => UserQueryMode::Normal,
    }
}

fn convert_start_agent_lifecycle_event_type(
    event_type: i32,
) -> Option<StartAgentLifecycleEventType> {
    let event_type = StartAgentLifecycleEventType::try_from(event_type).ok()?;
    (event_type != StartAgentLifecycleEventType::Unspecified).then_some(event_type)
}

fn convert_start_agent_v2_harness_type(
    harness: Option<api::start_agent_v2::execution_mode::Harness>,
) -> Option<String> {
    harness
        .map(|harness| harness.r#type)
        .filter(|harness_type| !harness_type.trim().is_empty())
}

/// Maps the proto `Harness` oneof to a client-side string identifier
/// (e.g. "oz", "claude"). Returns `None` for an unset variant.
pub(crate) fn convert_run_agents_harness(harness: Option<&api::Harness>) -> Option<String> {
    let variant = harness?.variant.as_ref()?;
    Some(
        match variant {
            api::harness::Variant::Oz(_) => "oz",
            api::harness::Variant::ClaudeCode(_) => "claude",
            api::harness::Variant::OpenCode(_) => "opencode",
            api::harness::Variant::Gemini(_) => "gemini",
            api::harness::Variant::Codex(_) => "codex",
        }
        .to_string(),
    )
}

fn convert_start_agent_execution_mode(
    execution_mode: Option<api::start_agent::ExecutionMode>,
) -> StartAgentExecutionMode {
    match execution_mode.and_then(|execution_mode| execution_mode.mode) {
        Some(api::start_agent::execution_mode::Mode::Remote(remote)) => {
            StartAgentExecutionMode::remote_with_defaults(remote.environment_id)
        }
        Some(api::start_agent::execution_mode::Mode::Local(_)) | None => {
            StartAgentExecutionMode::local_with_defaults()
        }
    }
}

fn convert_run_agents_execution_mode(
    execution_mode: Option<api::run_agents::ExecutionMode>,
) -> RunAgentsExecutionMode {
    match execution_mode {
        Some(api::run_agents::ExecutionMode::Remote(remote)) => RunAgentsExecutionMode::Remote {
            environment_id: remote.environment_id,
            worker_host: remote.worker_host,
            computer_use_enabled: remote.computer_use_enabled,
        },
        Some(api::run_agents::ExecutionMode::Local(_)) | None => RunAgentsExecutionMode::Local,
    }
}

fn convert_run_agents(
    run_agents: api::RunAgents,
    skill_path_origin: &SkillPathOrigin,
) -> AIAgentActionType {
    let api::RunAgents {
        summary,
        base_prompt,
        skills,
        model_id,
        harness,
        agent_run_configs,
        execution_mode,
        plan_id,
    } = run_agents;
    AIAgentActionType::RunAgents(RunAgentsRequest {
        summary,
        base_prompt,
        skills: skills
            .into_iter()
            .filter_map(|skill| skill_reference_from_api_skill_ref(skill, skill_path_origin))
            .collect(),
        model_id,
        harness_type: convert_run_agents_harness(harness.as_ref()).unwrap_or_default(),
        execution_mode: convert_run_agents_execution_mode(execution_mode),
        agent_run_configs: agent_run_configs
            .into_iter()
            .map(|config| RunAgentsAgentRunConfig {
                name: config.name,
                prompt: config.prompt,
                title: config.title,
            })
            .collect(),
        plan_id,
        // Auth secret is a client-side dispatch concern populated by the
        // confirmation card from `CloudAgentSettings.last_selected_auth_secret`
        // before Accept. The proto does not carry it.
        harness_auth_secret_name: None,
    })
}

fn convert_start_agent_v2_execution_mode(
    execution_mode: Option<api::start_agent_v2::ExecutionMode>,
    skill_path_origin: &SkillPathOrigin,
) -> StartAgentExecutionMode {
    match execution_mode.and_then(|execution_mode| execution_mode.mode) {
        Some(api::start_agent_v2::execution_mode::Mode::Remote(remote)) => {
            StartAgentExecutionMode::Remote {
                environment_id: remote.environment_id,
                skill_references: remote
                    .skills
                    .into_iter()
                    .filter_map(|skill| {
                        skill_reference_from_api_skill_ref(skill, skill_path_origin)
                    })
                    .collect(),
                model_id: remote.model_id,
                computer_use_enabled: remote.computer_use_enabled,
                worker_host: remote.worker_host,
                harness_type: convert_start_agent_v2_harness_type(remote.harness)
                    .unwrap_or_default(),
                title: remote.title,
                // Auth secret is plumbed client-side via `RunAgentsRequest`;
                // StartAgentV2 from the server never carries it.
                auth_secret_name: None,
            }
        }
        Some(api::start_agent_v2::execution_mode::Mode::Local(local)) => {
            convert_start_agent_v2_harness_type(local.harness)
                .map(StartAgentExecutionMode::local_harness)
                .unwrap_or_else(StartAgentExecutionMode::local_with_defaults)
        }
        None => StartAgentExecutionMode::local_with_defaults(),
    }
}

/// Unexpected errors when trying to convert an [`api::Message`] to an [`AIAgentOutputMessage`].
#[derive(Debug, thiserror::Error)]
pub enum MessageToAIAgentOutputMessageError {
    #[error("Missing expected message")]
    MissingMessage,
    #[error("Error converting tool to action: {0:?}")]
    ToolError(#[from] ToolToAIAgentActionError),
    #[error("Error converting citation: {0:?}")]
    CitationError(#[from] UnknownCitationTypeError),
}

/// Successful result when trying to convert an [`api::message::ToolCall`] to an [`AIAgentAction`].
#[allow(clippy::large_enum_variant)]
pub enum MaybeAIAgentOutputMessage {
    /// There is a mapping to a client output message.
    Message(AIAgentOutputMessage),
    /// We tried to parse a message that we don't care about.
    NoClientRepresentation,
}

/// Successful result when trying to convert an [`api::message::ToolCall`] to an [`AIAgentAction`].
#[allow(clippy::large_enum_variant)]
enum MaybeAIAgentAction {
    /// There is a mapping to a client action.
    Action(AIAgentAction),
    Subagent(SubagentCall),
    /// We tried to parse a tool call that we don't care about.
    NoClientRepresentation,
}

pub struct ConversionParams<'a> {
    pub task_id: &'a TaskId,
    pub current_todo_list: Option<&'a AIAgentTodoList>,
    pub active_code_review: Option<&'a CodeReview>,
    pub skill_path_origin: &'a SkillPathOrigin,
}

/// Trait for converting an [`api::Message`] to an [`AIAgentOutputMessage`].
pub trait ConvertAPIMessageToClientOutputMessage {
    fn to_client_output_message(
        self,
        params: ConversionParams,
    ) -> Result<MaybeAIAgentOutputMessage, MessageToAIAgentOutputMessageError>;
}

impl ConvertAPIMessageToClientOutputMessage for api::Message {
    fn to_client_output_message(
        self,
        params: ConversionParams,
    ) -> Result<MaybeAIAgentOutputMessage, MessageToAIAgentOutputMessageError> {
        let Some(message) = self.message else {
            // In shared-session streams we can receive skeleton placeholder task messages without payloads.
            // Treat them as having no client representation rather than erroring and aborting ingestion entirely.
            return Ok(MaybeAIAgentOutputMessage::NoClientRepresentation);
        };

        let citations = self
            .citations
            .iter()
            .map(|citation| (*citation).clone().try_into())
            .collect::<Result<Vec<AIAgentCitation>, UnknownCitationTypeError>>()?;

        match message {
            api::message::Message::AgentOutput(output) => Ok(MaybeAIAgentOutputMessage::Message(
                AIAgentOutputMessage::text(MessageId::new(self.id), output.into())
                    .with_citations(citations),
            )),
            api::message::Message::AgentReasoning(reasoning) => {
                let duration = reasoning
                    .finished_duration
                    .map(|d| Duration::from_secs(d.seconds as u64));
                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::reasoning(
                        MessageId::new(self.id),
                        reasoning.into(),
                        duration,
                    ),
                ))
            }
            api::message::Message::ToolCall(tool_call) => match tool_call.to_action(params)? {
                MaybeAIAgentAction::Action(action) => Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::action(MessageId::new(self.id), action)
                        .with_citations(citations),
                )),
                MaybeAIAgentAction::Subagent(subagent) => Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::subagent(MessageId::new(self.id), subagent)
                        .with_citations(citations),
                )),
                MaybeAIAgentAction::NoClientRepresentation => {
                    Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                }
            },
            api::message::Message::WebSearch(web_search) => {
                let status = match &web_search.status {
                    Some(api::message::web_search::Status {
                        r#type: Some(api::message::web_search::status::Type::Searching(searching)),
                    }) => WebSearchStatus::Searching {
                        query: if searching.query.is_empty() {
                            None
                        } else {
                            Some(searching.query.clone())
                        },
                    },
                    Some(api::message::web_search::Status {
                        r#type: Some(api::message::web_search::status::Type::Success(success)),
                    }) => WebSearchStatus::Success {
                        query: success.query.clone(),
                        pages: success
                            .pages
                            .iter()
                            .map(|p| (p.url.clone(), p.title.clone()))
                            .collect(),
                    },
                    Some(api::message::web_search::Status {
                        r#type: Some(api::message::web_search::status::Type::Error(_)),
                    }) => {
                        // Error type doesn't have a query field currently, use empty string
                        WebSearchStatus::Error {
                            query: String::new(),
                        }
                    }
                    _ => {
                        // Unknown or missing status
                        return Ok(MaybeAIAgentOutputMessage::NoClientRepresentation);
                    }
                };

                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::web_search(MessageId::new(self.id), status)
                        .with_citations(citations),
                ))
            }
            api::message::Message::WebFetch(web_fetch) => {
                let status = match &web_fetch.status {
                    Some(api::message::web_fetch::Status {
                        r#type: Some(api::message::web_fetch::status::Type::Fetching(fetching)),
                    }) => WebFetchStatus::Fetching {
                        urls: fetching.urls.clone(),
                    },
                    Some(api::message::web_fetch::Status {
                        r#type: Some(api::message::web_fetch::status::Type::Success(success)),
                    }) => WebFetchStatus::Success {
                        pages: success
                            .pages
                            .iter()
                            .map(|p| (p.url.clone(), p.title.clone(), p.success))
                            .collect(),
                    },
                    Some(api::message::web_fetch::Status {
                        r#type: Some(api::message::web_fetch::status::Type::Error(_)),
                    }) => WebFetchStatus::Error,
                    _ => {
                        // Unknown or missing status
                        return Ok(MaybeAIAgentOutputMessage::NoClientRepresentation);
                    }
                };

                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::web_fetch(MessageId::new(self.id), status)
                        .with_citations(citations),
                ))
            }
            api::message::Message::ModelUsed(_) => {
                Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
            }
            api::message::Message::UpdateTodos(update_todos) => {
                if let Some(operation) = update_todos.operation {
                    match operation {
                        api::message::update_todos::Operation::CreateTodoList(create_todo_list) => {
                            Ok(MaybeAIAgentOutputMessage::Message(
                                AIAgentOutputMessage::todo_operation(
                                    MessageId::new(self.id),
                                    TodoOperation::UpdateTodos {
                                        todos: create_todo_list
                                            .initial_todos
                                            .into_iter()
                                            .map(Into::into)
                                            .collect(),
                                    },
                                )
                                .with_citations(citations),
                            ))
                        }
                        api::message::update_todos::Operation::UpdatePendingTodos(
                            update_pending_todos,
                        ) => Ok(MaybeAIAgentOutputMessage::Message(
                            AIAgentOutputMessage::todo_operation(
                                MessageId::new(self.id),
                                TodoOperation::UpdateTodos {
                                    todos: params
                                        .current_todo_list
                                        .iter()
                                        .flat_map(|list| list.completed_items().iter().cloned())
                                        .chain(
                                            update_pending_todos
                                                .updated_pending_todos
                                                .into_iter()
                                                .map(Into::into),
                                        )
                                        .collect(),
                                },
                            )
                            .with_citations(citations),
                        )),
                        api::message::update_todos::Operation::MarkTodosCompleted(
                            mark_todos_completed,
                        ) => {
                            if mark_todos_completed.todo_ids.is_empty() {
                                Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                            } else {
                                // This is a mark as completed operation
                                Ok(MaybeAIAgentOutputMessage::Message(
                                    AIAgentOutputMessage::todo_operation(
                                        MessageId::new(self.id),
                                        TodoOperation::MarkAsCompleted {
                                            completed_todos: mark_todos_completed
                                                .todo_ids
                                                .into_iter()
                                                .filter_map(|todo_id| {
                                                    params.current_todo_list.and_then(|todo_list| {
                                                        todo_list
                                                            .completed_items()
                                                            .iter()
                                                            .find(|item| {
                                                                item.id.as_ref() == todo_id.as_str()
                                                            })
                                                            .cloned()
                                                    })
                                                })
                                                .collect(),
                                        },
                                    )
                                    .with_citations(citations),
                                ))
                            }
                        }
                    }
                } else {
                    Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                }
            }
            api::message::Message::Summarization(summarization) => {
                let duration = summarization
                    .finished_duration
                    .map(|d| Duration::from_secs(d.seconds as u64));
                let (text, summarization_type, token_count) = match summarization.summary_type {
                    Some(api::message::summarization::SummaryType::ConversationSummary(
                        conv_summary,
                    )) => {
                        let token_count = if conv_summary.token_count > 0 {
                            Some(conv_summary.token_count as u32)
                        } else {
                            None
                        };
                        let text = if !conv_summary.summary.is_empty() {
                            AIAgentText {
                                sections: parse_markdown_into_text_and_code_sections(
                                    &conv_summary.summary,
                                ),
                            }
                        } else {
                            AIAgentText { sections: vec![] }
                        };
                        (text, SummarizationType::ConversationSummary, token_count)
                    }
                    Some(api::message::summarization::SummaryType::ToolCallResultSummary(_)) => (
                        AIAgentText { sections: vec![] },
                        SummarizationType::ToolCallResultSummary,
                        None,
                    ),
                    None => {
                        // Default to ConversationSummary if not specified
                        (
                            AIAgentText { sections: vec![] },
                            SummarizationType::ConversationSummary,
                            None,
                        )
                    }
                };
                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::summarization(
                        MessageId::new(self.id),
                        text,
                        duration,
                        summarization_type,
                        token_count,
                    ),
                ))
            }
            api::message::Message::UpdateReviewComments(update_comments) => {
                if let Some(operation) = update_comments.operation {
                    match operation {
                        api::message::update_review_comments::Operation::AddressReviewComments(
                            address_comments,
                        ) => {
                            if let Some(current_comments) = params.active_code_review {
                                let addressed_comments = current_comments
                                    .addressed_comments
                                    .iter()
                                    .filter(|comment| {
                                        address_comments
                                            .comment_ids
                                            .iter()
                                            .any(|id| id == &comment.id.to_string())
                                    })
                                    .cloned()
                                    .collect();
                                Ok(MaybeAIAgentOutputMessage::Message(
                                    AIAgentOutputMessage::comments_addressed(
                                        MessageId::new(self.id),
                                        addressed_comments,
                                    )
                                    .with_citations(citations),
                                ))
                            } else {
                                Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                            }
                        }
                    }
                } else {
                    Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                }
            }
            api::message::Message::DebugOutput(debug_output) => {
                if ChannelState::enable_debug_features() {
                    Ok(MaybeAIAgentOutputMessage::Message(
                        AIAgentOutputMessage::debug_output(
                            MessageId::new(self.id),
                            debug_output.text,
                        ),
                    ))
                } else {
                    Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                }
            }
            api::message::Message::ArtifactEvent(artifact_event) => match artifact_event.event {
                Some(api::message::artifact_event::Event::Created(artifact_created)) => {
                    match artifact_created.artifact {
                        Some(
                            api::message::artifact_event::artifact_created::Artifact::PullRequest(
                                pr,
                            ),
                        ) => Ok(MaybeAIAgentOutputMessage::Message(
                            AIAgentOutputMessage::artifact_created(
                                MessageId::new(self.id),
                                ArtifactCreatedData::PullRequest {
                                    url: pr.url,
                                    branch: pr.branch,
                                },
                            )
                            .with_citations(citations),
                        )),
                        Some(
                            api::message::artifact_event::artifact_created::Artifact::Screenshot(
                                screenshot,
                            ),
                        ) => Ok(MaybeAIAgentOutputMessage::Message(
                            AIAgentOutputMessage::artifact_created(
                                MessageId::new(self.id),
                                ArtifactCreatedData::Screenshot {
                                    artifact_uid: screenshot.artifact_uid,
                                    mime_type: screenshot.mime_type,
                                    description: if screenshot.description.is_empty() {
                                        None
                                    } else {
                                        Some(screenshot.description)
                                    },
                                },
                            )
                            .with_citations(citations),
                        )),
                        Some(api::message::artifact_event::artifact_created::Artifact::File(
                            file,
                        )) => Ok(MaybeAIAgentOutputMessage::Message(
                            AIAgentOutputMessage::artifact_created(
                                MessageId::new(self.id),
                                ArtifactCreatedData::File {
                                    artifact_uid: file.artifact_uid,
                                    filename: sanitized_basename(&file.filepath)
                                        .unwrap_or_else(|| file.filepath.clone()),
                                    filepath: file.filepath,
                                    mime_type: file.mime_type,
                                    description: if file.description.is_empty() {
                                        None
                                    } else {
                                        Some(file.description)
                                    },
                                    size_bytes: file.size_bytes,
                                },
                            )
                            .with_citations(citations),
                        )),
                        None => Ok(MaybeAIAgentOutputMessage::NoClientRepresentation),
                    }
                }
                Some(api::message::artifact_event::Event::ForkArtifacts(_)) | None => {
                    Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
                }
            },
            api::message::Message::MessagesReceivedFromAgents(messages_received_from_agents) => {
                let messages = messages_received_from_agents
                    .messages
                    .into_iter()
                    .map(|msg| crate::ai::agent::ReceivedMessageDisplay {
                        message_id: msg.message_id,
                        sender_agent_id: msg.sender_agent_id,
                        addresses: msg.addresses,
                        subject: msg.subject,
                        message_body: msg.message_body,
                    })
                    .collect();
                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::messages_received_from_agents(
                        MessageId::new(self.id),
                        messages,
                    )
                    .with_citations(citations),
                ))
            }
            api::message::Message::EventsFromAgents(events) => {
                let event_ids = events
                    .agent_events
                    .iter()
                    .map(|e| e.event_id.clone())
                    .collect();
                Ok(MaybeAIAgentOutputMessage::Message(
                    AIAgentOutputMessage::events_from_agents(MessageId::new(self.id), event_ids)
                        .with_citations(citations),
                ))
            }
            // These messages don't indicate an error but they don't translate to a client-side output message.
            api::message::Message::UserQuery(_)
            | api::message::Message::SystemQuery(_)
            | api::message::Message::ToolCallResult(_)
            | api::message::Message::CodeReview(_)
            | api::message::Message::ServerEvent(_)
            | api::message::Message::InvokeSkill(_)
            | api::message::Message::PassiveSuggestionResult(_)
            // Stage 2 plan-card config snapshot: hydrated separately by the
            // plan card's `AIDocumentModel` subscription, not via the
            // exchange/output stream. No client output message representation.
            | api::message::Message::OrchestrationConfigSnapshot(_) => {
                Ok(MaybeAIAgentOutputMessage::NoClientRepresentation)
            }
        }
    }
}

impl From<api::message::AgentOutput> for AIAgentText {
    fn from(value: api::message::AgentOutput) -> Self {
        AIAgentText {
            sections: parse_markdown_into_text_and_code_sections(value.text.as_str()),
        }
    }
}

impl From<api::message::AgentReasoning> for AIAgentText {
    fn from(value: api::message::AgentReasoning) -> Self {
        AIAgentText {
            sections: parse_markdown_into_text_and_code_sections(value.reasoning.as_str()),
        }
    }
}

/// Trait for converting an [`api::Message`] to an [`AIAgentOutputMessage`].
trait ConvertAPIToolCallToAIAgentAction {
    fn to_action(
        self,
        params: ConversionParams,
    ) -> Result<MaybeAIAgentAction, ToolToAIAgentActionError>;
}

/// Tries to convert an [`api::message::ToolCall`] to an [`AIAgentAction`].
///
/// A [`Result::Error`] indicates an unexpected problem, while [`Ok(None)`]
/// indicates a tool call that we aren't expected to parse.
impl ConvertAPIToolCallToAIAgentAction for api::message::ToolCall {
    fn to_action(
        self,
        params: ConversionParams,
    ) -> Result<MaybeAIAgentAction, ToolToAIAgentActionError> {
        let Some(tool) = self.tool else {
            return Err(ToolToAIAgentActionError::MissingTool);
        };

        let create_standard_action = |action: AIAgentActionType| {
            Ok(MaybeAIAgentAction::Action(AIAgentAction {
                id: self.tool_call_id.clone().into(),
                task_id: params.task_id.clone(),
                action,
                requires_result: true,
            }))
        };

        match tool {
            api::message::tool_call::Tool::RunShellCommand(run_shell_command) => {
                create_standard_action(run_shell_command.into())
            }
            api::message::tool_call::Tool::WriteToLongRunningShellCommand(
                write_to_long_running_shell_command,
            ) => create_standard_action(write_to_long_running_shell_command.into()),
            api::message::tool_call::Tool::ReadFiles(read_files) => {
                create_standard_action(read_files.into())
            }
            api::message::tool_call::Tool::UploadFileArtifact(upload_file_artifact) => {
                create_standard_action(upload_file_artifact.try_into()?)
            }
            api::message::tool_call::Tool::SearchCodebase(search_codebase) => {
                create_standard_action(search_codebase.into())
            }
            api::message::tool_call::Tool::Grep(grep) => create_standard_action(grep.into()),
            #[allow(deprecated)]
            api::message::tool_call::Tool::FileGlob(glob) => create_standard_action(glob.into()),
            api::message::tool_call::Tool::FileGlobV2(glob) => create_standard_action(glob.into()),
            api::message::tool_call::Tool::ApplyFileDiffs(apply_file_diffs) => {
                create_standard_action(apply_file_diffs.into())
            }
            api::message::tool_call::Tool::ReadMcpResource(read_mcp_resource) => {
                create_standard_action(read_mcp_resource.into())
            }
            api::message::tool_call::Tool::CallMcpTool(call_mcp_tool) => {
                match call_mcp_tool.try_into() {
                    Ok(call_mcp_tool_action) => create_standard_action(call_mcp_tool_action),
                    Err(error) => Err(error),
                }
            }
            api::message::tool_call::Tool::SuggestNewConversation(suggest_new_conversation) => {
                create_standard_action(suggest_new_conversation.into())
            }
            api::message::tool_call::Tool::SuggestPrompt(suggest_prompt) => {
                match suggest_prompt.try_into() {
                    Ok(suggest_prompt_action) => create_standard_action(suggest_prompt_action),
                    Err(_) => Ok(MaybeAIAgentAction::NoClientRepresentation),
                }
            }
            api::message::tool_call::Tool::OpenCodeReview(_) => {
                create_standard_action(AIAgentActionType::OpenCodeReview)
            }
            api::message::tool_call::Tool::InitProject(_) => {
                create_standard_action(AIAgentActionType::InitProject)
            }
            api::message::tool_call::Tool::ReadDocuments(read_documents) => {
                create_standard_action(read_documents.into())
            }
            api::message::tool_call::Tool::EditDocuments(edit_documents) => {
                create_standard_action(edit_documents.into())
            }
            api::message::tool_call::Tool::CreateDocuments(create_documents) => {
                create_standard_action(create_documents.into())
            }
            api::message::tool_call::Tool::ReadShellCommandOutput(read_shell_command_output) => {
                create_standard_action(read_shell_command_output.into())
            }
            api::message::tool_call::Tool::TransferShellCommandControlToUser(
                transfer_shell_command_control_to_user,
            ) => create_standard_action(transfer_shell_command_control_to_user.into()),
            api::message::tool_call::Tool::UseComputer(use_computer) => {
                create_standard_action(use_computer.try_into()?)
            }
            api::message::tool_call::Tool::RequestComputerUse(request_computer_use) => {
                create_standard_action(request_computer_use.into())
            }
            api::message::tool_call::Tool::Subagent(subagent) => {
                use api::message::tool_call::subagent::conversation_search_metadata::Target;
                use api::message::tool_call::subagent::Metadata;
                let subagent_type = match subagent.metadata {
                    Some(Metadata::Cli(_)) => SubagentType::Cli,
                    Some(Metadata::Research(_)) => SubagentType::Research,
                    Some(Metadata::Advice(_)) => SubagentType::Advice,
                    Some(Metadata::ComputerUse(_)) => SubagentType::ComputerUse,
                    Some(Metadata::Summarization(_)) => SubagentType::Summarization,
                    Some(Metadata::ConversationSearch(cs_meta)) => {
                        let query = if cs_meta.query.is_empty() {
                            None
                        } else {
                            Some(cs_meta.query)
                        };
                        let (conversation_id, agent_run_id) = match cs_meta.target {
                            Some(Target::ConversationId(conversation_id))
                                if !conversation_id.is_empty() =>
                            {
                                (Some(conversation_id), None)
                            }
                            Some(Target::AgentRunId(agent_run_id)) if !agent_run_id.is_empty() => {
                                (None, Some(agent_run_id))
                            }
                            Some(Target::ConversationId(_))
                            | Some(Target::AgentRunId(_))
                            | None => (None, None),
                        };
                        SubagentType::ConversationSearch {
                            query,
                            conversation_id,
                            agent_run_id,
                        }
                    }
                    Some(Metadata::WarpDocumentationSearch(_)) => {
                        SubagentType::WarpDocumentationSearch
                    }
                    None => SubagentType::Unknown,
                };
                Ok(MaybeAIAgentAction::Subagent(SubagentCall {
                    task_id: subagent.task_id,
                    subagent_type,
                }))
            }
            api::message::tool_call::Tool::StartAgent(start_agent) => {
                create_standard_action(AIAgentActionType::StartAgent {
                    version: StartAgentVersion::V1,
                    name: start_agent.name,
                    prompt: start_agent.prompt,
                    execution_mode: convert_start_agent_execution_mode(start_agent.execution_mode),
                    lifecycle_subscription: start_agent.lifecycle_subscription.map(
                        |subscription| {
                            subscription
                                .event_types
                                .into_iter()
                                .filter_map(convert_start_agent_lifecycle_event_type)
                                .collect()
                        },
                    ),
                })
            }
            api::message::tool_call::Tool::StartAgentV2(start_agent) => {
                create_standard_action(AIAgentActionType::StartAgent {
                    version: StartAgentVersion::V2,
                    name: start_agent.name,
                    prompt: start_agent.prompt,
                    execution_mode: convert_start_agent_v2_execution_mode(
                        start_agent.execution_mode,
                        params.skill_path_origin,
                    ),
                    lifecycle_subscription: start_agent.lifecycle_subscription.map(
                        |subscription| {
                            subscription
                                .event_types
                                .into_iter()
                                .filter_map(convert_start_agent_lifecycle_event_type)
                                .collect()
                        },
                    ),
                })
            }
            api::message::tool_call::Tool::RunAgents(orchestrate) => {
                create_standard_action(convert_run_agents(orchestrate, params.skill_path_origin))
            }
            api::message::tool_call::Tool::SendMessageToAgent(send_message) => {
                create_standard_action(AIAgentActionType::SendMessageToAgent {
                    addresses: send_message.addresses,
                    subject: send_message.subject,
                    message: send_message.message,
                })
            }
            api::message::tool_call::Tool::InsertReviewComments(insert_review_comments) => {
                create_standard_action(insert_review_comments.into())
            }
            api::message::tool_call::Tool::ReadSkill(read_skill) => {
                create_standard_action(convert_read_skill(read_skill, params.skill_path_origin)?)
            }
            api::message::tool_call::Tool::FetchConversation(fetch_conversation) => {
                create_standard_action(fetch_conversation.into())
            }
            api::message::tool_call::Tool::AskUserQuestion(ask) => {
                let questions = ask
                    .questions
                    .into_iter()
                    .filter_map(convert_api_question)
                    .collect();
                create_standard_action(AIAgentActionType::AskUserQuestion { questions })
            }
            // Clients do not need to know how to parse server tool-calls but receiving
            // them is not an error.
            api::message::tool_call::Tool::Server(_) => {
                Ok(MaybeAIAgentAction::NoClientRepresentation)
            }
            api::message::tool_call::Tool::WaitForEvents(payload) => {
                create_standard_action(AIAgentActionType::WaitForEvents {
                    tool_call_id: self.tool_call_id.clone(),
                    idle_timeout_seconds: payload.idle_timeout_seconds,
                })
            }
            _ => Err(ToolToAIAgentActionError::UnexpectedTool),
        }
    }
}

impl From<api::Suggestions> for Suggestions {
    fn from(api_suggestions: api::Suggestions) -> Self {
        Self {
            rules: api_suggestions
                .rules
                .into_iter()
                .map(|rule| SuggestedRule {
                    name: rule.name,
                    content: rule.content,
                    logging_id: rule.logging_id.into(),
                })
                .collect(),
            agent_mode_workflows: api_suggestions
                .workflows
                .into_iter()
                .map(|workflow| SuggestedAgentModeWorkflow {
                    name: workflow.name,
                    prompt: workflow.prompt,
                    logging_id: workflow.logging_id.into(),
                })
                .collect(),
        }
    }
}

impl From<api::TodoItem> for AIAgentTodo {
    fn from(value: api::TodoItem) -> Self {
        AIAgentTodo {
            id: value.id.into(),
            title: value.title,
            description: value.description,
        }
    }
}

/// Reconstruct user inputs from the provided server messages
/// (for use in shared agent exchanges where the input was not provided in this session)
pub fn user_inputs_from_messages(messages: &[api::Message]) -> Vec<AIAgentInput> {
    let mut inputs = Vec::new();
    let mut document_versions: HashMap<AIDocumentId, AIDocumentVersion> = HashMap::new();
    for m in messages {
        let Some(inner) = &m.message else { continue };
        match inner {
            api::message::Message::UserQuery(uq) => {
                let context = convert_input_context(uq.context.as_ref());
                let referenced_attachments = uq
                    .referenced_attachments
                    .iter()
                    .filter_map(|(key, attachment)| {
                        AIAgentAttachment::try_from(attachment.clone())
                            .ok()
                            .map(|a| (key.clone(), a))
                    })
                    .collect();
                inputs.push(AIAgentInput::UserQuery {
                    query: uq.query.clone(),
                    context,
                    static_query_type: None,
                    referenced_attachments,
                    user_query_mode: convert_user_query_mode(uq.mode.as_ref()),
                    running_command: None,
                    intended_agent: Some(uq.intended_agent()),
                });
            }
            api::message::Message::SystemQuery(sq) => {
                let ctx = convert_input_context(sq.context.as_ref());
                if let Some(t) = &sq.r#type {
                    // These system queries appear as user inputs in ai blocks.
                    match t {
                        api::message::system_query::Type::CreateNewProject(p) => {
                            inputs.push(AIAgentInput::CreateNewProject {
                                query: p.query.clone(),
                                context: ctx,
                            });
                        }
                        api::message::system_query::Type::CloneRepository(p) => {
                            inputs.push(AIAgentInput::CloneRepository {
                                clone_repo_url: CloneRepositoryURL::new(p.url.clone()),
                                context: ctx,
                            });
                        }
                        api::message::system_query::Type::AutoCodeDiff(p) => {
                            inputs.push(AIAgentInput::AutoCodeDiffQuery {
                                query: p.query.clone(),
                                context: ctx,
                            });
                        }
                        api::message::system_query::Type::FetchReviewComments(fetch) => {
                            inputs.push(AIAgentInput::FetchReviewComments {
                                repo_path: fetch.repo_path.clone(),
                                context: ctx,
                            });
                        }
                        _ => {}
                    }
                }
            }
            api::message::Message::ToolCallResult(tcr) => {
                let task_id = TaskId::new(m.task_id.clone());
                if let Some(input) = convert_tool_call_result_to_input(
                    &task_id,
                    tcr,
                    &HashMap::new(),
                    &mut document_versions,
                ) {
                    inputs.push(input);
                }
            }
            _ => {}
        }
    }
    inputs
}

fn convert_api_question(
    q: api::ask_user_question::Question,
) -> Option<ai::agent::action::AskUserQuestionItem> {
    let Some(QuestionType::MultipleChoice(mc)) = q.question_type else {
        return None;
    };

    // Server sends -1 when there is no recommendation.
    let recommended_idx = usize::try_from(mc.recommended_option_index)
        .ok()
        .filter(|idx| *idx < mc.options.len());
    let options = mc
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| ai::agent::action::AskUserQuestionOption {
            label: opt.label.clone(),
            recommended: recommended_idx == Some(i),
        })
        .collect();
    Some(ai::agent::action::AskUserQuestionItem {
        question_id: q.question_id.clone(),
        question: q.question,
        question_type: ai::agent::action::AskUserQuestionType::MultipleChoice {
            is_multiselect: mc.is_multiselect,
            options,
            supports_other: mc.supports_other,
        },
    })
}

#[cfg(test)]
#[path = "convert_from_tests.rs"]
mod tests;
