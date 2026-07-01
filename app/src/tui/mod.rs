//! The headless `warp-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the real headless Warp app via [`crate::run_tui`]. Once
//! shared initialization is done, [`init`] registers the [`TuiLoginModel`] that
//! the TUI observes, mounts the TUI immediately (so it renders right away), and
//! — when the user isn't logged in yet — drives the device-authorization login
//! flow, flipping the model to [`TuiLoginPhase::LoggedIn`] when it completes.

use settings::Setting as _;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::settings::AISettings;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::TuiMountFn;

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
    type Event = ();
}

impl SingletonEntity for TuiLoginModel {}

/// Entry point invoked from `run_internal` once the headless app is initialized.
///
/// Registers the [`TuiLoginModel`], mounts the TUI immediately, and runs the
/// device-authorization login flow when the user isn't already logged in.
pub(crate) fn init(mount: TuiMountFn, ctx: &mut AppContext) {
    // Temporary end-to-end validation of the settings-file flow: log the parsed
    // values on startup and on every hot reload. Not wired into agent behavior
    // yet — it just proves the TUI reads and live-reloads its settings file.
    init_settings_validation_logging(ctx);

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

    // Mount the TUI now so it renders immediately; the root view shows the
    // login placeholder until the model flips to `LoggedIn`.
    mount(ctx);

    if logged_in {
        return;
    }

    // Reuses the same device-authorization flow as `oz login` (see
    // `app/src/ai/agent_sdk/admin.rs`). The browser handles login; control
    // returns here once the device code is approved.
    ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, event, ctx| match event {
        AuthManagerEvent::ReceivedDeviceAuthorizationCode {
            verification_url,
            verification_url_complete,
            user_code,
        } => {
            // Prefer the "complete" URL (device code pre-filled) for opening.
            let url_to_open = verification_url_complete
                .as_deref()
                .unwrap_or(verification_url.as_str());
            ctx.open_url(url_to_open);
            set_login_phase(
                ctx,
                TuiLoginPhase::AwaitingLogin {
                    verification_uri: Some(url_to_open.to_owned()),
                    user_code: Some(user_code.clone()),
                },
            );
        }
        AuthManagerEvent::AuthComplete => set_login_phase(ctx, TuiLoginPhase::LoggedIn),
        AuthManagerEvent::AuthFailed(err) => set_login_phase(
            ctx,
            TuiLoginPhase::Failed {
                message: format!("{err:#}"),
            },
        ),
        _ => {}
    });

    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}

/// Updates the shared [`TuiLoginModel`] phase and notifies observers, so the
/// root view re-renders (and the TUI driver repaints).
fn set_login_phase(ctx: &mut AppContext, phase: TuiLoginPhase) {
    TuiLoginModel::handle(ctx).update(ctx, |model, ctx| {
        model.phase = phase;
        ctx.notify();
    });
}

/// Sets up the temporary end-to-end validation of the settings-file flow.
///
/// Logs a snapshot of the parsed settings on startup, then re-logs it on every
/// settings-file reload (and logs any parse/validation errors). This is purely
/// a validation aid — it does not change agent behavior.
fn init_settings_validation_logging(ctx: &mut AppContext) {
    if let Some(err) = &GlobalResourceHandlesProvider::as_ref(ctx)
        .get()
        .settings_file_error
    {
        log::warn!("[tui-settings] startup parse/validation error: {err}");
    }
    log_settings_snapshot("startup", ctx);

    // The settings watcher (registered during `initialize_app`) reloads values
    // into the in-memory models and then emits these events; re-log the
    // refreshed snapshot so external file edits are observable in the log.
    ctx.subscribe_to_model(&WarpConfig::handle(ctx), |_, event, ctx| match event {
        WarpConfigUpdateEvent::SettingsErrorsCleared => {
            log_settings_snapshot("reloaded", ctx);
        }
        WarpConfigUpdateEvent::SettingsErrors(err) => {
            log::warn!("[tui-settings] reload error: {err}");
            // On a whole-file parse error the in-memory values are retained;
            // on per-key errors the offending keys fall back to defaults.
            log_settings_snapshot("values in effect after reload error", ctx);
        }
        WarpConfigUpdateEvent::Themes
        | WarpConfigUpdateEvent::LocalUserWorkflows
        | WarpConfigUpdateEvent::LaunchConfigs
        | WarpConfigUpdateEvent::TabConfigs
        | WarpConfigUpdateEvent::TabConfigErrors(_)
        | WarpConfigUpdateEvent::ModelConfigs
        | WarpConfigUpdateEvent::ModelConfigErrors(_)
        | WarpConfigUpdateEvent::Settings => {}
    });
}

/// Logs the settings file path and the current values of the TUI-relevant
/// agent settings.
fn log_settings_snapshot(when: &str, ctx: &AppContext) {
    let path = crate::settings::user_preferences_toml_file_path();
    let ai = AISettings::as_ref(ctx);
    log::info!(
        "[tui-settings] {when}: file={} model={} is_any_ai_enabled={} \
         agent_mode_coding_permissions={:?} agent_mode_execute_readonly_commands={} \
         command_allowlist_len={} command_denylist_len={}",
        path.display(),
        ai.agent_model.value(),
        *ai.is_any_ai_enabled.value(),
        ai.agent_mode_coding_permissions.value(),
        *ai.agent_mode_execute_read_only_commands.value(),
        ai.agent_mode_command_execution_allowlist.value().len(),
        ai.agent_mode_command_execution_denylist.value().len(),
    );
}
