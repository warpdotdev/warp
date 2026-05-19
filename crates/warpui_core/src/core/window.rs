use core::fmt;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{core::view::AnyViewHandle, AnyView, EntityId, ZoomFactor};

/// A unique identifier for a window.
///
/// These are globally unique and not reused across the lifetime of the
/// application.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WindowId(usize);

impl WindowId {
    /// Constructs a new globally-unique window ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> WindowId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        WindowId(raw)
    }

    pub fn from_usize(value: usize) -> WindowId {
        WindowId(value)
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// A structure holding all application state that is linked to a particular
/// window.
#[derive(Default)]
pub(super) struct Window {
    /// The set of views owned by this window, keyed by view ID.
    pub views: HashMap<EntityId, Box<dyn AnyView>>,

    /// A handle to the window's root view (top of the view hierarchy), if any.
    pub root_view: Option<AnyViewHandle>,

    /// The ID of the currently focused view, if any.
    pub focused_view: Option<EntityId>,

    /// Per-window zoom factor override. `None` means the window follows the
    /// app-wide default returned by [`AppContext::zoom_factor`].
    pub zoom_factor_override: Option<ZoomFactor>,
}
