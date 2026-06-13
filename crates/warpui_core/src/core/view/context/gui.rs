//! GUI-backend extensions to [`ViewContext`].

use pathfinder_geometry::rect::RectF;

use super::{View, ViewContext};

impl<'a, T: View> ViewContext<'a, T> {
    /// Presenter-backed and therefore GUI-only: the TUI backend has no
    /// layout-position cache in the core.
    pub fn element_position_by_id<S>(&self, id: S) -> Option<RectF>
    where
        S: AsRef<str>,
    {
        let presenter = self.app.presenter(self.window_id);

        if let Some(presenter) = presenter {
            let borrowed_presenter = presenter.borrow();
            borrowed_presenter.position_cache().get_position(id)
        } else {
            None
        }
    }
}
