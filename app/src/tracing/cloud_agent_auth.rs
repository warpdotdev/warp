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
//! payload contains the expected run or standby sandbox identity. This payload inspection is only a
//! rejection gate; the collector remains responsible for verifying the token's signature, audience,
//! expiry, and trusted trace resource attributes. Every refresh failure preserves the last valid
//! credential and enters bounded jittered backoff.
//!
//! Tokens must never appear in diagnostics or formatted values. Cached authorization headers are
//! marked sensitive, manual `Debug` implementations omit secrets, and token-store locks are always
//! released before network I/O.
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use async_channel::{Receiver, Sender};
use async_compat::Compat;
use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use futures_util::stream::AbortHandle;
use http::header::{AUTHORIZATION, HeaderValue};
use instant::Instant;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::SdkTracerProvider;
use serde::{Deserialize, Serialize};
use warp_managed_secrets::client::{IdentityTokenOptions, TaskIdentityToken};
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::server::server_api::managed_secrets::AppManagedSecretsClient;

/// The environment variables form the immutable dispatch-time authentication bootstrap.
pub(super) const CLOUD_AGENT_OTLP_TOKEN: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN";
pub(super) const CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT: &str = "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT";
const OZ_RUN_ID: &str = "OZ_RUN_ID";
const PREWARMED_SANDBOX_ID: &str = "WARP_PREWARMED_SANDBOX_ID";
/// The collector audience and requested lifetime are fixed by the cloud-agent trace contract.
const COLLECTOR_AUDIENCE: &str = "warp-cloud-agent-otel";
const REFRESHED_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);
/// Proactive refresh starts roughly twenty minutes before expiry, with jitter to spread load.
const PROACTIVE_REFRESH_BUFFER: Duration = Duration::from_secs(20 * 60);
const PROACTIVE_REFRESH_JITTER: Duration = Duration::from_secs(2 * 60);
const MIN_PROACTIVE_REFRESH_DELAY: Duration = Duration::from_secs(1);
/// Failed refreshes use bounded full-jitter exponential backoff and rate-limited diagnostics.
const INITIAL_FAILURE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(5 * 60);
const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(60);
/// A stalled identity-token request must release the single in-flight refresh slot.
const REFRESH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared dispatch authentication state between the exporter and the later refresh coordinator.
///
/// A missing expected identity does not gate initial tracing, but prevents credential refresh.
#[derive(Clone)]
pub(super) struct AuthContext {
    token_store: TokenStore,
    expected_identity: Arc<RwLock<Option<ExpectedIdentity>>>,
    refresh_hint_sender: Sender<()>,
    refresh_hint_receiver: Arc<Mutex<Option<Receiver<()>>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ExpectedIdentity {
    Standby(String),
    Run(String),
}

impl ExpectedIdentity {
    fn from_ids(run_id: Option<String>, sandbox_id: Option<String>) -> Option<Self> {
        run_id
            .filter(|id| !id.trim().is_empty())
            .map(Self::Run)
            .or_else(|| {
                sandbox_id
                    .filter(|id| !id.trim().is_empty())
                    .map(Self::Standby)
            })
    }

    fn claim(&self) -> (&str, &str) {
        match self {
            Self::Standby(id) => ("sandbox_id", id),
            Self::Run(id) => ("run_id", id),
        }
    }
}

impl AuthContext {
    /// Seeds authentication from a currently valid dispatch credential in the environment.
    ///
    /// The caller treats failure as an opt-out so normal processes and partially rolled-out cloud
    /// agents retain no-op tracing behavior.
    pub(super) fn from_environment() -> anyhow::Result<Self> {
        let token =
            std::env::var(CLOUD_AGENT_OTLP_TOKEN).context("Cloud-agent OTLP token is missing")?;
        // Remove the bootstrap secret as soon as it is owned so child processes cannot inherit it.
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var(CLOUD_AGENT_OTLP_TOKEN) };
        let token = token.trim().to_owned();
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
        let expected_identity = ExpectedIdentity::from_ids(
            std::env::var(OZ_RUN_ID).ok(),
            std::env::var(PREWARMED_SANDBOX_ID).ok(),
        );

