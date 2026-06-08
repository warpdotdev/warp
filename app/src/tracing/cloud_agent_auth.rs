//! Provides authenticated OTLP trace transport and credential refresh for opted-in cloud agents.
//!
//! Dispatch bootstraps tracing with a bearer token and expiry in the process environment. The
//! exporter is built once around [`AuthenticatedHttpClient`], which reads a shared token snapshot
//! immediately before every request so refresh never requires rebuilding the exporter. Processes
//! without the endpoint switch or a currently valid dispatch credential never initialize this
//! module.
//!
//! Refresh begins only after the application has an authenticated managed-secrets client. A
//! successful mint replaces the dispatch credential only after the returned JWT's unverified
//! payload contains a string `run_id` exactly matching the immutable startup `OZ_RUN_ID`. This
//! payload inspection is only a rejection gate; the collector remains responsible for verifying
//! the token's signature, audience, expiry, and trusted trace resource attributes. Every refresh
//! failure preserves the last valid credential and enters bounded jittered backoff.
//!
//! Tokens must never appear in diagnostics or formatted values. Cached authorization headers are
//! marked sensitive, manual `Debug` implementations omit secrets, and token-store locks are always
//! released before network I/O.
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use futures_util::stream::AbortHandle;
use http::header::{HeaderValue, AUTHORIZATION};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use warp_managed_secrets::client::{IdentityTokenOptions, ManagedSecretsClient, TaskIdentityToken};
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

/// The environment variables form the immutable dispatch-time authentication bootstrap.
const CLOUD_AGENT_OTLP_TOKEN: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN";
const CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT";
const OZ_RUN_ID: &str = "OZ_RUN_ID";
/// The collector audience and requested lifetime are fixed by the cloud-agent trace contract.
const COLLECTOR_AUDIENCE: &str = "warp-cloud-agent-otel";
const REFRESHED_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);
/// Proactive refresh starts roughly twenty minutes before expiry, with jitter to spread load.
const PROACTIVE_REFRESH_BUFFER: Duration = Duration::from_secs(20 * 60);
const PROACTIVE_REFRESH_JITTER: Duration = Duration::from_secs(2 * 60);
/// Failed refreshes use bounded full-jitter exponential backoff and rate-limited diagnostics.
const INITIAL_FAILURE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(60);
/// A stalled identity-token request must release the single in-flight refresh slot.
const REFRESH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Shares dispatch authentication state between the exporter and the later refresh coordinator.
///
/// The optional expected run ID intentionally does not gate initial tracing: a valid dispatch
/// credential remains usable when `OZ_RUN_ID` is missing or empty, but every refreshed credential
/// is rejected until an immutable expected run ID is available to the replacement gate.

#[derive(Clone)]
pub(super) struct AuthContext {
    token_store: TokenStore,
    expected_run_id: Option<Arc<str>>,
    refresh_hint_sender: Sender<()>,
    refresh_hint_receiver: Arc<Mutex<Option<Receiver<()>>>>,
}

impl AuthContext {
    /// Seeds authentication from a currently valid dispatch credential in the environment.
    ///
    /// The caller treats failure as an opt-out so normal processes and partially rolled-out cloud
    /// agents retain no-op tracing behavior.
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
        let expected_run_id = std::env::var(OZ_RUN_ID)
            .ok()
            .filter(|run_id| !run_id.trim().is_empty());

        let token_store = TokenStore::new(token, expires_at)?;
        let (refresh_hint_sender, refresh_hint_receiver) = async_channel::bounded(1);
        Ok(Self {
            token_store,
            expected_run_id: expected_run_id.map(Into::into),
            refresh_hint_sender,
            refresh_hint_receiver: Arc::new(Mutex::new(Some(refresh_hint_receiver))),
        })
    }

    /// Creates a transport sharing the latest credential while leaving the exporter itself stable.
    pub(super) fn http_client(&self) -> AuthenticatedHttpClient {
        AuthenticatedHttpClient {
            inner: reqwest::blocking::Client::new(),
            token_store: self.token_store.clone(),
            refresh_hint_sender: self.refresh_hint_sender.clone(),
        }
    }

    /// Transfers the bounded refresh-hint receiver to the one allowed coordinator.
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

