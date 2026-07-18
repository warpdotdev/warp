use std::path::PathBuf;
use std::{env, fs};

use chrono::Utc;
use tempfile::tempdir;
use warp_cli::artifact::UploadArtifactArgs;

use super::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::cloud_object::{Revision, ServerMetadata, ServerPermissions};
use crate::persistence::model::ConversationUsageMetadata;
use crate::server::ids::ServerId;

fn create_mock_server_metadata() -> ServerMetadata {
    ServerMetadata {
        uid: ServerId::default(),
        revision: Revision::now(),
        metadata_last_updated_ts: Utc::now().into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    }
}

fn create_conversation_metadata(
    conversation_id: &str,
    ambient_task_id: Option<&str>,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Artifact upload".to_string(),
        working_directory: None,
        harness: AIAgentHarness::Oz,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
            context_window_segments: Vec::new(),
        },
        metadata: create_mock_server_metadata(),
        creator: None,
        permissions: ServerPermissions::mock_personal(),
        ambient_agent_task_id: ambient_task_id.map(|task_id| task_id.parse().unwrap()),
        server_conversation_token: ServerConversationToken::new(conversation_id.to_string()),
        artifacts: Vec::new(),
    }
}

#[test]
fn normalize_artifact_filepath_preserves_shape_and_normalizes_separators() {
    let path = PathBuf::from(r"outputs\reports/final.txt");
    assert_eq!(
        normalize_artifact_filepath(&path),
        "outputs/reports/final.txt"
    );
}

#[test]
fn checked_graphql_size_bytes_for_upload_returns_none_for_overflow() {
    let path = PathBuf::from("outputs/large-artifact.bin");

    assert_eq!(
        checked_graphql_size_bytes_for_upload(&path, i32::MAX as u64),
        Some(i32::MAX)
    );
    assert_eq!(
        checked_graphql_size_bytes_for_upload(&path, i32::MAX as u64 + 1),
        None
    );
}

#[test]
fn file_size_and_prefix_for_path_returns_truncated_prefix() {
    let tempdir = tempdir().unwrap();
    let path = tempdir.path().join("artifact.bin");
    fs::write(&path, b"0123456789").unwrap();

    assert_eq!(
        file_size_and_prefix_for_path(&path, 4).unwrap(),
        (10, b"0123".to_vec())
    );
}

#[test]
fn file_size_and_prefix_for_path_returns_full_contents_when_prefix_exceeds_file() {
    let tempdir = tempdir().unwrap();
    let path = tempdir.path().join("artifact.bin");
    fs::write(&path, b"0123456789").unwrap();

    assert_eq!(
        file_size_and_prefix_for_path(&path, 32).unwrap(),
        (10, b"0123456789".to_vec())
    );
}

#[test]
fn single_conversation_metadata_returns_the_only_metadata_record() {
    let metadata = single_conversation_metadata(
        "conversation-123",
        vec![create_conversation_metadata(
            "conversation-123",
            Some("550e8400-e29b-41d4-a716-446655440000"),
        )],
    )
    .unwrap();

    let task_id = ambient_task_id_from_conversation_metadata("conversation-123", metadata).unwrap();
    assert_eq!(
        task_id,
        "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
    );
}

#[test]
fn single_conversation_metadata_errors_when_no_metadata_is_returned() {
    let err = single_conversation_metadata("conversation-123", Vec::new()).unwrap_err();

    assert!(err.to_string().contains("Conversation not found"));
}

#[test]
fn ambient_task_id_from_conversation_metadata_requires_cloud_task_metadata() {
    let err = ambient_task_id_from_conversation_metadata(
        "conversation-123",
        create_conversation_metadata("conversation-123", None),
    )
    .unwrap_err();

    assert!(err
        .to_string()
        .contains("Conversation 'conversation-123' is not backed by a cloud agent task"));
}

#[test]
fn explicit_run_id_wins_over_env_fallback() {
    let resolved = resolve_upload_association_from_sources(
        Some("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()),
        None,
        None,
        Some("11111111-1111-1111-1111-111111111111".to_string()),
    )
    .unwrap();

    assert_eq!(
        resolved,
        ResolvedUploadAssociation {
            conversation_id: None,
            run_id: Some("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()),
            ambient_task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        }
    );
}

#[test]
fn invalid_explicit_run_id_errors_even_if_env_fallback_exists() {
    let err = FileArtifactUploadRequest::try_from(UploadArtifactArgs {
        path: PathBuf::from("outputs/report.txt"),
        run_id: Some("not-a-run-id".to_string()),
        conversation_id: None,
        description: None,
    })
    .unwrap_err();

    assert!(err.to_string().contains("Invalid run ID 'not-a-run-id'"));
}

#[test]
fn valid_conversation_resolution_ignores_env_fallback() {
    let resolved = resolve_upload_association_from_sources(
        None,
        Some(ServerConversationToken::new("conversation-123".to_string())),
        Some(Ok("550e8400-e29b-41d4-a716-446655440000".parse().unwrap())),
        Some("11111111-1111-1111-1111-111111111111".to_string()),
    )
    .unwrap();

    assert_eq!(
        resolved,
        ResolvedUploadAssociation {
            conversation_id: Some(ServerConversationToken::new("conversation-123".to_string())),
            run_id: None,
            ambient_task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        }
    );
}

#[test]
fn failed_conversation_resolution_falls_back_to_env_run_id() {
    let resolved = resolve_upload_association_from_sources(
        None,
        Some(ServerConversationToken::new("conversation-123".to_string())),
        Some(Err(anyhow!(
            "Conversation 'conversation-123' is not backed by a cloud agent task"
        ))),
        Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
    )
    .unwrap();

    assert_eq!(
        resolved,
        ResolvedUploadAssociation {
            conversation_id: Some(ServerConversationToken::new("conversation-123".to_string())),
            run_id: None,
            ambient_task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        }
    );
}

#[test]
fn missing_args_fall_back_to_env_run_id_for_request_association() {
    let resolved = resolve_upload_association_from_sources(
        None,
        None,
        None,
        Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
    )
    .unwrap();

    assert_eq!(
        resolved,
        ResolvedUploadAssociation {
            conversation_id: None,
            run_id: Some("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()),
            ambient_task_id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
        }
    );
}

#[test]
fn missing_args_and_missing_env_return_clear_error() {
    let err = resolve_upload_association_from_sources(None, None, None, None).unwrap_err();

    assert!(err
        .to_string()
        .contains("no usable --run-id or --conversation-id was provided"));
    assert!(err.to_string().contains("OZ_RUN_ID"));
}

#[test]
fn invalid_env_run_id_returns_clear_error() {
    let err =
        resolve_upload_association_from_sources(None, None, None, Some("not-a-run-id".to_string()))
            .unwrap_err();

    assert!(err.to_string().contains("Invalid OZ_RUN_ID 'not-a-run-id'"));
}

#[test]
fn load_env_run_id_reads_variable() {
    let previous = env::var_os(OZ_RUN_ID_ENV_VAR);
    env::set_var(OZ_RUN_ID_ENV_VAR, "550e8400-e29b-41d4-a716-446655440000");

    let loaded = load_env_run_id().unwrap();

    match previous {
        Some(value) => env::set_var(OZ_RUN_ID_ENV_VAR, value),
        None => env::remove_var(OZ_RUN_ID_ENV_VAR),
    }

    assert_eq!(
        loaded.as_deref(),
        Some("550e8400-e29b-41d4-a716-446655440000")
    );
}
