use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use base64::Engine;
use blocking::unblock;
use instant::Instant;
use warp_core::channel::IapConfig;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{Entity, ModelContext, SingletonEntity};
#[cfg(not(target_family = "wasm"))]
use websocket::connect_error_http_response;

#[cfg(feature = "local_tty")]
use crate::terminal::local_shell::LocalShellState;
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
            // Gate on expiry even while `Loaded`: if a proactive refresh is
            // delayed (e.g. the machine slept across the refresh window), the
            // token may already be expired, and attaching it would guarantee an
            // IAP challenge. Returning `None` lets the caller proceed without a
            // doomed token while the reactive refresh recovers.
            IapCredentialsState::Loaded(cached) => cached.valid_token(),
            IapCredentialsState::EnvInjected { token } => Some(token.clone()),
            IapCredentialsState::Refreshing { previous }
            | IapCredentialsState::Failed { previous, .. } => {
                previous.as_ref().and_then(CachedToken::valid_token)
            }
            IapCredentialsState::Missing => None,
        }
    }

    pub fn proxy_auth_header(&self) -> Option<(&'static str, String)> {
        self.get_cached()
            .map(|token| http_client::iap::proxy_auth_header(&token))
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

    /// Returns a handle to the shared IAP credential state, if IAP is active.
    /// Mirrors how `AuthStateProvider` hands out the `Arc<AuthState>`, letting
    /// callers read cached credentials (e.g. to build a proxy-auth header) off
    /// a `ModelContext` without reaching through `ServerApi`.
    pub fn iap_state(&self) -> Option<Arc<IapState>> {
        self.state.clone()
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

        // Make `gcloud` findable even when Warp is launched from the macOS GUI
        // (i.e. in environments without something like `~/.zshrc && WarpDev` happening to init cli path)
        #[cfg(feature = "local_tty")]
        let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });
        #[cfg(not(feature = "local_tty"))]
        let path_future = futures::future::ready(None::<String>);

        ctx.spawn(
            async move {
                // Bound the interactive PATH capture. It spawns an interactive
                // login shell (sourcing rc files), which can hang indefinitely
                // on a misbehaving startup script. Without this bound the
                // spawned task would never reach the `GCLOUD_TIMEOUT`-guarded
                // fetch, stranding the state machine in `Refreshing` and
                // silently disabling every future refresh and IAP challenge
                // (both early-return while `Refreshing`). On timeout, fall back
                // to the ambient PATH so the fetch still runs and the state
                // machine can make progress (succeed or fail).
                const PATH_CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
                let path_env = match path_future.with_timeout(PATH_CAPTURE_TIMEOUT).await {
                    Ok(path_env) => path_env,
                    Err(_) => {
                        log::warn!(
                            "Interactive PATH capture timed out after {}s; \
                             falling back to ambient PATH for IAP token fetch",
                            PATH_CAPTURE_TIMEOUT.as_secs()
                        );
                        None
                    }
                };
                unblock(move || {
                    fetch_iap_token(&audiences, &service_account_email, path_env.as_deref())
                })
                .await
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

    /// Inspects a websocket *handshake* connect error for an IAP challenge.
    /// If detected, triggers a refresh so the caller's retry loop can pick up
    /// a fresh token on the next attempt.
    #[cfg(not(target_family = "wasm"))]
    pub fn check_ws_connect_error(&mut self, err: &anyhow::Error, ctx: &mut ModelContext<Self>) {
        if ws_connect_is_iap_challenge(err) {
            log::warn!("Received IAP challenge on websocket handshake; triggering refresh");
            self.handle_challenge(ctx);
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn check_ws_connect_error(&mut self, _err: &anyhow::Error, _ctx: &mut ModelContext<Self>) {}
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn ws_connect_is_iap_challenge(err: &anyhow::Error) -> bool {
    connect_error_http_response(err).is_some_and(|response| {
        http_client::iap::is_iap_challenge(response.status(), response.headers())
    })
}

impl Entity for IapManager {
    type Event = IapManagerEvent;
}

impl SingletonEntity for IapManager {}

/// How long to wait for `auth print-identity-token` command to respond before killing it.
const GCLOUD_TIMEOUT: Duration = Duration::from_secs(30);

fn fetch_iap_token(
    audiences: &str,
    service_account_email: &str,
    path_env: Option<&str>,
) -> Result<CachedToken> {
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

    let mut cmd = command::blocking::Command::new("gcloud");
    cmd
        // Prevent gcloud from waiting for interactive input (fail fast instead of hanging)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .args(args);
    // allows warp to resolve `gcloud` cli path
    if let Some(path_env) = path_env {
        cmd.env("PATH", path_env);
    }
    let mut child = cmd
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

#[cfg(test)]
#[path = "iap_tests.rs"]
mod tests;
