use std::time::{Duration, SystemTime};

use chrono::{DateTime, Local};
use warp_core::ui::Icon;
use warp_multi_agent_api as api;

/// Refresh the access token this long before its hard expiry
pub const GEAP_REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

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
}

impl From<GeapCredentials> for api::request::settings::api_keys::GoogleCloudCredentials {
    fn from(credentials: GeapCredentials) -> Self {
        Self {
            access_token: credentials.access_token,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadGeapCredentialsError {
    MintIdentityToken { detail: String },
    ExchangeToken { status: Option<u16>, detail: String },
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

#[cfg(test)]
#[path = "geap_credentials_tests.rs"]
mod tests;
