use std::any::Any;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::Arc;

use super::{ObjectType, ServerMetadata, ServerPermissions};
use crate::ids::SyncId;

#[derive(Clone, Debug, Default)]
pub enum ConflictStatus<T> {
    #[default]
    NoConflicts,
    ConflictingChanges {
        object: Arc<T>,
    },
}

impl<T> ConflictStatus<T> {
    /// Utility function that allows for a more ergonomic way of figuring out whether there is a
    /// conflict (for cases where we don't care about the conflict details).
    pub fn has_conflicts(&self) -> bool {
        matches!(self, ConflictStatus::ConflictingChanges { .. })
    }
}

/// Common behavior that server-backed models expose to generic server objects.
pub trait ServerObjectModel: Debug + Clone + Send + Sync + 'static {
    /// Returns the object type for this model.
    fn object_type(&self) -> ObjectType;
}

/// Common trait for server objects that allows us to use them as trait objects
/// and downcast to concrete types when needed.
pub trait ServerObject: Debug + Send + Sync {
    /// Returns the object type of this server object
    fn object_type(&self) -> ObjectType;

    /// Returns this object as a ref to the Any type.  Needed for typecasts.
    fn as_any(&self) -> &dyn Any;

    /// Returns the trait object as a concrete type reference by downcasting it.
    /// Returns None if the downcast fails.
    fn as_concrete_type<K, M>(
        server_object: &dyn ServerObject,
    ) -> Option<&GenericServerObject<K, M>>
    where
        Self: Sized,
        K: 'static,
        M: 'static,
    {
        server_object
            .as_any()
            .downcast_ref::<GenericServerObject<K, M>>()
    }

    /// Returns a cloned boxed version of this server object.
    /// Note that we can't force the ServerObject trait to derive from Cloned
    /// directly because that would make the trait not object safe.  This
    /// is a workaround.
    fn clone_box(&self) -> Box<dyn ServerObject>;
}

/// An object that maps directly to the data returned from the server
/// for a given model and id type.
pub struct GenericServerObject<K, M> {
    pub id: SyncId,
    pub model: M,
    pub metadata: ServerMetadata,
    pub permissions: ServerPermissions,
    _marker: PhantomData<fn() -> K>,
}

impl<K, M> Clone for GenericServerObject<K, M>
where
    M: Clone,
{
    fn clone(&self) -> Self {
        Self::new(
            self.id,
            self.model.clone(),
            self.metadata.clone(),
            self.permissions.clone(),
        )
    }
}

impl<K, M> Debug for GenericServerObject<K, M>
where
    M: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenericServerObject")
            .field("id", &self.id)
            .field("model", &self.model)
            .field("metadata", &self.metadata)
            .field("permissions", &self.permissions)
            .finish()
    }
}

impl<K, M> GenericServerObject<K, M> {
    /// Constructs a server object from its server-provided parts.
    pub fn new(
        id: SyncId,
        model: M,
        metadata: ServerMetadata,
        permissions: ServerPermissions,
    ) -> Self {
        Self {
            id,
            model,
            metadata,
            permissions,
            _marker: PhantomData,
        }
    }
}

impl<'a, K, M> From<&'a dyn ServerObject> for Option<&'a GenericServerObject<K, M>>
where
    K: 'static,
    M: 'static,
{
    fn from(value: &'a dyn ServerObject) -> Self {
        value.as_any().downcast_ref::<GenericServerObject<K, M>>()
    }
}

impl<'a, K, M> From<&'a Box<dyn ServerObject>> for Option<&'a GenericServerObject<K, M>>
where
    K: 'static,
    M: 'static,
{
    fn from(value: &'a Box<dyn ServerObject>) -> Self {
        value.as_ref().into()
    }
}

impl<K, M> ServerObject for GenericServerObject<K, M>
where
    K: 'static,
    M: ServerObjectModel,
{
    fn object_type(&self) -> ObjectType {
        self.model.object_type()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ServerObject> {
        Box::new(self.clone())
    }
}
