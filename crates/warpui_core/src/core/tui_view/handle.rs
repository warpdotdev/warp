use std::any::TypeId;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;

use super::context::TuiViewContext;
use super::{AnyTuiViewHandle, TuiView};
use crate::core::RefCounts;
use crate::{AppContext, EntityId, EntityLocation, Handle, WindowId};

pub struct TuiViewHandle<T> {
    window_id: WindowId,
    view_id: EntityId,
    view_type: PhantomData<T>,
    ref_counts: Weak<Mutex<RefCounts>>,
}

impl<T: TuiView> TuiViewHandle<T> {
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

    pub fn downgrade(&self) -> WeakTuiViewHandle<T> {
        WeakTuiViewHandle::new(self.view_id)
    }

    pub fn window_id(&self, app: &AppContext) -> WindowId {
        app.view_to_window
            .get(&self.view_id)
            .copied()
            .unwrap_or(self.window_id)
    }

    pub fn id(&self) -> EntityId {
        self.view_id
    }

    pub fn as_ref<'a, A: TuiViewAsRef>(&self, app: &'a A) -> &'a T {
        app.tui_view(self)
    }

    pub fn read<A, F, S>(&self, app: &A, read: F) -> S
    where
        A: ReadTuiView,
        F: FnOnce(&T, &AppContext) -> S,
    {
        app.read_tui_view(self, read)
    }

    pub fn update<A, F, S>(&self, app: &mut A, update: F) -> S
    where
        A: UpdateTuiView,
        F: FnOnce(&mut T, &mut TuiViewContext<T>) -> S,
    {
        app.update_tui_view(self, update)
    }
}

impl<T> Clone for TuiViewHandle<T> {
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

impl<T> PartialEq for TuiViewHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.window_id == other.window_id && self.view_id == other.view_id
    }
}

impl<T> Eq for TuiViewHandle<T> {}

impl<T> Debug for TuiViewHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(&format!("TuiViewHandle<{}>", core::any::type_name::<T>()))
            .field("window_id", &self.window_id)
            .field("view_id", &self.view_id)
            .finish()
    }
}

impl<T> Drop for TuiViewHandle<T> {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_view(self.window_id, self.view_id);
        }
    }
}

unsafe impl<T> Send for TuiViewHandle<T> {}
unsafe impl<T> Sync for TuiViewHandle<T> {}

impl<T> Handle<T> for TuiViewHandle<T> {
    fn id(&self) -> EntityId {
        self.view_id
    }

    fn location(&self) -> EntityLocation {
        EntityLocation::TuiView(self.window_id, self.view_id)
    }
}

impl AnyTuiViewHandle {
    pub fn id(&self) -> EntityId {
        self.view_id
    }

    pub fn is<T: 'static>(&self) -> bool {
        TypeId::of::<T>() == self.view_type
    }

    pub fn downcast<T: TuiView>(self) -> Option<TuiViewHandle<T>> {
        if self.is::<T>() {
            if let Some(ref_counts) = self.ref_counts.upgrade() {
                return Some(TuiViewHandle::new(
                    self.window_id,
                    self.view_id,
                    &ref_counts,
                ));
            }
        }
        None
    }
}

impl Clone for AnyTuiViewHandle {
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

impl<T: TuiView> From<&TuiViewHandle<T>> for AnyTuiViewHandle {
    fn from(handle: &TuiViewHandle<T>) -> Self {
        if let Some(ref_counts) = handle.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(handle.view_id);
        }
        AnyTuiViewHandle {
            window_id: handle.window_id,
            view_id: handle.view_id,
            view_type: TypeId::of::<T>(),
            ref_counts: handle.ref_counts.clone(),
        }
    }
}

impl<T: TuiView> From<TuiViewHandle<T>> for AnyTuiViewHandle {
    fn from(handle: TuiViewHandle<T>) -> Self {
        (&handle).into()
    }
}

impl Drop for AnyTuiViewHandle {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_view(self.window_id, self.view_id);
        }
    }
}

pub struct WeakTuiViewHandle<T> {
    view_id: EntityId,
    view_type: PhantomData<T>,
}

impl<T: TuiView> WeakTuiViewHandle<T> {
    pub(super) fn new(view_id: EntityId) -> Self {
        Self {
            view_id,
            view_type: PhantomData,
        }
    }

    pub fn upgrade(&self, app: &AppContext) -> Option<TuiViewHandle<T>> {
        let window_id = app.view_to_window.get(&self.view_id).copied()?;

        if app
            .tui_windows
            .get(&window_id)
            .and_then(|w| w.views.get(&self.view_id))
            .is_some()
        {
            Some(TuiViewHandle::new(window_id, self.view_id, &app.ref_counts))
        } else {
            None
        }
    }

    pub fn id(&self) -> EntityId {
        self.view_id
    }
}

impl<T> Clone for WeakTuiViewHandle<T> {
    fn clone(&self) -> Self {
        Self {
            view_id: self.view_id,
            view_type: PhantomData,
        }
    }
}

impl<T> Debug for WeakTuiViewHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(&format!(
            "WeakTuiViewHandle<{}>",
            core::any::type_name::<T>()
        ))
        .field("view_id", &self.view_id)
        .finish()
    }
}

unsafe impl<T> Send for WeakTuiViewHandle<T> {}
unsafe impl<T> Sync for WeakTuiViewHandle<T> {}

pub trait TuiViewAsRef {
    fn tui_view<T: TuiView>(&self, handle: &TuiViewHandle<T>) -> &T;
}

pub trait ReadTuiView: TuiViewAsRef {
    fn read_tui_view<T, F, S>(&self, handle: &TuiViewHandle<T>, read: F) -> S
    where
        T: TuiView,
        F: FnOnce(&T, &AppContext) -> S;
}

pub trait UpdateTuiView: ReadTuiView {
    fn update_tui_view<T, F, S>(&mut self, handle: &TuiViewHandle<T>, update: F) -> S
    where
        T: TuiView,
        F: FnOnce(&mut T, &mut TuiViewContext<T>) -> S;
}
