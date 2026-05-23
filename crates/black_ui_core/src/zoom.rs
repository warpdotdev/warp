//! Module containing definitions related to UI magnification via a [`ZoomFactor`].

use pathfinder_geometry::vector::Vector2F;

/// The zoom factor of the application. All UI elements are magnified by this value.
#[derive(Copy, Clone)]
pub struct ZoomFactor(f32);

impl ZoomFactor {
    pub fn new(zoom_level: f32) -> Self {
        Self(zoom_level)
    }
}

impl Default for ZoomFactor {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Helper trait that scales a value by the given [`ZoomFactor`].
pub trait Scale: Sized {
    /// Scales the current value up by the current [`ZoomFactor`].
    fn scale_up(self, zoom_level: ZoomFactor) -> Self;

    /// Scales the current value down by the current [`ZoomFactor`].
    fn scale_down(self, zoom_level: ZoomFactor) -> Self {
        self.scale_up(ZoomFactor::new(1.0 / zoom_level.0))
    }
}

impl Scale for f32 {
    fn scale_up(self, zoom_level: ZoomFactor) -> Self {
        self * zoom_level.0
    }
}

impl Scale for Vector2F {
    fn scale_up(self, zoom_level: ZoomFactor) -> Self {
        Vector2F::new(self.x() * zoom_level.0, self.y() * zoom_level.0)
    }
}
