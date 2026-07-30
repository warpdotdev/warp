//! The headless `warp-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the real headless Warp app via [`crate::run_tui`]. Once
//! shared initialization is done, [`init`] registers the [`TuiLoginModel`] that
//! the TUI observes, mounts the TUI immediately (so it renders right away), and
//! leaves device authorization behind an explicit welcome-screen action,
//! flipping the model to [`TuiLoginPhase::LoggedIn`] when it completes.
mod mcp;
mod user_info;
use std::env;

pub use mcp::{
    TuiMcpAction, TuiMcpConfigDiagnostic, TuiMcpFileScope, TuiMcpFileSource, TuiMcpInstallRequest,
    TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerSource,
    TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpSyncedTemplateProvenance, TuiMcpTemplateVariable,
    TuiMcpTransport, TuiMcpVariableValue,
};
use url::Url;
pub use user_info::{TuiUserInfoManager, TuiUserInfoManagerEvent, TuiUserInfoSnapshot};
use warp_core::channel::ChannelState;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::TuiMountFn;
use crate::ai::mcp::FileBasedMCPManager;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::{self, AuthStateProvider};
use crate::terminal::focus_env::FOCUS_URL_ENV;

/// Login state of the headless TUI, observed by the `warp_tui` root view to
/// decide whether to show the login placeholder or the input UI.
pub enum TuiLoginPhase {
    /// Logged out and waiting for the user to explicitly begin browser login.
    SignedOutWelcome,
    /// Waiting for the user to finish the device-authorization login. The
    /// exact URL opened in the browser is surfaced once known (the alt screen
    /// hides stdout, so it cannot be printed there).
    AwaitingLogin { browser_url: Option<String> },
    /// Login failed; the placeholder shows the message so the user can quit.
    Failed { message: String },
    /// Authenticated — the input UI can be shown.
    LoggedIn,
}

/// Events emitted by [`TuiLoginModel`].
pub enum TuiLoginEvent {
    /// The login phase changed and the root view must repaint.
    PhaseChanged,
    /// Authentication completed and the TUI can create its terminal session.
    LoggedIn,
    /// The current user logged out and the TUI should return to authentication.
    LoggedOut,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TuiAuthBrowserFlow {
    DirectDeviceAuthorization,
    LogoutThenDeviceAuthorizationPending,
    LogoutThenDeviceAuthorizationOpened,
}

/// Singleton holding the TUI's [`TuiLoginPhase`]. Updated by [`init`]'s auth
/// flow and read by the `warp_tui` root view.
pub struct TuiLoginModel {
    phase: TuiLoginPhase,
    browser_flow: TuiAuthBrowserFlow,
}

impl TuiLoginModel {
    /// The current login phase.
    pub fn phase(&self) -> &TuiLoginPhase {
        &self.phase
    }
    /// Starts device authorization from the signed-out welcome screen.
    pub fn start_device_login(ctx: &mut AppContext) {
        start_tui_device_login(ctx);
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn signed_out_for_test() -> Self {
        Self {
            phase: TuiLoginPhase::SignedOutWelcome,
            browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
        }
    }
}

impl Entity for TuiLoginModel {
    type Event = TuiLoginEvent;
}

impl SingletonEntity for TuiLoginModel {}

/// Entry point invoked from `run_internal` once the headless app is initialized.
///
/// Registers the [`TuiLoginModel`], mounts the TUI immediately, and shows an
/// explicit welcome screen when the user isn't already logged in.
pub(crate) fn init(mount: TuiMountFn, ctx: &mut AppContext) {
    let logged_in = AuthStateProvider::as_ref(ctx).get().is_logged_in();

    let initial_phase = if logged_in {
        TuiLoginPhase::LoggedIn
    } else {
        TuiLoginPhase::SignedOutWelcome
    };
    ctx.add_singleton_model(move |_| TuiLoginModel {
        phase: initial_phase,
        browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
    });
    ctx.add_singleton_model(TuiMcpManager::new);
    ctx.add_singleton_model(TuiUserInfoManager::new);

    // Keep the auth subscription alive for the full process lifetime so a
    // logged-in TUI can complete device authorization again after logout.
    ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, event, ctx| {
        handle_auth_manager_event(event, ctx);
    });
    // Mount the TUI now so it renders immediately; the root view shows the
    // login placeholder until the model flips to `LoggedIn`.
    mount(ctx);

