use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use base64::Engine;
use blocking::unblock;
use instant::Instant;
use serde::{Deserialize, Serialize};
use warp_core::channel::IapConfig;
use warpui_core::r#async::{BoxFuture, FutureExt as _, Timer};
use warpui_core::{AppContext, Entity, ModelContext, SingletonEntity};
#[cfg(not(target_family = "wasm"))]
use websocket::connect_error_http_response;

const PROACTIVE_REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);

const BASE_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
/// Maximum number of consecutive failed fetches to automatically retry
/// before giving up and waiting for a manual Refresh or an inbound
/// IAP challenge. i.e. so a persistently broken setup (no gcloud,
/// bad credentials) doesn't loop forever.
const MAX_FAILURE_RETRIES: u32 = 5;

/// Upper bound on establishing IAP access for a blocking caller (e.g. a one-shot
/// CLI command) before giving up, rather than hanging or firing a doomed (401)
/// warp-server request. Generous enough to cover a couple of STS/IAM round trips
/// plus a retry.
const IAP_ACCESS_TIMEOUT: Duration = Duration::from_secs(60);

// Endpoints and constants for the runner-context Workload Identity Federation
// mint (GCP STS token exchange + IAM Credentials `generateIdToken`).
const STS_TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";
const IAM_GENERATE_ID_TOKEN_URL: &str = "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa_email}:generateIdToken";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const SUBJECT_TOKEN_TYPE_ID_TOKEN: &str = "urn:ietf:params:oauth:token-type:id_token";
const REQUESTED_TOKEN_TYPE_ACCESS_TOKEN: &str = "urn:ietf:params:oauth:token-type:access_token";
const WIF_IDENTITY_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);
const WIF_MINT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Env var carrying a Warp OIDC task-identity JWT (audience = the WIF provider
/// resource name), injected by warp-server so a cold sandboxed runner can
/// bootstrap its first IAP mint without calling the IAP-gated identity-token
/// endpoint.
const STAGING_IAP_BOOTSTRAP_TOKEN_ENV_VAR: &str = "WARP_STAGING_IAP_BOOTSTRAP_JWT";

pub type PathResolver = Box<dyn Fn(&mut AppContext) -> BoxFuture<'static, Option<String>>>;

/// Mints a Warp-signed OIDC identity token for a given audience. Implemented in
/// the `app` crate over the managed-secrets client so this crate need not depend
/// on the managed-secrets stack.
pub trait IapIdentityTokenMinter: Send + Sync + 'static {
    fn mint_identity_token(
        &self,
        audience: String,
        requested_duration: Duration,
    ) -> BoxFuture<'static, Result<String>>;
}

/// Lets a sandboxed Oz runner self-mint IAP tokens via Workload Identity
/// Federation. Present only in runner context.
#[derive(Clone)]
pub struct ManagedIapMint {
    minter: Arc<dyn IapIdentityTokenMinter>,
}

impl ManagedIapMint {
    pub fn new(minter: Arc<dyn IapIdentityTokenMinter>) -> Self {
        Self { minter }
    }
}

#[derive(Debug, Clone)]
pub struct CachedToken {
    pub token: String,
    pub expires_at: Instant,
}

impl CachedToken {
    fn valid_token(&self) -> Option<String> {
        (self.expires_at > Instant::now()).then(|| self.token.clone())
    }
}

#[derive(Debug, Clone)]
pub enum IapCredentialsState {
    Missing,
    /// A credential fetch is in progress. `previous` carries the last
    /// successfully-loaded token (if any). Allows us to attach it to
    /// outbound requests while we're refreshing so that proactive refreshes
    /// (i.e. refresh the token 5min before exp) don't prevent active requests.
    Refreshing {
        previous: Option<CachedToken>,
    },
    Loaded(CachedToken),
    Failed {
        message: String,
        // in case the last token still works... we can try to use that for a couple more mins
        previous: Option<CachedToken>,
    },
}

impl IapCredentialsState {
    fn previous_token(&self) -> Option<CachedToken> {
        match self {
            IapCredentialsState::Loaded(cached) => Some(cached.clone()),
            IapCredentialsState::Refreshing { previous }
            | IapCredentialsState::Failed { previous, .. } => previous.clone(),
            IapCredentialsState::Missing => None,
        }
    }
}

