use futures::executor::block_on;
use mockito::{Matcher, Server};
use warp_core::channel::ChannelState;
use warp_server_client::base_client::{AMBIENT_WORKLOAD_TOKEN_HEADER, CLOUD_AGENT_ID_HEADER};

use super::{HarnessSupportClient, ServerApi};
use crate::ai::artifacts::Artifact;

#[test]
fn transcript_download_resolves_url_with_requested_task_scope() {
    let ambient_task = "11111111-1111-4111-8111-111111111111";
    for task_scope in [None, Some("22222222-2222-4222-8222-222222222222")] {
        let mut storage = Server::new();
        let storage_request = storage
            .mock("GET", "/transcript")
            .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, Matcher::Missing)
            .match_header(CLOUD_AGENT_ID_HEADER, Matcher::Missing)
            .match_header("x-warp-os-category", Matcher::Missing)
            .with_status(200)
            .with_body("raw transcript")
            .create();
        let api_request = {
            let mut server = ChannelState::mock_server();
            server
                .mock("GET", "/api/v1/harness-support/transcript")
                .match_query(Matcher::UrlEncoded("redirect".into(), "false".into()))
                .match_header(CLOUD_AGENT_ID_HEADER, task_scope.unwrap_or(ambient_task))
                .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, "test-workload-token")
                .with_status(200)
                .with_body(
                    serde_json::json!({"url": format!("{}/transcript", storage.url())}).to_string(),
                )
                .create()
        };
        let api = ServerApi::new_for_test();
        api.base_client
            .set_ambient_workload_token_for_test("test-workload-token".into(), None);
        api.set_ambient_agent_task_id(Some(ambient_task.parse().unwrap()));
        let bytes = block_on(async {
            if let Some(task_id) = task_scope {
                api.fetch_transcript_for_task(&task_id.parse().unwrap())
                    .await
            } else {
                api.fetch_transcript().await
            }
        })
        .unwrap();
        assert_eq!(bytes.as_ref(), b"raw transcript");
        api_request.assert();
        storage_request.assert();
    }
}

/// The server marshals its unset Go maps and slices as `null`, which a POST
/// target routinely carries for `headers`.
#[test]
fn upload_target_deserializes_null_headers_and_fields_as_empty() {
    use super::UploadTarget;

    let target: UploadTarget = serde_json::from_value(serde_json::json!({
        "url": "https://example.com/upload",
        "method": "PUT",
        "headers": null,
        "fields": null
    }))
    .unwrap();

    assert!(target.headers.is_empty());
    assert!(target.fields.is_empty());
}

/// Each `kind` here is a discriminator from the `UploadFieldValue` schema in
/// warp-server's `public_api/openapi.yaml`. A mismatch fails the whole target,
/// so every presigned POST upload would break.
#[test]
fn upload_field_value_deserializes_every_server_kind() {
    use super::{UploadFieldValue, UploadTarget};

    let target: UploadTarget = serde_json::from_value(serde_json::json!({
        "url": "https://example.com/upload",
        "method": "POST",
        "headers": {},
        "fields": [
            {"name": "key", "value": {"kind": "static", "value": "object/key"}},
            {"name": "x-amz-checksum-crc32c", "value": {"kind": "content_crc32c"}},
            {"name": "file", "value": {"kind": "content_data"}}
        ]
    }))
    .unwrap();

    assert!(matches!(
        &target.fields[0].value,
        UploadFieldValue::Static { value } if value == "object/key"
    ));
    assert!(matches!(
        target.fields[1].value,
        UploadFieldValue::ContentCrc32C
    ));
    assert!(matches!(
        target.fields[2].value,
        UploadFieldValue::ContentData
    ));
}

/// Assert that `Artifact`s serialize to the expected format for the /harness-support/report-artifact
/// endpoint.
/// If `Artifact` serialization changes, this test will catch it.
#[test]
fn pull_request_artifact_serializes_to_expected_wire_format() {
    let artifact = Artifact::PullRequest {
        url: "https://github.com/org/repo/pull/42".to_string(),
        branch: "feature-branch".to_string(),
        repo: Some("repo".to_string()),
        number: Some(42),
    };
    let json = serde_json::to_value(&artifact).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "artifact_type": "PULL_REQUEST",
            "data": {
                "url": "https://github.com/org/repo/pull/42",
                "branch": "feature-branch"
            }
        })
    );
}

#[test]
fn report_shutdown_abnormal_serializes_exit_code() {
    use super::ReportShutdownRequest;

    let request = ReportShutdownRequest::abnormal(
        "process_exit".to_string(),
        "agent exited with status 143".to_string(),
        Some(143),
    );
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "error": {
                "category": "process_exit",
                "message": "agent exited with status 143",
                "exit_code": 143
            }
        })
    );
}

/// An `EXTERNAL_REFERENCE` artifact with only the required fields omits `title`
/// and `metadata` from the wire format, matching the server's
/// `ExternalReferenceArtifactData` schema.
#[test]
fn external_reference_artifact_serializes_to_expected_wire_format() {
    let artifact = Artifact::ExternalReference {
        reference_type: "LINEAR_ISSUE".to_string(),
        url: "https://linear.app/warpdotdev/issue/REMOTE-2253".to_string(),
        title: None,
        metadata: None,
    };
    let json = serde_json::to_value(&artifact).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "artifact_type": "EXTERNAL_REFERENCE",
            "data": {
                "reference_type": "LINEAR_ISSUE",
                "url": "https://linear.app/warpdotdev/issue/REMOTE-2253"
            }
        })
    );
}

/// An `EXTERNAL_REFERENCE` artifact includes `title` and `metadata` when present.
#[test]
fn external_reference_artifact_includes_optional_title_and_metadata() {
    let artifact = Artifact::ExternalReference {
        reference_type: "GITHUB_PR".to_string(),
        url: "https://github.com/warpdotdev/warp/pull/1".to_string(),
        title: Some("My pull request".to_string()),
        metadata: Some(serde_json::json!({"key": "val"})),
    };
    let json = serde_json::to_value(&artifact).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "artifact_type": "EXTERNAL_REFERENCE",
            "data": {
                "reference_type": "GITHUB_PR",
                "url": "https://github.com/warpdotdev/warp/pull/1",
                "title": "My pull request",
                "metadata": {"key": "val"}
            }
        })
    );
}

/// An `EXTERNAL_REFERENCE` artifact round-trips through deserialize, so run
/// responses carrying it no longer fall through to the unknown/skip path.
#[test]
fn external_reference_artifact_round_trips() {
    let artifact = Artifact::ExternalReference {
        reference_type: "LINEAR_ISSUE".to_string(),
        url: "https://linear.app/warpdotdev/issue/REMOTE-2253".to_string(),
        title: Some("Add report-external-reference CLI subcommand".to_string()),
        metadata: Some(serde_json::json!({"estimate": "S"})),
    };
    let json = serde_json::to_value(&artifact).unwrap();
    let deserialized: Artifact = serde_json::from_value(json).unwrap();
    assert_eq!(artifact, deserialized);
}

#[test]
fn report_shutdown_clean_serializes_without_error() {
    use super::ReportShutdownRequest;

    let request = ReportShutdownRequest::clean();
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn report_shutdown_abnormal_serializes_with_error() {
    use super::ReportShutdownRequest;

    let request =
        ReportShutdownRequest::abnormal("oom".to_string(), "out of memory".to_string(), None);
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "error": {
                "category": "oom",
                "message": "out of memory"
            }
        })
    );
}
