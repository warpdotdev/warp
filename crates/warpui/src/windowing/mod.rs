#[cfg(winit)]
pub mod winit;

pub use warpui_core::windowing::*;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub use winit::WindowingSystem;

/// The minimum width a window can be resized to.
/// TODO(CORE-1891) Instead of being hard-coded, this should be configurable by the user via
/// [`crate::platform::WindowOptions`].
#[cfg(any(test, feature = "integration_tests"))]
pub const MIN_WINDOW_WIDTH: f32 = 124.;
#[cfg(not(any(test, feature = "integration_tests")))]
pub const MIN_WINDOW_WIDTH: f32 = 480.;

/// The minimum height a window can be resized to.
#[cfg(any(test, feature = "integration_tests"))]
pub const MIN_WINDOW_HEIGHT: f32 = 34.;
#[cfg(not(any(test, feature = "integration_tests")))]
pub const MIN_WINDOW_HEIGHT: f32 = 192.;
