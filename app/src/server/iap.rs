use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use base64::Engine;
use blocking::unblock;
use instant::Instant;
use warp_core::channel::IapConfig;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::view_components::DismissibleToast;
use crate::workspace::{ToastStack, WorkspaceAction};

const PROACTIVE_REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);
const INJECTED_TOKEN_ENV_VAR: &str = "WARP_IAP_TOKEN";

const BASE_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
/// Maximum number of consecutive failed fetches to automatically retry
/// before giving up and waiting for a manual Refresh or an inbound
/// IAP challenge. i.e. so a persistently broken setup (no gcloud,
/// bad credentials) doesn't loop forever.
const MAX_FAILURE_RETRIES: u32 = 5;

#[derive(Debug, Clone)]
pub struct CachedToken {
    pub token: String,
    pub expires_at: Instant,
}

impl CachedToken {
    fn valid_token(&self) -> Option<String> {
        (self.expires_at > Instant::now()).then(|| self.token.clone())
    }
}

#[derive(Debug, Clone)]
pub enum IapCredentialsState {
    Missing,
    /// A credential fetch is in progress. `previous` carries the last
    /// successfully-loaded token (if any). Allows us to attach it to
    /// outbound requests while we're refreshing so that proactive refreshes
    /// (i.e. refresh the token 5min before exp) don't prevent active requests.
    Refreshing {
        previous: Option<CachedToken>,
    },
    Loaded(CachedToken),
    Failed {
        message: String,
        // in case the last token still works... we can try to use that for a couple more mins
        previous: Option<CachedToken>,
    },
    /// Represents a terminal state in the iap creds state machine.
    /// The gcloud refresh loop will never run, and an IAP challenge is logged
    /// rather than triggering a refresh (we have no way to refresh a new token
    /// from ambient agent context yet).
    /// TODO(Isaiah/Jason): implement token refreshing scheme.
    /// see: https://linear.app/warpdotdev/issue/REMOTE-1370/refresh-github-token
    EnvInjected {
        token: String,
    },
}

impl IapCredentialsState {
    fn previous_token(&self) -> Option<CachedToken> {
        match self {
            IapCredentialsState::Loaded(cached) => Some(cached.clone()),
            IapCredentialsState::Refreshing { previous }
            | IapCredentialsState::Failed { previous, .. } => previous.clone(),
            IapCredentialsState::EnvInjected { .. } | IapCredentialsState::Missing => None,
        }
    }
}

pub struct IapState {
    audiences: String,
    service_account_email: String,
    inner: RwLock<IapCredentialsState>,
}

impl IapState {
    pub fn new(config: &IapConfig) -> Self {
        let initial = std::env::var(INJECTED_TOKEN_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty())
            .map(|token| IapCredentialsState::EnvInjected { token })
            .unwrap_or(IapCredentialsState::Missing);
        Self {
            audiences: config.audiences.to_string(),
            service_account_email: config.service_account_email.to_string(),
            inner: RwLock::new(initial),
        }
    }

    pub fn get_cached(&self) -> Option<String> {
        match &*self.inner.read().expect("IAP state lock poisoned") {
            IapCredentialsState::Loaded(cached) => Some(cached.token.clone()),
            IapCredentialsState::EnvInjected { token } => Some(token.clone()),
            IapCredentialsState::Refreshing { previous }
            | IapCredentialsState::Failed { previous, .. } => {
                previous.as_ref().and_then(CachedToken::valid_token)
            }
            IapCredentialsState::Missing => None,
        }
    }

    pub fn state(&self) -> IapCredentialsState {
        self.inner.read().expect("IAP state lock poisoned").clone()
    }

    pub fn audiences(&self) -> &str {
        &self.audiences
    }

    pub fn service_account_email(&self) -> &str {
        &self.service_account_email
    }

    fn set_refreshing(&self) {
        let mut state = self.inner.write().expect("IAP state lock poisoned");
        *state = IapCredentialsState::Refreshing {
            previous: state.previous_token(),
        };
    }

