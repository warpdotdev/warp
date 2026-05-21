//! Representation of Warp user credentials.
//!
//! The primary representation is [`Credentials`], which is the source of truth for how a user is
//! authenticated to Warp.
//!
//! Credentials can be split into two halves:
//! * [`LoginToken`], which is a long-lived token that we use to fetch user information.
//!   When using Firebase, this is an OAuth2 refresh token.
//! * [`AuthToken`], which is a short-lived token that's included in all other server requests.
//!   When using Firebase, this is an OAuth2 access token.
use warp_graphql::object_permissions::OwnerType;

use super::user::FirebaseAuthTokens;

/// Represents the different ways a user can authenticate with Warp.
#[derive(Clone, Debug)]
pub enum Credentials {
    /// Firebase authentication with ID token and refresh token.
    Firebase(FirebaseAuthTokens),
    /// API key for direct server authentication.
    ApiKey {
        key: String,
        /// The owner type for this API key. Only set after user info is fetched from the server.
        owner_type: Option<OwnerType>,
    },
    /// Request-scoped or externally managed bearer token.
    Bearer(String),
    /// Authentication derived from an ambient browser session cookie.
    SessionCookie,
    /// Test credentials used in unit tests, integration tests, and skip_login builds.
    #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
    Test,
}

impl Credentials {
    /// Returns the Firebase auth tokens if this is a Firebase credential.
    pub fn as_firebase(&self) -> Option<&FirebaseAuthTokens> {
        match self {
            Credentials::Firebase(tokens) => Some(tokens),
            Credentials::ApiKey { .. } => None,
            Credentials::Bearer(_) => None,
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the API key string if this is an API key credential.
    pub fn as_api_key(&self) -> Option<&str> {
        match self {
            Credentials::ApiKey { key, .. } => Some(key),
            Credentials::Firebase(_) => None,
            Credentials::Bearer(_) => None,
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the owner type if this is an API key credential.
    pub fn api_key_owner_type(&self) -> Option<OwnerType> {
        match self {
            Credentials::ApiKey { owner_type, .. } => *owner_type,
            Credentials::Firebase(_) => None,
            Credentials::Bearer(_) => None,
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the Firebase refresh token if this is a Firebase credential.
    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            Credentials::Firebase(tokens) => Some(&tokens.refresh_token),
            Credentials::ApiKey { .. } => None,
            Credentials::Bearer(_) => None,
            Credentials::SessionCookie => None,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }

    /// Returns the short-lived token to use in HTTP requests to the server.
    pub fn bearer_token(&self) -> AuthToken {
        match self {
            Credentials::Firebase(tokens) => AuthToken::Firebase(tokens.id_token.clone()),
            Credentials::ApiKey { key, .. } => AuthToken::ApiKey(key.clone()),
            Credentials::Bearer(token) => AuthToken::Bearer(token.clone()),
            Credentials::SessionCookie => AuthToken::NoAuth,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => AuthToken::NoAuth,
        }
    }
    /// Returns whether these credentials are externally managed and should not trigger local token
    /// refresh or reauth flows.
    pub fn is_externally_managed(&self) -> bool {
        match self {
            Credentials::Bearer(_) => true,
            Credentials::Firebase(_) | Credentials::ApiKey { .. } | Credentials::SessionCookie => {
                false
            }
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => false,
        }
    }

    /// Get the long-lived login token for these credentials. Returns `None` if there is no such token.
    pub fn login_token(&self) -> Option<LoginToken> {
        match self {
            Credentials::Firebase(tokens) => Some(LoginToken::Firebase(FirebaseToken::Refresh(
                RefreshToken::new(&tokens.refresh_token),
            ))),
            Credentials::ApiKey { key, .. } => Some(LoginToken::ApiKey(key.clone())),
            Credentials::Bearer(_) => None,
            Credentials::SessionCookie => Some(LoginToken::SessionCookie),
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            Credentials::Test => None,
        }
    }
}

/// Represents different types of authentication tokens.
#[derive(Debug, Clone)]
pub enum AuthToken {
    /// Firebase short-lived access token.
    Firebase(String),
    /// API key for direct server authentication.
    ApiKey(String),
    /// Request-scoped or externally managed bearer token.
    Bearer(String),
    /// No authentication token available (e.g. session cookie auth or test credentials).
    #[cfg_attr(
        not(any(test, feature = "integration_tests", feature = "skip_login")),
        allow(dead_code)
    )]
    NoAuth,
}

impl AuthToken {
    /// Returns the token string to use in an Authorization header, or `None` if auth is not
    /// header-based (e.g. session cookie) or there is no auth.
    pub fn as_bearer_token(&self) -> Option<&str> {
        match self {
            AuthToken::Firebase(token) => Some(token),
            AuthToken::ApiKey(key) => Some(key),
            AuthToken::Bearer(token) => Some(token),
            AuthToken::NoAuth => None,
        }
    }

    /// Returns the bearer token as an owned string, or `None` if auth is not header-based.
    pub fn bearer_token(&self) -> Option<String> {
        match self {
            AuthToken::Firebase(token) => Some(token.clone()),
            AuthToken::ApiKey(key) => Some(key.clone()),
            AuthToken::Bearer(token) => Some(token.clone()),
            AuthToken::NoAuth => None,
        }
    }
}

/// Long-lived credentials exchanged for user information.
#[derive(Debug)]
pub enum LoginToken {
    /// A Firebase token to be exchanged for auth tokens.
    Firebase(FirebaseToken),
    /// An API key for direct server authentication.
    ApiKey(String),
    /// Authentication derived from an ambient browser session cookie.
    SessionCookie,
}

/// The type of firebase token that can be used to authenticate a user.
/// For logged in users and anonymous users, we use a refresh token.
/// We use a short-lived custom token when we first create and fetch a new anonymous user.
/// In both cases the token can be exchanged for a short lived access token.
#[derive(Debug)]
pub enum FirebaseToken {
    /// The token type for a logged in user.
    Refresh(RefreshToken),

    /// The token type for an anonymous user.
    Custom(String),
}

impl FirebaseToken {
    /// Returns the url for trading this long lived token into an access token.
    pub fn access_token_url(&self, api_key: &str) -> String {
        // See https://firebase.google.com/docs/reference/rest/auth for info on these
        // authentication endpoints.
        match self {
            FirebaseToken::Refresh(_) => {
                format!("https://securetoken.googleapis.com/v1/token?key={api_key}")
            }
            FirebaseToken::Custom(_) => {
                format!("https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key={api_key}")
            }
        }
    }

    /// Returns the POST body for to include when trading this long lived token into an access token.
    pub fn access_token_request_body(&self) -> Vec<(&str, &str)> {
        match self {
            FirebaseToken::Refresh(refresh_token) => vec![
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.get()),
            ],
            FirebaseToken::Custom(custom_token) => {
                vec![("returnSecureToken", "true"), ("token", custom_token)]
            }
        }
    }

    /// Returns the proxy URL for trading this long lived token into an access token.
    /// Used when the initial request to Firebase fails and we want to try and proxy the request
    /// through our server.
    pub fn proxy_url(&self, server_root: &str, api_key: &str) -> String {
        match self {
            FirebaseToken::Refresh(_) => format!("{server_root}/proxy/token?key={api_key}"),
            FirebaseToken::Custom(_) => {
                format!("{server_root}/proxy/customToken?key={api_key}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefreshToken(String);

impl RefreshToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn get(&self) -> &str {
        self.0.as_str()
    }
}
