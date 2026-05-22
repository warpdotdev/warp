use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::InstanceId;
use crate::protocol::{
    ActionKind, ControlError, ErrorCode, ExecutionContextProof, InvocationContext, RiskTier,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken(String);

impl AuthToken {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRequest {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub action: ActionKind,
    pub invocation_context: InvocationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_context_proof: Option<ExecutionContextProof>,
}

impl CredentialRequest {
    pub fn new(action: ActionKind, invocation_context: InvocationContext) -> Self {
        Self {
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            action,
            invocation_context,
            execution_context_proof: None,
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub credential_id: String,
    pub instance_id: InstanceId,
    pub action: ActionKind,
    pub risk_tier: RiskTier,
    pub invocation_context: InvocationContext,
    pub authenticated_user_subject: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl CredentialGrant {
    pub fn new(
        instance_id: InstanceId,
        action: ActionKind,
        invocation_context: InvocationContext,
        ttl: Duration,
    ) -> Self {
        let issued_at = Utc::now();
        Self {
            credential_id: format!("cred_{}", Uuid::new_v4().simple()),
            instance_id,
            action,
            risk_tier: action.metadata().risk_tier,
            invocation_context,
            authenticated_user_subject: None,
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn verify_for_action(&self, action: ActionKind) -> Result<(), ControlError> {
        if Utc::now() >= self.expires_at {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential has expired",
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
        let metadata = action.metadata();
        if metadata.requires_authenticated_user && self.authenticated_user_subject.is_none() {
            return Err(ControlError::new(
                ErrorCode::AuthenticatedUserRequired,
                format!("{} requires an authenticated Warp user", action.as_str()),
            ));
        }
        if !metadata
            .allowed_invocation_contexts
            .contains(&self.invocation_context)
        {
            return Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                format!(
                    "{} cannot run from the credential invocation context",
                    action.as_str()
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_authorization_header() {
        let token = AuthToken::from_secret("secret");
        let err = token
            .verify_authorization_header(None)
            .expect_err("rejected");
        assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
    }

    #[test]
    fn rejects_wrong_bearer_token() {
        let token = AuthToken::from_secret("secret");
        let err = token
            .verify_authorization_header(Some("Bearer wrong"))
            .expect_err("rejected");
        assert_eq!(err.code, ErrorCode::UnauthorizedLocalClient);
    }

    #[test]
    fn accepts_matching_bearer_token() {
        let token = AuthToken::from_secret("secret");
        token
            .verify_authorization_header(Some("Bearer secret"))
            .expect("accepted");
    }

    #[test]
    fn scoped_credential_allows_only_granted_action() {
        let grant = CredentialGrant::new(
            InstanceId("inst_test".to_owned()),
            ActionKind::TabCreate,
            InvocationContext::OutsideWarp,
            Duration::minutes(5),
        );
        grant
            .verify_for_action(ActionKind::TabCreate)
            .expect("tab.create grant is accepted");
        let err = grant
            .verify_for_action(ActionKind::WindowCreate)
            .expect_err("other actions are rejected");
        assert_eq!(err.code, ErrorCode::InsufficientPermissions);
    }
}
