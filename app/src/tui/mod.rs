//! The headless `warp-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the real headless Warp app via [`crate::run_tui`]. Once
//! shared initialization is done, [`init`] registers the [`TuiLoginModel`] that
//! the TUI observes, mounts the TUI immediately (so it renders right away), and
//! leaves device authorization behind an explicit welcome-screen action. The
//! authentication gate remains visible until the browser flow completes.
mod mcp;
mod telemetry;
mod user_info;

pub use mcp::{
    TuiMcpAction, TuiMcpConfigDiagnostic, TuiMcpFileScope, TuiMcpFileSource, TuiMcpInstallRequest,
    TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerSource,
    TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpSyncedTemplateProvenance, TuiMcpTemplateVariable,
    TuiMcpTransport, TuiMcpVariableValue,
};
use telemetry::{
    AbandonmentPhase, AuthenticationEntrypoint, TuiOnboardingTelemetry, TuiOnboardingTelemetryEvent,
};
use url::Url;
pub use user_info::{TuiUserInfoManager, TuiUserInfoManagerEvent, TuiUserInfoSnapshot};
use warp_core::telemetry::TelemetryEvent as _;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::TuiMountFn;
use crate::ai::mcp::FileBasedMCPManager;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::auth_state::AuthState;
use crate::auth::{self, AuthStateProvider};
use crate::tui_onboarding_markers::TuiOnboardingMarkers;

/// Login state of the headless TUI, observed by the `warp_tui` root view to
/// decide whether to show the login placeholder or the input UI.
pub enum TuiLoginPhase {
    /// No validated user identity is available, so the login welcome remains visible.
    SignedOutWelcome,
    /// Waiting for the user to finish the device-authorization login. The
    /// exact URL opened in the browser is surfaced once known (the alt screen
    /// hides stdout, so it cannot be printed there).
    AwaitingLogin { browser_url: Option<String> },
    /// The authorization URL could not be opened automatically. The exact URL
    /// remains available for copy/retry.
    BrowserOpenFailed { browser_url: String },
    /// Login failed; the placeholder shows the message if no terminal is active.
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
    telemetry: TuiOnboardingTelemetry,
}

impl TuiLoginModel {
    /// The current login phase.
    pub fn phase(&self) -> &TuiLoginPhase {
        &self.phase
    }
    /// Starts or retries device authorization from a signed-out screen.
    pub fn start_device_login(ctx: &mut AppContext) {
        start_tui_device_login(ctx);
    }

    /// Starts device authorization and records that the generated URL should be copied.
    pub fn start_device_login_and_copy_url(ctx: &mut AppContext) {
        start_tui_device_login_with_entrypoint(AuthenticationEntrypoint::CopyUrl, ctx);
    }

    /// Records the outcome of copying the current authentication URL.
    pub fn record_login_url_copied(succeeded: bool, ctx: &mut AppContext) {
        let event =
            Self::handle(ctx).update(ctx, |model, _| model.telemetry.login_url_copied(succeeded));
        send_tui_onboarding_event(event, ctx);
    }

    /// Records that the user exited while the authentication UI was visible.
    pub fn record_authentication_abandoned(ctx: &mut AppContext) {
        let event = Self::handle(ctx).update(ctx, |model, _| {
            let phase = AbandonmentPhase::from_login_phase(&model.phase)?;
            model.telemetry.abandoned(phase)
        });
        send_tui_onboarding_event(event, ctx);
    }

    /// Records that the terminal became usable after interactive authentication.
    pub fn record_terminal_shown(ctx: &mut AppContext) {
        let event = Self::handle(ctx).update(ctx, |model, _| model.telemetry.completed());
        send_tui_onboarding_event(event, ctx);
    }

