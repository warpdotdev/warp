//! Credential request, issuance, and validation types for local control.
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::InstanceId;
use crate::protocol::{
    ActionKind, ControlError, ErrorCode, ExecutionContextProof, InvocationContext,
    PermissionCategory, RiskTier, StateDataCategory,
};

/// Bearer token used to authorize a single scoped local-control credential.
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

/// Request for a short-lived credential scoped to one action and invocation context.
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

    /// Verifies whether the caller may claim its requested invocation context.
    ///
    /// External callers do not receive elevated trust from this proof and are
    /// allowed only when the selected Warp instance enables outside-Warp
    /// control. Inside-Warp callers must eventually present an app-issued,
    /// session-bound `VerifiedWarpTerminal` proof; until that broker path lands,
    /// this foundation branch rejects inside-Warp credential requests rather
    /// than trusting a caller-declared label or spoofable environment variable.
    pub fn verify_execution_context_proof(&self) -> Result<(), ControlError> {
        match (&self.invocation_context, &self.execution_context_proof) {
            (InvocationContext::InsideWarp, _) => Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "inside-Warp credentials require an app-issued verified Warp terminal proof",
            )),
            (
                InvocationContext::OutsideWarp,
                None | Some(ExecutionContextProof::ExternalClient),
            ) => Ok(()),
            (
                InvocationContext::OutsideWarp,
                Some(ExecutionContextProof::VerifiedWarpTerminal { .. }),
            ) => Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "external clients cannot use a Warp terminal execution proof",
            )),
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
    pub risk_tier: RiskTier,
    pub state_data_category: StateDataCategory,
    pub permission_category: PermissionCategory,
    pub invocation_context: InvocationContext,
    pub authenticated_user: AuthenticatedUserGrant,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Authenticated user context attached to a credential grant when required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedUserGrant {
    pub required: bool,
    pub subject: Option<String>,
}

impl CredentialGrant {
    pub fn new(
        instance_id: InstanceId,
        action: ActionKind,
        invocation_context: InvocationContext,
        ttl: Duration,
    ) -> Self {
        let issued_at = Utc::now();
        let metadata = action.metadata();
        Self {
            credential_id: format!("cred_{}", Uuid::new_v4().simple()),
            instance_id,
            action,
            risk_tier: metadata.risk_tier,
            state_data_category: metadata.state_data_category,
            permission_category: metadata.permission_category,
            invocation_context,
            authenticated_user: AuthenticatedUserGrant {
                required: metadata.authenticated_user.required,
                subject: None,
            },
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn with_authenticated_user_subject(mut self, subject: impl Into<String>) -> Self {
        self.authenticated_user.subject = Some(subject.into());
        self
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
        if self.risk_tier != metadata.risk_tier
            || self.state_data_category != metadata.state_data_category
            || self.permission_category != metadata.permission_category
        {
            return Err(ControlError::new(
                ErrorCode::InsufficientPermissions,
                format!(
                    "credential grant metadata does not satisfy {}",
                    action.as_str()
                ),
            ));
        }
        if metadata.requires_authenticated_user && self.authenticated_user.subject.is_none() {
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
#[path = "auth_tests.rs"]
mod tests;
