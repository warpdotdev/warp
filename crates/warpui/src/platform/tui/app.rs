//! TUI platform backend entry point.
//!
//! Modeled on `headless/app.rs`: it constructs the platform delegate, window
//! manager and (monospace) font DB, wires up an [`AppCallbackDispatcher`], puts
//! the terminal into raw mode + alternate screen via [`TerminalRenderer`],
//! spawns a thread that reads terminal input, and then runs the main event
//! loop.
//!
//! [`AppCallbackDispatcher`]: warpui_core::platform::app::AppCallbackDispatcher

use std::sync::mpsc;
use std::thread;

use futures::future::LocalBoxFuture;

use super::delegate::{self, AppDelegate};
use super::event_loop::{self, AppEvent};
use super::font::FontDB;
use super::render::TerminalRenderer;
use super::windowing::WindowManager;
use crate::integration::TestDriver;
use crate::platform::app::TerminationResult;
use crate::platform::{self};
use crate::{AppContext, AssetProvider};

pub struct App {
    callbacks: platform::app::AppCallbacks,
    assets: Box<dyn AssetProvider>,
}

impl App {
    pub(in crate::platform) fn new(
        callbacks: platform::app::AppCallbacks,
        assets: Box<dyn AssetProvider>,
        test_driver: Option<&TestDriver>,
    ) -> Self {
        // The TUI backend does not use the integration test driver.
        let _ = test_driver;
        Self { callbacks, assets }
    }

    pub(in crate::platform) fn run(
        self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) -> TerminationResult {
        let App { callbacks, assets } = self;

        let (sender, receiver) = mpsc::channel::<AppEvent>();

        // Mark this thread as the main thread for DispatchDelegate checks.
        delegate::mark_current_thread_as_main();

        let platform_delegate = Box::new(AppDelegate::new(sender.clone()));
        let window_manager = Box::new(WindowManager::new(sender.clone()));
        let font_db: Box<dyn platform::FontDB> = Box::new(FontDB::new());

        let ui_app = crate::App::new(platform_delegate, window_manager, font_db, assets)
            .expect("should not fail to construct application");

        let mut callbacks =
            warpui_core::platform::app::AppCallbackDispatcher::new(callbacks, ui_app.clone());

        // Put the terminal into raw mode + alternate screen. Dropping the
        // renderer (when the event loop returns) restores the terminal.
        let renderer = match TerminalRenderer::new() {
            Ok(renderer) => renderer,
            Err(e) => {
                log::error!("Failed to initialize TUI terminal renderer: {e:#}");
                return Ok(());
            }
        };

        // Read terminal input on a background thread and forward each event to
        // the main loop.
        spawn_input_thread(sender.clone());

        event_loop::run(
            ui_app,
            &mut callbacks,
            Box::new(init_fn),
            receiver,
            sender,
            renderer,
        )
    }
}

/// Spawns a thread that blocks on crossterm input events and forwards each one
/// to the main event loop as an [`AppEvent::TerminalInput`].
fn spawn_input_thread(sender: mpsc::Sender<AppEvent>) {
    thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(event) => {
                if sender.send(AppEvent::TerminalInput(event)).is_err() {
                    break;
                }
            }
            Err(e) => {
                log::warn!("TUI input read error, stopping input thread: {e:#}");
                break;
            }
        }
    });
}
