//! The `Backend` marker trait and the bridges that let the shared application
//! core store and render backend-specific view objects without naming the
//! concrete view trait.
//!
//! This is the parameterization seam. [`AppContextImpl<B>`](super::AppContextImpl)
//! and [`Window<B>`](super::Window) are generic over `B: Backend`, and the
//! associated types below carry the pieces that differ between the GUI backend
//! (here, [`GuiBackend`]) and the TUI backend (added in a later milestone): the
//! type-erased per-window view object, a view's render output, and the
//! presentation layer.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{AnyView, AppContextImpl, InvalidationCallback};
use crate::presenter::PositionCache;
use crate::{Element, Presenter, View, WindowId, WindowInvalidation};

/// Marker trait selecting a UI backend (GUI or TUI).
///
/// Rust cannot express an "associated trait", so the divergent view trait
/// (`View` for GUI, `TuiView` for the TUI backend) is reached indirectly through
/// the type-erased [`AnyView`](Self::AnyView) object stored per window and the
/// [`RenderOutput`](Self::RenderOutput) it produces.
pub trait Backend: Sized + 'static {
    /// What a view of this backend renders to. GUI: `Box<dyn Element>`.
    type RenderOutput;

    /// The type-erased per-window view object, stored as `Box<Self::AnyView>` in
    /// [`Window<B>::views`](super::Window). GUI: `dyn AnyView`.
    ///
    /// Bounded by [`ErasedView<Self>`] so the shared core can render any stored
    /// view without naming the concrete view trait.
    type AnyView: ?Sized + ErasedView<Self>;

    /// The backend's presentation layer (lays out + paints a window's view tree)
    /// plus the bookkeeping that drives it. GUI: [`GuiPresenterState`].
    ///
    /// Hoisted here so the generic core stores `B::Presenter` and never names a
    /// backend-specific concrete presentation type; the GUI presenter API is
    /// reached only through methods on the `AppContext` alias whose signatures
    /// resolve through this associated type.
    type Presenter;
}

/// The object-safe surface the shared core needs from any stored view: render it
/// to the backend's output, and recover its concrete type during `update_view`.
///
/// For the GUI backend this is satisfied by the existing [`AnyView`] trait, which
/// additionally carries the focus/blur/keymap/a11y hooks.
pub trait ErasedView<B: Backend> {
    fn render(&self, app: &AppContextImpl<B>) -> B::RenderOutput;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Bridges a concrete view `T` into the backend's erased view object.
///
/// The single blanket impl per backend is where the `Box<T> -> Box<Self::AnyView>`
/// unsizing coercion happens, so the generic core can box any concrete view
/// without naming the view trait. A `T: View` bound on a GUI call site therefore
/// satisfies `T: BackendView<GuiBackend>` automatically.
pub trait BackendView<B: Backend>: 'static {
    fn into_any_view(self: Box<Self>) -> Box<B::AnyView>;
}

/// The GUI backend marker.
pub struct GuiBackend;

impl Backend for GuiBackend {
    type RenderOutput = Box<dyn Element>;
    type AnyView = dyn AnyView;
    type Presenter = GuiPresenterState;
}

impl ErasedView<GuiBackend> for dyn AnyView {
    fn render(&self, app: &AppContextImpl<GuiBackend>) -> Box<dyn Element> {
        AnyView::render(self, app)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        AnyView::as_any_mut(self)
    }
}

impl<T: View> BackendView<GuiBackend> for T {
    fn into_any_view(self: Box<Self>) -> Box<dyn AnyView> {
        self
    }
}

/// Presentation state for the GUI backend, stored on `AppContextImpl<GuiBackend>`
/// as `B::Presenter`. Wraps today's presenter collection plus the position cache
/// and window-invalidation bookkeeping, so the generic core holds only an opaque
/// `B::Presenter` while GUI method signatures that touch this state are unchanged.
#[derive(Default)]
pub struct GuiPresenterState {
    pub(crate) presenters: HashMap<WindowId, Rc<RefCell<Presenter>>>,
    pub(crate) last_frame_position_cache: HashMap<WindowId, PositionCache>,
    pub(crate) window_invalidations: HashMap<WindowId, WindowInvalidation>,
    pub(crate) invalidation_callbacks: HashMap<WindowId, Box<InvalidationCallback>>,
}
