use std::marker::PhantomData;

use super::ErrorExt;

#[macro_export]
macro_rules! register_error {
    ($error:ty) => {
        impl $crate::errors::RegisteredError for $error {}

        $crate::errors::submit! {
            $crate::errors::ErrorRegistration::<$error>::adapt()
        }
    };
}
pub use register_error;

/// Marker trait for known error events. We rely on this to implement [`ErrorExt`] for [`anyhow::Error`]
/// in a way that delegates to errors in the context chain.
///
/// DO NOT implement this trait directly - use the [`register_error!`] macro instead.
pub trait RegisteredError {}

/// A type-erased version of [`ErrorRegistration`]. This is only used by the
/// [`register_error!`] macro implementation.
#[doc(hidden)]
pub trait AnyErrorRegistration: Sync {
    // Returns true if
    fn downcast_and_is_actionable(&self, error: &(dyn std::error::Error + 'static))
        -> Option<bool>;
}

/// Adapter for statically registering all [`ErrorExt`] implementations.
#[doc(hidden)]
pub struct ErrorRegistration<T: ErrorExt + 'static> {
    /// Marker that `ErrorRegistration` references `T`, but doesn't own a `T` value.
    /// See https://doc.rust-lang.org/nomicon/phantom-data.html
    _marker: PhantomData<fn(T) -> T>,
}

impl<T: ErrorExt + 'static> ErrorRegistration<T> {
    pub const fn adapt() -> &'static dyn AnyErrorRegistration {
        &Self {
            _marker: PhantomData,
        }
    }
}

impl<T: ErrorExt + 'static> AnyErrorRegistration for ErrorRegistration<T> {
    fn downcast_and_is_actionable(
        &self,
        error: &(dyn std::error::Error + 'static),
    ) -> Option<bool> {
        let err = error.downcast_ref::<T>()?;
        Some(err.is_actionable())
    }
}

// Collect adapters for all registered error types. Because `inventory::collect!` requires a
// concrete type, we use `&static dyn Trait` to erase the generics.
inventory::collect!(&'static dyn AnyErrorRegistration);
