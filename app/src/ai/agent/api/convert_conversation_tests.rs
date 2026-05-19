use crate::ai::agent::api::convert_conversation::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{
    AIAgentHarness, AIConversationId, ServerAIConversationMetadata,
};
use crate::ai::agent::{AIAgentInput, UserQueryMode};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::cloud_object::{Revision, ServerMetadata, ServerPermissions};
use crate::persistence::model::ConversationUsageMetadata;
use crate::server::ids::ServerId;
use chrono::Utc;
use std::collections::HashMap;
use warp_multi_agent_api as api;

fn test_server_metadata(
    server_token: &str,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "test conversation".to_string(),
        working_directory: None,
        harness: AIAgentHarness::Oz,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
        },
        metadata: ServerMetadata {
            uid: ServerId::default(),
            revision: Revision::now(),
            metadata_last_updated_ts: Utc::now().into(),
            trashed_ts: None,
            folder_id: None,
            is_welcome_object: false,
            creator_uid: None,
            last_editor_uid: None,
            current_editor_uid: None,
        },
        permissions: ServerPermissions::mock_personal(),
        ambient_agent_task_id,
        server_conversation_token: ServerConversationToken::new(server_token.to_string()),
        artifacts: vec![],
    }
}

fn test_skill() -> api::Skill {
    api::Skill {
        descriptor: Some(api::SkillDescriptor {
            skill_reference: Some(api::skill_descriptor::SkillReference::Path(
                "/tmp/test-skill.md".to_string(),
            )),
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            scope: Some(api::skill_descriptor::Scope {
                r#type: Some(api::skill_descriptor::scope::Type::Project(())),
            }),
            provider: Some(api::skill_descriptor::Provider {
                r#type: Some(api::skill_descriptor::provider::Type::Warp(())),
            }),
        }),
        content: Some(api::FileContent {
            file_path: "/tmp/test-skill.md".to_string(),
            content: "Do the thing".to_string(),
            line_range: None,
        }),
    }
}

