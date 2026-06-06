use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{track_read, track_update};

/// An autotracked value type
///
/// This implements `Deref` and `DerefMut` for the underlying type `T`, so in most cases can be
/// used as the underlying type without any code changes.
///
/// When the underlying data is read or updated, the Autotracking system will be notified so that
/// it can manage Views' dependencies on `Tracked` values and automatically create invalidations
/// (effectively calls to `ctx.notify()`) for the appropriate Views.
///
/// Note: Since the autotracking system only works on the main thread, `Tracked` does not implement
/// `Send` or `Sync` and so cannot be shared between threads.
#[derive(Debug)]
pub struct Tracked<T> {
    id: TrackedId,
    inner: T,
    _no_send: PhantomData<*const u8>,
}

impl<T> Tracked<T> {
    pub fn new(value: T) -> Self {
        Tracked {
            id: TrackedId::next(),
            inner: value,
            _no_send: PhantomData,
        }
    }
}

impl<T> From<T> for Tracked<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        track_read(self.id);
        &self.inner
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        track_update(self.id);
        &mut self.inner
    }
}

impl<T> Default for Tracked<T>
where
    T: Default,
{
    fn default() -> Self {
        Tracked {
            id: TrackedId::next(),
            inner: T::default(),
            _no_send: PhantomData,
        }
    }
}

/// Autoincrementing identifier used to track a given `Tracked` value
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(super) struct TrackedId(usize);

impl TrackedId {
    /// Generate the next unique `TrackedId` value
    fn next() -> Self {
        static TRACKED_ID: AtomicUsize = AtomicUsize::new(0);
        let next = TRACKED_ID.fetch_add(1, Ordering::Relaxed);
        TrackedId(next)
    }
}
