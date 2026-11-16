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

use futures::channel::oneshot;
use warp_errors::report_error;
use warpui_core::ModelContext;
use warpui_core::r#async::Timer;

use self::oauth::TokenResponse;
use crate::api_keys::{ApiKeyManager, GrokRefreshOutcome, GrokTokens};

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

    /// Returns the refresh token to use for a request-time blocking refresh
    /// when the stored Grok token is at/past its hard expiry and eligible to be
    /// sent, or `None` when no refresh is warranted: BYO disabled, no stored
    /// token, the token not yet expired (the proactive timer handles the
    /// near-expiry window), or no refresh token.
    ///
    /// Pure eligibility read used by [`Self::begin_expired_grok_refresh`] and
    /// covered by unit tests. It deliberately does NOT consider whether a
    /// refresh is already in flight — that coordination (waiting on the
    /// existing refresh) lives in `begin_expired_grok_refresh`. `byo_allowed`
    /// is the BYO API key policy as freshly evaluated by the caller at request
    /// time.
    pub(crate) fn grok_expired_refresh_token(&self, byo_allowed: bool) -> Option<String> {
        if !byo_allowed {
            return None;
        }
        let tokens = self.grok_tokens()?;
        if !tokens.is_expired() {
            return None;
        }
        tokens.refresh_token.clone()
    }

    /// Ensures a refresh is running for an already-expired Grok token and
    /// returns a receiver that fires once that refresh finishes (success or
    /// failure). Returns `None` when no refresh is warranted (see
    /// [`Self::grok_expired_refresh_token`]).
    ///
    /// If a refresh is already in flight — started by the proactive timer or an
    /// earlier request — the caller attaches to it rather than starting a
    /// second one (strict single-flight); every waiter is woken when it
    /// finishes. The refresh runs on this manager's own (singleton) context, so
    /// the in-flight state is always cleared even if a caller's model is
    /// dropped mid-refresh. The caller waits on the receiver (bounded by its
    /// own timeout), then reads whichever token is then stored — refreshed on
    /// success, unchanged on failure/timeout (the server stays the authority on
    /// validity).
    pub fn begin_expired_grok_refresh(
        &mut self,
        byo_allowed: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Option<oneshot::Receiver<GrokRefreshOutcome>> {
        // Keep the proactive-refresh policy mirror in sync with the freshly
        // evaluated BYO policy (it can drift between `TeamsChanged` events). A
        // disabled -> enabled transition re-arms the proactive refresh loop, so
        // a successful blocking refresh below also reschedules the next one
        // instead of leaving the token to expire again unrefreshed.
        self.set_grok_refresh_allowed(byo_allowed, ctx);
        let refresh_token = self.grok_expired_refresh_token(byo_allowed)?;
        let (tx, rx) = oneshot::channel();
        log::info!("Grok OAuth token is expired at request time; waiting for refresh before send");
        spawn_grok_refresh(self, refresh_token, vec![tx], ctx);
        Some(rx)
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
                spawn_grok_refresh(manager, refresh_token, Vec::new(), ctx);
            }
        },
    );
}

/// Kicks off a background token refresh using `refresh_token`, applying the
/// result (which reschedules the next refresh) or logging the failure, then
/// waking every waiter.
///
/// If a refresh is already in flight, the new `waiters` are attached to it and
/// no second refresh starts (strict single-flight). Otherwise the refresh runs
/// on the manager's own (singleton) context, so the in-flight guard is always
/// cleared when it finishes even if a waiter's model was dropped meanwhile.
fn spawn_grok_refresh(
    manager: &mut ApiKeyManager,
    refresh_token: String,
    waiters: Vec<oneshot::Sender<GrokRefreshOutcome>>,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    // A refresh is already running; attach the new waiters to it instead of
    // starting a second one. They're woken when the in-flight refresh finishes.
    if let Some(existing) = manager.grok_refresh_waiters.as_mut() {
        existing.extend(waiters);
        return;
    }
    manager.grok_refresh_waiters = Some(waiters);
    ctx.spawn(
        async move { oauth::refresh_access_token(&refresh_token).await },
        move |manager, result, ctx| {
            // Clear the in-flight guard and take the waiters to wake below.
            let waiters = manager.grok_refresh_waiters.take().unwrap_or_default();
            let outcome = match result {
                Ok(response) => {
                    log::info!(
                        "Refreshed Grok OAuth token (expires_in={:?}, has_refresh_token={})",
                        response.expires_in,
                        response.refresh_token.is_some(),
                    );
                    apply_grok_tokens(manager, response, ctx);
                    GrokRefreshOutcome::Refreshed
                }
                Err(err) => {
                    // Leave the existing (possibly expired) token in place. The
                    // waiting request surfaces this failure instead of sending
                    // with the dead token; a later request re-triggers a refresh.
                    report_error!(err.context("Failed to refresh Grok OAuth token"));
                    GrokRefreshOutcome::Failed
                }
            };
            // Wake every request blocked on this refresh with the outcome.
            // Dropped receivers (callers gone) are safe to ignore.
            for waiter in waiters {
                let _ = waiter.send(outcome);
            }
        },
    );
}
