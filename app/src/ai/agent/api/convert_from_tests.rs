use super::{
    convert_api_question, ConversionParams, ConvertAPIMessageToClientOutputMessage,
    MaybeAIAgentOutputMessage,
};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionType, AIAgentOutputMessageType, LifecycleEventType, StartAgentExecutionMode,
};
use ai::agent::action::AskUserQuestionType;
use ai::skills::SkillReference;
use warp_multi_agent_api as api;

fn start_agent_tool_call_message(
    name: &str,
    prompt: &str,
    execution_mode: Option<api::start_agent::ExecutionMode>,
    lifecycle_subscription_event_types: Option<Vec<i32>>,
) -> api::Message {
    api::Message {
        id: "message-id".to_string(),
        task_id: "task-id".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "tool-call-id".to_string(),
            tool: Some(api::message::tool_call::Tool::StartAgent(api::StartAgent {
                name: name.to_string(),
                prompt: prompt.to_string(),
                execution_mode,
                lifecycle_subscription: lifecycle_subscription_event_types
                    .map(|event_types| api::start_agent::LifecycleSubscription { event_types }),
            })),
        })),
        request_id: "request-id".to_string(),
        timestamp: None,
    }
}

fn local_start_agent_v2_execution_mode(harness_type: &str) -> api::start_agent_v2::ExecutionMode {
    api::start_agent_v2::ExecutionMode {
        mode: Some(api::start_agent_v2::execution_mode::Mode::Local(
            api::start_agent_v2::execution_mode::Local {
                harness: Some(api::start_agent_v2::execution_mode::Harness {
                    r#type: harness_type.to_string(),
                }),
            },
        )),
    }
}

fn local_start_agent_v2_execution_mode_without_harness() -> api::start_agent_v2::ExecutionMode {
    api::start_agent_v2::ExecutionMode {
        mode: Some(api::start_agent_v2::execution_mode::Mode::Local(
            api::start_agent_v2::execution_mode::Local { harness: None },
        )),
    }
}

fn start_agent_v2_tool_call_message(
    name: &str,
    prompt: &str,
    execution_mode: Option<api::start_agent_v2::ExecutionMode>,
    lifecycle_subscription_event_types: Option<Vec<i32>>,
) -> api::Message {
    api::Message {
        id: "message-id".to_string(),
        task_id: "task-id".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "tool-call-id".to_string(),
            tool: Some(api::message::tool_call::Tool::StartAgentV2(
                api::StartAgentV2 {
                    name: name.to_string(),
                    prompt: prompt.to_string(),
                    execution_mode,
                    lifecycle_subscription: lifecycle_subscription_event_types.map(|event_types| {
                        api::start_agent_v2::LifecycleSubscription { event_types }
                    }),
                },
            )),
        })),
        request_id: "request-id".to_string(),
        timestamp: None,
    }
}

fn upload_artifact_tool_call_message(path: &str, description: &str) -> api::Message {
    api::Message {
        id: "message-id".to_string(),
        task_id: "task-id".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "tool-call-id".to_string(),
            tool: Some(api::message::tool_call::Tool::UploadFileArtifact(
                api::UploadFileArtifact {
                    file: Some(api::FilePathReference {
                        file_path: path.to_string(),
                    }),
                    description: description.to_string(),
                },
            )),
        })),
        request_id: "request-id".to_string(),
        timestamp: None,
    }
}

fn remote_start_agent_v2_execution_mode(
    environment_id: &str,
) -> api::start_agent_v2::ExecutionMode {
    api::start_agent_v2::ExecutionMode {
        mode: Some(api::start_agent_v2::execution_mode::Mode::Remote(
            api::start_agent_v2::execution_mode::Remote {
                environment_id: environment_id.to_string(),
                skills: vec![
                    api::SkillRef {
                        skill_reference: Some(api::skill_ref::SkillReference::Path(
                            "/tmp/SKILL.md".to_string(),
                        )),
                    },
                    api::SkillRef {
                        skill_reference: Some(api::skill_ref::SkillReference::BundledSkillId(
                            "review-comments".to_string(),
                        )),
                    },
                ],
                model_id: "gpt-test".to_string(),
                computer_use_enabled: true,
                worker_host: "worker-host".to_string(),
                harness: Some(api::start_agent_v2::execution_mode::Harness {
                    r#type: "claude-code".to_string(),
                }),
                title: "Remote child".to_string(),
            },
        )),
    }
}

