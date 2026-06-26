/// Returns the backing scale factor of the main display.
///
/// This is used to convert between pixel coordinates (as returned by screenshot tools)
/// and point coordinates (as used by CGEvent and screencapture).
pub fn main_display_scale_factor() -> f64 {
    use dispatch2::run_on_main;
    use objc2_app_kit::NSScreen;

    run_on_main(|mtm| {
        NSScreen::mainScreen(mtm)
            .map(|screen| screen.backingScaleFactor())
            .unwrap_or(1.0)
    })
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
