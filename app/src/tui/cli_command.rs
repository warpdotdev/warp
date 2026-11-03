use anyhow::anyhow;
use warp_server_client::iap::{IapManager, IapManagerEvent};
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};

type ExecuteCommand = Box<dyn FnOnce(&mut AppContext)>;

/// Refreshes saved credentials before executing a command, or terminates with an error.
pub(crate) fn authenticate_then_execute(execute: ExecuteCommand, ctx: &mut AppContext) {
    if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
        ctx.terminate_app(
            TerminationMode::ForceTerminate,
            Some(Err(anyhow!(
                "You are not logged in. Open Warp Agent CLI to log in first."
            ))),
        );
        return;
    }

    let iap = IapManager::handle(ctx);
    if !iap.as_ref(ctx).is_enabled() || iap.as_ref(ctx).has_valid_token() {
        refresh_then_execute(execute, ctx);
        return;
    }

    let mut execute = Some(execute);
    ctx.subscribe_to_model(&iap, move |iap, event, ctx| {
        if execute.is_none() {
            return;
        }
        match event {
            IapManagerEvent::StateChanged => {
                if iap.as_ref(ctx).has_valid_token()
                    && let Some(execute) = execute.take()
                {
                    refresh_then_execute(execute, ctx);
                }
            }
            IapManagerEvent::AccessUnavailable => {
                execute = None;
                ctx.terminate_app(
                    TerminationMode::ForceTerminate,
                    Some(Err(anyhow!(
                        "Timed out establishing IAP access to warp-server."
                    ))),
                );
            }
            IapManagerEvent::RefreshFailed { .. } => {}
        }
    });
    iap.update(ctx, |manager, ctx| manager.ensure_access(ctx));
}

fn refresh_then_execute(execute: ExecuteCommand, ctx: &mut AppContext) {
    let mut execute = Some(execute);
    ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, ctx| {
        if execute.is_none() {
            return;
        }
        match event {
            AuthManagerEvent::AuthComplete => {
                if let Some(execute) = execute.take() {
                    execute(ctx);
                }
            }
            AuthManagerEvent::NeedsReauth => {
                execute = None;
                ctx.terminate_app(
                    TerminationMode::ForceTerminate,
                    Some(Err(anyhow!(
                        "Your credentials are invalid. Open Warp Agent CLI to log in again."
                    ))),
                );
            }
            AuthManagerEvent::AuthFailed(error) => {
                execute = None;
                ctx.terminate_app(
                    TerminationMode::ForceTerminate,
                    Some(Err(anyhow!("Authentication failed: {error:#}"))),
                );
            }
            AuthManagerEvent::CreateAnonymousUserFailed
            | AuthManagerEvent::SkippedLogin
            | AuthManagerEvent::AttemptedLoginGatedFeature { .. }
            | AuthManagerEvent::LoginOverrideDetected(_)
            | AuthManagerEvent::MintCustomTokenFailed(_)
            | AuthManagerEvent::ReceivedDeviceAuthorizationCode { .. } => {}
        }
    });
    AuthManager::handle(ctx).update(ctx, |manager, ctx| manager.refresh_user(ctx));
}

#[cfg(test)]
#[path = "cli_command_tests.rs"]
mod tests;