fn file_artifact_created_message(filepath: &str, description: &str) -> api::Message {
    api::Message {
        id: "message-id".to_string(),
        task_id: "task-id".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ArtifactEvent(
            api::message::ArtifactEvent {
                event: Some(api::message::artifact_event::Event::Created(
                    api::message::artifact_event::ArtifactCreated {
                        artifact: Some(
                            api::message::artifact_event::artifact_created::Artifact::File(
                                api::message::artifact_event::FileArtifact {
                                    artifact_uid: "artifact-uid".to_string(),
                                    filepath: filepath.to_string(),
                                    mime_type: "text/plain".to_string(),
                                    size_bytes: 42,
                                    description: description.to_string(),
                                },
                            ),
                        ),
                    },
                )),
            },
        )),
        request_id: "request-id".to_string(),
        timestamp: None,
    }
}

fn remote_start_agent_execution_mode(environment_id: &str) -> api::start_agent::ExecutionMode {
    api::start_agent::ExecutionMode {
        mode: Some(api::start_agent::execution_mode::Mode::Remote(
            api::start_agent::execution_mode::Remote {
                environment_id: environment_id.to_string(),
            },
        )),
    }
}

fn build_multiple_choice_question(
    recommended_option_index: i32,
) -> api::ask_user_question::Question {
    api::ask_user_question::Question {
        question_id: "q1".to_string(),
        question: "Which option should we prefer?".to_string(),
        question_type: Some(
            api::ask_user_question::question::QuestionType::MultipleChoice(
                api::ask_user_question::MultipleChoice {
                    is_multiselect: false,
                    options: vec![
                        api::ask_user_question::Option {
                            label: "First".to_string(),
                        },
                        api::ask_user_question::Option {
                            label: "Second".to_string(),
                        },
                    ],
                    recommended_option_index,
                    supports_other: false,
                },
            ),
        ),
    }
}

#[test]
fn convert_api_question_treats_negative_recommended_index_as_no_recommendation() {
    let converted = convert_api_question(build_multiple_choice_question(-1))
        .expect("multiple choice questions should convert");

    let AskUserQuestionType::MultipleChoice { options, .. } = converted.question_type;
    assert_eq!(options.len(), 2);
    assert!(options.iter().all(|option| !option.recommended));
}

#[test]
fn convert_api_question_uses_zero_based_recommended_index_when_present() {
    let converted = convert_api_question(build_multiple_choice_question(0))
        .expect("multiple choice questions should convert");

    let AskUserQuestionType::MultipleChoice { options, .. } = converted.question_type;
    assert_eq!(options.len(), 2);
    assert!(options[0].recommended);
    assert!(!options[1].recommended);
}

fn extract_start_agent_action(
    output: MaybeAIAgentOutputMessage,
) -> (
    String,
    String,
    StartAgentExecutionMode,
    Option<Vec<LifecycleEventType>>,
) {
    let MaybeAIAgentOutputMessage::Message(output_message) = output else {
        panic!("expected output message");
    };
    let AIAgentOutputMessageType::Action(action) = output_message.message else {
        panic!("expected action output message");
    };
    let AIAgentActionType::StartAgent {
        version: _,
        name,
        prompt,
        execution_mode,
        lifecycle_subscription,
    } = action.action
    else {
        panic!("expected StartAgent action");
    };
    (name, prompt, execution_mode, lifecycle_subscription)
}

fn extract_upload_artifact_action(output: MaybeAIAgentOutputMessage) -> (String, Option<String>) {
    let MaybeAIAgentOutputMessage::Message(output_message) = output else {
        panic!("expected output message");
    };
    let AIAgentOutputMessageType::Action(action) = output_message.message else {
        panic!("expected action output message");
    };
    let AIAgentActionType::UploadArtifact(request) = action.action else {
        panic!("expected UploadArtifact action");
    };
    (request.file_path, request.description)
}

