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

/// An axis-aligned rectangle expressed as `(x, y, width, height)`, independent of any
/// particular windowing backend's rect type.
pub type SimpleRect = (f64, f64, f64, f64);

/// Returns whether `rect` overlaps with at least one of `screens`.
///
/// Used to validate that a persisted/exact window position still lands on a currently
/// connected display before applying it verbatim. A rect that was saved while a screen was
/// connected can point to empty space once that screen is disconnected (see GH#15184), so
/// callers should fall back to a default placement when this returns `false`.
pub fn rect_intersects_any_screen(rect: SimpleRect, screens: &[SimpleRect]) -> bool {
    screens.iter().any(|screen| rects_overlap(rect, *screen))
}

fn rects_overlap(a: SimpleRect, b: SimpleRect) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    aw > 0.
        && ah > 0.
        && bw > 0.
        && bh > 0.
        && ax < bx + bw
        && ax + aw > bx
        && ay < by + bh
        && ay + ah > by
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
