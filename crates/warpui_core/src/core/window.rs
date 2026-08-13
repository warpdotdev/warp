use core::fmt;
use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

#[cfg(feature = "tui")]
use crate::core::view::AnyTuiView;
use crate::core::view::AnyViewHandle;
use crate::core::{AnyView, BlurContext, FocusContext};
use crate::{AccessibilityData, AppContext, CursorInfo, EntityId, EntityIdMap, keymap};

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
    pub views: EntityIdMap<StoredView>,

    /// A handle to the window's root view (top of the view hierarchy), if any.
    pub root_view: Option<AnyViewHandle>,

    /// The ID of the currently focused view, if any.
    pub focused_view: Option<EntityId>,
}

/// A type-erased view stored in a window's view registry.
///
/// GUI views ([`AnyView`]) and TUI views ([`AnyTuiView`], additive behind the
/// `tui` feature) share the same registry, keyed by the same [`EntityId`]s, so
/// all of the neutral machinery (handles + ref counts, focus, the responder
/// chain, typed actions, subscriptions/observations, drop/transfer) flows
/// through the same paths for both. The inherent methods below delegate the
/// neutral hook subset; render and the GUI-only hooks are matched exhaustively
/// at their call sites.
pub(crate) enum StoredView {
    Gui(Box<dyn AnyView>),
    #[cfg(feature = "tui")]
    Tui(Box<dyn AnyTuiView>),
}

impl StoredView {
    pub fn as_any(&self) -> &dyn Any {
        match self {
            StoredView::Gui(view) => view.as_any(),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.as_any(),
        }
    }

    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        match self {
            StoredView::Gui(view) => view.as_any_mut(),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.as_any_mut(),
        }
    }

    pub fn ui_name(&self) -> &'static str {
        match self {
            StoredView::Gui(view) => view.ui_name(),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.ui_name(),
        }
    }

    pub fn on_focus(
        &mut self,
        focus_ctx: &FocusContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        match self {
            StoredView::Gui(view) => view.on_focus(focus_ctx, app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.on_focus(focus_ctx, app, window_id, view_id),
        }
    }

    pub fn on_blur(
        &mut self,
        blur_ctx: &BlurContext,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        match self {
            StoredView::Gui(view) => view.on_blur(blur_ctx, app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.on_blur(blur_ctx, app, window_id, view_id),
        }
    }

    pub fn keymap_context(&self, app: &AppContext) -> keymap::Context {
        match self {
            StoredView::Gui(view) => view.keymap_context(app),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.keymap_context(app),
        }
    }

    pub fn active_cursor_position(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<CursorInfo> {
        match self {
            StoredView::Gui(view) => view.active_cursor_position(app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(_) => None,
        }
    }

    pub fn on_window_closed(
        &mut self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        match self {
            StoredView::Gui(view) => view.on_window_closed(app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(_) => {}
        }
    }

    pub fn on_window_transferred(
        &mut self,
        source_window_id: WindowId,
        target_window_id: WindowId,
        app: &mut AppContext,
        view_id: EntityId,
    ) {
        match self {
            StoredView::Gui(view) => {
                view.on_window_transferred(source_window_id, target_window_id, app, view_id)
            }
            #[cfg(feature = "tui")]
            StoredView::Tui(_) => {}
        }
    }

    pub fn self_or_child_interacted_with(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) {
        match self {
            StoredView::Gui(view) => view.self_or_child_interacted_with(app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(_) => {}
        }
    }

    pub fn accessibility_data(
        &self,
        app: &mut AppContext,
        window_id: WindowId,
        view_id: EntityId,
    ) -> Option<AccessibilityData> {
        match self {
            StoredView::Gui(view) => view.accessibility_data(app, window_id, view_id),
            #[cfg(feature = "tui")]
            StoredView::Tui(_) => None,
        }
    }

    pub fn child_view_ids(&self, app: &AppContext) -> Vec<EntityId> {
        match self {
            StoredView::Gui(view) => view.child_view_ids(app),
            #[cfg(feature = "tui")]
            StoredView::Tui(view) => view.child_view_ids(app),
        }
    }
}
