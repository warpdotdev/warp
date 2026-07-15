//! OAuth support and refresh timing for a connected ChatGPT (Codex)
//! subscription.
//!
//! Token persistence and the manager-owned single-flight refresh lifecycle are
//! added with the API-key manager integration; this module defines the protocol
//! surface and the timing policy that lifecycle uses.

pub mod oauth;

use std::time::Duration;

/// Refresh this long before a known expiry to avoid racing token expiration.
pub(crate) const REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);
/// Without `expires_in`, refresh conservatively every 24 hours.
pub(crate) const UNKNOWN_EXPIRY_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn refresh_delay(expires_in: Option<u64>) -> Duration {
    expires_in
        .map(Duration::from_secs)
        .map(|lifetime| lifetime.saturating_sub(REFRESH_LEAD_TIME))
        .unwrap_or(UNKNOWN_EXPIRY_REFRESH_INTERVAL)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
