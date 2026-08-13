/// Returns the backing scale factor of the main display.
///
/// This is used to convert between pixel coordinates (as returned by screenshot tools)
/// and point coordinates (as used by CGEvent and screencapture).
///
/// This intentionally avoids `NSScreen::mainScreen`, which must run on the main thread and is
/// reached via a synchronous dispatch to the main queue. In the headless `agent run` CLI the
/// main thread never services that queue, so such a dispatch deadlocks. The backing scale factor
/// is instead derived purely from thread-safe Core Graphics calls as the ratio of the main
/// display's current mode pixel width to its point width.
pub fn main_display_scale_factor() -> f64 {
    use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode, CGMainDisplayID};

    let Some(mode) = CGDisplayCopyDisplayMode(CGMainDisplayID()) else {
        return 1.0;
    };
    let width_points = CGDisplayMode::width(Some(&mode));
    if width_points == 0 {
        return 1.0;
    }
    CGDisplayMode::pixel_width(Some(&mode)) as f64 / width_points as f64
}

/// Returns the main display's current-mode pixel dimensions as `(width, height)`.
///
/// Mirrors [`main_display_scale_factor`] by reading the main display's current
/// mode through thread-safe Core Graphics calls. Returns `(0, 0)` when the
/// display or its mode cannot be resolved (for example on a truly headless
/// host), which the recorder treats as an unsupported environment.
pub fn main_display_dimensions() -> (u32, u32) {
    use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode, CGMainDisplayID};

    let Some(mode) = CGDisplayCopyDisplayMode(CGMainDisplayID()) else {
        return (0, 0);
    };
    (
        CGDisplayMode::pixel_width(Some(&mode)) as u32,
        CGDisplayMode::pixel_height(Some(&mode)) as u32,
    )
}

/// Returns the backing scale factor of the display that fully contains a window.
///
/// A window spanning displays with different backing scale factors does not have one valid
/// target-wide pixel-to-point conversion and is therefore intentionally unsupported.
pub fn display_scale_factor_for_window(x: f64, y: f64, width: f64, height: f64) -> Option<f64> {
    use objc2_core_graphics::{
        CGDirectDisplayID, CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGError,
        CGGetActiveDisplayList,
    };

    const MAX_ACTIVE_DISPLAYS: u32 = 32;
    let mut displays: [CGDirectDisplayID; MAX_ACTIVE_DISPLAYS as usize] =
        [0; MAX_ACTIVE_DISPLAYS as usize];
    let mut display_count = 0;
    if unsafe {
        CGGetActiveDisplayList(
            MAX_ACTIVE_DISPLAYS,
            displays.as_mut_ptr(),
            &mut display_count,
        )
    } != CGError::Success
    {
        return None;
    }

    for id in displays.into_iter().take(display_count as usize) {
        let bounds = CGDisplayBounds(id);
        let contains_window = x >= bounds.origin.x
            && y >= bounds.origin.y
            && x + width <= bounds.origin.x + bounds.size.width
            && y + height <= bounds.origin.y + bounds.size.height;
        if !contains_window {
            continue;
        }

        let mode = CGDisplayCopyDisplayMode(id)?;
        let width_points = CGDisplayMode::width(Some(&mode));
        if width_points == 0 {
            return None;
        }
        return Some(CGDisplayMode::pixel_width(Some(&mode)) as f64 / width_points as f64);
    }

    None
}
