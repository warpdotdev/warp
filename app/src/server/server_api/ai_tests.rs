use chrono::TimeZone;
use chrono::Utc;
use futures::executor::block_on;

use super::super::auth::CLOUD_AGENT_ID_HEADER;
use super::super::ServerApi;
use super::{
    build_fork_conversation_url, build_list_agent_runs_url, build_run_followup_url,
    AgentMessageHeader, AgentRunEvent, AgentSource, AmbientAgentTaskState, Artifact,
    ArtifactDownloadResponse, ArtifactType, ExecutionLocation, ForkConversationResponse,
    ListRunsResponse, ReadAgentMessageResponse, RunFollowupRequest, RunSortBy, RunSortOrder,
    SpawnAgentRequest, TaskListFilter, UserQueryMode,
};
use crate::notebooks::NotebookId;

#[test]
fn ambient_agent_headers_for_task_overrides_existing_cloud_agent_header() {
    let server_api = ServerApi::new_for_test();
    let ambient_task_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let task_scoped_id = "123e4567-e89b-12d3-a456-426614174000".parse().unwrap();

    server_api.set_ambient_agent_task_id(Some(ambient_task_id));

    let cloud_agent_headers: Vec<_> =
        block_on(server_api.ambient_agent_headers_for_task(&task_scoped_id))
            .unwrap()
            .into_iter()
            .filter(|(name, _)| *name == CLOUD_AGENT_ID_HEADER)
            .collect();

    assert_eq!(
        cloud_agent_headers,
        vec![(CLOUD_AGENT_ID_HEADER, task_scoped_id.to_string())]
    );
}

#[test]
fn spawn_agent_request_serializes_agent_uid_as_agent_identity_uid() {
    let request = SpawnAgentRequest {
        prompt: "hello".to_string(),
        mode: UserQueryMode::Normal,
        config: None,
        title: None,
        team: None,
        agent_identity_uid: Some("agent_123".to_string()),
        skill: None,
        attachments: vec![],
        interactive: None,
        parent_run_id: None,
        runtime_skills: vec![],
        referenced_attachments: vec![],
        conversation_id: None,
        initial_snapshot_token: None,
        snapshot_disabled: None,
    };

    let value = serde_json::to_value(&request).unwrap();

    assert_eq!(
        value.get("agent_identity_uid").and_then(|v| v.as_str()),
        Some("agent_123")
    );
    assert!(value.get("agent_uid").is_none());
}

#[test]
fn test_deserialize_file_artifact_download_response() {
    let json = r#"{
        "artifact_uid": "artifact-123",
        "artifact_type": "FILE",
        "created_at": "2024-01-15T10:30:00Z",
        "data": {
            "download_url": "https://storage.example.com/report.txt",
            "expires_at": "2024-01-15T11:30:00Z",
            "content_type": "text/plain",
            "filepath": "outputs/report.txt",
            "filename": "report.txt",
            "description": "daily summary",
            "size_bytes": 42
        }
    }"#;

    let artifact: ArtifactDownloadResponse = serde_json::from_str(json).unwrap();

    let ArtifactDownloadResponse::File { common, data } = artifact else {
        panic!("expected File artifact download response");
    };
    assert_eq!(common.artifact_uid, "artifact-123");
    assert_eq!(common.created_at.to_rfc3339(), "2024-01-15T10:30:00+00:00");
    assert_eq!(data.download_url, "https://storage.example.com/report.txt");
    assert_eq!(data.expires_at.to_rfc3339(), "2024-01-15T11:30:00+00:00");
    assert_eq!(data.content_type, "text/plain");
    assert_eq!(data.filepath, "outputs/report.txt");
    assert_eq!(data.filename, "report.txt");
    assert_eq!(data.description.as_deref(), Some("daily summary"));
    assert_eq!(data.size_bytes, Some(42));
}

