use std::any::TypeId;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;

use super::context::ViewContext;
use crate::core::RefCounts;
use crate::{AppContext, Entity, EntityId, WindowId};

/// A strong reference to a particular [`View`] instance within the application.
///
/// Handles structures are used in place of references (e.g.: `&View`) to avoid
/// the complexity of reference lifetimes and appeasing the borrow checker.  A
/// handle can be combined with a reference to the application state (e.g.:
/// [`AppContext`]) to get access to the actual [`View`] instance behind the
/// handle.
pub struct ViewHandle<T> {
    window_id: WindowId,
    view_id: EntityId,
    view_type: PhantomData<T>,
    ref_counts: Weak<Mutex<RefCounts>>,
}

impl<T: Entity> ViewHandle<T> {
    pub(in crate::core) fn new(
        window_id: WindowId,
        view_id: EntityId,
        ref_counts: &Arc<Mutex<RefCounts>>,
    ) -> Self {
        ref_counts.lock().inc_entity(view_id);
        Self {
            window_id,
            view_id,
            view_type: PhantomData,
            ref_counts: Arc::downgrade(ref_counts),
        }
    }

    pub fn downgrade(&self) -> WeakViewHandle<T> {
        WeakViewHandle::new(self.view_id)
    }

    /// Convert a ViewHandle to a reference of the underlying View.
    pub fn as_ref<'a, A: ViewAsRef>(&self, app: &'a A) -> &'a T {
        app.view(self)
    }

    /// Try to convert a ViewHandle to a reference of the underlying View.
    /// Returns `None` if the view is currently borrowed (circular reference).
    pub fn try_as_ref<'a, A: ViewAsRef>(&self, app: &'a A) -> Option<&'a T> {
        app.try_view(self)
    }

    /// Reads a value out of the underlying View. This is especially useful when the view
    /// has a function that requires a `ViewContext` since `as_ref` does not create a `ViewHandle`
    /// to the view.
    pub fn read<A, F, S>(&self, app: &A, read: F) -> S
    where
        A: crate::ReadView,
        F: FnOnce(&T, &AppContext) -> S,
    {
        app.read_view(self, read)
    }

    /// Updates a value within the underlying View.
    pub fn update<A, F, S>(&self, app: &mut A, update: F) -> S
    where
        A: UpdateView,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S,
    {
        app.update_view(self, update)
    }

    pub fn is_focused(&self, app: &AppContext) -> bool {
        app.focused_view_id(self.window_id(app)) == Some(self.view_id)
    }

    // TODO: This is the same as the `is_self_or_child_focused` function in ViewContext.
    // Moving forward we should figure out a better interface to check whether a specific
    // view is focused or not.
    pub fn is_self_or_child_focused(&self, app: &mut AppContext) -> bool {
        let window_id = self.window_id(app);
        app.check_view_or_child_focused(window_id, &self.view_id)
    }
}

// These accessors don't depend on `T`, so they live in an unbounded impl block:
// this keeps them callable from the read path, where handles are only known to
// be `T: 'static` rather than `T: Entity`.
impl<T> ViewHandle<T> {
    /// Returns the current window this view belongs to.
    ///
    /// This looks up the window from the view_to_window mapping, which may differ
    /// from the window where the view was originally created if the view has been
    /// transferred between windows.
    pub fn window_id(&self, app: &AppContext) -> WindowId {
        app.view_to_window
            .get(&self.view_id)
            .copied()
            .unwrap_or(self.window_id)
    }

    pub fn id(&self) -> EntityId {
        self.view_id
    }
}

impl<T> Clone for ViewHandle<T> {
    fn clone(&self) -> Self {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(self.view_id);
        }

        Self {
            window_id: self.window_id,
            view_id: self.view_id,
            view_type: PhantomData,
            ref_counts: self.ref_counts.clone(),
        }
    }
}

impl<T> PartialEq for ViewHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.window_id == other.window_id && self.view_id == other.view_id
    }
}

impl<T> Eq for ViewHandle<T> {}

impl<T> Debug for ViewHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(&format!("ViewHandle<{}>", core::any::type_name::<T>()))
            .field("window_id", &self.window_id)
            .field("view_id", &self.view_id)
            .finish()
    }
}

impl<T> Drop for ViewHandle<T> {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_view(self.window_id, self.view_id);
        }
    }
}

unsafe impl<T> Send for ViewHandle<T> {}
unsafe impl<T> Sync for ViewHandle<T> {}

/// A type-erased strong reference to a particular [`View`] instance within the
/// application.
///
/// `AnyViewHandle` is used within the core UI framework in places where we need
/// to hold a strong reference to a `View`, but don't want to add a generic type
/// parameter to the containing structure.  See `root_view` in
/// [`Window`](crate::core::Window) for an example.
pub(in crate::core) struct AnyViewHandle {
    window_id: WindowId,
    view_id: EntityId,
    view_type: TypeId,
    ref_counts: Weak<Mutex<RefCounts>>,
}

