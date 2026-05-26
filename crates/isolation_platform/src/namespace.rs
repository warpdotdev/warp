use std::time::Duration;
use std::{env, fs};

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use command::r#async::Command;
use warp_core::channel::ChannelState;

use crate::{IsolationPlatformError, WorkloadToken};

/// Detect whether or not we are running in a Namespace instance.
pub fn is_in_namespace_instance() -> bool {
    // For Namespace, match their CLI's logic for detecting a token:
    // https://github.com/namespacelabs/integrations/blob/08d0acd17ce05f8486ec8da329066dd6a12572a0/auth/token.go#L116-L131
    env::var("NSC_TOKEN_FILE").is_ok() || fs::exists("/var/run/nsc/token.json").is_ok_and(|v| v)
}

/// Issue a Namespace workload identity token.
pub async fn issue_workload_token(
    duration: Option<Duration>,
) -> Result<WorkloadToken, IsolationPlatformError> {
    let mut nsc_command = Command::new("nsc");
    nsc_command
        .arg("auth")
        .arg("issue-id-token")
        .arg("--audience")
        .arg(&*ChannelState::workload_audience_url())
        .arg("--output")
        .arg("json");

    if let Some(duration) = duration {
        nsc_command
            .arg("--duration")
            .arg(format!("{}ns", duration.as_nanos()));
    }

    let output =
        nsc_command
            .output()
            .await
            .map_err(|err| IsolationPlatformError::CommandUnavailable {
                command: "nsc".to_owned(),
                source: err,
            })?;

    if !output.status.success() {
        log::warn!(
            "`nsc` command failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(IsolationPlatformError::CommandFailed {
            command: "nsc".to_owned(),
            status: output.status,
        });
    }

    /// JSON output from `nsc auth issue-id-token`.
    #[derive(serde::Deserialize)]
    struct NscTokenOutput {
        id_token: String,
    }

    let token_output = serde_json::from_slice::<NscTokenOutput>(&output.stdout)
        .map_err(|_| anyhow::anyhow!("Unexpected output from `nsc auth issue-id-token`"))?;

    // Namespace ID tokens are JWTs.
    let expires_at = parse_jwt_expiration(&token_output.id_token)?;

    Ok(WorkloadToken {
        token: token_output.id_token,
        expires_at: Some(expires_at),
    })
}

/// Parse the expiration time from a JWT token.
///
/// JWTs have three base64url-encoded parts separated by dots: header.payload.signature.
/// The payload contains an `exp` claim with the Unix timestamp of expiration.
fn parse_jwt_expiration(token: &str) -> Result<DateTime<Utc>, IsolationPlatformError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(
            anyhow::anyhow!("Invalid JWT format: expected 3 parts, got {}", parts.len()).into(),
        );
    }

    let payload_bytes = BASE64_URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| anyhow::anyhow!("Failed to decode JWT payload: {e}"))?;

    #[derive(serde::Deserialize)]
    struct JwtPayload {
        exp: i64,
    }

    let payload: JwtPayload = serde_json::from_slice(&payload_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse JWT payload: {e}"))?;

    DateTime::from_timestamp(payload.exp, 0)
        .ok_or_else(|| anyhow::anyhow!("Invalid exp timestamp in JWT: {}", payload.exp).into())
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
