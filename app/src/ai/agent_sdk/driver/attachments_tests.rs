use std::fs;
use std::io::Write;
use std::sync::Arc;

use mockito::{Matcher, Server};
use tempfile::{Builder as TempDirBuilder, NamedTempFile, TempDir};

use super::*;
use crate::ai::agent_sdk::test_support::build_test_http_client;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::MockAIClient;

fn use_english_locale() {
    i18n::set_locale("en");
}

#[test]
fn process_attachment_text_file() {
    let mut f = NamedTempFile::with_suffix(".txt").unwrap();
    write!(f, "hello world").unwrap();

    let result = process_attachment(&f.path().to_path_buf(), 0).unwrap();
    assert_eq!(
        result.file_name,
        f.path().file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(result.mime_type, "text/plain");
    assert_eq!(
        general_purpose::STANDARD.decode(&result.data).unwrap(),
        b"hello world"
    );
}

#[test]
fn process_attachment_too_large() {
    use_english_locale();

    let mut f = NamedTempFile::with_suffix(".bin").unwrap();
    let data = vec![0u8; MAX_ATTACHMENT_SIZE_BYTES + 1];
    f.write_all(&data).unwrap();

    let err = process_attachment(&f.path().to_path_buf(), 0).unwrap_err();
    assert!(err.to_string().contains("too large"));
}

#[test]
fn process_attachment_nonexistent_file() {
    use_english_locale();

    let path = std::path::PathBuf::from("/tmp/nonexistent_attachment_test_file.xyz");
    let err = process_attachment(&path, 0).unwrap_err();
    assert!(err.to_string().contains("Failed to read"));
}

// End-to-end handoff snapshot download tests. Each test drives the real
// `fetch_and_download_handoff_snapshot_attachments` pipeline (including the shared
// `with_bounded_retry` helper) against a `mockito::Server` + a real `http_client::Client`,
// with `MockAIClient` stubbing only the listing call.

fn handoff_tempdir() -> TempDir {
    TempDirBuilder::new()
        .prefix("handoff-test")
        .tempdir()
        .unwrap()
}

/// Construct a `TaskAttachment` pointing at `{server_base_url}/download/{file_id}` so each test
/// can register matching mocks without fishing for URLs.
fn make_attachment(server_base_url: &str, file_id: &str, filename: &str) -> TaskAttachment {
    TaskAttachment {
        file_id: file_id.to_string(),
        filename: filename.to_string(),
        download_url: format!("{server_base_url}/download/{file_id}"),
        mime_type: "application/octet-stream".to_string(),
    }
}

/// Regex path matcher for `/download/<file_id>`.
fn download_path(file_id: &str) -> Matcher {
    Matcher::Regex(format!("^/download/{file_id}$"))
}

/// Pre-parsed task id for tests that exercise the outer function. Any valid UUID works; the
/// mocked `AIClient` consumes this opaquely.
fn fake_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

/// Build a `MockAIClient` whose `get_handoff_snapshot_attachments` returns `attachments`.
fn mock_client_returning(attachments: Vec<TaskAttachment>) -> Arc<MockAIClient> {
    let mut mock = MockAIClient::new();
    mock.expect_get_handoff_snapshot_attachments()
        .times(1)
        .returning(move |_task_id| Ok(attachments.clone()));
    Arc::new(mock)
}

#[tokio::test]
async fn e2e_happy_path_downloads_all_and_writes_to_disk() {
    // Two attachments, both served 200 with distinct payloads. The pipeline must write each
    // byte stream to `{attachments_dir}/handoff/{file_id}` and report the dir back.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();
    let mut server = Server::new_async().await;

    let first_mock = server
        .mock("GET", download_path("alpha-uuid"))
        .with_status(200)
        .with_body("alpha-body")
        .expect(1)
        .create_async()
        .await;
    let second_mock = server
        .mock("GET", download_path("beta-uuid"))
        .with_status(200)
        .with_body("beta-body")
        .expect(1)
        .create_async()
        .await;

    let attachments = vec![
        make_attachment(&server.url(), "alpha-uuid", "alpha.patch"),
        make_attachment(&server.url(), "beta-uuid", "beta.patch"),
    ];

    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(attachments),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .expect("should not be fatal");

    assert_eq!(result.as_deref(), Some(&*attachments_dir.to_string_lossy()));
    assert_eq!(
        fs::read(attachments_dir.join("handoff").join("alpha-uuid")).unwrap(),
        b"alpha-body"
    );
    assert_eq!(
        fs::read(attachments_dir.join("handoff").join("beta-uuid")).unwrap(),
        b"beta-body"
    );
    first_mock.assert();
    second_mock.assert();
}

#[tokio::test]
async fn e2e_transient_5xx_retried_then_succeeds() {
    // Declare the failing 503 mock first, then the success mock. Mockito serves them in
    // registration order, so attempt #1 hits 503 and attempt #2 hits 200.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();
    let mut server = Server::new_async().await;

    let flaky_fail = server
        .mock("GET", download_path("flaky-uuid"))
        .with_status(503)
        .with_body("temporarily unavailable")
        .expect(1)
        .create_async()
        .await;
    let flaky_ok = server
        .mock("GET", download_path("flaky-uuid"))
        .with_status(200)
        .with_body("finally-here")
        .expect(1)
        .create_async()
        .await;

    let attachments = vec![make_attachment(&server.url(), "flaky-uuid", "flaky.patch")];
    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(attachments),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .unwrap();

    assert_eq!(result.as_deref(), Some(&*attachments_dir.to_string_lossy()));
    assert_eq!(
        fs::read(attachments_dir.join("handoff").join("flaky-uuid")).unwrap(),
        b"finally-here"
    );
    flaky_fail.assert_async().await;
    flaky_ok.assert_async().await;
}

#[tokio::test]
async fn e2e_permanent_4xx_fails_fast_without_retries() {
    // 404 is a permanent error; the retry loop must NOT retry. Exactly one GET is expected.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();
    let mut server = Server::new_async().await;

    let missing = server
        .mock("GET", download_path("missing-uuid"))
        .with_status(404)
        .with_body("not found")
        .expect(1) // exactly one attempt
        .create_async()
        .await;

    let attachments = vec![make_attachment(&server.url(), "missing-uuid", "gone.patch")];
    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(attachments),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .unwrap();

    assert!(result.is_none(), "no file landed, dir should be None");
    assert!(
        !attachments_dir
            .join("handoff")
            .join("missing-uuid")
            .exists(),
        "no file should be written on permanent failure"
    );
    missing.assert_async().await;
}

#[tokio::test]
async fn e2e_retry_exhaustion_marks_failed() {
    // Three persistent 5xxs: the retry loop bails out after MAX_ATTEMPTS (3); no file lands.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();
    let mut server = Server::new_async().await;

    let persistent = server
        .mock("GET", download_path("dead-uuid"))
        .with_status(502)
        .with_body("bad gateway")
        .expect(3) // exactly MAX_ATTEMPTS
        .create_async()
        .await;

    let attachments = vec![make_attachment(&server.url(), "dead-uuid", "dead.patch")];
    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(attachments),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .unwrap();

    assert!(result.is_none());
    assert!(!attachments_dir.join("handoff").join("dead-uuid").exists());
    persistent.assert_async().await;
}

#[tokio::test]
async fn e2e_partial_success_returns_dir_with_downloaded_subset() {
    // One attachment succeeds, one fails permanently. The dir is returned so the caller can
    // still see `{handoff_dir}/ok-uuid` downstream; the failed sibling's file is absent.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();
    let mut server = Server::new_async().await;

    let ok_mock = server
        .mock("GET", download_path("ok-uuid"))
        .with_status(200)
        .with_body("present")
        .expect(1)
        .create_async()
        .await;
    let bad_mock = server
        .mock("GET", download_path("bad-uuid"))
        .with_status(403)
        .with_body("forbidden")
        .expect(1)
        .create_async()
        .await;

    let attachments = vec![
        make_attachment(&server.url(), "ok-uuid", "ok.patch"),
        make_attachment(&server.url(), "bad-uuid", "bad.patch"),
    ];
    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(attachments),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .unwrap();

    assert_eq!(result.as_deref(), Some(&*attachments_dir.to_string_lossy()));
    assert_eq!(
        fs::read(attachments_dir.join("handoff").join("ok-uuid")).unwrap(),
        b"present"
    );
    assert!(!attachments_dir.join("handoff").join("bad-uuid").exists());
    ok_mock.assert_async().await;
    bad_mock.assert_async().await;
}

#[tokio::test]
async fn e2e_empty_attachment_list_returns_none_without_creating_dir() {
    // With zero attachments listed, the function returns None early and does NOT create the
    // handoff dir — nothing to land there.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();

    let result = fetch_and_download_handoff_snapshot_attachments(
        mock_client_returning(Vec::new()),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .expect("empty list should not be a fatal error");

    assert!(result.is_none());
    assert!(
        !attachments_dir.join("handoff").exists(),
        "handoff dir should not be created when there are no attachments"
    );
}

#[tokio::test]
async fn e2e_get_handoff_snapshot_attachments_failure_is_fatal() {
    use_english_locale();

    // When the listing call errors, the function must return Err wrapping the underlying
    // message with a context describing where it happened.
    let _guard = FeatureFlag::OzHandoff.override_enabled(true);
    let tempdir = handoff_tempdir();
    let http = build_test_http_client();

    let mut mock = MockAIClient::new();
    mock.expect_get_handoff_snapshot_attachments()
        .times(1)
        .returning(|_task_id| Err(anyhow::anyhow!("simulated listing failure")));

    let err = fetch_and_download_handoff_snapshot_attachments(
        Arc::new(mock),
        &http,
        fake_task_id(),
        tempdir.path().to_path_buf(),
    )
    .await
    .expect_err("listing failure must be fatal");

    let chain: Vec<String> = err.chain().map(|c| c.to_string()).collect();
    assert!(
        chain
            .iter()
            .any(|s| s.contains("Failed to fetch handoff snapshot attachments")),
        "expected context-wrapped error in chain: {chain:?}"
    );
    assert!(
        chain
            .iter()
            .any(|s| s.contains("simulated listing failure")),
        "expected underlying error in chain: {chain:?}"
    );
}

#[tokio::test]
async fn e2e_returns_none_when_oz_handoff_flag_is_disabled() {
    // With the feature flag off, the function short-circuits to None without calling the
    // AIClient. Any call site that forgot to gate on the flag would log an error; here we
    // just verify the return value.
    let _guard = FeatureFlag::OzHandoff.override_enabled(false);
    let tempdir = handoff_tempdir();
    let attachments_dir = tempdir.path().to_path_buf();
    let http = build_test_http_client();

    // No expect_get_handoff_snapshot_attachments: if the function calls it, the mock panics.
    let mock = MockAIClient::new();
    let result = fetch_and_download_handoff_snapshot_attachments(
        Arc::new(mock),
        &http,
        fake_task_id(),
        attachments_dir.clone(),
    )
    .await
    .expect("flag-disabled path should not be fatal");

    assert!(result.is_none());
    assert!(!attachments_dir.join("handoff").exists());
}
