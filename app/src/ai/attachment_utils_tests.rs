use std::sync::Arc;

use mockito::{Matcher, Server};
use tempfile::TempDir;

use super::*;
use crate::server::server_api::ai::{
    AttachmentDownloadInfo, DownloadAttachmentsResponse, MockAIClient,
};

fn fake_task_id() -> AmbientAgentTaskId {
    "550e8400-e29b-41d4-a716-446655440000".parse().unwrap()
}

fn download_path(attachment_id: &str) -> Matcher {
    Matcher::Regex(format!("^/download/{attachment_id}$"))
}

/// A plain HTTP client with no proxy/TLS-cert config and a disabled connection pool, so tests
/// don't hold sockets open past their return (see the identical helper in
/// `ai::agent_sdk::test_support`, not reusable here across module boundaries).
fn test_http_client() -> Arc<http_client::Client> {
    let builder = reqwest::ClientBuilder::new()
        .tls_certs_only([])
        .no_proxy()
        .pool_max_idle_per_host(0);
    Arc::new(
        http_client::Client::from_client_builder(builder)
            .expect("should not fail to build test http client"),
    )
}

// A download failure -- whether resolving URLs or fetching an individual file -- must never
// cause the whole batch to error out: the caller always sends the prompt these attachments are
// for, with whatever (possibly empty) subset actually downloaded.

#[tokio::test]
async fn url_lookup_failure_returns_empty_without_erroring() {
    let mut mock = MockAIClient::new();
    mock.expect_download_task_attachments()
        .times(1)
        .returning(|_task_id, _ids| Err(anyhow::anyhow!("simulated URL lookup failure")));

    let dest_dir = TempDir::new().unwrap();
    let result = download_task_file_attachments(
        Arc::new(mock),
        test_http_client(),
        fake_task_id(),
        dest_dir.path().to_path_buf(),
        vec![("attachment-1".to_string(), "file1.txt".to_string())],
    )
    .await;

    assert!(result.is_empty());
}

#[tokio::test]
async fn per_file_failure_is_skipped_while_others_still_download() {
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
        .with_status(404)
        .with_body("not found")
        .expect(1)
        .create_async()
        .await;

    let server_url = server.url();
    let mut mock = MockAIClient::new();
    mock.expect_download_task_attachments()
        .times(1)
        .returning(move |_task_id, _ids| {
            Ok(DownloadAttachmentsResponse {
                attachments: vec![
                    AttachmentDownloadInfo {
                        attachment_id: "ok-uuid".to_string(),
                        download_url: format!("{server_url}/download/ok-uuid"),
                    },
                    AttachmentDownloadInfo {
                        attachment_id: "bad-uuid".to_string(),
                        download_url: format!("{server_url}/download/bad-uuid"),
                    },
                ],
            })
        });

    let dest_dir = TempDir::new().unwrap();
    let downloaded = download_task_file_attachments(
        Arc::new(mock),
        test_http_client(),
        fake_task_id(),
        dest_dir.path().to_path_buf(),
        vec![
            ("ok-uuid".to_string(), "ok.txt".to_string()),
            ("bad-uuid".to_string(), "bad.txt".to_string()),
        ],
    )
    .await;

    assert_eq!(downloaded.len(), 1);
    assert_eq!(downloaded[0].file_id, "ok-uuid");
    assert_eq!(
        std::fs::read_to_string(&downloaded[0].file_path).unwrap(),
        "present"
    );
    ok_mock.assert_async().await;
    bad_mock.assert_async().await;
}

#[tokio::test]
async fn missing_download_url_for_an_attachment_is_skipped() {
    // The server can omit an attachment from the response entirely (e.g. it expired); that
    // specific file is skipped rather than the whole batch failing.
    let mut mock = MockAIClient::new();
    mock.expect_download_task_attachments()
        .times(1)
        .returning(|_task_id, _ids| {
            Ok(DownloadAttachmentsResponse {
                attachments: vec![],
            })
        });

    let dest_dir = TempDir::new().unwrap();
    let downloaded = download_task_file_attachments(
        Arc::new(mock),
        test_http_client(),
        fake_task_id(),
        dest_dir.path().to_path_buf(),
        vec![("missing-uuid".to_string(), "missing.txt".to_string())],
    )
    .await;

    assert!(downloaded.is_empty());
}
