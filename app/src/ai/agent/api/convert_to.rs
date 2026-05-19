//! Conversions from application types to MAA API types.

use ai::agent::convert::ConvertToAPITypeError;
use anyhow::anyhow;
use chrono::{DateTime, Local, Timelike};
use warp_multi_agent_api as api;

use crate::ai::{
    agent::{
        AIAgentActionResult, AIAgentActionResultType, AIAgentAttachment, AIAgentContext,
        AIAgentInput, DriveObjectPayload, MCPContext, PassiveSuggestionResultType,
        PassiveSuggestionTrigger, RunningCommand, StaticQueryType, Suggestions, UserQueryMode,
    },
    block_context::BlockContext,
};

fn local_datetime_to_timestamp(timestamp: DateTime<Local>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: timestamp.timestamp(),
        nanos: timestamp.timestamp_subsec_nanos() as i32,
    }
}

impl TryFrom<StaticQueryType> for api::request::input::query_with_canned_response::Type {
    type Error = ConvertToAPITypeError;

    fn try_from(value: StaticQueryType) -> Result<Self, Self::Error> {
        match value {
            StaticQueryType::Install => Ok(
                api::request::input::query_with_canned_response::Type::Install(
                    api::request::input::query_with_canned_response::Install {},
                ),
            ),
            StaticQueryType::Code => {
                Ok(api::request::input::query_with_canned_response::Type::Code(
                    api::request::input::query_with_canned_response::Code {},
                ))
            }
            StaticQueryType::Deploy => Ok(
                api::request::input::query_with_canned_response::Type::Deploy(
                    api::request::input::query_with_canned_response::Deploy {},
                ),
            ),
            StaticQueryType::SomethingElse => Ok(
                api::request::input::query_with_canned_response::Type::SomethingElse(
                    api::request::input::query_with_canned_response::SomethingElse {},
                ),
            ),
            StaticQueryType::CustomOnboardingRequest => Ok(
                api::request::input::query_with_canned_response::Type::CustomOnboardingRequest(
                    api::request::input::query_with_canned_response::CustomOnboardingRequest {},
                ),
            ),
            StaticQueryType::EvaluationSuite => {
                Err(anyhow::anyhow!("EvaluationSuite StaticQueryType not yet supported").into())
            }
        }
    }
}

