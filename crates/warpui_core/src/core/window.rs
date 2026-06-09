use core::fmt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::core::backend::Backend;
use crate::core::view::AnyViewHandle;
use crate::EntityId;

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
///
/// Generic over the active [`Backend`]: only `views` is backend-specific (it holds
/// the backend's type-erased view object). `root_view`/`focused_view` couple to
/// neither the view trait nor `B` and are shared across backends.
pub(super) struct Window<B: Backend> {
    /// The set of views owned by this window, keyed by view ID.
    pub views: HashMap<EntityId, Box<B::AnyView>>,

    /// A handle to the window's root view (top of the view hierarchy), if any.
    pub root_view: Option<AnyViewHandle>,

    /// The ID of the currently focused view, if any.
    pub focused_view: Option<EntityId>,
}

// Manual impl: `#[derive(Default)]` would wrongly require `B: Default`.
impl<B: Backend> Default for Window<B> {
    fn default() -> Self {
        Self {
            views: HashMap::new(),
            root_view: None,
            focused_view: None,
        }
    }
}
