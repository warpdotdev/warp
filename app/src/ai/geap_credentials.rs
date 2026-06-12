//! Mint/refresh lifecycle for Gemini Enterprise (GEAP) credentials.
//!
//! Local interactive agent requests authenticate against the workspace's GCP
//! project with a short-lived access token minted through Workload Identity
//! Federation. Every mint is a fixed 3-leg chain rooted in the user's Warp
//! session
//!
//! 1. **Warp OIDC JWT**: `IssueTaskIdentityToken` (authenticated by the Warp
//!    session) returns a short-lived Warp-signed JWT with `aud` =
//!    `gcpAudience`. Every mint grabs a brand-new JWT; it is consumed exactly
//!    once by the immediately following STS exchange and never cached, so an
//!    expired JWT can never be presented to Google.
//! 2. **STS exchange (RFC 8693)**: the JWT becomes a Google federated token;
//!    Google validates the issuer signature and audience here.
//! 3. **SA impersonation**: when `gcpSaEmail` is configured, the federated
//!    token mints a ~1h service-account access token via IAM
//!    `generateAccessToken`; skipped entirely when empty.
//!
//! The resulting token lands in [`GeapCredentialsState::Loaded`] on
//! [`ApiKeyManager`] for the request attach path. Three layers keep requests
//! authenticated: a proactive one-shot timer that re-mints
//! [`GEAP_REFRESH_LEAD_TIME`] before expiry, a request-time safety net that
//! re-arms a parked chain, and the server-side 401 backstop — tokens are
//! always sent, even past expiry, never silently dropped.
//!
//! Security invariants: the access token lives only in memory;
//! no refresh token, ADC file, or SA key exists anywhere in the flow.

use std::time::{Duration, SystemTime};

use ai::api_keys::{
    ApiKeyManager, GeapCredentials, GeapCredentialsState, GeapMintBinding, GeapRequestGate,
    GEAP_REFRESH_LEAD_TIME,
};
use serde::{Deserialize, Serialize};
use vec1::vec1;
use warp_managed_secrets::client::{IdentityTokenOptions, TaskIdentityToken};
use warp_managed_secrets::ManagedSecretManager;
use warpui::r#async::Timer;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Requested lifetime for the Warp OIDC JWT (leg 1). The JWT only has to
/// outlive the leg 1 -> leg 2 gap (seconds); its expiry is consulted exactly
/// once, as the conservative no-SA fallback bound in [`sts_expires_at`].
const GEAP_IDENTITY_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);

/// Floor on the proactive refresh timer delay so a near-expired store (e.g. a
/// badly skewed local clock) cannot spin mint -> store -> re-mint as a hot
/// loop; the floor rate-limits timer-driven re-mints to once per minute.
const GEAP_MIN_TIMER_DELAY: Duration = Duration::from_secs(60);

/// Cap on provider error detail captured into states/logs. Bodies are
/// sanitized to this length and never contain token material.
const ERROR_DETAIL_MAX_CHARS: usize = 512;

const STS_TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";
const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
/// Requested lifetime for the impersonated SA access token (leg 3).
const SA_ACCESS_TOKEN_LIFETIME: &str = "3600s";

/// Per-leg mint failures, so error copy can pinpoint the broken leg: a Warp
/// session problem (leg 1) vs. a pool/provider misconfiguration (leg 2) vs. a
/// missing IAM binding (leg 3). Details are capped at
/// [`ERROR_DETAIL_MAX_CHARS`] and never contain token material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadGeapCredentialsError {
    /// Leg 1: minting the Warp OIDC JWT failed (Warp session / network).
    MintIdentityToken(String),
    /// Leg 2: Google STS rejected the token exchange (pool/provider config).
    ExchangeToken(String),
    /// Leg 3: IAM `generateAccessToken` failed (missing
    /// `roles/iam.workloadIdentityUser` binding or disabled API).
    ImpersonateServiceAccount(String),
}

