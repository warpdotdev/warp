//! Refresh orchestration for a connected xAI / Grok subscription's OAuth
//! tokens.
//!
//! The tokens themselves live in [`ApiKeyManager`] (the request-building
//! source of truth, persisted to secure storage under `GrokOAuthTokens`).
//! This module owns the network-facing refresh lifecycle — converting a
//! [`TokenResponse`] into stored [`GrokTokens`], proactively refreshing the
//! access token shortly before it expires, and rescheduling the next refresh.
//!
//! The Grok subscription is BYO auth, so background refresh follows the BYO
//! API key policy. That policy lives in the app layer (workspace settings),
//! which this crate has no visibility into; the app wires it in via
//! [`ApiKeyManager::set_grok_refresh_allowed`].
//!
//! The network/protocol side of the connect flow (authorize URL, loopback
//! callback server, token exchange/refresh) lives in the [`oauth`] submodule.

pub mod oauth;

use std::time::{Duration, SystemTime};

use warpui_core::r#async::Timer;
use warpui_core::ModelContext;

use self::oauth::TokenResponse;
use crate::api_keys::{ApiKeyManager, GrokTokens};

/// Refresh the access token this long before its hard expiry so a request
/// never races the expiration. Possibly-expired tokens are still sent (the
/// server is the authority on validity), so this lead time is purely about
/// keeping the token fresh, not about when it stops being sent.
const REFRESH_LEAD_TIME: Duration = Duration::from_secs(5 * 60);

/// Builds [`GrokTokens`] from a token-endpoint [`TokenResponse`], computing the
/// absolute `expires_at` from the relative `expires_in`. Values not present in
/// the response are carried over from `previous`: the refresh token when xAI
/// doesn't return a new one (refresh-token rotation is optional in OAuth 2.0),
/// and `connected_at` so it keeps reflecting the initial connection time
/// (initialized to now when there are no previous tokens, i.e. a fresh
/// connect).
pub fn grok_tokens_from_response(
    response: TokenResponse,
    previous: Option<&GrokTokens>,
) -> GrokTokens {
    let expires_at = response
        .expires_in
        .and_then(|secs| u64::try_from(secs).ok())
        .and_then(|secs| SystemTime::now().checked_add(Duration::from_secs(secs)));
    GrokTokens {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .or_else(|| previous.and_then(|tokens| tokens.refresh_token.clone())),
        expires_at,
        connected_at: previous
            .and_then(|tokens| tokens.connected_at)
            .or_else(|| Some(SystemTime::now())),
    }
}

impl ApiKeyManager {
    /// Persists freshly obtained tokens (e.g. right after the connect flow) and
    /// schedules the next proactive refresh.
    pub fn store_grok_tokens(&mut self, response: TokenResponse, ctx: &mut ModelContext<Self>) {
        apply_grok_tokens(self, response, ctx);
    }

    /// Updates whether background refresh of the stored Grok tokens is
    /// allowed. The Grok subscription is BYO auth, so refresh follows the same
    /// policy gate as request injection ([`Self::api_keys_for_request`]):
    /// tokens that can never be sent shouldn't be kept fresh. The policy lives
    /// in the app layer, which calls this at startup and whenever the policy
    /// may have changed (e.g. team data arriving, or a workspace switch).
    ///
    /// Schedules a refresh on a disabled -> enabled transition (refreshing
    /// immediately if the token has already (nearly) expired); in-flight
    /// timers re-check the flag when they fire. Repeated calls with an
    /// unchanged value are no-ops, so duplicate timers can't pile up.
    pub fn set_grok_refresh_allowed(&mut self, allowed: bool, ctx: &mut ModelContext<Self>) {
        if self.grok_refresh_allowed == allowed {
            return;
        }
        self.grok_refresh_allowed = allowed;
        if allowed {
            schedule_grok_token_refresh(self, ctx);
        }
    }

    /// Request-time safety net: kicks off a background refresh of the stored
    /// Grok tokens when they are nearing (or already past) expiry, so
    /// upcoming requests can authenticate even if the proactive refresh loop
    /// never armed or died (e.g. a stale BYO policy at startup, or an earlier
    /// failed refresh). The triggering request still carries the currently
    /// stored token — the server is the authority on its validity.
    ///
    /// `byo_allowed` is the BYO API key policy as freshly evaluated by the
    /// caller at request time. It also re-syncs the stored policy mirror,
    /// which can go stale between `TeamsChanged` events; a disabled ->
    /// enabled transition re-arms the proactive refresh loop.
    pub fn refresh_grok_tokens_if_needed(
        &mut self,
        byo_allowed: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_grok_refresh_allowed(byo_allowed, ctx);
        if !byo_allowed || self.grok_refresh_in_flight {
            return;
        }
        let Some(tokens) = self.grok_tokens() else {
            return;
        };
        if !tokens.needs_refresh(REFRESH_LEAD_TIME) {
            return;
        }
        let Some(refresh_token) = tokens.refresh_token.clone() else {
            return;
        };
        log::info!(
            "Grok OAuth token is nearing or past expiry at request time; refreshing in background"
        );
        spawn_grok_refresh(self, refresh_token, ctx);
    }
}

/// Stores the tokens from `response` (carrying over the previous refresh token
/// and connection time when absent) and schedules the next proactive refresh.
fn apply_grok_tokens(
    manager: &mut ApiKeyManager,
    response: TokenResponse,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let tokens = grok_tokens_from_response(response, manager.grok_tokens());
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
    // When the BYO API key policy is disabled the token is never sent, so
    // don't refresh it in the background either. `set_grok_refresh_allowed`
    // re-establishes the loop if the policy is later enabled.
    if !manager.grok_refresh_allowed {
        return;
    }
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
            // The BYO policy may have flipped off while we slept;
            // `set_grok_refresh_allowed` restarts the loop if it flips back
            // on.
            if !manager.grok_refresh_allowed {
                return;
            }
            // The stored token may have changed (reconnect/disconnect) while we
            // slept; only refresh if our refresh token is still the current one.
            let still_current = manager
                .grok_tokens()
                .and_then(|t| t.refresh_token.as_deref())
                == Some(refresh_token.as_str());
            if still_current {
                spawn_grok_refresh(manager, refresh_token, ctx);
            }
        },
    );
}

/// Kicks off a background token refresh using `refresh_token`, applying the
/// result (which reschedules the next refresh) or logging the failure.
///
/// No-op when a refresh is already in flight, so the proactive timer and the
/// request-time safety net can't issue overlapping refreshes.
fn spawn_grok_refresh(
    manager: &mut ApiKeyManager,
    refresh_token: String,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    if manager.grok_refresh_in_flight {
        return;
    }
    manager.grok_refresh_in_flight = true;
    ctx.spawn(
        async move { oauth::refresh_access_token(&refresh_token).await },
        |manager, result, ctx| {
            manager.grok_refresh_in_flight = false;
            match result {
                Ok(response) => {
                    log::info!(
                        "Refreshed Grok OAuth token (expires_in={:?}, has_refresh_token={})",
                        response.expires_in,
                        response.refresh_token.is_some(),
                    );
                    apply_grok_tokens(manager, response, ctx);
                }
                Err(err) => {
                    // Leave the existing (possibly expired) token in place; the
                    // server remains the authority and will reject it if it's
                    // truly invalid. The request-time safety net
                    // (`ApiKeyManager::refresh_grok_tokens_if_needed`) retries
                    // on the next request.
                    log::error!("Failed to refresh Grok OAuth token: {err:#}");
                }
            }
        },
    );
}
