//! Credential request, issuance, and validation types for local control.
use std::collections::HashMap;

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::InstanceId;
use crate::protocol::{
    ActionKind, ControlError, ErrorCode, ExecutionContextProof, InvocationContext,
};
use crate::scripting::ScriptingGrant;

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

/// App-issued proof material for one Warp-managed terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSessionProof {
    pub proof_id: String,
    pub terminal_session_id: String,
    pub proof_secret: String,
}

struct TerminalSessionProofRef<'a> {
    proof_id: &'a str,
    terminal_session_id: &'a str,
    proof_secret: &'a str,
}

#[derive(Debug, Clone)]
struct TerminalSessionProofEntry {
    instance_id: InstanceId,
    terminal_session_id: String,
    proof_secret: String,
    expires_at: DateTime<Utc>,
    revoked: bool,
}

/// In-memory verifier for app-issued terminal-session proof material.
#[derive(Debug, Default, Clone)]
pub struct TerminalSessionProofRegistry {
    entries: HashMap<String, TerminalSessionProofEntry>,
}

impl TerminalSessionProofRegistry {
    pub fn issue(
        &mut self,
        instance_id: InstanceId,
        terminal_session_id: impl Into<String>,
        ttl: Duration,
    ) -> TerminalSessionProof {
        let terminal_session_id = terminal_session_id.into();
        let proof = TerminalSessionProof {
            proof_id: format!("term_proof_{}", Uuid::new_v4().simple()),
            terminal_session_id: terminal_session_id.clone(),
            proof_secret: AuthToken::generate().secret().to_owned(),
        };
        let entry = TerminalSessionProofEntry {
            instance_id,
            terminal_session_id,
            proof_secret: proof.proof_secret.clone(),
            expires_at: Utc::now() + ttl,
            revoked: false,
        };
        self.entries.insert(proof.proof_id.clone(), entry);
        proof
    }

    pub fn revoke_session(&mut self, terminal_session_id: &str) {
        for entry in self.entries.values_mut() {
            if entry.terminal_session_id == terminal_session_id {
                entry.revoked = true;
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }

    fn verify(
        &self,
        instance_id: &InstanceId,
        proof: TerminalSessionProofRef<'_>,
    ) -> Result<(), ControlError> {
        let Some(entry) = self.entries.get(proof.proof_id) else {
            return Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "Warp terminal proof is unknown or has been invalidated",
            ));
        };
        if entry.revoked || Utc::now() >= entry.expires_at {
            return Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "Warp terminal proof is expired or revoked",
            ));
        }
        if &entry.instance_id != instance_id
            || entry.terminal_session_id != proof.terminal_session_id
            || entry.proof_secret != proof.proof_secret
        {
            return Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "Warp terminal proof does not match the issuing session",
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

    pub fn verify_execution_context_proof_with_registry(
        &self,
        instance_id: &InstanceId,
        registry: &TerminalSessionProofRegistry,
    ) -> Result<(), ControlError> {
        match (&self.invocation_context, &self.execution_context_proof) {
            (
                InvocationContext::InsideWarp,
                Some(ExecutionContextProof::VerifiedWarpTerminal {
                    proof_id,
                    terminal_session_id,
                    proof_secret,
                }),
            ) => registry.verify(
                instance_id,
                TerminalSessionProofRef {
                    proof_id,
                    terminal_session_id,
                    proof_secret,
                },
            ),
            (InvocationContext::InsideWarp, None) => Err(ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "inside-Warp credentials require an app-issued verified Warp terminal proof",
            )),
            (InvocationContext::InsideWarp, Some(ExecutionContextProof::ExternalClient)) => {
                Err(ControlError::new(
                    ErrorCode::ExecutionContextNotAllowed,
                    "inside-Warp credentials require registry-verified terminal proof material",
                ))
            }
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

    pub fn verified_terminal_session_id(&self) -> Option<&str> {
        match &self.execution_context_proof {
            Some(ExecutionContextProof::VerifiedWarpTerminal {
                terminal_session_id,
                ..
            }) => Some(terminal_session_id),
            Some(ExecutionContextProof::ExternalClient) | None => None,
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
    pub invocation_context: InvocationContext,
    pub authenticated_user: AuthenticatedUserGrant,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripting_grant: Option<ScriptingGrant>,
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
            invocation_context,
            authenticated_user: AuthenticatedUserGrant {
                required: metadata.authenticated_user.required,
                subject: None,
            },
            scripting_grant: None,
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn with_authenticated_user_subject(mut self, subject: impl Into<String>) -> Self {
        self.authenticated_user.subject = Some(subject.into());
        self
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
        let metadata = action.metadata();
        if metadata.requires_authenticated_user && self.authenticated_user.subject.is_none() {
            return Err(ControlError::new(
                ErrorCode::AuthenticatedUserRequired,
                format!("{} requires an authenticated Warp user", action.as_str()),
            ));
        }
        if metadata.requires_authenticated_user {
            let Some(scripting_grant) = &self.scripting_grant else {
                return Err(ControlError::new(
                    ErrorCode::AuthenticatedUserRequired,
                    format!(
                        "{} requires a verified Warp terminal scripting grant",
                        action.as_str()
                    ),
                ));
            };
            scripting_grant.verify_action(action)?;
            if self.authenticated_user.subject.as_deref() != Some(scripting_grant.subject.as_str())
            {
                return Err(ControlError::new(
                    ErrorCode::AuthenticatedUserRequired,
                    format!("{} scripting grant subject does not match", action.as_str()),
                ));
            }
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
