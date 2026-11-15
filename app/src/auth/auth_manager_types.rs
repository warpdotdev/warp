use super::auth_view_modal::{AuthRedirectPayload, AuthViewVariant};
use crate::server::server_api::auth::{MintCustomTokenError, UserAuthenticationError};

#[derive(Debug)]
pub enum AuthManagerEvent {
    /// Successfully authenticated a user with no errors.
    AuthComplete,
    /// Failed to authenticate a user, due to a particular `UserAuthenticationError`.
    AuthFailed(UserAuthenticationError),
    /// Failed to create an anonymous user.
    CreateAnonymousUserFailed,
    /// The user chose to skip login entirely (no Firebase user created).
    SkippedLogin,
    /// The user now needs to reauthenticate. If the user needs to reauth, an `AuthFailed`
    /// event might be triggered instead, but there are some code paths where we don't
    /// refresh the entire user, only their token, which is when this event might be emitted.
    NeedsReauth,
    /// The user is anonymous and has attempted to access a login-gated feature or link.
    AttemptedLoginGatedFeature {
        auth_view_variant: AuthViewVariant,
    },
    // The current user is anonymous and the client has received a browser intent to sign in with a different Warp account.
    // Holds an auth payload from the received browser intent.
    LoginOverrideDetected(AuthRedirectPayload),
    /// Failed to mint a new custom token for an anonymous user.
    MintCustomTokenFailed(MintCustomTokenError),
    /// Received a device authorization code as part of the device auth flow.
    ReceivedDeviceAuthorizationCode {
        #[cfg_attr(target_family = "wasm", allow(unused))]
        verification_url: String,
        #[cfg_attr(target_family = "wasm", allow(unused))]
        verification_url_complete: Option<String>,
        #[cfg_attr(target_family = "wasm", allow(unused))]
        user_code: String,
    },
}

pub type LoginGatedFeature = &'static str;

#[derive(Clone, Debug)]
pub struct PersistedCurrentUserInformation {
    pub email: String,
}
