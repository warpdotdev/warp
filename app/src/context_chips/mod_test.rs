use super::{github_pull_request_from_chip_value, ChipValue, GithubPullRequestChipValue};

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
fn github_pull_request_from_chip_value_handles_structured_and_text_values() {
    let structured = ChipValue::GithubPullRequest(GithubPullRequestChipValue {
        url: "https://github.com/warpdotdev/warp/pull/789".to_string(),
        number: 789,
        state: "MERGED".to_string(),
        draft: false,
        base_branch: "main".to_string(),
    });
    assert_eq!(
        github_pull_request_from_chip_value(&structured)
            .expect("expected structured PR")
            .number,
        789
    );

    let text = ChipValue::Text("https://github.com/warpdotdev/warp/pull/321".to_string());
    assert_eq!(
        github_pull_request_from_chip_value(&text)
            .expect("expected text PR")
            .number,
        321
    );

    assert!(
        github_pull_request_from_chip_value(&ChipValue::GitDiffStats(
            super::display_chip::GitLineChanges {
                files_changed: 1,
                lines_added: 2,
                lines_removed: 3,
            },
        ))
        .is_none()
    );
}
