//! Refresh orchestration for a connected ChatGPT (Codex) subscription.

pub mod oauth;

use std::future::Future;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use futures::channel::oneshot;
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warpui_core::r#async::Timer;
use warpui_core::ModelContext;

use self::oauth::{chatgpt_account_id_from_id_token, TokenResponse};
use crate::api_keys::{ApiKeyManager, CodexRefreshOutcome, CodexTokens};

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

/// Builds persisted Codex tokens from a token response. Refresh responses may
/// omit rotated tokens, so those values and the original connection timestamp
/// are carried forward. A new ID token updates the account id atomically with
/// the access token.
pub fn codex_tokens_from_response(
    response: TokenResponse,
    previous: Option<&CodexTokens>,
) -> anyhow::Result<CodexTokens> {
    let chatgpt_account_id = match response.id_token.as_deref() {
        Some(id_token) => chatgpt_account_id_from_id_token(id_token)?,
        None => previous
            .map(|tokens| tokens.chatgpt_account_id.clone())
            .filter(|account_id| !account_id.trim().is_empty())
            .context("Codex token response omitted the ID token and no account id was stored")?,
    };
    let id_token = response
        .id_token
        .or_else(|| previous.and_then(|tokens| tokens.id_token.clone()));
    let expires_at = response
        .expires_in
        .and_then(|seconds| SystemTime::now().checked_add(Duration::from_secs(seconds)));

    Ok(CodexTokens {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .or_else(|| previous.and_then(|tokens| tokens.refresh_token.clone())),
        id_token,
        chatgpt_account_id,
        expires_at,
        connected_at: previous
            .and_then(|tokens| tokens.connected_at)
            .or_else(|| Some(SystemTime::now())),
    })
}

impl ApiKeyManager {
    /// Persists tokens obtained from a completed login and arms proactive
    /// refresh according to the current feature/BYO permission.
    pub fn store_codex_tokens(
        &mut self,
        response: TokenResponse,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        apply_codex_tokens(self, response, ctx)
    }

    /// Updates refresh permission. Codex refresh is permitted only when both
    /// the caller's BYO policy and the default-off feature gate are enabled.
    pub fn set_codex_refresh_allowed(&mut self, byo_allowed: bool, ctx: &mut ModelContext<Self>) {
        let allowed = byo_allowed && FeatureFlag::CodexSubscription.is_enabled();
        if self.codex_refresh_allowed == allowed {
            return;
        }
        self.codex_refresh_allowed = allowed;
        if allowed {
            schedule_codex_token_refresh(self, ctx);
        }
    }

    pub(crate) fn codex_expired_refresh_token(&self, byo_allowed: bool) -> Option<String> {
        if !byo_allowed || !FeatureFlag::CodexSubscription.is_enabled() {
            return None;
        }
        let tokens = self.codex_tokens()?;
        if !tokens.is_expired() {
            return None;
        }
        tokens.refresh_token.clone()
    }

