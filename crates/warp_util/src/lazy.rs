use std::cell::Cell;
use std::fmt;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

#[cfg(test)]
#[path = "lazy_tests.rs"]
mod tests;

/// A deferred compute closure, boxed to erase the (possibly capturing, per-instance) concrete
/// closure type so many differently-behaving instances can share the same `Lazy<T, S>` type.
type ComputeFn<T, S> = Box<dyn FnOnce(&S) -> T + Send>;

struct LazyInner<T, S> {
    cell: OnceLock<T>,
    /// Consumed at most once, inside `cell.get_or_init`, the first time [`Lazy::get`] is called.
    /// Wrapped in a `Mutex` purely for the interior mutability needed to move the closure out of
    /// `Option` from behind `&self`; `OnceLock::get_or_init` already guarantees the closure we
    /// register there (which does the taking) runs at most once.
    compute: Mutex<Option<ComputeFn<T, S>>>,
}

/// A value that is either supplied directly at construction (see [`Lazy::provided`]), or
/// computed from a `&S` the first time it's read via [`Lazy::get`] and cached thereafter (see
/// [`Lazy::deferred`]).
///
/// Unlike a plain `OnceLock`, the compute closure can capture per-instance data (e.g. an index
/// used to look up `S`-specific state), so a single `Lazy<T, S>` type can back many differently-
/// behaving instances. If `S` is itself expensive to obtain (e.g. requires acquiring a lock), that
/// cost is only paid the first time the value is read; every subsequent read returns the cached
/// value without invoking the closure (or needing a fresh `&S`) again.
///
/// Cloning a `Lazy` is a single `Arc::clone` and shares the same cache, so once any clone
/// computes the value, every other clone (and the original) observes the cached result instead
/// of recomputing it.
pub struct Lazy<T, S>(Arc<LazyInner<T, S>>);

impl<T, S> Lazy<T, S> {
    /// Wraps an already-computed `value`. Reading it via [`Lazy::get`] never invokes a compute
    /// closure (and therefore never needs a `&S`).
    pub fn provided(value: T) -> Self {
        Self(Arc::new(LazyInner {
            cell: OnceLock::from(value),
            compute: Mutex::new(None),
        }))
    }

    /// Defers computing the value until it's first read via [`Lazy::get`], at which point
    /// `compute` is called with the source value and the result is cached.
    pub fn deferred(compute: impl FnOnce(&S) -> T + Send + 'static) -> Self {
        Self(Arc::new(LazyInner {
            cell: OnceLock::new(),
            compute: Mutex::new(Some(Box::new(compute))),
        }))
    }

    /// Returns the value, computing (and caching) it from `source` first if necessary. `source`
    /// is only consulted (and the compute closure only invoked) the first time this is called;
    /// later calls return the cached value directly.
    pub fn get(&self, source: &S) -> &T {
        self.0.cell.get_or_init(|| {
            let compute = self
                .0
                .compute
                .lock()
                .take()
                .expect("Lazy value has no cached value and no deferred compute fn");
            compute(source)
        })
    }

    /// Returns a reference to the cached value, computing it first if necessary by invoking
    /// `with_source`, which is handed a `compute` callback to call once it has obtained a `&S`.
    ///
    /// This indirection (rather than simply taking a `impl FnOnce() -> &S`) exists because if `S`
    /// requires locking a mutex, returning a borrow `&S` out of a closure fails to compile: the
    /// guard would need to outlive the closure that produced it. By instead passing `compute`
    /// *into* the caller's scope, the lock guard can be held locally around the `compute(&S)`
    /// call and dropped immediately afterward.
    ///
    /// As with [`Lazy::get`], `with_source` (and therefore whatever lock it acquires) is only
    /// ever invoked the first time the value is read; later calls return the cached value
    /// directly.
    ///
    /// ```rust,ignore
    /// let value = lazy.get_with(|compute| {
    ///     let source_guard = source_mutex.lock();
    ///     compute(&source_guard)
    /// });
    /// ```
    pub fn get_with<F>(&self, with_source: F) -> &T
    where
        F: FnOnce(&dyn Fn(&S) -> T) -> T,
    {
        self.0.cell.get_or_init(|| {
            let compute = self
                .0
                .compute
                .lock()
                .take()
                .expect("Lazy value has no cached value and no deferred compute fn");

            // `with_source` requires a `Fn` (not `FnOnce`) callback so it can hold a lock guard
            // in local scope around the call, but `compute` is only safe to invoke once. A plain
            // (non-atomic) `Cell` suffices to guard against that, panicking if `with_source`
            // calls it more than once: this whole closure only ever runs on a single thread at a
            // time, since `OnceLock::get_or_init` guarantees at most one caller executes it.
            let compute_cell = Cell::new(Some(compute));
            with_source(&|source| {
                let compute = compute_cell
                    .take()
                    .expect("with_source callback invoked compute more than once");
                compute(source)
            })
        })
    }

    /// Returns the already-computed value without requiring a `&S`, or `None` if it hasn't been
    /// computed (or provided) yet.
    #[cfg(test)]
    pub fn get_if_cached(&self) -> Option<&T> {
        self.0.cell.get()
    }
}

impl<T, S> Clone for Lazy<T, S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: fmt::Debug, S> fmt::Debug for Lazy<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.cell.get() {
            Some(value) => fmt::Debug::fmt(value, f),
            None => f.write_str("Lazy(<uncomputed>)"),
        }
    }
}
