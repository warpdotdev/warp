//! Presentation state for the GUI renderer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::presenter::PositionCache;
use crate::{rendering, Presenter, WindowId};

/// Presentation state for the GUI renderer, stored on
/// [`AppContext`](crate::AppContext). Holds the GUI presenter collection plus
/// the position cache.
///
/// The backend-neutral window-invalidation bookkeeping
/// (`window_invalidations` / `invalidation_callbacks`) lives directly on
/// [`AppContext`](crate::AppContext) so any TUI runtime shares it.
#[derive(Default)]
pub struct GuiPresenterState {
    pub(crate) presenters: HashMap<WindowId, Rc<RefCell<Presenter>>>,
    pub(crate) last_frame_position_cache: HashMap<WindowId, PositionCache>,
    /// Configuration options related to rendering of the application.
    pub(crate) rendering_config: rendering::Config,
}
