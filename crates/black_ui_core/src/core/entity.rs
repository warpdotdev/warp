use core::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::ModelHandle;

/// A unique identifier for a View or a Model.
///
/// View and Model identifiers are not separately namespaced because we want to
/// use them interchangeably in several places, e.g. in observations.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(usize);

impl EntityId {
    /// Constructs a new globally-unique entity ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> EntityId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        EntityId(raw)
    }

    pub fn from_usize(value: usize) -> EntityId {
        EntityId(value)
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// An interface for a structure that can produce events.
///
/// TODO(vorporeal): This can probably be eliminated entirely, with View and
/// Model exposing the associated type Event independently.
pub trait Entity: 'static {
    type Event;
}

/// An interface for a structure holding global state for the application.
pub trait SingletonEntity: Entity + Sized {
    /// Returns the handle to the single model of this type stored within the
    /// provided application state.
    fn handle<T: GetSingletonModelHandle>(ctx: &T) -> ModelHandle<Self> {
        ctx.get_singleton_model_handle()
    }

    fn as_ref(ctx: &crate::AppContext) -> &Self {
        ctx.get_singleton_model_as_ref()
    }
}

/// A trait for retrieving a handle to a singleton model by type.
pub trait GetSingletonModelHandle {
    /// Returns the handle to the single model of this type stored within the
    /// provided application state.
    fn get_singleton_model_handle<T: SingletonEntity>(&self) -> ModelHandle<T>;
}

pub trait AddSingletonModel {
    fn add_singleton_model<T, F>(&mut self, build_model: F) -> ModelHandle<T>
    where
        T: SingletonEntity,
        F: FnOnce(&mut super::ModelContext<T>) -> T;
}