fn extract_file_artifact_created(
    output: MaybeAIAgentOutputMessage,
) -> (String, String, Option<String>, i64) {
    let MaybeAIAgentOutputMessage::Message(output_message) = output else {
        panic!("expected output message");
    };
    let AIAgentOutputMessageType::ArtifactCreated(artifact) = output_message.message else {
        panic!("expected artifact created output message");
    };
    let crate::ai::agent::ArtifactCreatedData::File {
        filepath,
        filename,
        description,
        size_bytes,
        ..
    } = artifact
    else {
        panic!("expected file artifact created output message");
    };
    (filepath, filename, description, size_bytes)
}

#[test]
fn converts_start_agent_tool_call_to_action_with_prompt() {
    let task_id = TaskId::new("task-id".to_string());
    let message =
        start_agent_tool_call_message("Agent 1", "run tests and report failures", None, None);

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (name, prompt, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);

    assert_eq!(name, "Agent 1");
    assert_eq!(prompt, "run tests and report failures");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
    assert_eq!(lifecycle_subscription, None);
}

#[test]
fn converts_local_start_agent_v2_without_harness_type_to_defaults() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_v2_tool_call_message(
        "Agent 7",
        "run in the default local harness",
        Some(local_start_agent_v2_execution_mode_without_harness()),
        None,
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (name, prompt, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);

    assert_eq!(name, "Agent 7");
    assert_eq!(prompt, "run in the default local harness");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
    assert_eq!(lifecycle_subscription, None);
}