pub struct IapState {
    audiences: String,
    service_account_email: String,
    inner: RwLock<IapCredentialsState>,
}

impl IapState {
    pub fn new(config: &IapConfig) -> Self {
        Self {
            audiences: config.audiences.to_string(),
            service_account_email: config.service_account_email.to_string(),
            inner: RwLock::new(IapCredentialsState::Missing),
        }
    }

    pub fn get_cached(&self) -> Option<String> {
        match &*self.inner.read().expect("IAP state lock poisoned") {
            // Gate on expiry even while `Loaded`: if a proactive refresh is
            // delayed (e.g. the machine slept across the refresh window), the
            // token may already be expired, and attaching it would guarantee an
            // IAP challenge. Returning `None` lets the caller proceed without a
            // doomed token while the reactive refresh recovers.
            IapCredentialsState::Loaded(cached) => cached.valid_token(),
            IapCredentialsState::Refreshing { previous }
            | IapCredentialsState::Failed { previous, .. } => {
                previous.as_ref().and_then(CachedToken::valid_token)
            }
            IapCredentialsState::Missing => None,
        }
    }

    pub fn proxy_auth_header(&self) -> Option<(&'static str, String)> {
        self.get_cached()
            .map(|token| http_client::iap::proxy_auth_header(&token))
    }

    pub fn state(&self) -> IapCredentialsState {
        self.inner.read().expect("IAP state lock poisoned").clone()
    }

    pub fn audiences(&self) -> &str {
        &self.audiences
    }

    pub fn service_account_email(&self) -> &str {
        &self.service_account_email
    }

    fn set_refreshing(&self) {
        let mut state = self.inner.write().expect("IAP state lock poisoned");
        *state = IapCredentialsState::Refreshing {
            previous: state.previous_token(),
        };
    }

    fn set_loaded(&self, cached: CachedToken) {
        *self.inner.write().expect("IAP state lock poisoned") = IapCredentialsState::Loaded(cached);
    }

    fn set_failed(&self, message: String) {
        let mut state = self.inner.write().expect("IAP state lock poisoned");
        *state = IapCredentialsState::Failed {
            message,
            previous: state.previous_token(),
        };
    }
}

impl http_client::iap::IapTokenProvider for IapState {
    fn cached_token(&self) -> Option<String> {
        self.get_cached()
    }
}

/// Owns the IAP refresh lifecycle: initial fetch, proactive time-based
/// refresh, and reactive refresh on challenge events.
pub struct IapManager {
    state: Option<Arc<IapState>>,
    path_resolver: PathResolver,
    /// Runner-context Workload Identity Federation mint. When present, refreshes
    /// self-mint via WIF instead of shelling out to gcloud.
    managed_mint: Option<ManagedIapMint>,
    /// Number of consecutive failed fetches since the last success.
    consecutive_failures: u32,
}

pub enum IapManagerEvent {
    StateChanged,
    /// Emitted when IAP access could not be established within the gate timeout,
    /// so a blocking caller can fail closed instead of hanging.
    AccessUnavailable,
    RefreshFailed {
        /// A human-readable error message describing why the refresh failed.
        message: String,
        /// Whether this is the first failure in a streak of failures.
        is_first_failure_of_streak: bool,
    },
}

impl IapManager {
    pub fn new(
        state: Option<Arc<IapState>>,
        path_resolver: PathResolver,
        managed_mint: Option<ManagedIapMint>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut manager = Self {
            state,
            path_resolver,
            managed_mint,
            consecutive_failures: 0,
        };
        manager.start_refresh(ctx);
        manager
    }

    /// Returns `true` if IAP is active for this build. When `false`, all
    /// other methods on this type are no-ops.
    pub fn is_enabled(&self) -> bool {
        self.state.is_some()
    }

    pub fn state(&self) -> Option<IapCredentialsState> {
        self.state.as_ref().map(|s| s.state())
    }