    fn set_loaded(&self, cached: CachedToken) {
        *self.inner.write().expect("IAP state lock poisoned") = IapCredentialsState::Loaded(cached);
    }

    fn set_failed(&self, message: String) {
        let mut state = self.inner.write().expect("IAP state lock poisoned");
        *state = IapCredentialsState::Failed {
            message,
            previous: state.previous_token(),
        };
    }
}

impl http_client::iap::IapTokenProvider for IapState {
    fn cached_token(&self) -> Option<String> {
        self.get_cached()
    }
}

/// Owns the IAP refresh lifecycle: initial fetch, proactive time-based
/// refresh, and reactive refresh on challenge events.
pub struct IapManager {
    state: Option<Arc<IapState>>,
    /// Number of consecutive failed fetches since the last success.
    consecutive_failures: u32,
}

pub enum IapManagerEvent {
    StateChanged,
}

impl IapManager {
    pub fn new(state: Option<Arc<IapState>>, ctx: &mut ModelContext<Self>) -> Self {
        let mut manager = Self {
            state,
            consecutive_failures: 0,
        };
        manager.start_refresh(ctx);
        manager
    }

    /// Returns `true` if IAP is active for this build. When `false`, all
    /// other methods on this type are no-ops.
    pub fn is_enabled(&self) -> bool {
        self.state.is_some()
    }

    pub fn state(&self) -> Option<IapCredentialsState> {
        self.state.as_ref().map(|s| s.state())
    }

    pub fn handle_challenge(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if matches!(state.state(), IapCredentialsState::EnvInjected { .. }) {
            log::warn!(
                "Env-injected IAP token ({INJECTED_TOKEN_ENV_VAR}) was rejected by IAP; \
                 token is likely stale — re-inject to recover"
            );
            return;
        }
        self.consecutive_failures = 0;
        self.start_refresh(ctx);
    }

    pub fn start_refresh(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(state) = self.state.clone() else {
            return;
        };
        // Don't touch state if a refresh is already running, or if we're
        // in the terminal env-injected state (no refresh path exists).
        if matches!(
            state.state(),
            IapCredentialsState::Refreshing { .. } | IapCredentialsState::EnvInjected { .. }
        ) {
            return;
        }
        state.set_refreshing();
        ctx.emit(IapManagerEvent::StateChanged);
        ctx.notify();

        let audiences = state.audiences().to_string();
        let service_account_email = state.service_account_email().to_string();

        ctx.spawn(
            async move {
                unblock(move || fetch_iap_token(&audiences, &service_account_email)).await
            },
            move |manager, result, ctx| {
                let Some(state) = manager.state.as_ref() else {
                    return;
                };
                match result {
                    Ok(cached) => {
                        let expires_at = cached.expires_at;
                        state.set_loaded(cached);
                        manager.consecutive_failures = 0;
                        log::info!("IAP token refreshed");
                        ctx.emit(IapManagerEvent::StateChanged);
                        ctx.notify();
                        manager.schedule_next_refresh(expires_at, ctx);
                    }
                    Err(err) => {
                        let message = format!("{err:#}");
                        log::warn!("IAP token fetch failed: {message}");
                        let is_first_failure_of_streak = manager.consecutive_failures == 0;
                        state.set_failed(message.clone());
                        if is_first_failure_of_streak {
                            manager.show_failure_toast(&message, ctx);
                        }
                        ctx.emit(IapManagerEvent::StateChanged);
                        ctx.notify();
                        manager.schedule_failure_retry(ctx);
                    }
                }
            },
        );
    }

    fn schedule_next_refresh(&mut self, expires_at: Instant, ctx: &mut ModelContext<Self>) {
        let sleep_duration = expires_at
            .saturating_duration_since(Instant::now())
            .saturating_sub(PROACTIVE_REFRESH_BUFFER);
        self.schedule_retry(sleep_duration, ctx);
    }

