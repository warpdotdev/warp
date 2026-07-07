use crate::ai::artifacts::Artifact;

#[test]
fn upload_target_deserializes_null_fields_as_empty() {
    use super::UploadTarget;

    let target: UploadTarget = serde_json::from_value(serde_json::json!({
        "url": "https://example.com/upload",
        "method": "PUT",
        "headers": {},
        "fields": null
    }))
    .unwrap();

    assert_eq!(target.fields.len(), 0);
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
