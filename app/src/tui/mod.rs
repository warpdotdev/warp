//! The headless `warp-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the real headless Warp app via [`crate::run_tui`]. Once
//! shared initialization is done, [`init`] registers the [`TuiLoginModel`] that
//! the TUI observes, mounts the TUI immediately (so it renders right away), and
//! — when the user isn't logged in yet — drives the device-authorization login
//! flow, flipping the model to [`TuiLoginPhase::LoggedIn`] when it completes.
mod mcp;
mod user_info;

pub use mcp::{
    TuiMcpAction, TuiMcpConfigState, TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId,
    TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpTransport,
};
use url::Url;
pub use user_info::{TuiUserInfoManager, TuiUserInfoManagerEvent, TuiUserInfoSnapshot};
use warpui::{AppContext, Entity, SingletonEntity};

use crate::TuiMountFn;
use crate::ai::mcp::FileBasedMCPManager;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::{self, AuthStateProvider};

/// Login state of the headless TUI, observed by the `warp_tui` root view to
/// decide whether to show the login placeholder or the input UI.
pub enum TuiLoginPhase {
    /// Waiting for the user to finish the device-authorization login. The
    /// verification URL/code are surfaced in the placeholder once known (the
    /// alt screen hides stdout, so they can't be printed there).
    AwaitingLogin {
        verification_uri: Option<String>,
        user_code: Option<String>,
    },
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

/// Singleton holding the TUI's [`TuiLoginPhase`]. Updated by [`init`]'s auth
/// flow and read by the `warp_tui` root view.
pub struct TuiLoginModel {
    phase: TuiLoginPhase,
}

impl TuiLoginModel {
    /// The current login phase.
    pub fn phase(&self) -> &TuiLoginPhase {
        &self.phase
    }
}

impl Entity for TuiLoginModel {
    type Event = TuiLoginEvent;
}

impl SingletonEntity for TuiLoginModel {}

/// Entry point invoked from `run_internal` once the headless app is initialized.
///
/// Registers the [`TuiLoginModel`], mounts the TUI immediately, and runs the
/// device-authorization login flow when the user isn't already logged in.
pub(crate) fn init(mount: TuiMountFn, ctx: &mut AppContext) {
    let logged_in = AuthStateProvider::as_ref(ctx).get().is_logged_in();

    let initial_phase = if logged_in {
        TuiLoginPhase::LoggedIn
    } else {
        TuiLoginPhase::AwaitingLogin {
            verification_uri: None,
            user_code: None,
        }
    };
    ctx.add_singleton_model(move |_| TuiLoginModel {
        phase: initial_phase,
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
    } else {
        authorize_device(ctx);
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
            let url_to_open = tui_verification_url(url_to_open);
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    verification_uri: Some(url_to_open.clone()),
                    user_code: Some(user_code.clone()),
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
        AuthManagerEvent::AuthFailed(err) => set_login_phase(
            ctx,
            TuiLoginPhase::Failed {
                message: format!("{err:#}"),
            },
        ),
        _ => {}
    }
}

fn authorize_device(ctx: &mut AppContext) {
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}
fn tui_verification_url(verification_url: &str) -> String {
    let Ok(mut verification_url) = Url::parse(verification_url) else {
        return verification_url.to_owned();
    };
    verification_url
        .query_pairs_mut()
        .append_pair("source", "warp-agent-cli");
    verification_url.into()
}

fn activate_global_mcp_servers(ctx: &mut AppContext) {
    FileBasedMCPManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.activate_global_warp_servers(ctx);
    });
}

/// Logs out the current TUI user and sends them to Warp web's logged-out flow.
pub fn log_out_tui(ctx: &mut AppContext) {
    auth::log_out_and_open_web(ctx);
    set_logged_out_phase(ctx);
}

fn set_logged_out_phase(ctx: &mut AppContext) {
    TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        model.phase = TuiLoginPhase::AwaitingLogin {
            verification_uri: None,
            user_code: None,
        };
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