        let context = Self::from_snapshot(CredentialSnapshot { token, expires_at })?;
        *context
            .expected_identity
            .write()
            .unwrap_or_else(|err| err.into_inner()) = expected_identity;
        Ok(context)
    }

    pub(super) fn from_snapshot(snapshot: CredentialSnapshot) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !snapshot.token.trim().is_empty(),
            "Cloud-agent OTLP token is empty"
        );
        anyhow::ensure!(
            snapshot.expires_at > Utc::now(),
            "Cloud-agent OTLP token is already expired"
        );
        let token_store = TokenStore::new(snapshot.token, snapshot.expires_at)?;
        let (refresh_hint_sender, refresh_hint_receiver) = async_channel::bounded(1);
        Ok(Self {
            token_store,
            expected_identity: Arc::new(RwLock::new(None)),
            refresh_hint_sender,
            refresh_hint_receiver: Arc::new(Mutex::new(Some(refresh_hint_receiver))),
        })
    }

    pub(super) fn snapshot(&self) -> Option<CredentialSnapshot> {
        self.token_store.valid_snapshot()
    }

    pub(super) fn assign_run(
        &self,
        credential: AssignmentCredential,
        provider: &SdkTracerProvider,
    ) -> anyhow::Result<()> {
        let identity = ExpectedIdentity::Run(credential.run_id);
        validate_refreshed_token_identity(&credential.snapshot.token, Some(&identity))?;
        anyhow::ensure!(
            credential.snapshot.expires_at > Utc::now(),
            "Assigned OTLP token is expired"
        );
        let snapshot =
            TokenSnapshot::new(credential.snapshot.token, credential.snapshot.expires_at)?;
        // Flush preparation spans under the standby identity without making export a prerequisite
        // for assigning the run.
        if provider.force_flush().is_err() {
            log::warn!(
                "Failed to flush standby traces before assigning cloud-agent OTLP credentials"
            );
        }
        let mut expected = self
            .expected_identity
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *self
            .token_store
            .inner
            .write()
            .unwrap_or_else(|err| err.into_inner()) = Some(snapshot);
        *expected = Some(identity);
        Ok(())
    }

    pub(super) fn disable(&self) {
        let mut expected = self
            .expected_identity
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *self
            .token_store
            .inner
            .write()
            .unwrap_or_else(|err| err.into_inner()) = None;
        *expected = None;
    }

    /// Creates a transport sharing the latest credential while leaving the exporter itself stable.
    pub(super) fn http_client(&self) -> AuthenticatedHttpClient {
        AuthenticatedHttpClient {
            inner: reqwest::Client::new(),
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

pub(super) struct AssignmentCredential {
    snapshot: CredentialSnapshot,
    run_id: String,
}

impl AssignmentCredential {
    pub(super) fn take(runtime_env: &mut HashMap<String, String>) -> anyhow::Result<Self> {
        let token = runtime_env.remove(CLOUD_AGENT_OTLP_TOKEN);
        let expires_at = runtime_env.remove(CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT);
        let token = token.context("Assigned cloud-agent OTLP token is missing")?;
        let expires_at = expires_at.context("Assigned cloud-agent OTLP expiry is missing")?;
        let expires_at = DateTime::parse_from_rfc3339(&expires_at)
            .map_err(|_| anyhow!("Assigned cloud-agent OTLP expiry is invalid"))?;
        anyhow::ensure!(
            expires_at.offset().local_minus_utc() == 0,
            "Assigned OTLP expiry is not UTC"
        );
        let run_id = runtime_env
            .get(OZ_RUN_ID)
            .filter(|id| !id.trim().is_empty())
            .context("Assigned cloud-agent run ID is missing")?
            .clone();
        Ok(Self {
            snapshot: CredentialSnapshot {
                token,
                expires_at: expires_at.with_timezone(&Utc),
            },
            run_id,
        })
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

/// A snapshot of the latest credential, stored behind a short-lived reader/writer lock.
///
/// Readers copy the credential or sensitive authorization header without holding the lock during
/// network I/O. Replacement validates a complete snapshot before taking the write lock so failures
/// preserve the last valid credential.
#[derive(Clone)]
struct TokenStore {
    inner: Arc<RwLock<Option<TokenSnapshot>>>,
}

impl TokenStore {
    /// Creates the initial store from the validated dispatch credential.
    fn new(token: String, expires_at: DateTime<Utc>) -> anyhow::Result<Self> {
        Ok(Self {
            inner: Arc::new(RwLock::new(Some(TokenSnapshot::new(token, expires_at)?))),
        })
    }

    /// Returns a cloned sensitive header only while the current credential remains unexpired.
    fn valid_authorization_header(&self) -> Option<HeaderValue> {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        let snapshot = snapshot.as_ref()?;
        (snapshot.expires_at > Utc::now()).then(|| snapshot.authorization_header.clone())
    }

    fn valid_snapshot(&self) -> Option<CredentialSnapshot> {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        let snapshot = snapshot.as_ref()?;
        if snapshot.expires_at <= Utc::now() {
            return None;
        }
        Some(CredentialSnapshot {
            token: snapshot
                .authorization_header
                .to_str()
                .ok()?
                .strip_prefix("Bearer ")?
                .to_owned(),
            expires_at: snapshot.expires_at,
        })
    }

    /// Atomically replaces the current snapshot only with a usable unexpired credential.
    fn replace(&self, token: String, expires_at: DateTime<Utc>) -> anyhow::Result<()> {
        anyhow::ensure!(
            expires_at > Utc::now(),
            "Refreshed cloud-agent OTLP token is already expired"
        );
        let snapshot = TokenSnapshot::new(token, expires_at)?;
        let mut current = self.inner.write().unwrap_or_else(|err| err.into_inner());
        anyhow::ensure!(current.is_some(), "Cloud-agent OTLP export is disabled");
        *current = Some(snapshot);
        Ok(())
    }

    /// Applies the exact-identity rejection gate before replacing the current credential.
    fn replace_refreshed(
        &self,
        token: String,
        expires_at: DateTime<Utc>,
        expected_identity: Option<&ExpectedIdentity>,
    ) -> anyhow::Result<()> {
        validate_refreshed_token_identity(&token, expected_identity)?;
        self.replace(token, expires_at)
    }
}

impl fmt::Debug for TokenStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.inner.read().unwrap_or_else(|err| err.into_inner());
        formatter
            .debug_struct("TokenStore")
            .field(
                "expires_at",
                &snapshot.as_ref().map(|snapshot| snapshot.expires_at),
            )
            .finish_non_exhaustive()
    }
}

/// An already-parsed sensitive authorization header and its trusted server expiry.
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

/// A one-time credential copy with its original expiry, never a renewed lifetime.
#[derive(Serialize, Deserialize)]
pub(super) struct CredentialSnapshot {
    token: String,
    expires_at: DateTime<Utc>,
}

impl fmt::Debug for CredentialSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialSnapshot")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Validates that the unverified refreshed-token payload names the expected identity exactly.
///
/// This local decode never establishes token authenticity. The collector remains responsible for
/// cryptographically verifying the token, while malformed or mismatched tokens fail closed here
/// before replacement and leave the existing credential untouched.
fn validate_refreshed_token_identity(
    token: &str,
    expected_identity: Option<&ExpectedIdentity>,
) -> anyhow::Result<()> {
    let (claim, expected_id) = expected_identity
        .context("Expected cloud-agent identity is missing")?
        .claim();
    anyhow::ensure!(
        !expected_id.trim().is_empty(),
        "Expected cloud-agent identity is empty"
    );

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
    let identity = payload
        .get(claim)
        .and_then(serde_json::Value::as_str)
        .context("Refreshed cloud-agent OTLP token has no string identity")?;
    anyhow::ensure!(
        identity == expected_id,
        "Refreshed cloud-agent OTLP token identity does not match"
    );
    Ok(())
}

/// The set of errors that can occur when making an HTTP request using [`AuthenticatedHttpClient`].
#[derive(thiserror::Error, Debug)]
enum AuthenticatedHttpError {
    #[error("No unexpired cloud-agent OTLP token is available")]
    NoValidToken,
    #[error("Cloud-agent OTLP request failed with HTTP status {0}")]
    HttpStatus(u16),
}

/// An HTTP client that injects the latest valid token immediately before each request.
///
/// The token-store lock is released before network I/O begins. A manual `Debug` implementation
/// prevents the client from formatting cached state, while sensitive [`HeaderValue`] instances
/// redact request headers. Expired credentials are removed and refused rather than sent.
pub(super) struct AuthenticatedHttpClient {
    inner: reqwest::Client,
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

        let request: reqwest::Request = request.try_into()?;
        // Reqwest requires a Tokio-compatible context, while the exporter may use another executor.
        let (status, response) = Compat::new(async {
            let mut response = self.inner.execute(request).await?;
            let status = response.status();
            let response = if status.is_success() {
                let headers = std::mem::take(response.headers_mut());
                Some((headers, response.bytes().await?))
            } else {
                None
            };
            Ok::<_, reqwest::Error>((status, response))
        })
        .await?;
        if status == http::StatusCode::UNAUTHORIZED {
            // The bounded nonblocking hint cannot recurse into or delay this export request.
            let _ = self.refresh_hint_sender.try_send(());
        }
        let Some((headers, body)) = response else {
            return Err(AuthenticatedHttpError::HttpStatus(status.as_u16()).into());
        };

        let mut response = Response::builder().status(status).body(body)?;
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
    client: Arc<AppManagedSecretsClient>,
    ctx: &mut AppContext,
) {
    let Some(refresh_hint_receiver) = auth_context.take_refresh_hint_receiver() else {
        return;
    };
    ctx.add_singleton_model(move |ctx| {
        AuthRefreshCoordinator::new(
            auth_context.token_store,
            auth_context.expected_identity,
            refresh_hint_receiver,
            client,
            ctx,
        )
    });
}

pub(super) fn restart_refresh_coordinator(
    auth_context: AuthContext,
    client: Arc<AppManagedSecretsClient>,
    ctx: &mut AppContext,
) {
    if auth_context
        .refresh_hint_receiver
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .is_some()
    {
        start_refresh_coordinator(auth_context, client, ctx);
        return;
    }
    AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
        coordinator.reset(client, ctx);
    });
}

