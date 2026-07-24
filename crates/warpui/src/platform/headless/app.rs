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
    query_microphone_access: bool,
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
        Self {
            callbacks,
            assets,
            query_microphone_access: false,
        }
    }

    pub(in crate::platform) fn enable_microphone_access_query(&mut self) {
        self.query_microphone_access = true;
    }

    pub(in crate::platform) fn run(
        self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) -> TerminationResult {
        let App {
            callbacks,
            assets,
            query_microphone_access,
        } = self;

        // Mark this thread as the main thread for DispatchDelegate checks.
        delegate::mark_current_thread_as_main();
        let (sender, receiver) = event_loop::channel();

        let platform_delegate = Box::new(AppDelegate::new(sender.clone(), query_microphone_access));
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
