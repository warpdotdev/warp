//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real headless Warp app via [`warp::run_tui`]. Once shared
//! initialization is done, the mount built here starts the TUI driver and
//! defers creating the transcript-capable terminal session until login.

use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::Result;
use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    dark_theme, Appearance, BannerState, IsSharedSessionCreator, LocalTtyTerminalManager,
    TerminalManagerTrait, TerminalSurfaceResult,
};
use warp::{TuiLoginModel, TuiLoginPhase};
use warpui::SingletonEntity;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{AddWindowOptions, AppContext, Entity, ModelHandle, ViewHandle};

use crate::root_view::RootTuiView;
use crate::terminal_session_view::TuiTerminalSessionView;

/// Holds the live TUI driver and, after login, the terminal manager.
struct TuiSession {
    #[expect(dead_code, reason = "keeps the TUI driver alive for the TUI session")]
    driver: TuiDriverHandle,
    manager: Option<ModelHandle<Box<dyn TerminalManagerTrait>>>,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Boots the headless Warp app and mounts the transcript-capable TUI session.
pub fn run() -> Result<()> {
    // If this process was re-exec'd as a Warp worker (e.g. the terminal
    // server), dispatch that instead of starting another TUI — otherwise the
    // worker re-exec would recursively launch TUIs.
    if let Some(result) = warp::run_tui_worker_if_requested() {
        return result;
    }
    warp::run_tui(Box::new(init))
}

/// Creates the login-gated TUI root and starts the headless draw + input driver.
fn init(ctx: &mut AppContext) {
    // Register the TUI views' keybindings (and, in debug builds, the
    // cross-surface binding validators) before any input can be dispatched.
    crate::keybindings::init(ctx);

    // The current TUI transcript design is dark-mode-only. Keep this scoped to
    // the TUI process by overriding the already-initialized Appearance theme at
    // mount time, without changing normal GUI theme selection or font settings.
    Appearance::handle(ctx).update(ctx, |appearance, ctx| {
        appearance.set_theme(dark_theme(), ctx);
    });

    let banner = ctx.add_model(|_| BannerState::default());
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_| RootTuiView::new(),
    );
    match spawn_tui_driver(ctx, window_id, root.clone()) {
        Ok(driver) => {
            let session = ctx.add_singleton_model(|_| TuiSession {
                driver,
                manager: None,
            });
            if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
                // Already authenticated at mount: create the session now.
                create_terminal_session_after_login(&session, &root, &banner, ctx);
            } else {
                // Otherwise wait for login to complete and create it then.
                let session_for_login = session.clone();
                let root_for_login = root.clone();
                let banner_for_login = banner.clone();
                let login_model = TuiLoginModel::handle(ctx);
                ctx.subscribe_to_model(&login_model, move |_, _, ctx| {
                    if matches!(TuiLoginModel::as_ref(ctx).phase(), TuiLoginPhase::LoggedIn) {
                        create_terminal_session_after_login(
                            &session_for_login,
                            &root_for_login,
                            &banner_for_login,
                            ctx,
                        );
                    }
                });
            }
        }
        Err(error) => {
            log::error!("failed to start transcript TUI: {error}");
            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error.into())));
        }
    }
}

/// Creates and retains the terminal manager after login.
fn create_terminal_session_after_login(
    session: &ModelHandle<TuiSession>,
    root: &ViewHandle<RootTuiView>,
    banner: &ModelHandle<BannerState>,
    ctx: &mut AppContext,
) {
    if session.read(ctx, |session, _| session.manager.is_some()) {
        return;
    }

    let root = root.clone();
    let manager = LocalTtyTerminalManager::<TuiTerminalSessionView>::create_tui_model(
        std::env::current_dir().ok(),
        HashMap::<OsString, OsString>::from_iter(std::env::vars_os()),
        IsSharedSessionCreator::No,
        None,
        banner.clone(),
        Vector2F::new(120., 24.),
        None,
        None,
        ctx,
        move |surface_init, ctx| {
            let surface = root.update(ctx, |root, ctx| {
                let surface = root.create_terminal_session(surface_init, ctx);
                // Re-render the root so it swaps the login placeholder for the session.
                ctx.notify();
                surface
            });
            TerminalSurfaceResult {
                surface,
                post_wire: |_manager: &mut LocalTtyTerminalManager<TuiTerminalSessionView>,
                            _surface: &ViewHandle<TuiTerminalSessionView>,
                            _ctx: &mut AppContext| {},
            }
        },
    );

    session.update(ctx, |session, ctx| {
        session.manager = Some(manager.manager);
        ctx.notify();
    });
}