pub(super) fn stop_refresh_coordinator(auth_context: &AuthContext, ctx: &mut AppContext) {
    if auth_context
        .refresh_hint_receiver
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .is_none()
    {
        AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, _| {
            coordinator.stop();
        });
    }
}

/// Owns serialized credential minting, proactive scheduling, failure backoff, and diagnostics.
///
/// At most one mint is in flight and one scheduled wakeup is retained. A bounded nonblocking 401
/// hint can accelerate refresh without recursing into or blocking the export request.
struct AuthRefreshCoordinator {
    token_store: TokenStore,
    expected_identity: Arc<RwLock<Option<ExpectedIdentity>>>,
    client: Arc<AppManagedSecretsClient>,
    consecutive_failures: u32,
    scheduled_refresh: Option<AbortHandle>,
    last_failure_diagnostic: Option<Instant>,
    refresh_request: Option<AbortHandle>,
    generation: u64,
}

impl AuthRefreshCoordinator {
    /// Installs the hint stream and immediately starts the first bounded refresh request.
    fn new(
        token_store: TokenStore,
        expected_identity: Arc<RwLock<Option<ExpectedIdentity>>>,
        refresh_hint_receiver: Receiver<()>,
        client: Arc<AppManagedSecretsClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut coordinator = Self {
            token_store,
            expected_identity,
            client,
            consecutive_failures: 0,
            scheduled_refresh: None,
            last_failure_diagnostic: None,
            refresh_request: None,
            generation: 0,
        };
        let _ = ctx.spawn_stream_local(
            refresh_hint_receiver,
            |coordinator, (), ctx| coordinator.start_refresh(ctx),
            |_, _| {},
        );
        coordinator.start_refresh(ctx);
        coordinator
    }