#[test]
fn test_deserialize_screenshot_artifact_download_response() {
    let json = r#"{
        "artifact_uid": "screenshot-123",
        "artifact_type": "SCREENSHOT",
        "created_at": "2024-01-15T10:30:00Z",
        "data": {
            "download_url": "https://storage.example.com/screenshot.png",
            "expires_at": "2024-01-15T11:30:00Z",
            "content_type": "image/png",
            "description": "dashboard screenshot"
        }
    }"#;

    let artifact: ArtifactDownloadResponse = serde_json::from_str(json).unwrap();

    let ArtifactDownloadResponse::Screenshot { common, data } = artifact else {
        panic!("expected Screenshot artifact download response");
    };
    assert_eq!(common.artifact_uid, "screenshot-123");
    assert_eq!(common.created_at.to_rfc3339(), "2024-01-15T10:30:00+00:00");
    assert_eq!(
        data.download_url,
        "https://storage.example.com/screenshot.png"
    );
    assert_eq!(data.expires_at.to_rfc3339(), "2024-01-15T11:30:00+00:00");
    assert_eq!(data.content_type, "image/png");
    assert_eq!(data.description.as_deref(), Some("dashboard screenshot"));
}

#[test]
fn test_deserialize_plan_artifact() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PLAN",
        "data": {
            "document_uid": "doc-uid-123",
            "notebook_uid": "1234567890123456789012",
            "title": "My Plan"
        }
    }"#;

    let artifact: Artifact = serde_json::from_str(json).unwrap();

    let Artifact::Plan {
        document_uid,
        notebook_uid,
        title,
    } = &artifact
    else {
        panic!("expected Plan artifact");
    };
    assert_eq!(document_uid, "doc-uid-123");
    assert_eq!(
        notebook_uid.as_ref().map(|n| n.to_string()),
        Some("1234567890123456789012".to_string())
    );
    assert_eq!(*title, Some("My Plan".to_string()));
}

#[test]
fn test_deserialize_pull_request_artifact() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PULL_REQUEST",
        "data": {
            "url": "https://github.com/org/repo/pull/42",
            "branch": "feature-branch"
        }
    }"#;

    let artifact: Artifact = serde_json::from_str(json).unwrap();

    let Artifact::PullRequest {
        url,
        branch,
        repo,
        number,
    } = &artifact
    else {
        panic!("expected PullRequest artifact");
    };
    assert_eq!(url, "https://github.com/org/repo/pull/42");
    assert_eq!(branch, "feature-branch");
    assert_eq!(*repo, Some("repo".to_string()));
    assert_eq!(*number, Some(42));
}

#[test]
fn test_deserialize_pull_request_non_github_url() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PULL_REQUEST",
        "data": {
            "url": "https://gitlab.com/org/repo/merge_requests/42",
            "branch": "feature-branch"
        }
    }"#;

    let artifact: Artifact = serde_json::from_str(json).unwrap();

    let Artifact::PullRequest { repo, number, .. } = &artifact else {
        panic!("expected PullRequest artifact");
    };
    assert_eq!(*repo, None);
    assert_eq!(*number, None);
}

#[test]
fn test_deserialize_plan_artifact_with_optional_fields_missing() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PLAN",
        "data": {
            "document_uid": "doc-uid-123",
            "notebook_uid": "abcdefghijklmnopqrstuv"
        }
    }"#;

    let artifact: Artifact = serde_json::from_str(json).unwrap();

    let Artifact::Plan {
        document_uid,
        notebook_uid,
        title,
    } = &artifact
    else {
        panic!("expected Plan artifact");
    };
    assert_eq!(document_uid, "doc-uid-123");
    assert_eq!(
        notebook_uid.as_ref().map(|n| n.to_string()),
        Some("abcdefghijklmnopqrstuv".to_string())
    );
    assert!(title.is_none());
}

