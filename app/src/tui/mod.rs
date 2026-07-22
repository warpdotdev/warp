//! The headless `warp-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the real headless Warp app via [`crate::run_tui`]. Once
//! shared initialization is done, [`init`] registers the [`TuiLoginModel`] that
//! the TUI observes, mounts the TUI immediately (so it renders right away), and
//! — when the user isn't logged in yet — drives the device-authorization login
//! flow, flipping the model to [`TuiLoginPhase::LoggedIn`] when it completes.
mod mcp;

pub use mcp::{
    TuiMcpAction, TuiMcpConfigState, TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId,
    TuiMcpServerSnapshot, TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpTransport,
};
use warpui::{AppContext, Entity, SingletonEntity};

use crate::TuiMountFn;
use crate::ai::mcp::FileBasedMCPManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};

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
    /// Authentication completed and the TUI can create its terminal session.
    LoggedIn,
    /// Authentication was cleared and the TUI should return to the login page.
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

    // Mount the TUI now so it renders immediately; the root view shows the
    // login placeholder until the model flips to `LoggedIn`.
    mount(ctx);

    // Reuses the same device-authorization flow as `oz login` (see
    // `app/src/ai/agent_sdk/admin.rs`). The browser handles login; control
    // returns here once the device code is approved.
    ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, event, ctx| match event {
        AuthManagerEvent::ReceivedDeviceAuthorizationCode {
            verification_url,
            verification_url_complete,
            user_code,
        } => {
            handle_received_device_authorization_code(
                verification_url,
                verification_url_complete.as_deref(),
                user_code,
                ctx,
            );
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
    });

    if logged_in {
        activate_global_mcp_servers(ctx);
        return;
    }

    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}

fn handle_received_device_authorization_code(
    verification_url: &str,
    verification_url_complete: Option<&str>,
    user_code: &str,
    ctx: &mut AppContext,
) {
    // A device-auth request can finish delivering its code after an
    // already-authenticated startup. It must not replace the logged-in TUI
    // with the login placeholder; device codes emitted after an explicit
    // logout are still accepted because the phase has already returned to
    // `AwaitingLogin`.
    if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
        return;
    }

    // Prefer the "complete" URL (device code pre-filled) for opening.
    let url_to_open = verification_url_complete.unwrap_or(verification_url);
    ctx.open_url(url_to_open);
    set_login_phase(
        ctx,
        TuiLoginPhase::AwaitingLogin {
            verification_uri: Some(url_to_open.to_owned()),
            user_code: Some(user_code.to_owned()),
        },
    );
}

/// Logs out the current TUI user and starts a fresh device-authorization flow.
///
/// The login model event is delivered after the current command handler returns,
/// allowing the session owner to tear down the dispatching terminal view safely.
pub fn log_out(ctx: &mut AppContext) {
    crate::auth::log_out(ctx);
    set_login_phase(
        ctx,
        TuiLoginPhase::AwaitingLogin {
            verification_uri: None,
            user_code: None,
        },
    );
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}

fn activate_global_mcp_servers(ctx: &mut AppContext) {
    FileBasedMCPManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.activate_global_warp_servers(ctx);
    });
}

/// Updates the shared [`TuiLoginModel`] phase and notifies observers, so the
/// root view re-renders (and the TUI driver repaints). Emits
/// [`TuiLoginEvent::LoggedIn`] when authentication completes.
fn set_login_phase(ctx: &mut AppContext, phase: TuiLoginPhase) {
    TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        let logged_in = matches!(&phase, TuiLoginPhase::LoggedIn);
        let logged_out = matches!(
            (&model.phase, &phase),
            (TuiLoginPhase::LoggedIn, TuiLoginPhase::AwaitingLogin { .. })
        );
        model.phase = phase;
        ctx.notify();
        if logged_in {
            ctx.emit(TuiLoginEvent::LoggedIn);
        }
        if logged_out {
            ctx.emit(TuiLoginEvent::LoggedOut);
        }
    });
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
