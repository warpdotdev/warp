//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real (headless) Warp app via [`warp::run_tui`], which runs
//! the full `initialize_app` (so auth state, `Appearance`, settings, etc.
//! exist) and then runs the mount built here. The mount calls [`init`] with
//! [`RootTuiView`]: it creates the TUI window and starts the non-blocking
//! [`spawn_tui_driver`] draw + input loop, keeping the returned
//! [`TuiDriverHandle`] alive in a [`TuiSession`] singleton for the app's
//! lifetime. `Ctrl-C` quits (handled by the driver).
//!
//! Living in `warp_tui` (rather than `warp`) keeps all of the TUI front-end —
//! the root view, the window/driver bootstrap, and the session that owns the
//! driver handle — in one crate, so `warp` stays unaware of the concrete UI and
//! never has to depend on `warp_tui`.

use anyhow::Result;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, SingletonEntity, TuiView, TypedActionView, ViewContext,
};

use crate::RootTuiView;

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Boots the headless Warp app and renders [`RootTuiView`] to the terminal.
///
/// The mount closure (which builds the root view and starts the driver) is
/// constructed here and handed to [`warp::run_tui`], so `warp` itself never has
/// to depend on `warp_tui`. Called by the `warp-tui` binaries.
pub fn run() -> Result<()> {
    // If this process was re-exec'd as a Warp worker (e.g. the terminal
    // server), dispatch that instead of starting another TUI — otherwise the
    // worker re-exec would recursively launch TUIs.
    if let Some(result) = warp::run_tui_worker_if_requested() {
        return result;
    }
    warp::run_tui(Box::new(|ctx| init(ctx, RootTuiView::new)))
}

/// Creates the TUI root window from `build_root` and starts the headless draw +
/// input driver. Registered as a singleton so the session lives for the app's
/// lifetime. Invoked from `run_internal` (via [`run`]) once the headless app is
/// initialized.
fn init<R, F>(ctx: &mut AppContext, build_root: F)
where
    R: TuiView + TypedActionView,
    F: FnOnce(&mut ViewContext<R>) -> R,
{
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        build_root,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            // The alternate screen was never entered (entering it is what
            // failed), so also print to stderr — otherwise the process exits
            // with the reason buried in the log file.
            eprintln!(
                "warp-tui: could not start the terminal UI: {error}\n\
                 Run it directly in an interactive terminal (a real TTY), not piped or backgrounded."
            );
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}
