use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cynic::{GraphQlError, GraphQlResponse};
use futures::executor::block_on;
use http::StatusCode;
use warp_graphql::client::{GraphQLError, RequestOptions};
use warp_server_auth::auth_state::AuthState;

use super::send_graphql_request;
use crate::auth::AuthEvent;
use crate::base_client::{AuthenticatedGraphqlConfig, BaseClient, GraphqlRoutingConfig};

fn base_client(auth_state: AuthState) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    (
        BaseClient::new(
            Arc::new(http_client::Client::new()),
            Arc::new(auth_state),
            event_sender,
            None,
            GraphqlRoutingConfig::default(),
            AuthenticatedGraphqlConfig::default(),
            false,
        ),
        event_receiver,
    )
}

#[test]
fn refreshable_user_not_in_context_emits_account_disabled_event() {
    let (base_client, event_receiver) = refreshable_base_client();
    let send_count = Arc::new(AtomicUsize::new(0));

    let error = block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::response_errors(
            None,
            send_count.clone(),
            vec!["User not in context: Not found".to_string()],
        ),
        None,
    ))
    .unwrap_err();

    assert!(error.to_string().contains("missing response data"));
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert_user_disabled_event(&event_receiver);
}

fn refreshable_base_client() -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    base_client(AuthState::new_for_test())
}

fn externally_authenticated_base_client(
    bearer_token: &str,
) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    let auth_state = AuthState::new_logged_out_for_test();
    auth_state.set_remote_server_bearer_token(bearer_token.to_string());
    base_client(auth_state)
}

fn missing_credentials_base_client() -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    base_client(AuthState::new_logged_out_for_test())
}

fn assert_no_events(event_receiver: &async_channel::Receiver<AuthEvent>) {
    assert!(event_receiver.try_recv().is_err());
}

fn assert_user_disabled_event(event_receiver: &async_channel::Receiver<AuthEvent>) {
    match event_receiver.try_recv().unwrap() {
        AuthEvent::UserAccountDisabled => {}
        event => panic!("Expected UserAccountDisabled event, got {event:?}"),
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
    let (base_client, event_receiver) = refreshable_base_client();
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful(None, send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(base_client.is_auth_refresh_allowed());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert_no_events(&event_receiver);
}

#[test]
fn refresh_disabled_sends_provided_bearer_token() {
    let (base_client, event_receiver) = externally_authenticated_base_client("daemon-token");
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(send_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful(Some("daemon-token"), send_count.clone()),
        None,
    ))
    .unwrap();

    assert!(!base_client.is_auth_refresh_allowed());
    assert_eq!(send_count.load(Ordering::SeqCst), 1);
    assert_no_events(&event_receiver);
}

#[test]
fn missing_request_credentials_returns_before_sending() {
    let (base_client, event_receiver) = missing_credentials_base_client();
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
    assert_no_events(&event_receiver);
}

#[test]
fn external_auth_rejection_returns_credentials_rejected_without_account_event() {
    let (base_client, event_receiver) = externally_authenticated_base_client("daemon-token");
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
    assert_no_events(&event_receiver);
}

#[test]
fn external_user_not_in_context_returns_credentials_rejected_without_account_event() {
    let (base_client, event_receiver) = externally_authenticated_base_client("daemon-token");
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
    assert_no_events(&event_receiver);
}
