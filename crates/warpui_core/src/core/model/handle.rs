use std::any::{type_name, TypeId};
use std::fmt::{self, Debug};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;

use crate::core::RefCounts;
use crate::{AppContext, Entity, EntityId, EntityLocation, Handle, ModelContext};

/// A strong reference to a particular [`Entity`] instance within the application.
///
/// Handles structures are used in place of references (e.g.: `&Entity`) to avoid
/// the complexity of reference lifetimes and appeasing the borrow checker.  A
/// handle can be combined with a reference to the application state (e.g.:
/// [`AppContext`]) to get access to the actual [`Entity`] instance behind the
/// handle.
pub struct ModelHandle<T> {
    model_id: EntityId,
    model_type: PhantomData<T>,
    ref_counts: Weak<Mutex<RefCounts>>,
}

impl<T: Entity> ModelHandle<T> {
    pub(in crate::core) fn new(model_id: EntityId, ref_counts: &Arc<Mutex<RefCounts>>) -> Self {
        ref_counts.lock().inc_entity(model_id);
        Self {
            model_id,
            model_type: PhantomData,
            ref_counts: Arc::downgrade(ref_counts),
        }
    }

    pub fn downgrade(&self) -> WeakModelHandle<T> {
        WeakModelHandle::new(self.model_id)
    }

    pub fn id(&self) -> EntityId {
        self.model_id
    }

    pub fn as_ref<'a, A: ModelAsRef>(&self, app: &'a A) -> &'a T {
        app.model(self)
    }

    pub fn read<A, F, S>(&self, app: &A, read: F) -> S
    where
        A: ReadModel,
        F: FnOnce(&T, &AppContext) -> S,
    {
        app.read_model(self, read)
    }

    pub fn update<A, F, S>(&self, app: &mut A, update: F) -> S
    where
        A: UpdateModel,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S,
    {
        app.update_model(self, update)
    }
}

impl<T> Clone for ModelHandle<T> {
    fn clone(&self) -> Self {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(self.model_id);
        }

        Self {
            model_id: self.model_id,
            model_type: PhantomData,
            ref_counts: self.ref_counts.clone(),
        }
    }
}

impl<T> PartialEq for ModelHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.model_id == other.model_id
    }
}

impl<T> Eq for ModelHandle<T> {}

impl<T> Hash for ModelHandle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.model_id.hash(state);
    }
}

impl<T> std::borrow::Borrow<EntityId> for ModelHandle<T> {
    fn borrow(&self) -> &EntityId {
        &self.model_id
    }
}

impl<T> Debug for ModelHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple(&format!("ModelHandle<{}>", type_name::<T>()))
            .field(&self.model_id)
            .finish()
    }
}

unsafe impl<T> Send for ModelHandle<T> {}
unsafe impl<T> Sync for ModelHandle<T> {}

impl<T> Drop for ModelHandle<T> {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_model(self.model_id);
        }
    }
}

impl<T> Handle<T> for ModelHandle<T> {
    fn id(&self) -> EntityId {
        self.model_id
    }

    fn location(&self) -> EntityLocation {
        EntityLocation::Model(self.model_id)
    }
}

/// A type-erased strong reference to a particular [`Entity`] instance within the
/// application.
///
/// `AnyModelHandle` is used within the core UI framework in places where we need
/// to hold a strong reference to a `Entity`, but don't want to add a generic type
/// parameter to the containing structure.  See `singleton_models` in
/// [`AppContext`](crate::core::AppContext) for an example.
pub struct AnyModelHandle {
    model_id: EntityId,
    model_type: TypeId,
    ref_counts: Weak<Mutex<RefCounts>>,
}

impl AnyModelHandle {
    pub fn id(&self) -> EntityId {
        self.model_id
    }

    pub fn is<T: 'static>(&self) -> bool {
        TypeId::of::<T>() == self.model_type
    }

    pub fn downcast<T: Entity>(self) -> Option<ModelHandle<T>> {
        if self.is::<T>() {
            if let Some(ref_counts) = self.ref_counts.upgrade() {
                return Some(ModelHandle::new(self.model_id, &ref_counts));
            }
        }
        None
    }

    pub fn downcast_ref<'a, T: Entity>(&'a self, ctx: &'a AppContext) -> Option<&'a T> {
        if self.is::<T>() {
            return ctx.models.get(&self.model_id)?.as_any().downcast_ref();
        }
        None
    }
}

impl Clone for AnyModelHandle {
    fn clone(&self) -> Self {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(self.model_id);
        }

        Self {
            model_id: self.model_id,
            model_type: self.model_type,
            ref_counts: self.ref_counts.clone(),
        }
    }
}

impl<T: Entity> From<ModelHandle<T>> for AnyModelHandle {
    fn from(handle: ModelHandle<T>) -> Self {
        if let Some(ref_counts) = handle.ref_counts.upgrade() {
            ref_counts.lock().inc_entity(handle.model_id);
        }

        Self {
            model_id: handle.model_id,
            model_type: TypeId::of::<T>(),
            ref_counts: handle.ref_counts.clone(),
        }
    }
}

impl Drop for AnyModelHandle {
    fn drop(&mut self) {
        if let Some(ref_counts) = self.ref_counts.upgrade() {
            ref_counts.lock().dec_model(self.model_id);
        }
    }
}

/// A weak reference to a particular [`Entity`] instance within the application.
///
/// `WeakModelHandle` is useful when a view wants to hold onto its own handle -
/// holding a strong reference via [`ModelHandle`] would create a reference
/// cycle that prevents the application from ever dropping the model.
pub struct WeakModelHandle<T> {
    model_id: EntityId,
    model_type: PhantomData<T>,
}

impl<T: Entity> WeakModelHandle<T> {
    pub(super) fn new(model_id: EntityId) -> Self {
        Self {
            model_id,
            model_type: PhantomData,
        }
    }

    pub fn upgrade(&self, app: &AppContext) -> Option<ModelHandle<T>> {
        if app.models.contains_key(&self.model_id) {
            Some(ModelHandle::new(self.model_id, &app.ref_counts))
        } else {
            None
        }
    }
}

impl<T> Clone for WeakModelHandle<T> {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id,
            model_type: PhantomData,
        }
    }
}

pub trait ModelAsRef {
    fn model<T: Entity>(&self, handle: &ModelHandle<T>) -> &T;
}

pub trait ReadModel: ModelAsRef {
    fn read_model<T, F, S>(&self, handle: &ModelHandle<T>, read: F) -> S
    where
        T: Entity,
        F: FnOnce(&T, &AppContext) -> S;
}

pub trait UpdateModel: ReadModel {
    fn update_model<T, F, S>(&mut self, handle: &ModelHandle<T>, update: F) -> S
    where
        T: Entity,
        F: FnOnce(&mut T, &mut ModelContext<T>) -> S;
}