#[test]
fn test_deserialize_list_tasks_response_with_artifacts() {
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Test Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true,
                "artifacts": [
                    {
                        "created_at": "2024-01-15T10:20:00Z",
                        "artifact_type": "PLAN",
                        "data": {
                            "document_uid": "doc-1",
                            "notebook_uid": "xyz1234567890123456789",
                            "title": "Plan Title"
                        }
                    },
                    {
                        "created_at": "2024-01-15T10:25:00Z",
                        "artifact_type": "PULL_REQUEST",
                        "data": {
                            "url": "https://github.com/org/repo/pull/1",
                            "branch": "main"
                        }
                    },
                    {
                        "created_at": "2024-01-15T10:27:00Z",
                        "artifact_type": "FILE",
                        "data": {
                            "artifact_uid": "artifact-file-1",
                            "filepath": "outputs/report.txt",
                            "filename": "report.txt",
                            "mime_type": "text/plain",
                            "description": "Daily summary",
                            "size_bytes": 42
                        }
                    }
                ]
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.runs.len(), 1);
    let task = &response.runs[0];
    assert_eq!(
        task.task_id.to_string(),
        "550e8400-e29b-41d4-a716-446655440000"
    );
    assert_eq!(task.artifacts.len(), 3);

    // Check first artifact (Plan)
    let Artifact::Plan {
        document_uid,
        title,
        ..
    } = &task.artifacts[0]
    else {
        panic!("expected Plan artifact");
    };
    assert_eq!(document_uid, "doc-1");
    assert_eq!(*title, Some("Plan Title".to_string()));

    // Check second artifact (PullRequest)
    let Artifact::PullRequest {
        url,
        branch,
        repo,
        number,
        ..
    } = &task.artifacts[1]
    else {
        panic!("expected PullRequest artifact");
    };
    assert_eq!(url, "https://github.com/org/repo/pull/1");
    assert_eq!(branch, "main");
    assert_eq!(*repo, Some("repo".to_string()));
    assert_eq!(*number, Some(1));

    let Artifact::File {
        artifact_uid,
        filepath,
        filename,
        mime_type,
        description,
        size_bytes,
    } = &task.artifacts[2]
    else {
        panic!("expected File artifact");
    };
    assert_eq!(artifact_uid, "artifact-file-1");
    assert_eq!(filepath, "outputs/report.txt");
    assert_eq!(filename, "report.txt");
    assert_eq!(mime_type, "text/plain");
    assert_eq!(*description, Some("Daily summary".to_string()));
    assert_eq!(*size_bytes, Some(42));
}

#[test]
fn test_deserialize_list_tasks_response_empty_artifacts() {
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "title": "Test Task",
                "state": "INPROGRESS",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true,
                "artifacts": []
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.runs.len(), 1);
    assert!(response.runs[0].artifacts.is_empty());
}

#[test]
fn test_deserialize_list_tasks_response_missing_artifacts_field() {
    // Server may not include artifacts field at all for older responses
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440002",
                "title": "Test Task",
                "state": "QUEUED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.runs.len(), 1);
    assert!(response.runs[0].artifacts.is_empty());
}

#[test]
fn test_deserialize_artifacts_skips_invalid_items() {
    // deserialize_artifacts should skip invalid items and keep valid ones
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Test Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true,
                "artifacts": [
                    {
                        "created_at": "2024-01-15T10:20:00Z",
                        "artifact_type": "PLAN",
                        "data": {
                            "document_uid": "valid-doc",
                            "notebook_uid": "validnotebook123456789",
                            "title": "Valid Plan"
                        }
                    },
                    {
                        "created_at": "2024-01-15T10:25:00Z",
                        "artifact_type": "UNKNOWN_TYPE",
                        "data": {
                            "some_field": "value"
                        }
                    },
                    {
                        "created_at": "2024-01-15T10:30:00Z",
                        "artifact_type": "PULL_REQUEST",
                        "data": {
                            "url": "https://github.com/org/repo/pull/1",
                            "branch": "main"
                        }
                    }
                ]
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.runs.len(), 1);
    // Invalid artifact skipped, valid ones kept
    assert_eq!(response.runs[0].artifacts.len(), 2);
    assert!(matches!(
        response.runs[0].artifacts[0],
        Artifact::Plan { .. }
    ));
    assert!(matches!(
        response.runs[0].artifacts[1],
        Artifact::PullRequest { .. }
    ));
}

#[test]
fn test_deserialize_artifacts_all_invalid_returns_empty() {
    // When all artifacts are invalid, result should be empty vec
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Test Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true,
                "artifacts": [
                    {
                        "created_at": "2024-01-15T10:20:00Z",
                        "artifact_type": "UNKNOWN_TYPE",
                        "data": {}
                    }
                ]
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.runs.len(), 1);
    assert!(response.runs[0].artifacts.is_empty());
}

