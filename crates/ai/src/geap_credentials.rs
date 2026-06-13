//! In-memory credential state for Gemini Enterprise (GEAP) BYOLLM requests.
//!
//! The Warp client mints a short-lived Google Cloud access token through
//! Workload Identity Federation (Warp OIDC JWT -> STS exchange -> optional
//! service-account impersonation) and attaches it to eligible agent requests.
//! This module owns the pure, request-facing state: the token + expiry, the
//! mint binding tying a token to the (user, federation config) it was minted
//! for, and the state machine stored on
//! [`ApiKeyManager`](crate::api_keys::ApiKeyManager). The network-facing
//! mint/refresh lifecycle lives in the app layer
//! (`app/src/ai/geap_credentials.rs`), which has visibility into workspace
//! policy and the Warp session.

use std::time::{Duration, SystemTime};

use warp_multi_agent_api as api;

/// Refresh the access token this long before its hard expiry so a request
/// never races the expiration. Possibly-expired tokens are still sent (Google
/// is the authority on validity), so this lead time is purely about keeping
/// the token *fresh*, never about when it stops being *sent*.
pub const GEAP_REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

/// A short-lived Google Cloud access token minted through WIF. Lives only in
/// memory and is never persisted; the fields are private so the only egress of
/// the raw token is the conversion into the request wire type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeapCredentials {
    access_token: String,
    expires_at: Option<SystemTime>,
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

    /// The access token whenever it is non-empty — regardless of expiry.
    /// Possibly-expired tokens are still sent so Google stays the final
    /// authority on validity.
    /// The background refresh layers replace stale tokens. Silently dropping a
    /// stale token would silently downgrade the request to another route
    /// instead of surfacing a recoverable 401.
    pub fn access_token_for_request(&self) -> Option<&str> {
        (!self.access_token.trim().is_empty()).then_some(self.access_token.as_str())
    }

    /// Whether the token is within [`GEAP_REFRESH_LEAD_TIME`] of expiry — or
    /// already past it — and should be re-minted. Used by the skip-if-valid
    /// refresh guard, the proactive timer, and the request-time safety net.
    /// An unknown expiry never reports as needing a refresh — there is no
    /// expiry signal to act on; for GEAP the expiry is always known by construction).
    pub fn needs_refresh(&self) -> bool {
        match self.expires_at {
            Some(expires_at) => expires_at <= SystemTime::now() + GEAP_REFRESH_LEAD_TIME,
            None => false,
        }
    }
}

/// The only egress of the raw token: the conversion into the request wire
/// type. Intentionally the entire shape — no token type (bearer is the
/// transport default) and no expiry (the server cannot refresh; Google is the
/// source of truth for staleness).
impl From<GeapCredentials> for api::request::settings::api_keys::GoogleCloudCredentials {
    fn from(credentials: GeapCredentials) -> Self {
        Self {
            access_token: credentials.access_token,
        }
    }
}

/// The identity + federation config a GEAP token was minted against: the Warp
/// user uid plus the `(audience, sa_email)` admin config at mint start.
///
/// The attach-time read treats a binding mismatch as not-loaded, so a token
/// minted for a different account (sign-out/account switch) or against a stale
/// federation config (admin changed audience/SA) is never attached, and is
/// replaced on the next trigger instead of surviving until expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeapMintBinding {
    pub user_uid: String,
    pub audience: String,
    pub sa_email: Option<String>,
}

impl GeapMintBinding {
    /// Whether this binding matches the expected binding computed at the
    /// request build site.
    pub fn matches(&self, gate: &GeapRequestGate) -> bool {
        self.user_uid == gate.user_uid
            && self.audience == gate.audience
            && self.sa_email == gate.sa_email
    }
}

/// The expected mint binding for the current request, computed at the request
/// build site while the GEAP enablement gate is on. Passing it into
/// [`ApiKeyManager::api_keys_for_request`](crate::api_keys::ApiKeyManager::api_keys_for_request)
/// keeps that function a pure `&self` read: the caller owns the policy
/// evaluation (auth, admin availability, enablement mode, member toggle), and
/// `None` means the gate is off and GEAP credentials are skipped entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeapRequestGate {
    pub user_uid: String,
    pub audience: String,
    pub sa_email: Option<String>,
}

/// GEAP credential state stored on
/// [`ApiKeyManager`](crate::api_keys::ApiKeyManager), mirroring
/// [`AwsCredentialsState`](crate::aws_credentials::AwsCredentialsState) in
/// shape with one deliberate divergence taken from the Grok subscription
/// lifecycle: [`Self::Refreshing`] carries the previous credentials so
/// requests keep authenticating during the ~1-3s re-mint — tokens stay until
/// replaced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GeapCredentialsState {
    /// GEAP is enabled but unconfigured (no audience) or no mint has happened
    /// yet. Cold start rests here until the first trigger fires.
    #[default]
    Missing,
    /// The enablement gate is off: admin off, enforced off, member opted out,
    /// or logged out. No token is retained while disabled.
    Disabled,
    /// A mint is in flight. `previous` (token + its binding) keeps serving
    /// requests until the new token replaces it; `None` during the very first
    /// mint, when there is nothing to serve yet.
    Refreshing {
        previous: Option<(GeapCredentials, GeapMintBinding)>,
    },
    /// A token is loaded and attachable for requests whose expected binding
    /// matches `minted_for`.
    Loaded {
        credentials: GeapCredentials,
        loaded_at: SystemTime,
        minted_for: GeapMintBinding,
    },
    /// A mint failed with no previous token to keep serving (the first mint,
    /// or a forced refresh where the user needs visible feedback).
    Failed { message: String },
}
