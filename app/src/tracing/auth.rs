use std::fmt;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::stream::AbortHandle;
use http::header::{HeaderValue, AUTHORIZATION};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use warp_managed_secrets::client::{IdentityTokenOptions, ManagedSecretsClient, TaskIdentityToken};
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

const CLOUD_AGENT_OTLP_TOKEN: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN";
const CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT";
const COLLECTOR_AUDIENCE: &str = "warp-cloud-agent-otel";
const REFRESHED_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);
const PROACTIVE_REFRESH_BUFFER: Duration = Duration::from_secs(20 * 60);
const PROACTIVE_REFRESH_JITTER: Duration = Duration::from_secs(2 * 60);
const INITIAL_FAILURE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(60);
const REFRESH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(super) struct AuthContext {
    token_store: TokenStore,
    refresh_hint_sender: Sender<()>,
    refresh_hint_receiver: Arc<Mutex<Option<Receiver<()>>>>,
}

impl AuthContext {
    pub(super) fn from_environment() -> anyhow::Result<Self> {
        let token = std::env::var(CLOUD_AGENT_OTLP_TOKEN)
            .context("Cloud-agent OTLP token is missing")?
            .trim()
            .to_owned();
        anyhow::ensure!(!token.is_empty(), "Cloud-agent OTLP token is empty");

        let expires_at = std::env::var(CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT)
            .context("Cloud-agent OTLP token expiry is missing")?;
        let expires_at = DateTime::parse_from_rfc3339(expires_at.trim())
            .context("Cloud-agent OTLP token expiry is not valid RFC3339")?;
        anyhow::ensure!(
            expires_at.offset().local_minus_utc() == 0,
            "Cloud-agent OTLP token expiry is not UTC"
        );
        let expires_at = expires_at.with_timezone(&Utc);
        anyhow::ensure!(
            expires_at > Utc::now(),
            "Cloud-agent OTLP token is already expired"
        );

        let token_store = TokenStore::new(token, expires_at)?;
        let (refresh_hint_sender, refresh_hint_receiver) = async_channel::bounded(1);
        Ok(Self {
            token_store,
            refresh_hint_sender,
            refresh_hint_receiver: Arc::new(Mutex::new(Some(refresh_hint_receiver))),
        })
    }

    pub(super) fn http_client(&self) -> AuthenticatedHttpClient {
        AuthenticatedHttpClient {
            inner: reqwest::blocking::Client::new(),
            token_store: self.token_store.clone(),
            refresh_hint_sender: self.refresh_hint_sender.clone(),
        }
    }

    fn take_refresh_hint_receiver(&self) -> Option<Receiver<()>> {
        self.refresh_hint_receiver
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
    }
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthContext")
            .field("token_store", &self.token_store)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct TokenStore {
    inner: Arc<RwLock<TokenSnapshot>>,
}

impl TokenStore {
    fn new(token: String, expires_at: DateTime<Utc>) -> anyhow::Result<Self> {
        Ok(Self {
            inner: Arc::new(RwLock::new(TokenSnapshot::new(token, expires_at)?)),
        })
    }

    fn valid_authorization_header(&self) -> Option<HeaderValue> {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        (snapshot.expires_at > Utc::now()).then(|| snapshot.authorization_header.clone())
    }

    fn replace(&self, token: String, expires_at: DateTime<Utc>) -> anyhow::Result<()> {
        anyhow::ensure!(
            expires_at > Utc::now(),
            "Refreshed cloud-agent OTLP token is already expired"
        );
        let snapshot = TokenSnapshot::new(token, expires_at)?;
        *self.inner.write().unwrap_or_else(|err| err.into_inner()) = snapshot;
        Ok(())
    }
}

impl fmt::Debug for TokenStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        formatter
            .debug_struct("TokenStore")
            .field("expires_at", &snapshot.expires_at)
            .finish_non_exhaustive()
    }
}

struct TokenSnapshot {
    authorization_header: HeaderValue,
    expires_at: DateTime<Utc>,
}

impl TokenSnapshot {
    fn new(token: String, expires_at: DateTime<Utc>) -> anyhow::Result<Self> {
        let mut authorization_header = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| anyhow!("Cloud-agent OTLP token cannot be used as an HTTP header"))?;
        authorization_header.set_sensitive(true);
        Ok(Self {
            authorization_header,
            expires_at,
        })
    }
}

impl fmt::Debug for TokenSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenSnapshot")
            .field("authorization_header", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(thiserror::Error, Debug)]
enum AuthenticatedHttpError {
    #[error("No unexpired cloud-agent OTLP token is available")]
    NoValidToken,
    #[error("Cloud-agent OTLP request failed with HTTP status {0}")]
    HttpStatus(u16),
}

