//! Credential request, issuance, and validation types for local control.
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::InstanceId;
use crate::protocol::{ActionKind, ControlError, ErrorCode};

/// Bearer token used to authorize a single scoped local-control credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken(String);

impl AuthToken {
    /// Generates a bearer secret from 32 bytes of operating-system CSPRNG output.
    ///
    /// Local-control bearer tokens are authentication material, so they use
    /// `OsRng` instead of a deterministic or fast userspace PRNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn from_secret(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub fn secret(&self) -> &str {
        &self.0
    }

    pub fn authorization_value(&self) -> String {
        format!("Bearer {}", self.0)
    }

    pub fn from_authorization_header(value: Option<&str>) -> Result<Self, ControlError> {
        let Some(value) = value else {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization header is required",
            ));
        };
        let Some(token) = value.strip_prefix("Bearer ") else {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization header must use the Bearer scheme",
            ));
        };
        Ok(Self::from_secret(token))
    }

    pub fn verify_authorization_header(&self, value: Option<&str>) -> Result<(), ControlError> {
        let token = Self::from_authorization_header(value)?;
        if token != *self {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization token is invalid",
            ));
        }
        Ok(())
    }
}

/// Request for a short-lived credential scoped to one exact action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRequest {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub action: ActionKind,
}

impl CredentialRequest {
    pub fn new(action: ActionKind) -> Self {
        Self {
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            action,
        }
    }
}

/// Client-facing credential response containing a bearer secret and its grant metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedCredential {
    pub bearer_token: String,
    pub grant: CredentialGrant,
}

impl ScopedCredential {
    pub fn authorization_value(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }
}

/// Authorization grant issued by the localhost server running inside Warp for a
/// single action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub credential_id: String,
    pub instance_id: InstanceId,
    pub action: ActionKind,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl CredentialGrant {
    pub fn new(instance_id: InstanceId, action: ActionKind, ttl: Duration) -> Self {
        let issued_at = Utc::now();
        Self {
            credential_id: format!("cred_{}", Uuid::new_v4().simple()),
            instance_id,
            action,
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn verify_for_action(
        &self,
        instance_id: &InstanceId,
        action: ActionKind,
    ) -> Result<(), ControlError> {
        if self.is_expired() {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential has expired",
            ));
        }
        if &self.instance_id != instance_id {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential belongs to a different Warp instance",
            ));
        }
        if self.action != action {
            return Err(ControlError::new(
                ErrorCode::InsufficientPermissions,
                format!(
                    "credential for {} cannot invoke {}",
                    self.action.as_str(),
                    action.as_str()
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
