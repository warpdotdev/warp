use warpui_core::integration::TestDriver;
use warpui_core::keymap::{CustomTag, Keystroke};
pub use warpui_core::platform::app::*;
use warpui_core::r#async::LocalBoxFuture;
use warpui_core::{AppContext, AssetProvider};

use super::AsInnerMut;

/// Platform-specific app implementation. On any given platform, there are at least two possible
/// implementations:
/// * The platform-native backend (e.g. Cocoa on macOS, or Winit+X11/Wayland on Linux)
/// * A headless backend
pub enum AppBackend {
    CurrentPlatform(Box<super::current::App>),
    Headless(Box<super::headless::App>),
    #[cfg(feature = "tui")]
    Tui(Box<super::tui::App>),
}

impl AppBackend {
    fn run(
        self,
        init_fn: impl FnOnce(&mut AppContext, LocalBoxFuture<'static, crate::App>) + 'static,
    ) -> TerminationResult {
        match self {
            AppBackend::CurrentPlatform(inner) => {
                inner.run(init_fn);
                // We don't report errors for the GUI app on termination.
                Ok(())
            }
            AppBackend::Headless(inner) => inner.run(init_fn),
            #[cfg(feature = "tui")]
            AppBackend::Tui(inner) => inner.run(init_fn),
        }
    }
}

/// A structure to help us construct and start the application.
pub struct AppBuilder {
    /// The actual platform-specific implementation of the app. This
    /// stores a strong reference to the application state and the
    /// callback functions to invoke when things occur at the platform
    /// level.
    inner: AppBackend,
    test_driver: Option<TestDriver>,
    custom_tag_to_keystroke_fn: Option<Box<dyn Fn(CustomTag) -> Option<Keystroke> + 'static>>,
    default_keystroke_trigger_for_custom_actions:
        Option<Box<dyn Fn(CustomTag) -> Option<Keystroke> + 'static>>,
}

impl AppBuilder {
    /// Constructs a new application using the current platform backend.
    pub fn new(
        callbacks: AppCallbacks,
        assets: Box<dyn AssetProvider>,
        test_driver: Option<TestDriver>,
    ) -> Self {
        let inner = super::current::App::new(callbacks, assets, test_driver.as_ref());

        Self {
            inner: AppBackend::CurrentPlatform(Box::new(inner)),
            test_driver,
            custom_tag_to_keystroke_fn: None,
            default_keystroke_trigger_for_custom_actions: None,
        }
    }

    /// Constructs a new application using the headless backend.
    pub fn new_headless(
        callbacks: AppCallbacks,
        assets: Box<dyn AssetProvider>,
        test_driver: Option<TestDriver>,
    ) -> Self {
        let inner = super::headless::App::new(callbacks, assets, test_driver.as_ref());
        Self {
            inner: AppBackend::Headless(Box::new(inner)),
            test_driver,
            custom_tag_to_keystroke_fn: None,
            default_keystroke_trigger_for_custom_actions: None,
        }
    }

    /// Constructs a new application using the terminal-UI (TUI) backend.
    #[cfg(feature = "tui")]
    pub fn new_tui(
        callbacks: AppCallbacks,
        assets: Box<dyn AssetProvider>,
        test_driver: Option<TestDriver>,
    ) -> Self {
        let inner = super::tui::App::new(callbacks, assets, test_driver.as_ref());
        Self {
            inner: AppBackend::Tui(Box::new(inner)),
            test_driver,
            custom_tag_to_keystroke_fn: None,
            default_keystroke_trigger_for_custom_actions: None,
        }
    }

    /// Converts any [`crate::keymap::Trigger::Custom`]-based binding to a traditional
    /// [`Keystroke`]-based binding using the provided `custom_tag_to_keystroke` function.
    ///
    /// This can be useful in the cases where an application registers a binding with a
    /// [`crate::keymap::Trigger::Custom`] for use in a Mac menu, but still wants to register the
    /// binding with its corresponding `Keystroke` on other platforms that don't support menus.
    pub fn convert_custom_triggers_to_keystroke_triggers(
        &mut self,
        custom_tag_to_keystroke: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.custom_tag_to_keystroke_fn = Some(Box::new(custom_tag_to_keystroke));
    }

    /// Registers a lookup function that returns the default keystroke for a given custom action.
    /// Used when converting custom actions to key events during keybinding editing.
    pub fn register_default_keystroke_triggers_for_custom_actions(
        &mut self,
        custom_tag_to_keystroke: impl Fn(CustomTag) -> Option<Keystroke> + 'static,
    ) {
        self.default_keystroke_trigger_for_custom_actions = Some(Box::new(custom_tag_to_keystroke));
    }

    /// Runs the application, invoking the provided function to
    /// initialize application state and be ready to process
    /// events from the main event loop.
    pub fn run(mut self, init_fn: impl FnOnce(&mut AppContext) + 'static) -> TerminationResult {
        let custom_tag_to_keystroke = self.custom_tag_to_keystroke_fn.take();
        // Wrap the initialization fn with one that first tries to convert custom triggers to
        // keystroke triggers.
        let init_fn = |ctx: &mut AppContext| {
            if let Some(custom_tag_to_keystroke) = custom_tag_to_keystroke {
                ctx.convert_custom_triggers_to_keystroke_triggers(custom_tag_to_keystroke);
            }
            if let Some(default_keystroke_trigger_for_custom_actions) =
                self.default_keystroke_trigger_for_custom_actions
            {
                ctx.register_default_keystroke_triggers_for_custom_actions(
                    default_keystroke_trigger_for_custom_actions,
                );
            }
            init_fn(ctx);
        };

        if let Some(test_driver) = self.test_driver {
            self.inner.run(move |ctx, ui_app_future| {
                init_fn(ctx);

                ctx.foreground_executor()
                    .spawn(async move {
                        let ui_app = ui_app_future.await;
                        test_driver.run_test_and_cleanup(ui_app).await;
                    })
                    .detach();
            })
        } else {
            self.inner.run(|ctx, _| init_fn(ctx))
        }
    }
}

impl AsInnerMut<AppBackend> for AppBuilder {
    fn as_inner_mut(&mut self) -> &mut AppBackend {
        &mut self.inner
    }
}
