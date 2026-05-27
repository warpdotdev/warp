use super::{ChipValue, GithubPullRequestChipValue};

#[test]
fn github_pull_request_chip_value_parses_structured_json() {
    let pull_request = GithubPullRequestChipValue::from_text(
        r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123","number":123,"state":"OPEN","draft":true,"base_branch":"main"}"#,
    )
    .expect("expected structured PR chip value");

    assert_eq!(
        pull_request.url,
        "https://github.com/warpdotdev/warp-internal/pull/123"
    );
    assert_eq!(pull_request.number, 123);
    assert_eq!(pull_request.state, "OPEN");
    assert!(pull_request.draft);
    assert_eq!(pull_request.base_branch, "main");
}

#[test]
fn github_pull_request_chip_value_parses_legacy_string_number() {
    let pull_request = GithubPullRequestChipValue::from_text(
        r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123","number":"123","state":"OPEN","draft":false,"base_branch":"main"}"#,
    )
    .expect("expected structured PR chip value");

    assert_eq!(pull_request.number, 123);
}

#[test]
fn github_pull_request_chip_value_parses_legacy_url() {
    let pull_request =
        GithubPullRequestChipValue::from_text("https://github.com/warpdotdev/warp/pull/456")
            .expect("expected legacy PR URL");

    assert_eq!(pull_request.number, 456);
    assert_eq!(pull_request.state, "");
    assert!(!pull_request.draft);
    assert_eq!(pull_request.base_branch, "");
}

#[test]
fn github_pull_request_chip_value_rejects_invalid_number_without_url_fallback() {
    assert!(GithubPullRequestChipValue::from_text(
        r#"{"url":"","number":"not-a-number","state":"OPEN","draft":false,"base_branch":"main"}"#,
    )
    .is_none());
}

#[test]
fn chip_value_deserializes_structured_github_pull_request() {
    let value = serde_json::from_str::<ChipValue>(
        r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123","number":123}"#,
    )
    .expect("expected structured PR chip value");

    assert_eq!(
        value.as_github_pull_request().map(|pr| pr.number),
        Some(123)
    );
}

#[test]
fn chip_value_rejects_unknown_object_as_github_pull_request() {
    assert!(serde_json::from_str::<ChipValue>(r#"{"unknown":"value"}"#).is_err());
}

#[test]
fn chip_value_rejects_github_pull_request_with_extra_fields() {
    assert!(
        serde_json::from_str::<ChipValue>(
            r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123","number":123,"unknown":"value"}"#,
        )
        .is_err()
    );
}

#[test]
fn chip_value_rejects_github_pull_request_without_url_or_number() {
    assert!(serde_json::from_str::<ChipValue>(
        r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123"}"#,
    )
    .is_err());
    assert!(serde_json::from_str::<ChipValue>(r#"{"number":123}"#).is_err());
}

#[test]
fn chip_value_rejects_github_pull_request_with_default_url_or_number() {
    assert!(serde_json::from_str::<ChipValue>(r#"{"url":"","number":123}"#).is_err());
    assert!(serde_json::from_str::<ChipValue>(
        r#"{"url":"https://github.com/warpdotdev/warp-internal/pull/123","number":0}"#,
    )
    .is_err());
}
