use anyhow::{Result, anyhow, bail};
use instant::Duration;
use warp_server_auth::credentials::{AuthToken, Credentials, FirebaseToken, LoginToken};

use super::UserAuthenticationError;

pub(super) const AUTH_UNAVAILABLE: &str =
    "Warp account authentication is unavailable in the local terminal";

/// The local terminal retains the server client's interface without a credential transport.
pub struct AuthSession;

impl AuthSession {
    pub fn allowed_to_refresh_token(&self) -> bool {
        false
    }

    pub async fn get_or_refresh_access_token(&self) -> Result<AuthToken> {
        bail!(AUTH_UNAVAILABLE)
    }

    pub async fn exchange_credentials(
        &self,
        _: LoginToken,
    ) -> Result<Credentials, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }

    pub async fn request_device_code(
        &self,
    ) -> Result<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }

    pub async fn exchange_device_access_token(
        &self,
        _: &oauth2::StandardDeviceAuthorizationResponse,
        _: Duration,
    ) -> Result<FirebaseToken, UserAuthenticationError> {
        Err(anyhow!(AUTH_UNAVAILABLE).into())
    }
}

#[cfg(test)]
#[path = "local_session_tests.rs"]
mod tests;