#[test]
fn test_deserialize_artifact_missing_data_field() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PLAN"
    }"#;

    let result = serde_json::from_str::<Artifact>(json);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("missing field"));
}

#[test]
fn test_deserialize_artifact_invalid_plan_data() {
    // Missing required `document_uid` field should fail deserialization
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PLAN",
        "data": {
            "title": "Only title, no document_uid"
        }
    }"#;

    let result = serde_json::from_str::<Artifact>(json);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("missing field"));
}

#[test]
fn test_deserialize_artifact_invalid_pr_data() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "PULL_REQUEST",
        "data": {
            "url": "https://github.com/org/repo/pull/1"
        }
    }"#;

    let result = serde_json::from_str::<Artifact>(json);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("missing field"));
}

#[test]
fn test_deserialize_artifact_unknown_variant() {
    let json = r#"{
        "created_at": "2024-01-15T10:30:00Z",
        "artifact_type": "UNKNOWN_TYPE",
        "data": {
            "some_field": "value"
        }
    }"#;

    let result = serde_json::from_str::<Artifact>(json);
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("unknown variant"));
}

// ---------------------------------------------------------------------------------------------------------------------
//  Tests for resilient task list deserialization (skipping malformed tasks while tolerating unknown states)
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn test_deserialize_list_tasks_skips_invalid_task() {
    // One valid task and one invalid task (missing required field)
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Valid Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "title": "Invalid Task",
                "state": "INPROGRESS"
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    // Should only have the valid task
    assert_eq!(response.runs.len(), 1);
    assert_eq!(
        response.runs[0].task_id.to_string(),
        "550e8400-e29b-41d4-a716-446655440000"
    );
    assert_eq!(response.runs[0].title, "Valid Task");
}

#[test]
fn test_deserialize_list_tasks_error_and_blocked_states() {
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Errored Task",
                "state": "ERROR",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": false
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "title": "Blocked Task",
                "state": "BLOCKED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": false
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.runs.len(), 2);
    assert_eq!(response.runs[0].state, AmbientAgentTaskState::Error);
    assert_eq!(response.runs[1].state, AmbientAgentTaskState::Blocked);
}

#[test]
fn test_deserialize_list_tasks_all_tasks_invalid_returns_empty() {
    // All tasks are missing required fields
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Missing State"
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "state": "SUCCEEDED"
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    // Should return empty list, not fail
    assert_eq!(response.runs.len(), 0);
}

#[test]
fn test_deserialize_list_tasks_invalid_state_enum() {
    // Task with an unknown state enum value
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Valid Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "title": "Task with Invalid State",
                "state": "INVALID_STATE",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    // Unknown states should deserialize to AmbientAgentTaskState::Unknown.
    assert_eq!(response.runs.len(), 2);
    assert_eq!(response.runs[0].title, "Valid Task");
    assert_eq!(response.runs[1].title, "Task with Invalid State");
    assert_eq!(response.runs[1].state, AmbientAgentTaskState::Unknown);
}

#[test]
fn test_deserialize_list_tasks_corrupted_json_in_middle() {
    // Mix of valid and completely malformed JSON
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "First Valid Task",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            },
            {
                "task_id": 12345,
                "title": 999,
                "state": true
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440002",
                "title": "Second Valid Task",
                "state": "INPROGRESS",
                "prompt": "test prompt 2",
                "created_at": "2024-01-15T11:00:00Z",
                "updated_at": "2024-01-15T11:30:00Z",
                "is_sandbox_running": false
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    // Should have both valid tasks, malformed one skipped
    assert_eq!(response.runs.len(), 2);
    assert_eq!(response.runs[0].title, "First Valid Task");
    assert_eq!(response.runs[1].title, "Second Valid Task");
}

#[test]
fn test_deserialize_list_tasks_empty_tasks_array() {
    // Empty tasks array should work fine
    let json = r#"{
        "runs": []
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.runs.len(), 0);
}

