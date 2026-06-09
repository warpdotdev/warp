use thiserror::Error;
#[cfg(test)]
pub use warp_server_client::auth::MockAuthClient;
pub use warp_server_client::auth::{
    AuthClient, FetchUserResult, MintCustomTokenError, SyncedUserSettings, UserAuthenticationError,
};

#[derive(Error, Debug)]
/// Error type when creating anonymous users.
pub enum AnonymousUserCreationError {
    #[error("The network request to create the anonymous user failed")]
    CreationFailed,

    #[error("Received a user facing error: {0}")]
    UserFacingError(String),

    /// Failure that occurs after the user is created, but the ID token could not be fetched.
    #[error("The user was created, but the ID token could not be fetched")]
    UserAuthenticationFailed(#[from] UserAuthenticationError),

    #[error("Failed to create anonymous user with unknown error")]
    Unknown,
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