impl std::fmt::Display for LoadGeapCredentialsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MintIdentityToken(detail) => write!(
                f,
                "Failed to authenticate with Warp while minting Gemini Enterprise credentials \
                 (check your Warp session and network): {detail}"
            ),
            Self::ExchangeToken(detail) => write!(
                f,
                "Google rejected the Gemini Enterprise token exchange — ask your workspace admin \
                 to verify the workload identity pool/provider configuration (`gcpAudience`): \
                 {detail}"
            ),
            Self::ImpersonateServiceAccount(detail) => write!(
                f,
                "Service account impersonation failed — ask your workspace admin to verify the \
                 `roles/iam.workloadIdentityUser` binding on the workspace's service account: \
                 {detail}"
            ),
        }
    }
}

impl std::error::Error for LoadGeapCredentialsError {}

/// The two admin-configured federation parameters the client consumes:
/// the workload identity provider resource name (the JWT `aud` claim and STS
/// `audience` parameter) and the optional service account to impersonate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeapWifConfig {
    pub audience: String,
    /// `None` means "use the federated token directly" (no leg 3).
    pub service_account_email: Option<String>,
}

impl GeapWifConfig {
    /// Builds the mint config from an evaluated request gate. `None` when the
    /// workspace is enabled but unconfigured (empty audience).
    fn from_gate(gate: &GeapRequestGate) -> Option<Self> {
        if gate.audience.is_empty() {
            return None;
        }
        Some(Self {
            audience: gate.audience.clone(),
            service_account_email: gate.sa_email.clone(),
        })
    }
}

/// Builds the expected mint binding from raw host-settings values, trimming
/// whitespace and normalizing empties so the gate, the mint config, and the
/// stored binding can never disagree on representation.
fn geap_request_gate_from_parts(
    user_uid: String,
    gcp_audience: Option<&str>,
    gcp_sa_email: Option<&str>,
) -> GeapRequestGate {
    GeapRequestGate {
        user_uid,
        audience: gcp_audience.map(str::trim).unwrap_or_default().to_string(),
        sa_email: gcp_sa_email
            .map(str::trim)
            .filter(|sa_email| !sa_email.is_empty())
            .map(str::to_string),
    }
}

/// The expected (user, audience, SA) binding when the GEAP enablement gate is
/// on; `None` when any part of the gate is off (admin off, enforced off,
/// member opted out, or logged out). This is the single policy evaluation
/// shared by the request build site, the refresh guard, and the mint
/// completion re-check.
pub(crate) fn current_geap_request_gate(app: &AppContext) -> Option<GeapRequestGate> {
    let user_workspaces = UserWorkspaces::as_ref(app);
    if !user_workspaces.is_gemini_enterprise_credentials_enabled(app) {
        return None;
    }
    // The enablement gate guarantees a logged-in user, so a missing uid is
    // unreachable; bail safely regardless — there is no principal to mint for.
    let user_uid = AuthStateProvider::as_ref(app).get().user_id()?.as_string();
    let settings = user_workspaces.gemini_enterprise_host_settings()?;
    Some(geap_request_gate_from_parts(
        user_uid,
        settings.gcp_audience.as_deref(),
        settings.gcp_sa_email.as_deref(),
    ))
}

fn binding_for_gate(gate: &GeapRequestGate) -> GeapMintBinding {
    GeapMintBinding {
        user_uid: gate.user_uid.clone(),
        audience: gate.audience.clone(),
        sa_email: gate.sa_email.clone(),
    }
}

/// Extension trait for [`ApiKeyManager`] wiring the GEAP credential refresh
/// triggers, mirroring [`crate::ai::aws_credentials::AwsCredentialRefresher`].
pub trait GeapCredentialRefresher {
    /// Subscribes the event-driven refresh triggers:
    /// - `UserWorkspaces`: `TeamsChanged` (startup / team or account switch)
    ///   and `UpdateWorkspaceSettingsSuccess` (admin saves). The mint binding
    ///   makes the refresh guard a no-op unless the audience/SA actually
    ///   changed, so unrelated admin saves cost no STS round-trip.
    /// - `AISettings`: the member flips their own toggle under
    ///   `RESPECT_USER_SETTING`.
    fn subscribe_to_geap_settings_changes(&mut self, ctx: &mut ModelContext<Self>)
    where
        Self: Sized;
}

impl GeapCredentialRefresher for ApiKeyManager {
    fn subscribe_to_geap_settings_changes(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, event, ctx| {
            if matches!(
                event,
                UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess
                    | UserWorkspacesEvent::TeamsChanged
            ) {
                refresh_geap_credentials(manager, ctx);
            }
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::GeminiEnterpriseCredentialsEnabled { .. }
            ) {
                refresh_geap_credentials(manager, ctx);
            }
        });
    }
}

