use std::sync::Arc;

use mockito::Matcher;
use serde_json::json;
use warp_server_auth::auth_state::AuthState;
use warp_server_auth::credentials::Credentials;

use super::*;

const PREVIEW_ID: Uuid = Uuid::from_u128(1);

#[test]
fn rereads_worker_credentials_for_the_bound_run() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (events, _) = async_channel::unbounded();
    let state = Arc::new(AuthState::new_for_test());
    let client = BaseClient::new(
        Arc::new(http_client::Client::new()),
        Arc::new(http_client::Client::new_external()),
        state.clone(),
        events,
        None,
        Default::default(),
        Default::default(),
        None,
    );
    for (worker, workload) in [
        ("first-worker", "first-workload"),
        ("next-worker", "next-workload"),
    ] {
        state.set_credentials(Some(Credentials::ApiKey {
            key: worker.to_owned(),
            owner_type: None,
        }));
        client.set_ambient_workload_token_for_test(workload.to_owned(), None);
        let response = {
            let mut server = ChannelState::mock_server();
            server
                .mock(
                    "POST",
                    format!("/api/v1/agent/runs/run-one/previews/{PREVIEW_ID}/authorize").as_str(),
                )
                .match_header("authorization", format!("Bearer {worker}").as_str())
                .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, workload)
                .match_header(super::super::CLOUD_AGENT_ID_HEADER, "run-one")
                .match_body(Matcher::Json(json!({
                    "ingress_token": "Bearer ingress",
                    "execution_id": 42,
                    "port": 3000,
                })))
                .with_status(204)
                .create()
        };

        runtime
            .block_on(client.validate_preview_ingress_token(
                "run-one",
                42,
                PREVIEW_ID,
                3000,
                "Bearer ingress",
            ))
            .unwrap();

        response.assert();
    }
}

async fn authorize(server: &mockito::ServerGuard) -> Result<()> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer worker"),
    );
    headers.insert(
        "x-warp-ambient-workload-token",
        HeaderValue::from_static("workload"),
    );
    send_ingress_validation(
        Url::parse(&format!("{}/authorize", server.url())).unwrap(),
        headers,
        IngressRequest {
            ingress_token: "Bearer ingress",
            execution_id: 42,
            port: 3000,
        },
    )
    .await
}

#[tokio::test]
async fn separates_worker_headers_from_ingress_token_body() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("POST", "/authorize")
        .match_header("authorization", "Bearer worker")
        .match_header(AMBIENT_WORKLOAD_TOKEN_HEADER, "workload")
        .match_header("x-warp-preview-authorization", Matcher::Missing)
        .match_header("x-warp-preview-ingress-token", Matcher::Missing)
        .match_body(Matcher::Json(json!({
            "ingress_token": "Bearer ingress",
            "execution_id": 42,
            "port": 3000,
        })))
        .with_status(204)
        .create_async()
        .await;

    authorize(&server).await.unwrap();

    request.assert_async().await;
}

#[tokio::test]
async fn rejects_same_origin_redirects_without_resending_ingress_tokens() {
    let mut server = mockito::Server::new_async().await;
    let redirect = server
        .mock("POST", "/authorize")
        .with_status(307)
        .with_header("location", "/credential-recipient")
        .create_async()
        .await;
    let recipient = server
        .mock("POST", "/credential-recipient")
        .expect(0)
        .with_status(204)
        .create_async()
        .await;

    assert!(authorize(&server).await.is_err());

    redirect.assert_async().await;
    recipient.assert_async().await;
}

#[tokio::test]
async fn rejects_every_status_except_no_content() {
    let mut server = mockito::Server::new_async().await;
    for status in [
        200, 201, 202, 206, 301, 307, 400, 401, 403, 404, 429, 500, 503,
    ] {
        let response = server
            .mock("POST", "/authorize")
            .with_status(status)
            .create_async()
            .await;

        assert!(authorize(&server).await.is_err(), "{status}");
        response.assert_async().await;
        response.remove_async().await;
    }
}
