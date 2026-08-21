//! Gemini Enterprise (GEAP) credential state and its lifecycle on the shared
//! [`ApiKeyManager`].
//!
//! The credential itself lives in [`ApiKeyManager`] (the request-building
//! source of truth). This module owns the GEAP-specific value types plus the
//! state transitions, request-time single-flight coordination, and mint-failure
//! cooldown that operate on them, mirroring how [`crate::grok_subscription`]
//! keeps the Grok lifecycle out of [`crate::api_keys`].
//!
//! The network-facing mint lives in the app layer, which has the workspace
//! settings and Warp OIDC access this crate cannot see. It drives this state
//! machine through [`ApiKeyManager::install_geap_refresh_waiter`] and
//! [`ApiKeyManager::take_geap_refresh_waiters`].

use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
#[cfg(not(target_family = "wasm"))]
use futures::channel::oneshot;
use warp_core::ui::Icon;
use warp_multi_agent_api as api;
use warpui_core::ModelContext;

use crate::api_keys::{ApiKeyManager, ApiKeyManagerEvent};

/// Refresh the access token this long before its hard expiry
pub const GEAP_REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

/// How long a failed mint suppresses the request-time blocking refresh.
pub const GEAP_MINT_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, PartialEq, Eq)]
pub struct GeapCredentials {
    access_token: String,
    expires_at: Option<SystemTime>,
}

impl std::fmt::Debug for GeapCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeapCredentials")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl GeapCredentials {
    pub fn new(access_token: String, expires_at: Option<SystemTime>) -> Self {
        Self {
            access_token,
            expires_at,
        }
    }

    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }

    pub fn access_token_for_request(&self) -> Option<&str> {
        (!self.access_token.trim().is_empty()).then_some(self.access_token.as_str())
    }

    pub fn needs_refresh(&self) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at <= SystemTime::now() + GEAP_REFRESH_LEAD_TIME,
            None => false,
        }
    }

    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at <= SystemTime::now(),
            None => false,
        }
    }
}

impl From<GeapCredentials> for api::request::settings::api_keys::GoogleCloudCredentials {
    fn from(credentials: GeapCredentials) -> Self {
        Self {
            access_token: credentials.access_token,
        }
    }
}

