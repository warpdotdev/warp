//! Unit tests for ambient agent CLI argument mapping and message helpers.
use bytes::Bytes;
use chrono::{TimeZone, Utc};
use warp_cli::SortOrderArg;
use warp_cli::json_filter::JsonOutput;
use warp_cli::task::{
    ArtifactTypeArg, ExecutionLocationArg, ListTasksArgs, RunSortByArg, RunSourceArg, RunStateArg,
};
use warp_server_client::HttpStatusError;

use super::*;
use crate::server::server_api::ai::{
    ArtifactType, ExecutionLocation, MockAIClient, RunSortBy, RunSortOrder,
};

const TASK_ID: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_TASK_ID: &str = "00000000-0000-0000-0000-000000000002";

/// A `ListTasksArgs` whose fields are all at their defaults.
fn empty_args() -> ListTasksArgs {
    ListTasksArgs {
        limit: 10,
        state: vec![],
        source: None,
        execution_location: None,
        creator: None,
        environment: None,
        skill: None,
        schedule: None,
        ancestor_run: None,
        name: None,
        model: None,
        artifact_type: None,
        created_after: None,
        created_before: None,
        updated_after: None,
        query: None,
        sort_by: None,
        sort_order: None,
        cursor: None,
        json_output: JsonOutput::default(),
    }
}

#[test]
fn empty_args_yields_default_filter() {
    let filter = filter_from_args(&empty_args());
    assert!(filter.creator_uid.is_none());
    assert!(filter.updated_after.is_none());
    assert!(filter.created_after.is_none());
    assert!(filter.created_before.is_none());
    assert!(filter.states.is_none());
    assert!(filter.source.is_none());
    assert!(filter.execution_location.is_none());
    assert!(filter.environment_id.is_none());
    assert!(filter.skill_spec.is_none());
    assert!(filter.schedule_id.is_none());
    assert!(filter.ancestor_run_id.is_none());
    assert!(filter.config_name.is_none());
    assert!(filter.model_id.is_none());
    assert!(filter.artifact_type.is_none());
    assert!(filter.search_query.is_none());
    assert!(filter.sort_by.is_none());
    assert!(filter.sort_order.is_none());
    assert!(filter.cursor.is_none());
}

#[test]
fn state_flags_map_to_filter() {
    let args = ListTasksArgs {
        state: vec![
            RunStateArg::Failed,
            RunStateArg::Error,
            RunStateArg::Cancelled,
        ],
        ..empty_args()
    };
    let filter = filter_from_args(&args);
    assert_eq!(
        filter.states.as_deref(),
        Some(
            [
                AmbientAgentTaskState::Failed,
                AmbientAgentTaskState::Error,
                AmbientAgentTaskState::Cancelled,
            ]
            .as_slice()
        )
    );
}

#[test]
fn source_cli_maps_to_cli() {
    let args = ListTasksArgs {
        source: Some(RunSourceArg::Cli),
        ..empty_args()
    };
    let filter = filter_from_args(&args);
    assert_eq!(filter.source, Some(AgentSource::Cli));
    // Sanity-check the wire value: `--source CLI` must send `source=CLI`.
    assert_eq!(filter.source.as_ref().map(AgentSource::as_str), Some("CLI"));
}

#[test]
fn source_interactive_maps_to_local() {
    // The public API uses `LOCAL` as the source value for local interactive
    // tasks. The CLI exposes this as `--source INTERACTIVE` for readability,
    // but the request sent to the server must use `LOCAL`.
    let args = ListTasksArgs {
        source: Some(RunSourceArg::Interactive),
        ..empty_args()
    };
    let filter = filter_from_args(&args);
    assert_eq!(filter.source, Some(AgentSource::Interactive));
    assert_eq!(
        filter.source.as_ref().map(AgentSource::as_str),
        Some("LOCAL")
    );
}