    fn schedule_failure_retry(&mut self, ctx: &mut ModelContext<Self>) {
        if self.consecutive_failures >= MAX_FAILURE_RETRIES {
            log::warn!(
                "IAP token fetch failed {MAX_FAILURE_RETRIES} times in a row; giving up until \
                 manual refresh or server challenge"
            );
            return;
        }
        // Delay = BASE * 2^failures, capped at MAX. Using u32 shift is
        // safe because we cap failures at MAX_FAILURE_RETRIES (< 32).
        let delay = BASE_FAILURE_RETRY_DELAY
            .saturating_mul(1u32 << self.consecutive_failures)
            .min(MAX_FAILURE_RETRY_DELAY);
        self.consecutive_failures += 1;
        log::info!(
            "Scheduling IAP refresh retry #{} in {}s",
            self.consecutive_failures,
            delay.as_secs()
        );
        self.schedule_retry(delay, ctx);
    }

    fn schedule_retry(&mut self, delay: Duration, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(delay).await;
            },
            |manager, _, ctx| {
                manager.start_refresh(ctx);
            },
        );
    }

    fn show_failure_toast(&self, message: &str, ctx: &mut ModelContext<Self>) {
        let window_id = ctx
            .windows()
            .active_window()
            .or_else(|| ctx.windows().ordered_window_ids().first().copied());
        let Some(window_id) = window_id else {
            return;
        };
        let toast: DismissibleToast<WorkspaceAction> =
            DismissibleToast::error(format!("IAP credential refresh failed: {message}"));
        ToastStack::handle(ctx).update(ctx, |stack, ctx| {
            stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }
}

impl Entity for IapManager {
    type Event = IapManagerEvent;
}

impl SingletonEntity for IapManager {}

/// How long to wait for `auth print-identity-token` command to respond before killing it.
const GCLOUD_TIMEOUT: Duration = Duration::from_secs(30);

fn fetch_iap_token(audiences: &str, service_account_email: &str) -> Result<CachedToken> {
    let args = [
        "auth",
        "print-identity-token",
        "--audiences",
        audiences,
        "--impersonate-service-account",
        service_account_email,
        "--include-email",
    ];
    let cmd_display = format!("gcloud {}", args.join(" "));

    let mut child = command::blocking::Command::new("gcloud")
        // Prevent gcloud from waiting for interactive input (fail fast instead of hanging)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .args(args)
        .spawn()
        .map_err(|err| anyhow::anyhow!("Failed to spawn `{cmd_display}`: {err}"))?;

    // Poll for completion, killing the child if it exceeds the timeout.
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > GCLOUD_TIMEOUT {
                    let _ = child.kill();
                    anyhow::bail!(
                        "`{cmd_display}` timed out after {}s",
                        GCLOUD_TIMEOUT.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => anyhow::bail!("Failed to wait for `{cmd_display}`: {err}"),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|err| anyhow::anyhow!("Failed to collect output from `{cmd_display}`: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("`{cmd_display}` failed: {stderr}");
    }

    let token = String::from_utf8(output.stdout)
        .map_err(|err| anyhow::anyhow!("gcloud output is not valid UTF-8: {err}"))?
        .trim()
        .to_string();

    anyhow::ensure!(!token.is_empty(), "gcloud returned an empty token");

    let expires_at = get_expires_at(&token)?;
    Ok(CachedToken { token, expires_at })
}

fn get_expires_at(token: &str) -> Result<Instant> {
    let exp = parse_exp_from_jwt(token).ok_or_else(|| {
        anyhow::anyhow!("IAP token missing or unparseable `exp` claim; refusing to cache")
    })?;
    // `exp` is Unix wall-clock seconds; `Instant` is monotonic and
    // has no Unix-time API, so bridge via `SystemTime::now()` to
    // compute a delta, then add that to `Instant::now()`.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| anyhow::anyhow!("system clock is before unix epoch: {err}"))?
        .as_secs();
    let secs_remaining = exp
        .checked_sub(now)
        .ok_or_else(|| anyhow::anyhow!("IAP token is already expired (exp={exp}, now={now})"))?;
    Ok(Instant::now() + Duration::from_secs(secs_remaining))
}

fn parse_exp_from_jwt(token: &str) -> Option<u64> {
    let payload_b64 = token.split('.').nth(1)?;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("exp")?.as_u64()
}
