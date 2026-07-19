use std::time::{Duration, SystemTime};

use ai::api_keys::{
    ApiKeyManager, GeapCredentials, GeapCredentialsState, GeapFederation, GeapMintBinding,
    LoadGeapCredentialsError, GEAP_REFRESH_LEAD_TIME,
};
use serde::{Deserialize, Serialize};
use vec1::vec1;
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warp_managed_secrets::client::{IdentityTokenOptions, TaskIdentityToken};
use warp_managed_secrets::ManagedSecretManager;
use warpui::r#async::Timer;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

const GEAP_IDENTITY_TOKEN_DURATION: Duration = Duration::from_secs(60 * 60);

/// Floor on the proactive refresh timer delay so a near-expired store
/// cannot spin mint -> store -> re-mint as a hot loop;
const GEAP_MIN_TIMER_DELAY: Duration = Duration::from_secs(60);

const STS_TOKEN_URL: &str = "https://sts.googleapis.com/v1/token";
const IAM_GENERATE_ACCESS_TOKEN_URL: &str =
    "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{sa_email}:generateAccessToken";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";
const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
const SA_ACCESS_TOKEN_LIFETIME: &str = "3600s";

const GEAP_MINT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GeapPolicy {
    Disabled,
    Unconfigured,
    Mintable(GeapMintBinding),
}

impl GeapPolicy {
    pub(crate) fn mint_binding(self) -> Option<GeapMintBinding> {
        match self {
            GeapPolicy::Mintable(binding) => Some(binding),
            GeapPolicy::Disabled | GeapPolicy::Unconfigured => None,
        }
    }
}

fn geap_mint_binding_from_parts(
    user_uid: String,
    gcp_audience: Option<&str>,
    gcp_sa_email: Option<&str>,
) -> Option<GeapMintBinding> {
    let audience = gcp_audience.map(str::trim).unwrap_or_default();
    if audience.is_empty() {
        return None;
    }
    let federation = match gcp_sa_email
        .map(str::trim)
        .filter(|sa_email| !sa_email.is_empty())
    {
        Some(email) => GeapFederation::ServiceAccount {
            email: email.to_string(),
        },
        None => GeapFederation::DirectWif,
    };
    Some(GeapMintBinding {
        user_uid,
        audience: audience.to_string(),
        federation,
    })
}

pub(crate) fn current_geap_policy(app: &AppContext) -> GeapPolicy {
    if !FeatureFlag::GeminiEnterprise.is_enabled() {
        return GeapPolicy::Disabled;
    }
    let user_workspaces = UserWorkspaces::as_ref(app);
    if !user_workspaces.is_gemini_enterprise_credentials_enabled(app) {
        return GeapPolicy::Disabled;
    }
    let Some(user_id) = AuthStateProvider::as_ref(app).get().user_id() else {
        return GeapPolicy::Disabled;
    };
    let Some(settings) = user_workspaces.gemini_enterprise_host_settings() else {
        return GeapPolicy::Unconfigured;
    };
    match geap_mint_binding_from_parts(
        user_id.as_string(),
        settings.gcp_audience.as_deref(),
        settings.gcp_sa_email.as_deref(),
    ) {
        Some(binding) => GeapPolicy::Mintable(binding),
        None => GeapPolicy::Unconfigured,
    }
}

pub trait GeapCredentialRefresher {
    fn subscribe_to_geap_settings_changes(&mut self, ctx: &mut ModelContext<Self>)
    where
        Self: Sized;
}

