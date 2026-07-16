//! Explicit tracking and planning of the IME candidate-window position.
//!
//! winit caches the last IME cursor area passed to
//! [`winit::window::Window::set_ime_cursor_area`] and skips notifying the
//! platform when the area is unchanged. That caching is keyed on the
//! *window-relative* logical position, so after a window move, resize, or
//! scale-factor change the platform's notion of where the candidate window
//! should appear is stale even though the window-relative position is
//! unchanged.
//!
//! [`ImePositionState`] tracks the last candidate area we applied and plans a
//! deterministic sequence of updates for a new target area:
//!
//! * If the target differs from the last applied area, a single update
//!   suffices — winit will forward it to the platform.
//! * If the target equals the last applied area, winit would silently drop
//!   the update, so we first apply a "nudge" area offset by one logical pixel
//!   and then the real target, forcing winit to re-send the position.

use winit::dpi::{LogicalPosition, LogicalSize};

use crate::CursorInfo;

/// Vertical offset factor: the candidate window is placed this many
/// font-size-multiples below the top of the text cursor, i.e. just below the
/// line being edited.
const CANDIDATE_WINDOW_VERTICAL_OFFSET_FACTOR: f32 = 1.2;

/// Offset (in logical pixels) used for the cache-busting nudge update.
const CACHE_BUST_OFFSET: f32 = 1.0;

/// The area (position and size) of the IME candidate window, in logical
/// pixels relative to the window's client area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ImeCandidateArea {
    pub position: LogicalPosition<f32>,
    pub size: LogicalSize<f32>,
}

impl ImeCandidateArea {
    /// Computes the candidate-window area for the given text-cursor info: the
    /// candidate window is anchored just below the cursor, and sized to the
    /// font size.
    ///
    /// Note: the size is not supported on X11 — winit ignores it there — but
    /// we calculate it anyway for the platforms that do support it.
    pub fn from_cursor_info(cursor_info: &CursorInfo) -> Self {
        Self {
            position: LogicalPosition::new(
                cursor_info.position.origin_x(),
                cursor_info.position.origin_y()
                    + (CANDIDATE_WINDOW_VERTICAL_OFFSET_FACTOR * cursor_info.font_size),
            ),
            size: LogicalSize::new(cursor_info.font_size, cursor_info.font_size),
        }
    }

    /// Returns a copy of this area offset by one logical pixel, used to bust
    /// winit's cached IME cursor area (see the module docs).
    fn nudged(&self) -> Self {
        Self {
            position: LogicalPosition::new(self.position.x, self.position.y + CACHE_BUST_OFFSET),
            size: self.size,
        }
    }
}

/// Per-window state tracking the last IME candidate area applied to winit.
#[derive(Debug, Default)]
pub(super) struct ImePositionState {
    /// The last candidate area passed to winit, i.e. the value winit has
    /// cached. `None` until the first update.
    last_applied: Option<ImeCandidateArea>,
}

impl ImePositionState {
    /// Plans the sequence of candidate areas to pass to
    /// `set_ime_cursor_area`, in order, so that the platform ends up with
    /// `target` as the effective candidate-window area.
    ///
    /// The caller must apply *all* returned updates in order.
    pub fn plan_updates(&mut self, target: ImeCandidateArea) -> Vec<ImeCandidateArea> {
        let updates = if self.last_applied == Some(target) {
            // winit would drop an update identical to its cached value, but
            // the platform may still need a refresh (e.g. the window moved).
            // Nudge the position first to force winit to re-send it.
            vec![target.nudged(), target]
        } else {
            vec![target]
        };
        self.last_applied = Some(target);
        updates
    }
}

#[cfg(test)]
mod tests {
    use pathfinder_geometry::rect::RectF;
    use pathfinder_geometry::vector::vec2f;

    use super::*;

    fn cursor_info(x: f32, y: f32, font_size: f32) -> CursorInfo {
        CursorInfo {
            position: RectF::new(vec2f(x, y), vec2f(2.0, font_size)),
            font_size,
        }
    }

    fn area(x: f32, y: f32, font_size: f32) -> ImeCandidateArea {
        ImeCandidateArea {
            position: LogicalPosition::new(x, y),
            size: LogicalSize::new(font_size, font_size),
        }
    }

    #[test]
    fn candidate_area_is_anchored_below_the_cursor() {
        let target = ImeCandidateArea::from_cursor_info(&cursor_info(100.0, 50.0, 10.0));
        // 50 + 1.2 * 10 = 62.
        assert_eq!(target, area(100.0, 62.0, 10.0));
    }

    #[test]
    fn first_update_is_applied_directly() {
        let mut state = ImePositionState::default();
        let target = area(10.0, 20.0, 12.0);
        assert_eq!(state.plan_updates(target), vec![target]);
    }

    #[test]
    fn changed_target_is_applied_directly() {
        let mut state = ImePositionState::default();
        let first = area(10.0, 20.0, 12.0);
        let second = area(10.0, 35.0, 12.0);
        state.plan_updates(first);
        assert_eq!(state.plan_updates(second), vec![second]);
    }

    #[test]
    fn unchanged_target_is_nudged_to_bust_winit_cache() {
        let mut state = ImePositionState::default();
        let target = area(10.0, 20.0, 12.0);
        state.plan_updates(target);

        // E.g. the window moved: the window-relative target is unchanged, but
        // winit must be forced to re-send the position to the platform.
        let updates = state.plan_updates(target);
        assert_eq!(updates, vec![area(10.0, 21.0, 12.0), target]);
        // The nudge differs from the cached value, and the final update
        // differs from the nudge, so winit forwards both.
        assert_ne!(updates[0], target);
    }

    #[test]
    fn repeated_refreshes_generate_the_same_sequence() {
        let mut state = ImePositionState::default();
        let target = area(1.0, 2.0, 14.0);
        state.plan_updates(target);
        let first_refresh = state.plan_updates(target);
        let second_refresh = state.plan_updates(target);
        assert_eq!(first_refresh, second_refresh);
        assert_eq!(second_refresh, vec![target.nudged(), target]);
    }
}