#[test]
fn test_deserialize_list_tasks_all_tasks_valid() {
    // Ensure we don't break the happy path
    let json = r#"{
        "runs": [
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440000",
                "title": "Task 1",
                "state": "SUCCEEDED",
                "prompt": "test prompt",
                "created_at": "2024-01-15T10:00:00Z",
                "updated_at": "2024-01-15T10:30:00Z",
                "is_sandbox_running": true
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440001",
                "title": "Task 2",
                "state": "INPROGRESS",
                "prompt": "test prompt 2",
                "created_at": "2024-01-15T11:00:00Z",
                "updated_at": "2024-01-15T11:30:00Z",
                "is_sandbox_running": false
            },
            {
                "task_id": "550e8400-e29b-41d4-a716-446655440002",
                "title": "Task 3",
                "state": "FAILED",
                "prompt": "test prompt 3",
                "created_at": "2024-01-15T12:00:00Z",
                "updated_at": "2024-01-15T12:30:00Z",
                "is_sandbox_running": false
            }
        ]
    }"#;

    let response: ListRunsResponse = serde_json::from_str(json).unwrap();

    // All tasks should be present
    assert_eq!(response.runs.len(), 3);
    assert_eq!(response.runs[0].title, "Task 1");
    assert_eq!(response.runs[1].title, "Task 2");
    assert_eq!(response.runs[2].title, "Task 3");
}

// ---------------------------------------------------------------------------------------------------------------------
//  We test roundtripping serialize and deserialize since we use this for persisting artifacts for local conversations.
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn test_artifact_plan_serialize_deserialize_roundtrip() {
    let original = Artifact::Plan {
        document_uid: "doc-123".to_string(),
        notebook_uid: Some(NotebookId::from("notebook12345678901234".to_string())),
        title: Some("My Plan".to_string()),
    };

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: Artifact = serde_json::from_str(&serialized).unwrap();

    assert_eq!(original, deserialized);
}

#[test]
fn test_deserialize_agent_message_headers() {
    let json = r#"[
        {
            "message_id": "message-1",
            "sender_run_id": "run-1",
            "subject": "Build finished",
            "sent_at": "2026-04-09T20:00:00Z",
            "delivered_at": "2026-04-09T20:01:00Z",
            "read_at": null
        }
    ]"#;

    let headers: Vec<AgentMessageHeader> = serde_json::from_str(json).unwrap();

    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].message_id, "message-1");
    assert_eq!(headers[0].sender_run_id, "run-1");
    assert_eq!(headers[0].subject, "Build finished");
    assert_eq!(headers[0].sent_at, "2026-04-09T20:00:00Z");
    assert_eq!(
        headers[0].delivered_at.as_deref(),
        Some("2026-04-09T20:01:00Z")
    );
    assert_eq!(headers[0].read_at, None);
}

#[test]
fn test_deserialize_read_agent_message_response_with_timestamps() {
    let json = r#"{
        "message_id": "message-1",
        "sender_run_id": "run-1",
        "subject": "Build finished",
        "body": "Everything passed.",
        "sent_at": "2026-04-09T20:00:00Z",
        "delivered_at": "2026-04-09T20:01:00Z",
        "read_at": "2026-04-09T20:02:00Z"
    }"#;

    let response: ReadAgentMessageResponse = serde_json::from_str(json).unwrap();

    assert_eq!(response.message_id, "message-1");
    assert_eq!(response.sender_run_id, "run-1");
    assert_eq!(response.subject, "Build finished");
    assert_eq!(response.body, "Everything passed.");
    assert_eq!(response.sent_at, "2026-04-09T20:00:00Z");
    assert_eq!(
        response.delivered_at.as_deref(),
        Some("2026-04-09T20:01:00Z")
    );
    assert_eq!(response.read_at.as_deref(), Some("2026-04-09T20:02:00Z"));
}