/// Standard (non-forced) refresh: the skip-if-valid guard decides whether a
/// mint is actually needed.
pub(crate) fn refresh_geap_credentials(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    refresh_geap_credentials_with_options(manager, false, ctx);
}

/// Forced refresh: re-mints unconditionally (subject to the one-mint-at-a-time
/// rule). Consumed by the Settings "Refresh credentials" button and the inline
/// credential error view's auto-remint/Retry, which land in a follow-up PR.
#[allow(dead_code)]
pub(crate) fn force_refresh_geap_credentials(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    refresh_geap_credentials_with_options(manager, true, ctx);
}

/// Request-time safety net (the GEAP analog of
/// `ApiKeyManager::refresh_grok_tokens_if_needed`): re-arms a parked or
/// never-armed refresh chain on every agent request build. No-ops unless the
/// gate is on AND (there is no usable token, the binding mismatches, or the
/// token is within the refresh lead window / already expired). The triggering
/// request is never delayed — it carries the currently stored token.
pub(crate) fn refresh_geap_credentials_if_needed(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let Some(gate) = current_geap_request_gate(ctx) else {
        // Gate off: a pure no-op on the request path (the attach site already
        // skips GEAP); state transitions are owned by the event triggers.
        return;
    };
    if gate.audience.is_empty() {
        // Enabled but unconfigured: nothing to mint from.
        return;
    }
    let needs_mint = match manager.geap_credentials_state() {
        // One mint at a time; the in-flight result lands in ~1-3s.
        GeapCredentialsState::Refreshing { .. } => false,
        GeapCredentialsState::Loaded {
            credentials,
            minted_for,
            ..
        } => !minted_for.matches(&gate) || credentials.needs_refresh(),
        GeapCredentialsState::Missing
        | GeapCredentialsState::Disabled
        | GeapCredentialsState::Failed { .. } => true,
    };
    if needs_mint {
        log::info!("GEAP: request-time safety net arming a credential refresh");
        refresh_geap_credentials(manager, ctx);
    }
}

/// The refresh guard + mint kickoff that all triggers funnel through.
fn refresh_geap_credentials_with_options(
    manager: &mut ApiKeyManager,
    force: bool,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let Some(gate) = current_geap_request_gate(ctx) else {
        // Gate off (admin off, enforced off, member opted out, or logged
        // out): drop any held token — tokens are never retained while
        // disabled.
        manager.set_geap_credentials_state(GeapCredentialsState::Disabled, ctx);
        return;
    };
    let Some(config) = GeapWifConfig::from_gate(&gate) else {
        // Enabled but unconfigured (empty `gcpAudience`).
        manager.set_geap_credentials_state(GeapCredentialsState::Missing, ctx);
        return;
    };
    // One mint at a time, force included: the in-flight result lands in ~1-3s
    // and `KeysUpdated` re-renders whoever asked.
    if matches!(
        manager.geap_credentials_state(),
        GeapCredentialsState::Refreshing { .. }
    ) {
        return;
    }
    let minted_for = binding_for_gate(&gate);
    // Skip-if-valid: don't hammer STS. A binding mismatch (current user/config
    // vs. the binding recorded at mint) falls through and re-mints under the
    // fresh principal + config.
    if !force {
        if let GeapCredentialsState::Loaded {
            credentials,
            minted_for: current_binding,
            ..
        } = manager.geap_credentials_state()
        {
            if *current_binding == minted_for && !credentials.needs_refresh() {
                return;
            }
        }
    }
    // The previous token (if any) keeps serving requests while the re-mint is
    // in flight — tokens stay until replaced, so a request landing in the
    // ~1-3s mint window still authenticates. Only a token minted for the SAME
    // binding as this mint is carried: a binding-mismatched token is
    // unservable (the attach matrix rejects it), and restoring it on a failed
    // re-mint would mask the `Failed` state behind a misleading `Loaded`.
    let previous = match manager.geap_credentials_state() {
        GeapCredentialsState::Loaded {
            credentials,
            minted_for: current_binding,
            ..
        } if *current_binding == minted_for => Some((credentials.clone(), current_binding.clone())),
        _ => None,
    };
    log::info!(
        "GEAP: minting credentials (audience={}, force={force})",
        config.audience
    );
    manager.set_geap_credentials_state(GeapCredentialsState::Refreshing { previous }, ctx);

    // Leg 1: every mint — initial or re-mint, timer/trigger/forced — starts
    // with a brand-new Warp OIDC JWT, consumed exactly once by the STS
    // exchange below and never cached across mints.
    let token_future = ManagedSecretManager::handle(ctx)
        .as_ref(ctx)
        .issue_task_identity_token(IdentityTokenOptions {
            audience: config.audience.clone(),
            requested_duration: GEAP_IDENTITY_TOKEN_DURATION,
            subject_template: vec1!["principal".to_string()],
        });
    let _ = ctx.spawn(
        async move {
            let identity_token = token_future.await.map_err(|err| {
                LoadGeapCredentialsError::MintIdentityToken(truncate_error_detail(&format!(
                    "{err:#}"
                )))
            })?;
            exchange_identity_token_for_geap_credentials(identity_token, &config).await
        },
        move |manager, result, ctx| apply_geap_mint_result(manager, result, minted_for, force, ctx),
    );
}

