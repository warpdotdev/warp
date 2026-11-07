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

use super::{
    send_graphql_request, send_task_scoped_graphql_request, send_team_scoped_graphql_request,
};
use crate::auth::AuthEvent;
use crate::base_client::{
    AMBIENT_WORKLOAD_TOKEN_HEADER, AuthenticatedGraphqlConfig, BaseClient, CLOUD_AGENT_ID_HEADER,
    GraphqlRoutingConfig, PREWARMED_SANDBOX_ID_HEADER, PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER,
    TEAM_UID_HEADER,
};

fn base_client(auth_state: AuthState) -> (BaseClient, async_channel::Receiver<AuthEvent>) {
    let (event_sender, event_receiver) = async_channel::unbounded();
    (
        BaseClient::new(
            Arc::new(http_client::Client::new()),
            Arc::new(http_client::Client::new_external()),
            Arc::new(auth_state),
            event_sender,
            None,
            GraphqlRoutingConfig::default(),
            AuthenticatedGraphqlConfig::default(),
            None,
        ),
        event_receiver,
    )
}

#[test]
fn task_scoped_requests_preserve_workload_and_global_identity() {
    let (client, events) = externally_authenticated_base_client("daemon-token");
    client.set_ambient_agent_task_id(Some("global-run".to_string()));
    client.set_ambient_workload_token_for_test("workload-proof".to_string(), None);
    let sends = Arc::new(AtomicUsize::new(0));
    let operation = |run_id: &str| {
        let mut operation = FakeGraphqlOperation::successful(Some("daemon-token"), sends.clone());
        operation.expected_ambient = Some((run_id.to_string(), Some("workload-proof".to_string())));
        operation
    };
    block_on(async {
        let (first, second) = futures::join!(
            send_task_scoped_graphql_request(
                &client,
                operation("first-run"),
                None,
                "first-run".to_string(),
            ),
            send_task_scoped_graphql_request(
                &client,
                operation("second-run"),
                None,
                "second-run".to_string(),
            )
        );
        first.unwrap();
        second.unwrap();
        send_graphql_request(&client, operation("global-run"), None)
            .await
            .unwrap();
    });
    assert_eq!(sends.load(Ordering::SeqCst), 3);
    assert_no_events(&events);
}

#[test]
fn task_scoped_request_supports_interactive_client_without_ambient_identity() {
    let (client, events) = externally_authenticated_base_client("daemon-token");
    let sends = Arc::new(AtomicUsize::new(0));
    let mut operation = FakeGraphqlOperation::successful(Some("daemon-token"), sends.clone());
    operation.expected_ambient = Some(("conversation-run".to_string(), None));
    block_on(send_task_scoped_graphql_request(
        &client,
        operation,
        None,
        "conversation-run".to_string(),
    ))
    .unwrap();
    let options = block_on(client.graphql_request_options(None)).unwrap();
    assert!(!options.headers.contains_key(CLOUD_AGENT_ID_HEADER));
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert_no_events(&events);
}

#[test]
fn task_scoped_request_rejects_standby_without_sending() {
    let (client, events) = externally_authenticated_base_client("daemon-token");
    client.set_prewarmed_sandbox_id("standby".to_string());
    client.set_ambient_workload_token_for_test("standby-proof".to_string(), None);
    let sends = Arc::new(AtomicUsize::new(0));
    let error = block_on(send_task_scoped_graphql_request(
        &client,
        FakeGraphqlOperation::successful(Some("daemon-token"), sends.clone()),
        None,
        "run".to_string(),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("standby"));
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    assert_no_events(&events);
}

#[test]
fn task_scoped_request_rejects_empty_task_without_sending() {
    let (client, events) = externally_authenticated_base_client("daemon-token");
    let sends = Arc::new(AtomicUsize::new(0));
    for task_id in ["", " "] {
        let error = block_on(send_task_scoped_graphql_request(
            &client,
            FakeGraphqlOperation::successful(Some("daemon-token"), sends.clone()),
            None,
            task_id.to_string(),
        ))
        .unwrap_err();
        assert!(error.to_string().contains("Task ID"));
    }
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    assert_no_events(&events);
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
    expected_team_uid: Option<String>,
    expected_ambient: Option<(String, Option<String>)>,
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
            expected_team_uid: None,
            expected_ambient: None,
            send_count,
            result: FakeGraphqlResult::Success,
        }
    }

    fn successful_for_team(
        expected_auth_token: Option<&str>,
        expected_team_uid: &str,
        send_count: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
            expected_team_uid: Some(expected_team_uid.to_string()),
            expected_ambient: None,
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
            expected_team_uid: None,
            send_count,
            result: FakeGraphqlResult::Rejected(status),
            expected_ambient: None,
        }
    }

    fn response_errors(
        expected_auth_token: Option<&str>,
        send_count: Arc<AtomicUsize>,
        messages: Vec<String>,
    ) -> Self {
        Self {
            expected_auth_token: expected_auth_token.map(ToOwned::to_owned),
            expected_team_uid: None,
            send_count,
            result: FakeGraphqlResult::ResponseErrors(messages),
            expected_ambient: None,
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
            assert_eq!(
                options.headers.get(TEAM_UID_HEADER),
                self.expected_team_uid.as_ref()
            );
            if let Some((task_id, workload_token)) = self.expected_ambient {
                assert_eq!(options.headers.get(CLOUD_AGENT_ID_HEADER), Some(&task_id));
                assert_eq!(
                    options.headers.get(AMBIENT_WORKLOAD_TOKEN_HEADER),
                    workload_token.as_ref()
                );
                assert!(!options.headers.contains_key(PREWARMED_SANDBOX_ID_HEADER));
                assert!(
                    !options
                        .headers
                        .contains_key(PREWARMED_SANDBOX_WORKLOAD_TOKEN_HEADER)
                );
            }
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
fn team_scoped_request_sends_configured_options_and_team_header() {
    let (base_client, event_receiver) = externally_authenticated_base_client("daemon-token");
    let send_count = Arc::new(AtomicUsize::new(0));

    block_on(send_team_scoped_graphql_request(
        &base_client,
        FakeGraphqlOperation::successful_for_team(
            Some("daemon-token"),
            "team-uid",
            send_count.clone(),
        ),
        None,
        "team-uid".to_string(),
    ))
    .unwrap();

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
