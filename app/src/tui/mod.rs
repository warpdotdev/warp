//! The headless `warp-tui` front-end's app-side entry point.
//!
//! For this first step the TUI only proves out auth reuse. The `warp_tui` crate
//! boots the real (headless) Warp app via [`crate::run_tui`], which runs the
//! full `initialize_app` (so `AuthManager` and the auth state exist) and then
//! calls [`init`]. [`init`] logs the user in when needed (reusing the OAuth
//! device-authorization flow that `oz login` uses), auto-opens the browser,
//! prints the authenticated user's ID, and exits. No terminal UI is rendered
//! yet.

// This module is pre-built infrastructure not yet wired into the warp-tui binary.
// Suppress dead_code and unused_imports until it is integrated.
#[allow(dead_code, unused_imports)]
pub mod input;

use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;

/// Entry point invoked from `run_internal` once the headless app is initialized.
///
/// If the user is already logged in, prints their ID immediately. Otherwise it
/// starts the OAuth device-authorization flow, opens the browser (printing the
/// URL/code as a fallback), and prints the ID once login completes. Terminates
/// the app when done.
pub fn init(ctx: &mut AppContext) {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_logged_in() {
        print_user_id_and_exit(ctx);
        return;
    }

    println!("Welcome to Warp TUI. Let's get you logged in.");

    // Reuses the same device-authorization flow as `oz login` (see
    // `app/src/ai/agent_sdk/admin.rs`). The browser handles login and control
    // returns here once the device code is approved.
    ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, ctx| match event {
        AuthManagerEvent::ReceivedDeviceAuthorizationCode {
            verification_url,
            verification_url_complete,
            user_code,
        } => {
            // Prefer the "complete" URL (device code pre-filled) for opening.
            let url_to_open = verification_url_complete
                .as_deref()
                .unwrap_or(verification_url.as_str());

            // Auto-open the browser (works in headless mode too), and also print
            // the URL/code as a fallback for remote/SSH sessions where a local
            // browser can't be opened.
            if verification_url_complete.is_some() {
                println!("Opening your browser to log in.\nIf it doesn't open, visit:\n{url_to_open}");
            } else {
                println!(
                    "Opening your browser to log in.\nIf it doesn't open, visit {verification_url} and enter this code: {user_code}"
                );
            }
            ctx.open_url(url_to_open);
        }
        AuthManagerEvent::AuthComplete => {
            print_user_id_and_exit(ctx);
        }
        AuthManagerEvent::AuthFailed(err) => {
            ctx.terminate_app(
                TerminationMode::ForceTerminate,
                Some(Err(anyhow::anyhow!("Authentication failed: {err:#}"))),
            );
        }
        _ => {}
    });

    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
        auth_manager.authorize_device(ctx);
    });
}

/// Prints the authenticated user's ID to stdout, then terminates the app.
fn print_user_id_and_exit(ctx: &mut AppContext) {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    match auth_state.user_id() {
        Some(user_id) => {
            // Strip the service-account prefix the same way `oz whoami` does.
            let uid_full = user_id.as_string();
            let uid = uid_full
                .strip_prefix("serviceAccount:")
                .unwrap_or(uid_full.as_str());
            println!("Logged in. User ID: {uid}");
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
        None => {
            ctx.terminate_app(
                TerminationMode::ForceTerminate,
                Some(Err(anyhow::anyhow!(
                    "Could not determine user ID after login."
                ))),
            );
        }
    }
}