/// Stores a finished mint's result, re-checking the world first: the gate or
/// config may have changed during the ~1-3s mint.
fn apply_geap_mint_result(
    manager: &mut ApiKeyManager,
    result: Result<GeapCredentials, LoadGeapCredentialsError>,
    minted_for: GeapMintBinding,
    force: bool,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    // Gate flipped off mid-mint: discard the result; no token is retained
    // while disabled.
    let Some(gate) = current_geap_request_gate(ctx) else {
        log::info!("GEAP: gate flipped off mid-mint; discarding the mint result");
        manager.set_geap_credentials_state(GeapCredentialsState::Disabled, ctx);
        return;
    };
    // No state transition may store credentials under a binding that does not
    // match the world at storage time. That covers `previous` too: a carried
    // token is only restorable while it still matches the current gate — a
    // mismatched token is unservable (the attach matrix rejects it), and
    // restoring it would mask a failure behind a misleading `Loaded`.
    let current_binding = binding_for_gate(&gate);
    let previous = match manager.geap_credentials_state() {
        GeapCredentialsState::Refreshing {
            previous: Some((credentials, binding)),
        } if *binding == current_binding => Some((credentials.clone(), binding.clone())),
        _ => None,
    };

    // The user/account or federation config changed while the mint was in
    // flight: the result is stamped for a stale binding and would never be
    // attachable. Discard it (success or failure alike) and immediately
    // re-mint under the current binding — waiting for the next trigger would
    // leave a one-request token-less window, i.e. a silent Direct API
    // fallback under RESPECT_USER_SETTING.
    if minted_for != current_binding {
        log::info!("GEAP: binding changed mid-mint; discarding the result and re-minting");
        match previous {
            Some((credentials, minted_for)) => {
                manager.set_geap_credentials_state(
                    GeapCredentialsState::Loaded {
                        credentials,
                        loaded_at: SystemTime::now(),
                        minted_for,
                    },
                    ctx,
                );
                // Keep the proactive loop alive in case the re-mint below
                // skips (the restored token may already be fresh).
                schedule_geap_token_refresh(manager, ctx);
            }
            None => {
                manager.set_geap_credentials_state(GeapCredentialsState::Missing, ctx);
            }
        }
        refresh_geap_credentials(manager, ctx);
        return;
    }

    match result {
        Ok(credentials) => {
            log::info!(
                "GEAP: credentials minted (audience={}, expires_at={:?})",
                minted_for.audience,
                credentials.expires_at()
            );
            manager.set_geap_credentials_state(
                GeapCredentialsState::Loaded {
                    credentials,
                    loaded_at: SystemTime::now(),
                    minted_for,
                },
                ctx,
            );
            // Arm the next one-shot proactive refresh — this is what makes
            // the ~hourly loop self-sustaining.
            schedule_geap_token_refresh(manager, ctx);
        }
        Err(err) => {
            log::error!("GEAP: credential mint failed: {err}");
            match previous {
                // A failed background re-mint keeps the previous token — even
                // near/past expiry (Google remains the authority on validity;
                // sending it can only yield a visible, recoverable 401, never
                // a silent downgrade) — and parks the chain. No reschedule:
                // the next agent request's safety net re-arms it, so a
                // hard-down network cannot cause unbounded STS traffic.
                Some((credentials, minted_for)) if !force => {
                    manager.set_geap_credentials_state(
                        GeapCredentialsState::Loaded {
                            credentials,
                            loaded_at: SystemTime::now(),
                            minted_for,
                        },
                        ctx,
                    );
                }
                // First mint (nothing servable to keep), or a forced refresh
                // where the user explicitly asked and needs visible feedback.
                _ => {
                    manager.set_geap_credentials_state(
                        GeapCredentialsState::Failed {
                            message: err.to_string(),
                        },
                        ctx,
                    );
                }
            }
        }
    }
}