#[test]
fn test_deserialize_agent_run_events_with_optional_fields() {
    let json = r#"[
        {
            "event_type": "run_started",
            "run_id": "run-1",
            "ref_id": null,
            "execution_id": "exec-1",
            "occurred_at": "2026-04-09T20:00:00Z",
            "sequence": 7
        },
        {
            "event_type": "new_message",
            "run_id": "run-2",
            "ref_id": "message-9",
            "execution_id": null,
            "occurred_at": "2026-04-09T20:05:00Z",
            "sequence": 8
        }
    ]"#;

    let events: Vec<AgentRunEvent> = serde_json::from_str(json).unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "run_started");
    assert_eq!(events[0].execution_id.as_deref(), Some("exec-1"));
    assert_eq!(events[0].ref_id, None);
    assert_eq!(events[0].sequence, 7);
    assert_eq!(events[1].event_type, "new_message");
    assert_eq!(events[1].ref_id.as_deref(), Some("message-9"));
    assert_eq!(events[1].execution_id, None);
    assert_eq!(events[1].sequence, 8);
}

#[test]
fn test_artifact_plan_serialize_deserialize_roundtrip_no_notebook_uid() {
    let original = Artifact::Plan {
        document_uid: "doc-123".to_string(),
        notebook_uid: None,
        title: Some("My Plan".to_string()),
    };

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: Artifact = serde_json::from_str(&serialized).unwrap();

    assert_eq!(original, deserialized);
}

#[test]
fn test_artifact_pr_serialize_deserialize_roundtrip() {
    let original = Artifact::PullRequest {
        url: "https://github.com/org/repo/pull/42".to_string(),
        branch: "feature-branch".to_string(),
        repo: Some("repo".to_string()),
        number: Some(42),
    };

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: Artifact = serde_json::from_str(&serialized).unwrap();

    // repo/number are re-derived from URL on deserialize, so should match
    assert_eq!(original, deserialized);
}

#[test]
fn test_artifact_file_serialize_deserialize_roundtrip() {
    let original = Artifact::File {
        artifact_uid: "artifact-file-1".to_string(),
        filepath: "outputs/report.txt".to_string(),
        filename: "report.txt".to_string(),
        mime_type: "text/plain".to_string(),
        description: Some("Daily summary".to_string()),
        size_bytes: Some(42),
    };

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: Artifact = serde_json::from_str(&serialized).unwrap();

    assert_eq!(original, deserialized);
}

#[test]
fn test_artifact_vec_serialize_deserialize_roundtrip() {
    let original = vec![
        Artifact::Plan {
            document_uid: "doc-1".to_string(),
            notebook_uid: None,
            title: Some("Plan 1".to_string()),
        },
        Artifact::PullRequest {
            url: "https://github.com/org/repo/pull/1".to_string(),
            branch: "main".to_string(),
            repo: Some("repo".to_string()),
            number: Some(1),
        },
        Artifact::File {
            artifact_uid: "artifact-file-1".to_string(),
            filepath: "outputs/report.txt".to_string(),
            filename: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            description: Some("Daily summary".to_string()),
            size_bytes: Some(42),
        },
    ];

    let serialized = serde_json::to_string(&original).unwrap();
    let deserialized: Vec<Artifact> = serde_json::from_str(&serialized).unwrap();

    assert_eq!(original, deserialized);
}

#[test]
fn build_list_agent_runs_url_empty_filter() {
    let url = build_list_agent_runs_url(10, &TaskListFilter::default());
    assert_eq!(url, "agent/runs?limit=10");
}

