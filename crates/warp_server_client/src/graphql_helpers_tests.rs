use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, bail};
use async_trait::async_trait;
use cynic::{GraphQlError, GraphQlResponse};
use futures::executor::block_on;
use http::StatusCode;
use instant::Duration;
use warp_graphql::client::{GraphQLError, RequestOptions};

use super::send_graphql_request;
use crate::auth::AgentIdentity;
use crate::base_client::BaseClient;

struct FakeBaseClient {
    auth_token: Option<String>,
    request_options_error: Option<&'static str>,
    auth_refresh_allowed: bool,
    disabled_event_count: Arc<AtomicUsize>,
}

impl FakeBaseClient {
    fn configured(auth_token: Option<&str>, auth_refresh_allowed: bool) -> Self {
        Self {
            auth_token: auth_token.map(ToOwned::to_owned),
            request_options_error: None,
            auth_refresh_allowed,
            disabled_event_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn missing_credentials() -> Self {
        Self {
            auth_token: None,
            request_options_error: Some("missing authentication credentials"),
            auth_refresh_allowed: false,
            disabled_event_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl BaseClient for FakeBaseClient {
    fn http_client(&self) -> Arc<http_client::Client> {
        Arc::new(http_client::Client::new())
    }

    fn anonymous_id(&self) -> String {
        String::new()
    }

    fn unauthenticated_graphql_request_options(&self) -> RequestOptions {
        RequestOptions::default()
    }

    async fn graphql_request_options(&self, timeout: Option<Duration>) -> Result<RequestOptions> {
        if let Some(message) = self.request_options_error {
            bail!(message);
        }
        Ok(RequestOptions {
            auth_token: self.auth_token.clone(),
            timeout,
            ..RequestOptions::default()
        })
    }

    async fn list_agent_identities(&self) -> Result<Vec<AgentIdentity>> {
        Ok(Vec::new())
    }

    async fn get_or_create_ambient_workload_token(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn is_auth_refresh_allowed(&self) -> bool {
        self.auth_refresh_allowed
    }

    fn on_graphql_staging_access_blocked(&self) {}

    fn on_graphql_iap_challenge_received(&self) {}

    fn on_graphql_user_account_disabled(&self) {
        self.disabled_event_count.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeGraphqlOperation {
    expected_auth_token: Option<String>,
    send_count: Arc<AtomicUsize>,
    result: FakeGraphqlResult,
}

enum FakeGraphqlResult {
    Success,
    Rejected(StatusCode),
    ResponseErrors(Vec<String>),
}

impl FakeGraphqlOperation {
    fn successful(expected_auth_token: Option<&str>, send_count: Arc<AtomicUsize>) -> Self {
        Self {
            expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
            send_count,
            result: FakeGraphqlResult::Success,
        }
    }

    fn rejected(
        expected_auth_token: Option<&str>,
        send_count: Arc<AtomicUsize>,
        status: StatusCode,
    ) -> Self {
        Self {
            expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
            send_count,
            result: FakeGraphqlResult::Rejected(status),
        }
    }

    fn response_errors(
        expected_auth_token: Option<&str>,
        send_count: Arc<AtomicUsize>,
        messages: Vec<String>,
    ) -> Self {
        Self {
            expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
            send_count,
            result: FakeGraphqlResult::ResponseErrors(messages),
        }
    }
}

impl warp_graphql::client::Operation<()> for FakeGraphqlOperation {
    fn operation_name(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed("FakeGraphqlOperation"))
    }

    fn send_request(
        self,
        _client: Arc<http_client::Client>,
        options: RequestOptions,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<GraphQlResponse<()>, GraphQLError>>
                + Send
                + 'static,
        >,
    >
    where
        Self: Sized,
    {
        Box::pin(async move {
            assert_eq!(options.auth_token, self.expected_auth_token);
            self.send_count.fetch_add(1, Ordering::SeqCst);
            match self.result {
                FakeGraphqlResult::Success => Ok(GraphQlResponse {
                    data: Some(()),
                    errors: None,
                }),
                FakeGraphqlResult::Rejected(status) => Err(GraphQLError::HttpError {
                    status,
                    body: "redacted auth rejection".to_string(),
                }),
                FakeGraphqlResult::ResponseErrors(messages) => Ok(GraphQlResponse {
                    data: None,
                    errors: Some(
                        messages
                            .into_iter()
                            .map(|message| GraphQlError::new(message, None, None, None))
                            .collect(),
                    ),
                }),
            }
        })
    }
}

fn has_error_message(error: &anyhow::Error, expected: &str) -> bool {
    error.chain().any(|cause| cause.to_string() == expected)
}

#[test]
fn refresh_enabled_sends_configured_request_options() {
    let base_client = FakeBaseClient::configured(None, true);
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful(None, send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(base_client.is_auth_refresh_allowed());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[test]
fn refresh_disabled_sends_provided_bearer_token() {
    let base_client = FakeBaseClient::configured(Some("daemon-token"), false);
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful(Some("daemon-token"), send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(!base_client.is_auth_refresh_allowed());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_request_credentials_returns_before_sending() {
    let base_client = FakeBaseClient::missing_credentials();
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful(Some("unused-token"), send_count.clone()),
        None,
    ))
    .unwrap_err();

    assert!(has_error_message(
        &error,
        "missing authentication credentials"
    ));
    assert_eq!(send_count.load(Ordering::SeqCst), 0);
    assert_eq!(base_client.disabled_event_count.load(Ordering::SeqCst), 0);
}

#[test]
fn external_auth_rejection_returns_credentials_rejected_without_account_event() {
    let base_client = FakeBaseClient::configured(Some("daemon-token"), false);
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::rejected(
            Some("daemon-token"),
            send_count.clone(),
            StatusCode::UNAUTHORIZED,
        ),
        None,
    ))
    .unwrap_err();

    assert!(has_error_message(
        &error,
        "server rejected authentication credentials"
    ));
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert_eq!(base_client.disabled_event_count.load(Ordering::SeqCst), 0);
}

#[test]
fn external_user_not_in_context_returns_credentials_rejected_without_account_event() {
    let base_client = FakeBaseClient::configured(Some("daemon-token"), false);
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::response_errors(
            Some("daemon-token"),
            send_count.clone(),
            vec!["User not in context: Not found".to_string()],
        ),
        None,
    ))
    .unwrap_err();

    assert!(has_error_message(
        &error,
        "server rejected authentication credentials"
    ));
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert_eq!(base_client.disabled_event_count.load(Ordering::SeqCst), 0);
}