    if logged_in {
        activate_global_mcp_servers(ctx);
    }
}

fn handle_auth_manager_event(event: &AuthManagerEvent, ctx: &mut AppContext) {
    match event {
        AuthManagerEvent::ReceivedDeviceAuthorizationCode {
            verification_url,
            verification_url_complete,
            user_code,
        } => {
            // Prefer the "complete" URL (device code pre-filled) for opening.
            let url_to_open = verification_url_complete
                .as_deref()
                .unwrap_or(verification_url.as_str());
            let verification_url = tui_verification_url(url_to_open, user_code);
            let url_to_open =
                TuiLoginModel::handle(ctx).update(ctx, |model, _| match model.browser_flow {
                    TuiAuthBrowserFlow::DirectDeviceAuthorization => Some(verification_url.clone()),
                    TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending => {
                        model.browser_flow =
                            TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationOpened;
                        Some(
                            auth::web_logout_url_with_continue(&verification_url)
                                .unwrap_or_else(auth::web_logout_url),
                        )
                    }
                    TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationOpened => None,
                });
            let Some(url_to_open) = url_to_open else {
                return;
            };
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    browser_url: Some(url_to_open.clone()),
                },
            );
            if !ctx.try_open_url(&url_to_open) {
                log::warn!("Unable to open the device authorization URL in the default browser");
            }
        }
        AuthManagerEvent::AuthComplete => {
            set_login_phase(ctx, TuiLoginPhase::LoggedIn);
            activate_global_mcp_servers(ctx);
        }
        AuthManagerEvent::AuthFailed(err) => {
            let should_finish_web_logout = TuiLoginModel::handle(ctx).update(ctx, |model, _| {
                let should_finish_web_logout = matches!(
                    model.browser_flow,
                    TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending
                );
                model.browser_flow = TuiAuthBrowserFlow::DirectDeviceAuthorization;
                should_finish_web_logout
            });
            if should_finish_web_logout {
                let logout_url = auth::web_logout_url();
                if !ctx.try_open_url(&logout_url) {
                    log::warn!("Unable to open the logout URL in the default browser");
                }
            }
            set_login_phase(
                ctx,
                TuiLoginPhase::Failed {
                    message: format!("{err:#}"),
                },
            );
        }
        _ => {}
    }
}

fn authorize_device(ctx: &mut AppContext) {
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}
fn tui_verification_url(verification_url: &str, user_code: &str) -> String {
    let focus_url = env::var(FOCUS_URL_ENV).ok();
    tui_verification_url_with_return(verification_url, user_code, focus_url.as_deref())
}

fn tui_verification_url_with_return(
    verification_url: &str,
    user_code: &str,
    focus_url: Option<&str>,
) -> String {
    let Ok(mut verification_url) = Url::parse(verification_url) else {
        return verification_url.to_owned();
    };
    let has_user_code = verification_url
        .query_pairs()
        .any(|(key, value)| key == "user_code" && !value.is_empty());
    let return_to = validated_tui_focus_url(focus_url);
    let mut query = verification_url.query_pairs_mut();
    if !has_user_code {
        query.append_pair("user_code", user_code);
    }
    query.append_pair("source", "warp-agent-cli");
    if let Some(return_to) = return_to {
        query.append_pair("return_to", &return_to);
    }
    drop(query);
    verification_url.into()
}

fn validated_tui_focus_url(focus_url: Option<&str>) -> Option<String> {
    let mut focus_url = Url::parse(focus_url?).ok()?;
    if focus_url.scheme() != ChannelState::url_scheme()
        || focus_url.host_str() != Some("session")
        || !focus_url.username().is_empty()
        || focus_url.password().is_some()
        || focus_url.port().is_some()
        || focus_url.query().is_some()
        || focus_url.fragment().is_some()
    {
        return None;
    }

    let session_uuid = {
        let mut path_segments = focus_url.path_segments()?;
        let session_uuid = path_segments.next()?.to_ascii_lowercase();
        if path_segments.next().is_some()
            || session_uuid.len() != 32
            || !session_uuid.chars().all(|char| char.is_ascii_hexdigit())
        {
            return None;
        }
        session_uuid
    };
    focus_url.set_path(&format!("/{session_uuid}"));
    Some(focus_url.into())
}

fn activate_global_mcp_servers(ctx: &mut AppContext) {
    FileBasedMCPManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.activate_global_warp_servers(ctx);
    });
}

/// Starts a fresh direct device-authorization flow from the signed-out welcome screen.
pub fn start_tui_device_login(ctx: &mut AppContext) {
    let should_authorize = TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        if !matches!(model.phase, TuiLoginPhase::SignedOutWelcome) {
            return false;
        }
        model.phase = TuiLoginPhase::AwaitingLogin { browser_url: None };
        model.browser_flow = TuiAuthBrowserFlow::DirectDeviceAuthorization;
        ctx.notify();
        ctx.emit(TuiLoginEvent::PhaseChanged);
        true
    });
    if should_authorize {
        authorize_device(ctx);
    }
}
/// Logs out the current TUI user and sends them to Warp web's logged-out flow.
pub fn log_out_tui(ctx: &mut AppContext) {
    auth::log_out(ctx);
    set_logged_out_phase(ctx);
    authorize_device(ctx);
}

fn set_logged_out_phase(ctx: &mut AppContext) {
    TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        model.phase = TuiLoginPhase::AwaitingLogin { browser_url: None };
        model.browser_flow = TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending;
        ctx.notify();
        ctx.emit(TuiLoginEvent::PhaseChanged);
        ctx.emit(TuiLoginEvent::LoggedOut);
    });
}

/// Updates the shared [`TuiLoginModel`] phase and notifies observers, so the
/// root view re-renders (and the TUI driver repaints). Emits
/// [`TuiLoginEvent::LoggedIn`] when authentication completes.
fn set_login_phase(ctx: &mut AppContext, phase: TuiLoginPhase) {
    TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        let logged_in = matches!(phase, TuiLoginPhase::LoggedIn);
        model.phase = phase;
        if logged_in {
            model.browser_flow = TuiAuthBrowserFlow::DirectDeviceAuthorization;
        }
        ctx.notify();
        ctx.emit(TuiLoginEvent::PhaseChanged);
        if logged_in {
            ctx.emit(TuiLoginEvent::LoggedIn);
        }
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