    /// Returns a handle to the shared IAP credential state, if IAP is active.
    /// Mirrors how `AuthStateProvider` hands out the `Arc<AuthState>`, letting
    /// callers read cached credentials (e.g. to build a proxy-auth header) off
    /// a `ModelContext` without reaching through `ServerApi`.
    pub fn iap_state(&self) -> Option<Arc<IapState>> {
        self.state.clone()
    }

    /// Returns `true` when a currently-valid (unexpired) IAP token is available.
    /// Used to gate warp-server requests until IAP access is established.
    pub fn has_valid_token(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(|s| s.get_cached().is_some())
    }

    /// Establishes IAP access for a blocking caller: triggers a refresh and, if
    /// no valid token lands within [`IAP_ACCESS_TIMEOUT`], emits
    /// [`IapManagerEvent::AccessUnavailable`] so the caller can fail closed.
    /// A no-op when IAP is inactive.
    ///
    /// Callers should subscribe before calling: on [`IapManagerEvent::StateChanged`],
    /// check [`Self::has_valid_token`] and, once it returns `true`, proceed with the
    /// IAP-gated request; treat [`IapManagerEvent::AccessUnavailable`] as fatal.
    pub fn ensure_access(&mut self, ctx: &mut ModelContext<Self>) {
        if self.state.is_none() {
            return;
        }
        self.start_refresh(ctx);
        ctx.spawn(
            async { Timer::after(IAP_ACCESS_TIMEOUT).await },
            |manager, _, ctx| {
                if !manager.has_valid_token() {
                    ctx.emit(IapManagerEvent::AccessUnavailable);
                }
            },
        );
    }

    pub fn handle_challenge(&mut self, ctx: &mut ModelContext<Self>) {
        self.consecutive_failures = 0;
        self.start_refresh(ctx);
    }

    pub fn start_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // Don't touch state if a refresh is already running.
        if matches!(state.state(), IapCredentialsState::Refreshing { .. }) {
            return;
        }

        // Runner context: self-mint via Workload Identity Federation. This is the
        // only refresh path that works in a sandboxed Oz runner, which ships
        // without gcloud.
        if let Some(mint) = self.managed_mint.clone() {
            self.start_wif_refresh(state, mint, ctx);
            return;
        }

        state.set_refreshing();
        ctx.emit(IapManagerEvent::StateChanged);
        ctx.notify();

        let audiences = state.audiences().to_string();
        let service_account_email = state.service_account_email().to_string();

        // Make `gcloud` findable even when Warp is launched from the macOS GUI
        // (i.e. in environments without something like `~/.zshrc && WarpDev` happening to init cli path)
        let path_future = (self.path_resolver)(ctx);