#[test]
fn converts_upload_artifact_tool_call_to_action() {
    let task_id = TaskId::new("task-id".to_string());
    let message = upload_artifact_tool_call_message(
        "/tmp/build/output.log",
        "Build output for the latest run",
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (file_path, description) = extract_upload_artifact_action(output);

    assert_eq!(file_path, "/tmp/build/output.log");
    assert_eq!(
        description.as_deref(),
        Some("Build output for the latest run")
    );
}

#[test]
fn converts_file_artifact_created_message_with_filename() {
    let task_id = TaskId::new("task-id".to_string());
    let message =
        file_artifact_created_message("outputs/report.txt", "Build output for the latest run");

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (filepath, filename, description, size_bytes) = extract_file_artifact_created(output);

    assert_eq!(filepath, "outputs/report.txt");
    assert_eq!(filename, "report.txt");
    assert_eq!(
        description.as_deref(),
        Some("Build output for the latest run")
    );
    assert_eq!(size_bytes, 42);
}

#[test]
fn converts_start_agent_tool_calls_with_different_prompt_lengths() {
    let task_id = TaskId::new("task-id".to_string());
    let partial_message = start_agent_tool_call_message("Agent 2", "run tests", None, None);
    let updated_message = start_agent_tool_call_message(
        "Agent 2",
        "run tests and then summarize failures",
        None,
        None,
    );

    let partial_output = partial_message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("partial conversion should succeed");
    let updated_output = updated_message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("updated conversion should succeed");

    let (_, partial_prompt, partial_execution_mode, _) = extract_start_agent_action(partial_output);
    let (_, updated_prompt, updated_execution_mode, _) = extract_start_agent_action(updated_output);

    assert_eq!(partial_prompt, "run tests");
    assert_eq!(updated_prompt, "run tests and then summarize failures");
    assert_eq!(
        partial_execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
    assert_eq!(
        updated_execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
}

#[test]
fn converts_start_agent_with_explicit_empty_lifecycle_subscription() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_tool_call_message("Agent 3", "run tests", None, Some(vec![]));

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (_, _, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
    assert_eq!(lifecycle_subscription, Some(vec![]));
}

#[test]
fn converts_start_agent_with_cancelled_and_blocked_lifecycle_subscription() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_tool_call_message(
        "Agent 4",
        "wait for approval",
        None,
        Some(vec![
            api::LifecycleEventType::Cancelled as i32,
            api::LifecycleEventType::Blocked as i32,
        ]),
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (_, _, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_with_defaults()
    );
    assert_eq!(
        lifecycle_subscription,
        Some(vec![
            LifecycleEventType::Cancelled,
            LifecycleEventType::Blocked
        ])
    );
}

#[test]
fn converts_remote_start_agent_with_environment_id() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_tool_call_message(
        "Agent 5",
        "run in the remote environment",
        Some(remote_start_agent_execution_mode("env-123")),
        None,
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (name, prompt, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);

    assert_eq!(name, "Agent 5");
    assert_eq!(prompt, "run in the remote environment");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::Remote {
            environment_id: "env-123".to_string(),
            skill_references: vec![],
            model_id: String::new(),
            computer_use_enabled: false,
            worker_host: String::new(),
            harness_type: String::new(),
            title: String::new(),
            auth_secret_name: None,
        }
    );
    assert_eq!(lifecycle_subscription, None);
}

#[test]
fn converts_remote_start_agent_v2_with_skill_references() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_v2_tool_call_message(
        "Agent 6",
        "run in the remote environment",
        Some(remote_start_agent_v2_execution_mode("env-123")),
        None,
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (name, prompt, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);

    assert_eq!(name, "Agent 6");
    assert_eq!(prompt, "run in the remote environment");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::Remote {
            environment_id: "env-123".to_string(),
            skill_references: vec![
                SkillReference::Path("/tmp/SKILL.md".into()),
                SkillReference::BundledSkillId("review-comments".to_string()),
            ],
            model_id: "gpt-test".to_string(),
            computer_use_enabled: true,
            worker_host: "worker-host".to_string(),
            harness_type: "claude-code".to_string(),
            title: "Remote child".to_string(),
            auth_secret_name: None,
        }
    );
    assert_eq!(lifecycle_subscription, None);
}

#[test]
fn converts_local_start_agent_v2_with_harness_type() {
    let task_id = TaskId::new("task-id".to_string());
    let message = start_agent_v2_tool_call_message(
        "Agent 6",
        "run in the local claude harness",
        Some(local_start_agent_v2_execution_mode("claude-code")),
        None,
    );

    let output = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("conversion should succeed");

    let (name, prompt, execution_mode, lifecycle_subscription) = extract_start_agent_action(output);

    assert_eq!(name, "Agent 6");
    assert_eq!(prompt, "run in the local claude harness");
    assert_eq!(
        execution_mode,
        StartAgentExecutionMode::local_harness("claude-code".to_string())
    );
    assert_eq!(lifecycle_subscription, None);
}

#[test]
fn transfer_control_tool_call_converts_to_action_message() {
    let task_id = TaskId::new("task".to_string());
    let reason = "Please finish the interactive flow".to_string();
    let message = api::Message {
        id: "message".to_string(),
        task_id: "task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: "tool_call".to_string(),
            tool: Some(
                api::message::tool_call::Tool::TransferShellCommandControlToUser(
                    api::message::tool_call::TransferShellCommandControlToUser {
                        reason: reason.clone(),
                    },
                ),
            ),
        })),
        request_id: "req".to_string(),
        timestamp: None,
    };

    let converted = message
        .to_client_output_message(ConversionParams {
            task_id: &task_id,
            current_todo_list: None,
            active_code_review: None,
        })
        .expect("transfer-control conversion should succeed");

    match converted {
        MaybeAIAgentOutputMessage::Message(output) => match output.message {
            AIAgentOutputMessageType::Action(action) => {
                assert_eq!(action.task_id, task_id);
                assert_eq!(
                    action.action,
                    AIAgentActionType::TransferShellCommandControlToUser { reason }
                );
                assert!(action.requires_result);
            }
            other => panic!("Expected action message, got {other:?}"),
        },
        MaybeAIAgentOutputMessage::NoClientRepresentation => {
            panic!("Expected transfer-control tool call to produce a client action")
        }
    }
}
