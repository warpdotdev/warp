//! Authenticated scripting identity types for local Warp control.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::catalog::{ActionKind, PermissionCategory};
use crate::protocol::{ControlError, ErrorCode};

/// Permission scope carried by an authenticated scripting grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptingScope {
    LocalControlReadMetadata,
    LocalControlReadUnderlyingData,
    LocalControlMutateAppState,
    LocalControlMutateMetadataConfiguration,
    LocalControlMutateUnderlyingData,
}

impl ScriptingScope {
    pub fn for_permission(permission: PermissionCategory) -> Self {
        match permission {
            PermissionCategory::ReadMetadata => Self::LocalControlReadMetadata,
            PermissionCategory::ReadUnderlyingData => Self::LocalControlReadUnderlyingData,
            PermissionCategory::MutateAppState => Self::LocalControlMutateAppState,
            PermissionCategory::MutateMetadataConfiguration => {
                Self::LocalControlMutateMetadataConfiguration
            }
            PermissionCategory::MutateUnderlyingData => Self::LocalControlMutateUnderlyingData,
        }
    }
}

/// How an authenticated scripting grant was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ScriptingIdentitySource {
    VerifiedWarpTerminal { session_id: String },
    ExternalApiKey { key_id: String },
}

/// Authenticated scripting grant attached to a local-control credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptingGrant {
    pub source: ScriptingIdentitySource,
    pub subject: String,
    pub scopes: Vec<ScriptingScope>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

impl ScriptingGrant {
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn has_scope(&self, scope: &ScriptingScope) -> bool {
        self.scopes.contains(scope)
    }

    pub fn verify_for_action(
        &self,
        action: ActionKind,
        app_user_subject: Option<&str>,
    ) -> Result<(), ControlError> {
        if self.revoked {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "authenticated scripting grant has been revoked",
            ));
        }
        if self.is_expired() {
            return Err(ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "authenticated scripting grant has expired",
            ));
        }
        if action.metadata().requires_authenticated_user {
            let Some(app_user_subject) = app_user_subject else {
                return Err(ControlError::new(
                    ErrorCode::AuthenticatedUserUnavailable,
                    format!("{} requires a logged-in Warp user", action.as_str()),
                ));
            };
            if self.subject != app_user_subject {
                return Err(ControlError::new(
                    ErrorCode::AuthenticatedUserUnavailable,
                    "authenticated scripting grant subject does not match the selected app user",
                ));
            }
        }
        let required_scope = ScriptingScope::for_permission(action.metadata().permission_category);
        if !self.has_scope(&required_scope) {
            return Err(ControlError::new(
                ErrorCode::InsufficientPermissions,
                format!(
                    "authenticated scripting grant lacks scope for {}",
                    action.as_str()
                ),
            ));
        }
        Ok(())
    }
}

/// Reference to an API key held by the authenticated scripting broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyStorageRef {
    pub key_id: String,
    pub subject: String,
    pub scopes: Vec<ScriptingScope>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

impl ApiKeyStorageRef {
    pub fn status(&self) -> ApiKeyStatus {
        if self.revoked || Utc::now() >= self.expires_at {
            return ApiKeyStatus::NotConfigured;
        }
        ApiKeyStatus::Configured {
            key_id: self.key_id.clone(),
            subject: self.subject.clone(),
            scopes: self.scopes.clone(),
            expires_at: self.expires_at,
        }
    }
}

/// Status of a stored or configured external scripting API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyStatus {
    NotConfigured,
    Configured {
        key_id: String,
        subject: String,
        scopes: Vec<ScriptingScope>,
        expires_at: DateTime<Utc>,
    },
}

/// Summary emitted by `warpctrl auth status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStatusSummary {
    pub instance_id: String,
    pub local_control_enabled: bool,
    pub app_user_logged_in: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_user_subject: Option<String>,
    pub outside_warp_authenticated_grants_enabled: bool,
    pub api_key_status: ApiKeyStatus,
}

/// Raw API-key material with redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeySecret(String);

impl ApiKeySecret {
    pub fn new(secret: String) -> Result<Self, ControlError> {
        if secret.trim().is_empty() {
            return Err(ControlError::new(
                ErrorCode::InvalidParams,
                "API key input cannot be empty",
            ));
        }
        Ok(Self(secret))
    }

    pub fn expose_for_exchange(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKeySecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKeySecret([REDACTED])")
    }
}

pub fn exchange_api_key_stub(secret: &ApiKeySecret) -> Result<ApiKeyStorageRef, ControlError> {
    if secret.expose_for_exchange().len() < 16 {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "API key is too short to validate as an external scripting key",
        ));
    }
    Ok(ApiKeyStorageRef {
        key_id: format!("key_ref_{}", uuid::Uuid::new_v4().simple()),
        subject: "external-api-key-subject".to_owned(),
        scopes: vec![
            ScriptingScope::LocalControlReadMetadata,
            ScriptingScope::LocalControlReadUnderlyingData,
            ScriptingScope::LocalControlMutateAppState,
            ScriptingScope::LocalControlMutateMetadataConfiguration,
            ScriptingScope::LocalControlMutateUnderlyingData,
        ],
        expires_at: Utc::now() + Duration::days(90),
        revoked: false,
    })
}

#[cfg(test)]
#[path = "scripting_tests.rs"]
mod tests;