#[test]
fn every_field_maps_through() {
    let created_after = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
    let created_before = Utc.with_ymd_and_hms(2026, 4, 2, 0, 0, 0).unwrap();
    let updated_after = Utc.with_ymd_and_hms(2026, 4, 3, 12, 30, 0).unwrap();

    let args = ListTasksArgs {
        limit: 20,
        state: vec![RunStateArg::InProgress],
        source: Some(RunSourceArg::Api),
        execution_location: Some(ExecutionLocationArg::Remote),
        creator: Some("user-uid".to_string()),
        environment: Some("env-123".to_string()),
        skill: Some("owner/repo:SKILL.md".to_string()),
        schedule: Some("sched-1".to_string()),
        ancestor_run: Some("run-parent".to_string()),
        name: Some("nightly".to_string()),
        model: Some("claude-4-5".to_string()),
        artifact_type: Some(ArtifactTypeArg::PullRequest),
        created_after: Some(created_after),
        created_before: Some(created_before),
        updated_after: Some(updated_after),
        query: Some("oz run".to_string()),
        sort_by: Some(RunSortByArg::CreatedAt),
        sort_order: Some(SortOrderArg::Asc),
        cursor: Some("abcd==".to_string()),
        json_output: JsonOutput::default(),
    };

    let filter = filter_from_args(&args);

    assert_eq!(filter.creator_uid.as_deref(), Some("user-uid"));
    assert_eq!(filter.updated_after, Some(updated_after));
    assert_eq!(filter.created_after, Some(created_after));
    assert_eq!(filter.created_before, Some(created_before));
    assert_eq!(
        filter.states.as_deref(),
        Some([AmbientAgentTaskState::InProgress].as_slice())
    );
    assert_eq!(filter.source, Some(AgentSource::AgentWebhook));
    assert_eq!(filter.execution_location, Some(ExecutionLocation::Remote));
    assert_eq!(filter.environment_id.as_deref(), Some("env-123"));
    assert_eq!(filter.skill_spec.as_deref(), Some("owner/repo:SKILL.md"));
    assert_eq!(filter.schedule_id.as_deref(), Some("sched-1"));
    assert_eq!(filter.ancestor_run_id.as_deref(), Some("run-parent"));
    assert_eq!(filter.config_name.as_deref(), Some("nightly"));
    assert_eq!(filter.model_id.as_deref(), Some("claude-4-5"));
    assert_eq!(filter.artifact_type, Some(ArtifactType::PullRequest));
    assert_eq!(filter.search_query.as_deref(), Some("oz run"));
    assert_eq!(filter.sort_by, Some(RunSortBy::CreatedAt));
    assert_eq!(filter.sort_order, Some(RunSortOrder::Asc));
    assert_eq!(filter.cursor.as_deref(), Some("abcd=="));
}

#[test]
fn task_id_from_run_id_accepts_task_uuid() {
    let task_id = task_id_from_run_id(TASK_ID).expect("valid task id");

    assert_eq!(task_id.to_string(), TASK_ID);
}

#[test]
fn task_id_from_run_id_ignores_non_task_ids() {
    assert!(task_id_from_run_id("local-child-run").is_none());
}

#[test]
#[serial_test::serial]
fn task_id_for_message_send_prefers_sender_run_id() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(warp_cli::OZ_RUN_ID_ENV, OTHER_TASK_ID) };
    let task_id = task_id_for_message_send(TASK_ID)
        .expect("valid task id")
        .expect("task id");
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(warp_cli::OZ_RUN_ID_ENV) };

    assert_eq!(task_id.to_string(), TASK_ID);
}

#[test]
#[serial_test::serial]
fn task_id_for_message_send_falls_back_to_oz_run_id() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(warp_cli::OZ_RUN_ID_ENV, TASK_ID) };
    let task_id = task_id_for_message_send("local-child-run")
        .expect("valid env task id")
        .expect("task id");
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(warp_cli::OZ_RUN_ID_ENV) };

    assert_eq!(task_id.to_string(), TASK_ID);
}

#[test]
#[serial_test::serial]
fn task_id_from_oz_run_id_env_rejects_invalid_value() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(warp_cli::OZ_RUN_ID_ENV, "not-a-task-id") };
    let err = task_id_from_oz_run_id_env().expect_err("invalid task id");
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(warp_cli::OZ_RUN_ID_ENV) };

    assert!(err.to_string().contains("Invalid OZ_RUN_ID"));
}

fn http_error(status: u16, body: &str) -> anyhow::Error {
    anyhow::Error::new(HttpStatusError {
        status,
        body: body.to_string(),
    })
    .context(format!("API request failed with status {status}"))
}

fn operation_not_supported_error() -> anyhow::Error {
    http_error(
        422,
        r#"{"type":"https://docs.warp.dev/errors/operation_not_supported","title":"normalized conversations are only supported for Warp-native transcripts"}"#,
    )
}

fn native_conversation() -> serde_json::Value {
    serde_json::json!({
        "conversation_id": "conv-native",
        "steps": []
    })
}

#[test]
fn normalized_conversation_unsupported_matches_rfc7807_type() {
    assert!(is_normalized_conversation_unsupported(
        &operation_not_supported_error()
    ));
}