/// Arms a one-shot timer that re-mints [`GEAP_REFRESH_LEAD_TIME`] before the
/// loaded token's expiry. The timer is armed once per token — no periodic
/// polling; the process wakes exactly once per token lifetime. Stale timers
/// from rapid mints are harmless: the callback runs the non-forced refresh,
/// whose skip-if-valid guard no-ops when another trigger already minted a
/// fresh token while the timer slept.
fn schedule_geap_token_refresh(manager: &mut ApiKeyManager, ctx: &mut ModelContext<ApiKeyManager>) {
    let GeapCredentialsState::Loaded { credentials, .. } = manager.geap_credentials_state() else {
        return;
    };
    // `expires_at` is always known for GEAP (both mint paths produce one);
    // a missing expiry is a safe default with no timer, Grok-parity by
    // construction.
    let Some(expires_at) = credentials.expires_at() else {
        return;
    };
    let delay = geap_refresh_timer_delay(expires_at, SystemTime::now());
    let _ = ctx.spawn(
        async move {
            Timer::after(delay).await;
        },
        |manager, _output, ctx| {
            refresh_geap_credentials(manager, ctx);
        },
    );
}

/// Pure timer-delay computation: fire [`GEAP_REFRESH_LEAD_TIME`] before
/// `expires_at`, clamped up to the [`GEAP_MIN_TIMER_DELAY`] floor — never
/// immediate, so a near-expired store (e.g. a badly skewed local clock)
/// cannot spin mint -> store -> re-mint as a hot loop.
fn geap_refresh_timer_delay(expires_at: SystemTime, now: SystemTime) -> Duration {
    let fire_at = expires_at
        .checked_sub(GEAP_REFRESH_LEAD_TIME)
        .unwrap_or(now);
    fire_at
        .duration_since(now)
        .unwrap_or(Duration::ZERO)
        .max(GEAP_MIN_TIMER_DELAY)
}

#[derive(Serialize)]
struct StsTokenExchangeRequest<'a> {
    grant_type: &'a str,
    audience: &'a str,
    scope: &'a str,
    requested_token_type: &'a str,
    subject_token: &'a str,
    subject_token_type: &'a str,
}