#[test]
#[allow(deprecated)]
fn test_convert_conversation_data_to_ai_conversation_sets_restored_run_id() {
    let conversation_id = AIConversationId::new();
    let ambient_agent_task_id: AmbientAgentTaskId =
        "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let conversation_data = api::ConversationData {
        tasks: vec![api::Task {
            id: "root".to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        ordered_message_ids: vec![],
    };

    let conversation = convert_conversation_data_to_ai_conversation(
        conversation_id,
        &conversation_data,
        test_server_metadata("server-token", Some(ambient_agent_task_id)),
        RestorationMode::Continue,
    )
    .expect("conversation should restore");

    assert_eq!(conversation.id(), conversation_id);
    assert_eq!(conversation.task_id(), Some(ambient_agent_task_id));
    assert_eq!(
        conversation.run_id(),
        Some(ambient_agent_task_id.to_string())
    );
}

#[test]
fn test_convert_tool_call_result_to_input_transfer_control_snapshot() {
    let task_id = crate::ai::agent::task::TaskId::new("task".to_string());
    let mut document_versions = HashMap::new();
    let tool_call_result = api::message::ToolCallResult {
        tool_call_id: "tool_call".to_string(),
        context: None,
        result: Some(
            api::message::tool_call_result::Result::TransferShellCommandControlToUser(
                api::TransferShellCommandControlToUserResult {
                    result: Some(
                        api::transfer_shell_command_control_to_user_result::Result::LongRunningCommandSnapshot(
                        api::LongRunningShellCommandSnapshot {
                                command_id: "cmd1".to_string(),
                                output: "snapshot".to_string(),
                                cursor: "<|cursor|>".to_string(),
                                is_alt_screen_active: false,
                                is_preempted: false,
                            },
                        ),
                    ),
                },
            ),
        ),
    };

    let input = convert_tool_call_result_to_input(
        &task_id,
        &tool_call_result,
        &HashMap::new(),
        &mut document_versions,
    )
    .unwrap();

    match input {
        AIAgentInput::ActionResult { result, .. } => match result.result {
            crate::ai::agent::AIAgentActionResultType::TransferShellCommandControlToUser(
                crate::ai::agent::TransferShellCommandControlToUserResult::Snapshot {
                    block_id,
                    grid_contents,
                    cursor,
                    ..
                },
            ) => {
                assert_eq!(block_id.to_string(), "cmd1");
                assert_eq!(grid_contents, "snapshot");
                assert_eq!(cursor, "<|cursor|>");
            }
            other => panic!("Expected transfer-control snapshot result, got {other:?}"),
        },
        other => panic!("Expected action-result input, got {other:?}"),
    }
}

#[test]
fn test_convert_tool_call_result_to_input_upload_artifact_success() {
    let task_id = crate::ai::agent::task::TaskId::new("task".to_string());
    let mut document_versions = HashMap::new();
    let tool_call_result = api::message::ToolCallResult {
        tool_call_id: "tool_call".to_string(),
        context: None,
        result: Some(api::message::tool_call_result::Result::UploadFileArtifact(
            api::UploadFileArtifactResult {
                result: Some(api::upload_file_artifact_result::Result::Success(
                    api::upload_file_artifact_result::Success {
                        artifact_uid: "artifact-123".to_string(),
                        mime_type: "text/plain".to_string(),
                        size_bytes: 42,
                    },
                )),
            },
        )),
    };

    let input = convert_tool_call_result_to_input(
        &task_id,
        &tool_call_result,
        &HashMap::new(),
        &mut document_versions,
    )
    .unwrap();

    match input {
        AIAgentInput::ActionResult { result, .. } => match result.result {
            crate::ai::agent::AIAgentActionResultType::UploadArtifact(
                crate::ai::agent::UploadArtifactResult::Success {
                    artifact_uid,
                    filepath,
                    mime_type,
                    description,
                    size_bytes,
                },
            ) => {
                assert_eq!(artifact_uid, "artifact-123");
                assert_eq!(filepath, None);
                assert_eq!(mime_type, "text/plain");
                assert_eq!(description, None);
                assert_eq!(size_bytes, 42);
            }
            other => panic!("Expected upload-artifact success result, got {other:?}"),
        },
        other => panic!("Expected action-result input, got {other:?}"),
    }
}

#[test]
fn test_convert_tool_call_result_to_input_upload_artifact_missing_result_is_error() {
    let task_id = crate::ai::agent::task::TaskId::new("task".to_string());
    let mut document_versions = HashMap::new();
    let tool_call_result = api::message::ToolCallResult {
        tool_call_id: "tool_call".to_string(),
        context: None,
        result: Some(api::message::tool_call_result::Result::UploadFileArtifact(
            api::UploadFileArtifactResult { result: None },
        )),
    };

    let input = convert_tool_call_result_to_input(
        &task_id,
        &tool_call_result,
        &HashMap::new(),
        &mut document_versions,
    )
    .unwrap();

    match input {
        AIAgentInput::ActionResult { result, .. } => match result.result {
            crate::ai::agent::AIAgentActionResultType::UploadArtifact(
                crate::ai::agent::UploadArtifactResult::Error(message),
            ) => {
                assert_eq!(message, "Upload artifact tool call returned no result");
            }
            other => panic!("Expected upload-artifact error result, got {other:?}"),
        },
        other => panic!("Expected action-result input, got {other:?}"),
    }
}

#[test]
fn test_convert_tool_call_result_to_input_start_agent_v2_results() {
    let task_id = crate::ai::agent::task::TaskId::new("task".to_string());

    let cases = [
        (
            "success",
            Some(api::start_agent_v2_result::Result::Success(
                api::start_agent_v2_result::Success {
                    agent_id: "agent-123".to_string(),
                },
            )),
        ),
        (
            "error",
            Some(api::start_agent_v2_result::Result::Error(
                api::start_agent_v2_result::Error {
                    error: "child failed".to_string(),
                },
            )),
        ),
        ("cancelled", None),
    ];

    for (name, result) in cases {
        let mut document_versions = HashMap::new();
        let tool_call_result = api::message::ToolCallResult {
            tool_call_id: format!("tool_call_{name}"),
            context: None,
            result: Some(api::message::tool_call_result::Result::StartAgentV2(
                api::StartAgentV2Result { result },
            )),
        };

        let input = convert_tool_call_result_to_input(
            &task_id,
            &tool_call_result,
            &HashMap::new(),
            &mut document_versions,
        )
        .unwrap();

        match input {
            AIAgentInput::ActionResult { result, .. } => match result.result {
                crate::ai::agent::AIAgentActionResultType::StartAgent(
                    crate::ai::agent::StartAgentResult::Success { agent_id, version },
                ) if name == "success" => {
                    assert_eq!(agent_id, "agent-123");
                    assert_eq!(version, ai::agent::action_result::StartAgentVersion::V2);
                }
                crate::ai::agent::AIAgentActionResultType::StartAgent(
                    crate::ai::agent::StartAgentResult::Error { error, version },
                ) if name == "error" => {
                    assert_eq!(error, "child failed");
                    assert_eq!(version, ai::agent::action_result::StartAgentVersion::V2);
                }
                crate::ai::agent::AIAgentActionResultType::StartAgent(
                    crate::ai::agent::StartAgentResult::Cancelled { version },
                ) if name == "cancelled" => {
                    assert_eq!(version, ai::agent::action_result::StartAgentVersion::V2);
                }
                other => panic!("Unexpected start-agent-v2 result for {name}: {other:?}"),
            },
            other => panic!("Expected action-result input for {name}, got {other:?}"),
        }
    }
}

#[test]
fn test_convert_tool_call_result_to_input_transfer_control_cancelled() {
    let task_id = crate::ai::agent::task::TaskId::new("task".to_string());
    let mut document_versions = HashMap::new();
    let original_tool_call = api::message::ToolCall {
        tool_call_id: "tool_call".to_string(),
        tool: Some(
            api::message::tool_call::Tool::TransferShellCommandControlToUser(
                api::message::tool_call::TransferShellCommandControlToUser {
                    reason: "Need user help".to_string(),
                },
            ),
        ),
    };
    let mut tool_call_map = HashMap::new();
    tool_call_map.insert("tool_call".to_string(), &original_tool_call);
    let tool_call_result = api::message::ToolCallResult {
        tool_call_id: "tool_call".to_string(),
        context: None,
        result: Some(api::message::tool_call_result::Result::Cancel(())),
    };

    let input = convert_tool_call_result_to_input(
        &task_id,
        &tool_call_result,
        &tool_call_map,
        &mut document_versions,
    )
    .unwrap();

    match input {
        AIAgentInput::ActionResult { result, .. } => match result.result {
            crate::ai::agent::AIAgentActionResultType::TransferShellCommandControlToUser(
                crate::ai::agent::TransferShellCommandControlToUserResult::Cancelled,
            ) => {}
            other => panic!("Expected cancelled transfer-control result, got {other:?}"),
        },
        other => panic!("Expected action-result input, got {other:?}"),
    }
}

#[test]
fn test_into_exchanges_basic() {
    // Create minimal test data
    let messages = vec![
        api::Message {
            id: "user_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "test query".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "agent_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "test response".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "user_msg2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "second query".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "agent_msg2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "second response".to_string(),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "user_msg3".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "third query".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req3".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "agent_msg3".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "third response".to_string(),
                },
            )),
            request_id: "req3".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    // Test the function
    let exchanges = task.into_exchanges();

    // We expect 3 exchanges (one for each user query + agent response pair)
    assert_eq!(exchanges.len(), 3, "Should create 3 exchanges");

    // Each exchange should have exactly one input (the user query)
    for exchange in &exchanges {
        assert_eq!(
            exchange.input.len(),
            1,
            "Each exchange should have one input"
        );
    }
}