#[test]
fn normalized_conversation_unsupported_matches_type_regardless_of_title() {
    assert!(is_normalized_conversation_unsupported(&http_error(
        422,
        r#"{"type":"https://docs.warp.dev/errors/operation_not_supported","title":"forking is not supported"}"#,
    )));
}

#[test]
fn normalized_conversation_unsupported_rejects_title_only_rfc7807() {
    assert!(!is_normalized_conversation_unsupported(&http_error(
        422,
        r#"{"title":"normalized conversations are only supported for Warp-native transcripts"}"#,
    )));
}

#[test]
fn normalized_conversation_unsupported_rejects_arbitrary_substrings() {
    assert!(!is_normalized_conversation_unsupported(&http_error(
        422,
        "https://docs.warp.dev/errors/operation_not_supported normalized conversations are only supported for Warp-native transcripts",
    )));
    assert!(!is_normalized_conversation_unsupported(&http_error(
        422,
        r#"{"error":"validation failed"}"#,
    )));
    assert!(!is_normalized_conversation_unsupported(&anyhow::anyhow!(
        "operation_not_supported"
    )));
}

#[test]
fn normalized_conversation_unsupported_rejects_wrong_problem_type() {
    assert!(!is_normalized_conversation_unsupported(&http_error(
        422,
        r#"{"type":"https://docs.warp.dev/errors/invalid_request","title":"normalized conversations are only supported for Warp-native transcripts"}"#,
    )));
}

#[test]
fn normalized_conversation_unsupported_rejects_non_422_status() {
    assert!(!is_normalized_conversation_unsupported(&http_error(
        404,
        r#"{"type":"https://docs.warp.dev/errors/operation_not_supported","title":"normalized conversations are only supported for Warp-native transcripts"}"#,
    )));
}

#[test]
fn write_conversation_cli_output_pretty_prints_normalized_json() {
    let mut buf = Vec::new();
    write_conversation_cli_output(
        &ConversationCliOutput::Normalized(serde_json::json!({"conversation_id": "c-1"})),
        &mut buf,
    )
    .unwrap();

    assert_eq!(buf, b"{\n  \"conversation_id\": \"c-1\"\n}\n");
}

#[test]
fn write_conversation_cli_output_writes_raw_transcript_bytes_exactly() {
    let mut without_newline = Vec::new();
    write_conversation_cli_output(
        &ConversationCliOutput::RawTranscript(Bytes::from_static(b"no trailing newline")),
        &mut without_newline,
    )
    .unwrap();
    assert_eq!(without_newline, b"no trailing newline");

    let mut non_utf8 = Vec::new();
    write_conversation_cli_output(
        &ConversationCliOutput::RawTranscript(Bytes::from_static(&[0xff, 0xfe, 0x00, b'x'])),
        &mut non_utf8,
    )
    .unwrap();
    assert_eq!(non_utf8, vec![0xff, 0xfe, 0x00, b'x']);
}

#[tokio::test]
async fn load_run_conversation_returns_normalized_json_for_native_runs() {
    let conversation = native_conversation();
    let mut mock = MockAIClient::new();
    mock.expect_get_run_conversation().times(1).returning({
        let conversation = conversation.clone();
        move |run_id| {
            assert_eq!(run_id, TASK_ID);
            Ok(conversation.clone())
        }
    });
    mock.expect_download_run_transcript().times(0);

    let output = load_run_conversation(&mock, TASK_ID).await.unwrap();

    assert_eq!(output, ConversationCliOutput::Normalized(conversation));
}

#[tokio::test]
async fn load_run_conversation_falls_back_to_raw_transcript_for_third_party_harness() {
    const RAW_TRANSCRIPT: &[u8] = b"{\"type\":\"claude_code\"}\n";
    let mut mock = MockAIClient::new();
    mock.expect_get_run_conversation()
        .times(1)
        .returning(|_| Err(operation_not_supported_error()));
    mock.expect_download_run_transcript()
        .times(1)
        .returning(|run_id| {
            assert_eq!(run_id.to_string(), TASK_ID);
            Ok(Bytes::from_static(RAW_TRANSCRIPT))
        });

    let output = load_run_conversation(&mock, TASK_ID).await.unwrap();

    assert_eq!(
        output,
        ConversationCliOutput::RawTranscript(Bytes::from_static(RAW_TRANSCRIPT))
    );
}