#[test]
fn build_list_agent_runs_url_all_fields() {
    let filter = TaskListFilter {
        creator_uid: Some("user-uid".to_string()),
        updated_after: Some(Utc.with_ymd_and_hms(2026, 4, 3, 12, 30, 0).unwrap()),
        created_after: Some(Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap()),
        created_before: Some(Utc.with_ymd_and_hms(2026, 4, 2, 0, 0, 0).unwrap()),
        states: Some(vec![
            AmbientAgentTaskState::Failed,
            AmbientAgentTaskState::Error,
        ]),
        source: Some(AgentSource::AgentWebhook),
        execution_location: Some(ExecutionLocation::Remote),
        environment_id: Some("env-123".to_string()),
        skill_spec: Some("owner/repo:SKILL.md".to_string()),
        schedule_id: Some("sched-1".to_string()),
        ancestor_run_id: Some("run-parent".to_string()),
        config_name: Some("nightly".to_string()),
        model_id: Some("claude-4-5".to_string()),
        artifact_type: Some(ArtifactType::PullRequest),
        search_query: Some("oz run".to_string()),
        sort_by: Some(RunSortBy::CreatedAt),
        sort_order: Some(RunSortOrder::Asc),
        cursor: Some("abcd==".to_string()),
    };

    let url = build_list_agent_runs_url(42, &filter);
    assert_eq!(
        url,
        "agent/runs?limit=42\
         &creator=user-uid\
         &updated_after=2026-04-03T12%3A30%3A00%2B00%3A00\
         &created_after=2026-04-01T00%3A00%3A00%2B00%3A00\
         &created_before=2026-04-02T00%3A00%3A00%2B00%3A00\
         &state=FAILED\
         &state=ERROR\
         &source=API\
         &execution_location=REMOTE\
         &environment_id=env-123\
         &skill_spec=owner%2Frepo%3ASKILL.md\
         &schedule_id=sched-1\
         &ancestor_run_id=run-parent\
         &name=nightly\
         &model_id=claude-4-5\
         &artifact_type=PULL_REQUEST\
         &q=oz%20run\
         &sort_by=created_at\
         &sort_order=asc\
         &cursor=abcd%3D%3D"
    );
}

#[test]
fn build_list_agent_runs_url_repeats_state_filter() {
    let filter = TaskListFilter {
        states: Some(vec![
            AmbientAgentTaskState::Queued,
            AmbientAgentTaskState::InProgress,
            AmbientAgentTaskState::Succeeded,
        ]),
        ..TaskListFilter::default()
    };
    let url = build_list_agent_runs_url(5, &filter);
    assert_eq!(
        url,
        "agent/runs?limit=5&state=QUEUED&state=INPROGRESS&state=SUCCEEDED"
    );
}

#[test]
fn build_list_agent_runs_url_skips_unknown_state() {
    // The deserializer keeps `Unknown` for forward compatibility, but we shouldn't send it to
    // the server as a filter value.
    let filter = TaskListFilter {
        states: Some(vec![
            AmbientAgentTaskState::Unknown,
            AmbientAgentTaskState::Succeeded,
        ]),
        ..TaskListFilter::default()
    };
    let url = build_list_agent_runs_url(1, &filter);
    assert_eq!(url, "agent/runs?limit=1&state=SUCCEEDED");
}

#[test]
fn build_list_agent_runs_url_routes_to_runs_not_tasks() {
    let url = build_list_agent_runs_url(10, &TaskListFilter::default());
    assert!(url.starts_with("agent/runs?"));
    assert!(!url.starts_with("agent/tasks"));
}

#[test]
fn build_run_followup_url_routes_to_run_followups() {
    let run_id = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    assert_eq!(
        build_run_followup_url(&run_id),
        "agent/runs/550e8400-e29b-41d4-a716-446655440000/followups"
    );
}

#[test]
fn serialize_run_followup_request() {
    let request = RunFollowupRequest {
        message: "continue from here".to_string(),
    };

    let json = serde_json::to_value(request).unwrap();

    assert_eq!(
        json,
        serde_json::json!({
            "message": "continue from here",
        })
    );
}

#[test]
fn build_fork_conversation_url_routes_to_conversation_fork() {
    assert_eq!(
        build_fork_conversation_url("550e8400-e29b-41d4-a716-446655440000"),
        "agent/conversations/550e8400-e29b-41d4-a716-446655440000/fork"
    );
}

#[test]
fn build_fork_conversation_url_escapes_path_param() {
    assert_eq!(
        build_fork_conversation_url("conversation/with spaces"),
        "agent/conversations/conversation%2Fwith%20spaces/fork"
    );
}

#[test]
fn deserialize_fork_conversation_response() {
    let response: ForkConversationResponse = serde_json::from_value(serde_json::json!({
        "forked_conversation_id": "abcdef01-2345-6789-abcd-ef0123456789",
    }))
    .unwrap();
    assert_eq!(
        response.forked_conversation_id,
        "abcdef01-2345-6789-abcd-ef0123456789"
    );
}