#[test]
fn test_invoke_skill_arguments_round_trip() {
    let query = "arg1 arg2".to_string();
    let messages = vec![
        api::Message {
            id: "invoke_skill_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::InvokeSkill(
                api::message::InvokeSkill {
                    skill: Some(test_skill()),
                    user_query: Some(api::message::UserQuery {
                        query: query.clone(),
                        context: None,
                        referenced_attachments: HashMap::new(),
                        mode: None,
                        intended_agent: Default::default(),
                    }),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        api::Message {
            id: "agent_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Done".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1);

    match &exchanges[0].input[0] {
        AIAgentInput::InvokeSkill {
            skill, user_query, ..
        } => {
            assert_eq!(skill.name, "test-skill");
            assert_eq!(
                user_query.as_ref().map(|uq| uq.query.as_str()),
                Some("arg1 arg2")
            );
            assert_eq!(
                exchanges[0].input[0].user_query().as_deref(),
                Some("/test-skill arg1 arg2")
            );
        }
        input => panic!("Expected InvokeSkill input, got {input:?}"),
    }
}

#[test]
fn test_invoke_skill_missing_user_query_maps_to_none() {
    let messages = vec![api::Message {
        id: "invoke_skill_msg".to_string(),
        task_id: "task1".to_string(),
        server_message_data: "".to_string(),
        citations: vec![],
        message: Some(api::message::Message::InvokeSkill(
            api::message::InvokeSkill {
                skill: Some(test_skill()),
                user_query: None,
            },
        )),
        request_id: "req1".to_string(),
        timestamp: None,
    }];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1);

    match &exchanges[0].input[0] {
        AIAgentInput::InvokeSkill {
            skill, user_query, ..
        } => {
            assert_eq!(skill.name, "test-skill");
            assert_eq!(user_query, &None);
            assert_eq!(
                exchanges[0].input[0].user_query().as_deref(),
                Some("/test-skill")
            );
        }
        input => panic!("Expected InvokeSkill input, got {input:?}"),
    }
}

#[test]
fn test_into_exchanges_with_tool_calls_and_cancellation() {
    let messages = vec![
        // User query
        api::Message {
            id: "user_query".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "run parallel commands".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent response
        api::Message {
            id: "agent_response".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Running commands in parallel".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Tool call 1
        api::Message {
            id: "tool_call_1".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "call_1".to_string(),
                tool: Some(api::message::tool_call::Tool::RunShellCommand(
                    api::message::tool_call::RunShellCommand {
                        command: "echo 1".to_string(),
                        is_read_only: false,
                        uses_pager: false,
                        citations: vec![],
                        is_risky: false,
                        wait_until_complete_value: None,
                        risk_category: 0,
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Tool call 2
        api::Message {
            id: "tool_call_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "call_2".to_string(),
                tool: Some(api::message::tool_call::Tool::RunShellCommand(
                    api::message::tool_call::RunShellCommand {
                        command: "echo 2".to_string(),
                        is_read_only: false,
                        uses_pager: false,
                        citations: vec![],
                        is_risky: false,
                        wait_until_complete_value: None,
                        risk_category: 0,
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Tool call 3
        api::Message {
            id: "tool_call_3".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "call_3".to_string(),
                tool: Some(api::message::tool_call::Tool::RunShellCommand(
                    api::message::tool_call::RunShellCommand {
                        command: "echo 3".to_string(),
                        is_read_only: false,
                        uses_pager: false,
                        citations: vec![],
                        is_risky: false,
                        wait_until_complete_value: None,
                        risk_category: 0,
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Tool call result - cancelled (call_2)
        api::Message {
            id: "result_cancelled".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "call_2".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::Cancel(())),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Tool call result - success (call_1)
        api::Message {
            id: "result_success_1".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "call_1".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::RunShellCommand(
                        #[allow(deprecated)]
                        api::RunShellCommandResult {
                            command: "echo 1".to_string(),
                            output: Default::default(),
                            exit_code: Default::default(),
                            result: Some(api::run_shell_command_result::Result::CommandFinished(
                                api::ShellCommandFinished {
                                    command_id: "command_1".to_string(),
                                    output: "1".to_string(),
                                    exit_code: 0,
                                    start_ts: None,
                                    finish_ts: None,
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Tool call result - success (call_3)
        api::Message {
            id: "result_success_3".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "call_3".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::RunShellCommand(
                        #[allow(deprecated)]
                        api::RunShellCommandResult {
                            command: "echo 3".to_string(),
                            output: "".to_string(),
                            exit_code: 0,
                            result: Some(api::run_shell_command_result::Result::CommandFinished(
                                api::ShellCommandFinished {
                                    command_id: "command_2".to_string(),
                                    output: "3".to_string(),
                                    exit_code: 0,
                                    start_ts: None,
                                    finish_ts: None,
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Final agent response
        api::Message {
            id: "final_response".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Commands completed - 2 succeeded, 1 cancelled".to_string(),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Follow-up user query
        api::Message {
            id: "followup_query".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "did it work?".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req3".to_string(),
            timestamp: None,
        },
        // Final agent response
        api::Message {
            id: "final_response2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Yes, partially worked".to_string(),
                },
            )),
            request_id: "req3".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    // Expected 3 persisted blocks based on the pattern
    let expected_persisted_block_count = 3;

    // Test the function
    let exchanges = task.into_exchanges();

    // Verify we get the expected number of exchanges
    assert_eq!(
        exchanges.len(),
        expected_persisted_block_count,
        "Should create {expected_persisted_block_count} exchanges to match persisted blocks"
    );

    // First exchange: user query + agent output + tool calls
    let first_exchange = &exchanges[0];
    assert_eq!(
        first_exchange.input.len(),
        1,
        "First exchange should have 1 input (user query)"
    );

    // Second exchange: action results + agent response
    let second_exchange = &exchanges[1];
    let action_result_count = second_exchange
        .input
        .iter()
        .filter(|input| matches!(input, crate::ai::agent::AIAgentInput::ActionResult { .. }))
        .count();
    assert_eq!(
        action_result_count, 3,
        "Second exchange should have 3 action results"
    );

    // Third exchange: follow-up query + response
    let third_exchange = &exchanges[2];
    assert_eq!(
        third_exchange.input.len(),
        1,
        "Third exchange should have 1 input (follow-up query)"
    );

    // Verify tool call results include both successful and cancelled
    let mut found_cancelled = false;
    let mut found_successful = 0;

    for input in &second_exchange.input {
        if let crate::ai::agent::AIAgentInput::ActionResult { result, .. } = input {
            if let crate::ai::agent::AIAgentActionResultType::RequestCommandOutput(command_result) =
                &result.result
            {
                match command_result {
                    crate::ai::agent::RequestCommandOutputResult::CancelledBeforeExecution => {
                        found_cancelled = true;
                    }
                    crate::ai::agent::RequestCommandOutputResult::Completed { .. } => {
                        found_successful += 1;
                    }
                    _ => {}
                }
            }
        }
    }

    assert!(
        found_cancelled,
        "Should find at least one cancelled tool call result"
    );
    assert_eq!(
        found_successful, 2,
        "Should find exactly 2 successful tool call results"
    );
}

#[test]
fn test_into_exchanges_with_code_diffs() {
    let messages = vec![
        // User query asking for code changes
        api::Message {
            id: "user_query".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "Fix the imports in main.rs".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent response
        api::Message {
            id: "agent_response".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "I'll fix the import issues".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // File diff tool call
        api::Message {
            id: "diff_call".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "diff_1".to_string(),
                tool: Some(api::message::tool_call::Tool::ApplyFileDiffs(
                    api::message::tool_call::ApplyFileDiffs {
                        summary: "".to_string(),
                        new_files: vec![],
                        diffs: vec![],
                        v4a_updates: vec![],
                        deleted_files: vec![],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // User cancels the diff
        api::Message {
            id: "diff_cancelled".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "diff_1".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::Cancel(())),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // User provides feedback
        api::Message {
            id: "user_feedback".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "Actually, let's remove the unused import instead".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Agent response
        api::Message {
            id: "agent_response_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "I'll remove the unused import".to_string(),
                },
            )),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // Second file diff tool call
        api::Message {
            id: "diff_call_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "diff_2".to_string(),
                tool: Some(api::message::tool_call::Tool::ApplyFileDiffs(
                    api::message::tool_call::ApplyFileDiffs {
                        summary: "".to_string(),
                        new_files: vec![],
                        diffs: vec![],
                        v4a_updates: vec![],
                        deleted_files: vec![],
                    },
                )),
            })),
            request_id: "req2".to_string(),
            timestamp: None,
        },
        // User accepts the diff
        api::Message {
            id: "diff_accepted".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "diff_2".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::ApplyFileDiffs(
                        api::ApplyFileDiffsResult {
                            result: Some(api::apply_file_diffs_result::Result::Success(
                                #[allow(deprecated)]
                                api::apply_file_diffs_result::Success {
                                    updated_files_v2: vec![],
                                    updated_files: vec![],
                                    deleted_files: vec![],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req3".to_string(),
            timestamp: None,
        },
        // Final agent response
        api::Message {
            id: "final_response".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Fixed! Removed the unused import.".to_string(),
                },
            )),
            request_id: "req3".to_string(),
            timestamp: None,
        },
        // Follow-up user query
        api::Message {
            id: "followup".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "Great! Now does it compile?".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req4".to_string(),
            timestamp: None,
        },
        // Final agent response
        api::Message {
            id: "final_response_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Yes, it should compile cleanly now!".to_string(),
                },
            )),
            request_id: "req4".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    // Expected 4 persisted blocks: initial query+diff, cancelled diff feedback, accepted diff, followup
    let expected_persisted_block_count = 4;

    // Test the function
    let exchanges = task.into_exchanges();

    // Verify we get the expected number of exchanges
    assert_eq!(
        exchanges.len(),
        expected_persisted_block_count,
        "Should create {expected_persisted_block_count} exchanges to match persisted blocks"
    );

    // Verify the first exchange has a user query
    let first_exchange = &exchanges[0];
    match &first_exchange.input[0] {
        crate::ai::agent::AIAgentInput::UserQuery { query, .. } => {
            assert_eq!(query, "Fix the imports in main.rs");
        }
        _ => panic!("First exchange should start with user query"),
    }

    // Verify the second exchange has a cancelled file diff result
    let second_exchange = &exchanges[1];
    let has_cancelled_diff = second_exchange.input.iter().any(|input| {
        if let crate::ai::agent::AIAgentInput::ActionResult { result, .. } = input {
            matches!(
                result.result,
                crate::ai::agent::AIAgentActionResultType::RequestFileEdits(
                    crate::ai::agent::RequestFileEditsResult::Cancelled
                )
            )
        } else {
            false
        }
    });
    assert!(
        has_cancelled_diff,
        "Should have cancelled diff in second exchange"
    );

    // Verify the third exchange has a successful file diff result
    let third_exchange = &exchanges[2];
    let has_successful_diff = third_exchange.input.iter().any(|input| {
        if let crate::ai::agent::AIAgentInput::ActionResult { result, .. } = input {
            matches!(
                result.result,
                crate::ai::agent::AIAgentActionResultType::RequestFileEdits(
                    crate::ai::agent::RequestFileEditsResult::Success { .. }
                )
            )
        } else {
            false
        }
    });
    assert!(
        has_successful_diff,
        "Should have successful diff in third exchange"
    );

    // Verify the fourth exchange is the follow-up query
    let fourth_exchange = &exchanges[3];
    match &fourth_exchange.input[0] {
        crate::ai::agent::AIAgentInput::UserQuery { query, .. } => {
            assert_eq!(query, "Great! Now does it compile?");
        }
        _ => panic!("Fourth exchange should start with follow-up query"),
    }
}

#[test]
fn test_user_query_mode_conversion() {
    // Test conversion with Plan mode
    let messages = vec![api::Message {
        id: "user_msg".to_string(),
        task_id: "task1".to_string(),
        server_message_data: "".to_string(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "test query with plan mode".to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: Some(api::UserQueryMode {
                r#type: Some(api::user_query_mode::Type::Plan(())),
            }),
            intended_agent: Default::default(),
        })),
        request_id: String::new(),
        timestamp: None,
    }];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1);

    match &exchanges[0].input[0] {
        AIAgentInput::UserQuery {
            user_query_mode: UserQueryMode::Plan,
            ..
        } => {
            // Success - the mode was correctly converted
        }
        AIAgentInput::UserQuery {
            user_query_mode, ..
        } => {
            panic!("Expected Plan mode, got: {user_query_mode:?}");
        }
        _ => panic!("Expected UserQuery input"),
    }

    // Test conversion with Normal mode (no type set)
    let messages_normal = vec![api::Message {
        id: "user_msg".to_string(),
        task_id: "task1".to_string(),
        server_message_data: "".to_string(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "test query with normal mode".to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: Some(api::UserQueryMode { r#type: None }),
            intended_agent: Default::default(),
        })),
        request_id: String::new(),
        timestamp: None,
    }];

    let task_normal = api::Task {
        id: "task1".to_string(),
        messages: messages_normal,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges_normal = task_normal.into_exchanges();
    assert_eq!(exchanges_normal.len(), 1);

    match &exchanges_normal[0].input[0] {
        AIAgentInput::UserQuery {
            user_query_mode: UserQueryMode::Normal,
            ..
        } => {
            // Success - the mode was correctly converted
        }
        AIAgentInput::UserQuery {
            user_query_mode, ..
        } => {
            panic!("Expected Normal mode, got: {user_query_mode:?}");
        }
        _ => panic!("Expected UserQuery input"),
    }

    // Test conversion with no mode field (should default to Normal)
    let messages_default = vec![api::Message {
        id: "user_msg".to_string(),
        task_id: "task1".to_string(),
        server_message_data: "".to_string(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "test query with default mode".to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: String::new(),
        timestamp: None,
    }];

    let task_default = api::Task {
        id: "task1".to_string(),
        messages: messages_default,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges_default = task_default.into_exchanges();
    assert_eq!(exchanges_default.len(), 1);

    match &exchanges_default[0].input[0] {
        AIAgentInput::UserQuery {
            user_query_mode: UserQueryMode::Normal,
            ..
        } => {
            // Success - the mode was correctly converted to default (Normal)
        }
        AIAgentInput::UserQuery {
            user_query_mode, ..
        } => {
            panic!("Expected Normal (default) mode, got: {user_query_mode:?}");
        }
        _ => panic!("Expected UserQuery input"),
    }
}

#[test]
fn test_exchanges_grouped_by_request_id() {
    // This test is based on a real example where messages should be grouped by request_id
    // Request 1: 78e236b8-84a2-45df-876e-ebfb86ceafc4 (UserQuery + AgentOutput + ToolCall)
    // Request 2: 59a3947f-fc7e-413a-96b5-baecd7e406dc (ToolCallResult + ToolCall for subagent)
    // Request 3: 9f85acb2-0b1f-41b1-a0de-3623e131758a (Subagent result + Final ToolCallResult + AgentOutput)

    let messages = vec![
        // Message 0: Server message (should be ignored or handled gracefully)
        api::Message {
            id: "2512077c-0ede-46b0-8f69-230c8792df07".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "78e236b8-84a2-45df-876e-ebfb86ceafc4".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "39740c13-892c-4f0e-8aa9-6d305d81f174".to_string(),
                tool: Some(api::message::tool_call::Tool::Server(
                    api::message::tool_call::Server {
                        payload: String::new(),
                    },
                )),
            })),
        },
        // Message 1: User query with request_id 78e236b8
        api::Message {
            id: "4d6c450d-3d54-446f-974c-5c414e6083e9".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "78e236b8-84a2-45df-876e-ebfb86ceafc4".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "demonstrate your ability".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
        },
        // Message 2: Agent output with same request_id
        api::Message {
            id: "10210d1a-5298-45ef-90ba-df6367805080".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "78e236b8-84a2-45df-876e-ebfb86ceafc4".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "I'll demonstrate opening".to_string(),
                },
            )),
        },
        // Message 3: Tool call with same request_id
        api::Message {
            id: "936c7c86-eb4a-4edf-97c0-22f5c61b35a6".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "78e236b8-84a2-45df-876e-ebfb86ceafc4".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "toolu_016dGTLFAPqvxW9yyFcF1WzT".to_string(),
                tool: Some(api::message::tool_call::Tool::RunShellCommand(
                    api::message::tool_call::RunShellCommand {
                        command: "vim".to_string(),
                        is_read_only: true,
                        uses_pager: false,
                        citations: vec![],
                        is_risky: false,
                        wait_until_complete_value: None,
                        risk_category: 0,
                    },
                )),
            })),
        },
        // Message 4: Tool call result with NEW request_id 59a3947f (starts new exchange)
        api::Message {
            id: "cbebf5fb-4dd8-4aef-be45-bb916eff552c".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "59a3947f-fc7e-413a-96b5-baecd7e406dc".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "toolu_016dGTLFAPqvxW9yyFcF1WzT".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::RunShellCommand(
                        #[allow(deprecated)]
                        api::RunShellCommandResult {
                            command: "vim".to_string(),
                            output: "".to_string(),
                            exit_code: 0,
                            result: Some(
                                api::run_shell_command_result::Result::LongRunningCommandSnapshot(
                                    api::LongRunningShellCommandSnapshot {
                                        command_id: "cmd1".to_string(),
                                        output: "\n<|cursor|>".to_string(),
                                        cursor: String::new(),
                                        is_alt_screen_active: false,
                                        is_preempted: false,
                                    },
                                ),
                            ),
                        },
                    )),
                },
            )),
        },
        // Message 5: Agent output with same request_id
        api::Message {
            id: "7a89857d-fa33-4d45-88e3-5fa9cbce3f20".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "59a3947f-fc7e-413a-96b5-baecd7e406dc".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Vim opened in interact mode".to_string(),
                },
            )),
        },
        // Message 6: Write to long running command with NEW request_id 9f85acb2 (starts new exchange)
        api::Message {
            id: "dac6d336-9fcb-4e34-bc2b-b06e70f52ec5".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "9f85acb2-0b1f-41b1-a0de-3623e131758a".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "toolu_write_to_vim".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::WriteToLongRunningShellCommand(
                        api::WriteToLongRunningShellCommandResult {
                            result: Some(
                                api::write_to_long_running_shell_command_result::Result::LongRunningCommandSnapshot(
                                    api::LongRunningShellCommandSnapshot {
                                        command_id: "cmd1".to_string(),
                                        output: "wrote to vim".to_string(),
                                        cursor: String::new(),
                                        is_alt_screen_active: false,
                                        is_preempted: false,
                                    },
                                ),
                            ),
                        },
                    )),
                },
            )),
        },
        // Message 7: Final tool call result with same request_id
        api::Message {
            id: "ad319d66-fac0-4169-8bf1-e6004aca1619".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "9f85acb2-0b1f-41b1-a0de-3623e131758a".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "toolu_016dGTLFAPqvxW9yyFcF1WzT".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::RunShellCommand(
                        #[allow(deprecated)]
                        api::RunShellCommandResult {
                            command: "vim".to_string(),
                            output: "".to_string(),
                            exit_code: 0,
                            result: Some(api::run_shell_command_result::Result::CommandFinished(
                                api::ShellCommandFinished {
                                    command_id: "cmd1".to_string(),
                                    output: "Done".to_string(),
                                    exit_code: 0,
                                    start_ts: None,
                                    finish_ts: None,
                                },
                            )),
                        },
                    )),
                },
            )),
        },
        // Message 8: Final agent output with same request_id
        api::Message {
            id: "f15f8a59-2e9c-416e-b216-83b3bd52d6be".to_string(),
            task_id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
            request_id: "9f85acb2-0b1f-41b1-a0de-3623e131758a".to_string(),
            timestamp: None,
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Done. Vim was opened and closed successfully".to_string(),
                },
            )),
        },
    ];

    let task = api::Task {
        id: "d02463e1-2429-48de-ac8f-552df4acc4d0".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();

    // We expect 3 exchanges based on the 3 different request_ids
    assert_eq!(
        exchanges.len(),
        3,
        "Should create 3 exchanges based on 3 different request_ids"
    );

    // First exchange: request_id 78e236b8 (UserQuery + outputs)
    let first_exchange = &exchanges[0];
    assert_eq!(
        first_exchange.input.len(),
        1,
        "First exchange should have 1 input (user query)"
    );
    assert!(
        matches!(first_exchange.input[0], AIAgentInput::UserQuery { .. }),
        "First input should be a UserQuery"
    );

    // Second exchange: request_id 59a3947f (ToolCallResult input)
    let second_exchange = &exchanges[1];
    assert_eq!(
        second_exchange.input.len(),
        1,
        "Second exchange should have 1 input (tool call result from vim command)"
    );
    assert!(
        matches!(second_exchange.input[0], AIAgentInput::ActionResult { .. }),
        "Second input should be an ActionResult"
    );

    // Third exchange: request_id 9f85acb2 (2 ToolCallResults + output)
    let third_exchange = &exchanges[2];
    assert_eq!(
        third_exchange.input.len(),
        2,
        "Third exchange should have 2 inputs (write command + final result)"
    );
    assert!(
        matches!(third_exchange.input[0], AIAgentInput::ActionResult { .. }),
        "Third exchange first input should be an ActionResult"
    );
    assert!(
        matches!(third_exchange.input[1], AIAgentInput::ActionResult { .. }),
        "Third exchange second input should be an ActionResult"
    );
}

/// Regression test for APP-3273: Multiple CreateDocuments tool call results should each get
/// the default document version (v1), since each creates a brand-new document.
/// Previously, a global version counter was used, so the second CreateDocuments got v2,
/// causing a version mismatch during restoration and preventing the inline action from rendering.
#[test]
fn test_multiple_create_documents_get_default_version() {
    use crate::ai::agent::{AIAgentActionResultType, CreateDocumentsResult};
    use crate::ai::document::ai_document_model::AIDocumentVersion;

    let doc_id_a = uuid::Uuid::new_v4().to_string();
    let doc_id_b = uuid::Uuid::new_v4().to_string();

    let messages = vec![
        // User query
        api::Message {
            id: "user_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "create two plans".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output
        api::Message {
            id: "agent_text".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Creating plan A".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // First CreateDocuments tool call
        api::Message {
            id: "tool_call_create_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "tc_create_a".to_string(),
                tool: Some(api::message::tool_call::Tool::CreateDocuments(
                    api::message::tool_call::CreateDocuments {
                        new_documents: vec![
                            api::message::tool_call::create_documents::NewDocument {
                                title: "Plan A".to_string(),
                                content: "# Plan A".to_string(),
                            },
                        ],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // First CreateDocuments result
        api::Message {
            id: "result_create_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "tc_create_a".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::CreateDocuments(
                        api::CreateDocumentsResult {
                            result: Some(api::create_documents_result::Result::Success(
                                api::create_documents_result::Success {
                                    created_documents: vec![api::DocumentContent {
                                        document_id: doc_id_a.clone(),
                                        content: "# Plan A content".to_string(),
                                        line_range: None,
                                    }],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output before second plan
        api::Message {
            id: "agent_text_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Creating plan B".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Second CreateDocuments tool call
        api::Message {
            id: "tool_call_create_b".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "tc_create_b".to_string(),
                tool: Some(api::message::tool_call::Tool::CreateDocuments(
                    api::message::tool_call::CreateDocuments {
                        new_documents: vec![
                            api::message::tool_call::create_documents::NewDocument {
                                title: "Plan B".to_string(),
                                content: "# Plan B".to_string(),
                            },
                        ],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Second CreateDocuments result
        api::Message {
            id: "result_create_b".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "tc_create_b".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::CreateDocuments(
                        api::CreateDocumentsResult {
                            result: Some(api::create_documents_result::Result::Success(
                                api::create_documents_result::Success {
                                    created_documents: vec![api::DocumentContent {
                                        document_id: doc_id_b.clone(),
                                        content: "# Plan B content".to_string(),
                                        line_range: None,
                                    }],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1, "Should create 1 exchange");

    // Find all CreateDocuments action results and verify their versions
    let create_doc_versions: Vec<AIDocumentVersion> = exchanges[0]
        .input
        .iter()
        .filter_map(|input| match input {
            AIAgentInput::ActionResult { result, .. } => match &result.result {
                AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                    created_documents,
                }) => Some(
                    created_documents
                        .iter()
                        .map(|doc| doc.document_version)
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            },
            _ => None,
        })
        .flatten()
        .collect();

    assert_eq!(
        create_doc_versions.len(),
        2,
        "Should have 2 CreateDocuments results"
    );

    // Both documents should have the default version since they are newly created.
    let default_version = AIDocumentVersion::default();
    assert_eq!(
        create_doc_versions[0], default_version,
        "First created document should have default version"
    );
    assert_eq!(
        create_doc_versions[1], default_version,
        "Second created document should also have default version (regression: APP-3273)"
    );
}

/// Test that the create-then-edit flow produces correct per-document versions:
/// - Create doc A → v1
/// - Edit doc A → v2 (incremented from v1)
/// - Create doc B → v1 (independent new document)
#[test]
fn test_create_then_edit_then_create_version_tracking() {
    use crate::ai::agent::{AIAgentActionResultType, CreateDocumentsResult, EditDocumentsResult};
    use crate::ai::document::ai_document_model::AIDocumentVersion;

    let doc_id_a = uuid::Uuid::new_v4().to_string();
    let doc_id_b = uuid::Uuid::new_v4().to_string();

    let messages = vec![
        // User query
        api::Message {
            id: "user_msg".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query: "create and edit plans".to_string(),
                context: None,
                referenced_attachments: HashMap::new(),
                mode: None,
                intended_agent: Default::default(),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output
        api::Message {
            id: "agent_text".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Creating plan A".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Create doc A tool call
        api::Message {
            id: "tool_call_create_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "tc_create_a".to_string(),
                tool: Some(api::message::tool_call::Tool::CreateDocuments(
                    api::message::tool_call::CreateDocuments {
                        new_documents: vec![
                            api::message::tool_call::create_documents::NewDocument {
                                title: "Plan A".to_string(),
                                content: "# Plan A".to_string(),
                            },
                        ],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Create doc A result
        api::Message {
            id: "result_create_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "tc_create_a".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::CreateDocuments(
                        api::CreateDocumentsResult {
                            result: Some(api::create_documents_result::Result::Success(
                                api::create_documents_result::Success {
                                    created_documents: vec![api::DocumentContent {
                                        document_id: doc_id_a.clone(),
                                        content: "# Plan A v1".to_string(),
                                        line_range: None,
                                    }],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output before edit
        api::Message {
            id: "agent_text_2".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Editing plan A".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Edit doc A tool call
        api::Message {
            id: "tool_call_edit_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "tc_edit_a".to_string(),
                tool: Some(api::message::tool_call::Tool::EditDocuments(
                    api::message::tool_call::EditDocuments {
                        diffs: vec![api::message::tool_call::edit_documents::DocumentDiff {
                            document_id: doc_id_a.clone(),
                            search: "# Plan A".to_string(),
                            replace: "# Plan A (edited)".to_string(),
                        }],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Edit doc A result
        api::Message {
            id: "result_edit_a".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "tc_edit_a".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::EditDocuments(
                        api::EditDocumentsResult {
                            result: Some(api::edit_documents_result::Result::Success(
                                api::edit_documents_result::Success {
                                    updated_documents: vec![api::DocumentContent {
                                        document_id: doc_id_a.clone(),
                                        content: "# Plan A (edited)".to_string(),
                                        line_range: None,
                                    }],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output before second create
        api::Message {
            id: "agent_text_3".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "Creating plan B".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Create doc B tool call
        api::Message {
            id: "tool_call_create_b".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "tc_create_b".to_string(),
                tool: Some(api::message::tool_call::Tool::CreateDocuments(
                    api::message::tool_call::CreateDocuments {
                        new_documents: vec![
                            api::message::tool_call::create_documents::NewDocument {
                                title: "Plan B".to_string(),
                                content: "# Plan B".to_string(),
                            },
                        ],
                    },
                )),
            })),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Create doc B result
        api::Message {
            id: "result_create_b".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "tc_create_b".to_string(),
                    context: None,
                    result: Some(api::message::tool_call_result::Result::CreateDocuments(
                        api::CreateDocumentsResult {
                            result: Some(api::create_documents_result::Result::Success(
                                api::create_documents_result::Success {
                                    created_documents: vec![api::DocumentContent {
                                        document_id: doc_id_b.clone(),
                                        content: "# Plan B v1".to_string(),
                                        line_range: None,
                                    }],
                                },
                            )),
                        },
                    )),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1, "Should create 1 exchange");

    let default_version = AIDocumentVersion::default();

    // Collect all document-related action results in order
    let mut create_a_version: Option<AIDocumentVersion> = None;
    let mut edit_a_version: Option<AIDocumentVersion> = None;
    let mut create_b_version: Option<AIDocumentVersion> = None;

    for input in &exchanges[0].input {
        if let AIAgentInput::ActionResult { result, .. } = input {
            match &result.result {
                AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
                    created_documents,
                }) => {
                    for doc in created_documents {
                        let id_str = doc.document_id.to_string();
                        if id_str == doc_id_a {
                            create_a_version = Some(doc.document_version);
                        } else if id_str == doc_id_b {
                            create_b_version = Some(doc.document_version);
                        }
                    }
                }
                AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success {
                    updated_documents,
                }) => {
                    for doc in updated_documents {
                        let id_str = doc.document_id.to_string();
                        if id_str == doc_id_a {
                            edit_a_version = Some(doc.document_version);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Create doc A should be v1 (default)
    assert_eq!(
        create_a_version,
        Some(default_version),
        "Created doc A should have default version (v1)"
    );

    // Edit doc A should be v2 (one increment from v1)
    assert_eq!(
        edit_a_version,
        Some(default_version.next()),
        "Edited doc A should have version v2 (incremented from create)"
    );

    // Create doc B should be v1 (independent new document)
    assert_eq!(
        create_b_version,
        Some(default_version),
        "Created doc B should have default version (v1), independent of doc A"
    );
}

/// Verify that a `SystemQuery::HandoffRehydration` message does not produce
/// a displayed input when restoring a conversation. It must be treated as
/// hidden, so the exchange should have zero user-visible inputs.
#[test]
fn test_handoff_rehydration_system_query_is_hidden() {
    let messages = vec![
        // HandoffRehydration system query – should be hidden
        api::Message {
            id: "msg_handoff".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::SystemQuery(
                api::message::SystemQuery {
                    r#type: Some(api::message::system_query::Type::HandoffRehydration(
                        api::message::HandoffRehydration {
                            instructions: "restore handoff state".to_string(),
                        },
                    )),
                    context: None,
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
        // Agent output that follows the hidden system query
        api::Message {
            id: "msg_output".to_string(),
            task_id: "task1".to_string(),
            server_message_data: "".to_string(),
            citations: vec![],
            message: Some(api::message::Message::AgentOutput(
                api::message::AgentOutput {
                    text: "I have restored the handoff state.".to_string(),
                },
            )),
            request_id: "req1".to_string(),
            timestamp: None,
        },
    ];

    let task = api::Task {
        id: "task1".to_string(),
        messages,
        dependencies: None,
        description: "".to_string(),
        summary: "".to_string(),
        server_data: "".to_string(),
    };

    let exchanges = task.into_exchanges();
    assert_eq!(exchanges.len(), 1, "Should produce exactly one exchange");

    let exchange = &exchanges[0];
    // The HandoffRehydration should NOT appear as input
    assert!(
        exchange.input.is_empty(),
        "HandoffRehydration must not produce a displayed input, got: {:?}",
        exchange.input
    );

    // The agent output should still be present
    let output = exchange.output_status.output().expect("should have output");
    assert!(
        !output.get().messages.is_empty(),
        "Agent output should still be rendered"
    );
}
