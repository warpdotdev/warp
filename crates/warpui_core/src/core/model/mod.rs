pub mod context;
pub mod handle;

use std::any::Any;

pub use self::context::*;
pub use self::handle::*;
use crate::Entity;

pub trait AnyModel {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T> AnyModel for T
where
    T: Entity,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
