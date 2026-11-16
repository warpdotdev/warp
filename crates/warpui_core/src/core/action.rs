use std::any::{Any, TypeId};
use std::fmt::Debug;

/// Trait representing a Typed action.
///
/// We require that an action implement a number of parent traits:
///
/// - `Any` to support downcasting to the absolute type
/// - `Debug` so that we can show log messages about the action as it is dispatched
pub trait Action: Any + Debug + Send + Sync {
    /// Convert this `Action` into a `dyn Any` reference, necessary for passing to the handler
    /// function, as trait upcasting isn't yet stable, so we can't treat a value of `&dyn Action`
    /// as a value of `&dyn Any` directly.
    fn as_any(&self) -> &dyn Any;

    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Blanket impl for `Action`, allowing any type that implements the parent traits to
/// automatically be treated as an `Action`, without the app needing to do anything special
impl<T> Action for T
where
    T: Any + Debug + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(PartialEq, Eq, Hash)]
pub(super) struct ActionType(TypeId);

impl ActionType {
    pub fn of<T: ?Sized + 'static>() -> Self {
        ActionType(TypeId::of::<T>())
    }
}

impl From<&dyn Action> for ActionType {
    fn from(action: &dyn Action) -> Self {
        ActionType(action.type_id())
    }
}
