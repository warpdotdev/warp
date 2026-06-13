//! GUI-backend extensions to [`WindowManager`].

use super::WindowManager;
use crate::scene::{CornerRadius, Radius};

impl WindowManager {
    /// The window itself usually has rounded corners, except when running in a tiling window
    /// manager or when on Windows. We don't need to specify a custom window corner radius on
    /// Windows because we use OS APIs to round the corners of the window.
    pub fn window_corner_radius(&self) -> CornerRadius {
        let radius = if self.is_tiling_window_manager() || cfg!(windows) {
            0.
        } else {
            8.
        };
        CornerRadius::with_all(Radius::Pixels(radius))
    }
}
