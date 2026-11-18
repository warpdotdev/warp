use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::stream::BoxStream;
use futures::{FutureExt as _, StreamExt as _};
use http::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use serde_json::json;
use sse_stream::{Error as SseError, Sse};

use super::{RefreshableHeaderClient, RefreshableHeaders, RejectionHandler};

const EXPIRED_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_expired""#;

/// Records the `Authorization` value of every request and rejects the ones that do not carry
/// the accepted value.
#[derive(Clone)]
struct FakeClient {
    accepted: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeClient {
    fn accepting(accepted: &'static str) -> Self {
        Self {
            accepted,
            seen: Arc::default(),
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("seen requests").clone()
    }

    fn check(
        &self,
        headers: &HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<io::Error>> {
        let authorization = headers
            .get(&http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let accepted = authorization == self.accepted;
        self.seen.lock().expect("seen requests").push(authorization);
        if accepted {
            Ok(())
        } else {
            Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                EXPIRED_CHALLENGE.to_owned(),
            )))
        }
    }
}

impl StreamableHttpClient for FakeClient {
    type Error = io::Error;

    async fn post_message(
        &self,
        _uri: Arc<str>,
        _message: ClientJsonRpcMessage,
        _session_id: Option<Arc<str>>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.check(&custom_headers)?;
        Ok(StreamableHttpPostResponse::Accepted)
    }

    async fn delete_session(
        &self,
        _uri: Arc<str>,
        _session_id: Arc<str>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.check(&custom_headers)
    }

    async fn get_stream(
        &self,
        _uri: Arc<str>,
        _session_id: Option<Arc<str>>,
        _last_event_id: Option<String>,
        _auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.check(&custom_headers)?;
        Ok(futures::stream::empty().boxed())
    }
}

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn notification() -> ClientJsonRpcMessage {
    serde_json::from_value(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .expect("client notification")
}

async fn post(
    client: &RefreshableHeaderClient<FakeClient>,
    custom_headers: HashMap<HeaderName, HeaderValue>,
) -> Result<StreamableHttpPostResponse, StreamableHttpError<io::Error>> {
    client
        .post_message(
            "http://mcp.test".into(),
            notification(),
            None,
            None,
            custom_headers,
        )
        .await
}

/// A handler that replaces the headers it is installed on with `replacement`, counting its calls
/// and keeping the challenges it was given.
struct ReplacingHandler {
    target: Arc<Mutex<Option<RefreshableHeaders>>>,
    calls: Arc<AtomicUsize>,
    challenges: Arc<Mutex<Vec<String>>>,
}

impl ReplacingHandler {
    fn install(
        initial: &[(&str, &str)],
        replacement: Option<&[(&str, &str)]>,
    ) -> (RefreshableHeaders, Self) {
        let handler = Self {
            target: Arc::default(),
            calls: Arc::default(),
            challenges: Arc::default(),
        };
        let target = handler.target.clone();
        let calls = handler.calls.clone();
        let challenges = handler.challenges.clone();
        let replacement = replacement.map(headers);
        let on_rejected: RejectionHandler = Arc::new(move |challenge| {
            calls.fetch_add(1, Ordering::SeqCst);
            challenges.lock().expect("challenges").push(challenge);
            let target = target.lock().expect("handler target").clone();
            let replacement = replacement.clone();
            async move {
                if let (Some(target), Some(replacement)) = (target, replacement) {
                    target.replace(replacement).expect("valid replacement");
                }
            }
            .boxed()
        });
        let refreshable =
            RefreshableHeaders::new(headers(initial), Some(on_rejected)).expect("valid headers");
        *handler.target.lock().expect("handler target") = Some(refreshable.clone());
        (refreshable, handler)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn current_headers_win_over_the_transports_own() {
    let http_client = FakeClient::accepting("Bearer current");
    let refreshable =
        RefreshableHeaders::new(headers(&[("Authorization", "Bearer current")]), None)
            .expect("valid headers");
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable);

    let custom_headers = HashMap::from([(
        http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer from-transport"),
    )]);
    post(&client, custom_headers)
        .await
        .expect("request should be accepted");

    assert_eq!(http_client.seen(), vec!["Bearer current"]);
}

#[tokio::test]
async fn replaced_headers_apply_to_later_requests() {
    let http_client = FakeClient::accepting("Bearer second");
    let refreshable = RefreshableHeaders::new(headers(&[("Authorization", "Bearer first")]), None)
        .expect("valid headers");
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable.clone());

    refreshable
        .replace(headers(&[("Authorization", "Bearer second")]))
        .expect("valid replacement");
    post(&client, HashMap::new())
        .await
        .expect("request should be accepted");

    assert_eq!(http_client.seen(), vec!["Bearer second"]);
}

#[tokio::test]
async fn rejected_request_is_retried_once_with_replaced_headers() {
    let http_client = FakeClient::accepting("Bearer fresh");
    let (refreshable, handler) = ReplacingHandler::install(
        &[("Authorization", "Bearer expired")],
        Some(&[("Authorization", "Bearer fresh")]),
    );
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable);

    post(&client, HashMap::new())
        .await
        .expect("retry should be accepted");

    assert_eq!(http_client.seen(), vec!["Bearer expired", "Bearer fresh"]);
    assert_eq!(handler.calls(), 1);
    assert_eq!(
        *handler.challenges.lock().expect("challenges"),
        vec![EXPIRED_CHALLENGE]
    );
}

#[tokio::test]
async fn rejection_is_returned_when_the_headers_were_not_replaced() {
    let http_client = FakeClient::accepting("Bearer fresh");
    let (refreshable, handler) =
        ReplacingHandler::install(&[("Authorization", "Bearer expired")], None);
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable);