pub(super) fn convert_input(
    mut inputs: Vec<AIAgentInput>,
) -> Result<api::request::Input, ConvertToAPITypeError> {
    if inputs.is_empty() {
        return Err(anyhow!("Attempted to send multi-agent request with no input").into());
    }
    let api_context = inputs
        .iter()
        .rev()
        .find_map(AIAgentInput::context)
        .map(convert_context);

    let mut api_inputs = vec![];
    if inputs.len() == 1 {
        match inputs.pop().expect("Input exists.") {
            AIAgentInput::UserQuery {
                query,
                context,
                static_query_type: Some(query_type),
                ..
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::QueryWithCannedResponse(
                        api::request::input::QueryWithCannedResponse {
                            query,
                            r#type: Some(query_type.try_into()?),
                        },
                    )),
                });
            }
            AIAgentInput::AutoCodeDiffQuery { query, context } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::AutoCodeDiffQuery(
                        api::request::input::AutoCodeDiffQuery { query },
                    )),
                });
            }
            AIAgentInput::ResumeConversation { context } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::ResumeConversation(
                        api::request::input::ResumeConversation {},
                    )),
                });
            }
            AIAgentInput::InitProjectRules { context, .. } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::InitProjectRules(
                        api::request::input::InitProjectRules {},
                    )),
                });
            }
            AIAgentInput::CreateEnvironment {
                context,
                repo_paths,
                ..
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::CreateEnvironment(
                        api::request::input::CreateEnvironment { repo_paths },
                    )),
                });
            }
            AIAgentInput::TriggerPassiveSuggestion {
                context,
                attachments,
                trigger,
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::GeneratePassiveSuggestions(
                        api::request::input::GeneratePassiveSuggestions {
                            attachments: attachments
                                .into_iter()
                                .map(|attachment| attachment.into())
                                .collect(),
                            trigger: Some(trigger.into()),
                        },
                    )),
                });
            }
            AIAgentInput::CreateNewProject { query, context } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::CreateNewProject(
                        api::request::input::CreateNewProject { query },
                    )),
                });
            }
            AIAgentInput::CloneRepository {
                clone_repo_url,
                context,
                ..
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::CloneRepository(
                        api::request::input::CloneRepository {
                            url: clone_repo_url.into_url(),
                        },
                    )),
                });
            }
            AIAgentInput::CodeReview {
                context,
                review_comments,
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::CodeReview(
                        api::request::input::CodeReview {
                            operation: Some(
                                api::request::input::code_review::Operation::InitialReviewComments(
                                    api::request::input::code_review::InitialReviewComments {
                                        review_comments: review_comments
                                            .comments
                                            .into_iter()
                                            .map(Into::into)
                                            .collect(),
                                        diff_set: Some(api::DiffSet {
                                            hunks: review_comments
                                                .diff_set
                                                .into_iter()
                                                .flat_map(|(file_path, hunks)| {
                                                    hunks.into_iter().map(move |hunk| {
                                                        hunk.convert_to_api(file_path.clone())
                                                    })
                                                })
                                                .collect(),
                                            curr_ref: None,
                                            base_ref: None,
                                        }),
                                    },
                                ),
                            ),
                        },
                    )),
                });
            }
            AIAgentInput::FetchReviewComments { repo_path, context } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::FetchReviewComments(
                        api::request::input::FetchReviewComments { repo_path },
                    )),
                });
            }
            AIAgentInput::SummarizeConversation { prompt, context } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::SummarizeConversation(
                        api::request::input::SummarizeConversation {
                            prompt: prompt.unwrap_or_default(),
                        },
                    )),
                });
            }
            AIAgentInput::InvokeSkill {
                context,
                skill,
                user_query,
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::InvokeSkill(
                        api::request::input::InvokeSkill {
                            skill: Some(skill.into()),
                            user_query: user_query.map(|user_query| {
                                api::request::input::UserQuery {
                                    query: user_query.query,
                                    referenced_attachments: user_query
                                        .referenced_attachments
                                        .into_iter()
                                        .map(|(k, attachment)| (k, attachment.into()))
                                        .collect(),
                                    mode: None,
                                    intended_agent: Default::default(),
                                }
                            }),
                        },
                    )),
                });
            }
            AIAgentInput::StartFromAmbientRunPrompt {
                ambient_run_id,
                context,
                runtime_skill,
                attachments_dir,
            } => {
                return Ok(api::request::Input {
                    context: Some(convert_context(context.as_ref())),
                    r#type: Some(api::request::input::Type::StartFromAmbientRunPrompt(
                        api::request::input::StartFromAmbientRunPrompt {
                            ambient_run_id,
                            // Deprecated, we always resolve base_prompt from the stored task config.
                            runtime_base_prompt: String::new(),

                            runtime_skill: runtime_skill.map(|skill| skill.into()),
                            attachments_dir: attachments_dir.unwrap_or_default(),
                        },
                    )),
                });
            }
            other_input => match convert_input_to_user_input(other_input) {
                Ok(api_input) => api_inputs.push(api_input),
                Err(ConvertToAPITypeError::Ignore) => (),
                Err(e) => return Err(e),
            },
        }
    }

    for input in inputs.into_iter() {
        match convert_input_to_user_input(input) {
            Ok(api_input) => api_inputs.push(api_input),
            Err(ConvertToAPITypeError::Ignore) => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(api::request::Input {
        context: api_context,
        r#type: Some(api::request::input::Type::UserInputs(
            api::request::input::UserInputs {
                inputs: api_inputs
                    .into_iter()
                    .map(|input| api::request::input::user_inputs::UserInput { input: Some(input) })
                    .collect(),
            },
        )),
    })
}

