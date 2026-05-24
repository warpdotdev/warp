pub(crate) mod app;
pub mod delegate;
mod event_loop;
pub(crate) mod fonts;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod linux;

mod notifications;
#[cfg(target_family = "wasm")]
pub mod wasm;

mod window;

#[cfg(target_os = "windows")]
pub mod windows;

use app::CustomEvent;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub use app::WindowingSystem;
use event_loop::EventLoop;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub use window::get_os_window_manager_name;
use window::Window;
