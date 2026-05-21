//! Automatic change tracking (autotracking) system to reduce the need to call `ctx.notify()`
//!
//! This module provides a wrapper type `Tracked`, which automatically tracks changes to the
//! underlying data and invalidates any Views that depend on that data (the equivalent of calling
//! `ctx.notify()` directly).
//!
//! ## Use
//!
//! The `Tracked` type is intended to be straightforward to use: Any data that is wrapped in a
//! `Tracked` will be hooked into the Autotracking system and will not need any calls to `notify`
//! from any views that depend on the data for rendering. Creating a `Tracked` can be done in two
//! ways:
//!
//! 1. Directly using the `Tracked::new` constructor, e.g. `Tracked::new(true)`
//! 2. Via `Into`, e.g. `true.into()`
//!
//! `Tracked` implements `Deref` and `DerefMut` for the underlying type, so in most cases you
//! should be able to use it directly wherever the underlying data would be expected. It may
//! require an explicit dereference (e.g. `*my_value`) in some cases, but other than that the
//! intent is for it to be mostly transparent.
//!
//! See the `autotracking` example in the `examples/` directory for more.
//!
//! ## Limitations
//!
//! `Tracked` is currently single-threaded, meaning that all of the values you want to have
//! automatically tracked must be on the main thread of the app. This should apply to anything
//! stored in a Model or View, as those are owned by the main thread already. To ensure that
//! autotracked data is not incorrectly shared between threads, `Tracked` explicitly does not
//! implement `Send` or `Sync`.
//!
//! Additionally, `Tracked` currently detects any mutable _access_ to the data as an update. It
//! explicitly does not do any diff of the data to determine if there was an actual change made.
//! This means it could generate false positive updates if you update it to the same values or
//! otherwise take mutable access without modifying the data.
//!
//! Similar to the above, since `Tracked` relies on _mutable_ access, it will not detect changes
//! in a type that uses interior mutability to make changes through a shared reference (`&self`
//! instead of `&mut self`).
//!
//! ## Granularity
//!
//! Since `Tracked` can wrap any type, it is up to the user to determine how granular you want the
//! tracking to be. It can wrap each individual value to give you very granular updates when only
//! specific dependencies are changed, or it can wrap an entire model and give coarse updates
//! whenever the model is modified.
//!
//! ## Details
//!
//! Internally, the Autotracking system works by tracking access in two ways:
//!
//! 1. While a View is rendering, any _reads_ of `Tracked` data are cached as dependencies for that
//!    view.
//! 2. When any `Tracked` data is _updated_, all Views that depended on that data are marked for
//!    invalidation.
//!
//! ### `Tracked`
//!
//! To track reads and updates, each instance of a `Tracked` includes a unique identifier used by
//! the autotracking system. The `Deref` and `DerefMut` implementations for `Tracked` send that
//! identifier to the Autotracking system indicating a read or update, respectively. The fact that
//! the tracking is tied to `Deref` and `DerefMut` is the reason behind the limitation listed above
//! that it could generate false-positive results.
//!
//! ### Reads
//!
//! In order to track reads and cache dependencies, whenever the UI Framework begins rendering a
//! View (i.e. calling `View::render` on it), it first notifies the Autotracking system that a
//! render is starting. The autotracking system clears the cache for that View and holds onto the
//! `WindowId` and `ViewId` for the duration of the render. During that time, any reads of
//! `Tracked` data result in the autotracking cache being updated to list that tracked data as a
//! dependency of the rendering view.
//!
//! When the call to `View::render` is complete, the UI Framework notifies the Autotracking system
//! that it's over and it stops associating reads with a View dependency.
//!
//! ### Updates
//!
//! Whenever a `Tracked` data is updated, the Autotracking system adds all views that depend on
//! that data to a set of invalidations. Then, every time the UI Framework collects the manual
//! invalidations (e.g. those created by calls to `ctx.notify()`), it also drains the stored
//! invalidations from the autotracking system. From that point forward, they are treated exactly
//! the same as if you had called `ctx.notify()` for the relevant views.
//!
//! ### Removing Views
//!
//! When the UI Framework removes a view or window, it notifies the Autotracking system of that
//! removal and any Views that no longer exist are removed from the dependency cache. This ensures
//! that we aren't wasting resources trying to invalidate views that no longer exist.
//!
//! ### Cache
//!
//! All of the Autotracking cached data is stored in a thread-local static on the main thread. This
//! removes the need for synchronization (e.g. `Mutex` or `RwLock`), as the data will only ever be
//! accessed by a single thread. This also allows the `Tracked` instances to notify about any reads
//! or updates without having to maintain a reference to the `AppContext` or similar app
//! state. However, this is also the source of the limitation that all data using `Tracked` must
//! be on the main thread and the lack of support for multithreaded change tracking.

mod tracked;

#[cfg(test)]
#[path = "autotracking_tests.rs"]
mod tests;

use itertools::Itertools as _;
pub use tracked::Tracked;

use super::{EntityId, WindowId};
use std::cell::UnsafeCell;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::mem;
use tracked::TrackedId;