fn convert_input_to_user_input(
    input: AIAgentInput,
) -> Result<api::request::input::user_inputs::user_input::Input, ConvertToAPITypeError> {
    match input {
        AIAgentInput::UserQuery {
            query,
            static_query_type: None,
            referenced_attachments,
            user_query_mode,
            running_command: None,
            intended_agent,
            ..
        } => Ok(
            api::request::input::user_inputs::user_input::Input::UserQuery(
                api::request::input::UserQuery {
                    query,
                    referenced_attachments: referenced_attachments.into_iter().map(|(k, attachment)| (k, attachment.into())).collect(),
                    mode: Some(user_query_mode.into()),
                    intended_agent: intended_agent.map(|agent| agent.into()).unwrap_or_default(),
                },
            ),
        ),
        AIAgentInput::UserQuery {
            query,
            static_query_type: None,
            referenced_attachments,
            user_query_mode,
            running_command: Some(RunningCommand{
                command,
                block_id,
                grid_contents: output,
                cursor,
                requested_command_id,
                is_alt_screen_active,
            }),
            ..
        } => {
            Ok(api::request::input::user_inputs::user_input::Input::CliAgentUserQuery(
                api::request::input::CliAgentUserQuery {
                    user_query: Some(api::request::input::UserQuery {
                            query,
                            referenced_attachments: referenced_attachments.into_iter().map(|(k, attachment)| (k, attachment.into())).collect(),
                            mode: Some(user_query_mode.into()),
                            intended_agent: api::AgentType::Cli.into(),
                        }),
                    running_command: Some(api::RunningShellCommand{
                        command,
                        snapshot: Some(api::LongRunningShellCommandSnapshot {
                            output,
                            cursor,
                            command_id: block_id.as_str().to_owned(),
                            is_alt_screen_active,
                            is_preempted: false,
                        }),
                    }),
                    run_shell_command_tool_call_id: requested_command_id.map(|id| id.to_string()).unwrap_or_default(),
                }
            ))
        }
        AIAgentInput::ActionResult { result, .. } => result.try_into(),
        AIAgentInput::MessagesReceivedFromAgents { messages } => Ok(
            api::request::input::user_inputs::user_input::Input::MessagesReceivedFromAgents(
                api::request::input::user_inputs::MessagesReceivedFromAgents {
                    messages: messages
                        .into_iter()
                        .map(
                            |msg| api::request::input::user_inputs::messages_received_from_agents::ReceivedMessage {
                                message_id: msg.message_id,
                                sender_agent_id: msg.sender_agent_id,
                                addresses: msg.addresses,
                                subject: msg.subject,
                                message_body: msg.message_body,
                            },
                        )
                        .collect(),
                },
            ),
        ),
        AIAgentInput::EventsFromAgents { events } => Ok(
            api::request::input::user_inputs::user_input::Input::EventsFromAgents(
                api::request::input::user_inputs::EventsFromAgents {
                    agent_events: events,
                },
            ),
        ),
        AIAgentInput::PassiveSuggestionResult {
            trigger,
            suggestion,
            ..
        } => {
            let api_trigger = match trigger {
                Some(PassiveSuggestionTrigger::ShellCommandCompleted(shell_trigger)) => Some(
                    api::passive_suggestion_result_type::Trigger::ExecutedShellCommand(
                        (*shell_trigger.executed_shell_command).into(),
                    ),
                ),
                Some(PassiveSuggestionTrigger::AgentResponseCompleted { .. }) => Some(
                    api::passive_suggestion_result_type::Trigger::AgentResponseCompleted(
                        api::passive_suggestion_result_type::AgentResponseCompleted {},
                    ),
                ),
                _ => None,
            };
            let api_suggestion = match suggestion {
                PassiveSuggestionResultType::Prompt { prompt } => Some(
                    api::passive_suggestion_result_type::Suggestion::Prompt(
                        api::passive_suggestion_result_type::Prompt { prompt },
                    ),
                ),
                PassiveSuggestionResultType::CodeDiff {
                    diffs,
                    summary,
                    accepted,
                } => Some(
                    api::passive_suggestion_result_type::Suggestion::CodeDiff(
                        api::passive_suggestion_result_type::CodeDiff {
                            diffs: diffs
                                .into_iter()
                                .map(|d| api::passive_suggestion_result_type::code_diff::Diff {
                                    file_path: d.file_path,
                                    search: d.search,
                                    replace: d.replace,
                                })
                                .collect(),
                            summary,
                            accepted,
                        },
                    ),
                ),
            };
            Ok(
                api::request::input::user_inputs::user_input::Input::PassiveSuggestionResult(
                    api::request::input::user_inputs::PassiveSuggestionResultInput {
                        result: Some(api::PassiveSuggestionResultType {
                            trigger: api_trigger,
                            suggestion: api_suggestion,
                        }),
                    },
                ),
            )
        }
        AIAgentInput::OrchestrationConfigUpdate {
            plan_id,
            config,
            status,
        } => Ok(
            api::request::input::user_inputs::user_input::Input::OrchestrationConfigUpdate(
                api::OrchestrationConfigUpdate {
                    plan_id,
                    config: Some(config.to_proto()),
                    status: status.to_proto(),
                },
            ),
        ),
        AIAgentInput::ResumeConversation { .. } => Err(ConvertToAPITypeError::Ignore),
        AIAgentInput::InitProjectRules { .. } => Err(ConvertToAPITypeError::Ignore),
        AIAgentInput::CodeReview { .. } => Err(ConvertToAPITypeError::Ignore),
        AIAgentInput::FetchReviewComments { .. } => Err(ConvertToAPITypeError::Ignore),
        AIAgentInput::CreateEnvironment { .. } => Err(ConvertToAPITypeError::Ignore),
        AIAgentInput::InvokeSkill { .. } => Err(ConvertToAPITypeError::Ignore),
        invalid_input => Err(anyhow!(
            "Cannot convert non user query or action result input into API UserInput: {invalid_input:?}"
        ).into()),
    }
}