/// Injects the latest valid token immediately before each OTLP request.
///
/// The token-store lock is released before network I/O begins. A manual `Debug` implementation
/// prevents either the cached token or a request's authorization header from being formatted.
pub(super) struct AuthenticatedHttpClient {
    inner: reqwest::blocking::Client,
    token_store: TokenStore,
    refresh_hint_sender: Sender<()>,
}

impl fmt::Debug for AuthenticatedHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedHttpClient")
            .field("token_store", &self.token_store)
            .finish_non_exhaustive()
    }
}

impl AuthenticatedHttpClient {
    fn authorize_request(
        &self,
        request: &mut Request<Bytes>,
    ) -> Result<(), AuthenticatedHttpError> {
        request.headers_mut().remove(AUTHORIZATION);
        let authorization = self
            .token_store
            .valid_authorization_header()
            .ok_or(AuthenticatedHttpError::NoValidToken)?;
        request.headers_mut().insert(AUTHORIZATION, authorization);
        Ok(())
    }
}

#[async_trait]
impl HttpClient for AuthenticatedHttpClient {
    async fn send_bytes(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        self.authorize_request(&mut request)?;

        let request: reqwest::blocking::Request = request.try_into()?;
        let mut response = self.inner.execute(request)?;
        let status = response.status();
        if status == http::StatusCode::UNAUTHORIZED {
            let _ = self.refresh_hint_sender.try_send(());
        }

        if !status.is_success() {
            return Err(AuthenticatedHttpError::HttpStatus(status.as_u16()).into());
        }

        let headers = std::mem::take(response.headers_mut());
        let mut response = Response::builder().status(status).body(response.bytes()?)?;
        *response.headers_mut() = headers;
        Ok(response)
    }
}

pub(super) fn start_refresh_coordinator(
    auth_context: AuthContext,
    client: Arc<dyn ManagedSecretsClient>,
    ctx: &mut AppContext,
) {
    let Some(refresh_hint_receiver) = auth_context.take_refresh_hint_receiver() else {
        return;
    };
    ctx.add_singleton_model(move |ctx| {
        AuthRefreshCoordinator::new(auth_context.token_store, refresh_hint_receiver, client, ctx)
    });
}

struct AuthRefreshCoordinator {
    token_store: TokenStore,
    client: Arc<dyn ManagedSecretsClient>,
    refresh_in_flight: bool,
    consecutive_failures: u32,
    scheduled_refresh: Option<AbortHandle>,
    last_failure_diagnostic: Option<std::time::Instant>,
}

impl AuthRefreshCoordinator {
    fn new(
        token_store: TokenStore,
        refresh_hint_receiver: Receiver<()>,
        client: Arc<dyn ManagedSecretsClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut coordinator = Self {
            token_store,
            client,
            refresh_in_flight: false,
            consecutive_failures: 0,
            scheduled_refresh: None,
            last_failure_diagnostic: None,
        };
        let _ = ctx.spawn_stream_local(
            refresh_hint_receiver,
            |coordinator, (), ctx| coordinator.start_refresh(ctx),
            |_, _| {},
        );
        coordinator.start_refresh(ctx);
        coordinator
    }