        ctx.spawn(
            async move {
                // Bound the interactive PATH capture. It spawns an interactive
                // login shell (sourcing rc files), which can hang indefinitely
                // on a misbehaving startup script. Without this bound the
                // spawned task would never reach the `GCLOUD_TIMEOUT`-guarded
                // fetch, stranding the state machine in `Refreshing` and
                // silently disabling every future refresh and IAP challenge
                // (both early-return while `Refreshing`). On timeout, fall back
                // to the ambient PATH so the fetch still runs and the state
                // machine can make progress (succeed or fail).
                const PATH_CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
                let path_env = match path_future.with_timeout(PATH_CAPTURE_TIMEOUT).await {
                    Ok(path_env) => path_env,
                    Err(_) => {
                        log::warn!(
                            "Interactive PATH capture timed out after {}s; \
                             falling back to ambient PATH for IAP token fetch",
                            PATH_CAPTURE_TIMEOUT.as_secs()
                        );
                        None
                    }
                };
                unblock(move || {
                    fetch_iap_token(&audiences, &service_account_email, path_env.as_deref())
                })
                .await
            },
            move |manager, result, ctx| manager.apply_refresh_result(result, ctx),
        );
    }

    /// Runner-context refresh: mint an IAP-valid ID token via Workload Identity
    /// Federation (Warp OIDC JWT -> STS -> IAM `generateIdToken`). No gcloud.
    fn start_wif_refresh(
        &mut self,
        state: Arc<IapState>,
        mint: ManagedIapMint,
        ctx: &mut ModelContext<Self>,
    ) {
        // Fast path: reuse a fresh IAP token cache. This is actually required by
        // child processes when the main process spawns a child oz sub-process...
        // i.e. GCP SDK calling `oz federate issue-token --audience warp-cloud-agent-otel`
        if let Some(cached) = cache::read() {
            let expires_at = cached.expires_at;
            state.set_loaded(cached);
            ctx.emit(IapManagerEvent::StateChanged);
            ctx.notify();
            self.schedule_next_refresh(expires_at, ctx);
            return;
        }

        state.set_refreshing();
        ctx.emit(IapManagerEvent::StateChanged);
        ctx.notify();

        let minter = mint.minter.clone();
        let iap_audience = state.audiences().to_string();
        let service_account_email = state.service_account_email().to_string();

        ctx.spawn(
            async move {
                // The env var persists for the process lifetime, so its `aud` stays
                // readable even once the token itself has expired, which is what
                // refresh-time minting needs.
                let injected_jwt = std::env::var(STAGING_IAP_BOOTSTRAP_TOKEN_ENV_VAR)
                    .ok()
                    .filter(|jwt| !jwt.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{STAGING_IAP_BOOTSTRAP_TOKEN_ENV_VAR} is unset; cannot mint an IAP token via WIF"
                        )
                    })?;
                fetch_iap_token_via_wif(
                    minter,
                    injected_jwt,
                    iap_audience,
                    service_account_email,
                    &WifEndpoints {
                        sts_token_url: STS_TOKEN_URL.to_string(),
                        iam_generate_id_token_url_template: IAM_GENERATE_ID_TOKEN_URL
                            .to_string(),
                    },
                )
                .await
            },
            move |manager, result, ctx| manager.apply_refresh_result(result, ctx),
        );
    }

    /// Shared completion handler for both the gcloud and WIF refresh paths.
    fn apply_refresh_result(&mut self, result: Result<CachedToken>, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        match result {
            Ok(cached) => {
                let expires_at = cached.expires_at;
                // Publish for short-lived child `oz` processes to reuse
                // i.e. `oz federate issue-token --audience warp-cloud-agent-otel`
                // needs to transit IAP to get the token from warp-server
                if self.managed_mint.is_some() {
                    cache::write(&cached.token);
                }
                state.set_loaded(cached);
                self.consecutive_failures = 0;
                log::info!("Warp Staging IAP token refreshed");
                ctx.emit(IapManagerEvent::StateChanged);
                ctx.notify();
                self.schedule_next_refresh(expires_at, ctx);
            }
            Err(err) => {
                let message = format!("{err:#}");
                log::warn!("Warp Staging IAP token fetch failed: {message}");
                let is_first_failure_of_streak = self.consecutive_failures == 0;
                state.set_failed(message.clone());
                ctx.emit(IapManagerEvent::RefreshFailed {
                    message,
                    is_first_failure_of_streak,
                });
                ctx.emit(IapManagerEvent::StateChanged);
                ctx.notify();
                self.schedule_failure_retry(ctx);
            }
        }
    }

    fn schedule_next_refresh(&mut self, expires_at: Instant, ctx: &mut ModelContext<Self>) {
        let sleep_duration = expires_at
            .saturating_duration_since(Instant::now())
            .saturating_sub(PROACTIVE_REFRESH_BUFFER);
        self.schedule_retry(sleep_duration, ctx);
    }

    fn schedule_failure_retry(&mut self, ctx: &mut ModelContext<Self>) {
        if self.consecutive_failures >= MAX_FAILURE_RETRIES {
            log::warn!(
                "IAP token fetch failed {MAX_FAILURE_RETRIES} times in a row; giving up until \
                 manual refresh or server challenge"
            );
            return;
        }
        // Delay = BASE * 2^failures, capped at MAX. Using u32 shift is
        // safe because we cap failures at MAX_FAILURE_RETRIES (< 32).
        let delay = BASE_FAILURE_RETRY_DELAY
            .saturating_mul(1u32 << self.consecutive_failures)
            .min(MAX_FAILURE_RETRY_DELAY);
        self.consecutive_failures += 1;
        log::info!(
            "Scheduling IAP refresh retry #{} in {}s",
            self.consecutive_failures,
            delay.as_secs()
        );
        self.schedule_retry(delay, ctx);
    }

    fn schedule_retry(&mut self, delay: Duration, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(delay).await;
            },
            |manager, _, ctx| {
                manager.start_refresh(ctx);
            },
        );
    }

    /// Inspects a websocket *handshake* connect error for an IAP challenge.
    /// If detected, triggers a refresh so the caller's retry loop can pick up
    /// a fresh token on the next attempt.
    #[cfg(not(target_family = "wasm"))]
    pub fn check_ws_connect_error(&mut self, err: &anyhow::Error, ctx: &mut ModelContext<Self>) {
        if ws_connect_is_iap_challenge(err) {
            log::warn!("Received IAP challenge on websocket handshake; triggering refresh");
            self.handle_challenge(ctx);
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn check_ws_connect_error(&mut self, _err: &anyhow::Error, _ctx: &mut ModelContext<Self>) {}
}

#[cfg(not(target_family = "wasm"))]
pub fn ws_connect_is_iap_challenge(err: &anyhow::Error) -> bool {
    connect_error_http_response(err).is_some_and(|response| {
        http_client::iap::is_iap_challenge(response.status(), response.headers())
    })
}

impl Entity for IapManager {
    type Event = IapManagerEvent;
}

impl SingletonEntity for IapManager {}

#[derive(Serialize)]
struct StsTokenExchangeRequest<'a> {
    grant_type: &'a str,
    audience: &'a str,
    scope: &'a str,
    requested_token_type: &'a str,
    subject_token: &'a str,
    subject_token_type: &'a str,
}