impl AnyViewHandle {
    pub fn id(&self) -> EntityId {
        self.view_id
    }

    /// Returns the current window this view belongs to.
    pub fn window_id(&self, app: &AppContext) -> WindowId {
        app.view_to_window
            .get(&self.view_id)
            .copied()
            .unwrap_or(self.window_id)
    }

    pub fn is<T: 'static>(&self) -> bool {
        TypeId::of::<T>() == self.view_type
    }

    pub fn downcast<T: Entity>(self) -> Option<ViewHandle<T>> {
        if self.is::<T>()
            && let Some(ref_counts) = self.ref_counts.upgrade()
        {
            return Some(ViewHandle::new(self.window_id, self.view_id, &ref_counts));
        }
        None
    }
}

impl Clone for AnyViewHandle {
    fn clone(&self) -> Self {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(self.view_id);
        }

        Self {
            view_id: self.view_id,
            window_id: self.window_id,
            view_type: self.view_type,
            ref_counts: self.ref_counts.clone(),
        }
    }
}

impl<T: Entity> From<&ViewHandle<T>> for AnyViewHandle {
    fn from(handle: &ViewHandle<T>) -> Self {
        if let Some(ref_counts) = handle.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(handle.view_id);
        }
        AnyViewHandle {
            window_id: handle.window_id,
            view_id: handle.view_id,
            view_type: TypeId::of::<T>(),
            ref_counts: handle.ref_counts.clone(),
        }
    }
}

impl<T: Entity> From<ViewHandle<T>> for AnyViewHandle {
    fn from(handle: ViewHandle<T>) -> Self {
        (&handle).into()
    }
}

impl Drop for AnyViewHandle {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_view(self.window_id, self.view_id);
        }
    }
}

/// A weak reference to a particular [`View`] instance within the application.
///
/// `WeakViewHandle` is useful when a view wants to hold onto its own handle -
/// holding a strong reference via `ViewHandle` would create a reference cycle
/// that prevents the application from ever dropping the view.
pub struct WeakViewHandle<T> {
    view_id: EntityId,
    view_type: PhantomData<T>,
}

impl<T: Entity> WeakViewHandle<T> {
    pub(super) fn new(view_id: EntityId) -> Self {
        Self {
            view_id,
            view_type: PhantomData,
        }
    }

    pub fn upgrade(&self, app: &AppContext) -> Option<ViewHandle<T>> {
        // Look up the current window for this view
        let window_id = app.view_to_window.get(&self.view_id).copied()?;

        if app
            .windows
            .get(&window_id)
            .and_then(|w| w.views.get(&self.view_id))
            .is_some()
            && !app.ref_counts.lock().is_view_dropped(self.view_id)
        {
            Some(ViewHandle::new(window_id, self.view_id, &app.ref_counts))
        } else {
            None
        }
    }

    pub fn id(&self) -> EntityId {
        self.view_id
    }

    /// Returns the current window this view belongs to, if any.
    pub fn window_id(&self, app: &AppContext) -> Option<WindowId> {
        app.view_to_window.get(&self.view_id).copied()
    }
}

impl<T> Clone for WeakViewHandle<T> {
    fn clone(&self) -> Self {
        Self {
            view_id: self.view_id,
            view_type: PhantomData,
        }
    }
}

impl<T> Debug for WeakViewHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(&format!("WeakViewHandle<{}>", core::any::type_name::<T>()))
            .field("view_id", &self.view_id)
            .finish()
    }
}

unsafe impl<T> Send for WeakViewHandle<T> {}
unsafe impl<T> Sync for WeakViewHandle<T> {}

pub trait ViewAsRef {
    // Reading a view only needs `T: 'static` to downcast and hand out `&T`; the
    // view-ness is already carried by `ViewHandle<T>`, which is only mintable for
    // view types, so we don't need the stronger `Entity` bound here.
    fn view<T: 'static>(&self, handle: &ViewHandle<T>) -> &T;

    /// Try to get a reference to the view. Returns `None` if the view is
    /// currently borrowed (e.g., during a circular reference scenario).
    fn try_view<T: 'static>(&self, handle: &ViewHandle<T>) -> Option<&T>;
}

pub trait ReadView: ViewAsRef {
    // As with `ViewAsRef`, a read closure only ever sees `&T` and `&AppContext`,
    // so `T: 'static` is all that is required.
    fn read_view<T, F, S>(&self, handle: &ViewHandle<T>, read: F) -> S
    where
        T: 'static,
        F: FnOnce(&T, &AppContext) -> S;
}

pub trait UpdateView: ReadView {
    // `update_view` keeps the `Entity` bound because it hands the closure a
    // `&mut ViewContext<T>`, which exposes `emit(T::Event)` and therefore needs
    // `T: Entity`.
    fn update_view<T, F, S>(&mut self, handle: &ViewHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ViewContext<T>) -> S;
}