    fn start_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if self.refresh_in_flight {
            return;
        }
        self.cancel_scheduled_refresh();
        self.refresh_in_flight = true;
        let client = self.client.clone();
        ctx.spawn(
            async move {
                client
                    .issue_task_identity_token(IdentityTokenOptions {
                        audience: COLLECTOR_AUDIENCE.to_owned(),
                        requested_duration: REFRESHED_TOKEN_DURATION,
                        subject_template: vec1::vec1!["principal".to_owned()],
                    })
                    .with_timeout(REFRESH_REQUEST_TIMEOUT)
                    .await
                    .map_err(|_| anyhow!("Cloud-agent OTLP authorization refresh timed out"))?
            },
            |coordinator, result, ctx| coordinator.finish_refresh(result, ctx),
        );
    }

    fn finish_refresh(
        &mut self,
        result: anyhow::Result<TaskIdentityToken>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.refresh_in_flight = false;
        match result {
            Ok(token) => {
                let expires_at = token.expires_at;
                if self.token_store.replace(token.token, expires_at).is_ok() {
                    self.consecutive_failures = 0;
                    log::info!("Cloud-agent OTLP authorization refreshed");
                    self.schedule_proactive_refresh(expires_at, ctx);
                } else {
                    self.warn_refresh_failure();
                    self.schedule_failure_retry(ctx);
                }
            }
            Err(_) => {
                self.warn_refresh_failure();
                self.schedule_failure_retry(ctx);
            }
        }
    }

    fn schedule_proactive_refresh(
        &mut self,
        expires_at: DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) {
        let jitter = PROACTIVE_REFRESH_JITTER.mul_f64(rand::random::<f64>());
        let refresh_buffer = PROACTIVE_REFRESH_BUFFER.saturating_add(jitter);
        let remaining = (expires_at - Utc::now()).to_std().unwrap_or_default();
        self.schedule_refresh(remaining.saturating_sub(refresh_buffer), ctx);
    }

    fn schedule_failure_retry(&mut self, ctx: &mut ModelContext<Self>) {
        let exponent = self.consecutive_failures.min(31);
        let upper_bound = INITIAL_FAILURE_BACKOFF
            .saturating_mul(1u32 << exponent)
            .min(MAX_FAILURE_BACKOFF);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let delay = upper_bound.mul_f64(rand::random::<f64>());
        self.schedule_refresh(delay, ctx);
    }

    fn schedule_refresh(&mut self, delay: Duration, ctx: &mut ModelContext<Self>) {
        self.cancel_scheduled_refresh();
        let task = ctx.spawn(
            async move {
                Timer::after(delay).await;
            },
            |coordinator, _, ctx| {
                coordinator.scheduled_refresh = None;
                coordinator.start_refresh(ctx);
            },
        );
        self.scheduled_refresh = Some(task.abort_handle());
    }

    fn cancel_scheduled_refresh(&mut self) {
        if let Some(handle) = self.scheduled_refresh.take() {
            handle.abort();
        }
    }

    fn warn_refresh_failure(&mut self) {
        let now = std::time::Instant::now();
        if self
            .last_failure_diagnostic
            .is_none_or(|last| now.duration_since(last) >= FAILURE_LOG_INTERVAL)
        {
            self.last_failure_diagnostic = Some(now);
            log::warn!("Cloud-agent OTLP authorization refresh failed");
        }
    }
}

impl Entity for AuthRefreshCoordinator {
    type Event = ();
}

impl SingletonEntity for AuthRefreshCoordinator {}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    fn client_with_expiry(token: &str, expires_at: DateTime<Utc>) -> AuthenticatedHttpClient {
        let (refresh_hint_sender, _) = async_channel::bounded(1);
        AuthenticatedHttpClient {
            inner: reqwest::blocking::Client::new(),
            token_store: TokenStore::new(token.to_owned(), expires_at).unwrap(),
            refresh_hint_sender,
        }
    }

    #[test]
    fn authorization_overwrites_supplied_header() {
        let client = client_with_expiry(
            "current-test-token",
            Utc::now() + TimeDelta::try_minutes(5).unwrap(),
        );
        let mut request = Request::builder()
            .header(AUTHORIZATION, "Bearer stale-test-token")
            .body(Bytes::new())
            .unwrap();

        client.authorize_request(&mut request).unwrap();

        assert_eq!(
            request.headers().get(AUTHORIZATION).unwrap(),
            "Bearer current-test-token"
        );
    }

    #[test]
    fn expired_token_is_refused_and_supplied_header_is_removed() {
        let client = client_with_expiry(
            "expired-test-token",
            Utc::now() - TimeDelta::try_minutes(5).unwrap(),
        );
        let mut request = Request::builder()
            .header(AUTHORIZATION, "Bearer stale-test-token")
            .body(Bytes::new())
            .unwrap();

        assert!(matches!(
            client.authorize_request(&mut request),
            Err(AuthenticatedHttpError::NoValidToken)
        ));
        assert!(!request.headers().contains_key(AUTHORIZATION));
    }

    #[test]
    fn debug_output_redacts_token() {
        let client = client_with_expiry(
            "secret-test-token",
            Utc::now() + TimeDelta::try_minutes(5).unwrap(),
        );

        let debug_output = format!("{client:?}");

        assert!(!debug_output.contains("secret-test-token"));
        assert!(debug_output.contains("expires_at"));
    }

    #[test]
    fn authorized_request_debug_redacts_token() {
        let client = client_with_expiry(
            "secret-request-test-token",
            Utc::now() + TimeDelta::try_minutes(5).unwrap(),
        );
        let mut request = Request::builder().body(Bytes::new()).unwrap();

        client.authorize_request(&mut request).unwrap();
        let request_debug = format!("{request:?}");
        let headers_debug = format!("{:?}", request.headers());

        assert!(!request_debug.contains("secret-request-test-token"));
        assert!(!headers_debug.contains("secret-request-test-token"));
        assert!(request_debug.contains("Sensitive"));
        assert!(headers_debug.contains("Sensitive"));
    }
}
