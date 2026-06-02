use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use cynic::{GraphQlError, GraphQlResponse};
use futures::executor::block_on;

use super::*;

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
        options: warp_graphql::client::RequestOptions,
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
fn send_graphql_request_refresh_enabled_uses_auth_state() {
    let server_api = ServerApi::new_for_test();
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(server_api.send_graphql_request(
        FakeGraphqlOperation::successful(None, send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(server_api.allowed_to_refresh_token());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[test]
fn send_graphql_request_refresh_disabled_uses_provided_bearer_token() {
    let (event_sender, _) = async_channel::unbounded();
    let server_api =
        ServerApi::new_for_test_with_bearer_token(Some("daemon-token".to_string()), event_sender);
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(server_api.send_graphql_request(
        FakeGraphqlOperation::successful(Some("daemon-token"), send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(!server_api.allowed_to_refresh_token());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
}

#[test]
fn send_graphql_request_refresh_disabled_missing_token_returns_auth_error() {
    let (event_sender, event_receiver) = async_channel::unbounded();
    let server_api = ServerApi::new_for_test_with_bearer_token(None, event_sender);
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(server_api.send_graphql_request(
        FakeGraphqlOperation::successful(Some("unused-token"), send_count.clone()),
        None,
    ))
    .unwrap_err();

    assert!(has_error_message(
        &error,
        "missing authentication credentials"
    ));
    assert_eq!(send_count.load(Ordering::SeqCst), 0);
    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn send_graphql_request_refresh_disabled_auth_rejection_is_credentials_rejected() {
    let (event_sender, event_receiver) = async_channel::unbounded();
    let server_api =
        ServerApi::new_for_test_with_bearer_token(Some("daemon-token".to_string()), event_sender);
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(server_api.send_graphql_request(
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
    assert!(event_receiver.try_recv().is_err());
}

#[test]
fn send_graphql_request_refresh_disabled_user_not_in_context_is_credentials_rejected() {
    let (event_sender, event_receiver) = async_channel::unbounded();
    let server_api =
        ServerApi::new_for_test_with_bearer_token(Some("daemon-token".to_string()), event_sender);
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(server_api.send_graphql_request(
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
    assert!(event_receiver.try_recv().is_err());
}
