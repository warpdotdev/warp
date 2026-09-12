//! A small, platform-independent helper for the "pre-create the next server instance before
//! handing out the current one" pattern used by [`crate::native`]'s Windows named-pipe listener.
//!
//! Kept generic and separate from any Windows/tokio types so its recovery behavior -- the fix
//! for a bug where a single failed accept would leave the slot empty and panic on the next call
//! -- can be unit tested on every platform. It is only ever used on Windows.
#![allow(dead_code)]
use std::sync::Mutex;

/// Holds the next `T` to hand out, recreating it on demand via a caller-supplied factory if a
/// previous call left the slot empty (for example, because creating a replacement failed after a
/// prior [`take_or_create`](Self::take_or_create)).
///
/// This is deliberately *not* built to support concurrent callers: [`take_or_create`]/[`restore`]
/// pairs are expected to be used serially by a single caller (a single accept loop), which is the
/// only way `crate::native`'s Windows listener uses it.
///
/// [`take_or_create`]: Self::take_or_create
/// [`restore`]: Self::restore
pub(crate) struct NextInstance<T> {
    slot: Mutex<Option<T>>,
}

impl<T> NextInstance<T> {
    pub(crate) fn new(initial: T) -> Self {
        Self {
            slot: Mutex::new(Some(initial)),
        }
    }

    /// Takes the current instance if one is present; otherwise calls `create` to produce one.
    ///
    /// This is what makes the slot self-healing: if a previous `restore(None)` (see below) left
    /// the slot empty after a failed creation attempt, the *next* call recovers by trying to
    /// create an instance again, rather than assuming the slot is always populated.
    pub(crate) fn take_or_create(
        &self,
        create: impl FnOnce() -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let existing = self
            .slot
            .lock()
            .expect("NextInstance mutex poisoned")
            .take();
        match existing {
            Some(value) => Ok(value),
            None => create(),
        }
    }

    /// Stores `value` (which may be `None`, if the caller could not create a replacement) back
    /// into the slot for the next [`take_or_create`](Self::take_or_create) call.
    pub(crate) fn restore(&self, value: Option<T>) {
        *self.slot.lock().expect("NextInstance mutex poisoned") = value;
    }
}

#[cfg(test)]
#[path = "next_instance_tests.rs"]
mod tests;