impl GeapCredentialRefresher for ApiKeyManager {
    fn subscribe_to_geap_settings_changes(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |manager, _, event, ctx| {
            if matches!(
                event,
                UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess
                    | UserWorkspacesEvent::TeamsChanged
            ) {
                refresh_geap_credentials(manager, ctx);
            }
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |manager, _, event, ctx| {
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

pub(crate) fn force_refresh_geap_credentials(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    refresh_geap_credentials_with_options(manager, true, ctx);
}

/// Request-time safety net. The triggering request is never delayed —
/// it carries the currently stored token.
pub(crate) fn refresh_geap_credentials_if_needed(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let binding = match current_geap_policy(ctx) {
        GeapPolicy::Disabled | GeapPolicy::Unconfigured => return,
        GeapPolicy::Mintable(binding) => binding,
    };
    let needs_mint = match manager.geap_credentials_state() {
        GeapCredentialsState::Refreshing { .. } => false,
        GeapCredentialsState::Loaded {
            credentials,
            minted_for,
            ..
        } => *minted_for != binding || credentials.needs_refresh(),
        GeapCredentialsState::Missing
        | GeapCredentialsState::Unconfigured
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
    let minted_for = match current_geap_policy(ctx) {
        GeapPolicy::Disabled => {
            manager.set_geap_credentials_state(GeapCredentialsState::Disabled, ctx);
            return;
        }
        GeapPolicy::Unconfigured => {
            manager.set_geap_credentials_state(GeapCredentialsState::Unconfigured, ctx);
            return;
        }
        GeapPolicy::Mintable(binding) => binding,
    };
    if matches!(
        manager.geap_credentials_state(),
        GeapCredentialsState::Refreshing { .. }
    ) {
        return;
    }
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
        minted_for.audience
    );
    manager.set_geap_credentials_state(GeapCredentialsState::Refreshing { previous }, ctx);

    // Leg 1: every mint — initial or re-mint, timer/trigger/forced — starts
    // with a brand-new Warp OIDC JWT, consumed exactly once by the STS
    // exchange below and never cached across mints.
    let token_future = ManagedSecretManager::handle(ctx)
        .as_ref(ctx)
        .issue_task_identity_token(IdentityTokenOptions {
            audience: minted_for.audience.clone(),
            requested_duration: GEAP_IDENTITY_TOKEN_DURATION,
            subject_template: vec1!["principal".to_string()],
        });
    let binding = minted_for.clone();
    let _ = ctx.spawn(
        async move {
            let identity_token =
                token_future
                    .await
                    .map_err(|err| LoadGeapCredentialsError::MintIdentityToken {
                        detail: format!("{err:#}"),
                    })?;
            exchange_identity_token_for_geap_credentials(identity_token, &binding).await
        },
        move |manager, result, ctx| apply_geap_mint_result(manager, result, minted_for, force, ctx),
    );
}

fn apply_geap_mint_result(
    manager: &mut ApiKeyManager,
    result: Result<GeapCredentials, LoadGeapCredentialsError>,
    minted_for: GeapMintBinding,
    force: bool,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let current_binding = match current_geap_policy(ctx) {
        GeapPolicy::Disabled => {
            log::info!("GEAP: gate flipped off mid-mint; discarding the mint result");
            manager.set_geap_credentials_state(GeapCredentialsState::Disabled, ctx);
            return;
        }
        GeapPolicy::Unconfigured => {
            log::info!("GEAP: gate unconfigured mid-mint; discarding the mint result");
            manager.set_geap_credentials_state(GeapCredentialsState::Unconfigured, ctx);
            return;
        }
        GeapPolicy::Mintable(binding) => binding,
    };
    let previous = match manager.geap_credentials_state() {
        GeapCredentialsState::Refreshing {
            previous: Some((credentials, binding)),
        } if *binding == current_binding => Some((credentials.clone(), binding.clone())),
        _ => None,
    };

    // The user/account or federation config changed while the mint was in
    // flight. Discard it and immediately re-mint under the current binding.
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
            report_error!("GEAP: credential mint failed", extra: { "error" => ?err });
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
                        GeapCredentialsState::Failed { error: err },
                        ctx,
                    );
                }
            }
        }
    }
}

/// A one-shot timer that re-mints [`GEAP_REFRESH_LEAD_TIME`] before the
/// loaded token's expiry. The timer is armed once per token — no periodic
/// polling; the process wakes exactly once per token lifetime.
fn schedule_geap_token_refresh(manager: &mut ApiKeyManager, ctx: &mut ModelContext<ApiKeyManager>) {
    let GeapCredentialsState::Loaded { credentials, .. } = manager.geap_credentials_state() else {
        return;
    };
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
    expire_time: String,
}