    /// Opens the current device-authorization URL.
    pub fn open_login_url(browser_url: &str, ctx: &mut AppContext) {
        let is_current_url = matches!(
            TuiLoginModel::as_ref(ctx).phase(),
            TuiLoginPhase::AwaitingLogin {
                browser_url: Some(current_url),
            } if current_url == browser_url
        ) || matches!(
            TuiLoginModel::as_ref(ctx).phase(),
            TuiLoginPhase::BrowserOpenFailed {
                browser_url: current_url,
            } if current_url == browser_url
        );
        if !is_current_url {
            return;
        }

        let retrying_after_failure = matches!(
            TuiLoginModel::as_ref(ctx).phase(),
            TuiLoginPhase::BrowserOpenFailed { .. }
        );
        let browser_opened = ctx.try_open_url(browser_url);
        let event = TuiLoginModel::handle(ctx).update(ctx, |model, _| {
            model.telemetry.browser_launch(browser_opened)
        });
        send_tui_onboarding_event(event, ctx);
        if !browser_opened {
            TuiLoginModel::handle(ctx).update(ctx, |model, _| {
                if model.browser_flow == TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationOpened {
                    model.browser_flow = TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending;
                }
            });
            set_login_phase(
                ctx,
                TuiLoginPhase::BrowserOpenFailed {
                    browser_url: browser_url.to_owned(),
                },
            );
            log::warn!("Unable to open the device authorization URL in the default browser");
            return;
        }

        TuiLoginModel::handle(ctx).update(ctx, |model, _| {
            if model.browser_flow == TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending {
                model.browser_flow = TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationOpened;
            }
        });
        if retrying_after_failure {
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    browser_url: Some(browser_url.to_owned()),
                },
            );
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn signed_out_for_test() -> Self {
        Self {
            phase: TuiLoginPhase::SignedOutWelcome,
            browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
            telemetry: TuiOnboardingTelemetry::new(false),
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn failed_for_test(message: impl Into<String>) -> Self {
        Self {
            phase: TuiLoginPhase::Failed {
                message: message.into(),
            },
            browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
            telemetry: TuiOnboardingTelemetry::new(false),
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn awaiting_login_for_test(browser_url: Option<String>) -> Self {
        Self {
            phase: TuiLoginPhase::AwaitingLogin { browser_url },
            browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
            telemetry: TuiOnboardingTelemetry::new(false),
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
    let initial_phase = initial_login_phase(AuthStateProvider::as_ref(ctx).get());
    let logged_in = matches!(&initial_phase, TuiLoginPhase::LoggedIn);
    ctx.add_singleton_model(move |_| TuiLoginModel {
        phase: initial_phase,
        browser_flow: TuiAuthBrowserFlow::DirectDeviceAuthorization,
        telemetry: TuiOnboardingTelemetry::new(logged_in),
    });
    ctx.add_singleton_model(TuiMcpManager::new);
    ctx.add_singleton_model(TuiUserInfoManager::new);
    let onboarding_markers = ctx.add_singleton_model(TuiOnboardingMarkers::new);

    // Keep the auth subscription alive for the full process lifetime so a
    // logged-in TUI can complete device authorization again after logout.
    ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, event, ctx| {
        handle_auth_manager_event(event, ctx);
    });
    if logged_in {
        onboarding_markers.update(ctx, |markers, ctx| {
            markers.load_current_account(ctx);
        });
    }
    // Mount the TUI now so it renders immediately; signed-out users see the
    // welcome screen before explicitly starting browser authentication.
    mount(ctx);

    if logged_in {
        activate_global_mcp_servers(ctx);
    }
}

fn has_validated_identity(auth_state: &AuthState) -> bool {
    auth_state.is_logged_in() && auth_state.user_id().is_some()
}

fn initial_login_phase(auth_state: &AuthState) -> TuiLoginPhase {
    if has_validated_identity(auth_state) {
        TuiLoginPhase::LoggedIn
    } else {
        TuiLoginPhase::SignedOutWelcome
    }
}

fn handle_auth_manager_event(event: &AuthManagerEvent, ctx: &mut AppContext) {
    match event {
        AuthManagerEvent::ReceivedDeviceAuthorizationCode {
            verification_url,
            verification_url_complete,
            user_code,
        } => {
            let event = TuiLoginModel::handle(ctx)
                .update(ctx, |model, _| model.telemetry.device_authorization_ready());
            send_tui_onboarding_event(event, ctx);
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
            TuiLoginModel::open_login_url(&url_to_open, ctx);
        }
        AuthManagerEvent::AuthComplete => {
            set_login_phase(ctx, TuiLoginPhase::LoggedIn);
            TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
                markers.load_current_account(ctx);
            });
            activate_global_mcp_servers(ctx);
        }
        AuthManagerEvent::AuthFailed(err) => {
            let event = TuiLoginModel::handle(ctx)
                .update(ctx, |model, _| model.telemetry.authentication_failed(err));
            send_tui_onboarding_event(event, ctx);
            let should_finish_web_logout = matches!(
                TuiLoginModel::as_ref(ctx).browser_flow,
                TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending
            );
            let browser_flow = if should_finish_web_logout {
                let logout_url = auth::web_logout_url();
                if ctx.try_open_url(&logout_url) {
                    TuiAuthBrowserFlow::DirectDeviceAuthorization
                } else {
                    log::warn!("Unable to open the logout URL in the default browser");
                    TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending
                }
            } else {
                TuiAuthBrowserFlow::DirectDeviceAuthorization
            };
            TuiLoginModel::handle(ctx).update(ctx, |model, _| {
                model.browser_flow = browser_flow;
            });
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
    let Ok(mut verification_url) = Url::parse(verification_url) else {
        return verification_url.to_owned();
    };
    let has_user_code = verification_url
        .query_pairs()
        .any(|(key, value)| key == "user_code" && !value.is_empty());
    let mut query = verification_url.query_pairs_mut();
    if !has_user_code {
        query.append_pair("user_code", user_code);
    }
    query.append_pair("source", "warp-agent-cli");
    drop(query);
    verification_url.into()
}

fn activate_global_mcp_servers(ctx: &mut AppContext) {
    FileBasedMCPManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.activate_global_warp_servers(ctx);
    });
}

/// Starts device authorization from a signed-out screen, preserving any required web logout.
pub fn start_tui_device_login(ctx: &mut AppContext) {
    start_tui_device_login_with_entrypoint(AuthenticationEntrypoint::OpenBrowser, ctx);
}

fn start_tui_device_login_with_entrypoint(
    entrypoint: AuthenticationEntrypoint,
    ctx: &mut AppContext,
) {
    let (should_authorize, event) = TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        match model.phase {
            TuiLoginPhase::SignedOutWelcome => {
                model.browser_flow = TuiAuthBrowserFlow::DirectDeviceAuthorization;
            }
            TuiLoginPhase::Failed { .. } => {}
            TuiLoginPhase::AwaitingLogin { .. }
            | TuiLoginPhase::BrowserOpenFailed { .. }
            | TuiLoginPhase::LoggedIn => return (false, None),
        }
        model.phase = TuiLoginPhase::AwaitingLogin { browser_url: None };
        let event = model.telemetry.authentication_started(entrypoint);
        ctx.notify();
        ctx.emit(TuiLoginEvent::PhaseChanged);
        (true, Some(event))
    });
    send_tui_onboarding_event(event, ctx);
    if should_authorize {
        authorize_device(ctx);
    }
}
/// Logs out the current TUI user and sends them to Warp web's logged-out flow.
pub fn log_out_tui(ctx: &mut AppContext) {
    auth::log_out(ctx);
    TuiOnboardingMarkers::handle(ctx).update(ctx, |markers, ctx| {
        markers.reset_for_account_transition(ctx);
    });
    set_logged_out_phase(ctx);
    authorize_device(ctx);
}

fn set_logged_out_phase(ctx: &mut AppContext) {
    let event = TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        model.phase = TuiLoginPhase::AwaitingLogin { browser_url: None };
        model.browser_flow = TuiAuthBrowserFlow::LogoutThenDeviceAuthorizationPending;
        let event = model.telemetry.post_logout_authentication_started();
        ctx.notify();
        ctx.emit(TuiLoginEvent::PhaseChanged);
        ctx.emit(TuiLoginEvent::LoggedOut);
        event
    });
    send_tui_onboarding_event(Some(event), ctx);
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

fn send_tui_onboarding_event(event: Option<TuiOnboardingTelemetryEvent>, ctx: &mut AppContext) {
    if let Some(event) = event {
        warp_core::send_telemetry_from_app_ctx!(event, ctx);
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
