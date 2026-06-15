use std::time::{Duration, SystemTime};

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
