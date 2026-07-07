use std::fmt::{self, Debug};
use std::ops::{Deref, DerefMut};

/// Wrapper type for values which may contain user input.
///
/// Use this to prevent logging user input in production builds. In local development builds, the
/// value will still be shown.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct UserInput<T>(T);

impl<T> UserInput<T> {
    pub fn new<U: Into<T>>(value: U) -> Self {
        Self(value.into())
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for UserInput<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for UserInput<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Debug> Debug for UserInput<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if cfg!(debug_assertions) {
            f.debug_tuple("UserInput").field(&self.0).finish()
        } else {
            f.debug_struct("UserInput").finish_non_exhaustive()
        }
    }
}