    fn reset(&mut self, client: Arc<AppManagedSecretsClient>, ctx: &mut ModelContext<Self>) {
        self.stop();
        self.client = client;
        self.start_refresh(ctx);
    }

    fn stop(&mut self) {
        self.cancel_scheduled_refresh();
        if let Some(refresh) = self.refresh_request.take() {
            refresh.abort();
        }
        self.generation += 1;
        self.consecutive_failures = 0;
        self.last_failure_diagnostic = None;
    }

    /// Starts one mint and coalesces all triggers while it remains in flight.
    ///
    /// Each request asks for the fixed collector audience and principal-only subject, and the
    /// timeout guarantees a stalled request eventually enters the ordinary failure path.
    fn start_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if self.refresh_request.is_some() {
            return;
        }
        if self
            .token_store
            .inner
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .is_none()
        {
            return;
        }
        self.cancel_scheduled_refresh();
        let client = self.client.clone();
        let generation = self.generation;
        let task = ctx.spawn(
            async move {
                client
                    .issue_task_identity_token(
                        None,
                        IdentityTokenOptions {
                            audience: COLLECTOR_AUDIENCE.to_owned(),
                            requested_duration: REFRESHED_TOKEN_DURATION,
                            subject_template: vec1::vec1!["principal".to_owned()],
                        },
                    )
                    .with_timeout(REFRESH_REQUEST_TIMEOUT)
                    .await
                    .map_err(|_| anyhow!("Cloud-agent OTLP authorization refresh timed out"))?
            },
            move |coordinator, result, ctx| {
                coordinator.finish_refresh(generation, result, ctx);
            },
        );
        self.refresh_request = Some(task.abort_handle());
    }

    /// Accepts a refreshed credential only after all replacement gates succeed.
    ///
    /// Any mint, timeout, expiry, header, or identity failure retains the last valid token and enters
    /// the same bounded retry path without logging token contents.
    fn finish_refresh(
        &mut self,
        generation: u64,
        result: anyhow::Result<TaskIdentityToken>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        self.refresh_request = None;
        match result {
            Ok(token) => {
                let expires_at = token.expires_at;
                let replacement = self.token_store.replace_refreshed(
                    token.token,
                    expires_at,
                    self.expected_identity
                        .read()
                        .unwrap_or_else(|err| err.into_inner())
                        .as_ref(),
                );
                if replacement.is_ok() {
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

    /// Schedules a refresh to occur before the current token expires.
    ///
    /// This leaves some buffer for retries in case the refresh fails, but also guarantees
    /// some minimum amount of time before the first refresh attempt.
    fn schedule_proactive_refresh(
        &mut self,
        expires_at: DateTime<Utc>,
        ctx: &mut ModelContext<Self>,
    ) {
        let jitter = PROACTIVE_REFRESH_JITTER.mul_f64(rand::random::<f64>());
        let refresh_buffer = PROACTIVE_REFRESH_BUFFER.saturating_add(jitter);
        let remaining = (expires_at - Utc::now()).to_std().unwrap_or_default();
        let delay = remaining
            .saturating_sub(refresh_buffer)
            .max(remaining.mul_f64(0.5))
            .max(MIN_PROACTIVE_REFRESH_DELAY);
        self.schedule_refresh(delay, ctx);
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
        let now = Instant::now();
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