impl From<PassiveSuggestionTrigger> for api::request::input::generate_passive_suggestions::Trigger {
    fn from(value: PassiveSuggestionTrigger) -> Self {
        match value {
            PassiveSuggestionTrigger::FilesChanged => {
                api::request::input::generate_passive_suggestions::Trigger::FilesChanged(())
            }
            PassiveSuggestionTrigger::CommandRun => {
                api::request::input::generate_passive_suggestions::Trigger::CommandRun(())
            }
            PassiveSuggestionTrigger::ShellCommandCompleted(shell_trigger) => {
                api::request::input::generate_passive_suggestions::Trigger::ShellCommandCompleted(
                    api::request::input::generate_passive_suggestions::ShellCommandCompleted {
                        executed_shell_command: Some(
                            (*shell_trigger.executed_shell_command).into(),
                        ),
                        relevant_files: shell_trigger
                            .relevant_files
                            .into_iter()
                            .flat_map(|file| Vec::<api::AnyFileContent>::from(file).into_iter())
                            .collect(),
                    },
                )
            }
            PassiveSuggestionTrigger::AgentResponseCompleted { .. } => {
                api::request::input::generate_passive_suggestions::Trigger::AgentResponseCompleted(
                    api::request::input::generate_passive_suggestions::AgentResponseCompleted {},
                )
            }
        }
    }
}

