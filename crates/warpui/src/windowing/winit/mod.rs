pub(crate) mod app;
pub mod delegate;
mod event_loop;
pub(crate) mod fonts;
#[cfg(target_os = "linux")]
pub mod linux;

mod notifications;

mod window;

#[cfg(target_os = "windows")]
pub mod windows;

use app::CustomEvent;
#[cfg(target_os = "linux")]
pub use app::WindowingSystem;
use event_loop::EventLoop;
#[cfg(target_os = "linux")]
pub use window::get_os_window_manager_name;
use window::Window;