    /// Starts or joins a single refresh for a hard-expired Codex token.
    pub fn begin_expired_codex_refresh(
        &mut self,
        byo_allowed: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Option<oneshot::Receiver<CodexRefreshOutcome>> {
        self.set_codex_refresh_allowed(byo_allowed, ctx);
        let refresh_token = self.codex_expired_refresh_token(byo_allowed)?;
        let (sender, receiver) = oneshot::channel();
        log::info!("Codex OAuth token is expired at request time; waiting for refresh before send");
        spawn_codex_refresh(self, refresh_token, vec![sender], ctx);
        Some(receiver)
    }
}

fn apply_codex_tokens(
    manager: &mut ApiKeyManager,
    response: TokenResponse,
    ctx: &mut ModelContext<ApiKeyManager>,
) -> anyhow::Result<()> {
    let tokens = codex_tokens_from_response(response, manager.codex_tokens())?;
    manager.set_codex_tokens(Some(tokens), ctx);
    schedule_codex_token_refresh(manager, ctx);
    Ok(())
}

fn schedule_codex_token_refresh(
    manager: &mut ApiKeyManager,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    if !manager.codex_refresh_allowed {
        return;
    }
    let Some(tokens) = manager.codex_tokens() else {
        return;
    };
    let Some(refresh_token) = tokens.refresh_token.clone() else {
        return;
    };
    let expires_in = tokens.expires_at.map(|expires_at| {
        expires_at
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
            .as_secs()
    });
    let delay = refresh_delay(expires_in);

    ctx.spawn(
        async move {
            Timer::after(delay).await;
        },
        move |manager, _output, ctx| {
            if !manager.codex_refresh_allowed {
                return;
            }
            let still_current = manager
                .codex_tokens()
                .and_then(|tokens| tokens.refresh_token.as_deref())
                == Some(refresh_token.as_str());
            if still_current {
                spawn_codex_refresh(manager, refresh_token, Vec::new(), ctx);
            }
        },
    );
}

/// Registers waiters and returns whether the caller owns the new refresh.
pub(crate) fn register_codex_refresh(
    manager: &mut ApiKeyManager,
    waiters: Vec<oneshot::Sender<CodexRefreshOutcome>>,
) -> bool {
    if let Some(existing) = &mut manager.codex_refresh_waiters {
        existing.extend(waiters);
        false
    } else {
        manager.codex_refresh_waiters = Some(waiters);
        true
    }
}

pub(crate) fn finish_codex_refresh(
    manager: &mut ApiKeyManager,
    outcome: CodexRefreshOutcome,
) {
    for waiter in manager.codex_refresh_waiters.take().unwrap_or_default() {
        let _ = waiter.send(outcome);
    }
}

fn spawn_codex_refresh(
    manager: &mut ApiKeyManager,
    refresh_token: String,
    waiters: Vec<oneshot::Sender<CodexRefreshOutcome>>,
    ctx: &mut ModelContext<ApiKeyManager>,
) {
    let requested_refresh_token = refresh_token.clone();
    spawn_codex_refresh_with(
        manager,
        requested_refresh_token,
        waiters,
        async move { oauth::refresh_access_token(&refresh_token).await },
        ctx,
    );
}

fn spawn_codex_refresh_with<F>(
    manager: &mut ApiKeyManager,
    requested_refresh_token: String,
    waiters: Vec<oneshot::Sender<CodexRefreshOutcome>>,
    refresh: F,
    ctx: &mut ModelContext<ApiKeyManager>,
) where
    F: Future<Output = anyhow::Result<TokenResponse>> + Send + 'static,
{
    if !register_codex_refresh(manager, waiters) {
        return;
    }
    ctx.spawn(refresh, move |manager, result, ctx| {
        let outcome = match result {
            Ok(response) => {
                let still_current = manager
                    .codex_tokens()
                    .and_then(|tokens| tokens.refresh_token.as_deref())
                    == Some(requested_refresh_token.as_str());
                if !still_current {
                    log::info!(
                        "Discarding Codex OAuth refresh response because its credentials are no longer current"
                    );
                    CodexRefreshOutcome::Failed
                } else {
                    log::info!(
                        "Refreshed Codex OAuth token (expires_in={:?}, has_refresh_token={})",
                        response.expires_in,
                        response.refresh_token.is_some(),
                    );
                    match apply_codex_tokens(manager, response, ctx) {
                        Ok(()) => CodexRefreshOutcome::Refreshed,
                        Err(error) => {
                            report_error!(
                                error.context("Failed to apply refreshed Codex OAuth token")
                            );
                            CodexRefreshOutcome::Failed
                        }
                    }
                }
            }
            Err(error) => {
                report_error!(error.context("Failed to refresh Codex OAuth token"));
                CodexRefreshOutcome::Failed
            }
        };
        finish_codex_refresh(manager, outcome);
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
