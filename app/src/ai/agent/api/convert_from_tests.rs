use ai::agent::action::AskUserQuestionType;
use ai::skills::SkillPathOrigin;
use warp_multi_agent_api as api;

use super::{
    ConversionParams, ConvertAPIMessageToClientOutputMessage, MaybeAIAgentOutputMessage,
    convert_api_question,
};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionType, AIAgentOutputMessageType};

fn upload_artifact_tool_call_message(path: &str, description: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
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

fn file_artifact_created_message(filepath: &str, description: &str) -> api::Message {
    api::Message {
        fetched_memories: vec![],
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
            skill_path_origin: &SkillPathOrigin::Local,
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
            skill_path_origin: &SkillPathOrigin::Local,
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
fn transfer_control_tool_call_converts_to_action_message() {
    let task_id = TaskId::new("task".to_string());
    let reason = "Please finish the interactive flow".to_string();
    let message = api::Message {
        fetched_memories: vec![],
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
            skill_path_origin: &SkillPathOrigin::Local,
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