/// Internal state cache used for autotracking changes
///
/// Rendering dependencies are stored in two maps, one from `TrackedId` -> Set of `View`s that
/// depend on that value; and the other from `View` -> Set of `TrackedId`s that View depends on.
/// This double-map allows us to insert and retrieve dependencies in O(1) time while also limiting
/// the time it takes to remove a view from the cache.
///
/// When we start rendering a view, we first clear that view's dependencies from the existing
/// cache, then we track that view in `rendering_view`. Subsequently, when a `Tracked` value is
/// read, we update the maps to reflect that dependency.
///
/// When a `Tracked` value is updated, we refer back to the cached set of Views that depend on that
/// value and add them all to the `invalidations` list, so that those views will be considered
/// invalidated on the next render.
#[derive(Default)]
struct Cache {
    rendering_view: Option<View>,
    view_dependencies: HashMap<View, HashSet<TrackedId>>,
    value_dependencies: HashMap<TrackedId, HashSet<View>>,
    invalidations: HashSet<View>,
}

/// Helper struct to encapsulate a View with its associated Window, necessary for properly tracking
/// invalidations by window.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
struct View {
    window_id: WindowId,
    view_id: EntityId,
}

thread_local! {
    static CACHE: UnsafeCell<Cache> = UnsafeCell::new(Cache::default())
}

/// Helper method for dereferencing the cache value and providing it to the caller via a callback.
fn with_cache<F, R>(callback: F) -> R
where
    F: FnOnce(&mut Cache) -> R,
{
    CACHE.with(|cache_cell| {
        // Safety: The cache is thread-local and only ever accessed by functions in this module.
        // Therefore, there is only ever one reference active at a time.
        let cache = unsafe { &mut *cache_cell.get() };

        callback(cache)
    })
}

/// Render a View using the provided callback while tracking any reads of `Tracked` values.
///
/// While the render is performed, any reads of `Tracked` values will be stored as dependencies for
/// the provided View.
///
/// ## Invariants
///
/// This function requires that only one view is rendered at a time, so the callback cannot result
/// in a recursive call to `render`
pub(super) fn render_view<F, R>(window_id: WindowId, view_id: EntityId, render_callback: F) -> R
where
    F: FnOnce() -> R,
{
    with_cache(|cache| {
        debug_assert!(cache.rendering_view.is_none());

        let view = View { window_id, view_id };
        // Clear the dependency cache for this view as it is being rendered again
        remove_view_internal(view, cache);
        cache.rendering_view = Some(view);

        let return_value = render_callback();

        cache.rendering_view = None;

        return_value
    })
}

/// Returns the list of windows that have invalidations caused by the
/// Autotracking system.
pub(super) fn windows_with_invalidations() -> Vec<WindowId> {
    with_cache(|cache| {
        cache
            .invalidations
            .iter()
            .map(|view| view.window_id)
            .unique()
            .collect_vec()
    })
}

/// Retrieves any invalidations for the given window caused by the Autotracking
/// system.
///
/// Note: This will clear the cache of invalidations for this window.
pub(super) fn take_invalidations_for_window(window_id: WindowId) -> HashSet<EntityId> {
    with_cache(|cache| {
        let (matching, remainder) = mem::take(&mut cache.invalidations)
            .into_iter()
            .partition(|view| view.window_id == window_id);
        cache.invalidations = remainder;
        matching.into_iter().map(|view| view.view_id).collect()
    })
}

/// Notify the Autotracking system that a Window is being closed
///
/// This will remove all Views associated with that Window from the dependencies cache to make
/// sure that we don't invalidate views from closed windows.
///
/// ## Invariants
///
/// This should not be called during the rendering of a View
pub(super) fn close_window(window_id: WindowId) {
    with_cache(|cache| {
        debug_assert!(cache.rendering_view.is_none());

        let removed_views = cache
            .view_dependencies
            .keys()
            .filter(|view| view.window_id == window_id)
            .copied()
            .collect::<Vec<_>>();

        for removed_view in removed_views {
            remove_view_internal(removed_view, cache);
        }
    })
}

/// Remove a view from the dependency cache and any existing invalidations
///
/// ## Invariants
///
/// Should not be called during the rendering of a View
pub(super) fn remove_view(window_id: WindowId, view_id: EntityId) {
    with_cache(|cache| {
        debug_assert!(cache.rendering_view.is_none());

        remove_view_internal(View { window_id, view_id }, cache);
    });
}

fn remove_view_internal(view: View, cache: &mut Cache) {
    for tracked_id in cache.view_dependencies.remove(&view).into_iter().flatten() {
        if let Entry::Occupied(mut entry) = cache.value_dependencies.entry(tracked_id) {
            entry.get_mut().remove(&view);

            if entry.get().is_empty() {
                entry.remove();
            }
        }
    }
}

/// Notify the Autotracking system that a given `Tracked` value was read
fn track_read(field: TrackedId) {
    with_cache(|cache| {
        if let Some(view) = cache.rendering_view {
            cache
                .view_dependencies
                .entry(view)
                .or_default()
                .insert(field);
            cache
                .value_dependencies
                .entry(field)
                .or_default()
                .insert(view);
        }
    });
}

/// Notify the Autotracking system that a given `Tracked` value was updated
fn track_update(field: TrackedId) {
    with_cache(|cache| {
        cache
            .invalidations
            .extend(cache.value_dependencies.get(&field).into_iter().flatten());
    })
}
