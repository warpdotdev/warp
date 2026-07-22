use futures::future::LocalBoxFuture;

use super::delegate::{self, AppDelegate};
use super::event_loop;
use super::windowing::WindowManager;
use crate::integration::TestDriver;
use crate::platform::app::TerminationResult;
use crate::platform::test::FontDB as TestFontDB;
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
        // Other platforms use the test_driver parameter to enable an alternative platform delegate implementation
        // in integration tests - that doesn't apply here.
        let _ = test_driver;
        Self { callbacks, assets }
    }

    pub(in crate::platform) fn run(
        self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) -> TerminationResult {
        let App { callbacks, assets } = self;

        // Mark this thread as the main thread for DispatchDelegate checks.
        delegate::mark_current_thread_as_main();
        let (sender, receiver) = event_loop::channel();

        let platform_delegate = Box::new(AppDelegate::new(sender.clone()));
        let window_manager = Box::new(WindowManager::new(sender.clone()));
        // Reuse the testing FontDB implementation, as no font features are needed in headless mode.
        let font_db: Box<dyn platform::FontDB> = Box::new(TestFontDB::new());

        let ui_app = crate::App::new(platform_delegate, window_manager, font_db, assets)
            .expect("should not fail to construct application");

        let mut callbacks =
            warpui_core::platform::app::AppCallbackDispatcher::new(callbacks, ui_app.clone());

        // Run the event loop until the app terminates.
        event_loop::run(ui_app, &mut callbacks, Box::new(init_fn), receiver, sender)
    }
}

/// Construct a headless [`crate::App`] (with an [`AppContext`]) **without**
/// running the blocking platform event loop.
///
/// This is intended for embeddings that drive the app asynchronously from a
/// host runtime (e.g. the wasm32-unknown-unknown + Node prototype, REMOTE-2264):
/// on wasm the foreground/background executors schedule via
/// `wasm_bindgen_futures::spawn_local`, so a `#[wasm_bindgen] pub async fn`
/// can spawn work on the returned app's foreground executor and `await` it —
/// the JS event loop polls the spawned futures as the async fn yields, with no
/// blocking `event_loop::run` (which would hang the single JS thread).
///
/// Platform callbacks that route through the event-loop channel (terminate,
/// file pickers, notifications) become no-ops when the channel receiver is
/// dropped (the `EventSender` logs and discards them), which is acceptable for
/// a headless agent run with no UI.
pub fn new_headless_app(assets: Box<dyn AssetProvider>) -> anyhow::Result<crate::App> {
    // Mark this thread as the main thread for DispatchDelegate checks.
    delegate::mark_current_thread_as_main();
    let (sender, _receiver) = event_loop::channel();

    let platform_delegate = Box::new(AppDelegate::new(sender.clone()));
    let window_manager = Box::new(WindowManager::new(sender.clone()));
    let font_db: Box<dyn platform::FontDB> = Box::new(TestFontDB::new());

    crate::App::new(platform_delegate, window_manager, font_db, assets)
}
