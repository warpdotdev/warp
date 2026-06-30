//! The headless `warp-tui` front-end's session bootstrap.
//!
//! [`run`] boots the real headless Warp app via [`warp::run_tui`]. Once shared
//! initialization is done, the mount built here starts the transcript-capable
//! TUI session and keeps both the TUI driver and terminal manager alive for the
//! app's lifetime.

use std::collections::HashMap;
use std::ffi::OsString;

use anyhow::Result;
use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    tui_dark_theme, Appearance, BannerState, IsSharedSessionCreator, LocalTtyTerminalManager,
    TerminalManagerTrait, TerminalSurfaceResult,
};
use warpui::SingletonEntity;
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{AddWindowOptions, AppContext, Entity, ModelHandle, ViewHandle};

use crate::root_view::RootTuiView;
use crate::terminal_session_view::TuiTerminalSessionView;

/// Holds the live TUI session for the app's lifetime.
struct TuiSession {
    _driver: TuiDriverHandle,
    _manager: ModelHandle<Box<dyn TerminalManagerTrait>>,
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

/// Creates the transcript root surface and starts the headless draw + input
/// driver. Registered as a singleton so the session lives for the app lifetime.
fn init(ctx: &mut AppContext) {
    // The current TUI transcript design is dark-mode-only. Keep this scoped to
    // the TUI process by overriding the already-initialized Appearance theme at
    // mount time, without changing normal GUI theme selection or font settings.
    Appearance::handle(ctx).update(ctx, |appearance, ctx| {
        appearance.set_theme(tui_dark_theme(), ctx);
    });

    let banner = ctx.add_model(|_| BannerState::default());
    let mut root_for_driver = None;
    let manager = LocalTtyTerminalManager::<TuiTerminalSessionView>::create_tui_model(
        std::env::current_dir().ok(),
        HashMap::<OsString, OsString>::from_iter(std::env::vars_os()),
        IsSharedSessionCreator::No,
        None,
        banner,
        Vector2F::new(120., 24.),
        None,
        None,
        ctx,
        |surface_init, ctx| {
            let mut terminal_session_for_manager = None;
            let (window_id, root) = ctx.add_tui_window(
                AddWindowOptions {
                    window_style: WindowStyle::NotStealFocus,
                    ..Default::default()
                },
                |ctx| {
                    let terminal_session = ctx.add_typed_action_tui_view(|ctx| {
                        TuiTerminalSessionView::new(surface_init, ctx)
                    });
                    terminal_session_for_manager = Some(terminal_session.clone());
                    RootTuiView::new(terminal_session)
                },
            );
            let surface = terminal_session_for_manager
                .expect("root view construction should create a terminal session surface");
            root_for_driver = Some((window_id, root));
            TerminalSurfaceResult {
                surface,
                post_wire: |_manager: &mut LocalTtyTerminalManager<TuiTerminalSessionView>,
                            _surface: &ViewHandle<TuiTerminalSessionView>,
                            _ctx: &mut AppContext| {},
            }
        },
    );
    let Some((window_id, root)) = root_for_driver else {
        log::error!("failed to create TUI root view");
        ctx.terminate_app(
            TerminationMode::ForceTerminate,
            Some(Err(anyhow::anyhow!("failed to create TUI root view"))),
        );
        return;
    };
    match spawn_tui_driver(ctx, window_id, root) {
        Ok(driver) => {
            ctx.add_singleton_model(|_| TuiSession {
                _driver: driver,
                _manager: manager.manager,
            });
        }
        Err(error) => {
            log::error!("failed to start transcript TUI: {error}");
            ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error.into())));
        }
    }
}
