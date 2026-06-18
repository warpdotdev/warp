//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`WarpTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! For v0.0.1 this renders a centered Warp logo + version and quits on
//! `q` / `Esc` / `Ctrl-C`; no agent harness is wired up yet. [`init`] is called
//! from `run_internal` once the headless app is up (see [`crate::run_tui`]).

use warpui_core::elements::tui::{
    Modifier, TuiCenter, TuiColumn, TuiElement, TuiEventContext, TuiEventHandler, TuiStyle, TuiText,
};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, SingletonEntity, TuiView, TypedActionView,
};

/// The Warp wordmark, one entry per row. Rows are padded to a common width at
/// render time so center-aligning each row keeps the block aligned. (Placeholder
/// art; refine later.)
const WARP_LOGO_ROWS: &[&str] = &[
    "██     ██  █████  ██████  ██████",
    "██     ██ ██   ██ ██   ██ ██   ██",
    "██  █  ██ ███████ ██████  ██████",
    "██ ███ ██ ██   ██ ██   ██ ██",
    " ███ ███  ██   ██ ██   ██ ██",
];

const VERSION: &str = "v0.0.1";

/// Joins [`WARP_LOGO_ROWS`] into one string, padding every row (with trailing
/// spaces) to the widest row so center alignment offsets each row identically.
fn logo_text() -> String {
    let width = WARP_LOGO_ROWS
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);
    WARP_LOGO_ROWS
        .iter()
        .map(|row| {
            let padding = width - row.chars().count();
            format!("{row}{:padding$}", "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The root TUI view: a centered Warp logo above the version string.
struct WarpTuiView;

impl Entity for WarpTuiView {
    type Event = ();
}

impl TuiView for WarpTuiView {
    fn ui_name() -> &'static str {
        "WarpTuiView"
    }

    fn render(&self, _ctx: &AppContext) -> Box<dyn TuiElement> {
        let logo = TuiText::new(logo_text())
            .with_style(TuiStyle::default().add_modifier(Modifier::BOLD))
            .centered()
            .truncate();
        let version = TuiText::new(VERSION)
            .with_style(TuiStyle::default().add_modifier(Modifier::DIM))
            .centered();

        let column = TuiColumn::new()
            .child(logo)
            .child(TuiText::new(" "))
            .child(version);

        Box::new(
            TuiEventHandler::new(TuiCenter::new(column))
                .on_key("q", |_, ctx, _| request_quit(ctx))
                .on_key("escape", |_, ctx, _| request_quit(ctx)),
        )
    }
}

impl TypedActionView for WarpTuiView {
    // No typed actions yet; quitting is requested directly via the event context.
    type Action = ();
}

/// Queues app termination from a TUI event handler, which only has shared access
/// to the [`AppContext`] during dispatch.
fn request_quit(ctx: &mut TuiEventContext) {
    ctx.dispatch_app_update(|ctx| ctx.terminate_app(TerminationMode::ForceTerminate, None));
}

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Creates the TUI root window and starts the headless draw + input driver.
/// Registered as a singleton so the session lives for the app's lifetime.
pub fn init(ctx: &mut AppContext) {
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        |_| WarpTuiView,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}