    let result = post(&client, HashMap::new()).await;

    assert!(matches!(result, Err(StreamableHttpError::AuthRequired(_))));
    assert_eq!(http_client.seen(), vec!["Bearer expired"]);
    assert_eq!(handler.calls(), 1);
}

#[tokio::test]
async fn rejection_is_returned_without_a_handler() {
    let http_client = FakeClient::accepting("Bearer fresh");
    let refreshable =
        RefreshableHeaders::new(headers(&[("Authorization", "Bearer expired")]), None)
            .expect("valid headers");
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable);

    let result = post(&client, HashMap::new()).await;

    assert!(matches!(result, Err(StreamableHttpError::AuthRequired(_))));
    assert_eq!(http_client.seen(), vec!["Bearer expired"]);
}

#[tokio::test]
async fn a_second_rejection_is_not_retried_again() {
    let http_client = FakeClient::accepting("Bearer fresh");
    let (refreshable, handler) = ReplacingHandler::install(
        &[("Authorization", "Bearer expired")],
        Some(&[("Authorization", "Bearer also-rejected")]),
    );
    let client = RefreshableHeaderClient::new(http_client.clone(), refreshable);

    let result = post(&client, HashMap::new()).await;

    assert!(matches!(result, Err(StreamableHttpError::AuthRequired(_))));
    assert_eq!(
        http_client.seen(),
        vec!["Bearer expired", "Bearer also-rejected"]
    );
    assert_eq!(handler.calls(), 1);
}

#[tokio::test]
async fn headers_replaced_since_the_request_was_sent_are_used_without_asking() {
    let (refreshable, handler) =
        ReplacingHandler::install(&[("Authorization", "Bearer expired")], None);
    let (_, sent_generation) = refreshable.applied_to(&HashMap::new());

    refreshable
        .replace(headers(&[("Authorization", "Bearer fresh")]))
        .expect("valid replacement");

    assert!(
        refreshable
            .refreshed_after_rejection(sent_generation, EXPIRED_CHALLENGE)
            .await
    );
    assert_eq!(handler.calls(), 0);
}

#[test]
fn applies_over_and_excludes_configured_headers_of_the_same_name() {
    let refreshable =
        RefreshableHeaders::new(headers(&[("Authorization", "Bearer current")]), None)
            .expect("valid headers");
    let configured = headers(&[("authorization", "Bearer configured"), ("X-Team", "core")]);

    let (applied, generation) = refreshable.applied_to(&configured);
    assert_eq!(
        applied,
        headers(&[("Authorization", "Bearer current"), ("X-Team", "core")])
    );
    assert_eq!(generation, 0);
    assert_eq!(
        refreshable.excluded_from(&configured),
        headers(&[("X-Team", "core")])
    );
}

#[test]
fn debug_output_and_errors_omit_header_values() {
    let refreshable =
        RefreshableHeaders::new(headers(&[("Authorization", "Bearer secret-token")]), None)
            .expect("valid headers");
    assert!(!format!("{refreshable:?}").contains("secret-token"));

    let error = RefreshableHeaders::new(headers(&[("Authorization", "secret\nvalue")]), None)
        .expect_err("a value with a line break is not a valid header");
    assert!(!error.to_string().contains("secret"));
    assert!(error.to_string().contains("Authorization"));
}