impl From<UserQueryMode> for warp_multi_agent_api::UserQueryMode {
    fn from(value: UserQueryMode) -> Self {
        match value {
            UserQueryMode::Normal => warp_multi_agent_api::UserQueryMode { r#type: None },
            UserQueryMode::Plan => warp_multi_agent_api::UserQueryMode {
                r#type: Some(warp_multi_agent_api::user_query_mode::Type::Plan(())),
            },
            UserQueryMode::Orchestrate => warp_multi_agent_api::UserQueryMode {
                r#type: Some(warp_multi_agent_api::user_query_mode::Type::Orchestrate(())),
            },
        }
    }
}

impl From<AIAgentAttachment> for api::Attachment {
    fn from(attachment: AIAgentAttachment) -> Self {
        match attachment {
            AIAgentAttachment::PlainText(text) => api::Attachment {
                value: Some(api::attachment::Value::PlainText(text)),
            },
            AIAgentAttachment::Block(block) => api::Attachment {
                value: Some(api::attachment::Value::ExecutedShellCommand(block.into())),
            },
            AIAgentAttachment::DriveObject { uid, payload } => api::Attachment {
                value: Some(api::attachment::Value::DriveObject(api::DriveObject {
                    uid,
                    object_payload: payload.map(|p| match p {
                        DriveObjectPayload::Workflow {
                            name,
                            description,
                            command,
                        } => api::drive_object::ObjectPayload::Workflow(api::Workflow {
                            name,
                            description,
                            command,
                        }),
                        DriveObjectPayload::Notebook { title, content } => {
                            api::drive_object::ObjectPayload::Notebook(api::Notebook {
                                title,
                                content,
                            })
                        }
                        DriveObjectPayload::GenericStringObject {
                            payload,
                            object_type,
                        } => api::drive_object::ObjectPayload::GenericStringObject(
                            api::GenericStringObject {
                                payload,
                                object_type,
                            },
                        ),
                    }),
                })),
            },
            #[allow(deprecated)]
            AIAgentAttachment::DiffHunk {
                file_path,
                line_range,
                diff_content,
                lines_added,
                lines_removed,
                current,
                base,
            } => api::Attachment {
                value: Some(api::attachment::Value::DiffHunk(api::DiffHunk {
                    file_path,
                    line_range: Some(api::FileContentLineRange {
                        start: line_range.start.as_usize() as u32,
                        end: line_range.end.as_usize() as u32,
                    }),
                    diff_content,
                    lines_added,
                    lines_removed,
                    current: current.map(Into::into),
                    base: Some(base.into()),
                })),
            },
            AIAgentAttachment::DocumentContent {
                document_id,
                content,
                line_range,
                // TODO: Add attachment source to API
                ..
            } => api::Attachment {
                value: Some(api::attachment::Value::DocumentContent(
                    api::DocumentContent {
                        document_id,
                        content,
                        line_range: line_range.map(|range| api::FileContentLineRange {
                            start: range.start.as_usize() as u32,
                            end: range.end.as_usize() as u32,
                        }),
                    },
                )),
            },
            AIAgentAttachment::DiffSet {
                file_diffs,
                current,
                base,
            } => api::Attachment {
                value: Some(api::attachment::Value::DiffSet(api::DiffSet {
                    hunks: file_diffs
                        .into_iter()
                        .flat_map(|(file_path, hunks)| {
                            hunks
                                .into_iter()
                                .map(move |hunk| hunk.convert_to_api(file_path.clone()))
                        })
                        .collect(),
                    curr_ref: current.map(Into::into),
                    base_ref: Some(base.into()),
                })),
            },
            AIAgentAttachment::FilePathReference { file_path, .. } => api::Attachment {
                value: Some(api::attachment::Value::FilePathReference(
                    api::FilePathReference { file_path },
                )),
            },
        }
    }
}

impl TryFrom<AIAgentActionResult> for api::request::input::user_inputs::user_input::Input {
    type Error = ConvertToAPITypeError;