#[derive(Deserialize)]
struct StsTokenExchangeResponse {
    access_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateIdTokenRequest<'a> {
    audience: &'a str,
    include_email: bool,
}

#[derive(Deserialize)]
struct GenerateIdTokenResponse {
    token: String,
}

/// GCP endpoints for the WIF mint. The production call site passes the real
/// Google endpoint consts; tests override them to point at a mock server.
struct WifEndpoints {
    sts_token_url: String,
    /// `generateIdToken` URL carrying a `{sa_email}` placeholder for the
    /// impersonated service account.
    iam_generate_id_token_url_template: String,
}

/// Leg 1 of the WIF mint: pick the OIDC subject token for the STS exchange.
/// Prefer the injected bootstrap JWT while it's still valid so a cold runner
/// needn't call the IAP-gated identity-token endpoint; once it expires, mint a
/// fresh one via the server (now reachable through IAP).
async fn resolve_wif_identity_token(
    injected_jwt: String,
    federation_audience: &str,
    minter: &Arc<dyn IapIdentityTokenMinter>,
) -> Result<String> {
    if get_expires_at(&injected_jwt).is_ok() {
        Ok(injected_jwt)
    } else {
        minter
            .mint_identity_token(federation_audience.to_string(), WIF_IDENTITY_TOKEN_DURATION)
            .await
            .map_err(|err| anyhow::anyhow!("failed to mint Warp identity token: {err:#}"))
    }
}