#[tokio::test]
async fn load_run_conversation_preserves_unrelated_422() {
    let mut mock = MockAIClient::new();
    mock.expect_get_run_conversation()
        .times(1)
        .returning(|_| Err(http_error(422, r#"{"error":"validation failed"}"#)));
    mock.expect_download_run_transcript().times(0);

    let err = load_run_conversation(&mock, TASK_ID).await.unwrap_err();

    assert!(
        err.to_string()
            .contains("API request failed with status 422")
    );
    assert!(
        err.chain().any(|cause| cause
            .downcast_ref::<HttpStatusError>()
            .is_some_and(|status| {
                status.status == 422 && status.body.contains("validation failed")
            }))
    );
}

#[tokio::test]
async fn load_run_conversation_preserves_not_found() {
    let mut mock = MockAIClient::new();
    mock.expect_get_run_conversation()
        .times(1)
        .returning(|_| {
            Err(http_error(
                404,
                r#"{"type":"https://docs.warp.dev/errors/resource_not_found","title":"conversation not found"}"#,
            ))
        });
    mock.expect_download_run_transcript().times(0);

    let err = load_run_conversation(&mock, TASK_ID).await.unwrap_err();

    assert!(
        err.to_string()
            .contains("API request failed with status 404")
    );
}

#[tokio::test]
async fn load_run_conversation_reports_missing_raw_transcript() {
    let mut mock = MockAIClient::new();
    mock.expect_get_run_conversation()
        .times(1)
        .returning(|_| Err(operation_not_supported_error()));
    mock.expect_download_run_transcript()
        .times(1)
        .returning(|_| {
            Err(http_error(
                404,
                r#"{"type":"https://docs.warp.dev/errors/resource_not_found","title":"no transcript path in manifest"}"#,
            ))
        });

    let err = load_run_conversation(&mock, TASK_ID).await.unwrap_err();

    assert_eq!(
        err.to_string(),
        "Raw transcript not found for run 00000000-0000-0000-0000-000000000001. It may not have been uploaded yet."
    );
}

#[tokio::test]
async fn load_public_conversation_returns_normalized_json() {
    let conversation = native_conversation();
    let mut mock = MockAIClient::new();
    mock.expect_get_public_conversation().times(1).returning({
        let conversation = conversation.clone();
        move |conversation_id| {
            assert_eq!(conversation_id, "conv-native");
            Ok(conversation.clone())
        }
    });
    mock.expect_download_conversation_transcript().times(0);

    let output = load_public_conversation(&mock, "conv-native")
        .await
        .unwrap();

    assert_eq!(output, ConversationCliOutput::Normalized(conversation));
}

#[tokio::test]
async fn load_public_conversation_falls_back_to_raw_transcript_for_third_party_harness() {
    const RAW_TRANSCRIPT: &[u8] = b"{\"type\":\"claude_code\"}\n";
    let mut mock = MockAIClient::new();
    mock.expect_get_public_conversation()
        .times(1)
        .returning(|_| Err(operation_not_supported_error()));
    mock.expect_download_conversation_transcript()
        .times(1)
        .returning(|conversation_id| {
            assert_eq!(conversation_id, "conv-third-party");
            Ok(Bytes::from_static(RAW_TRANSCRIPT))
        });

    let output = load_public_conversation(&mock, "conv-third-party")
        .await
        .unwrap();

    assert_eq!(
        output,
        ConversationCliOutput::RawTranscript(Bytes::from_static(RAW_TRANSCRIPT))
    );
}

#[tokio::test]
async fn load_public_conversation_preserves_unrelated_422() {
    let mut mock = MockAIClient::new();
    mock.expect_get_public_conversation()
        .times(1)
        .returning(|_| Err(http_error(422, r#"{"error":"validation failed"}"#)));
    mock.expect_download_conversation_transcript().times(0);

    let err = load_public_conversation(&mock, "conv-third-party")
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("API request failed with status 422")
    );
}

#[tokio::test]
async fn load_public_conversation_reports_missing_raw_transcript() {
    let mut mock = MockAIClient::new();
    mock.expect_get_public_conversation()
        .times(1)
        .returning(|_| Err(operation_not_supported_error()));
    mock.expect_download_conversation_transcript()
        .times(1)
        .returning(|_| {
            Err(http_error(
                404,
                r#"{"type":"https://docs.warp.dev/errors/resource_not_found","title":"no transcript path in manifest"}"#,
            ))
        });

    let err = load_public_conversation(&mock, "conv-third-party")
        .await
        .unwrap_err();

    assert_eq!(
        err.to_string(),
        "Raw transcript not found for conversation conv-third-party. It may not have been uploaded yet."
    );
}