/// Stores the latest credential snapshot behind a short-lived reader/writer lock.
///
/// Readers clone only the sensitive authorization header, and no caller holds this lock during
/// network I/O. Replacement constructs and validates a complete snapshot before taking the write
/// lock so failures preserve the last valid credential.
#[derive(Clone)]
struct TokenStore {
    inner: Arc<RwLock<TokenSnapshot>>,
}

impl TokenStore {
    /// Creates the initial store from the validated dispatch credential.
    fn new(token: String, expires_at: DateTime<Utc>) -> anyhow::Result<Self> {
        Ok(Self {
            inner: Arc::new(RwLock::new(TokenSnapshot::new(token, expires_at)?)),
        })
    }

    /// Returns a cloned sensitive header only while the current credential remains unexpired.
    fn valid_authorization_header(&self) -> Option<HeaderValue> {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        (snapshot.expires_at > Utc::now()).then(|| snapshot.authorization_header.clone())
    }

    /// Atomically replaces the current snapshot only with a usable unexpired credential.
    fn replace(&self, token: String, expires_at: DateTime<Utc>) -> anyhow::Result<()> {
        anyhow::ensure!(
            expires_at > Utc::now(),
            "Refreshed cloud-agent OTLP token is already expired"
        );
        let snapshot = TokenSnapshot::new(token, expires_at)?;
        *self.inner.write().unwrap_or_else(|err| err.into_inner()) = snapshot;
        Ok(())
    }