#[derive(Debug, Deserialize)]
struct StsTokenExchangeResponse {
    access_token: String,
    /// Relative lifetime in seconds. Optional per RFC 8693; when omitted the
    /// caller falls back to the Warp JWT's own expiry as a conservative bound.
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateAccessTokenRequest {
    scope: Vec<String>,
    lifetime: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateAccessTokenResponse {
    access_token: String,
    /// RFC 3339 timestamp; always present in IAM responses.
    expire_time: String,
}

/// Legs 2 and 3 of the mint: exchanges the Warp OIDC JWT at Google STS for a
/// federated token, then (when configured) impersonates the workspace's
/// service account for the final ~1h access token. Runs entirely off the
/// request path via `http_client` (the Compat-wrapped reqwest required to run
/// off-Tokio on the warpui executor).
async fn exchange_identity_token_for_geap_credentials(
    identity_token: TaskIdentityToken,
    config: &GeapWifConfig,
) -> Result<GeapCredentials, LoadGeapCredentialsError> {
    // Leg 2: STS token exchange (RFC 8693). Google validates the issuer
    // signature (against the public JWKS) and the audience against the pool's
    // allowed audiences here.
    let response = http_client::Client::new()
        .post(STS_TOKEN_URL)
        .form(&StsTokenExchangeRequest {
            grant_type: TOKEN_EXCHANGE_GRANT_TYPE,
            audience: &config.audience,
            scope: CLOUD_PLATFORM_SCOPE,
            requested_token_type: ACCESS_TOKEN_TYPE,
            subject_token: &identity_token.token,
            subject_token_type: ID_TOKEN_TYPE,
        })
        .send()
        .await
        .map_err(|err| {
            LoadGeapCredentialsError::ExchangeToken(truncate_error_detail(&format!(
                "request failed: {err:#}"
            )))
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LoadGeapCredentialsError::ExchangeToken(format!(
            "HTTP {status}: {}",
            truncate_error_detail(&body)
        )));
    }
    let sts_response: StsTokenExchangeResponse = response.json().await.map_err(|err| {
        LoadGeapCredentialsError::ExchangeToken(truncate_error_detail(&format!(
            "failed to parse the STS response: {err:#}"
        )))
    })?;
    log::info!(
        "GEAP: STS exchange succeeded (audience={})",
        config.audience
    );

    let federated_expires_at = sts_expires_at(
        sts_response.expires_in,
        SystemTime::from(identity_token.expires_at),
        SystemTime::now(),
    );

    let Some(sa_email) = config.service_account_email.as_deref() else {
        // No impersonation configured: the federated token is used directly.
        return Ok(GeapCredentials::new(
            sts_response.access_token,
            Some(federated_expires_at),
        ));
    };

    // Leg 3: SA impersonation. IAM authorizes this only if the pool identity
    // holds `roles/iam.workloadIdentityUser` on the SA — the customer's
    // control point for who may become the SA.
    let url = format!(
        "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa_email}:generateAccessToken"
    );
    let response = http_client::Client::new()
        .post(&url)
        .bearer_auth(&sts_response.access_token)
        .json(&GenerateAccessTokenRequest {
            scope: vec![CLOUD_PLATFORM_SCOPE.to_string()],
            lifetime: SA_ACCESS_TOKEN_LIFETIME.to_string(),
        })
        .send()
        .await
        .map_err(|err| {
            LoadGeapCredentialsError::ImpersonateServiceAccount(truncate_error_detail(&format!(
                "request failed: {err:#}"
            )))
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LoadGeapCredentialsError::ImpersonateServiceAccount(
            format!("HTTP {status}: {}", truncate_error_detail(&body)),
        ));
    }
    let impersonation: GenerateAccessTokenResponse = response.json().await.map_err(|err| {
        LoadGeapCredentialsError::ImpersonateServiceAccount(truncate_error_detail(&format!(
            "failed to parse the impersonation response: {err:#}"
        )))
    })?;
    let expires_at = parse_generate_access_token_expiry(&impersonation.expire_time)
        .map_err(LoadGeapCredentialsError::ImpersonateServiceAccount)?;
    log::info!(
        "GEAP: service account impersonation succeeded (audience={})",
        config.audience
    );
    Ok(GeapCredentials::new(
        impersonation.access_token,
        Some(expires_at),
    ))
}

/// Absolute expiry for the STS federated token: `now + expires_in` when STS
/// reports one; otherwise the Warp JWT's own expiry as a conservative bound
/// (the federated token cannot outlive the JWT it was minted from).
fn sts_expires_at(
    expires_in: Option<u64>,
    jwt_expires_at: SystemTime,
    now: SystemTime,
) -> SystemTime {
    expires_in
        .and_then(|secs| now.checked_add(Duration::from_secs(secs)))
        .unwrap_or(jwt_expires_at)
}

/// Parses IAM's RFC 3339 `expireTime`. The timestamp string is safe to embed
/// in the error — it carries no credential material.
fn parse_generate_access_token_expiry(expire_time: &str) -> Result<SystemTime, String> {
    chrono::DateTime::parse_from_rfc3339(expire_time)
        .map(SystemTime::from)
        .map_err(|err| {
            format!("invalid expireTime `{expire_time}` in the impersonation response: {err}")
        })
}

/// Caps provider error detail at [`ERROR_DETAIL_MAX_CHARS`] characters
/// (char-aligned so multi-byte UTF-8 is never split).
fn truncate_error_detail(detail: &str) -> String {
    detail.chars().take(ERROR_DETAIL_MAX_CHARS).collect()
}

#[cfg(test)]
#[path = "geap_credentials_tests.rs"]
mod tests;