/// Outcome of a Gemini Enterprise credential mint, delivered to requests
/// waiting for an expired credential to be replaced before sending.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeapRefreshOutcome {
    Refreshed,
    /// The mint failed; the stored credential is unchanged.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeapFederation {
    DirectWif,
    ServiceAccount { email: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeapMintBinding {
    pub user_uid: String,
    pub audience: String,
    pub federation: GeapFederation,
}

// Keep every message static: `detail` carries the raw Google STS/IAM response
// body, so interpolating it would leak it to Sentry via `report_error!`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadGeapCredentialsError {
    #[error("failed to mint a Warp identity token")]
    MintIdentityToken { detail: String },
    #[error("STS token exchange failed")]
    ExchangeToken { status: Option<u16>, detail: String },
    #[error("service account impersonation failed")]
    ImpersonateServiceAccount { status: Option<u16>, detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GeapCredentialsState {
    #[default]
    Missing,
    Disabled,
    Unconfigured,
    Refreshing {
        previous: Option<(GeapCredentials, GeapMintBinding)>,
    },
    Loaded {
        credentials: GeapCredentials,
        loaded_at: SystemTime,
        minted_for: GeapMintBinding,
    },
    Failed {
        error: LoadGeapCredentialsError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeapRecoveryAction {
    Retry,
    ContactAdmin,
}

/// A 4xx status (other than 429 Too Many Requests) from a Google auth leg means
/// a configuration/permission problem an admin must fix.
fn is_admin_config_status(status: Option<u16>) -> bool {
    match status {
        Some(code) => (400..500).contains(&code) && code != 429,
        None => false,
    }
}

impl LoadGeapCredentialsError {
    pub fn user_facing(&self) -> (String, String, GeapRecoveryAction) {
        match self {
            Self::MintIdentityToken { .. } => (
                "Couldn't authenticate with Warp".to_string(),
                "Warp couldn't create Gemini Enterprise credentials. Check your network \
                 connection and that you're signed in to Warp, then try again."
                    .to_string(),
                GeapRecoveryAction::Retry,
            ),
            Self::ExchangeToken { status, .. } if is_admin_config_status(*status) => (
                "Gemini Enterprise is misconfigured".to_string(),
                "Google rejected Warp's identity token. Ask your team admin to verify \
                 that the Workload Identity Federation audience exactly matches the provider's \
                 full resource name, the provider trusts Warp's OIDC issuer, and its attribute \
                 condition allows this workspace."
                    .to_string(),
                GeapRecoveryAction::ContactAdmin,
            ),
            Self::ExchangeToken { .. } => (
                "Couldn't reach Google to authorize Gemini Enterprise".to_string(),
                "Google's token service was unavailable. This is usually temporary — try again \
                 in a moment."
                    .to_string(),
                GeapRecoveryAction::Retry,
            ),
            Self::ImpersonateServiceAccount { status, .. } if is_admin_config_status(*status) => (
                "Gemini Enterprise service account access is misconfigured".to_string(),
                "Warp couldn't obtain credentials for the service account configured by your \
                 team admin. Ask them to verify the service account email, confirm the Warp \
                 workload identity has the Workload Identity User role on that service account, \
                 and ensure the IAM Service Account Credentials API is enabled."
                    .to_string(),
                GeapRecoveryAction::ContactAdmin,
            ),
            Self::ImpersonateServiceAccount { .. } => (
                "Couldn't reach Google to authorize Gemini Enterprise".to_string(),
                "Google's IAM service was unavailable while authorizing the service account. \
                 This is usually temporary — try again in a moment."
                    .to_string(),
                GeapRecoveryAction::Retry,
            ),
        }
    }

    pub fn recovery_action(&self) -> GeapRecoveryAction {
        self.user_facing().2
    }
}

fn format_status_timestamp(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    if datetime.date_naive() == Local::now().date_naive() {
        datetime.format("%-I:%M %p").to_string()
    } else {
        datetime.format("%b %-d at %-I:%M %p").to_string()
    }
}

fn refresh_scheduled_at(expires_at: SystemTime) -> SystemTime {
    expires_at
        .checked_sub(GEAP_REFRESH_LEAD_TIME)
        .unwrap_or(expires_at)
}

impl GeapCredentialsState {
    pub fn user_facing_components(&self) -> (String, String, Icon) {
        match self {
            Self::Missing => (
                "Gemini Enterprise credentials not loaded".to_string(),
                "Warp hasn't loaded your Gemini Enterprise credentials yet.".to_string(),
                Icon::Key,
            ),
            Self::Disabled => (
                "Gemini Enterprise disabled".to_string(),
                "Warp will not load Gemini Enterprise credentials until it's enabled by you or \
                 your team admin."
                    .to_string(),
                Icon::Key,
            ),
            Self::Unconfigured => (
                "Gemini Enterprise setup incomplete".to_string(),
                "Your team admin needs to configure the Workload Identity Federation audience \
                before Warp can load credentials."
                    .to_string(),
                Icon::AlertTriangle,
            ),
            Self::Refreshing { .. } => (
                "Refreshing credentials...".to_string(),
                "Loading your Gemini Enterprise credentials into Warp".to_string(),
                Icon::RefreshCw04,
            ),
            Self::Loaded {
                credentials,
                loaded_at,
                ..
            } => (
                "Credentials loaded".to_string(),
                match credentials.expires_at() {
                    Some(expires_at) => format!(
                        "Loaded at {} · Refresh scheduled for {}",
                        format_status_timestamp(*loaded_at),
                        format_status_timestamp(refresh_scheduled_at(expires_at))
                    ),
                    None => format!("Loaded at {}", format_status_timestamp(*loaded_at)),
                },
                Icon::CheckCircleBroken,
            ),
            Self::Failed { error } => {
                let (title, description, _) = error.user_facing();
                (title, description, Icon::AlertTriangle)
            }
        }
    }

    pub fn recovery_action(&self) -> Option<GeapRecoveryAction> {
        match self {
            Self::Failed { error } => Some(error.recovery_action()),
            // An incomplete admin setup can't be retried from the client, so it
            // routes to the same admin-guidance affordance as a config failure.
            Self::Unconfigured => Some(GeapRecoveryAction::ContactAdmin),
            Self::Missing | Self::Disabled | Self::Refreshing { .. } | Self::Loaded { .. } => None,
        }
    }

    pub fn requires_admin_action(&self) -> bool {
        self.recovery_action() == Some(GeapRecoveryAction::ContactAdmin)
    }
}

impl ApiKeyManager {
    pub fn set_geap_credentials_state(
        &mut self,
        state: GeapCredentialsState,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.geap_credentials_state == state {
            return;
        }
        self.geap_credentials_state = state;
        ctx.emit(ApiKeyManagerEvent::KeysUpdated);
    }

    pub fn geap_credentials_state(&self) -> &GeapCredentialsState {
        &self.geap_credentials_state
    }

    /// The Gemini Enterprise credential to attach to an outgoing request, or
    /// `None` when there is nothing attachable.
    ///
    /// A credential attaches only when it was minted for this same
    /// (user, audience, service account). A re-mint in flight keeps
    /// serving the previous credential, and possibly-expired credentials are
    /// still attached.
    pub fn geap_credentials_for_request(
        &self,
        binding: &GeapMintBinding,
    ) -> Option<api::request::settings::api_keys::GoogleCloudCredentials> {
        match self.geap_credentials_state {
            GeapCredentialsState::Loaded {
                ref credentials,
                ref minted_for,
                ..
            } if minted_for == binding => credentials
                .access_token_for_request()
                .map(|_| credentials.clone().into()),
            GeapCredentialsState::Refreshing {
                previous: Some((ref credentials, ref minted_for)),
            } if minted_for == binding => credentials
                .access_token_for_request()
                .map(|_| credentials.clone().into()),
            _ => None,
        }
    }

    /// Whether a request whose model may route to Gemini Enterprise should
    /// block on a mint before sending.
    ///
    /// True only when the stored credential was minted for this same binding
    /// and is at or past hard expiry, and no mint has failed recently.
    #[cfg(not(target_family = "wasm"))]
    pub fn geap_expired_refresh_eligibility(&self, binding: &GeapMintBinding) -> bool {
        if self.geap_mint_recently_failed() {
            return false;
        }
        match self.geap_credentials_state() {
            GeapCredentialsState::Loaded {
                credentials,
                minted_for,
                ..
            } if minted_for == binding => credentials.is_expired(),
            GeapCredentialsState::Refreshing {
                previous: Some((credentials, minted_for)),
            } if minted_for == binding => credentials.is_expired(),
            _ => false,
        }
    }

    /// Ensures one mint is in flight for an expired GEAP credential and returns
    /// a receiver for its completion, or `None` when no wait is warranted.
    #[cfg(not(target_family = "wasm"))]
    pub fn begin_expired_geap_refresh<F>(
        &mut self,
        binding: &GeapMintBinding,
        ctx: &mut ModelContext<Self>,
        start_refresh: F,
    ) -> Option<oneshot::Receiver<GeapRefreshOutcome>>
    where
        F: FnOnce(&mut Self, oneshot::Sender<GeapRefreshOutcome>, &mut ModelContext<Self>),
    {
        if !self.geap_expired_refresh_eligibility(binding) {
            return None;
        }

        let (tx, rx) = oneshot::channel();
        match self.geap_refresh_waiters.as_mut() {
            // A mint is genuinely in flight, since the waiter list is installed
            // by the mint itself. Attach to it.
            Some(waiters) => waiters.push(tx),
            None => start_refresh(self, tx, ctx),
        }
        Some(rx)
    }

    /// Opens the single-flight window for a mint that is about to start.
    #[cfg(not(target_family = "wasm"))]
    pub fn install_geap_refresh_waiter(
        &mut self,
        waiter: Option<oneshot::Sender<GeapRefreshOutcome>>,
    ) {
        self.geap_refresh_waiters = Some(waiter.into_iter().collect());
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn take_geap_refresh_waiters(&mut self) -> Vec<oneshot::Sender<GeapRefreshOutcome>> {
        self.geap_refresh_waiters.take().unwrap_or_default()
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn record_geap_mint_failure(&mut self) {
        self.geap_last_mint_failure = Some(SystemTime::now());
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn clear_geap_mint_failure(&mut self) {
        self.geap_last_mint_failure = None;
    }

    #[cfg(not(target_family = "wasm"))]
    fn geap_mint_recently_failed(&self) -> bool {
        self.geap_last_mint_failure.is_some_and(|failed_at| {
            SystemTime::now()
                .duration_since(failed_at)
                .is_ok_and(|elapsed| elapsed < GEAP_MINT_FAILURE_COOLDOWN)
        })
    }
}

#[cfg(test)]
#[path = "geap_credentials_tests.rs"]
mod tests;
