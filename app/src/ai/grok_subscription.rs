//! App-layer orchestration for a connected xAI / Grok subscription's OAuth
//! tokens.
//!
//! The tokens themselves live in `ai::api_keys::ApiKeyManager` (the
//! request-building source of truth, persisted to secure storage under
//! `GrokOAuthTokens`). This module mirrors the AWS credential refresher
//! (`crate::ai::aws_credentials`): it owns the network-facing refresh lifecycle
//! — converting a [`TokenResponse`] into stored [`GrokTokens`], proactively
//! refreshing the access token shortly before it expires, and rescheduling the
//! next refresh — via the [`GrokTokenRefresher`] extension trait on
//! [`ApiKeyManager`].

use std::time::{Duration, SystemTime};

use ai::api_keys::{ApiKeyManager, GrokTokens};
use warpui::r#async::Timer;
use warpui::ModelContext;

use crate::ai::grok_oauth::{self, TokenResponse};

/// Refresh the access token this long before its hard expiry so a request never
/// races the expiration. Must be larger than `GrokTokens::EXPIRY_SKEW` so the
/// token is refreshed before it stops being sent.
const REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

/// Builds [`GrokTokens`] from a token-endpoint [`TokenResponse`], computing the
/// absolute `expires_at` from the relative `expires_in` and carrying over the
/// previous refresh token when xAI doesn't return a new one (refresh-token
/// rotation is optional in OAuth 2.0).
pub fn grok_tokens_from_response(
    response: TokenResponse,
    previous_refresh_token: Option<String>,
) -> GrokTokens {
    let expires_at = response
        .expires_in
        .and_then(|secs| u64::try_from(secs).ok())
        .map(|secs| SystemTime::now() + Duration::from_secs(secs));
    GrokTokens {
        access_token: response.access_token,
        refresh_token: response.refresh_token.or(previous_refresh_token),
        expires_at,
    }
}

/// App-layer extension to [`ApiKeyManager`] that keeps a connected Grok
/// subscription's OAuth tokens fresh, mirroring `AwsCredentialRefresher`.
pub trait GrokTokenRefresher {
    /// Persists freshly obtained tokens (e.g. right after the connect flow) and
    /// schedules the next proactive refresh.
    fn store_grok_tokens(&mut self, response: TokenResponse, ctx: &mut ModelContext<Self>)
    where
        Self: Sized;

    /// Ensures the stored token is (or becomes) valid: refreshes now if it has
    /// already (nearly) expired, otherwise schedules a refresh shortly before
    /// expiry. Safe to call on startup; a no-op without a stored refresh token
    /// or expiry.
    fn ensure_grok_token_fresh(&mut self, ctx: &mut ModelContext<Self>)
    where
        Self: Sized;
}

impl GrokTokenRefresher for ApiKeyManager {
    fn store_grok_tokens(&mut self, response: TokenResponse, ctx: &mut ModelContext<Self>) {
        apply_grok_tokens(self, response, ctx);
    }

    fn ensure_grok_token_fresh(&mut self, ctx: &mut ModelContext<Self>) {
        schedule_grok_token_refresh(self, ctx);
    }
}

/// Stores the tokens from `response` (carrying over the previous refresh token
/// when absent) and schedules the next proactive refresh.
fn apply_grok_tokens(
    manager: &mut ApiKeyManager,
    response: TokenResponse,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let previous_refresh_token = manager.grok_tokens().and_then(|t| t.refresh_token.clone());
    let tokens = grok_tokens_from_response(response, previous_refresh_token);
    manager.set_grok_tokens(Some(tokens), ctx);
    schedule_grok_token_refresh(manager, ctx);
}