    /// Applies the exact-run rejection gate before allowing a refreshed credential to replace the
    /// dispatch or previous refresh credential.
    fn replace_refreshed(
        &self,
        token: String,
        expires_at: DateTime<Utc>,
        expected_run_id: Option<&str>,
    ) -> anyhow::Result<()> {
        validate_refreshed_token_run_id(&token, expected_run_id)?;
        self.replace(token, expires_at)
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

/// Caches the already-parsed sensitive authorization header with its trusted server expiry.
struct TokenSnapshot {
    authorization_header: HeaderValue,
    expires_at: DateTime<Utc>,
}

impl TokenSnapshot {
    /// Constructs a snapshot whose header redacts its value from standard debug formatting.
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

/// Requires the unverified refreshed-token payload to name the immutable expected run exactly.
///
/// This local decode never establishes token authenticity. The collector remains responsible for
/// cryptographically verifying the token, while malformed or mismatched tokens fail closed here
/// before replacement and leave the existing credential untouched.
fn validate_refreshed_token_run_id(
    token: &str,
    expected_run_id: Option<&str>,
) -> anyhow::Result<()> {
    let expected_run_id = expected_run_id
        .filter(|run_id| !run_id.trim().is_empty())
        .context("Expected cloud-agent run ID is missing or empty")?;

    let mut segments = token.split('.');
    let _header = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .context("Refreshed cloud-agent OTLP token is not a valid JWT")?;
    let payload = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .context("Refreshed cloud-agent OTLP token is not a valid JWT")?;
    let _signature = segments
        .next()
        .filter(|segment| !segment.is_empty())
        .context("Refreshed cloud-agent OTLP token is not a valid JWT")?;
    anyhow::ensure!(
        segments.next().is_none(),
        "Refreshed cloud-agent OTLP token is not a valid JWT"
    );
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| anyhow!("Refreshed cloud-agent OTLP token payload is not valid base64"))?;
    let payload: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|_| anyhow!("Refreshed cloud-agent OTLP token payload is not valid JSON"))?;
    let run_id = payload
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .context("Refreshed cloud-agent OTLP token has no string run ID")?;
    anyhow::ensure!(
        run_id == expected_run_id,
        "Refreshed cloud-agent OTLP token run ID does not match"
    );
    Ok(())
}

/// Reports only token-free transport failure categories to the exporter.
#[derive(thiserror::Error, Debug)]
enum AuthenticatedHttpError {
    #[error("No unexpired cloud-agent OTLP token is available")]
    NoValidToken,
    #[error("Cloud-agent OTLP request failed with HTTP status {0}")]
    HttpStatus(u16),
}

/// Injects the latest valid token immediately before each request from the exporter built at
/// startup.
///
/// The token-store lock is released before network I/O begins. A manual `Debug` implementation
/// prevents the client from formatting cached state, while sensitive [`HeaderValue`] instances
/// redact request headers. Expired credentials are removed and refused rather than sent.
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
    /// Overwrites any supplied authorization header with the latest unexpired credential.
    ///
    /// Removing the supplied header first ensures an expired store fails closed rather than
    /// accidentally sending a stale or caller-provided credential.
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
            // The bounded nonblocking hint cannot recurse into or delay this export request.
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

/// Starts the one refresh coordinator after authenticated server connectivity is available.
///
/// Consuming the bounded hint receiver coalesces concurrent starts, and the coordinator immediately
/// mints once so the short-lived dispatch credential is replaced as soon as possible.
pub(super) fn start_refresh_coordinator(
    auth_context: AuthContext,
    client: Arc<dyn ManagedSecretsClient>,
    ctx: &mut AppContext,
) {
    let Some(refresh_hint_receiver) = auth_context.take_refresh_hint_receiver() else {
        return;
    };
    ctx.add_singleton_model(move |ctx| {
        AuthRefreshCoordinator::new(
            auth_context.token_store,
            auth_context.expected_run_id,
            refresh_hint_receiver,
            client,
            ctx,
        )
    });
}

/// Owns serialized credential minting, proactive scheduling, failure backoff, and diagnostics.
///
/// At most one mint is in flight and one scheduled wakeup is retained. A bounded nonblocking 401
/// hint can accelerate refresh without recursing into or blocking the export request.
struct AuthRefreshCoordinator {
    token_store: TokenStore,
    expected_run_id: Option<Arc<str>>,
    client: Arc<dyn ManagedSecretsClient>,
    refresh_in_flight: bool,
    consecutive_failures: u32,
    scheduled_refresh: Option<AbortHandle>,
    last_failure_diagnostic: Option<std::time::Instant>,
}

impl AuthRefreshCoordinator {
    /// Installs the hint stream and immediately starts the first bounded refresh request.
    fn new(
        token_store: TokenStore,
        expected_run_id: Option<Arc<str>>,
        refresh_hint_receiver: Receiver<()>,
        client: Arc<dyn ManagedSecretsClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut coordinator = Self {
            token_store,
            expected_run_id,
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

    /// Starts one mint and coalesces all triggers while it remains in flight.
    ///
    /// Each request asks for the fixed collector audience and principal-only subject, and the
    /// timeout guarantees a stalled request eventually enters the ordinary failure path.
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

    /// Accepts a refreshed credential only after all replacement gates succeed.
    ///
    /// Any mint, timeout, expiry, header, or run-ID failure retains the last valid token and enters
    /// the same bounded retry path without logging token contents.
    fn finish_refresh(
        &mut self,
        result: anyhow::Result<TaskIdentityToken>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.refresh_in_flight = false;
        match result {
            Ok(token) => {
                let expires_at = token.expires_at;
                if self
                    .token_store
                    .replace_refreshed(token.token, expires_at, self.expected_run_id.as_deref())
                    .is_ok()
                {
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

    /// Schedules successful refreshes around twenty minutes before server-returned expiry.
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

    /// Schedules a full-jitter exponential retry capped at five minutes.
    fn schedule_failure_retry(&mut self, ctx: &mut ModelContext<Self>) {
        let exponent = self.consecutive_failures.min(31);
        let upper_bound = INITIAL_FAILURE_BACKOFF
            .saturating_mul(1u32 << exponent)
            .min(MAX_FAILURE_BACKOFF);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let delay = upper_bound.mul_f64(rand::random::<f64>());
        self.schedule_refresh(delay, ctx);
    }

    /// Replaces the one scheduled wakeup so proactive, retry, and hint triggers stay coalesced.
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

    /// Cancels the prior wakeup without affecting a refresh already in flight.
    fn cancel_scheduled_refresh(&mut self) {
        if let Some(handle) = self.scheduled_refresh.take() {
            handle.abort();
        }
    }

    /// Emits a local token-free failure diagnostic at most once per configured interval.
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
#[path = "cloud_agent_auth_tests.rs"]
mod tests;