/// Legs 2 and 3 of the mint: exchanges the Warp OIDC JWT at Google STS for a
/// federated token, then (when configured) impersonates the workspace's
/// service account for the final ~1h access token.
async fn exchange_identity_token_for_geap_credentials(
    identity_token: TaskIdentityToken,
    binding: &GeapMintBinding,
) -> Result<GeapCredentials, LoadGeapCredentialsError> {
    // STS token exchange.
    let response = http_client::Client::new()
        .post(STS_TOKEN_URL)
        .form(&StsTokenExchangeRequest {
            grant_type: TOKEN_EXCHANGE_GRANT_TYPE,
            audience: &binding.audience,
            scope: CLOUD_PLATFORM_SCOPE,
            requested_token_type: ACCESS_TOKEN_TYPE,
            subject_token: &identity_token.token,
            subject_token_type: ID_TOKEN_TYPE,
        })
        .timeout(GEAP_MINT_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|err| LoadGeapCredentialsError::ExchangeToken {
            status: None,
            detail: format!("request failed: {err:#}"),
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LoadGeapCredentialsError::ExchangeToken {
            status: Some(status.as_u16()),
            detail: body,
        });
    }
    let sts_response: StsTokenExchangeResponse =
        response
            .json()
            .await
            .map_err(|err| LoadGeapCredentialsError::ExchangeToken {
                status: None,
                detail: format!("failed to parse the STS response: {err:#}"),
            })?;
    log::info!(
        "GEAP: STS exchange succeeded (audience={})",
        binding.audience
    );

    let federated_expires_at = sts_expires_at(
        sts_response.expires_in,
        SystemTime::from(identity_token.expires_at),
        SystemTime::now(),
    );

    let GeapFederation::ServiceAccount { email: sa_email } = &binding.federation else {
        // DirectWif: no impersonation — the federated STS token is used directly.
        return Ok(GeapCredentials::new(
            sts_response.access_token,
            Some(federated_expires_at),
        ));
    };

    // Leg 3: SA impersonation. IAM authorizes this only if the pool identity
    // holds `roles/iam.workloadIdentityUser` on the SA — the customer's
    // control point for who may become the SA.
    let url = IAM_GENERATE_ACCESS_TOKEN_URL.replace("{sa_email}", sa_email);
    let response = http_client::Client::new()
        .post(&url)
        .bearer_auth(&sts_response.access_token)
        .json(&GenerateAccessTokenRequest {
            scope: vec![CLOUD_PLATFORM_SCOPE.to_string()],
            lifetime: SA_ACCESS_TOKEN_LIFETIME.to_string(),
        })
        .timeout(GEAP_MINT_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|err| LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: None,
            detail: format!("request failed: {err:#}"),
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: Some(status.as_u16()),
            detail: body,
        });
    }
    let impersonation: GenerateAccessTokenResponse = response.json().await.map_err(|err| {
        LoadGeapCredentialsError::ImpersonateServiceAccount {
            status: None,
            detail: format!("failed to parse the impersonation response: {err:#}"),
        }
    })?;
    let expires_at =
        parse_generate_access_token_expiry(&impersonation.expire_time).map_err(|detail| {
            LoadGeapCredentialsError::ImpersonateServiceAccount {
                status: None,
                detail,
            }
        })?;
    log::info!(
        "GEAP: service account impersonation succeeded (audience={})",
        binding.audience
    );
    Ok(GeapCredentials::new(
        impersonation.access_token,
        Some(expires_at),
    ))
}

fn sts_expires_at(
    expires_in: Option<u64>,
    jwt_expires_at: SystemTime,
    now: SystemTime,
) -> SystemTime {
    expires_in
        .and_then(|secs| now.checked_add(Duration::from_secs(secs)))
        .unwrap_or(jwt_expires_at)
}

fn parse_generate_access_token_expiry(expire_time: &str) -> Result<SystemTime, String> {
    chrono::DateTime::parse_from_rfc3339(expire_time)
        .map(SystemTime::from)
        .map_err(|err| {
            format!("invalid expireTime `{expire_time}` in the impersonation response: {err}")
        })
}

#[cfg(test)]
#[path = "geap_credentials_tests.rs"]
mod tests;