    fn try_from(action_result: AIAgentActionResult) -> Result<Self, Self::Error> {
        let result = match action_result.result {
            AIAgentActionResultType::RequestCommandOutput(request_command_result) => {
                Some(request_command_result.try_into()?)
            }
            AIAgentActionResultType::WriteToLongRunningShellCommand(result) => {
                Some(result.try_into()?)
            }
            AIAgentActionResultType::ReadFiles(read_files_result) => {
                Some(read_files_result.try_into()?)
            }
            AIAgentActionResultType::UploadArtifact(upload_artifact_result) => {
                Some(upload_artifact_result.try_into()?)
            }
            AIAgentActionResultType::SearchCodebase(search_codebase_result) => {
                Some(search_codebase_result.try_into()?)
            }
            AIAgentActionResultType::RequestFileEdits(request_file_edits_result) => {
                Some(request_file_edits_result.try_into()?)
            }
            AIAgentActionResultType::Grep(grep_result) => Some(grep_result.try_into()?),
            AIAgentActionResultType::FileGlob(file_glob_result) => {
                Some(file_glob_result.try_into()?)
            }
            AIAgentActionResultType::FileGlobV2(file_glob_result) => {
                Some(file_glob_result.try_into()?)
            }
            AIAgentActionResultType::ReadMCPResource(read_mcp_resource_result) => {
                Some(read_mcp_resource_result.try_into()?)
            }
            AIAgentActionResultType::CallMCPTool(call_mcp_tool_result) => {
                Some(call_mcp_tool_result.try_into()?)
            }
            AIAgentActionResultType::ReadSkill(read_skill_result) => {
                Some(read_skill_result.try_into()?)
            }
            AIAgentActionResultType::SuggestNewConversation(suggest_new_conversation_result) => {
                Some(suggest_new_conversation_result.try_into()?)
            }
            AIAgentActionResultType::SuggestPrompt(suggest_prompt_result) => {
                Some(suggest_prompt_result.try_into()?)
            }
            AIAgentActionResultType::OpenCodeReview => Some(
                warp_multi_agent_api::request::input::tool_call_result::Result::OpenCodeReview(
                    warp_multi_agent_api::OpenCodeReviewResult {},
                ),
            ),
            AIAgentActionResultType::InsertReviewComments(insert_review_comments_result) => {
                Some(insert_review_comments_result.try_into()?)
            }
            AIAgentActionResultType::InitProject => Some(
                warp_multi_agent_api::request::input::tool_call_result::Result::InitProject(
                    warp_multi_agent_api::InitProjectResult {},
                ),
            ),
            AIAgentActionResultType::ReadDocuments(read_documents_result) => {
                Some(read_documents_result.try_into()?)
            }
            AIAgentActionResultType::EditDocuments(edit_documents_result) => {
                Some(edit_documents_result.try_into()?)
            }
            AIAgentActionResultType::CreateDocuments(create_documents_result) => {
                Some(create_documents_result.try_into()?)
            }
            AIAgentActionResultType::ReadShellCommandOutput(read_shell_command_output_result) => {
                Some(read_shell_command_output_result.try_into()?)
            }
            AIAgentActionResultType::UseComputer(use_computer_result) => {
                Some(use_computer_result.try_into()?)
            }
            AIAgentActionResultType::RequestComputerUse(request_computer_use_result) => {
                Some(request_computer_use_result.try_into()?)
            }
            AIAgentActionResultType::FetchConversation(fetch_conversation_result) => {
                Some(fetch_conversation_result.try_into()?)
            }
            AIAgentActionResultType::StartAgent(start_agent_result) => {
                Some(start_agent_result.into())
            }
            AIAgentActionResultType::SendMessageToAgent(send_message_result) => {
                Some(send_message_result.into())
            }
            AIAgentActionResultType::TransferShellCommandControlToUser(transfer_control_result) => {
                Some(transfer_control_result.try_into()?)
            }
            AIAgentActionResultType::AskUserQuestion(ask_user_question_result) => {
                Some(ask_user_question_result.into())
            }
            AIAgentActionResultType::RunAgents(orchestrate_result) => {
                Some(orchestrate_result.try_into()?)
            }
        };
        Ok(
            api::request::input::user_inputs::user_input::Input::ToolCallResult(
                api::request::input::ToolCallResult {
                    tool_call_id: action_result.id.into(),
                    result,
                },
            ),
        )
    }
}

fn convert_context(context: &[AIAgentContext]) -> api::InputContext {
    let mut api_context = api::InputContext::default();
    for context in context.iter().cloned() {
        match context {
            AIAgentContext::Block(block) => {
                #[allow(deprecated)]
                api_context.executed_shell_commands.push((*block).into());
            }
            AIAgentContext::Directory {
                pwd,
                home_dir,
                are_file_symbols_indexed,
            } => {
                api_context.directory = Some(api::input_context::Directory {
                    pwd: pwd.unwrap_or_default(),
                    home: home_dir.unwrap_or_default(),
                    pwd_file_symbols_indexed: are_file_symbols_indexed,
                });
            }
            AIAgentContext::SelectedText(text) => {
                api_context
                    .selected_text
                    .push(api::input_context::SelectedText { text });
            }
            AIAgentContext::ExecutionEnvironment(execution_ctx) => {
                api_context.shell = Some(api::input_context::Shell {
                    name: execution_ctx.shell_name,
                    version: execution_ctx.shell_version.unwrap_or_default(),
                });

                if execution_ctx.os.category.is_none() && execution_ctx.os.distribution.is_none() {
                    continue;
                }
                api_context.operating_system = Some(api::input_context::OperatingSystem {
                    platform: execution_ctx.os.category.unwrap_or_default(),
                    distribution: execution_ctx.os.distribution.unwrap_or_default(),
                });
            }
            AIAgentContext::CurrentTime { current_time } => {
                let utc_time = current_time.to_utc();
                api_context.current_time = Some(prost_types::Timestamp {
                    seconds: utc_time.timestamp(),
                    nanos: utc_time.nanosecond() as i32,
                });
            }
            AIAgentContext::Image(image_context) => {
                api_context.images.push(api::input_context::Image {
                    data: image_context.data.into(),
                    mime_type: image_context.mime_type,
                });
            }
            AIAgentContext::Codebase { path, name } => {
                api_context
                    .codebases
                    .push(api::input_context::Codebase { path, name });
            }
            AIAgentContext::ProjectRules {
                root_path,
                active_rules,
                additional_rule_paths,
            } => {
                api_context
                    .project_rules
                    .push(api::input_context::ProjectRules {
                        root_path,
                        active_rule_files: active_rules
                            .into_iter()
                            .flat_map(|rule| {
                                let file_contents: Vec<api::FileContent> = rule.into();
                                file_contents.into_iter()
                            })
                            .collect(),
                        additional_rule_file_paths: additional_rule_paths,
                    });
            }
            AIAgentContext::File(file_context) => {
                let contents: Vec<api::FileContent> = file_context.into();

                for content in contents {
                    api_context.files.push(api::input_context::File {
                        content: Some(content),
                    });
                }
            }
            AIAgentContext::Git { head, branch } => {
                let git_context = api_git_context(&mut api_context);
                git_context.head = head;
                git_context.branch = branch.unwrap_or_default();
            }
            AIAgentContext::Repository { name, owner } => {
                api_git_context(&mut api_context).repository =
                    Some(api::input_context::git::Repository {
                        name,
                        owner: owner.unwrap_or_default(),
                    });
            }
            AIAgentContext::PullRequest {
                number,
                state,
                draft,
                base_branch,
            } => {
                if number <= 0 {
                    continue;
                }
                let Some(state) = api_pull_request_state(&state, draft) else {
                    continue;
                };
                api_git_context(&mut api_context).pull_request =
                    Some(api::input_context::git::PullRequest {
                        number,
                        state: state as i32,
                        base_branch,
                    });
            }
            AIAgentContext::Skills { skills } => {
                api_context.updated_skills_context = Some(api::input_context::SkillsContext {
                    available_skills: skills
                        .into_iter()
                        .map(|skill| api::SkillDescriptor {
                            skill_reference: Some(skill.reference.into()),
                            name: skill.name,
                            description: skill.description,
                            provider: Some(skill.provider.into()),
                            scope: Some(skill.scope.into()),
                        })
                        .collect(),
                });
            }
        }
    }
    api_context
}

fn api_git_context(api_context: &mut api::InputContext) -> &mut api::input_context::Git {
    api_context.git.get_or_insert_with(Default::default)
}

/// Maps a GitHub PR state plus draft flag to the proto `State` enum.
///
/// Returns `None` for unknown states so the caller can skip emitting a
/// `pull_request` sub-message rather than sending `STATE_UNSPECIFIED` to the
/// server.
fn api_pull_request_state(
    state: &str,
    draft: bool,
) -> Option<api::input_context::git::pull_request::State> {
    use api::input_context::git::pull_request::State;
    match state.to_ascii_uppercase().as_str() {
        "OPEN" if draft => Some(State::OpenDraft),
        "OPEN" => Some(State::Open),
        "CLOSED" => Some(State::Closed),
        "MERGED" => Some(State::Merged),
        _ => None,
    }
}

impl From<Suggestions> for api::Suggestions {
    fn from(value: Suggestions) -> Self {
        Self {
            rules: value
                .rules
                .into_iter()
                .map(|rule| api::SuggestedRule {
                    name: rule.name,
                    content: rule.content,
                    logging_id: rule.logging_id.to_string(),
                })
                .collect(),
            workflows: value
                .agent_mode_workflows
                .into_iter()
                .map(|workflow| api::SuggestedAgentModeWorkflow {
                    name: workflow.name,
                    prompt: workflow.prompt,
                    logging_id: workflow.logging_id.to_string(),
                })
                .collect(),
        }
    }
}

// Convert rmcp resource to proto format.
fn convert_mcp_resource(resource: rmcp::model::Resource) -> api::request::mcp_context::McpResource {
    let rmcp::model::RawResource {
        uri,
        name,
        description,
        mime_type,
        ..
    } = resource.raw;
    api::request::mcp_context::McpResource {
        uri,
        name,
        description: description.unwrap_or_default(),
        mime_type: mime_type.unwrap_or_default(),
    }
}

// Convert rmcp tool to proto format, skipping tools with invalid schemas.
fn convert_mcp_tool(tool: rmcp::model::Tool) -> Option<api::request::mcp_context::McpTool> {
    let Ok(prost_types::Value {
        kind: Some(prost_types::value::Kind::StructValue(input_schema)),
    }) = serde_json_to_prost(tool.input_schema.as_ref().clone().into())
    else {
        return None;
    };

    Some(api::request::mcp_context::McpTool {
        name: tool.name.to_string(),
        description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
        input_schema: Some(input_schema),
    })
}

impl From<MCPContext> for api::request::McpContext {
    #[allow(deprecated)]
    fn from(value: MCPContext) -> Self {
        // Check if we're using the old flat structure (no servers)
        // or the new grouped structure (servers populated)
        if value.servers.is_empty() {
            // Old behavior: use deprecated flat resources and tools lists
            api::request::McpContext {
                #[allow(deprecated)]
                resources: value
                    .resources
                    .into_iter()
                    .map(convert_mcp_resource)
                    .collect(),
                #[allow(deprecated)]
                tools: value
                    .tools
                    .into_iter()
                    .filter_map(convert_mcp_tool)
                    .collect(),
                servers: vec![], // Empty for old behavior
            }
        } else {
            // New behavior: group by server
            let servers: Vec<_> = value
                .servers
                .into_iter()
                .map(|server| api::request::mcp_context::McpServer {
                    id: server.id,
                    name: server.name,
                    description: server.description,
                    resources: server
                        .resources
                        .into_iter()
                        .map(convert_mcp_resource)
                        .collect(),
                    tools: server
                        .tools
                        .into_iter()
                        .filter_map(convert_mcp_tool)
                        .collect(),
                })
                .collect();

            api::request::McpContext {
                #[allow(deprecated)]
                resources: vec![], // Empty - everything is grouped by server
                #[allow(deprecated)]
                tools: vec![], // Empty - everything is grouped by server
                servers,
            }
        }
    }
}

impl From<BlockContext> for api::ExecutedShellCommand {
    fn from(block: BlockContext) -> Self {
        api::ExecutedShellCommand {
            command: block.command,
            output: block.output,
            exit_code: block.exit_code.value(),
            command_id: block.id.into(),
            is_auto_attached: block.is_auto_attached,
            started_ts: block.started_ts.map(local_datetime_to_timestamp),
            finished_ts: block.finished_ts.map(local_datetime_to_timestamp),
        }
    }
}

/// Tries to convert a [`serde_json::Value`] to a [`prost_types::Value`].
#[cfg_attr(target_family = "wasm", allow(dead_code))]
fn serde_json_to_prost(value: serde_json::Value) -> Result<prost_types::Value, String> {
    use prost_types::value::Kind::*;
    use serde_json::Value::*;
    use std::collections::BTreeMap;

    Ok(prost_types::Value {
        kind: Some(match value {
            Null => NullValue(0),
            Bool(v) => BoolValue(v),
            Number(n) => NumberValue(
                n.as_f64()
                    .ok_or_else(|| format!("float {n} is not valid JSON number"))?,
            ),
            String(s) => StringValue(s),
            Array(a) => ListValue(prost_types::ListValue {
                values: a
                    .into_iter()
                    .map(serde_json_to_prost)
                    .collect::<Result<Vec<_>, std::string::String>>()?,
            }),
            Object(v) => StructValue(prost_types::Struct {
                fields: v
                    .into_iter()
                    .map(|(k, v)| serde_json_to_prost(v).map(|v| (k, v)))
                    .collect::<Result<BTreeMap<_, _>, std::string::String>>()?,
            }),
        }),
    })
}

#[cfg(test)]
#[path = "convert_to_tests.rs"]
mod tests;
