//! Tracks live, in-flight [`Foreground`](super::executor::Foreground) tasks
//! by the call site that spawned them.
//!
//! When the main thread accumulates memory, a heap profile's leaf frames
//! typically bottom out in `DispatchDelegate::run_on_main_thread` ->
//! `async_task::Runnable::run` -> a boxed `dyn Future::poll`: every
//! foreground task shares that same boxed-future shape, so the stack alone
//! can't say which task is responsible. This census fills that gap by
//! recording, per spawn call site, how many tasks spawned from it are
//! currently alive.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::panic::Location;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use futures_util::future::LocalBoxFuture;

#[derive(Default, Clone, Copy)]
struct SiteStats {
    live: u64,
    total_spawned: u64,
}

/// A live census of in-flight foreground tasks, keyed by the
/// `#[track_caller]` location of the call that spawned them.
///
/// [`Foreground`](super::executor::Foreground) is confined to the main
/// thread, so this is a plain [`RefCell`] rather than a `Mutex`: every
/// mutation happens on the thread that owns the executor, and there is no
/// lock contention to pay for on this hot path.
///
/// This is `pub` only because it appears in a field of the `pub` `Foreground`
/// enum; it isn't re-exported, and every method that could mutate or read
/// its state is crate-private.
#[derive(Default)]
pub struct ForegroundTaskCensus {
    sites: RefCell<HashMap<&'static Location<'static>, SiteStats>>,
}

impl ForegroundTaskCensus {
    fn record_spawn(&self, location: &'static Location<'static>) {
        let mut sites = self.sites.borrow_mut();
        let stats = sites.entry(location).or_default();
        stats.live += 1;
        stats.total_spawned += 1;
    }

    fn record_finish(&self, location: &'static Location<'static>) {
        if let Some(stats) = self.sites.borrow_mut().get_mut(location) {
            stats.live = stats.live.saturating_sub(1);
        }
    }

    /// Returns the total number of live foreground tasks, along with the
    /// `limit` spawn sites with the highest live counts, sorted descending.
    pub(super) fn snapshot(&self, limit: usize) -> ForegroundTaskCensusSnapshot {
        let sites = self.sites.borrow();
        let total_live_tasks: u64 = sites.values().map(|stats| stats.live).sum();

        let mut top_spawn_sites: Vec<SpawnSiteSnapshot> = sites
            .iter()
            .filter(|(_, stats)| stats.live > 0)
            .map(|(location, stats)| SpawnSiteSnapshot {
                location: format!("{}:{}", location.file(), location.line()),
                live_tasks: stats.live,
                total_spawned: stats.total_spawned,
            })
            .collect();
        top_spawn_sites.sort_unstable_by(|a, b| b.live_tasks.cmp(&a.live_tasks));
        top_spawn_sites.truncate(limit);

        ForegroundTaskCensusSnapshot {
            total_live_tasks,
            top_spawn_sites,
        }
    }
}

/// Wraps `future` so that spawning it is recorded against `location` on
/// `census`, and finishing it -- or dropping it, e.g. because it was aborted
/// or the executor was torn down -- decrements the live count again.
///
/// Reuses the caller's existing `LocalBoxFuture` allocation instead of
/// boxing a second time, so tracking costs one hash-map entry update plus an
/// `Rc` clone per spawn.
pub(super) fn track(
    census: Rc<ForegroundTaskCensus>,
    location: &'static Location<'static>,
    future: LocalBoxFuture<'static, ()>,
) -> TrackedTask {
    census.record_spawn(location);
    TrackedTask {
        future,
        guard: Some(CensusGuard { census, location }),
    }
}

struct CensusGuard {
    census: Rc<ForegroundTaskCensus>,
    location: &'static Location<'static>,
}

impl Drop for CensusGuard {
    fn drop(&mut self) {
        self.census.record_finish(self.location);
    }
}

/// Future adapter that decrements a task's live count exactly once: either
/// as soon as the wrapped future resolves, or -- if the task is instead
/// dropped while still pending, as happens on abort/cancellation -- when
/// [`CensusGuard`] itself is dropped.
pub(super) struct TrackedTask {
    future: LocalBoxFuture<'static, ()>,
    guard: Option<CensusGuard>,
}

impl Future for TrackedTask {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.future.as_mut().poll(cx) {
            Poll::Ready(()) => {
                // Drop the guard now rather than waiting for the task itself
                // to be dropped, so a completed-but-not-yet-dropped task
                // (e.g. one whose `async_task::Task` handle is held onto
                // without being polled again) isn't counted as still live.
                self.guard = None;
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// A point-in-time snapshot of live foreground tasks, suitable for attaching
/// to a heap profile or Sentry event.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ForegroundTaskCensusSnapshot {
    /// The total number of foreground tasks alive across all spawn sites,
    /// not just the ones in [`Self::top_spawn_sites`].
    pub total_live_tasks: u64,
    /// The spawn sites with the highest live task counts, descending.
    pub top_spawn_sites: Vec<SpawnSiteSnapshot>,
}

/// The live/total task counts for a single `#[track_caller]` spawn site.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpawnSiteSnapshot {
    /// `file:line` of the call that spawned these tasks.
    pub location: String,
    /// The number of tasks spawned from this call site that are still live.
    pub live_tasks: u64,
    /// The total number of tasks ever spawned from this call site, across
    /// the lifetime of the executor.
    pub total_spawned: u64,
}
