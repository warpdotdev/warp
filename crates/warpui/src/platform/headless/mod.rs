//! A headless implementation of the UI framework's platform abstraction.
//!
//! This provides enough functionality to run an app, but no GUI or visible output.

mod app;
mod delegate;
mod event_loop;
mod windowing;

pub use app::App;
pub use delegate::AppDelegate;
#[cfg(target_os = "macos")]
pub(crate) use windowing::Window;
