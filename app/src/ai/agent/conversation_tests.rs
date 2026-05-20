use std::collections::HashMap;

use super::{
    artifact_from_fork_proto, AIConversation, AIConversationAutoexecuteMode, AIConversationId,
};
use crate::ai::artifacts::Artifact;
use crate::persistence::model::AgentConversationData;
use warp_core::features::FeatureFlag;
use warp_multi_agent_api as api;

fn restored_conversation(conversation_data: Option<AgentConversationData>) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        conversation_data,
    )
    .unwrap()
}

fn user_query_message(id: &str, request_id: &str, query: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_string(),
            context: None,
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn agent_output_message(id: &str, request_id: &str) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: "Done".to_string(),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
    }
}

fn restored_conversation_with_queries(queries: &[&str]) -> AIConversation {
    let messages = queries
        .iter()
        .enumerate()
        .flat_map(|(index, query)| {
            let request_id = format!("request-{index}");
            [
                user_query_message(&format!("user-{index}"), &request_id, query),
                agent_output_message(&format!("agent-{index}"), &request_id),
            ]
        })
        .collect();

    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "root-task".to_string(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
    )
    .unwrap()
}

#[test]
fn latest_user_query_returns_latest_non_empty_user_query() {
    let conversation =
        restored_conversation_with_queries(&["write unit tests", "fix the failing test"]);

    assert_eq!(
        conversation.latest_user_query(),
        Some("fix the failing test".to_string())
    );
}

#[test]
fn latest_user_query_trims_and_skips_empty_queries() {
    let conversation = restored_conversation_with_queries(&["  write unit tests  ", "  "]);

    assert_eq!(
        conversation.latest_user_query(),
        Some("write unit tests".to_string())
    );
}

#[test]
fn restored_conversation_defaults_autoexecute_override_when_not_persisted() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null}"#).unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn restored_conversation_uses_persisted_last_event_sequence() {
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null,"last_event_sequence":42}"#)
            .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(conversation.last_event_sequence(), Some(42));
}

#[test]
fn restored_conversation_uses_persisted_remote_child_marker() {
    let conversation_data: AgentConversationData =
        serde_json::from_str(r#"{"server_conversation_token":null,"is_remote_child":true}"#)
            .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert!(conversation.is_remote_child());
}

#[test]
fn child_conversation_detection_uses_parent_agent_id() {
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"parent_agent_id":"parent-run-id"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert!(conversation.is_child_agent_conversation());
    assert_eq!(conversation.parent_conversation_id(), None);
}

#[test]
fn cli_agent_transcript_vehicle_is_excluded_from_navigation() {
    let conversation = AIConversation::new(false, true);

    assert!(conversation.should_exclude_from_navigation());
}

#[test]
fn restored_conversation_defaults_unknown_persisted_autoexecute_override() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"UnexpectedValue"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn restored_conversation_uses_persisted_autoexecute_override_when_enabled() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(true);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"RunToCompletion"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RunToCompletion
    );
}

#[test]
fn restored_conversation_ignores_persisted_autoexecute_override_when_disabled() {
    let _flag = FeatureFlag::RememberFastForwardState.override_enabled(false);
    let conversation_data: AgentConversationData = serde_json::from_str(
        r#"{"server_conversation_token":null,"autoexecute_override":"RunToCompletion"}"#,
    )
    .unwrap();

    let conversation = restored_conversation(Some(conversation_data));

    assert_eq!(
        conversation.autoexecute_override(),
        AIConversationAutoexecuteMode::RespectUserSettings
    );
}

#[test]
fn fork_artifacts_adds_file_artifacts_to_conversation() {
    let proto_artifact = api::message::artifact_event::ConversationArtifact {
        artifact: Some(
            api::message::artifact_event::conversation_artifact::Artifact::File(
                api::message::artifact_event::FileArtifact {
                    artifact_uid: "artifact-file-1".to_string(),
                    filepath: "outputs/report.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 42,
                    description: "Daily summary".to_string(),
                },
            ),
        ),
    };

    assert_eq!(
        artifact_from_fork_proto(&proto_artifact),
        Some(Artifact::File {
            artifact_uid: "artifact-file-1".to_string(),
            filepath: "outputs/report.txt".to_string(),
            filename: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: Some(42),
        })
    );
}
