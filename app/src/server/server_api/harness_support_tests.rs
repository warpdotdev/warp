use crate::ai::artifacts::Artifact;

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

    let request = ReportShutdownRequest::abnormal("oom".to_string(), "out of memory".to_string());
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
