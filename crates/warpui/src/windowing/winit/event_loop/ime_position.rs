//! Explicit IME candidate-window position tracking for winit.
//!
//! Winit caches the last IME cursor area by value. When the logical cursor
//! position is unchanged but the window geometry changes (move, resize, or
//! scale-factor change), a single `set_ime_cursor_area` call with the same
//! values is a no-op. To force a refresh we briefly set a one-pixel-offset
//! position and then the real position.
//!
//! The IME size argument is calculated even on X11, where the platform does
//! not support it; callers should still pass it through so non-X11 backends
//! can use it.

use winit::dpi::{LogicalPosition, LogicalSize};

use crate::CursorInfo;

/// Why the IME candidate window should be refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImePositionRefreshReason {
    /// The text-editing caret moved or the focused field changed.
    CursorMoved,
    /// The window's outer position changed.
    WindowMoved,
    /// The window's inner size changed.
    WindowResized,
    /// The display scale factor changed.
    ScaleFactorChanged,
}

/// Logical IME cursor area passed to the platform window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ImeCursorArea {
    pub position: LogicalPosition<f32>,
    pub size: LogicalSize<f32>,
}

impl ImeCursorArea {
    /// Builds the candidate-window area from active cursor info.
    ///
    /// The vertical origin is placed slightly below the caret using the font
    /// size so the candidate window does not cover the current line. Size is
    /// always derived from `font_size` even though X11 ignores it.
    pub fn from_cursor_info(cursor: &CursorInfo) -> Self {
        Self {
            position: LogicalPosition::new(
                cursor.position.origin_x(),
                cursor.position.origin_y() + (1.2 * cursor.font_size),
            ),
            // Currently the size argument is not supported on X11. We calculate it here anyway.
            size: LogicalSize::new(cursor.font_size, cursor.font_size),
        }
    }

    /// One-pixel vertical offset used to bust winit's position cache.
    fn cache_bust_offset(self) -> Self {
        Self {
            position: LogicalPosition::new(self.position.x, self.position.y + 1.),
            size: self.size,
        }
    }
}

/// A single platform update to apply for the IME candidate window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ImePositionUpdate {
    Set(ImeCursorArea),
}

/// Tracks IME enablement and the last area we told the platform about.
#[derive(Debug, Default)]
pub(super) struct ImePositionState {
    enabled: bool,
    last_area: Option<ImeCursorArea>,
}

impl ImePositionState {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.last_area = None;
        }
    }

    /// Returns the platform update sequence needed to place the IME candidate
    /// window at `area`, or an empty sequence when IME is disabled.
    ///
    /// The sequence always includes a one-pixel offset followed by the real
    /// area so winit cannot treat an identical re-application as a no-op after
    /// window geometry changes.
    pub fn plan_refresh(
        &mut self,
        area: ImeCursorArea,
        _reason: ImePositionRefreshReason,
    ) -> Vec<ImePositionUpdate> {
        if !self.enabled {
            return Vec::new();
        }

        let updates = vec![
            ImePositionUpdate::Set(area.cache_bust_offset()),
            ImePositionUpdate::Set(area),
        ];
        self.last_area = Some(area);
        updates
    }

    #[cfg(test)]
    pub fn last_area(&self) -> Option<ImeCursorArea> {
        self.last_area
    }
}

#[cfg(test)]
#[path = "ime_position_tests.rs"]
mod tests;
