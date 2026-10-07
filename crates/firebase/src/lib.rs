use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// Format for error response payloads for Google APIs.
///
/// This error format is standardized across 'v1' Google APIs; its used for both
/// POST /v1/accounts/lookup and POST /v1/token requests.
///
/// This format is documented at https://firebase.google.com/docs/reference/rest/auth#section-error-format
/// as well as https://cloud.google.com/apis/design/errors#error_mapping. In the Google Cloud
/// documentation, refer to the 'HTTP mapping' for the schema (which is canonically defined in
/// protobuf).
///
/// The nested 'errors' field mentioned in the documentation is explicitly omitted since it is
/// deprecated and largely redundant with the top level error `code` and `message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirebaseError {
    pub code: i32,
    pub message: String,
}

impl std::error::Error for FirebaseError {}

impl std::fmt::Display for FirebaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Firebase request failed with status {} and message: {}",
            self.code, self.message
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderUserInfo {
    display_name: Option<String>,
    email: Option<String>,
    provider_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub local_id: String,
    pub photo_url: Option<String>,
    pub screen_name: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    #[serde(default)]
    provider_user_info: Vec<ProviderUserInfo>,
}

impl AccountInfo {
    /// Construct a partial `AccountInfo` given user profile information.
    pub fn from_profile(
        firebase_uid: String,
        photo_url: Option<String>,
        display_name: Option<String>,
        email: Option<String>,
    ) -> Self {
        Self {
            local_id: firebase_uid,
            photo_url,
            screen_name: display_name.clone(),
            display_name,
            email,
            provider_user_info: vec![],
        }
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref().or_else(|| {
            self.provider_user_info
                .iter()
                .find_map(|user_info| user_info.display_name.as_deref())
        })
    }

    pub fn email(&self) -> Result<&str> {
        self.email
            .as_deref()
            .or_else(|| {
                self.provider_user_info
                    .iter()
                    .find_map(|user_info| user_info.email.as_deref())
            })
            .ok_or_else(|| anyhow!("Email address missing from user information"))
    }

    pub fn has_sso_link(&self) -> bool {
        self.provider_user_info
            .iter()
            .any(|user_info| user_info.provider_id.as_deref() == Some("oidc.workos"))
    }
}

/// Format for successful response payload for POST /v1/accounts/lookup request.
///
/// Reference: https://cloud.google.com/identity-platform/docs/use-rest-api#section-get-account-info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAccountInfoResponsePayload {
    users: Vec<AccountInfo>,
}

impl GetAccountInfoResponsePayload {
    pub fn user_account_info(self) -> Result<AccountInfo> {
        self.users.into_iter().next().ok_or_else(|| {
            anyhow!("field `users` was unexpectedly empty in GetAccountInfoResponse")
        })
    }
}

/// Top-level response format for POST /v1/accounts/lookup request (part of the identitytoolkit GCP
/// API).
///
/// Reference: https://cloud.google.com/identity-platform/docs/use-rest-api#section-get-account-info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetAccountInfoResponse {
    Success(GetAccountInfoResponsePayload),
    Error { error: FirebaseError },
}

/// The possible response values from fetching an access token from a refresh token
/// See https://firebase.google.com/docs/reference/rest/auth
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FetchAccessTokenResponse {
    // Note we need to support both camel and snake case because the two different
    // endpoints for converting refresh / custom tokens to access tokens use different conventions.
    Success {
        #[serde(alias = "expiresIn")]
        expires_in: String,

        #[serde(alias = "idToken")]
        id_token: String,

        #[serde(alias = "refreshToken")]
        refresh_token: String,
    },
    Error {
        error: FirebaseError,
    },
}
