//! Authenticated scripting grants for local Warp control.
//!
//! Authenticated local-control actions are only eligible for grants that were
//! minted from a verified Warp-managed terminal proof. No standalone external
//! authenticated grant source is modeled here.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::protocol::{ActionKind, ControlError, ErrorCode};

/// How a scripting grant was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ScriptingIdentitySource {
    VerifiedWarpTerminal { terminal_session_id: String },
}

/// Authenticated scripting grant attached to a local-control credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptingGrant {
    pub source: ScriptingIdentitySource,
    pub subject: String,
    pub actions: Vec<ActionKind>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ScriptingGrant {
    pub fn verified_warp_terminal(
        terminal_session_id: impl Into<String>,
        subject: impl Into<String>,
        actions: Vec<ActionKind>,
        ttl: Duration,
    ) -> Self {
        let issued_at = Utc::now();
        Self {
            source: ScriptingIdentitySource::VerifiedWarpTerminal {
                terminal_session_id: terminal_session_id.into(),
            },
            subject: subject.into(),
            actions,
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn has_action(&self, action: ActionKind) -> bool {
        self.actions.contains(&action)
    }

    pub fn verify_action(&self, action: ActionKind) -> Result<(), ControlError> {
        if self.is_expired() {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "authenticated scripting grant has expired",
            ));
        }
        if !self.has_action(action) {
            return Err(ControlError::new(
                ErrorCode::InsufficientPermissions,
                format!(
                    "authenticated scripting grant cannot invoke {}",
                    action.as_str()
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "scripting_tests.rs"]
mod tests;