/// Schedules a one-shot proactive refresh [`REFRESH_LEAD_TIME`] before the
/// current token's expiry (immediately if already within that window).
///
/// No-op when there's nothing to refresh against (no tokens, no refresh token,
/// or no known expiry). Reschedules itself after each successful refresh, so a
/// single call establishes an ongoing refresh loop for the lifetime of the
/// connection.
fn schedule_grok_token_refresh(manager: &mut ApiKeyManager, ctx: &mut ModelContext<ApiKeyManager>) {
    let Some(tokens) = manager.grok_tokens() else {
        return;
    };
    let Some(refresh_token) = tokens.refresh_token.clone() else {
        return;
    };
    let Some(expires_at) = tokens.expires_at else {
        // No expiry signal, so there's nothing to schedule against.
        return;
    };

    let now = SystemTime::now();
    let fire_at = expires_at.checked_sub(REFRESH_LEAD_TIME).unwrap_or(now);
    let delay = fire_at.duration_since(now).unwrap_or(Duration::ZERO);

    ctx.spawn(
        async move {
            Timer::after(delay).await;
        },
        move |manager, _output, ctx| {
            // The stored token may have changed (reconnect/disconnect) while we
            // slept; only refresh if our refresh token is still the current one.
            let still_current = manager
                .grok_tokens()
                .and_then(|t| t.refresh_token.as_deref())
                == Some(refresh_token.as_str());
            if still_current {
                spawn_grok_refresh(refresh_token, ctx);
            }
        },
    );
}

/// Kicks off a background token refresh using `refresh_token`, applying the
/// result (which reschedules the next refresh) or logging the failure.
fn spawn_grok_refresh(refresh_token: String, ctx: &mut ModelContext<ApiKeyManager>) {
    ctx.spawn(
        async move { grok_oauth::refresh_access_token(&refresh_token).await },
        |manager, result, ctx| match result {
            Ok(response) => {
                log::info!(
                    "Refreshed Grok OAuth token (expires_in={:?}, has_refresh_token={})",
                    response.expires_in,
                    response.refresh_token.is_some(),
                );
                apply_grok_tokens(manager, response, ctx);
            }
            Err(err) => {
                // Leave the existing (soon-to-expire) token in place; the server
                // remains the authority and will reject it if it's truly invalid.
                log::error!("Failed to refresh Grok OAuth token: {err:#}");
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(
        access_token: &str,
        refresh_token: Option<&str>,
        expires_in: Option<i64>,
    ) -> TokenResponse {
        TokenResponse {
            access_token: access_token.to_owned(),
            refresh_token: refresh_token.map(str::to_owned),
            token_type: Some("Bearer".to_owned()),
            expires_in,
            scope: None,
        }
    }

    #[test]
    fn grok_tokens_from_response_sets_expiry_from_expires_in() {
        let before = SystemTime::now();
        let tokens = grok_tokens_from_response(response("a", Some("r"), Some(3600)), None);
        let after = SystemTime::now();

        assert_eq!(tokens.access_token, "a");
        assert_eq!(tokens.refresh_token.as_deref(), Some("r"));
        let expires_at = tokens.expires_at.expect("expiry should be set");
        assert!(expires_at >= before + Duration::from_secs(3600));
        assert!(expires_at <= after + Duration::from_secs(3600));
    }

    #[test]
    fn grok_tokens_from_response_without_expires_in_has_no_expiry() {
        let tokens = grok_tokens_from_response(response("a", Some("r"), None), None);
        assert!(tokens.expires_at.is_none());
    }

    #[test]
    fn grok_tokens_from_response_carries_over_previous_refresh_token() {
        // xAI omitted a new refresh token, so we keep the previous one.
        let tokens =
            grok_tokens_from_response(response("a", None, Some(60)), Some("old".to_owned()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("old"));
    }

    #[test]
    fn grok_tokens_from_response_prefers_new_refresh_token() {
        // A rotated refresh token in the response wins over the previous one.
        let tokens =
            grok_tokens_from_response(response("a", Some("new"), Some(60)), Some("old".to_owned()));
        assert_eq!(tokens.refresh_token.as_deref(), Some("new"));
    }

    #[test]
    fn grok_tokens_from_response_ignores_negative_expires_in() {
        // A negative expires_in can't be represented; treat it as unknown.
        let tokens = grok_tokens_from_response(response("a", Some("r"), Some(-1)), None);
        assert!(tokens.expires_at.is_none());
    }
}