/// Mints an IAP-valid ID token for a sandboxed Oz runner via Workload Identity
/// Federation: Warp OIDC JWT -> GCP STS federated token -> IAM `generateIdToken`
/// impersonating the IAP access service account. Requires no local gcloud.
async fn fetch_iap_token_via_wif(
    minter: Arc<dyn IapIdentityTokenMinter>,
    injected_jwt: String,
    iap_audience: String,
    service_account_email: String,
    endpoints: &WifEndpoints,
) -> Result<CachedToken> {
    // The WIF provider resource name is the `aud` of the server-injected bootstrap
    // JWT, so we read it straight off that token instead of carrying it as
    // separate client config.
    let federation_audience = parse_aud_from_jwt(&injected_jwt)
        .ok_or_else(|| anyhow::anyhow!("injected OIDC JWT has no readable `aud` claim"))?;

    // Leg 1: obtain the Warp-signed OIDC JWT used as the STS subject token.
    let identity_token =
        resolve_wif_identity_token(injected_jwt, &federation_audience, &minter).await?;

    // Leg 2: exchange the JWT at GCP STS for a federated access token.
    let response = http_client::Client::new()
        .post(&endpoints.sts_token_url)
        .form(&StsTokenExchangeRequest {
            grant_type: TOKEN_EXCHANGE_GRANT_TYPE,
            audience: &federation_audience,
            scope: CLOUD_PLATFORM_SCOPE,
            requested_token_type: REQUESTED_TOKEN_TYPE_ACCESS_TOKEN,
            subject_token: &identity_token,
            subject_token_type: SUBJECT_TOKEN_TYPE_ID_TOKEN,
        })
        .timeout(WIF_MINT_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("STS token exchange request failed: {err:#}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("STS token exchange failed (status {status}): {body}");
    }
    let sts_response: StsTokenExchangeResponse = response
        .json()
        .await
        .map_err(|err| anyhow::anyhow!("failed to parse the STS response: {err:#}"))?;

    // Leg 3: impersonate the IAP access service account to mint an ID token whose
    // audience is the IAP OAuth client ID. IAM authorizes this only if the
    // runner's federated identity holds roles/iam.serviceAccountTokenCreator on
    // the service account.
    let url = endpoints
        .iam_generate_id_token_url_template
        .replace("{sa_email}", &service_account_email);
    let response = http_client::Client::new()
        .post(&url)
        .bearer_auth(&sts_response.access_token)
        .json(&GenerateIdTokenRequest {
            audience: &iap_audience,
            include_email: true,
        })
        .timeout(WIF_MINT_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("generateIdToken request failed: {err:#}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("generateIdToken failed (status {status}): {body}");
    }
    let id_token_response: GenerateIdTokenResponse = response
        .json()
        .await
        .map_err(|err| anyhow::anyhow!("failed to parse the generateIdToken response: {err:#}"))?;

    let expires_at = get_expires_at(&id_token_response.token)?;
    Ok(CachedToken {
        token: id_token_response.token,
        expires_at,
    })
}

/// How long to wait for `auth print-identity-token` command to respond before killing it.
const GCLOUD_TIMEOUT: Duration = Duration::from_secs(30);

// gcloud ships as `gcloud.cmd` on Windows
#[cfg(windows)]
const GCLOUD_PROGRAM: &str = "gcloud.cmd";
#[cfg(not(windows))]
const GCLOUD_PROGRAM: &str = "gcloud";

fn fetch_iap_token(
    audiences: &str,
    service_account_email: &str,
    path_env: Option<&str>,
) -> Result<CachedToken> {
    let args = [
        "auth",
        "print-identity-token",
        "--audiences",
        audiences,
        "--impersonate-service-account",
        service_account_email,
        "--include-email",
    ];
    let cmd_display = format!("{GCLOUD_PROGRAM} {}", args.join(" "));

    let mut cmd = command::blocking::Command::new(GCLOUD_PROGRAM);
    cmd
        // Prevent gcloud from waiting for interactive input (fail fast instead of hanging)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .args(args);
    // allows warp to resolve `gcloud` cli path
    if let Some(path_env) = path_env {
        cmd.env("PATH", path_env);
    }
    let mut child = cmd
        .spawn()
        .map_err(|err| anyhow::anyhow!("Failed to spawn `{cmd_display}`: {err}"))?;

    // Poll for completion, killing the child if it exceeds the timeout.
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > GCLOUD_TIMEOUT {
                    let _ = child.kill();
                    anyhow::bail!(
                        "`{cmd_display}` timed out after {}s",
                        GCLOUD_TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => anyhow::bail!("Failed to wait for `{cmd_display}`: {err}"),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| anyhow::anyhow!("Failed to collect output from `{cmd_display}`: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("`{cmd_display}` failed: {stderr}");
    }

    let token = String::from_utf8(output.stdout)
        .map_err(|err| anyhow::anyhow!("gcloud output is not valid UTF-8: {err}"))?
        .trim()
        .to_string();

    anyhow::ensure!(!token.is_empty(), "gcloud returned an empty token");

    let expires_at = get_expires_at(&token)?;
    Ok(CachedToken { token, expires_at })
}

fn get_expires_at(token: &str) -> Result<Instant> {
    let exp = parse_exp_from_jwt(token).ok_or_else(|| {
        anyhow::anyhow!("IAP token missing or unparseable `exp` claim; refusing to cache")
    })?;
    // `exp` is Unix wall-clock seconds; `Instant` is monotonic and
    // has no Unix-time API, so bridge via `SystemTime::now()` to
    // compute a delta, then add that to `Instant::now()`.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| anyhow::anyhow!("system clock is before unix epoch: {err}"))?
        .as_secs();
    let secs_remaining = exp
        .checked_sub(now)
        .ok_or_else(|| anyhow::anyhow!("IAP token is already expired (exp={exp}, now={now})"))?;
    Ok(Instant::now() + Duration::from_secs(secs_remaining))
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload_b64 = token.split('.').nth(1)?;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    serde_json::from_slice(&payload_bytes).ok()
}

fn parse_exp_from_jwt(token: &str) -> Option<u64> {
    decode_jwt_payload(token)?.get("exp")?.as_u64()
}

/// Reads the `aud` claim from a JWT. `aud` may be a single string or an array of
/// strings (per RFC 7519); we take the first entry in the array case.
fn parse_aud_from_jwt(token: &str) -> Option<String> {
    match decode_jwt_payload(token)?.get("aud")? {
        serde_json::Value::String(aud) => Some(aud.clone()),
        serde_json::Value::Array(auds) => auds.iter().find_map(|v| v.as_str().map(str::to_string)),
        _ => None,
    }
}

/// On-disk cache of the current IAP token, written by the long-lived parent
/// process (which keeps it fresh) so short-lived child `oz` processes can reuse
/// it instead of each doing their own STS/IAM mint. The file stores the raw IAP
/// JWT; validity is derived from its own `exp` claim on read.
///
/// This is a plain owner-only file rather than OS secure storage: it only exists
/// in the staging Oz runner (headless Linux), where no keyring is available and
/// the container already holds the bootstrap JWT / API key / GitHub token.
#[cfg(not(target_family = "wasm"))]
mod cache {
    use std::time::Duration;

    use instant::Instant;

    use super::{CachedToken, get_expires_at};

    /// Discard a cached token with less than this remaining, so we never hand back
    /// a near-dead credential the mint path should refresh instead.
    const CACHE_SKEW: Duration = Duration::from_secs(30);

    fn cache_path() -> std::path::PathBuf {
        warp_core::paths::secure_state_dir()
            .unwrap_or_else(warp_core::paths::state_dir)
            .join("staging")
            .join("iap_cache.jwt")
    }

    pub(super) fn read() -> Option<CachedToken> {
        let token = std::fs::read_to_string(cache_path()).ok()?;
        let token = token.trim().to_owned();
        if token.is_empty() {
            return None;
        }
        let expires_at = get_expires_at(&token).ok()?;
        if expires_at <= Instant::now() + CACHE_SKEW {
            return None;
        }
        Some(CachedToken { token, expires_at })
    }

    pub(super) fn write(token: &str) {
        if let Err(err) = write_atomic(token) {
            log::warn!("Failed to write IAP token cache: {err}");
        }
    }

    fn write_atomic(token: &str) -> std::io::Result<()> {
        use std::io::Write as _;

        let path = cache_path();
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        // Write to a uniquely-named temp file in the same dir, then atomically
        // persist it over the target: readers never observe a partial token,
        // concurrent writers don't share a temp path, and the temp file is
        // cleaned up on drop if any step before `persist` fails. `NamedTempFile`
        // is created owner-only (0600) on unix.
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(token.as_bytes())?;
        tmp.persist(&path)?;
        Ok(())
    }
}

/// On wasm there is no filesystem, and IAP self-minting never runs, so the cache
/// is a no-op.
#[cfg(target_family = "wasm")]
mod cache {
    use super::CachedToken;

    pub(super) fn read() -> Option<CachedToken> {
        None
    }

    pub(super) fn write(_token: &str) {}
}

#[cfg(test)]
#[path = "iap_tests.rs"]
mod tests;
